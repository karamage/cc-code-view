//! 設計図。6 つの観点で書かれた章を、図と解説の文書として読む。
//!
//! 章は観点ごとに Claude が書き、書けた章から順に現れる。図は Mermaid を手元で配置して描き、
//! 要素を押すとそのファイルを開く。
use crate::ai_task::run_ai;
use crate::code::group_digits;
use crate::diagram_view::{DiagramEvent, DiagramView, GpuiMeasure};
use crate::ja_text::{paragraph, rich_paragraph};
use crate::model_picker::model_picker;
use crate::state::{AppState, ClaudeStatus};
use crate::theme;
use crate::ui::{button_primary, button_secondary, kbd, text_button};
use ccv_ai::blueprint::{ChapterContext, write_chapter};
use ccv_ai::runner::{RunError, RunEvent};
use ccv_core::diagram::{self, Kind};
use ccv_core::model::{Analysis, Chapter, ChapterKind, RepoIndex};
use gpui::{
    AnimationExt, Context, Entity, EventEmitter, FontWeight, Hsla, PathBuilder, Render,
    ScrollHandle, SharedString, Subscription, Window, canvas, div, point, prelude::*, px,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 同時に動かす claude の数。多すぎると利用枠の上限に当たりやすい
const PARALLEL: usize = 3;
const CONTENT_MAX: f32 = 880.0;

pub enum BlueprintEvent {
    Open {
        path: String,
        line: u32,
        band: Option<(u32, u32)>,
    },
    /// 図の要素に触れている。星図でそのファイルを灯す
    Hover(Option<String>),
    /// 生成中の claude が読んでいるファイルの絶対パス
    Reading(String),
}

#[derive(Clone, Debug, PartialEq)]
enum SlotState {
    Idle,
    Queued,
    Running { activity: String, started: Instant },
    Failed(String),
}

struct Slot {
    kind: ChapterKind,
    chapter: Option<Chapter>,
    state: SlotState,
    diagrams: Vec<Option<Entity<DiagramView>>>,
    /// 図を解析できなかった理由。保存済みの図は検証済みなので、通常は空
    broken: Vec<Option<String>>,
}

pub struct BlueprintView {
    repo: PathBuf,
    name: SharedString,
    index: Arc<RepoIndex>,
    analysis: Option<Analysis>,
    slots: Vec<Slot>,
    scroll: ScrollHandle,
    width: f32,
    cancel: Arc<AtomicBool>,
    context: Option<Arc<ChapterContext>>,
    /// 拡大して見ている図（章、図）
    focus: Option<(usize, usize)>,
    hovered_file: Option<String>,
    ticking: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<BlueprintEvent> for BlueprintView {}

impl Drop for BlueprintView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 章ごとの色。星座の色と同じ並びから、隣り合う章が似ないように選ぶ。
pub fn chapter_hue(kind: ChapterKind) -> gpui::Rgba {
    theme::hue([0, 1, 2, 3, 4, 5][kind.number() - 1])
}

/// UNIX 時刻を「11月14日」の形にする（UTC）。
fn short_date(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let days = (secs / 86_400) as i64;
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{m}月{d}日")
}

impl BlueprintView {
    pub fn new(
        repo: PathBuf,
        index: Arc<RepoIndex>,
        analysis: Option<Analysis>,
        cx: &mut Context<Self>,
    ) -> BlueprintView {
        let store = cx.global::<AppState>().store.clone();
        let name = repo
            .file_name()
            .map_or_else(|| repo.display().to_string(), |n| n.to_string_lossy().into_owned());
        let slots = ChapterKind::ALL
            .iter()
            .map(|&kind| {
                let chapter = store.load_chapter(&repo, kind);
                Slot {
                    kind,
                    chapter,
                    state: SlotState::Idle,
                    diagrams: Vec::new(),
                    broken: Vec::new(),
                }
            })
            .collect();
        BlueprintView {
            repo,
            name: name.into(),
            index,
            analysis,
            slots,
            scroll: ScrollHandle::new(),
            width: 900.0,
            cancel: Default::default(),
            context: None,
            focus: None,
            hovered_file: None,
            ticking: false,
            _subscriptions: Vec::new(),
        }
    }

    pub fn set_width(&mut self, width: f32) {
        self.width = width;
    }

    pub fn set_analysis(&mut self, analysis: Option<Analysis>) {
        self.analysis = analysis;
        self.context = None;
    }

    pub fn ready_count(&self) -> usize {
        self.slots.iter().filter(|s| s.chapter.is_some()).count()
    }

    pub fn busy(&self) -> bool {
        self.slots.iter().any(|s| matches!(s.state, SlotState::Running { .. } | SlotState::Queued))
    }

    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    pub fn hovered_file(&self) -> Option<&str> {
        self.hovered_file.as_deref()
    }

    /// 目次に出す章の状態。
    pub fn toc(&self) -> Vec<(ChapterKind, TocState)> {
        self.slots
            .iter()
            .map(|s| {
                let state = match (&s.state, &s.chapter) {
                    (SlotState::Running { .. }, _) => TocState::Running,
                    (SlotState::Queued, _) => TocState::Queued,
                    (SlotState::Failed(_), None) => TocState::Failed,
                    (_, Some(c)) if c.head != self.index.head => TocState::Stale,
                    (_, Some(_)) => TocState::Ready,
                    (_, None) => TocState::Missing,
                };
                (s.kind, state)
            })
            .collect()
    }

    /// 画面の上端に来ている章。
    pub fn active_chapter(&self) -> Option<usize> {
        let top = self.scroll.top_item();
        // 先頭は表紙。少し読み進めたら最初の章とみなす
        if top == 0 {
            let offset = -f32::from(self.scroll.offset().y);
            let hero = self.scroll.bounds_for_item(0).map_or(0.0, |b| f32::from(b.size.height));
            return (offset > hero * 0.6).then_some(0);
        }
        Some((top - 1).min(self.slots.len() - 1))
    }

    pub fn scroll_to(&mut self, chapter: usize, cx: &mut Context<Self>) {
        self.scroll.scroll_to_top_of_item(chapter + 1);
        cx.notify();
    }

    /// 撮影用。今の位置からさらに下へ送る。
    pub fn scroll_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        let o = self.scroll.offset();
        self.scroll.set_offset(point(o.x, o.y - px(dy)));
        cx.notify();
    }

    pub fn close_focus(&mut self, cx: &mut Context<Self>) {
        if let Some((c, d)) = self.focus.take()
            && let Some(Some(view)) = self.slots.get(c).and_then(|s| s.diagrams.get(d))
        {
            view.update(cx, |v, cx| v.set_expanded(false, cx));
        }
        cx.notify();
    }

    pub fn open_focus(&mut self, chapter: usize, diagram: usize, cx: &mut Context<Self>) {
        if let Some(Some(view)) = self.slots.get(chapter).and_then(|s| s.diagrams.get(diagram)) {
            view.update(cx, |v, cx| v.set_expanded(true, cx));
            self.focus = Some((chapter, diagram));
            cx.notify();
        }
    }

    /// 撮影のため、書いている途中の見た目を作る。
    pub fn demo_running(&mut self, cx: &mut Context<Self>) {
        let reading: Vec<String> = self
            .index
            .files
            .iter()
            .filter(|f| !f.binary)
            .map(|f| f.path.clone())
            .take(40)
            .collect();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            match i {
                2 => {
                    slot.chapter = None;
                    slot.state = SlotState::Running {
                        activity: reading.get(7).cloned().unwrap_or_default(),
                        started: Instant::now() - Duration::from_secs(96),
                    };
                }
                4 => {
                    slot.chapter = None;
                    slot.state = SlotState::Running {
                        activity: reading.get(19).cloned().unwrap_or_default(),
                        started: Instant::now() - Duration::from_secs(41),
                    };
                }
                5 => {
                    slot.chapter = None;
                    slot.state = SlotState::Queued;
                }
                _ => {}
            }
        }
        cx.notify();
    }

    fn context(&mut self) -> Arc<ChapterContext> {
        if let Some(c) = &self.context {
            return c.clone();
        }
        let c = Arc::new(ChapterContext::new(&self.index, self.analysis.clone()));
        self.context = Some(c.clone());
        c
    }

    /// 章を順番待ちに入れて、空きがあれば書き始める。
    pub fn generate(&mut self, kinds: &[ChapterKind], cx: &mut Context<Self>) {
        if cx.global::<AppState>().claude_status != ClaudeStatus::Ready {
            return;
        }
        if !self.busy() {
            self.cancel = Arc::new(AtomicBool::new(false));
        }
        for slot in &mut self.slots {
            if kinds.contains(&slot.kind) && !matches!(slot.state, SlotState::Running { .. }) {
                slot.state = SlotState::Queued;
            }
        }
        self.pump(cx);
        cx.notify();
    }

    pub fn cancel_all(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        for slot in &mut self.slots {
            if slot.state == SlotState::Queued {
                slot.state = SlotState::Idle;
            }
        }
        cx.notify();
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        let running =
            self.slots.iter().filter(|s| matches!(s.state, SlotState::Running { .. })).count();
        let free = PARALLEL.saturating_sub(running);
        let next: Vec<usize> = (0..self.slots.len())
            .filter(|i| self.slots[*i].state == SlotState::Queued)
            .take(free)
            .collect();
        if next.is_empty() {
            return;
        }
        let context = self.context();
        let app = cx.global::<AppState>();
        let (cfg, model, store) = (app.claude.clone(), app.settings.last_model, app.store.clone());
        for i in next {
            let kind = self.slots[i].kind;
            self.slots[i].state =
                SlotState::Running { activity: String::new(), started: Instant::now() };
            let (cfg, repo, index, context, cancel, store) = (
                cfg.clone(),
                self.repo.clone(),
                self.index.clone(),
                context.clone(),
                self.cancel.clone(),
                store.clone(),
            );
            run_ai(
                cx,
                move |on_event| {
                    write_chapter(&cfg, &repo, &index, &context, kind, model, cancel, on_event)
                },
                move |this: &mut BlueprintView, e, cx| {
                    let label = match &e {
                        RunEvent::Reading(path) => {
                            cx.emit(BlueprintEvent::Reading(path.clone()));
                            std::path::Path::new(path)
                                .strip_prefix(&this.repo)
                                .map_or_else(|_| path.clone(), |p| p.display().to_string())
                        }
                        RunEvent::Tool(name) => format!("{name} で調べています"),
                    };
                    if let Some(SlotState::Running { activity, .. }) =
                        this.slots.get_mut(i).map(|s| &mut s.state)
                    {
                        *activity = label;
                    }
                },
                move |this: &mut BlueprintView, result, cx| {
                    let slot = &mut this.slots[i];
                    match result {
                        Ok(chapter) => {
                            if let Err(e) = store.save_chapter(&this.repo, &chapter) {
                                eprintln!("設計図の章を保存できませんでした: {e}");
                            }
                            slot.chapter = Some(chapter);
                            slot.diagrams.clear();
                            slot.broken.clear();
                            slot.state = SlotState::Idle;
                        }
                        Err(RunError::Cancelled) => slot.state = SlotState::Idle,
                        Err(e) => {
                            eprintln!("設計図の章を書けませんでした: {e}");
                            slot.state = SlotState::Failed(e.to_string());
                        }
                    }
                    this.pump(cx);
                    cx.notify();
                },
            );
        }
    }

    /// 図を配置する。描くたびにではなく、章が届いたときに一度だけ行う。
    fn ensure_diagrams(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 撮影では演出の途中を写さない
        let reduce_motion =
            cx.global::<AppState>().reduce_motion || cx.global::<AppState>().demo.is_some();
        let mut measure: Option<GpuiMeasure> = None;
        let width = self.content_width();
        for ci in 0..self.slots.len() {
            let Some(chapter) = &self.slots[ci].chapter else { continue };
            if self.slots[ci].diagrams.len() == chapter.diagrams.len() {
                continue;
            }
            let specs = chapter.diagrams.clone();
            let kind = chapter.kind;
            let m = measure.get_or_insert_with(|| GpuiMeasure::new(window));
            let mut views = Vec::new();
            let mut broken = Vec::new();
            for (di, spec) in specs.into_iter().enumerate() {
                match diagram::parse(&spec.mermaid) {
                    Ok(d) => {
                        let scene = Arc::new(diagram::layout_to_fit(&d, m, width - 40.0, 600.0));
                        let mut links = HashMap::new();
                        for l in &spec.links {
                            if let Some(i) = scene
                                .nodes
                                .iter()
                                .position(|n| n.id == l.id && n.mirror_of.is_none())
                            {
                                links.insert(i, (l.path.clone(), l.line));
                            }
                        }
                        let flow = kind == ChapterKind::DataFlow || d.kind() == Kind::Sequence;
                        // 撮影で、触れている状態を写せるようにする
                        let demo_hover = std::env::var("CCV_HOVER").ok().filter(|_| cx.global::<AppState>().demo.is_some());
                        let view = cx.new(|_| {
                            let mut v = DiagramView::new(scene, links, flow, reduce_motion);
                            if let Some(id) = &demo_hover {
                                v.demo_hover(id);
                            }
                            v
                        });
                        let sub =
                            cx.subscribe(&view, move |this, _, e: &DiagramEvent, cx| match e {
                                DiagramEvent::Open { path, line } => {
                                    let line = *line;
                                    cx.emit(BlueprintEvent::Open {
                                        path: path.clone(),
                                        line: line.saturating_sub(1),
                                        band: (line > 0).then_some((line, line)),
                                    });
                                }
                                DiagramEvent::Hover(path) => {
                                    this.hovered_file = path.clone();
                                    cx.emit(BlueprintEvent::Hover(path.clone()));
                                    cx.notify();
                                }
                                DiagramEvent::Expand => this.open_focus(ci, di, cx),
                            });
                        self._subscriptions.push(sub);
                        views.push(Some(view));
                        broken.push(None);
                    }
                    Err(e) => {
                        views.push(None);
                        broken.push(Some(e.to_string()));
                    }
                }
            }
            self.slots[ci].diagrams = views;
            self.slots[ci].broken = broken;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TocState {
    Ready,
    Stale,
    Running,
    Queued,
    Failed,
    Missing,
}

/// 章の印。観点の形を線で描く。書体に頼らず、どの環境でも同じ形にする。
pub fn chapter_glyph(kind: ChapterKind, color: Hsla, size_px: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let s = size_px / 24.0;
            let p = |a: f32, b: f32| point(px(x + a * s), px(y + b * s));
            let mut path = PathBuilder::stroke(px(1.4));
            match kind {
                ChapterKind::Architecture => {
                    // 重なった層
                    for (i, top) in [4.0f32, 10.0, 16.0].iter().enumerate() {
                        let inset = i as f32 * 0.0;
                        path.move_to(p(3.0 + inset, *top));
                        path.line_to(p(12.0, top - 3.0));
                        path.line_to(p(21.0 - inset, *top));
                        path.line_to(p(12.0, top + 3.0));
                        path.close();
                    }
                }
                ChapterKind::DataStructures => {
                    path.add_polygon(
                        &[p(3.0, 4.0), p(21.0, 4.0), p(21.0, 20.0), p(3.0, 20.0)],
                        true,
                    );
                    path.move_to(p(3.0, 9.0));
                    path.line_to(p(21.0, 9.0));
                    path.move_to(p(3.0, 14.5));
                    path.line_to(p(21.0, 14.5));
                    path.move_to(p(10.0, 9.0));
                    path.line_to(p(10.0, 20.0));
                }
                ChapterKind::DataFlow => {
                    path.move_to(p(2.0, 16.0));
                    path.cubic_bezier_to(p(12.0, 8.0), p(6.0, 16.0), p(8.0, 8.0));
                    path.cubic_bezier_to(p(22.0, 8.0), p(16.0, 8.0), p(18.0, 16.0));
                    path.move_to(p(19.0, 5.0));
                    path.line_to(p(22.0, 8.0));
                    path.line_to(p(19.0, 11.0));
                }
                ChapterKind::Domain => {
                    let pts: Vec<_> = (0..6)
                        .map(|i| {
                            let a =
                                std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
                            p(12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin())
                        })
                        .collect();
                    path.add_polygon(&pts, true);
                    path.move_to(p(12.0, 3.0));
                    path.line_to(p(12.0, 12.0));
                    path.line_to(p(19.8, 16.5));
                    path.move_to(p(12.0, 12.0));
                    path.line_to(p(4.2, 16.5));
                }
                ChapterKind::UseCases => {
                    path.add_polygon(&[p(5.0, 4.0), p(9.0, 4.0), p(9.0, 8.0), p(5.0, 8.0)], true);
                    path.move_to(p(7.0, 9.0));
                    path.line_to(p(7.0, 15.0));
                    path.move_to(p(3.5, 11.5));
                    path.line_to(p(10.5, 11.5));
                    path.move_to(p(7.0, 15.0));
                    path.line_to(p(4.0, 20.0));
                    path.move_to(p(7.0, 15.0));
                    path.line_to(p(10.0, 20.0));
                    path.move_to(p(13.0, 12.0));
                    path.line_to(p(21.0, 12.0));
                    path.move_to(p(18.0, 9.0));
                    path.line_to(p(21.0, 12.0));
                    path.line_to(p(18.0, 15.0));
                }
                ChapterKind::Modules => {
                    for (a, b) in [(3.0, 3.0), (13.0, 3.0), (3.0, 13.0), (13.0, 13.0)] {
                        path.add_polygon(
                            &[p(a, b), p(a + 8.0, b), p(a + 8.0, b + 8.0), p(a, b + 8.0)],
                            true,
                        );
                    }
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(size_px))
    .flex_none()
}

fn pulse(color: gpui::Rgba) -> impl IntoElement {
    div().flex_none().size(px(7.)).rounded_full().bg(color).with_animation(
        "blueprint-pulse",
        gpui::Animation::new(Duration::from_millis(1400))
            .repeat()
            .with_easing(gpui::pulsating_between(0.35, 1.0)),
        |d, t| d.opacity(t),
    )
}

fn eyebrow(text: impl Into<SharedString>, color: impl Into<Hsla>) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .font_family(theme::CODE_FONT)
        .text_color(color.into())
        .child(text.into())
}

// 画面の部品
impl BlueprintView {
    fn content_width(&self) -> f32 {
        (self.width - self.gutter() * 2.0).clamp(320.0, CONTENT_MAX)
    }

    fn gutter(&self) -> f32 {
        if self.width < 760.0 { 24.0 } else { 44.0 }
    }

    fn render_hero(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let width = self.content_width();
        let ready = self.ready_count();
        let total = self.slots.len();
        let busy = self.busy();
        let app = cx.global::<AppState>();
        let current = app.settings.last_model;
        let claude = app.claude_status.clone();
        let diagrams: usize =
            self.slots.iter().filter_map(|s| s.chapter.as_ref()).map(|c| c.diagrams.len()).sum();
        let linked: usize = self
            .slots
            .iter()
            .filter_map(|s| s.chapter.as_ref())
            .flat_map(|c| c.diagrams.iter().flat_map(|d| d.links.iter().map(|l| l.path.as_str())))
            .collect::<std::collections::HashSet<_>>()
            .len();
        let missing: Vec<ChapterKind> =
            self.slots.iter().filter(|s| s.chapter.is_none()).map(|s| s.kind).collect();
        let stale: Vec<ChapterKind> = self
            .slots
            .iter()
            .filter(|s| s.chapter.as_ref().is_some_and(|c| c.head != self.index.head))
            .map(|s| s.kind)
            .collect();
        let lede = match &self.analysis {
            Some(a) => a.overview.clone(),
            None => "全体アーキテクチャ、データ構造、データフロー、ドメイン、ユースケース、画面とモジュール構成。6 つの観点で Claude がコードを読み、図と解説にまとめます。".into(),
        };
        let columns = if width >= 700.0 { 3 } else { 2 };
        let card_w = (width - 12.0 * (columns as f32 - 1.0)) / columns as f32;
        let toc = self.toc();
        let cards: Vec<gpui::AnyElement> = toc
            .iter()
            .enumerate()
            .map(|(i, (kind, state))| self.render_index_card(i, *kind, *state, card_w, window, cx))
            .collect();
        let rows: Vec<gpui::AnyElement> = cards
            .into_iter()
            .collect::<Vec<_>>()
            .chunks_mut(columns)
            .map(|row| {
                div()
                    .flex()
                    .gap(px(12.))
                    .children(
                        row.iter_mut().map(|c| std::mem::replace(c, div().into_any_element())),
                    )
                    .into_any_element()
            })
            .collect();

        let first_model =
            self.slots.iter().filter_map(|s| s.chapter.as_ref()).map(|c| c.model).next();
        let sha = self.index.head.as_ref().map(|h| h.chars().take(7).collect::<String>());
        let action: gpui::AnyElement = match claude {
            ClaudeStatus::Problem(message) => div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().size(px(7.)).rounded_full().bg(theme::danger()))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text_muted())
                        .child(format!("AI 機能を使えません。{message}")),
                )
                .into_any_element(),
            _ if busy => {
                let running = self
                    .slots
                    .iter()
                    .filter(|s| matches!(s.state, SlotState::Running { .. }))
                    .count();
                let queued = self.slots.iter().filter(|s| s.state == SlotState::Queued).count();
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(pulse(theme::teal()))
                    .child(div().text_size(px(13.)).text_color(theme::text()).child(
                        if queued > 0 {
                            format!(
                                "{running} 章を書いています。あと {queued} 章が順番を待っています"
                            )
                        } else {
                            format!("{running} 章を書いています")
                        },
                    ))
                    .child(div().flex_1())
                    .child(
                        button_secondary("blueprint-cancel", "中断")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_all(cx))),
                    )
                    .into_any_element()
            }
            ClaudeStatus::Checking => div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child("Claude Code を確認しています…")
                .into_any_element(),
            ClaudeStatus::Ready => {
                let (label, targets) = if !missing.is_empty() {
                    (
                        if missing.len() == total {
                            "設計図を描く".to_string()
                        } else {
                            format!("残りの {} 章を描く", missing.len())
                        },
                        missing.clone(),
                    )
                } else if !stale.is_empty() {
                    (format!("更新された {} 章を描き直す", stale.len()), stale.clone())
                } else {
                    (String::new(), Vec::new())
                };
                let empty = targets.is_empty();
                let generate_button = (!empty).then(|| {
                    button_primary("blueprint-generate", label)
                        .on_click(cx.listener(move |this, _, _, cx| this.generate(&targets, cx)))
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(12.))
                            .child(model_picker("blueprint-model", current, move |model, _, cx| {
                                AppState::set_model(cx, model);
                                cx.refresh_windows();
                            }))
                            .children(generate_button)
                            .when(empty, |d| {
                                d.child(
                                    button_secondary("blueprint-regenerate", "すべて描き直す")
                                        .on_click(cx.listener(|this, _, _, cx| this.generate(&ChapterKind::ALL, cx))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(theme::text_muted())
                            .child(format!("章ごとに Claude がコードを読み取り専用で読みます。{PARALLEL} 章ずつ並べて書き、書けた章から現れます。{}", current.hint())),
                    )
                    .into_any_element()
            }
        };

        div()
            .pt(px(40.))
            .pb(px(28.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(eyebrow("BLUEPRINT", theme::teal()))
                    .child(div().w(px(18.)).h(px(1.)).bg(theme::alpha(theme::teal(), 0.5)))
                    .child(eyebrow(format!("{ready} / {total} 章"), theme::text_muted())),
            )
            .child(
                div()
                    .mt(px(14.))
                    .text_size(px(34.))
                    .line_height(px(42.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::white())
                    .child(format!("{} の設計図", self.name)),
            )
            .child(div().mt(px(14.)).child(paragraph(
                &lede,
                width.min(720.0),
                15.,
                26.,
                theme::text_muted(),
                window,
            )))
            .child(
                div()
                    .mt(px(18.))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .gap(px(18.))
                    .text_size(px(11.5))
                    .font_family(theme::CODE_FONT)
                    .text_color(theme::text_muted())
                    .when(ready > 0, |d| {
                        d.child(format!("図 {diagrams}")).child(format!(
                            "コードにつながる要素のファイル {}",
                            group_digits(linked as u64)
                        ))
                    })
                    .when_some(first_model, |d, m| d.child(format!("{} が執筆", m.label())))
                    .when_some(sha, |d, sha| d.child(format!("コミット {sha}"))),
            )
            .child(
                div()
                    .mt(px(24.))
                    .p(px(16.))
                    .rounded(px(12.))
                    .bg(theme::alpha(theme::panel_raised(), 0.7))
                    .border_1()
                    .border_color(if busy {
                        theme::alpha(theme::teal(), 0.35)
                    } else {
                        theme::hairline().into()
                    })
                    .child(action),
            )
            .child(div().mt(px(20.)).flex().flex_col().gap(px(12.)).children(rows))
            .into_any_element()
    }

    fn render_index_card(
        &self,
        i: usize,
        kind: ChapterKind,
        state: TocState,
        w: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let hue = chapter_hue(kind);
        let slot = &self.slots[i];
        let ready = slot.chapter.is_some();
        let status: gpui::AnyElement = match (&slot.state, &slot.chapter) {
            (SlotState::Running { activity, started }, _) => div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(pulse(theme::teal()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::teal())
                        .child(if activity.is_empty() {
                            format!("読み始めています {}秒", started.elapsed().as_secs())
                        } else {
                            activity.clone()
                        }),
                )
                .into_any_element(),
            (SlotState::Queued, _) => div()
                .text_size(px(11.5))
                .text_color(theme::text_muted())
                .child("順番を待っています")
                .into_any_element(),
            (SlotState::Failed(m), _) => div()
                .text_size(px(11.5))
                .text_color(theme::danger())
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(m.clone())
                .into_any_element(),
            (_, Some(c)) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.))
                .font_family(theme::CODE_FONT)
                .text_color(theme::text_muted())
                .child(format!("図 {}  要点 {}", c.diagrams.len(), c.points.len()))
                .when(state == TocState::Stale, |d| {
                    d.child(div().text_color(theme::tok_number()).child("更新あり"))
                })
                .into_any_element(),
            (_, None) => div()
                .text_size(px(11.5))
                .text_color(theme::alpha(theme::text_muted(), 0.75))
                .child("まだ描いていません")
                .into_any_element(),
        };
        let headline = slot.chapter.as_ref().map(|c| c.headline.clone());
        div()
            .id(("blueprint-card", i))
            .w(px(w))
            .flex_none()
            .p(px(14.))
            .rounded(px(12.))
            .cursor_pointer()
            .bg(if ready {
                theme::alpha(theme::panel_raised(), 0.85)
            } else {
                theme::alpha(theme::panel_raised(), 0.35)
            })
            .border_1()
            .border_color(if ready { theme::alpha(hue, 0.28) } else { theme::hairline().into() })
            .when(!ready, |d| d.border_dashed())
            .hover(|h| {
                h.border_color(theme::alpha(hue, 0.7)).bg(theme::alpha(theme::panel_raised(), 1.0))
            })
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(eyebrow(format!("{:02}", kind.number()), hue))
                    .child(chapter_glyph(
                        kind,
                        theme::alpha(hue, if ready { 0.95 } else { 0.5 }),
                        22.0,
                    )),
            )
            .child(
                div()
                    .text_size(px(14.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::white())
                    .child(kind.title()),
            )
            .child(paragraph(
                &headline.unwrap_or_else(|| kind.question().to_string()),
                w - 28.0,
                12.,
                18.,
                if ready { theme::text() } else { theme::text_muted() },
                window,
            ))
            .child(div().mt(px(2.)).child(status))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.slots[i].chapter.is_some() {
                    this.scroll_to(i, cx);
                } else if this.slots[i].state == SlotState::Idle
                    || matches!(this.slots[i].state, SlotState::Failed(_))
                {
                    this.generate(&[kind], cx);
                } else {
                    this.scroll_to(i, cx);
                }
            }))
            .into_any_element()
    }

    fn render_chapter(
        &mut self,
        i: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let width = self.content_width();
        let slot = &self.slots[i];
        let kind = slot.kind;
        let hue = chapter_hue(kind);
        let stale = slot.chapter.as_ref().is_some_and(|c| c.head != self.index.head);
        let available = cx.global::<AppState>().claude_status == ClaudeStatus::Ready;
        let header = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(chapter_glyph(kind, hue.into(), 26.0))
                    .child(eyebrow(format!("CHAPTER {:02}", kind.number()), hue))
                    .child(div().flex_1().h(px(1.)).bg(theme::alpha(hue, 0.22))),
            )
            .child(
                div()
                    .mt(px(14.))
                    .text_size(px(26.))
                    .line_height(px(34.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::white())
                    .child(kind.title()),
            )
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(kind.question()),
            );

        let running = match &slot.state {
            SlotState::Running { activity, started } => {
                Some((activity.clone(), started.elapsed().as_secs()))
            }
            _ => None,
        };
        let status_banner = |text: String| {
            div()
                .mt(px(16.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(theme::alpha(theme::teal(), 0.08))
                .border_1()
                .border_color(theme::alpha(theme::teal(), 0.25))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(pulse(theme::teal()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(theme::teal())
                        .child(text),
                )
        };

        let Some(chapter) = slot.chapter.clone() else {
            let failed =
                if let SlotState::Failed(m) = &slot.state { Some(m.clone()) } else { None };
            let queued = slot.state == SlotState::Queued;
            return div()
                .pt(px(56.))
                .pb(px(16.))
                .child(header)
                .child(
                    div()
                        .mt(px(20.))
                        .h(px(180.))
                        .rounded(px(14.))
                        .border_1()
                        .border_dashed()
                        .border_color(theme::alpha(hue, 0.3))
                        .bg(theme::alpha(hue, 0.025))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .child(chapter_glyph(kind, theme::alpha(hue, 0.6), 34.0))
                        .when_some(running.clone(), |d, (activity, secs)| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(pulse(theme::teal()))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(theme::text())
                                            .child(format!("書いています  {secs}秒")),
                                    ),
                            )
                            .child(
                                div()
                                    .max_w(px(460.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(11.5))
                                    .font_family(theme::CODE_FONT)
                                    .text_color(theme::teal())
                                    .child(activity),
                            )
                        })
                        .when(queued, |d| {
                            d.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme::text_muted())
                                    .child("順番を待っています"),
                            )
                        })
                        .when_some(failed, |d, m| {
                            d.child(
                                div()
                                    .max_w(px(520.))
                                    .text_size(px(12.5))
                                    .text_color(theme::danger())
                                    .child(m),
                            )
                        })
                        .when(running.is_none() && !queued && available, |d| {
                            d.child(
                                button_secondary(
                                    SharedString::from(format!("chapter-generate-{i}")),
                                    "この章を描く",
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.generate(&[kind], cx)),
                                ),
                            )
                        }),
                )
                .into_any_element();
        };

        let views = slot.diagrams.clone();
        let broken = slot.broken.clone();
        let focus = self.focus;
        let diagram_cards: Vec<gpui::AnyElement> = chapter
            .diagrams
            .iter()
            .enumerate()
            .map(|(di, spec)| {
                let view = views.get(di).cloned().flatten();
                let expanded = focus == Some((i, di));
                let kind_label = view.as_ref().map_or("図", |v| v.read(cx).scene().kind.label());
                let elements = view.as_ref().map_or(0, |v| {
                    v.read(cx).scene().nodes.iter().filter(|n| n.mirror_of.is_none()).count()
                });
                let height = view.as_ref().map_or(200.0, |v| v.read(cx).natural_height(width));
                let canvas_area: gpui::AnyElement = match (view, expanded) {
                    (Some(_), true) => div()
                        .h(px(height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child("拡大して表示しています")
                        .into_any_element(),
                    (Some(v), false) => div().h(px(height)).child(v).into_any_element(),
                    (None, _) => div()
                        .h(px(120.))
                        .p(px(16.))
                        .text_size(px(12.))
                        .text_color(theme::danger())
                        .child(format!(
                            "この図は表示できません: {}",
                            broken.get(di).cloned().flatten().unwrap_or_default()
                        ))
                        .into_any_element(),
                };
                div()
                    .mt(px(22.))
                    .rounded(px(14.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme::hairline_strong())
                    .bg(theme::panel())
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(11.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(theme::hairline())
                            .child(
                                div()
                                    .px(px(7.))
                                    .py(px(1.))
                                    .rounded(px(5.))
                                    .text_size(px(10.5))
                                    .text_color(hue)
                                    .bg(theme::alpha(hue, 0.12))
                                    .child(kind_label),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::white())
                                    .child(spec.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .font_family(theme::CODE_FONT)
                                    .text_color(theme::text_muted())
                                    .child(format!("要素 {elements}  コード {}", spec.links.len())),
                            ),
                    )
                    .child(canvas_area)
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(11.))
                            .border_t_1()
                            .border_color(theme::hairline())
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .when(!spec.caption.is_empty(), |d| {
                                d.child(rich_paragraph(
                                    &spec.caption,
                                    width - 34.0,
                                    12.5,
                                    20.,
                                    theme::text_muted(),
                                    window,
                                ))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(14.))
                                    .text_size(px(11.))
                                    .text_color(theme::alpha(theme::text_muted(), 0.8))
                                    .when(!spec.links.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(5.))
                                                .child(div().text_color(theme::teal()).child("↗"))
                                                .child("の要素を押すとコードを開く"),
                                        )
                                    })
                                    .child("触れるとつながりが灯る")
                                    .child("⌘ + スクロールで拡大"),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        let two_columns = width >= 760.0;
        let point_w = if two_columns { (width - 20.0) / 2.0 } else { width };
        let points: Vec<gpui::AnyElement> = chapter
            .points
            .iter()
            .enumerate()
            .map(|(pi, p)| {
                let refs = p.refs.clone();
                div()
                    .w(px(point_w))
                    .flex_none()
                    .p(px(16.))
                    .rounded(px(12.))
                    .bg(theme::alpha(theme::panel_raised(), 0.55))
                    .border_1()
                    .border_color(theme::hairline())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .font_family(theme::CODE_FONT)
                                    .text_color(hue)
                                    .child(format!("{:02}", pi + 1)),
                            )
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .line_height(px(21.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::white())
                                    .child(p.heading.clone()),
                            ),
                    )
                    .child(rich_paragraph(
                        &p.body,
                        point_w - 32.0,
                        13.5,
                        23.,
                        theme::text(),
                        window,
                    ))
                    .when(!refs.is_empty(), |d| {
                        d.child(div().mt(px(2.)).flex().flex_wrap().gap(px(6.)).children(
                            refs.into_iter().enumerate().map(|(ri, r)| {
                                let (path, start, end) = (r.path.clone(), r.start_line, r.end_line);
                                let name = r.path.rsplit('/').next().unwrap_or(&r.path).to_string();
                                div()
                                    .id(SharedString::from(format!("ref-{i}-{pi}-{ri}")))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(theme::alpha(theme::teal(), 0.3))
                                    .bg(theme::alpha(theme::teal(), 0.06))
                                    .hover(|h| {
                                        h.bg(theme::alpha(theme::teal(), 0.16))
                                            .border_color(theme::teal())
                                    })
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(11.))
                                    .font_family(theme::CODE_FONT)
                                    .child(div().text_color(theme::teal()).child(name))
                                    .child(div().text_color(theme::text_muted()).child(
                                        if start == end {
                                            format!("{start}")
                                        } else {
                                            format!("{start}–{end}")
                                        },
                                    ))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(BlueprintEvent::Open {
                                            path: path.clone(),
                                            line: start.saturating_sub(1),
                                            band: Some((start, end)),
                                        });
                                    }))
                            }),
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        let mut point_rows: Vec<gpui::AnyElement> = Vec::new();
        let mut points = points.into_iter();
        while let Some(a) = points.next() {
            let b = if two_columns { points.next() } else { None };
            point_rows.push(
                div().flex().gap(px(20.)).items_start().child(a).children(b).into_any_element(),
            );
        }

        let glossary = chapter.glossary.clone();
        let meta =
            format!("{} が執筆  {}", chapter.model.label(), short_date(chapter.generated_at));
        div()
            .pt(px(56.))
            .pb(px(16.))
            .child(header)
            .when_some(running, |d, (activity, secs)| {
                d.child(status_banner(format!("描き直しています  {secs}秒  {activity}")))
            })
            .child(div().mt(px(22.)).pl(px(16.)).border_l_2().border_color(hue).child(paragraph(
                &chapter.headline,
                width - 20.0,
                19.,
                30.,
                theme::white(),
                window,
            )))
            .child(div().mt(px(18.)).child(rich_paragraph(
                &chapter.summary,
                width.min(760.0),
                14.5,
                26.,
                theme::text(),
                window,
            )))
            .children(diagram_cards)
            .child(div().mt(px(30.)).mb(px(12.)).child(eyebrow("要点", theme::text_muted())))
            .child(div().flex().flex_col().gap(px(16.)).children(point_rows))
            .when(!glossary.is_empty(), |d| {
                let term_w = 150.0;
                d.child(div().mt(px(30.)).mb(px(8.)).child(eyebrow("用語", theme::text_muted())))
                    .child(
                        div()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(theme::hairline())
                            .overflow_hidden()
                            .children(glossary.into_iter().enumerate().map(|(ti, t)| {
                                div()
                                    .flex()
                                    .gap(px(16.))
                                    .px(px(16.))
                                    .py(px(10.))
                                    .when(ti > 0, |d| {
                                        d.border_t_1().border_color(theme::hairline())
                                    })
                                    .child(
                                        div()
                                            .w(px(term_w))
                                            .flex_none()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hue)
                                            .child(t.term),
                                    )
                                    .child(rich_paragraph(
                                        &t.meaning,
                                        width - term_w - 66.0,
                                        13.,
                                        21.,
                                        theme::text(),
                                        window,
                                    ))
                            })),
                    )
            })
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .text_size(px(11.))
                    .text_color(theme::alpha(theme::text_muted(), 0.8))
                    .child(meta)
                    .when(stale, |d| {
                        d.child(
                            div()
                                .px(px(6.))
                                .rounded(px(4.))
                                .text_color(theme::tok_number())
                                .bg(theme::alpha(theme::tok_number(), 0.12))
                                .child("書いたあとにコードが更新されています"),
                        )
                    })
                    .child(div().flex_1())
                    .when(
                        available
                            && !matches!(slot.state, SlotState::Running { .. } | SlotState::Queued),
                        |d| {
                            d.child(
                                text_button(
                                    SharedString::from(format!("chapter-redo-{i}")),
                                    "この章を描き直す",
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.generate(&[kind], cx)),
                                ),
                            )
                        },
                    ),
            )
            .into_any_element()
    }

    fn render_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (ci, di) = self.focus?;
        let slot = self.slots.get(ci)?;
        let chapter = slot.chapter.as_ref()?;
        let spec = chapter.diagrams.get(di)?;
        let view = slot.diagrams.get(di)?.clone()?;
        let hue = chapter_hue(slot.kind);
        let kind_label = view.read(cx).scene().kind.label();
        let caption = spec.caption.clone();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .bg(theme::space())
                .occlude()
                .child(
                    div()
                        .flex_none()
                        .px(px(20.))
                        .py(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .border_b_1()
                        .border_color(theme::hairline())
                        .child(chapter_glyph(slot.kind, hue.into(), 20.0))
                        .child(eyebrow(
                            format!("{:02} {}", slot.kind.number(), slot.kind.title()),
                            hue,
                        ))
                        .child(div().w(px(1.)).h(px(14.)).bg(theme::hairline_strong()))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::white())
                                .child(spec.title.clone()),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .rounded(px(4.))
                                .text_size(px(10.5))
                                .text_color(hue)
                                .bg(theme::alpha(hue, 0.12))
                                .child(kind_label),
                        )
                        .child(div().flex_1())
                        .child(
                            text_button("focus-close", "閉じる")
                                .child(kbd("esc"))
                                .on_click(cx.listener(|this, _, _, cx| this.close_focus(cx))),
                        ),
                )
                .child(div().flex_1().min_h_0().child(view))
                .when(!caption.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px(px(20.))
                            .py(px(10.))
                            .border_t_1()
                            .border_color(theme::hairline())
                            .child(rich_paragraph(
                                &caption,
                                (self.width - 40.0).max(200.0),
                                12.5,
                                20.,
                                theme::text_muted(),
                                window,
                            )),
                    )
                })
                .into_any_element(),
        )
    }
}

impl Render for BlueprintView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_diagrams(window, cx);
        if self.busy() && !self.ticking {
            // 経過時間の表示を進める。1 秒ごとで足りる
            self.ticking = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(1000)).await;
                let _ = this.update(cx, |this, cx| {
                    this.ticking = false;
                    cx.notify();
                });
            })
            .detach();
        }
        let gutter = self.gutter();
        let width = self.content_width();
        let hero = self.render_hero(window, cx);
        let chapters: Vec<gpui::AnyElement> =
            (0..self.slots.len()).map(|i| self.render_chapter(i, window, cx)).collect();
        let focus = self.render_focus(window, cx);
        let footer = div()
            .pt(px(48.))
            .pb(px(64.))
            .flex()
            .justify_center()
            .text_size(px(11.))
            .text_color(theme::alpha(theme::text_muted(), 0.7))
            .child("図と解説は Claude がコードを読んで書いたものです。根拠の行を開いて確かめながら読んでください。");
        let column = |child: gpui::AnyElement| {
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(gutter))
                .child(div().w(px(width)).child(child))
        };
        let mut items: Vec<gpui::AnyElement> = Vec::new();
        items.push(column(hero).into_any_element());
        for c in chapters {
            items.push(column(c).into_any_element());
        }
        items.push(column(footer.into_any_element()).into_any_element());
        div()
            .relative()
            .size_full()
            .bg(theme::space())
            .child(
                div()
                    .id("blueprint-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(items),
            )
            .children(focus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_formatted_in_japanese() {
        assert_eq!(short_date(0), "");
        // 2026-11-14 00:00:00 UTC
        assert_eq!(short_date(1_794_614_400), "11月14日");
    }
}
