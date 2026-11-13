//! コード表示。行単位の仮想スクロール、ハイライト、シンボル、定義ジャンプ。
pub mod buffer;
pub mod history;

use crate::theme;
use buffer::{Buffer, BufferKind, word_at};
use ccv_core::highlight::{Span, Token, highlight_path};
use ccv_core::model::{RepoIndex, Symbol, SymbolKind};
use gpui::{
    Context, EventEmitter, FontWeight, HighlightStyle, MouseButton, MouseDownEvent, Render,
    ScrollStrategy, SharedString, StyledText, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use crate::text_input::{InputChanged, TextInput};
use gpui::{Entity, Focusable, Subscription, actions};
use history::History;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

actions!(code_find, [FindNext, FindPrev, FindClose]);

const MAX_LINE_CHARS: usize = 2_000;
const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FindMode {
    Text,
    Line,
}

struct FindBar {
    mode: FindMode,
    input: Entity<TextInput>,
    /// 行と、その行の中のバイト範囲
    matches: Vec<(usize, Range<usize>)>,
    current: usize,
    _subscription: Subscription,
}

/// 大文字を含むときだけ大文字小文字を区別する。
pub fn find_matches(text: &str, query: &str) -> Vec<(usize, Range<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let sensitive = query.chars().any(char::is_uppercase);
    let needle = if sensitive { query.to_string() } else { query.to_lowercase() };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        // 小文字にすると長さが変わる文字があるので、そのときは区別して探す
        let lowered;
        let hay = if sensitive {
            line
        } else {
            lowered = line.to_lowercase();
            if lowered.len() == line.len() { lowered.as_str() } else { line }
        };
        let mut from = 0;
        while let Some(at) = hay[from..].find(&needle) {
            let start = from + at;
            out.push((i, start..start + needle.len()));
            if out.len() >= MAX_MATCHES {
                return out;
            }
            from = start + needle.len().max(1);
        }
    }
    out
}

/// 色の区間に、検索の一致の背景を重ねる。区間は重ならないように切り直す。
fn overlay_matches(
    base: Vec<(Range<usize>, HighlightStyle)>,
    marks: &[(Range<usize>, bool)],
    len: usize,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if marks.is_empty() {
        return base;
    }
    let mut cuts: Vec<usize> = vec![0, len];
    for (r, _) in &base {
        cuts.extend([r.start, r.end]);
    }
    for (r, _) in marks {
        cuts.extend([r.start.min(len), r.end.min(len)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut style = base
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, s)| *s)
            .unwrap_or_default();
        let mark = marks.iter().find(|(r, _)| r.start <= a && b <= r.end);
        match mark {
            Some((_, current)) => {
                style.background_color = Some(if *current {
                    theme::alpha(theme::teal(), 0.55)
                } else {
                    theme::alpha(theme::violet(), 0.3)
                });
                if *current {
                    style.color = Some(theme::space().into());
                }
            }
            None if style == HighlightStyle::default() => continue,
            None => {}
        }
        out.push((a..b, style));
    }
    out
}
const LINE_HEIGHT: f32 = 21.0;

pub enum CodeEvent {
    /// 別のファイルへ移動した。星図を追従させる
    Navigated(String),
    /// 検索を閉じた。キー操作を画面へ戻す
    FindClosed,
}

/// 名前が一致するシンボルの定義を探す。
pub fn definitions(index: &RepoIndex, name: &str) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = index
        .symbols
        .iter()
        .flat_map(|(path, symbols)| {
            symbols.iter().filter(|s| s.name == name).map(|s| (path.clone(), s.line))
        })
        .collect();
    out.sort();
    out
}

#[derive(Clone, Debug, PartialEq)]
struct Location {
    path: String,
    line: u32,
}

enum Content {
    Empty,
    Failed(String),
    Ready(Arc<Buffer>),
}

pub struct CodeView {
    repo: PathBuf,
    index: Arc<RepoIndex>,
    path: Option<String>,
    content: Content,
    spans: Arc<Vec<Vec<Span>>>,
    band: Option<(u32, u32)>,
    scroll: UniformListScrollHandle,
    history: History<Location>,
    candidates: Vec<(String, u32)>,
    load_generation: u64,
    longest: usize,
    /// 選択中の行範囲。始点と現在位置で、順序は問わない
    selection: Option<(usize, usize)>,
    selecting: bool,
    find: Option<FindBar>,
}

impl EventEmitter<CodeEvent> for CodeView {}

fn token_color(token: Token) -> gpui::Rgba {
    match token {
        Token::Keyword => theme::tok_keyword(),
        Token::Function => theme::tok_function(),
        Token::Type => theme::tok_type(),
        Token::String => theme::tok_string(),
        Token::Number => theme::tok_number(),
        Token::Comment => theme::tok_comment(),
        Token::Constant => theme::tok_constant(),
        Token::Plain => theme::tok_plain(),
    }
}

fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1024.0 / 1024.0),
    }
}

/// 3 桁区切りで数を表示する。
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 色の区間を、切り詰めたあとの行に収める。
fn clip_spans(spans: &[Span], text_len: usize) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut out = Vec::with_capacity(spans.len());
    let mut at = 0usize;
    for s in spans {
        let (start, end) = ((s.start as usize).max(at), (s.end as usize).min(text_len));
        if start >= end {
            continue;
        }
        let style = HighlightStyle { color: Some(token_color(s.token).into()), ..Default::default() };
        out.push((start..end, style));
        at = end;
    }
    out
}

impl CodeView {
    pub fn new(repo: PathBuf, index: Arc<RepoIndex>) -> CodeView {
        CodeView {
            repo,
            index,
            path: None,
            content: Content::Empty,
            spans: Arc::new(Vec::new()),
            band: None,
            scroll: UniformListScrollHandle::new(),
            history: History::default(),
            candidates: Vec::new(),
            load_generation: 0,
            longest: 0,
            selection: None,
            selecting: false,
            find: None,
        }
    }

    /// ファイルを開く。`line` は 0 始まり。`band` は強調する行範囲（1 始まり、両端を含む）。
    pub fn open(
        &mut self,
        path: &str,
        line: u32,
        band: Option<(u32, u32)>,
        cx: &mut Context<Self>,
    ) {
        self.history.push(Location { path: path.to_string(), line });
        self.show(path, line, band, cx);
    }

    fn show(&mut self, path: &str, line: u32, band: Option<(u32, u32)>, cx: &mut Context<Self>) {
        self.band = band;
        self.candidates.clear();
        if self.path.as_deref() == Some(path) {
            self.scroll_to(line);
            cx.notify();
            return;
        }
        self.path = Some(path.to_string());
        self.selection = None;
        self.find = None;
        self.spans = Arc::new(Vec::new());
        self.load_generation += 1;
        let generation = self.load_generation;
        let full = self.repo.join(path);
        let path_for_highlight = path.to_string();
        let path_owned = path.to_string();
        cx.emit(CodeEvent::Navigated(path_owned));

        let read = cx.background_spawn(async move {
            let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
            Ok::<Buffer, String>(Buffer::from_bytes(bytes))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let buffer = match result {
                Ok(b) => Arc::new(b),
                Err(message) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.load_generation == generation {
                            this.content = Content::Failed(message);
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let for_highlight = buffer.clone();
            let keep_going = this
                .update(cx, |this, cx| {
                    if this.load_generation != generation {
                        return false;
                    }
                    this.longest = buffer.longest_line();
                    this.content = Content::Ready(buffer);
                    this.scroll_to(line);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !keep_going || for_highlight.kind() != BufferKind::Text {
                return;
            }
            // 描画を止めないよう、色付けは後から届ける
            let spans = cx
                .background_spawn(async move { highlight_path(&path_for_highlight, for_highlight.text()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.load_generation == generation {
                    this.spans = Arc::new(spans);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn scroll_to(&self, line: u32) {
        // 文脈が見えるよう、少し上に余白を残す
        let target = (line as usize).saturating_sub(3);
        self.scroll.scroll_to_item_strict(target, ScrollStrategy::Top);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(loc) = self.history.back().cloned() {
            self.show(&loc.path, loc.line, None, cx);
        }
    }

    pub fn forward(&mut self, cx: &mut Context<Self>) {
        if let Some(loc) = self.history.forward().cloned() {
            self.show(&loc.path, loc.line, None, cx);
        }
    }

    /// 選択中の行をコピーする。選択がなければ何もしない。
    pub fn copy_selection(&self, cx: &mut Context<Self>) {
        let (Some((a, b)), Content::Ready(buffer)) = (self.selection, &self.content) else {
            return;
        };
        let text = buffer.lines_text(a.min(b), a.max(b));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    pub fn line_count(&self) -> Option<usize> {
        match &self.content {
            Content::Ready(b) if b.kind() == BufferKind::Text => Some(b.line_count()),
            _ => None,
        }
    }

    /// 画面のいちばん上に見えている行。0 始まり
    pub fn top_line(&self) -> usize {
        let state = self.scroll.0.borrow();
        match &state.deferred_scroll_to_item {
            Some(d) => d.item_index,
            // 上の余白を差し引き、行の高さで割る
            None => ((-f32::from(state.base_handle.offset().y) - 8.0).max(0.0) / LINE_HEIGHT) as usize,
        }
    }

    pub fn outline(&self) -> &[Symbol] {
        self.symbols()
    }

    /// いま読んでいる場所を含むシンボル。ツアーの範囲があればそれを優先する
    pub fn active_symbol(&self) -> Option<usize> {
        let symbols = self.symbols();
        let top = self.top_line() as u32;
        // 範囲が画面から外れたら、範囲ではなく見ている位置で決める
        let line = match self.band {
            Some((start, _)) if start > top && start <= top + 40 => start.saturating_sub(1),
            // 上に残す余白の分だけ下を見る
            _ => top + 3,
        };
        symbols.iter().rposition(|s| s.line <= line)
    }

    /// ファイル内の検索か、行への移動を始める。
    pub fn open_find(&mut self, mode: FindMode, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(&self.content, Content::Ready(b) if b.kind() == BufferKind::Text) {
            return;
        }
        let keep = self.find.as_ref().filter(|f| f.mode == mode).map(|f| f.input.clone());
        let input = match keep {
            Some(input) => input,
            None => {
                let placeholder = match mode {
                    FindMode::Text => "このファイルを検索",
                    FindMode::Line => "移動する行の番号",
                };
                let input = cx.new(|cx| TextInput::new(placeholder, cx));
                let subscription = cx.subscribe(&input, |this, _, _: &InputChanged, cx| this.refresh_find(cx));
                self.find = Some(FindBar { mode, input: input.clone(), matches: Vec::new(), current: 0, _subscription: subscription });
                input
            }
        };
        let handle = input.focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// 撮影のために、検索語を直接入れる。
    pub fn set_find_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if let Some(find) = &self.find {
            let input = find.input.clone();
            input.update(cx, |i, cx| i.set_content(query.to_string(), cx));
            self.refresh_find(cx);
        }
    }

    pub fn find_open(&self) -> bool {
        self.find.is_some()
    }

    fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let Content::Ready(buffer) = &self.content else { return };
        let buffer = buffer.clone();
        let Some(find) = &mut self.find else { return };
        let query = find.input.read(cx).content().to_string();
        match find.mode {
            FindMode::Text => {
                find.matches = find_matches(buffer.text(), &query);
                // 画面に見えている位置から近い一致を選ぶ
                let top = {
                    let state = self.scroll.0.borrow();
                    ((-f32::from(state.base_handle.offset().y)).max(0.0) / LINE_HEIGHT) as usize
                };
                let Some(find) = &mut self.find else { return };
                find.current = find.matches.iter().position(|(l, _)| *l >= top).unwrap_or(0);
                if let Some((line, _)) = find.matches.get(find.current) {
                    let line = *line;
                    self.reveal_line(line);
                }
            }
            FindMode::Line => {
                if let Ok(n) = query.trim().parse::<usize>()
                    && n > 0
                {
                    let line = (n - 1).min(buffer.line_count().saturating_sub(1));
                    self.band = Some((line as u32 + 1, line as u32 + 1));
                    self.reveal_line(line);
                }
            }
        }
        cx.notify();
    }

    /// 一致した行を、画面の上から少し下の位置に出す。
    fn reveal_line(&self, line: usize) {
        self.scroll.scroll_to_item_strict(line.saturating_sub(6), ScrollStrategy::Top);
    }

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        match find.mode {
            FindMode::Text => {
                if find.matches.is_empty() {
                    return;
                }
                let n = find.matches.len() as isize;
                find.current = ((find.current as isize + delta).rem_euclid(n)) as usize;
                let line = find.matches[find.current].0;
                self.reveal_line(line);
            }
            FindMode::Line => {
                // 行への移動は確定して閉じる
                let line = self.band.map(|(a, _)| a.saturating_sub(1));
                self.find = None;
                if let Some(line) = line {
                    self.jump(line, cx);
                }
                cx.emit(CodeEvent::FindClosed);
            }
        }
        cx.notify();
    }

    fn on_find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step_find(1, cx);
    }

    fn on_find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.step_find(-1, cx);
    }

    fn on_find_close(&mut self, _: &FindClose, _: &mut Window, cx: &mut Context<Self>) {
        self.close_find(cx);
    }

    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            cx.emit(CodeEvent::FindClosed);
            cx.notify();
        }
    }

    /// 同じファイルの中で行へ移る。`line` は 0 始まり
    pub fn jump(&mut self, line: u32, cx: &mut Context<Self>) {
        if let Some(path) = self.path.clone() {
            self.open(&path, line, None, cx);
        }
    }

    fn go_to_definition(&mut self, word: &str, cx: &mut Context<Self>) {
        let found = definitions(&self.index, word);
        match found.as_slice() {
            [] => {
                self.candidates.clear();
                cx.notify();
            }
            [(path, line)] => {
                let (path, line) = (path.clone(), *line);
                self.open(&path, line, None, cx);
            }
            _ => {
                self.candidates = found;
                cx.notify();
            }
        }
    }

    fn symbols(&self) -> &[Symbol] {
        self.path.as_ref().and_then(|p| self.index.symbols.get(p)).map_or(&[], Vec::as_slice)
    }

    fn render_lines(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let Content::Ready(buffer) = &self.content else { return Vec::new() };
        let buffer = buffer.clone();
        let band = self.band;
        let selection = self.selection;
        let spans = self.spans.clone();
        let (matches, current): (Vec<(usize, Range<usize>)>, usize) = match &self.find {
            Some(f) if f.mode == FindMode::Text => {
                let lo = f.matches.partition_point(|(l, _)| *l < range.start);
                let hi = f.matches.partition_point(|(l, _)| *l < range.end);
                (f.matches[lo..hi].to_vec(), f.current.wrapping_sub(lo))
            }
            _ => (Vec::new(), usize::MAX),
        };
        range
            .map(|i| {
                let raw = buffer.line(i);
                let (text, truncated) = if raw.len() > MAX_LINE_CHARS {
                    let mut cut = MAX_LINE_CHARS;
                    while !raw.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    (format!("{}…", &raw[..cut]), true)
                } else {
                    (raw.to_string(), false)
                };
                let limit = if truncated { text.len() - '…'.len_utf8() } else { text.len() };
                let highlights =
                    spans.get(i).map(|s| clip_spans(s, limit)).unwrap_or_default();
                let marks: Vec<(Range<usize>, bool)> = matches
                    .iter()
                    .enumerate()
                    .filter(|(_, (l, r))| *l == i && r.end <= limit)
                    .map(|(k, (_, r))| (r.clone(), k == current))
                    .collect();
                let highlights = overlay_matches(highlights, &marks, limit);
                let number = i as u32 + 1;
                let in_band = band.is_some_and(|(a, b)| number >= a && number <= b);
                let styled = StyledText::new(SharedString::from(text)).with_highlights(highlights);
                let layout = styled.layout().clone();
                let line_text: Arc<str> = raw.into();
                let selected = selection.is_some_and(|(a, b)| i >= a.min(b) && i <= a.max(b));
                div()
                    .id(i)
                    .h(px(LINE_HEIGHT))
                    .w_full()
                    .flex()
                    .flex_row()
                    .cursor_text()
                    .when(in_band, |d| d.bg(theme::alpha(theme::violet(), 0.11)))
                    .when(selected, |d| d.bg(theme::alpha(theme::teal(), 0.17)))
                    .child(
                        div()
                            .w(px(3.))
                            .flex_none()
                            .h_full()
                            .when(in_band, |d| d.bg(theme::teal())),
                    )
                    .child(
                        div()
                            .w(px(58.))
                            .flex_none()
                            .pr(px(16.))
                            .flex()
                            .justify_end()
                            .text_color(if in_band { theme::text() } else { theme::tok_comment() })
                            .child(number.to_string()),
                    )
                    .child(div().flex_none().whitespace_nowrap().pr(px(40.)).child(styled))
                    .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                        if this.selecting && e.dragging() {
                            if let Some((start, end)) = this.selection
                                && end != i
                            {
                                this.selection = Some((start, i));
                                cx.notify();
                            }
                        } else {
                            this.selecting = false;
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.selecting = false;
                            // 1 行だけのクリックは選択にしない
                            if this.selection.is_some_and(|(a, b)| a == b) {
                                this.selection = None;
                            }
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            if !e.modifiers.platform {
                                this.selection = Some((i, i));
                                this.selecting = true;
                                cx.notify();
                                return;
                            }
                            let byte = match layout.index_for_position(e.position) {
                                Ok(i) | Err(i) => i,
                            };
                            if let Some(word) = word_at(&line_text, byte.min(line_text.len())) {
                                let word = word.to_string();
                                this.go_to_definition(&word, cx);
                            }
                        }),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

pub fn symbol_glyph(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "ƒ",
        SymbolKind::Type => "T",
        SymbolKind::Module => "◇",
        SymbolKind::Constant => "c",
    }
}

impl Render for CodeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.path.clone().unwrap_or_default();
        let symbol = self.active_symbol().and_then(|i| self.symbols().get(i).cloned());

        let body = match &self.content {
            Content::Empty => div().flex_1().into_any_element(),
            Content::Failed(message) => message_box(
                "このファイルを開けませんでした",
                message.clone(),
            ),
            Content::Ready(b) if b.kind() == BufferKind::Binary => {
                message_box("バイナリファイルは表示しません", human_bytes(b.bytes()))
            }
            Content::Ready(b) if b.kind() == BufferKind::TooLarge => {
                message_box("大きすぎるため表示しません", human_bytes(b.bytes()))
            }
            Content::Ready(b) => {
                let count = b.line_count();
                uniform_list(
                    "code-lines",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.render_lines(range, cx)),
                )
                .track_scroll(self.scroll.clone())
                .with_horizontal_sizing_behavior(gpui::ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(self.longest))
                .flex_1()
                .pt(px(8.))
                .font_family(theme::CODE_FONT)
                .text_size(px(13.))
                .line_height(px(LINE_HEIGHT))
                .text_color(theme::tok_plain())
                .into_any_element()
            }
        };

        // 右へはみ出す行があることを、端のぼかしで伝える
        let fade_right = gpui::linear_gradient(
            270.,
            gpui::linear_color_stop(theme::panel(), 0.),
            gpui::linear_color_stop(theme::alpha(theme::panel(), 0.), 1.),
        );
        let crumbs: Vec<String> = path.split('/').map(str::to_string).collect();
        let crumb_count = crumbs.len();
        let separator = || div().px(px(2.)).text_color(theme::hairline_strong()).child("›");

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                div()
                    .h(px(30.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(8.))
                    .pr(px(16.))
                    .gap(px(4.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .child(nav_button("nav-back", "‹", self.history.can_back()).on_click(
                        cx.listener(|this, _, _, cx| this.back(cx)),
                    ))
                    .child(nav_button("nav-forward", "›", self.history.can_forward()).on_click(
                        cx.listener(|this, _, _, cx| this.forward(cx)),
                    ))
                    .child(div().w(px(6.)))
                    .children(crumbs.into_iter().enumerate().map(|(i, c)| {
                        let last = i + 1 == crumb_count;
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .when(i > 0, |d| d.child(separator()))
                            .child(
                                div()
                                    .text_color(if last { theme::text() } else { theme::text_muted() })
                                    .when(last, |d| d.font_weight(FontWeight::MEDIUM))
                                    .child(c),
                            )
                    }))
                    // いま読んでいる関数まで示し、ファイルの中の位置も分かるようにする
                    .when_some(symbol, |d, s| {
                        d.child(separator()).child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .font_family(theme::CODE_FONT)
                                .text_size(px(11.5))
                                .child(div().text_color(theme::violet()).child(symbol_glyph(s.kind)))
                                .child(div().text_color(theme::text()).child(s.name)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .relative()
                    .child(
                        div()
                            .id("code-body")
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            // 読んでいる場所の表示を、スクロールに合わせて更新する
                            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                            .child(body)
                            .child(div().absolute().top_0().bottom_0().right_0().w(px(28.)).bg(fade_right)),
                    )
                    .when_some(self.render_find(cx), |d, bar| d.child(bar))
                    .when(!self.candidates.is_empty(), |d| {
                        let items = self.candidates.clone();
                        d.child(
                            div()
                                .id("definition-candidates")
                                .absolute()
                                .top(px(12.))
                                .left(px(72.))
                                .w(px(460.))
                                .max_h(px(280.))
                                .overflow_y_scroll()
                                .bg(theme::panel_raised())
                                .border_1()
                                .border_color(theme::hairline_strong())
                                .rounded(px(8.))
                                .shadow_lg()
                                .py(px(6.))
                                .child(
                                    div()
                                        .px(px(12.))
                                        .py(px(4.))
                                        .text_size(px(11.))
                                        .text_color(theme::text_muted())
                                        .child(format!("定義の候補 {} 件", items.len())),
                                )
                                .children(items.into_iter().enumerate().map(|(i, (path, line))| {
                                    let label = format!("{}:{}", path, line + 1);
                                    div()
                                        .id(("candidate", i))
                                        .px(px(12.))
                                        .py(px(4.))
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .font_family(theme::CODE_FONT)
                                        .text_color(theme::text())
                                        .hover(|h| h.bg(theme::alpha(theme::violet(), 0.14)))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open(&path, line, None, cx);
                                        }))
                                })),
                        )
                    }),
            )
    }
}

impl CodeView {
    fn render_find(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let find = self.find.as_ref()?;
        let has_query = !find.input.read(cx).content().is_empty();
        let (label, status) = match find.mode {
            FindMode::Text => {
                let status = if !has_query {
                    String::new()
                } else if find.matches.is_empty() {
                    "見つかりません".to_string()
                } else {
                    let more = if find.matches.len() >= MAX_MATCHES { "+" } else { "" };
                    format!("{} / {}{}", find.current + 1, find.matches.len(), more)
                };
                ("検索", status)
            }
            FindMode::Line => {
                let total = self.line_count().unwrap_or(0);
                ("行へ移動", format!("1 – {}", group_digits(total as u64)))
            }
        };
        let empty = find.mode == FindMode::Text && has_query && find.matches.is_empty();
        Some(
            div()
                .key_context("CodeFind")
                .on_action(cx.listener(Self::on_find_next))
                .on_action(cx.listener(Self::on_find_prev))
                .on_action(cx.listener(Self::on_find_close))
                .absolute()
                .top(px(10.))
                .right(px(22.))
                .w(px(380.))
                .h(px(36.))
                .pl(px(12.))
                .pr(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(9.))
                .bg(theme::panel_raised())
                .border_1()
                .border_color(if empty { theme::alpha(theme::danger(), 0.6) } else { theme::hairline_strong().into() })
                .shadow_lg()
                .occlude()
                .child(div().flex_none().text_size(px(11.)).text_color(theme::violet()).child(label))
                .child(div().flex_1().min_w_0().text_size(px(13.)).child(find.input.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .font_family(theme::CODE_FONT)
                        .text_color(if empty { theme::danger() } else { theme::text_muted() })
                        .child(status),
                )
                .when(find.mode == FindMode::Text, |d| {
                    d.child(nav_button("find-prev", "‹", !find.matches.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.step_find(-1, cx)),
                    ))
                    .child(nav_button("find-next", "›", !find.matches.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.step_find(1, cx)),
                    ))
                })
                .child(nav_button("find-close", "×", true).on_click(cx.listener(|this, _, _, cx| this.close_find(cx))))
                .into_any_element(),
        )
    }
}

fn nav_button(id: &'static str, label: &'static str, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_size(px(15.))
        .text_color(if enabled { theme::text() } else { theme::alpha(theme::text_muted(), 0.4).into() })
        .when(enabled, |d| d.cursor_pointer().hover(|h| h.bg(theme::alpha(theme::white(), 0.07))))
        .child(label)
}

fn message_box(title: &'static str, detail: String) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .child(div().text_size(px(15.)).text_color(theme::text()).child(title))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(detail))
        .into_any_element()
}

pub fn icon_button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(8.))
        .py(px(3.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(11.))
        .text_color(theme::text_muted())
        .font_family(theme::UI_FONT)
        .hover(|h| h.bg(theme::alpha(theme::violet(), 0.12)).text_color(theme::text()))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccv_core::model::Symbol;

    #[test]
    fn definitions_are_sorted_and_exact() {
        let mut index = RepoIndex::default();
        let s = |n: &str, line| Symbol { name: n.into(), kind: SymbolKind::Function, line };
        index.symbols.insert("b.rs".into(), vec![s("run", 3), s("running", 4)]);
        index.symbols.insert("a.rs".into(), vec![s("run", 1)]);
        assert_eq!(definitions(&index, "run"), [("a.rs".to_string(), 1), ("b.rs".to_string(), 3)]);
        assert!(definitions(&index, "nope").is_empty());
    }

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn clip_spans_respects_truncation_and_overlaps() {
        let sp = |start, end| Span { start, end, token: Token::Comment };
        let got = clip_spans(&[sp(0, 4), sp(2, 6), sp(8, 20)], 10);
        let ranges: Vec<_> = got.into_iter().map(|(r, _)| r).collect();
        assert_eq!(ranges, [0..4, 4..6, 8..10]);
    }

    #[test]
    fn find_is_smart_case_and_per_line() {
        let text = "let Foo = 1;\nfoo(foo);\n";
        assert_eq!(find_matches(text, "foo"), [(0, 4..7), (1, 0..3), (1, 4..7)]);
        assert_eq!(find_matches(text, "Foo"), [(0, 4..7)]);
        assert!(find_matches(text, "").is_empty());
    }

    #[test]
    fn overlay_splits_syntax_spans_without_overlap() {
        let kw = HighlightStyle { color: Some(theme::violet().into()), ..Default::default() };
        let out = overlay_matches(vec![(0..6, kw)], &[(3..9, true)], 10);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..3, 3..6, 6..9]);
        assert!(out[1].1.background_color.is_some());
        assert!(out[0].1.background_color.is_none());
    }
}
