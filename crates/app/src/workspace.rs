//! リポジトリを開いた画面。星図、コード、ツアー、解析をつなぐ。
use crate::analysis::{AnalysisState, Ev, transition};
use crate::args::DemoState;
use crate::code::{CodeEvent, CodeView, icon_button};
use crate::model_picker::model_picker;
use crate::palette::{Palette, PaletteEvent, PaletteMode};
use crate::starmap::sky::build_sky;
use crate::starmap::{StarMap, StarMapEvent};
use crate::state::{AppState, ClaudeStatus};
use crate::theme;
use crate::tour::{TourPlayer, validity};
use ccv_ai::analyze::{analyze_repo, deep_dive};
use ccv_ai::runner::{RunError, RunEvent};
use ccv_core::grouping::provisional_modules;
use ccv_core::index::build_index;
use ccv_core::model::{Analysis, DeepDive, ModelChoice, Module, RepoIndex};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, Render,
    SharedString, Subscription, Window, actions, div, prelude::*, px,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

actions!(
    workspace,
    [
        OpenFiles,
        OpenSearch,
        TourNext,
        TourPrev,
        TourToggle,
        CloseTop,
        NavBack,
        NavForward,
        StartAnalysis,
        GoHome,
        ZoomIn,
        ZoomOut,
        FitAll,
        ToggleSymbols,
    ]
);

pub enum WorkspaceEvent {
    GoHome,
}

enum DeepState {
    Idle,
    Running { activity: String, started: Instant },
    Failed(String),
}

struct DeepPanel {
    constellation: usize,
    name: SharedString,
    summary: SharedString,
    files: usize,
    lines: u64,
    module: Option<Module>,
    dive: Option<DeepDive>,
    state: DeepState,
}

struct Loaded {
    index: Arc<RepoIndex>,
    file_at: HashMap<String, usize>,
    map: Entity<StarMap>,
    code: Entity<CodeView>,
    modules: Vec<Module>,
    analysis: Option<Analysis>,
    _subscriptions: Vec<Subscription>,
}

pub struct Workspace {
    repo: PathBuf,
    name: SharedString,
    focus: FocusHandle,
    loaded: Option<Loaded>,
    code_open: bool,
    palette: Option<(Entity<Palette>, Subscription)>,
    tour: Option<TourPlayer>,
    state: AnalysisState,
    show_reanalyze: bool,
    deep: Option<DeepPanel>,
    cancel: Arc<AtomicBool>,
    deep_cancel: Arc<AtomicBool>,
    recent_reads: Vec<usize>,
    demo_applied: bool,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // 画面を閉じたら、実行中の claude も止める
        self.cancel.store(true, Ordering::Relaxed);
        self.deep_cancel.store(true, Ordering::Relaxed);
    }
}

/// バックグラウンドで AI 処理を動かし、進捗と結果を受け取る。
fn run_ai<T: Send + 'static>(
    cx: &mut Context<Workspace>,
    work: impl FnOnce(&mut dyn FnMut(RunEvent)) -> Result<T, RunError> + Send + 'static,
    on_event: impl Fn(&mut Workspace, RunEvent, &mut Context<Workspace>) + 'static,
    on_done: impl FnOnce(&mut Workspace, Result<T, RunError>, &mut Context<Workspace>) + 'static,
) {
    let events: Arc<Mutex<Vec<RunEvent>>> = Default::default();
    let result: Arc<Mutex<Option<Result<T, RunError>>>> = Arc::new(Mutex::new(None));
    {
        let (events, result) = (events.clone(), result.clone());
        std::thread::spawn(move || {
            let mut push = |e: RunEvent| events.lock().unwrap().push(e);
            let r = work(&mut push);
            *result.lock().unwrap() = Some(r);
        });
    }
    cx.spawn(async move |this, cx| {
        let mut on_done = Some(on_done);
        loop {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let drained: Vec<RunEvent> = std::mem::take(&mut *events.lock().unwrap());
            let finished = result.lock().unwrap().take();
            let done = finished.is_some();
            let alive = this.update(cx, |this, cx| {
                for e in drained {
                    on_event(this, e, cx);
                }
                if let (Some(r), Some(f)) = (finished, on_done.take()) {
                    f(this, r, cx);
                }
                // 経過時間の表示を進める
                cx.notify();
            });
            if done || alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

impl Workspace {
    pub fn new(repo: PathBuf, cx: &mut Context<Self>) -> Workspace {
        let name = repo.file_name().map_or_else(
            || repo.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let scan_root = repo.clone();
        let task = cx.background_spawn(async move {
            let index = build_index(&scan_root);
            let sky_modules = provisional_modules(&index.files);
            (index, sky_modules)
        });
        cx.spawn(async move |this, cx| {
            let (index, modules) = task.await;
            let _ = this.update(cx, |this, cx| this.finish_loading(index, modules, cx));
        })
        .detach();
        Workspace {
            repo,
            name: name.into(),
            focus: cx.focus_handle(),
            loaded: None,
            code_open: false,
            palette: None,
            tour: None,
            state: AnalysisState::Idle,
            show_reanalyze: false,
            deep: None,
            cancel: Default::default(),
            deep_cancel: Default::default(),
            recent_reads: Vec::new(),
            demo_applied: false,
        }
    }

    fn finish_loading(
        &mut self,
        index: RepoIndex,
        provisional: Vec<Module>,
        cx: &mut Context<Self>,
    ) {
        let state = cx.global::<AppState>();
        let analysis = state.store.load_analysis(&self.repo);
        let reduce_motion = state.reduce_motion;
        let modules = analysis.as_ref().map_or(provisional, |a| a.modules.clone());
        let index = Arc::new(index);
        let sky = Arc::new(build_sky(&index, &modules));
        let map = cx.new(|_| StarMap::new(sky, reduce_motion));
        let code = cx.new(|_| CodeView::new(self.repo.clone(), index.clone()));
        let subscriptions = vec![
            cx.subscribe(&map, |this, _, e: &StarMapEvent, cx| match e {
                StarMapEvent::OpenFile(i) => this.open_star(*i, cx),
                StarMapEvent::OpenConstellation(c) => this.open_constellation(*c, cx),
            }),
            cx.subscribe(&code, |this, _, e: &CodeEvent, cx| match e {
                CodeEvent::Navigated(path) => this.follow(path.clone(), cx),
                CodeEvent::Close => this.close_code(cx),
            }),
            cx.observe(&map, |_, _, cx| cx.notify()),
        ];
        let file_at =
            index.files.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
        self.loaded = Some(Loaded {
            index,
            file_at,
            map,
            code,
            modules,
            analysis,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn close_code(&mut self, cx: &mut Context<Self>) {
        self.code_open = false;
        if let Some(l) = &self.loaded {
            l.map.update(cx, |m, cx| m.set_selected(None, cx));
        }
        cx.notify();
    }

    fn follow(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        if let Some(i) = l.file_at.get(&path).copied() {
            l.map.update(cx, |m, cx| {
                m.set_selected(Some(i), cx);
                m.fly_to_file(i, cx);
            });
        }
    }

    fn open_star(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let path = l.index.files[file].path.clone();
        self.open_path(&path, 0, None, cx);
    }

    fn open_path(&mut self, path: &str, line: u32, band: Option<(u32, u32)>, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        self.code_open = true;
        l.code.update(cx, |c, cx| c.open(path, line, band, cx));
        cx.notify();
    }

    fn open_constellation(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let sky = l.map.read(cx).sky().clone();
        let Some(c) = sky.constellations.get(index) else { return };
        let module = c.module_index.and_then(|m| l.modules.get(m)).cloned();
        let store = cx.global::<AppState>().store.clone();
        let dive = module.as_ref().and_then(|m| store.load_deep_dive(&self.repo, &m.id));
        self.deep_cancel.store(true, Ordering::Relaxed);
        self.deep = Some(DeepPanel {
            constellation: index,
            name: c.name.clone(),
            summary: module.as_ref().map_or("", |m| m.summary.as_str()).to_string().into(),
            files: c.file_count,
            lines: c.lines,
            module,
            dive,
            state: DeepState::Idle,
        });
        l.map.update(cx, |m, cx| m.fly_to_constellation(index, cx));
        cx.notify();
    }

    fn start_deep_dive(&mut self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let Some(panel) = &mut self.deep else { return };
        let Some(module) = panel.module.clone() else { return };
        let app = cx.global::<AppState>();
        let (cfg, model) = (app.claude.clone(), app.settings.last_model);
        let cancel = Arc::new(AtomicBool::new(false));
        self.deep_cancel = cancel.clone();
        panel.state = DeepState::Running { activity: String::new(), started: Instant::now() };
        let (repo, index) = (self.repo.clone(), l.index.clone());
        let store = app.store.clone();
        let target = panel.constellation;
        run_ai(
            cx,
            move |on_event| deep_dive(&cfg, &repo, &index, &module, model, cancel, on_event),
            |this, e, _| {
                let label = this.describe(&e);
                if let Some(DeepPanel { state: DeepState::Running { activity, .. }, .. }) = &mut this.deep {
                    *activity = label;
                }
            },
            move |this, result, _| {
                let Some(panel) = &mut this.deep else { return };
                if panel.constellation != target {
                    return;
                }
                match result {
                    Ok(dive) => {
                        let _ = store.save_deep_dive(&this.repo, &dive);
                        panel.dive = Some(dive);
                        panel.state = DeepState::Idle;
                    }
                    Err(RunError::Cancelled) => panel.state = DeepState::Idle,
                    Err(e) => panel.state = DeepState::Failed(e.to_string()),
                }
            },
        );
        cx.notify();
    }

    fn describe(&self, e: &RunEvent) -> String {
        match e {
            RunEvent::Reading(path) => {
                std::path::Path::new(path)
                    .strip_prefix(&self.repo)
                    .map_or_else(|_| path.clone(), |p| p.display().to_string())
            }
            RunEvent::Tool(name) => format!("{name} で調べています"),
        }
    }

    fn start_analysis(&mut self, _: &StartAnalysis, _: &mut Window, cx: &mut Context<Self>) {
        // 既存の解析があるときの Enter で再解析しない
        if self.loaded.as_ref().is_some_and(|l| l.analysis.is_some()) && !self.show_reanalyze {
            return;
        }
        self.run_analysis(cx);
    }

    fn run_analysis(&mut self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let app = cx.global::<AppState>();
        if let ClaudeStatus::Problem(message) = &app.claude_status {
            self.state = AnalysisState::Unavailable(message.clone());
            cx.notify();
            return;
        }
        let (cfg, model) = (app.claude.clone(), app.settings.last_model);
        let store = app.store.clone();
        let next = transition(self.state.clone(), Ev::Start(model));
        if next == self.state {
            return;
        }
        self.state = next;
        self.show_reanalyze = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (repo, index) = (self.repo.clone(), l.index.clone());
        run_ai(
            cx,
            move |on_event| analyze_repo(&cfg, &repo, &index, model, cancel, on_event),
            |this, e, cx| this.on_run_event(e, cx),
            move |this, result, cx| this.on_analysis_done(result, store, cx),
        );
        cx.notify();
    }

    fn on_run_event(&mut self, e: RunEvent, cx: &mut Context<Self>) {
        let label = self.describe(&e);
        if let (RunEvent::Reading(_), Some(l)) = (&e, &self.loaded)
            && let Some(i) = l.file_at.get(&label).copied()
        {
            self.recent_reads.push(i);
            let start = self.recent_reads.len().saturating_sub(5);
            self.recent_reads.drain(..start);
            let lit = self.recent_reads.clone();
            l.map.update(cx, |m, cx| m.set_lit(lit, cx));
        }
        self.state = transition(self.state.clone(), Ev::Activity(label));
    }

    fn on_analysis_done(
        &mut self,
        result: Result<Analysis, RunError>,
        store: Arc<ccv_core::store::Store>,
        cx: &mut Context<Self>,
    ) {
        self.recent_reads.clear();
        if let Some(l) = &self.loaded {
            l.map.update(cx, |m, cx| m.set_lit(Vec::new(), cx));
        }
        match result {
            Ok(analysis) => {
                if let Err(e) = store.save_analysis(&self.repo, &analysis) {
                    eprintln!("解析結果を保存できませんでした: {e}");
                }
                self.state = transition(self.state.clone(), Ev::Finished);
                AppState::set_tour_position(cx, &self.repo, 0);
                self.apply_analysis(analysis, cx);
            }
            Err(e) => self.state = transition(self.state.clone(), Ev::Failed(e)),
        }
    }

    fn apply_analysis(&mut self, analysis: Analysis, cx: &mut Context<Self>) {
        let Some(l) = &mut self.loaded else { return };
        l.modules = analysis.modules.clone();
        let sky = Arc::new(build_sky(&l.index, &l.modules));
        l.analysis = Some(analysis);
        l.map.update(cx, |m, cx| m.set_sky(sky, cx));
        self.deep = None;
        self.tour = None;
    }

    fn cancel_analysis(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        cx.notify();
    }

    fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let (repo, index) = (self.repo.clone(), l.index.clone());
        let palette = cx.new(|cx| Palette::new(mode, repo, index, cx));
        let subscription = cx.subscribe_in(&palette, window, |this, _, e: &PaletteEvent, window, cx| {
            match e {
                PaletteEvent::Open(path, line) => {
                    let (path, line) = (path.clone(), *line);
                    this.close_palette(window, cx);
                    let band = line.map(|l| (l + 1, l + 1));
                    this.open_path(&path, line.unwrap_or(0), band, cx);
                }
                PaletteEvent::Preview(path) => {
                    if let Some(l) = &this.loaded
                        && let Some(i) = l.file_at.get(path).copied()
                    {
                        l.map.update(cx, |m, cx| m.set_lit(vec![i], cx));
                    }
                }
                PaletteEvent::Dismiss => this.close_palette(window, cx),
            }
        });
        let handle = palette.focus_handle(cx);
        window.focus(&handle);
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        if let Some(l) = &self.loaded {
            l.map.update(cx, |m, cx| m.set_lit(Vec::new(), cx));
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn on_open_files(&mut self, _: &OpenFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Files, window, cx);
    }

    fn on_open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Content, window, cx);
    }

    fn start_tour(&mut self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let Some(analysis) = &l.analysis else { return };
        let stops = analysis.tour.clone();
        let valid = validity(&stops, &l.index);
        let saved = AppState::tour_position(cx, &self.repo);
        self.tour = Some(TourPlayer::new(stops, valid, saved));
        self.deep = None;
        self.visit_stop(cx);
    }

    fn visit_stop(&mut self, cx: &mut Context<Self>) {
        let Some(tour) = &self.tour else { return };
        let Some(stop) = tour.current().cloned() else {
            cx.notify();
            return;
        };
        let position = tour.position();
        let visited: Vec<String> = tour.visited().into_iter().map(str::to_string).collect();
        if let Some(l) = &self.loaded {
            let route: Vec<usize> =
                visited.iter().filter_map(|p| l.file_at.get(p).copied()).collect();
            l.map.update(cx, |m, cx| m.set_route(route, cx));
        }
        AppState::set_tour_position(cx, &self.repo, position);
        self.open_path(
            &stop.path,
            stop.start_line.saturating_sub(1),
            Some((stop.start_line, stop.end_line)),
            cx,
        );
        self.follow(stop.path, cx);
    }

    fn end_tour(&mut self, cx: &mut Context<Self>) {
        self.tour = None;
        if let Some(l) = &self.loaded {
            l.map.update(cx, |m, cx| {
                m.set_route(Vec::new(), cx);
            });
        }
        cx.notify();
    }

    fn on_tour_toggle(&mut self, _: &TourToggle, _: &mut Window, cx: &mut Context<Self>) {
        if self.tour.is_some() {
            self.end_tour(cx);
        } else {
            self.start_tour(cx);
        }
    }

    fn on_tour_next(&mut self, _: &TourNext, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = &mut self.tour
            && t.next().is_some()
        {
            self.visit_stop(cx);
        }
    }

    fn on_tour_prev(&mut self, _: &TourPrev, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = &mut self.tour
            && t.prev().is_some()
        {
            self.visit_stop(cx);
        }
    }

    fn on_close_top(&mut self, _: &CloseTop, _: &mut Window, cx: &mut Context<Self>) {
        if self.tour.is_some() {
            self.end_tour(cx);
        } else if self.deep.is_some() {
            self.deep = None;
            cx.notify();
        } else if self.code_open {
            self.close_code(cx);
        }
    }

    fn on_back(&mut self, _: &NavBack, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(l) = &self.loaded {
            l.code.update(cx, |c, cx| c.back(cx));
        }
    }

    fn on_forward(&mut self, _: &NavForward, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(l) = &self.loaded {
            l.code.update(cx, |c, cx| c.forward(cx));
        }
    }

    fn on_go_home(&mut self, _: &GoHome, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::GoHome);
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.with_map(cx, |m, cx| m.zoom_step(1.4, cx));
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.with_map(cx, |m, cx| m.zoom_step(1.0 / 1.4, cx));
    }

    fn on_fit(&mut self, _: &FitAll, _: &mut Window, cx: &mut Context<Self>) {
        self.with_map(cx, |m, cx| m.fit_all(cx));
    }

    fn on_toggle_symbols(&mut self, _: &ToggleSymbols, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(l) = &self.loaded {
            l.code.update(cx, |c, cx| c.toggle_symbols(cx));
        }
    }

    fn with_map(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut StarMap, &mut Context<StarMap>)) {
        if let Some(l) = &self.loaded {
            l.map.update(cx, f);
        }
    }

    /// 撮影のために、指定された状態へ直接進む。
    fn apply_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let demo = cx.global::<AppState>().demo;
        let Some(demo) = demo else { return };
        let Some(l) = &self.loaded else { return };
        let biggest = l
            .index
            .files
            .iter()
            .filter(|f| f.lang != ccv_core::model::Lang::Other)
            .max_by_key(|f| f.lines)
            .map(|f| f.path.clone());
        match demo {
            DemoState::Analyzing => {
                let lit: Vec<usize> = (0..l.index.files.len()).step_by(11).take(4).collect();
                let activity = biggest.clone().unwrap_or_default();
                l.map.update(cx, |m, cx| m.set_lit(lit, cx));
                self.state = AnalysisState::Running {
                    model: ModelChoice::Sonnet,
                    activity,
                    started: Instant::now() - Duration::from_secs(84),
                };
            }
            DemoState::Tour => {
                AppState::set_tour_position(cx, &self.repo, 3);
                self.start_tour(cx);
            }
            DemoState::Code => {
                if let Some(path) = biggest {
                    self.open_path(&path, 0, None, cx);
                }
            }
            DemoState::Search => {
                self.open_palette(PaletteMode::Content, window, cx);
                if let Some((p, _)) = &self.palette {
                    p.update(cx, |p, cx| p.set_query("risk", cx));
                }
            }
            DemoState::Deep => {
                self.open_constellation(0, cx);
            }
            DemoState::Error => {
                self.state = AnalysisState::Failed {
                    message: RunError::LimitReached("5 時間の利用枠を使い切りました。".into()).to_string(),
                    can_retry: true,
                };
            }
            DemoState::Unavailable => {
                self.state = AnalysisState::Unavailable(RunError::NotInstalled.to_string());
            }
            DemoState::Home | DemoState::Fresh | DemoState::Done => {}
        }
    }

    fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let stale = self.loaded.as_ref().is_some_and(|l| {
            l.analysis.as_ref().is_some_and(|a| a.head != l.index.head)
        });
        let sha = self
            .loaded
            .as_ref()
            .and_then(|l| l.index.head.as_ref())
            .map(|h| h.chars().take(7).collect::<String>());
        div()
            .id("titlebar")
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(84.))
            .pr(px(12.))
            .gap(px(10.))
            .border_b_1()
            .border_color(theme::hairline())
            .bg(theme::space())
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .child(
                icon_button("go-home", "← 一覧").on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(WorkspaceEvent::GoHome);
                })),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(self.name.clone()),
            )
            .when_some(sha, |d, sha| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::text_muted())
                        .child(sha),
                )
            })
            .when(stale, |d| {
                d.child(
                    div()
                        .px(px(7.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_color(theme::tok_number())
                        .bg(theme::alpha(theme::tok_number(), 0.12))
                        .child("解析が古い"),
                )
            })
            .child(div().flex_1())
            .child(icon_button("open-files", "ファイル  ⌘P").on_click(cx.listener(
                |this, _, window, cx| this.open_palette(PaletteMode::Files, window, cx),
            )))
            .child(icon_button("open-search", "全文検索  ⌘⇧F").on_click(cx.listener(
                |this, _, window, cx| this.open_palette(PaletteMode::Content, window, cx),
            )))
    }

    fn render_dock(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let current = cx.global::<AppState>().settings.last_model;
        let checking = cx.global::<AppState>().claude_status == ClaudeStatus::Checking;
        if let ClaudeStatus::Problem(message) = &cx.global::<AppState>().claude_status
            && !matches!(self.state, AnalysisState::Unavailable(_))
        {
            self.state = AnalysisState::Unavailable(message.clone());
        }
        let has_analysis = self.loaded.as_ref().is_some_and(|l| l.analysis.is_some());
        let dock = || {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(theme::alpha(theme::panel_raised(), 0.94))
                .border_1()
                .border_color(theme::hairline_strong())
                .shadow_lg()
                .text_size(px(13.))
        };
        let primary = |id: &'static str, label: SharedString| {
            div()
                .id(id)
                .flex_none()
                .px(px(13.))
                .py(px(6.))
                .rounded(px(7.))
                .cursor_pointer()
                .bg(theme::violet())
                .text_color(theme::space())
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .hover(|h| h.bg(theme::white()))
                .child(label)
        };
        let picker = |cx: &mut Context<Self>| {
            let _ = cx;
            model_picker("model", current, move |model, _, cx| {
                AppState::set_model(cx, model);
                cx.refresh_windows();
            })
        };

        match self.state.clone() {
            AnalysisState::Unavailable(message) => dock()
                .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::danger()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_color(theme::text()).child("AI 機能を使えません"))
                        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(message)),
                )
                .into_any_element(),
            AnalysisState::Running { model, activity, started } => {
                let secs = started.elapsed().as_secs();
                dock()
                    .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::teal()))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme::text())
                            .child(format!("{} が読んでいます", model.label())),
                    )
                    .child(
                        div()
                            .max_w(px(320.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_family(theme::CODE_FONT)
                            .text_size(px(12.))
                            .text_color(theme::teal())
                            .child(activity),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::CODE_FONT)
                            .text_size(px(12.))
                            .text_color(theme::text_muted())
                            .child(format!("{:02}:{:02}", secs / 60, secs % 60)),
                    )
                    .child(
                        icon_button("cancel-analysis", "中断")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_analysis(cx))),
                    )
                    .into_any_element()
            }
            AnalysisState::Failed { message, can_retry } => dock()
                .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::danger()))
                .child(
                    div()
                        .max_w(px(420.))
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(theme::text())
                        .child(message),
                )
                .child(picker(cx))
                .when(can_retry, |d| {
                    d.child(primary("retry", "再試行".into()).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.state = AnalysisState::Idle;
                            this.run_analysis(cx);
                        },
                    )))
                })
                .into_any_element(),
            AnalysisState::Idle if !has_analysis => dock()
                .child(picker(cx))
                .child(
                    primary(
                        "start-analysis",
                        if checking { "確認しています…" } else { "AI 解析を開始  ⏎" }.into(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.run_analysis(cx))),
                )
                .into_any_element(),
            AnalysisState::Idle => {
                let (model_used, stops) = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.analysis.as_ref())
                    .map_or((ModelChoice::Default, 0), |a| (a.model, a.tour.len()));
                let show = self.show_reanalyze;
                dock()
                    .when(show, |d| {
                        d.child(picker(cx))
                            .child(primary("run-reanalyze", "再解析を実行  ⏎".into()).on_click(
                                cx.listener(|this, _, _, cx| this.run_analysis(cx)),
                            ))
                            .child(icon_button("cancel-reanalyze", "やめる").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.show_reanalyze = false;
                                    cx.notify();
                                }),
                            ))
                    })
                    .when(!show, |d| {
                        d.when(stops > 0, |d| {
                            d.child(
                                primary("start-tour", format!("{stops} か所のツアーを始める  T").into())
                                    .on_click(cx.listener(|this, _, _, cx| this.start_tour(cx))),
                            )
                        })
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child(format!("{} で解析", model_used.label())),
                        )
                        .child(icon_button("reanalyze", "再解析").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.show_reanalyze = true;
                                cx.notify();
                            },
                        )))
                    })
                    .into_any_element()
            }
        }
    }

    fn render_tour_note(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let tour = self.tour.as_ref()?;
        let stop = tour.current()?.clone();
        let (position, total) = (tour.position(), tour.len());
        Some(
            div()
                .flex_none()
                .px(px(22.))
                .pt(px(16.))
                .pb(px(18.))
                .bg(theme::panel_raised())
                .border_t_1()
                .border_color(theme::hairline_strong())
                .child(
                    div()
                        .flex()
                        .gap(px(3.))
                        .mb(px(12.))
                        .children((0..total).map(|i| {
                            div().h(px(2.)).flex_1().rounded_full().bg(if i <= position {
                                theme::teal()
                            } else {
                                theme::hairline_strong()
                            })
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::teal())
                                .child(format!("STOP {} / {}", position + 1, total)),
                        )
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::white())
                                .child(stop.title),
                        )
                        .child(div().flex_1())
                        .child(icon_button("tour-prev", "←").on_click(cx.listener(
                            |this, _, window, cx| this.on_tour_prev(&TourPrev, window, cx),
                        )))
                        .child(icon_button("tour-next", "→").on_click(cx.listener(
                            |this, _, window, cx| this.on_tour_next(&TourNext, window, cx),
                        )))
                        .child(icon_button("tour-exit", "終了  esc").on_click(
                            cx.listener(|this, _, _, cx| this.end_tour(cx)),
                        )),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .max_w(px(720.))
                        .text_size(px(13.5))
                        .line_height(px(23.))
                        .text_color(theme::text())
                        .child(stop.body),
                )
                .into_any_element(),
        )
    }

    fn render_deep(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let panel = self.deep.as_ref()?;
        let current = cx.global::<AppState>().settings.last_model;
        let body: gpui::AnyElement = match (&panel.dive, &panel.state) {
            (_, DeepState::Running { activity, started }) => div()
                .mt(px(14.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::text())
                        .child(format!("読んでいます  {}秒", started.elapsed().as_secs())),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::teal())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(activity.clone()),
                )
                .child(
                    div().mt(px(4.)).child(icon_button("deep-cancel", "中断").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.deep_cancel.store(true, Ordering::Relaxed);
                            cx.notify();
                        }),
                    )),
                )
                .into_any_element(),
            (Some(dive), _) => {
                let files = dive.key_files.clone();
                div()
                    .mt(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .line_height(px(23.))
                            .text_color(theme::text())
                            .child(dive.explanation.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(format!("{} による解説", dive.model.label())),
                    )
                    .children(files.into_iter().enumerate().map(|(i, k)| {
                        let path = k.path.clone();
                        div()
                            .id(("key-file", i))
                            .cursor_pointer()
                            .py(px(6.))
                            .border_t_1()
                            .border_color(theme::hairline())
                            .hover(|h| h.bg(theme::alpha(theme::violet(), 0.08)))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_family(theme::CODE_FONT)
                                    .text_color(theme::teal())
                                    .child(k.path.clone()),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .text_size(px(12.5))
                                    .line_height(px(19.))
                                    .text_color(theme::text_muted())
                                    .child(k.why),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_path(&path, 0, None, cx);
                            }))
                    }))
                    .into_any_element()
            }
            (None, state) => {
                let has_module = panel.module.is_some();
                let available = cx.global::<AppState>().claude_status == ClaudeStatus::Ready;
                div()
                    .mt(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .when_some(
                        if let DeepState::Failed(m) = state { Some(m.clone()) } else { None },
                        |d, m| {
                            d.child(
                                div()
                                    .text_size(px(12.5))
                                    .line_height(px(19.))
                                    .text_color(theme::danger())
                                    .child(m),
                            )
                        },
                    )
                    .when(has_module && available, |d| {
                        d.child(model_picker("deep-model", current, move |model, _, cx| {
                            AppState::set_model(cx, model);
                            cx.refresh_windows();
                        }))
                        .child(
                            div()
                                .id("generate-deep")
                                .px(px(12.))
                                .py(px(7.))
                                .rounded(px(7.))
                                .cursor_pointer()
                                .flex()
                                .justify_center()
                                .bg(theme::violet())
                                .text_color(theme::space())
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .hover(|h| h.bg(theme::white()))
                                .child("解説を生成")
                                .on_click(cx.listener(|this, _, _, cx| this.start_deep_dive(cx))),
                        )
                    })
                    .into_any_element()
            }
        };
        Some(
            div()
                .id("deep-panel")
                .absolute()
                .top(px(16.))
                .left(px(16.))
                .bottom(px(16.))
                .w(px(340.))
                .p(px(18.))
                .overflow_y_scroll()
                .rounded(px(12.))
                .bg(theme::alpha(theme::panel_raised(), 0.96))
                .border_1()
                .border_color(theme::hairline_strong())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::white())
                                .child(panel.name.clone()),
                        )
                        .child(icon_button("deep-close", "閉じる").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.deep = None;
                                cx.notify();
                            },
                        ))),
                )
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(11.5))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::text_muted())
                        .child(format!("{} files · {} lines", panel.files, panel.lines)),
                )
                .when(!panel.summary.is_empty(), |d| {
                    d.child(
                        div()
                            .mt(px(12.))
                            .text_size(px(13.))
                            .line_height(px(21.))
                            .text_color(theme::text_muted())
                            .child(panel.summary.clone()),
                    )
                })
                .child(body)
                .into_any_element(),
        )
    }

    fn render_overview(&self) -> Option<gpui::AnyElement> {
        if self.deep.is_some() || self.code_open {
            return None;
        }
        let analysis = self.loaded.as_ref()?.analysis.as_ref()?;
        Some(
            div()
                .absolute()
                .top(px(20.))
                .left(px(22.))
                .w(px(340.))
                .text_size(px(13.5))
                .line_height(px(23.))
                .text_color(theme::text())
                .child(analysis.overview.clone())
                .into_any_element(),
        )
    }

    fn render_stats(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let l = self.loaded.as_ref()?;
        let sky = l.map.read(cx).sky().clone();
        let hovered = l.map.read(cx).hovered();
        let content = match hovered {
            Some(i) => {
                let f = &l.index.files[i];
                format!("{}  ·  {} lines  ·  直近 1 年で {} 回変更", f.path, f.lines, f.churn)
            }
            None => {
                let total: u64 = l.index.files.iter().map(|f| f.lines as u64).sum();
                format!(
                    "{} files · {} lines · 星座 {}　　大きさ = コード量　明るさ = 変更頻度",
                    l.index.files.len(),
                    total,
                    sky.constellations.len()
                )
            }
        };
        Some(
            div()
                .absolute()
                .bottom(px(16.))
                .left(px(22.))
                .text_size(px(11.5))
                .font_family(theme::CODE_FONT)
                .text_color(if hovered.is_some() { theme::teal() } else { theme::text_muted() })
                .child(content)
                .into_any_element(),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loaded.is_some() && !self.demo_applied {
            self.demo_applied = true;
            self.apply_demo(window, cx);
        }
        let titlebar = self.render_titlebar(cx).into_any_element();
        let root = div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_open_files))
            .on_action(cx.listener(Self::on_open_search))
            .on_action(cx.listener(Self::on_tour_next))
            .on_action(cx.listener(Self::on_tour_prev))
            .on_action(cx.listener(Self::on_tour_toggle))
            .on_action(cx.listener(Self::on_close_top))
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_forward))
            .on_action(cx.listener(Self::start_analysis))
            .on_action(cx.listener(Self::on_go_home))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_fit))
            .on_action(cx.listener(Self::on_toggle_symbols))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::space())
            .text_color(theme::text())
            .child(titlebar);

        if self.loaded.is_none() {
            return root.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .child(div().text_size(px(15.)).child("星を数えています…"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_family(theme::CODE_FONT)
                            .text_color(theme::text_muted())
                            .child(self.repo.display().to_string()),
                    ),
            );
        }

        let dock = self.render_dock(cx);
        let tour_note = self.render_tour_note(cx);
        let deep = self.render_deep(cx);
        let overview = self.render_overview();
        let stats = self.render_stats(cx);
        let touring = self.tour.is_some();
        let code_open = self.code_open;
        let l = self.loaded.as_ref().expect("上で確認した");
        let (map, code) = (l.map.clone(), l.code.clone());

        root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .child(
                    div()
                        .relative()
                        .h_full()
                        .when(code_open, |d| d.w(gpui::relative(0.38)).flex_none())
                        .when(!code_open, |d| d.flex_1())
                        .border_r_1()
                        .border_color(theme::hairline())
                        .child(map)
                        .children(overview)
                        .children(stats)
                        .when(!touring, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .bottom(px(44.))
                                    .left_0()
                                    .right_0()
                                    .flex()
                                    .justify_center()
                                    .child(dock),
                            )
                        })
                        .children(deep),
                )
                .when(code_open, |d| {
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(div().flex_1().min_h_0().child(code))
                            .children(tour_note),
                    )
                }),
        )
        .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |d, palette| {
            d.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .pt(px(96.))
                    .bg(theme::alpha(theme::space(), 0.55))
                    .child(palette),
            )
        })
    }
}
