//! 左のファイルツリー。いま読んでいるファイルを常に見える位置に置く。
use crate::theme;
use ccv_core::model::RepoIndex;
use ccv_core::tree::{FileTree, Row};
use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, FontWeight, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

actions!(file_tree, [Up, Down, Collapse, Expand, Activate]);

pub const ROW_HEIGHT: f32 = 24.0;
const INDENT: f32 = 12.0;
const LEFT: f32 = 12.0;

pub enum TreeEvent {
    Open(String),
    /// 矢印でたどったファイル。下見のタブで開く
    Preview(String),
}

pub struct TreeView {
    tree: Arc<FileTree>,
    index: Arc<RepoIndex>,
    expanded: HashSet<String>,
    rows: Vec<Row>,
    active: Option<String>,
    cursor: Option<usize>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// ファイルの添字から星座の番号
    hues: Vec<Option<usize>>,
    /// ファイルの添字からツアーの停留所の番号（1 始まり）
    stops: HashMap<usize, usize>,
}

impl EventEmitter<TreeEvent> for TreeView {}

impl Focusable for TreeView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TreeView {
    pub fn new(index: Arc<RepoIndex>, cx: &mut Context<Self>) -> TreeView {
        let tree = Arc::new(FileTree::build(index.files.iter().map(|f| f.path.as_str())));
        let expanded = tree.initial_expanded();
        let rows = tree.rows(&expanded);
        TreeView {
            tree,
            hues: vec![None; index.files.len()],
            index,
            expanded,
            rows,
            active: None,
            cursor: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            stops: HashMap::new(),
        }
    }

    pub fn set_marks(&mut self, hues: Vec<Option<usize>>, stops: HashMap<usize, usize>, cx: &mut Context<Self>) {
        self.hues = hues;
        self.stops = stops;
        cx.notify();
    }

    /// 開いたファイルまでフォルダーを開き、その行を見える位置へ送る。
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() == Some(path) {
            return;
        }
        self.active = Some(path.to_string());
        let mut changed = false;
        for dir in self.tree.ancestors(path) {
            changed |= self.expanded.insert(dir);
        }
        if changed {
            self.rows = self.tree.rows(&self.expanded);
        }
        if let Some(i) = self.rows.iter().position(|r| self.tree.nodes[r.node].path == path) {
            self.cursor = Some(i);
            self.scroll.scroll_to_item(i, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub fn clear_active(&mut self, cx: &mut Context<Self>) {
        self.active = None;
        cx.notify();
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.rows = self.tree.rows(&self.expanded);
        self.cursor = None;
        cx.notify();
    }

    fn toggle(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(r) = self.rows.get(row) else { return };
        let node = &self.tree.nodes[r.node];
        if node.is_dir() {
            if !self.expanded.remove(&node.path) {
                self.expanded.insert(node.path.clone());
            }
            self.rows = self.tree.rows(&self.expanded);
        } else {
            cx.emit(TreeEvent::Open(node.path.clone()));
        }
        self.cursor = Some(row);
        cx.notify();
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let current = self
            .cursor
            .or_else(|| {
                let a = self.active.as_ref()?;
                self.rows.iter().position(|r| &self.tree.nodes[r.node].path == a)
            })
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        self.cursor = Some(next);
        self.scroll.scroll_to_item(next, ScrollStrategy::Top);
        // ファイルは選んだ時点で下見する。矢印だけで中身を覗けるように
        let node = &self.tree.nodes[self.rows[next].node];
        if !node.is_dir() {
            cx.emit(TreeEvent::Preview(node.path.clone()));
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(-1, cx);
    }

    fn on_down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(1, cx);
    }

    fn on_collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.cursor else { return };
        let r = self.rows[c];
        let node = &self.tree.nodes[r.node];
        if node.is_dir() && self.expanded.contains(&node.path) {
            self.toggle(c, cx);
        } else if r.depth > 0 {
            // 親のフォルダーへ戻る
            if let Some(parent) = self.rows[..c].iter().rposition(|p| p.depth < r.depth) {
                self.cursor = Some(parent);
                self.scroll.scroll_to_item(parent, ScrollStrategy::Top);
                cx.notify();
            }
        }
    }

    fn on_expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.cursor else { return };
        let node = &self.tree.nodes[self.rows[c].node];
        if node.is_dir() && !self.expanded.contains(&node.path) {
            self.toggle(c, cx);
        }
    }

    fn on_activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = self.cursor {
            self.toggle(c, cx);
        }
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let focused = self.focus.is_focused(window);
        range
            .map(|i| {
                let row = self.rows[i];
                let node = &self.tree.nodes[row.node];
                let is_dir = node.is_dir();
                let open = is_dir && self.expanded.contains(&node.path);
                let active = !is_dir && self.active.as_deref() == Some(node.path.as_str());
                let cursor = focused && self.cursor == Some(i);
                let hue = node.file.and_then(|f| self.hues.get(f).copied().flatten());
                let stop = node.file.and_then(|f| self.stops.get(&f).copied());
                let lines = node.file.map(|f| self.index.files[f].lines).unwrap_or(0);
                let name: SharedString = node.name.clone().into();
                let indent = LEFT + row.depth as f32 * INDENT;
                div()
                    .id(("tree-row", i))
                    .relative()
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .pl(px(indent))
                    .pr(px(10.))
                    .gap(px(7.))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(active, |d| d.bg(theme::alpha(theme::violet(), 0.16)))
                    .when(!active, |d| d.hover(|h| h.bg(theme::alpha(theme::white(), 0.04))))
                    .when(cursor, |d| d.border_1().border_color(theme::alpha(theme::violet(), 0.55)))
                    // 階層の線。深いファイルでも、どのフォルダーの中かを追える
                    .children((0..row.depth).map(|d| {
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(LEFT + d as f32 * INDENT + 4.0))
                            .w(px(1.))
                            .bg(theme::alpha(theme::hairline_strong(), 0.55))
                    }))
                    .when(active, |d| {
                        d.child(div().absolute().left_0().top_0().bottom_0().w(px(2.)).bg(theme::teal()))
                    })
                    .child(
                        div()
                            .w(px(9.))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .text_size(px(8.))
                            .text_color(theme::alpha(theme::text_muted(), 0.85))
                            .when(is_dir, |d| d.child(if open { "▼" } else { "▶" }))
                            .when(!is_dir, |d| {
                                d.child(
                                    div().size(px(6.)).rounded_full().bg(match hue {
                                        Some(h) => theme::alpha(theme::hue(h), 0.95),
                                        None => theme::alpha(theme::text_muted(), 0.35),
                                    }),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(if active {
                                theme::white()
                            } else if is_dir {
                                theme::text()
                            } else {
                                theme::tok_plain()
                            })
                            .when(is_dir, |d| d.font_weight(FontWeight::MEDIUM))
                            .when(active, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(name),
                    )
                    .when_some(stop, |d, n| {
                        d.child(
                            div()
                                .flex_none()
                                .h(px(16.))
                                .min_w(px(16.))
                                .px(px(4.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(theme::alpha(theme::teal(), 0.6))
                                .text_size(px(9.5))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::teal())
                                .child(n.to_string()),
                        )
                    })
                    .when(active && stop.is_none() && lines > 0, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(10.5))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::text_muted())
                                .child(crate::code::group_digits(lines as u64)),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus);
                        this.toggle(i, cx);
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for TreeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("FileTree")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_collapse))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_activate))
            .size_full()
            .flex()
            .flex_col()
            .child(
                uniform_list(
                    "file-tree",
                    self.rows.len(),
                    cx.processor(|this, range: Range<usize>, window, cx| this.render_rows(range, window, cx)),
                )
                .track_scroll(self.scroll.clone())
                .flex_1()
                .pb(px(12.)),
            )
    }
}
