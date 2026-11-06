//! ⌘P のファイル検索と ⌘⇧F の全文検索。
use crate::text_input::{InputChanged, TextInput};
use crate::theme;
use ccv_core::model::RepoIndex;
use ccv_core::search::{Hit, fuzzy_files, search_content};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, Render, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

actions!(palette, [SelectNext, SelectPrev, Confirm, Dismiss]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    Files,
    Content,
}

pub enum PaletteEvent {
    Open(String, Option<u32>),
    /// 選択中のファイルと、検索結果に含まれるファイル
    Preview(Vec<String>),
    Dismiss,
}

#[derive(Default)]
pub struct Selection {
    index: usize,
    len: usize,
}

impl Selection {
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.index = 0;
    }

    pub fn next(&mut self) {
        if self.len > 0 {
            self.index = (self.index + 1) % self.len;
        }
    }

    pub fn prev(&mut self) {
        if self.len > 0 {
            self.index = (self.index + self.len - 1) % self.len;
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn current(&self) -> Option<usize> {
        (self.len > 0).then_some(self.index)
    }
}

/// 一致箇所が見えるよう、前後を切り詰めた文字列と、その中の一致範囲を返す。
fn snippet_around(
    text: &str,
    query: &str,
    before: usize,
    max_chars: usize,
) -> (String, Option<Range<usize>>) {
    let trimmed = text.trim_start();
    // 大文字小文字を無視して探す。文字数が変わる文字があれば強調だけ諦める
    let lower = trimmed.to_lowercase();
    let at = (lower.len() == trimmed.len() && !query.is_empty())
        .then(|| lower.find(&query.to_lowercase()))
        .flatten();
    let Some(at) = at else {
        return (trimmed.chars().take(max_chars).collect(), None);
    };
    let chars_before = trimmed[..at].chars().count();
    let skip = chars_before.saturating_sub(before);
    let prefix = if skip > 0 { "…" } else { "" };
    let body: String = trimmed.chars().skip(skip).take(max_chars).collect();
    let local = trimmed[..at].chars().skip(skip).map(char::len_utf8).sum::<usize>();
    let start = prefix.len() + local;
    let end = (start + query.len()).min(prefix.len() + body.len());
    (format!("{prefix}{body}"), Some(start..end))
}

enum Row {
    File { path: SharedString, matched: Vec<u32> },
    Line { hit: Hit },
}

const LIMIT: usize = 300;

pub struct Palette {
    mode: PaletteMode,
    repo: PathBuf,
    index: Arc<RepoIndex>,
    input: Entity<TextInput>,
    rows: Vec<Row>,
    selection: Selection,
    scroll: UniformListScrollHandle,
    cancel: Arc<AtomicBool>,
    searching: bool,
    generation: u64,
    query: String,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Palette {
    pub fn new(
        mode: PaletteMode,
        repo: PathBuf,
        index: Arc<RepoIndex>,
        cx: &mut Context<Self>,
    ) -> Palette {
        let placeholder = match mode {
            PaletteMode::Files => "ファイル名で探す",
            PaletteMode::Content => "リポジトリ全体から文字列を探す",
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        let subscription = cx.subscribe(&input, |this, _, _: &InputChanged, cx| this.refresh(cx));
        let mut palette = Palette {
            mode,
            repo,
            index,
            input,
            rows: Vec::new(),
            selection: Selection::default(),
            scroll: UniformListScrollHandle::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            searching: false,
            generation: 0,
            query: String::new(),
            _subscription: subscription,
        };
        palette.refresh(cx);
        palette
    }

    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_content(query.to_string(), cx));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).content().to_string();
        self.query = query.clone();
        match self.mode {
            PaletteMode::Files => {
                self.rows = fuzzy_files(&query, &self.index.files, LIMIT)
                    .into_iter()
                    .map(|(i, matched)| Row::File {
                        path: self.index.files[i].path.clone().into(),
                        matched,
                    })
                    .collect();
                self.after_results(cx);
            }
            PaletteMode::Content => self.search(query, cx),
        }
    }

    fn after_results(&mut self, cx: &mut Context<Self>) {
        self.selection.set_len(self.rows.len());
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.preview(cx);
        cx.notify();
    }

    fn search(&mut self, query: String, cx: &mut Context<Self>) {
        // 前回の検索を止める
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.generation += 1;
        let generation = self.generation;
        if query.is_empty() {
            self.rows.clear();
            self.searching = false;
            self.after_results(cx);
            return;
        }
        self.searching = true;
        cx.notify();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let hits = cx
                .background_spawn(async move { search_content(&repo, &query, LIMIT, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.rows = hits.into_iter().map(|hit| Row::Line { hit }).collect();
                    this.searching = false;
                    this.after_results(cx);
                }
            });
        })
        .detach();
    }

    fn preview(&self, cx: &mut Context<Self>) {
        let path_of = |r: &Row| match r {
            Row::File { path, .. } => path.to_string(),
            Row::Line { hit } => hit.path.clone(),
        };
        // 全文検索では、一致したファイルをすべて星図で光らせる
        let mut paths: Vec<String> = Vec::new();
        if let Some(selected) = self.selection.current().and_then(|i| self.rows.get(i)) {
            paths.push(path_of(selected));
        }
        if self.mode == PaletteMode::Content {
            for row in self.rows.iter().take(120) {
                let p = path_of(row);
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
        cx.emit(PaletteEvent::Preview(paths));
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.next();
        self.moved(cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.prev();
        self.moved(cx);
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.scroll.scroll_to_item(self.selection.index(), ScrollStrategy::Center);
        self.preview(cx);
        cx.notify();
    }

    fn open_row(&self, i: usize, cx: &mut Context<Self>) {
        match self.rows.get(i) {
            Some(Row::File { path, .. }) => cx.emit(PaletteEvent::Open(path.to_string(), None)),
            Some(Row::Line { hit }) => {
                cx.emit(PaletteEvent::Open(hit.path.clone(), Some(hit.line)))
            }
            None => {}
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selection.current() {
            self.open_row(i, cx);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        cx.emit(PaletteEvent::Dismiss);
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let selected = self.selection.current();
        range
            .map(|i| {
                let active = selected == Some(i);
                let base = div()
                    .id(("palette-row", i))
                    .h(px(34.))
                    .w_full()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .font_family(theme::CODE_FONT)
                    .when(active, |d| d.bg(theme::alpha(theme::violet(), 0.16)))
                    .hover(|h| h.bg(theme::alpha(theme::violet(), 0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_row(i, cx)));
                match &self.rows[i] {
                    Row::File { path, matched } => {
                        let highlights: Vec<_> = path
                            .char_indices()
                            .enumerate()
                            .filter(|(ci, _)| matched.contains(&(*ci as u32)))
                            .map(|(_, (b, c))| {
                                (
                                    b..b + c.len_utf8(),
                                    gpui::HighlightStyle {
                                        color: Some(theme::teal().into()),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect();
                        base.text_color(theme::text())
                            .whitespace_nowrap()
                            .child(gpui::StyledText::new(path.clone()).with_highlights(highlights))
                            .into_any_element()
                    }
                    Row::Line { hit } => {
                        let (snippet, range) = snippet_around(&hit.text, &self.query, 34, 90);
                        let highlight = gpui::HighlightStyle {
                            color: Some(theme::white().into()),
                            background_color: Some(theme::alpha(theme::teal(), 0.32)),
                            ..Default::default()
                        };
                        base.whitespace_nowrap()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(236.))
                                    .overflow_hidden()
                                    .text_color(theme::text_muted())
                                    .child(format!("{}:{}", hit.path, hit.line + 1)),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_color(theme::tok_plain())
                                    .child(
                                        gpui::StyledText::new(snippet)
                                            .with_highlights(range.map(|r| (r, highlight))),
                                    ),
                            )
                            .into_any_element()
                    }
                }
            })
            .collect()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.rows.is_empty();
        let has_query = !self.input.read(cx).content().is_empty();
        let label = match self.mode {
            PaletteMode::Files => "ファイル",
            PaletteMode::Content => "全文",
        };
        let status = if self.searching {
            "探しています…".to_string()
        } else {
            format!("{} 件", self.rows.len())
        };
        div()
            .key_context("Palette")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(760.))
            .max_h(px(480.))
            .flex()
            .flex_col()
            .bg(theme::panel_raised())
            .border_1()
            .border_color(theme::hairline_strong())
            .rounded(px(12.))
            .overflow_hidden()
            .shadow_2xl()
            .child(
                div()
                    .flex_none()
                    .h(px(56.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(theme::hairline())
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(theme::violet())
                            .child(label),
                    )
                    .child(div().flex_1().child(self.input.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(status),
                    ),
            )
            .child(if empty {
                div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(if !has_query {
                        "入力すると、リポジトリ全体を検索します"
                    } else if self.searching {
                        "探しています…"
                    } else {
                        "見つかりませんでした"
                    })
                    .into_any_element()
            } else {
                uniform_list(
                    "palette-results",
                    self.rows.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(self.scroll.clone())
                .h(px(34. * 12.))
                .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_keeps_the_match_visible_and_marks_it() {
        let long = format!("{}needle tail", "x".repeat(100));
        let (text, range) = snippet_around(&long, "NEEDLE", 10, 40);
        let range = range.unwrap();
        assert!(text.starts_with('…'));
        assert_eq!(&text[range], "needle");
        let (text, range) = snippet_around("  短い needle です", "needle", 10, 40);
        assert_eq!(text, "短い needle です");
        assert_eq!(&text[range.unwrap()], "needle");
        assert_eq!(snippet_around("abc", "zzz", 10, 40), ("abc".to_string(), None));
    }

    #[test]
    fn selection_wraps_and_resets_on_new_results() {
        let mut s = Selection::default();
        s.set_len(3);
        s.prev();
        assert_eq!(s.index(), 2);
        s.next();
        assert_eq!(s.index(), 0);
        s.next();
        s.set_len(1);
        assert_eq!(s.index(), 0);
        s.set_len(0);
        assert_eq!(s.current(), None);
    }
}
