//! コード表示。行単位の仮想スクロール、ハイライト、シンボル、定義ジャンプ。
pub mod buffer;
pub mod history;

use crate::theme;
use buffer::{Buffer, BufferKind, word_at};
use ccv_core::highlight::{Span, Token, highlight};
use ccv_core::model::{Lang, RepoIndex, Symbol, SymbolKind};
use gpui::{
    Context, EventEmitter, FontWeight, HighlightStyle, MouseButton, MouseDownEvent, Render,
    ScrollStrategy, SharedString, StyledText, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use history::History;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_LINE_CHARS: usize = 2_000;
const LINE_HEIGHT: f32 = 21.0;

pub enum CodeEvent {
    /// 別のファイルへ移動した。星図を追従させる
    Navigated(String),
    Close,
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
    symbols_open: bool,
    load_generation: u64,
    longest: usize,
    /// 選択中の行範囲。始点と現在位置で、順序は問わない
    selection: Option<(usize, usize)>,
    selecting: bool,
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
            symbols_open: true,
            load_generation: 0,
            longest: 0,
            selection: None,
            selecting: false,
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
        self.spans = Arc::new(Vec::new());
        self.load_generation += 1;
        let generation = self.load_generation;
        let full = self.repo.join(path);
        let lang = Lang::from_path(path);
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
                .background_spawn(async move { highlight(lang, for_highlight.text()) })
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

    pub fn toggle_symbols(&mut self, cx: &mut Context<Self>) {
        self.symbols_open = !self.symbols_open;
        cx.notify();
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

fn symbol_glyph(kind: SymbolKind) -> &'static str {
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
        let symbols: Vec<Symbol> = self.symbols().to_vec();
        // 数個しかないときは、コードの幅を優先する
        let show_symbols = self.symbols_open && symbols.len() >= 4;

        let body = match &self.content {
            Content::Empty => div().flex_1().into_any_element(),
            Content::Failed(message) => message_box(
                "このファイルを開けませんでした",
                message.clone(),
            ),
            Content::Ready(b) if b.kind() == BufferKind::Binary => {
                message_box("バイナリファイル", human_bytes(b.bytes()))
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
                .font_family(theme::CODE_FONT)
                .text_size(px(13.))
                .line_height(px(LINE_HEIGHT))
                .text_color(theme::tok_plain())
                .into_any_element()
            }
        };

        // ツアーの範囲が属する関数を、シンボル一覧で示す
        let active_symbol = self.band.and_then(|(start, _)| {
            symbols.iter().rposition(|s| s.line < start)
        });
        let fade_top = gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(theme::panel(), 0.),
            gpui::linear_color_stop(theme::alpha(theme::panel(), 0.), 1.),
        );
        let fade_bottom = gpui::linear_gradient(
            0.,
            gpui::linear_color_stop(theme::panel(), 0.),
            gpui::linear_color_stop(theme::alpha(theme::panel(), 0.), 1.),
        );
        // 右へはみ出す行があることを、端のぼかしで伝える
        let fade_right = gpui::linear_gradient(
            270.,
            gpui::linear_color_stop(theme::panel(), 0.),
            gpui::linear_color_stop(theme::alpha(theme::panel(), 0.), 1.),
        );
        let crumbs: Vec<String> = path.split('/').map(str::to_string).collect();
        let crumb_count = crumbs.len();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(16.))
                    .gap(px(6.))
                    .border_b_1()
                    .border_color(theme::hairline())
                    .text_size(px(12.))
                    .font_family(theme::CODE_FONT)
                    .child(icon_button("nav-back", "←").on_click(cx.listener(|this, _, _, cx| {
                        this.back(cx);
                    })))
                    .child(icon_button("nav-forward", "→").on_click(cx.listener(|this, _, _, cx| {
                        this.forward(cx);
                    })))
                    .children(crumbs.into_iter().enumerate().map(|(i, c)| {
                        let last = i + 1 == crumb_count;
                        div()
                            .flex()
                            .gap(px(6.))
                            .when(i > 0, |d| {
                                d.child(div().text_color(theme::hairline_strong()).child("/"))
                            })
                            .child(
                                div()
                                    .text_color(if last { theme::text() } else { theme::text_muted() })
                                    .when(last, |d| d.font_weight(FontWeight::BOLD))
                                    .child(c),
                            )
                    }))
                    .child(div().flex_1())
                    .child(
                        icon_button("close-code", "閉じる  esc")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CodeEvent::Close))),
                    ),
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
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(body)
                            .child(div().absolute().top_0().left_0().right_0().h(px(14.)).bg(fade_top))
                            .child(div().absolute().top_0().bottom_0().right_0().w(px(28.)).bg(fade_right))
                            .child(
                                div().absolute().bottom_0().left_0().right_0().h(px(18.)).bg(fade_bottom),
                            ),
                    )
                    .when(show_symbols, |d| {
                        d.child(
                            div()
                                .id("symbols")
                                .w(px(176.))
                                .flex_none()
                                .h_full()
                                .overflow_y_scroll()
                                .border_l_1()
                                .border_color(theme::hairline())
                                .py(px(10.))
                                .children(symbols.into_iter().enumerate().map(|(i, s)| {
                                    let line = s.line;
                                    let active = active_symbol == Some(i);
                                    div()
                                        .id(("symbol", i))
                                        .when(active, |d| {
                                            d.bg(theme::alpha(theme::teal(), 0.12))
                                                .border_l_2()
                                                .border_color(theme::teal())
                                        })
                                        .px(px(14.))
                                        .py(px(3.))
                                        .flex()
                                        .gap(px(8.))
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .font_family(theme::CODE_FONT)
                                        .hover(|h| h.bg(theme::alpha(theme::violet(), 0.1)))
                                        .child(
                                            div()
                                                .w(px(10.))
                                                .flex_none()
                                                .text_color(theme::violet())
                                                .child(symbol_glyph(s.kind)),
                                        )
                                        .child(
                                            div()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_color(theme::tok_plain())
                                                .child(crate::palette::middle_ellipsis(&s.name, 17)),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let path = this.path.clone().unwrap_or_default();
                                            this.open(&path, line, None, cx);
                                        }))
                                })),
                        )
                    })
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
}
