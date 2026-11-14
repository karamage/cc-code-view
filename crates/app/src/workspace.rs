//! リポジトリを開いた画面。左にファイルツリー、中央にタブとコード、右に現在地の星図を置く。
use crate::ai_task::run_ai;
use crate::analysis::{AnalysisState, Ev, transition};
use crate::args::DemoState;
use crate::blueprint::{BlueprintEvent, BlueprintView, TocState, chapter_glyph, chapter_hue};
use crate::code::{CodeEvent, CodeView, FindMode, group_digits, symbol_glyph};
use crate::ja_text::paragraph;
use crate::model_picker::model_picker;
use crate::palette::{Palette, PaletteEvent, PaletteMode};
use crate::starmap::sky::{build_sky, preview_of};
use crate::starmap::{StarMap, StarMapEvent};
use crate::state::{AppState, ClaudeStatus};
use crate::theme;
use crate::tour::{TourPlayer, validity};
use crate::tree_view::{TreeEvent, TreeView};
use crate::ui::{button_primary, button_secondary, kbd, panel_icon, section_label, text_button};
use ccv_ai::analyze::{analyze_repo, deep_dive};
use ccv_ai::runner::{RunError, RunEvent};
use ccv_core::grouping::provisional_modules;
use ccv_core::index::build_index;
use ccv_core::model::{Analysis, DeepDive, Lang, ModelChoice, Module, RepoIndex};
use gpui::{
    AnimationExt, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton,
    Render, SharedString, Subscription, Window, actions, div, prelude::*, px,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
        ToggleRail,
        ToggleSidebar,
        CopySelection,
        CloseTab,
        ShowMap,
        NextTab,
        PrevTab,
        FocusTree,
        FindInFile,
        GoToLine,
        ShowBlueprint,
    ]
);

const RAIL_WIDTH: f32 = 320.;
const RAIL_TEXT_WIDTH: f32 = RAIL_WIDTH - 40.;
const RAIL_MAP_HEIGHT: f32 = 212.;
const SIDEBAR_MIN: f32 = 180.;
const SIDEBAR_MAX: f32 = 520.;
const TITLEBAR: f32 = 40.;
const TABBAR: f32 = 34.;
const STATUSBAR: f32 = 26.;
const MAX_TABS: usize = 12;

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
    tree: Entity<TreeView>,
    modules: Vec<Module>,
    analysis: Option<Analysis>,
    blueprint: Entity<BlueprintView>,
    _subscriptions: Vec<Subscription>,
}

/// 開いているファイル。切り替えて戻ったとき、読んでいた位置から再開する
struct Tab {
    path: String,
    line: u32,
    /// 下見のタブ。ツリーを矢印でたどる間は一つを使い回し、タブを増やさない
    preview: bool,
}

pub struct Workspace {
    repo: PathBuf,
    name: SharedString,
    focus: FocusHandle,
    loaded: Option<Loaded>,
    tabs: Vec<Tab>,
    /// `None` なら星図か設計図のタブ
    active: Option<usize>,
    /// ファイルを開いていないとき、星図ではなく設計図を出す
    blueprint_open: bool,
    /// 最後に開いていたのが設計図か。最後のタブを閉じたときの戻り先
    came_from_blueprint: bool,
    sidebar_open: bool,
    sidebar_width: f32,
    rail_open: bool,
    resizing: bool,
    palette: Option<(Entity<Palette>, Subscription)>,
    tour: Option<TourPlayer>,
    state: AnalysisState,
    show_reanalyze: bool,
    deep: Option<DeepPanel>,
    cancel: Arc<AtomicBool>,
    deep_cancel: Arc<AtomicBool>,
    recent_reads: Vec<usize>,
    reads_total: usize,
    read_stars: Vec<usize>,
    demo_applied: bool,
    /// 星図の置き場所。変わったときだけカメラを合わせ直す
    map_in_rail: Option<bool>,
    /// 次の描画でキー操作を画面へ戻す
    refocus: bool,
    /// 窓の幅。狭いときは右のパネルを畳み、タイトルの帯を詰める
    viewport_width: f32,
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

/// 状態の帯に出す言語名。
fn language_label(path: &str, lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "Rust",
        Lang::TypeScript => "TypeScript",
        Lang::Tsx => "TSX",
        Lang::JavaScript => "JavaScript",
        Lang::Python => "Python",
        Lang::Go => "Go",
        Lang::Other => match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
            Some("md" | "mdx") => "Markdown",
            Some("json" | "jsonc") => "JSON",
            Some("toml") => "TOML",
            Some("yml" | "yaml") => "YAML",
            Some("css" | "scss") => "CSS",
            Some("html") => "HTML",
            Some("sh" | "bash" | "zsh") => "Shell",
            Some("swift") => "Swift",
            Some("sql") => "SQL",
            _ => "テキスト",
        },
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
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
            tabs: Vec::new(),
            active: None,
            blueprint_open: false,
            came_from_blueprint: false,
            sidebar_open: true,
            sidebar_width: 256.,
            rail_open: true,
            resizing: false,
            palette: None,
            tour: None,
            state: AnalysisState::Idle,
            show_reanalyze: false,
            deep: None,
            cancel: Default::default(),
            deep_cancel: Default::default(),
            recent_reads: Vec::new(),
            reads_total: 0,
            read_stars: Vec::new(),
            demo_applied: false,
            map_in_rail: None,
            refocus: false,
            viewport_width: 1280.,
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
        self.save_preview(&sky, cx);
        let map = cx.new(|_| StarMap::new(sky, reduce_motion));
        let code = cx.new(|_| CodeView::new(self.repo.clone(), index.clone()));
        let tree = cx.new(|cx| TreeView::new(index.clone(), cx));
        let blueprint = cx.new(|cx| BlueprintView::new(self.repo.clone(), index.clone(), analysis.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&map, |this, _, e: &StarMapEvent, cx| match e {
                StarMapEvent::OpenFile(i) => this.open_star(*i, cx),
                StarMapEvent::OpenConstellation(c) => this.open_constellation(*c, cx),
            }),
            cx.subscribe(&code, |this, _, e: &CodeEvent, cx| match e {
                CodeEvent::Navigated(path) => this.follow(path.clone(), cx),
                CodeEvent::FindClosed => {
                    this.refocus = true;
                    cx.notify();
                }
            }),
            cx.subscribe(&tree, |this, _, e: &TreeEvent, cx| match e {
                TreeEvent::Open(path) => {
                    let path = path.clone();
                    this.open_path(&path, 0, None, cx);
                }
                TreeEvent::Preview(path) => {
                    let path = path.clone();
                    this.open_path_as(&path, 0, None, true, cx);
                }
            }),
            cx.subscribe(&blueprint, |this, _, e: &BlueprintEvent, cx| match e {
                BlueprintEvent::Open { path, line, band } => {
                    let (path, line, band) = (path.clone(), *line, *band);
                    this.open_path(&path, line, band, cx);
                }
                BlueprintEvent::Hover(path) => {
                    if let Some(l) = &this.loaded {
                        let lit: Vec<usize> = path.as_ref().and_then(|p| l.file_at.get(p).copied()).into_iter().collect();
                        l.map.update(cx, |m, cx| m.set_lit(lit, cx));
                    }
                }
                BlueprintEvent::Reading(path) => {
                    let label = this.describe(&RunEvent::Reading(path.clone()));
                    if let Some(l) = &this.loaded
                        && let Some(i) = l.file_at.get(&label).copied()
                    {
                        this.recent_reads.push(i);
                        let start = this.recent_reads.len().saturating_sub(5);
                        this.recent_reads.drain(..start);
                        let lit = this.recent_reads.clone();
                        l.map.update(cx, |m, cx| m.set_lit(lit, cx));
                    }
                }
            }),
            cx.observe(&map, |_, _, cx| cx.notify()),
            cx.observe(&code, |_, _, cx| cx.notify()),
            cx.observe(&blueprint, |_, _, cx| cx.notify()),
        ];
        let file_at =
            index.files.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
        self.loaded = Some(Loaded {
            index,
            file_at,
            map,
            code,
            tree,
            modules,
            analysis,
            blueprint,
            _subscriptions: subscriptions,
        });
        self.update_tree_marks(cx);
        cx.notify();
    }

    /// ツリーの点の色と停留所の番号を、星図と解析に合わせる。
    fn update_tree_marks(&self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let sky = l.map.read(cx).sky().clone();
        // 仮の星座でも星図と同じ色にし、ツリーと星図の対応を保つ
        let hues: Vec<Option<usize>> = (0..l.index.files.len())
            .map(|i| sky.star_constellation.get(i).map(|c| *c as usize))
            .collect();
        let mut stops: HashMap<usize, usize> = HashMap::new();
        if let Some(a) = &l.analysis {
            for (n, s) in a.tour.iter().enumerate() {
                if let Some(i) = l.file_at.get(&s.path) {
                    stops.entry(*i).or_insert(n + 1);
                }
            }
        }
        l.tree.update(cx, |t, cx| t.set_marks(hues, stops, cx));
    }

    /// 右のパネルを出すか。狭い窓ではコードの幅を優先して畳む
    fn rail_visible(&self) -> bool {
        self.rail_open && self.viewport_width >= 1080.
    }

    fn narrow(&self) -> bool {
        self.viewport_width < 1180.
    }

    fn reading(&self) -> bool {
        self.active.is_some()
    }

    fn active_path(&self) -> Option<&str> {
        self.active.and_then(|i| self.tabs.get(i)).map(|t| t.path.as_str())
    }

    fn dock_visible(&self) -> bool {
        !self.reading() && !self.blueprint_open && self.tour.is_none() && self.deep.is_none()
    }

    /// 星図を右のパネルに縮めて置く画面か。
    fn map_in_rail(&self) -> bool {
        self.reading() || self.blueprint_open
    }

    /// 星図の置き場所と余白を、いまの画面に合わせる。
    fn place_map(&mut self, cx: &mut Context<Self>) {
        let in_rail = self.map_in_rail();
        let dock = self.dock_visible();
        let Some(l) = &self.loaded else { return };
        let insets = if in_rail { (14.0, 14.0, 14.0, 14.0) } else { (28.0, 28.0, if dock { 120.0 } else { 56.0 }, 28.0) };
        l.map.update(cx, |m, cx| m.set_insets(insets, cx));
        if self.map_in_rail != Some(in_rail) {
            self.map_in_rail = Some(in_rail);
            let selected = l.map.read(cx).selected();
            let touring = self.tour.is_some();
            l.map.update(cx, |m, cx| match (in_rail, selected) {
                // ツアー中は停留所の移動がカメラを決める
                (true, Some(i)) if !touring => m.fly_to_file(i, cx),
                (true, _) => {}
                // 初回の配置は星図が自分で全景に合わせる
                (false, _) if m.is_laid_out() => m.fit_all(cx),
                (false, _) => {}
            });
        }
    }

    /// ホーム画面の小さな星図のために保存する。失敗しても閲覧には影響しない。
    fn save_preview(&self, sky: &crate::starmap::sky::Sky, cx: &Context<Self>) {
        let store = cx.global::<AppState>().store.clone();
        if let Err(e) = store.save_preview(&self.repo, &preview_of(sky)) {
            eprintln!("小さな星図を保存できませんでした: {e}");
        }
    }

    /// 状態に応じて空そのものを変える。パネルだけで知らせない。
    fn update_sky_mood(&self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let dim = match self.state {
            // 星図は使えるので、失敗しても暗くしすぎない
            AnalysisState::Failed { .. } => 0.8,
            AnalysisState::Unavailable(_) => 1.0,
            AnalysisState::Running { .. } => 0.62,
            AnalysisState::Idle => 1.0,
        };
        // 読んでいるファイルの星座を際立たせる
        let selected = l.map.read(cx).selected();
        let reading = self
            .reading()
            .then(|| selected.map(|i| l.map.read(cx).sky().star_constellation[i] as usize))
            .flatten();
        let focus = self.deep.as_ref().map(|d| d.constellation).or(reading);
        // ツアーを始める前から、全景で読む順番を見せる
        if self.tour.is_none() {
            let preview: Vec<usize> = if self.map_in_rail() || self.deep.is_some() {
                Vec::new()
            } else {
                l.analysis.as_ref().map_or(Vec::new(), |a| {
                    let mut stops: Vec<usize> = Vec::new();
                    for s in &a.tour {
                        if let Some(i) = l.file_at.get(&s.path)
                            && stops.last() != Some(i)
                        {
                            stops.push(*i);
                        }
                    }
                    stops
                })
            };
            l.map.update(cx, |m, cx| m.set_route(Vec::new(), preview, cx));
        }
        let read = if matches!(self.state, AnalysisState::Running { .. }) {
            self.read_stars.clone()
        } else {
            Vec::new()
        };
        l.map.update(cx, |m, cx| {
            m.set_dim(dim, cx);
            m.set_focus_constellation(focus, cx);
            m.set_read(read, cx);
            m.set_minimap(false, cx);
        });
    }

    /// 星図のタブへ切り替える。開いているファイルはタブに残す。
    fn show_map(&mut self, cx: &mut Context<Self>) {
        self.remember_line(cx);
        self.active = None;
        self.blueprint_open = false;
        self.came_from_blueprint = false;
        if let Some(l) = &self.loaded {
            l.tree.update(cx, |t, cx| t.clear_active(cx));
        }
        cx.notify();
    }

    /// 設計図のタブへ切り替える。
    fn show_blueprint(&mut self, cx: &mut Context<Self>) {
        self.remember_line(cx);
        self.active = None;
        self.blueprint_open = true;
        self.came_from_blueprint = true;
        self.deep = None;
        if let Some(l) = &self.loaded {
            l.tree.update(cx, |t, cx| t.clear_active(cx));
        }
        cx.notify();
    }

    /// 離れるタブの読んでいた行を覚えておく。
    fn remember_line(&mut self, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let top = l.code.read(cx).top_line() as u32;
        if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            tab.line = top + 3;
        }
    }

    /// ファイルのタブを前に出す。なければ追加する。ツリーと星図も追従させる。
    fn focus_tab(&mut self, path: &str, preview: bool, cx: &mut Context<Self>) {
        let at = match self.tabs.iter().position(|t| t.path == path) {
            Some(i) => {
                // 下見で開いたタブも、改めて開けば残す
                if !preview {
                    self.tabs[i].preview = false;
                }
                i
            }
            None if preview && let Some(i) = self.tabs.iter().position(|t| t.preview) => {
                self.tabs[i] = Tab { path: path.to_string(), line: 0, preview: true };
                i
            }
            None => {
                if self.tabs.len() >= MAX_TABS {
                    // 古いものから閉じる。いま読んでいるタブは残す
                    let drop = (0..self.tabs.len()).find(|i| Some(*i) != self.active).unwrap_or(0);
                    self.tabs.remove(drop);
                    if let Some(a) = self.active
                        && a > drop
                    {
                        self.active = Some(a - 1);
                    }
                }
                let insert = self.active.map_or(self.tabs.len(), |a| (a + 1).min(self.tabs.len()));
                self.tabs.insert(insert, Tab { path: path.to_string(), line: 0, preview });
                insert
            }
        };
        self.active = Some(at);
        self.blueprint_open = false;
        let Some(l) = &self.loaded else { return };
        l.tree.update(cx, |t, cx| t.reveal(path, cx));
        if self.tour.is_none()
            && let Some(i) = l.file_at.get(path).copied()
        {
            l.map.update(cx, |m, cx| {
                if m.selected() != Some(i) {
                    m.set_selected(Some(i), cx);
                    m.fly_to_file(i, cx);
                }
            });
        }
    }

    fn follow(&mut self, path: String, cx: &mut Context<Self>) {
        // 定義へのジャンプや履歴の移動で開いたファイルは、下見ではなく残す
        let keep = self.active_path() == Some(path.as_str()) && self.active.is_some_and(|a| self.tabs[a].preview);
        self.focus_tab(&path, keep, cx);
        cx.notify();
    }

    fn open_star(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(l) = &self.loaded else { return };
        let path = l.index.files[file].path.clone();
        self.open_path(&path, 0, None, cx);
    }

    fn open_path(&mut self, path: &str, line: u32, band: Option<(u32, u32)>, cx: &mut Context<Self>) {
        self.open_path_as(path, line, band, false, cx);
    }

    fn open_path_as(
        &mut self,
        path: &str,
        line: u32,
        band: Option<(u32, u32)>,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if self.loaded.is_none() {
            return;
        }
        if self.active_path() != Some(path) {
            self.remember_line(cx);
        }
        self.focus_tab(path, preview, cx);
        let Some(l) = &self.loaded else { return };
        l.code.update(cx, |c, cx| c.open(path, line, band, cx));
        cx.notify();
    }

    fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        if self.active == Some(index) {
            return;
        }
        let (path, line, preview) = (tab.path.clone(), tab.line, tab.preview);
        self.open_path_as(&path, line, None, preview, cx);
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let was_active = self.active == Some(index);
        self.tabs.remove(index);
        match self.active {
            Some(a) if a > index => self.active = Some(a - 1),
            _ => {}
        }
        if was_active {
            self.active = None;
            if self.tabs.is_empty() {
                // 設計図からファイルを開いていたなら、設計図へ戻る
                if self.came_from_blueprint {
                    self.show_blueprint(cx);
                } else {
                    self.show_map(cx);
                }
            } else {
                let next = index.min(self.tabs.len() - 1);
                let t = &self.tabs[next];
                let (path, line, preview) = (t.path.clone(), t.line, t.preview);
                self.open_path_as(&path, line, None, preview, cx);
            }
        }
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
        self.rail_open = true;
        l.map.update(cx, |m, cx| m.fly_to_constellation(index, cx));
        cx.notify();
    }

    fn close_deep(&mut self, cx: &mut Context<Self>) {
        self.deep = None;
        if let Some(l) = &self.loaded {
            let selected = l.map.read(cx).selected();
            let reading = self.reading();
            l.map.update(cx, |m, cx| match (reading, selected) {
                (true, Some(i)) => m.fly_to_file(i, cx),
                _ => m.fit_all(cx),
            });
        }
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
                    Err(e) => {
                        eprintln!("解説の生成に失敗しました: {e}");
                        panel.state = DeepState::Failed(e.to_string());
                    }
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
        // 既存の解析があるときの Enter で再解析しない。読んでいる間と設計図を見ている間も始めない
        if self.reading() || self.blueprint_open {
            return;
        }
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
        self.reads_total = 0;
        self.read_stars.clear();
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
            self.reads_total += 1;
            if !self.read_stars.contains(&i) {
                self.read_stars.push(i);
            }
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
            Err(e) => {
                eprintln!("解析に失敗しました: {e}");
                self.state = transition(self.state.clone(), Ev::Failed(e));
            }
        }
    }

    fn apply_analysis(&mut self, analysis: Analysis, cx: &mut Context<Self>) {
        let Some(l) = &mut self.loaded else { return };
        l.modules = analysis.modules.clone();
        let sky = Arc::new(build_sky(&l.index, &l.modules));
        l.blueprint.update(cx, |b, _| b.set_analysis(Some(analysis.clone())));
        l.analysis = Some(analysis);
        l.map.update(cx, |m, cx| m.set_sky(sky.clone(), cx));
        self.save_preview(&sky, cx);
        self.deep = None;
        self.tour = None;
        self.update_tree_marks(cx);
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
                PaletteEvent::Preview(paths) => {
                    if let Some(l) = &this.loaded {
                        let lit: Vec<usize> =
                            paths.iter().filter_map(|p| l.file_at.get(p).copied()).collect();
                        l.map.update(cx, |m, cx| m.set_lit(lit, cx));
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
        // ツリーに残ったキー操作を、ツアーの矢印へ戻す
        self.refocus = true;
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
        let upcoming: Vec<String> = tour.upcoming().into_iter().map(str::to_string).collect();
        AppState::set_tour_position(cx, &self.repo, position);
        self.open_path(
            &stop.path,
            stop.start_line.saturating_sub(1),
            Some((stop.start_line, stop.end_line)),
            cx,
        );
        if let Some(l) = &self.loaded {
            let route: Vec<usize> =
                visited.iter().filter_map(|p| l.file_at.get(p).copied()).collect();
            let (previous, current) = (
                route.len().checked_sub(2).map(|i| route[i]),
                route.last().copied(),
            );
            let next: Vec<usize> =
                upcoming.iter().filter_map(|p| l.file_at.get(p).copied()).collect();
            l.map.update(cx, |m, cx| {
                m.set_route(route.clone(), next, cx);
                if let Some(current) = current {
                    m.set_selected(Some(current), cx);
                    m.fly_between(previous, current, cx);
                }
            });
        }
    }

    fn end_tour(&mut self, cx: &mut Context<Self>) {
        self.tour = None;
        if let Some(l) = &self.loaded {
            l.map.update(cx, |m, cx| {
                m.set_route(Vec::new(), Vec::new(), cx);
            });
        }
        cx.notify();
    }

    /// 入力欄に文字を打っている。単独のキーで動く操作は、その間は文字として通す
    fn typing(&self, cx: &Context<Self>) -> bool {
        self.palette.is_some() || self.loaded.as_ref().is_some_and(|l| l.code.read(cx).find_open())
    }

    fn on_tour_toggle(&mut self, _: &TourToggle, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing(cx) {
            cx.propagate();
            return;
        }
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
        if let Some(l) = &self.loaded
            && self.blueprint_open
            && l.blueprint.read(cx).has_focus()
        {
            l.blueprint.update(cx, |b, cx| b.close_focus(cx));
        } else if self.tour.is_some() {
            self.end_tour(cx);
        } else if self.deep.is_some() {
            self.close_deep(cx);
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

    fn on_toggle_rail(&mut self, _: &ToggleRail, _: &mut Window, cx: &mut Context<Self>) {
        self.rail_open = !self.rail_open;
        cx.notify();
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    fn on_copy(&mut self, _: &CopySelection, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(l) = &self.loaded {
            l.code.update(cx, |c, cx| c.copy_selection(cx));
        }
    }

    fn on_close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = self.active {
            self.close_tab(a, cx);
        }
    }

    fn on_show_map(&mut self, _: &ShowMap, _: &mut Window, cx: &mut Context<Self>) {
        self.show_map(cx);
    }

    fn on_show_blueprint(&mut self, _: &ShowBlueprint, _: &mut Window, cx: &mut Context<Self>) {
        self.show_blueprint(cx);
    }

    /// タブの並び（星図、設計図、ファイル…）での位置。
    fn tab_position(&self) -> usize {
        match self.active {
            Some(a) => a + 2,
            None if self.blueprint_open => 1,
            None => 0,
        }
    }

    fn go_to_tab_position(&mut self, at: usize, cx: &mut Context<Self>) {
        match at {
            0 => self.show_map(cx),
            1 => self.show_blueprint(cx),
            n => self.activate_tab(n - 2, cx),
        }
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        // 星図と設計図のタブを先頭に含めて巡回する
        let count = self.tabs.len() + 2;
        self.go_to_tab_position((self.tab_position() + 1) % count, cx);
    }

    fn on_prev_tab(&mut self, _: &PrevTab, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.tabs.len() + 2;
        self.go_to_tab_position((self.tab_position() + count - 1) % count, cx);
    }

    fn on_find(&mut self, _: &FindInFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(FindMode::Text, window, cx);
    }

    fn on_go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(FindMode::Line, window, cx);
    }

    fn open_find(&mut self, mode: FindMode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reading() {
            return;
        }
        if let Some(l) = &self.loaded {
            l.code.update(cx, |c, cx| c.open_find(mode, window, cx));
        }
    }

    fn on_focus_tree(&mut self, _: &FocusTree, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = true;
        if let Some(l) = &self.loaded {
            let handle = l.tree.focus_handle(cx);
            window.focus(&handle);
        }
        cx.notify();
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
                let activity = biggest.clone().unwrap_or_default();
                self.reads_total = 23;
                // 読み終えたファイルは灯したままにする
                let current: Vec<usize> = l.file_at.get(&activity).copied().into_iter().collect();
                let mut read: Vec<usize> = (0..l.index.files.len())
                    .filter(|i| !l.index.files[*i].binary)
                    .step_by(9)
                    .take(23)
                    .collect();
                read.retain(|i| !current.contains(i));
                self.read_stars = read;
                l.map.update(cx, |m, cx| m.set_lit(current, cx));
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
                // 普段の読み方に近づけるため、いくつかのタブを開いた状態にする
                let mut others: Vec<String> = l
                    .index
                    .files
                    .iter()
                    .filter(|f| f.lang != ccv_core::model::Lang::Other && Some(&f.path) != biggest.as_ref())
                    .map(|f| f.path.clone())
                    .collect();
                others.sort_by_key(|p| std::cmp::Reverse(p.len()));
                for p in others.into_iter().take(2) {
                    self.open_path(&p, 0, None, cx);
                }
                if let Some(path) = biggest {
                    self.open_path(&path, 40, None, cx);
                }
            }
            DemoState::Find => {
                if let Some(path) = biggest {
                    self.open_path(&path, 0, None, cx);
                    // 読み込みを待ってから検索する
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(600)).await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if let Some(l) = &this.loaded {
                                l.code.update(cx, |c, cx| {
                                    c.open_find(FindMode::Text, window, cx);
                                    c.set_find_query("const", cx);
                                });
                            }
                        });
                    })
                    .detach();
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
            DemoState::DeepRun => {
                self.open_constellation(0, cx);
                self.start_deep_dive(cx);
            }
            DemoState::Error => {
                self.state = AnalysisState::Failed {
                    message: RunError::LimitReached("しばらく待ってからお試しください。".into()).to_string(),
                    can_retry: true,
                };
            }
            DemoState::Unavailable => {
                self.state = AnalysisState::Unavailable(RunError::NotInstalled.to_string());
            }
            DemoState::Home | DemoState::Fresh | DemoState::Done => {}
            DemoState::Blueprint => {
                let blueprint = l.blueprint.clone();
                self.show_blueprint(cx);
                // 撮影で章を指定できるようにする
                if let Some(chapter) = std::env::var("CCV_CHAPTER").ok().and_then(|v| v.parse::<usize>().ok()) {
                    cx.spawn_in(window, async move |_, cx| {
                        cx.background_executor().timer(Duration::from_millis(300)).await;
                        let _ = blueprint.update(cx, |b, cx| b.scroll_to(chapter, cx));
                        // 図を配置すると高さが変わるので、落ち着いてからもう一度送る
                        cx.background_executor().timer(Duration::from_millis(500)).await;
                        let _ = blueprint.update(cx, |b, cx| b.scroll_to(chapter, cx));
                        let extra = std::env::var("CCV_SCROLL").ok().and_then(|v| v.parse::<f32>().ok());
                        if let Some(extra) = extra {
                            cx.background_executor().timer(Duration::from_millis(300)).await;
                            let _ = blueprint.update(cx, |b, cx| b.scroll_by(extra, cx));
                        }
                    })
                    .detach();
                }
            }
            DemoState::BlueprintFocus => {
                self.show_blueprint(cx);
                // 図の配置は最初の描画で行うので、それを待ってから開く
                cx.spawn_in(window, async move |this, cx| {
                    let at = std::env::var("CCV_FOCUS").ok().and_then(|v| {
                        let (c, d) = v.split_once(',')?;
                        Some((c.parse::<usize>().ok()?, d.parse::<usize>().ok()?))
                    });
                    let (c, d) = at.unwrap_or((2, 0));
                    for _ in 0..20 {
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        let done = this
                            .update_in(cx, |this, _, cx| {
                                let Some(l) = &this.loaded else { return true };
                                l.blueprint.update(cx, |b, cx| b.open_focus(c, d, cx));
                                l.blueprint.read(cx).has_focus()
                            })
                            .unwrap_or(true);
                        if done {
                            break;
                        }
                    }
                })
                .detach();
            }
            DemoState::BlueprintRun => {
                let blueprint = l.blueprint.clone();
                self.show_blueprint(cx);
                let kinds: Vec<ccv_core::model::ChapterKind> = match std::env::var("CCV_CHAPTERS") {
                    Ok(list) => ccv_core::model::ChapterKind::ALL
                        .into_iter()
                        .filter(|k| list.split(',').any(|s| s.trim() == k.slug()))
                        .collect(),
                    Err(_) => vec![ccv_core::model::ChapterKind::UseCases],
                };
                // claude の確認が終わってから始める
                let cancel_after = std::env::var("CCV_CANCEL_AFTER").ok().and_then(|v| v.parse::<u64>().ok());
                cx.spawn(async move |_, cx| {
                    for _ in 0..60 {
                        cx.background_executor().timer(Duration::from_millis(500)).await;
                        let ready = cx
                            .update(|cx| cx.global::<AppState>().claude_status == ClaudeStatus::Ready)
                            .unwrap_or(false);
                        if ready {
                            break;
                        }
                    }
                    let _ = blueprint.update(cx, |b, cx| b.generate(&kinds, cx));
                    if let Some(secs) = cancel_after {
                        cx.background_executor().timer(Duration::from_secs(secs)).await;
                        eprintln!("CCV_DEMO cancel");
                        let _ = blueprint.update(cx, |b, cx| b.cancel_all(cx));
                    }
                })
                .detach();
            }
            DemoState::BlueprintRunning => {
                let blueprint = l.blueprint.clone();
                self.show_blueprint(cx);
                blueprint.update(cx, |b, cx| b.demo_running(cx));
            }
        }
    }
}

// 画面の部品
impl Workspace {
    fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let stale = self.loaded.as_ref().is_some_and(|l| {
            l.analysis.as_ref().is_some_and(|a| a.head != l.index.head)
        });
        let can_tour = self
            .loaded
            .as_ref()
            .is_some_and(|l| l.analysis.as_ref().is_some_and(|a| !a.tour.is_empty()));
        let touring = self.tour.is_some();
        let narrow = self.narrow();
        let stale = stale && !narrow;
        let sha = self
            .loaded
            .as_ref()
            .and_then(|l| l.index.head.as_ref())
            .map(|h| h.chars().take(7).collect::<String>());
        div()
            .id("titlebar")
            .h(px(TITLEBAR))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .pl(px(82.))
            .pr(px(10.))
            .gap(px(8.))
            .border_b_1()
            .border_color(theme::hairline())
            .bg(theme::chrome())
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .child(
                text_button("go-home", "一覧")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(WorkspaceEvent::GoHome))),
            )
            .child(div().w(px(1.)).h(px(14.)).bg(theme::hairline_strong()))
            .child(
                div()
                    .pl(px(4.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::white())
                    .child(self.name.clone()),
            )
            .when_some(sha.filter(|_| !narrow), |d, sha| {
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
                        .py(px(1.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_color(theme::tok_number())
                        .bg(theme::alpha(theme::tok_number(), 0.12))
                        .child("解析のあとに更新あり"),
                )
            })
            .child(div().flex_1())
            .child(
                // 中央の検索。どの画面からでもファイルへ飛べる入口
                div().absolute().top_0().bottom_0().left_0().right_0().flex().items_center().justify_center().child(
                    div()
                        .id("open-files")
                        .occlude()
                        .w(px(if narrow { 220. } else { 340. }))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .bg(theme::alpha(theme::white(), 0.035))
                        .border_1()
                        .border_color(theme::hairline())
                        .hover(|h| h.bg(theme::alpha(theme::white(), 0.06)).border_color(theme::hairline_strong()))
                        .child(search_glyph())
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(if narrow { "ファイルへ移動".to_string() } else { format!("{} のファイルへ移動", self.name) }),
                        )
                        .child(kbd("⌘P"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_palette(PaletteMode::Files, window, cx)
                        })),
                ),
            )
            .child(
                text_button("open-search", "全文検索")
                    .child(kbd("⌘⇧F"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_palette(PaletteMode::Content, window, cx)
                    })),
            )
            .when(can_tour, |d| {
                d.child(
                    text_button("title-tour", if touring { "ツアーを終了" } else { "ツアー" })
                        .when(touring, |b| b.text_color(theme::teal()))
                        .child(kbd("T"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_tour_toggle(&TourToggle, window, cx)
                        })),
                )
            })
            .child(div().w(px(4.)))
            .child(panel_icon("toggle-sidebar", true, self.sidebar_open).on_click(cx.listener(
                |this, _, window, cx| this.on_toggle_sidebar(&ToggleSidebar, window, cx),
            )))
            .child(panel_icon("toggle-rail", false, self.rail_visible()).on_click(cx.listener(
                |this, _, window, cx| this.on_toggle_rail(&ToggleRail, window, cx),
            )))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.sidebar_open {
            return None;
        }
        let l = self.loaded.as_ref()?;
        let tree = l.tree.clone();
        let files = l.index.files.len();
        Some(
            div()
                .relative()
                .w(px(self.sidebar_width))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .bg(theme::chrome())
                .border_r_1()
                .border_color(theme::hairline())
                .child(
                    div()
                        .h(px(TABBAR))
                        .flex_none()
                        .flex()
                        .items_center()
                        .pl(px(14.))
                        .pr(px(8.))
                        .gap(px(8.))
                        .child(section_label("ファイル"))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::alpha(theme::text_muted(), 0.7))
                                .child(group_digits(files as u64)),
                        )
                        .child(div().flex_1())
                        .child(text_button("collapse-all", "すべて閉じる").on_click(cx.listener(
                            |this, _, _, cx| {
                                if let Some(l) = &this.loaded {
                                    l.tree.update(cx, |t, cx| t.collapse_all(cx));
                                }
                            },
                        ))),
                )
                .child(div().flex_1().min_h_0().child(tree))
                .child(
                    // 幅を変える取っ手
                    div()
                        .id("sidebar-resize")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right(px(-3.))
                        .w(px(6.))
                        .cursor_col_resize()
                        .when(self.resizing, |d| d.bg(theme::alpha(theme::violet(), 0.35)))
                        .hover(|h| h.bg(theme::alpha(theme::violet(), 0.25)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing = true;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hue_of = |path: &str| -> Option<gpui::Rgba> {
            let l = self.loaded.as_ref()?;
            let i = *l.file_at.get(path)?;
            let sky = l.map.read(cx).sky().clone();
            sky.star_constellation.get(i).map(|c| theme::hue(*c as usize))
        };
        let map_active = self.active.is_none() && !self.blueprint_open;
        let blueprint_active = self.active.is_none() && self.blueprint_open;
        let blueprint_badge = self.loaded.as_ref().map(|l| {
            let b = l.blueprint.read(cx);
            (b.ready_count(), b.busy())
        });
        let tab_shell = |id: gpui::ElementId, active: bool| {
            div()
                .id(id)
                .relative()
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(7.))
                .pl(px(12.))
                .pr(px(8.))
                .cursor_pointer()
                .border_r_1()
                .border_color(theme::hairline())
                .text_size(px(12.))
                .when(active, |d| d.bg(theme::panel()).text_color(theme::white()))
                .when(!active, |d| {
                    d.text_color(theme::text_muted())
                        .hover(|h| h.bg(theme::alpha(theme::white(), 0.03)).text_color(theme::text()))
                })
                .when(active, |d| {
                    d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(theme::teal()))
                })
        };
        let tabs: Vec<gpui::AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let active = self.active == Some(i);
                let hue = hue_of(&t.path);
                tab_shell(("file-tab", i).into(), active)
                    .child(div().size(px(6.)).rounded_full().bg(match hue {
                        Some(h) => theme::alpha(h, 0.95),
                        None => theme::alpha(theme::text_muted(), 0.35),
                    }))
                    .child(
                        div()
                            .when(t.preview, |d| d.italic().text_color(theme::alpha(theme::text_muted(), if active { 1.0 } else { 0.8 })))
                            .child(file_name(&t.path).to_string()),
                    )
                    .child(
                        div()
                            .id(("close-tab", i))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_size(px(13.))
                            .text_color(if active { theme::text_muted() } else { theme::alpha(theme::text_muted(), 0.45).into() })
                            .hover(|h| h.bg(theme::alpha(theme::white(), 0.1)).text_color(theme::white()))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(i, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                        // ダブルクリックで下見のタブを残す
                        if e.click_count() >= 2
                            && let Some(t) = this.tabs.get_mut(i)
                        {
                            t.preview = false;
                        }
                        this.activate_tab(i, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .id("tabbar")
            .h(px(TABBAR))
            .flex_none()
            .flex()
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::hairline())
            .overflow_x_scroll()
            .child(
                tab_shell("map-tab".into(), map_active)
                    .pr(px(14.))
                    .child(div().text_size(px(11.)).text_color(theme::violet()).child("✦"))
                    .child("星図")
                    .on_click(cx.listener(|this, _, _, cx| this.show_map(cx))),
            )
            .child(
                tab_shell("blueprint-tab".into(), blueprint_active)
                    .pr(px(14.))
                    .child(chapter_glyph(ccv_core::model::ChapterKind::Architecture, theme::teal().into(), 13.0))
                    .child("設計図")
                    .when_some(blueprint_badge, |d, (ready, busy)| {
                        d.when(busy, |d| d.child(pulse_dot(theme::teal())))
                            .when(!busy && ready > 0, |d| {
                                d.child(div().text_size(px(10.5)).font_family(theme::CODE_FONT).text_color(theme::text_muted()).child(format!("{ready}/6")))
                            })
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.show_blueprint(cx))),
            )
            .children(tabs)
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
                .occlude()
                .text_size(px(13.))
        };
        let total_files = self.loaded.as_ref().map_or(0, |l| l.index.files.len());
        let picker = |id: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(model_picker(id, current, move |model, _, cx| {
                    AppState::set_model(cx, model);
                    cx.refresh_windows();
                }))
                .child(
                    div()
                        .pl(px(4.))
                        .text_size(px(11.5))
                        .text_color(theme::text_muted())
                        .child(current.hint()),
                )
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
                .child(div().w(px(6.)))
                .child(button_primary("install-claude", "入手方法を開く").on_click(|_, _, cx| {
                    cx.open_url("https://claude.com/claude-code");
                }))
                .child(button_secondary("recheck", "再確認").on_click(cx.listener(|this, _, _, cx| {
                    this.state = AnalysisState::Idle;
                    cx.global_mut::<AppState>().claude_status = ClaudeStatus::Checking;
                    crate::state::check_claude(cx);
                    cx.notify();
                })))
                .into_any_element(),
            AnalysisState::Running { model, activity, started } => {
                let secs = started.elapsed().as_secs();
                dock()
                    .child(pulse_dot(theme::teal()))
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
                            .text_ellipsis()
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
                            .child(format!(
                                "読了 {} / {}  {:02}:{:02}",
                                self.reads_total,
                                total_files,
                                secs / 60,
                                secs % 60
                            )),
                    )
                    .child(
                        button_secondary("cancel-analysis", "中断")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_analysis(cx))),
                    )
                    .into_any_element()
            }
            AnalysisState::Failed { message, can_retry } => {
                let label = match current {
                    ModelChoice::Default => "再試行".to_string(),
                    other => format!("{} で再試行", other.label()),
                };
                dock()
                    .border_color(theme::alpha(theme::danger(), 0.6))
                    .child(div().flex_none().size(px(9.)).rounded_full().bg(theme::danger()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .max_w(px(400.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .text_color(theme::white())
                                    .child(message),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(theme::text_muted())
                                    .child("星図とコードはこのまま使えます。"),
                            ),
                    )
                    .when(can_retry, |d| {
                        d.child(button_primary("retry", label).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state = AnalysisState::Idle;
                                this.run_analysis(cx);
                            },
                        )))
                    })
                    // モデルを変える人は、いつもの選択画面へ戻す
                    .child(button_secondary("change-model", "モデルを変える").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.state = AnalysisState::Idle;
                            this.show_reanalyze = true;
                            cx.notify();
                        },
                    )))
                    .into_any_element()
            }
            AnalysisState::Idle if !has_analysis && !self.show_reanalyze => dock()
                .child(picker("model"))
                .child(
                    button_primary(
                        "start-analysis",
                        if checking { "確認しています…" } else { "AI で解析する" },
                    )
                    .when(!checking, |b| b.child(kbd_dark("⏎")))
                    .on_click(cx.listener(|this, _, _, cx| this.run_analysis(cx))),
                )
                .into_any_element(),
            AnalysisState::Idle => {
                let stops = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.analysis.as_ref())
                    .map_or(0, |a| a.tour.len());
                let show = self.show_reanalyze;
                dock()
                    .when(show, |d| {
                        d.child(picker("model"))
                            .child(button_primary("run-reanalyze", "再解析する").child(kbd_dark("⏎")).on_click(
                                cx.listener(|this, _, _, cx| this.run_analysis(cx)),
                            ))
                            .child(text_button("cancel-reanalyze", "やめる").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.show_reanalyze = false;
                                    cx.notify();
                                }),
                            ))
                    })
                    .when(!show, |d| {
                        d.when(stops > 0, |d| {
                            d.child(
                                button_primary("start-tour", format!("{stops} か所を順に読む"))
                                    .child(kbd_dark("T"))
                                    .on_click(cx.listener(|this, _, _, cx| this.start_tour(cx))),
                            )
                        })
                        .child(button_secondary("reanalyze", "再解析").on_click(cx.listener(
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

    fn render_tour_note(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let tour = self.tour.as_ref()?;
        let stop = tour.current()?.clone();
        let (position, total) = (tour.position(), tour.len());
        let reduce_motion = cx.global::<AppState>().reduce_motion;
        let body = paragraph(&stop.body, (width - 48.0).clamp(240.0, 760.0), 13.5, 23.0, theme::text(), window);
        Some(
            div()
                .flex_none()
                .px(px(24.))
                .pt(px(14.))
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
                                .child(format!("{} / {}", position + 1, total)),
                        )
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::white())
                                .child(stop.title),
                        )
                        .child(div().flex_1())
                        .child(button_secondary("tour-prev", "前へ").child(kbd("←")).on_click(cx.listener(
                            |this, _, window, cx| this.on_tour_prev(&TourPrev, window, cx),
                        )))
                        .child(button_primary("tour-next", "次へ").child(kbd_dark("→")).on_click(cx.listener(
                            |this, _, window, cx| this.on_tour_next(&TourNext, window, cx),
                        )))
                        .child(text_button("tour-exit", "終了").child(kbd("esc")).on_click(
                            cx.listener(|this, _, _, cx| this.end_tour(cx)),
                        )),
                )
                .child(div().mt(px(8.)).child(body.with_animation(
                    // 停留所ごとに識別子を変えて、移動のたびに解説を灯し直す
                    gpui::ElementId::NamedInteger("tour-note".into(), position as u64),
                    gpui::Animation::new(Duration::from_millis(if reduce_motion {
                        theme::REDUCED_MS
                    } else {
                        theme::TRANSITION_MS
                    }))
                    .with_easing(gpui::ease_in_out),
                    |d, t| d.opacity(t),
                )))
                .into_any_element(),
        )
    }

    /// 星座の解説。右のパネルに置き、読みながら開いたままにできる。
    fn render_deep(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let panel = self.deep.as_ref()?;
        let current = cx.global::<AppState>().settings.last_model;
        let hue = theme::hue(panel.constellation);
        let body: gpui::AnyElement = match (&panel.dive, &panel.state) {
            (_, DeepState::Running { activity, started }) => div()
                .mt(px(16.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(theme::text())
                        .child(pulse_dot(theme::teal()))
                        .child(format!("読んでいます  {}秒", started.elapsed().as_secs())),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::teal())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(activity.clone()),
                )
                .child(
                    div().mt(px(4.)).child(button_secondary("deep-cancel", "中断").on_click(
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
                    .mt(px(16.))
                    .flex()
                    .flex_col()
                    .child(paragraph(&dive.explanation, RAIL_TEXT_WIDTH, 13., 22.0, theme::text(), window))
                    .child(
                        div()
                            .mt(px(8.))
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(format!("{} による解説", dive.model.label())),
                    )
                    .child(div().mt(px(20.)).mb(px(6.)).child(section_label("まず読むファイル")))
                    .children(files.into_iter().enumerate().map(|(i, k)| {
                        let path = k.path.clone();
                        div()
                            .id(("key-file", i))
                            .cursor_pointer()
                            .mx(px(-8.))
                            .px(px(8.))
                            .py(px(7.))
                            .rounded(px(6.))
                            .hover(|h| h.bg(theme::alpha(theme::white(), 0.05)))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_family(theme::CODE_FONT)
                                    .text_color(theme::teal())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(k.path.clone()),
                            )
                            .child(div().mt(px(2.)).child(paragraph(
                                &k.why,
                                RAIL_TEXT_WIDTH,
                                12.,
                                18.0,
                                theme::text_muted(),
                                window,
                            )))
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
                    .mt(px(16.))
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
                    .when(!has_module, |d| {
                        d.child(
                            div()
                                .text_size(px(12.5))
                                .line_height(px(19.))
                                .text_color(theme::text_muted())
                                .child("フォルダーから仮に作った星座です。AI で解析すると、名前と解説を付けられます。"),
                        )
                    })
                    .when(has_module && available, |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(theme::text_muted())
                                .child("この星座のファイルを AI が読み、役割と読む順番を解説します。"),
                        )
                        .child(model_picker("deep-model", current, move |model, _, cx| {
                            AppState::set_model(cx, model);
                            cx.refresh_windows();
                        }))
                        .child(
                            button_primary("generate-deep", "解説を作る")
                                .justify_center()
                                .on_click(cx.listener(|this, _, _, cx| this.start_deep_dive(cx))),
                        )
                    })
                    .into_any_element()
            }
        };
        Some(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_none().size(px(8.)).rounded_full().bg(hue))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(17.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::white())
                                .child(panel.name.clone()),
                        )
                        .child(text_button("deep-close", "閉じる").on_click(cx.listener(
                            |this, _, _, cx| this.close_deep(cx),
                        ))),
                )
                .child(
                    div()
                        .mt(px(4.))
                        .pl(px(16.))
                        .text_size(px(11.5))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::text_muted())
                        .child(format!("{} ファイル  {} 行", group_digits(panel.files as u64), group_digits(panel.lines))),
                )
                .when(!panel.summary.is_empty(), |d| {
                    d.child(div().mt(px(14.)).child(paragraph(
                        &panel.summary,
                        RAIL_TEXT_WIDTH,
                        13.,
                        21.,
                        theme::text_muted(),
                        window,
                    )))
                })
                .child(body)
                .child(self.render_constellation_files(panel.constellation, cx))
                .into_any_element(),
        )
    }

    /// 星座に属するファイル。大きい順に並べ、どこから読むかの手がかりにする。
    fn render_constellation_files(&self, constellation: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        const SHOWN: usize = 40;
        let Some(l) = &self.loaded else { return div().into_any_element() };
        let sky = l.map.read(cx).sky().clone();
        let mut files: Vec<usize> = (0..l.index.files.len())
            .filter(|i| sky.star_constellation.get(*i).is_some_and(|c| *c as usize == constellation))
            .collect();
        files.sort_by_key(|i| std::cmp::Reverse(l.index.files[*i].lines));
        let hidden = files.len().saturating_sub(SHOWN);
        files.truncate(SHOWN);
        let current = self.active_path().map(str::to_string);
        div()
            .mt(px(22.))
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(6.))
                    .flex()
                    .justify_between()
                    .child(section_label("この星座のファイル"))
                    .child(div().text_size(px(11.)).text_color(theme::text_muted()).child("行数")),
            )
            .children(files.into_iter().map(|i| {
                let f = &l.index.files[i];
                let path = f.path.clone();
                let open = current.as_deref() == Some(path.as_str());
                div()
                    .id(("constellation-file", i))
                    .h(px(24.))
                    .mx(px(-8.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(open, |d| d.bg(theme::alpha(theme::violet(), 0.14)))
                    .hover(|h| h.bg(theme::alpha(theme::white(), 0.05)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .font_family(theme::CODE_FONT)
                            .text_color(if open { theme::white() } else { theme::tok_plain() })
                            .child(crate::palette::middle_ellipsis(&f.path, 34)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .font_family(theme::CODE_FONT)
                            .text_color(theme::alpha(theme::text_muted(), 0.8))
                            .child(group_digits(f.lines as u64)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_path(&path, 0, None, cx)))
            }))
            .when(hidden > 0, |d| {
                d.child(
                    div()
                        .mt(px(4.))
                        .text_size(px(11.))
                        .text_color(theme::text_muted())
                        .child(format!("ほか {hidden} ファイル")),
                )
            })
            .into_any_element()
    }

    /// 直近 1 年でよく変更されたファイル。どこが動いているかの手がかりにする。
    fn render_hot_files(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let l = self.loaded.as_ref()?;
        let mut files: Vec<usize> = (0..l.index.files.len())
            .filter(|i| l.index.files[*i].churn > 0 && !l.index.files[*i].binary)
            .collect();
        if files.is_empty() {
            return None;
        }
        files.sort_by_key(|i| std::cmp::Reverse(l.index.files[*i].churn));
        files.truncate(8);
        let most = l.index.files[files[0]].churn.max(1) as f32;
        Some(
            div()
                .mt(px(24.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .mb(px(6.))
                        .flex()
                        .justify_between()
                        .child(section_label("よく変更されるファイル"))
                        .child(div().text_size(px(11.)).text_color(theme::text_muted()).child("1 年の回数")),
                )
                .children(files.into_iter().map(|i| {
                    let f = &l.index.files[i];
                    let path = f.path.clone();
                    let share = f.churn as f32 / most;
                    div()
                        .id(("hot-file", i))
                        .h(px(24.))
                        .mx(px(-8.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .hover(|h| h.bg(theme::alpha(theme::white(), 0.05)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(12.))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::tok_plain())
                                .child(crate::palette::middle_ellipsis(&f.path, 30)),
                        )
                        .child(
                            div().flex_none().w(px(28.)).flex().justify_end().child(
                                div().w(px(2.0 + share * 26.0)).h(px(3.)).rounded_full().bg(theme::alpha(theme::teal(), 0.6)),
                            ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(26.))
                                .flex()
                                .justify_end()
                                .text_size(px(10.5))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::alpha(theme::text_muted(), 0.8))
                                .child(f.churn.to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.open_path(&path, 0, None, cx)))
                }))
                .into_any_element(),
        )
    }

    /// 星図のタブで右に出す概要。解析の有無で中身を変える。
    fn render_overview(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let l = self.loaded.as_ref()?;
        let sky = l.map.read(cx).sky().clone();
        let total: u64 = l.index.files.iter().map(|f| f.lines as u64).sum();
        let Some(analysis) = l.analysis.as_ref() else {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::white())
                            .child("まだ名前のない空"),
                    )
                    .child(paragraph(
                        "いまの星座はフォルダーの区切りで仮に並べたものです。AI で解析すると、役割ごとに星座をまとめ直して名前を付け、読む順番のツアーを作ります。",
                        RAIL_TEXT_WIDTH,
                        13.,
                        22.,
                        theme::text_muted(),
                        window,
                    ))
                    .child(stat_grid(vec![
                        ("ファイル", group_digits(l.index.files.len() as u64)),
                        ("行", group_digits(total)),
                        ("仮の星座", sky.constellations.len().to_string()),
                    ]))
                    .child(paragraph(
                        "解析しなくても、左のツリーと検索でコードは読めます。",
                        RAIL_TEXT_WIDTH,
                        12.,
                        19.,
                        theme::alpha(theme::text_muted(), 0.8),
                        window,
                    ))
                    .children(self.render_hot_files(cx))
                    .into_any_element(),
            );
        };
        let mut order: Vec<usize> = (0..sky.constellations.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(sky.constellations[*i].lines));
        let biggest = order.first().map_or(1, |i| sky.constellations[*i].lines).max(1) as f32;
        let rows: Vec<_> = order
            .into_iter()
            .map(|ci| {
                let c = &sky.constellations[ci];
                // 実際の比が分かるよう、棒は線形にする
                let share = c.lines as f32 / biggest;
                div()
                    .id(("constellation-row", ci))
                    .h(px(28.))
                    .px(px(8.))
                    .mx(px(-8.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .cursor_pointer()
                    .hover(|h| h.bg(theme::alpha(theme::white(), 0.05)))
                    .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::hue(ci)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .text_color(theme::text())
                            .child(c.name.clone()),
                    )
                    .child(
                        div().flex_none().w(px(40.)).flex().justify_end().child(
                            div()
                                .w(px(2.0 + share * 38.0))
                                .h(px(3.))
                                .rounded_full()
                                .bg(theme::alpha(theme::hue(ci), 0.7)),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(46.))
                            .flex()
                            .justify_end()
                            .text_size(px(11.))
                            .font_family(theme::CODE_FONT)
                            .text_color(theme::text_muted())
                            .child(group_digits(c.lines)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_constellation(ci, cx)))
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .child(paragraph(&analysis.overview, RAIL_TEXT_WIDTH, 14., 23.0, theme::white(), window))
                .child(
                    div()
                        .mt(px(10.))
                        .text_size(px(11.))
                        .text_color(theme::text_muted())
                        .child(format!("{} の解析", analysis.model.label())),
                )
                .child(div().mt(px(18.)).child(stat_grid(vec![
                    ("ファイル", group_digits(l.index.files.len() as u64)),
                    ("行", group_digits(total)),
                    ("星座", sky.constellations.len().to_string()),
                ])))
                .child(
                    div()
                        .mt(px(22.))
                        .mb(px(6.))
                        .flex()
                        .justify_between()
                        .child(section_label("星座"))
                        .child(div().text_size(px(11.)).text_color(theme::text_muted()).child("行数")),
                )
                .children(rows)
                .children(self.render_hot_files(cx))
                .into_any_element(),
        )
    }

    /// 読んでいる間の右のパネル。星図の現在地、星座、アウトラインを縦に並べる。
    fn render_reading_rail(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let l = self.loaded.as_ref()?;
        let path = self.active_path()?.to_string();
        let file = l.file_at.get(&path).copied();
        let sky = l.map.read(cx).sky().clone();
        let constellation = file.and_then(|i| sky.star_constellation.get(i).map(|c| *c as usize));
        let code = l.code.read(cx);
        let symbols = code.outline().to_vec();
        let active_symbol = code.active_symbol();
        let summary: Option<String> = constellation
            .and_then(|c| sky.constellations.get(c))
            .and_then(|c| c.module_index)
            .and_then(|m| l.modules.get(m))
            .map(|m| m.summary.clone())
            .filter(|s| !s.is_empty() && l.analysis.is_some());
        Some(
            div()
                .flex()
                .flex_col()
                .when_some(constellation.and_then(|c| sky.constellations.get(c).map(|s| (c, s.name.clone()))), |d, (ci, name)| {
                    d.child(
                        div()
                            .id("current-constellation")
                            .mx(px(-8.))
                            .px(px(8.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|h| h.bg(theme::alpha(theme::white(), 0.04)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::hue(ci)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::white())
                                            .child(name),
                                    )
                                    .child(div().text_size(px(11.)).text_color(theme::text_muted()).child("解説")),
                            )
                            .when_some(summary.clone(), |d, s| {
                                d.child(div().mt(px(6.)).pl(px(15.)).child(paragraph(
                                    &s,
                                    RAIL_TEXT_WIDTH - 15.0,
                                    12.,
                                    19.,
                                    theme::text_muted(),
                                    window,
                                )))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.open_constellation(ci, cx))),
                    )
                })
                .child(
                    div()
                        .mt(px(14.))
                        .mb(px(4.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(section_label("アウトライン"))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_family(theme::CODE_FONT)
                                .text_color(theme::alpha(theme::text_muted(), 0.7))
                                .child(symbols.len().to_string()),
                        ),
                )
                .when(symbols.is_empty(), |d| {
                    d.child(
                        div()
                            .py(px(6.))
                            .text_size(px(12.))
                            .text_color(theme::alpha(theme::text_muted(), 0.8))
                            .child("関数や型は見つかりませんでした"),
                    )
                })
                .children(symbols.into_iter().enumerate().map(|(i, s)| {
                    let line = s.line;
                    let active = active_symbol == Some(i);
                    div()
                        .id(("symbol", i))
                        .relative()
                        .mx(px(-8.))
                        .px(px(8.))
                        .h(px(24.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .font_family(theme::CODE_FONT)
                        .text_size(px(12.))
                        .when(active, |d| d.bg(theme::alpha(theme::teal(), 0.1)))
                        .when(!active, |d| d.hover(|h| h.bg(theme::alpha(theme::white(), 0.05))))
                        .child(
                            div()
                                .w(px(10.))
                                .flex_none()
                                .text_color(if active { theme::teal() } else { theme::violet() })
                                .child(symbol_glyph(s.kind)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(if active { theme::white() } else { theme::tok_plain() })
                                .child(s.name),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(10.5))
                                .text_color(theme::alpha(theme::text_muted(), 0.7))
                                .child((line + 1).to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(l) = &this.loaded {
                                l.code.update(cx, |c, cx| c.jump(line, cx));
                            }
                        }))
                }))
                .into_any_element(),
        )
    }

    /// 設計図を読んでいる間の右のパネル。章の目次と、触れている要素のファイル。
    fn render_blueprint_rail(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let l = self.loaded.as_ref()?;
        let b = l.blueprint.read(cx);
        let toc = b.toc();
        let active = b.active_chapter();
        let hovered = b.hovered_file().map(str::to_string);
        let file = hovered.as_ref().and_then(|p| l.file_at.get(p).map(|i| &l.index.files[*i]));
        Some(
            div()
                .flex()
                .flex_col()
                .child(div().mb(px(6.)).child(section_label("章")))
                .children(toc.into_iter().enumerate().map(|(i, (kind, state))| {
                    let hue = chapter_hue(kind);
                    let current = active == Some(i);
                    let (dot, note): (gpui::Hsla, &str) = match state {
                        TocState::Ready => (theme::alpha(hue, 0.9), ""),
                        TocState::Stale => (theme::tok_number().into(), "更新あり"),
                        TocState::Running => (theme::teal().into(), "執筆中"),
                        TocState::Queued => (theme::alpha(theme::text_muted(), 0.6), "待機"),
                        TocState::Failed => (theme::danger().into(), "失敗"),
                        TocState::Missing => (theme::alpha(theme::text_muted(), 0.3), "未作成"),
                    };
                    div()
                        .id(("toc", i))
                        .relative()
                        .h(px(34.))
                        .mx(px(-8.))
                        .px(px(8.))
                        .rounded(px(7.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .cursor_pointer()
                        .when(current, |d| d.bg(theme::alpha(hue, 0.1)))
                        .hover(|h| h.bg(theme::alpha(theme::white(), 0.05)))
                        .when(current, |d| d.child(div().absolute().left_0().top(px(8.)).bottom(px(8.)).w(px(2.)).rounded_full().bg(hue)))
                        .child(div().w(px(18.)).flex_none().text_size(px(11.)).font_family(theme::CODE_FONT).text_color(hue).child(format!("{:02}", kind.number())))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.5))
                                .text_color(if current { theme::white() } else { theme::text() })
                                .child(kind.title()),
                        )
                        .when(!note.is_empty(), |d| d.child(div().flex_none().text_size(px(10.5)).text_color(theme::text_muted()).child(note)))
                        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(l) = &this.loaded {
                                l.blueprint.update(cx, |b, cx| b.scroll_to(i, cx));
                            }
                        }))
                }))
                .child(div().mt(px(22.)).mb(px(6.)).child(section_label("触れている要素")))
                .child(match (hovered, file) {
                    (Some(path), Some(f)) => div()
                        .p(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::alpha(theme::teal(), 0.35))
                        .bg(theme::alpha(theme::teal(), 0.05))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(12.)).font_family(theme::CODE_FONT).text_color(theme::teal()).child(crate::palette::middle_ellipsis(&path, 36)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme::text_muted())
                                .child(format!("{} 行  {}  1 年で {} 回変更", group_digits(f.lines as u64), language_label(&f.path, f.lang), f.churn)),
                        )
                        .into_any_element(),
                    _ => div()
                        .text_size(px(12.))
                        .line_height(px(19.))
                        .text_color(theme::alpha(theme::text_muted(), 0.85))
                        .child("図の要素に触れると、上の星図でそのファイルの星が光ります。")
                        .into_any_element(),
                })
                .into_any_element(),
        )
    }

    fn render_rail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.rail_visible() {
            return None;
        }
        let l = self.loaded.as_ref()?;
        let reading = self.map_in_rail();
        let map = l.map.clone();
        let content = if self.deep.is_some() {
            self.render_deep(window, cx)
        } else if self.blueprint_open && self.active.is_none() {
            self.render_blueprint_rail(cx)
        } else if reading {
            self.render_reading_rail(window, cx)
        } else {
            self.render_overview(window, cx)
        };
        Some(
            div()
                .w(px(RAIL_WIDTH))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .bg(theme::chrome())
                .border_l_1()
                .border_color(theme::hairline())
                .when(reading, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .p(px(10.))
                            .pb(px(0.))
                            .child(
                                div()
                                    .relative()
                                    .h(px(RAIL_MAP_HEIGHT))
                                    .rounded(px(10.))
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(theme::hairline())
                                    .bg(theme::space())
                                    .child(map)
                                    .child(
                                        div()
                                            .id("rail-map-open")
                                            .absolute()
                                            .bottom(px(8.))
                                            .right(px(8.))
                                            .px(px(8.))
                                            .py(px(3.))
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .occlude()
                                            .bg(theme::alpha(theme::chrome(), 0.8))
                                            .border_1()
                                            .border_color(theme::hairline())
                                            .text_size(px(11.))
                                            .text_color(theme::text_muted())
                                            .hover(|h| h.text_color(theme::white()).border_color(theme::hairline_strong()))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .child("全景")
                                            .child(kbd("⌘1"))
                                            .on_click(cx.listener(|this, _, _, cx| this.show_map(cx))),
                                    ),
                            ),
                    )
                })
                .child(
                    div()
                        .id("rail-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(20.))
                        .pt(px(if reading { 12. } else { 20. }))
                        .pb(px(24.))
                        .children(content),
                )
                .into_any_element(),
        )
    }

    fn render_map_hint(&self, width: f32, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        // 狭いときは凡例だけにし、途中で切れた案内を出さない
        let (fits_hint, roomy) = (width >= 640.0, width >= 980.0);
        let l = self.loaded.as_ref()?;
        let hovered = l.map.read(cx).hovered();
        let content = match hovered {
            Some(i) => {
                let f = &l.index.files[i];
                format!(
                    "{}   {} 行   直近 1 年で {} 回変更",
                    f.path,
                    group_digits(f.lines as u64),
                    f.churn
                )
            }
            None => String::new(),
        };
        // AI を使えないときは、解説の案内を出さない
        let hint = if matches!(self.state, AnalysisState::Unavailable(_)) {
            "星でコードを開く　⌘ + スクロールで拡大"
        } else {
            "星でコードを開く　星座名で解説　⌘ + スクロールで拡大"
        };
        let key = |color: gpui::Rgba, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(div().size(px(6.)).rounded_full().bg(color))
                .child(label)
        };
        Some(
            div()
                .absolute()
                .bottom(px(14.))
                .left(px(20.))
                .right(px(20.))
                .flex()
                .items_center()
                .gap(px(16.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .when(hovered.is_some(), |d| {
                    d.child(div().font_family(theme::CODE_FONT).text_color(theme::teal()).child(content))
                })
                .when(hovered.is_none(), |d| {
                    d.child(key(theme::white(), "コード"))
                        .child(key(theme::star_test(), "テスト"))
                        .child(key(theme::star_doc(), "文書と設定"))
                        .when(roomy, |d| {
                            d.child(div().text_color(theme::alpha(theme::text_muted(), 0.8)).child("大きさは行数、明るさは変更の多さ"))
                        })
                        .when(fits_hint, |d| d.child(div().flex_1()).child(hint))
                })
                .into_any_element(),
        )
    }

    fn render_statusbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.loaded.as_ref();
        let total_files = l.map_or(0, |l| l.index.files.len());
        let analysis_note: (gpui::Rgba, String) = match &self.state {
            AnalysisState::Running { model, started, .. } => {
                let secs = started.elapsed().as_secs();
                (
                    theme::teal(),
                    format!(
                        "{} が解析中  {} / {}  {:02}:{:02}",
                        model.label(),
                        self.reads_total,
                        total_files,
                        secs / 60,
                        secs % 60
                    ),
                )
            }
            AnalysisState::Failed { .. } => (theme::danger(), "解析に失敗しました".into()),
            AnalysisState::Unavailable(_) => (theme::danger(), "AI 機能を使えません".into()),
            AnalysisState::Idle => match l.and_then(|l| l.analysis.as_ref()) {
                Some(a) => (theme::violet(), format!("{} で解析済み", a.model.label())),
                None => (theme::alpha(theme::text_muted(), 0.6).into(), "未解析".into()),
            },
        };
        let file_info = l.and_then(|l| {
            let path = self.active_path()?;
            let i = *l.file_at.get(path)?;
            let f = &l.index.files[i];
            let code = l.code.read(cx);
            let top = code.top_line() + 1;
            let count = code.line_count();
            Some((
                language_label(&f.path, f.lang),
                count.map(|c| format!("{} / {} 行", group_digits(top.min(c) as u64), group_digits(c as u64))),
                f.churn,
            ))
        });
        let tour = self.tour.as_ref().map(|t| format!("ツアー {} / {}", t.position() + 1, t.len()));
        let item = || div().flex().items_center().gap(px(6.));
        div()
            .h(px(STATUSBAR))
            .flex_none()
            .flex()
            .items_center()
            .px(px(12.))
            .gap(px(18.))
            .bg(theme::chrome())
            .border_t_1()
            .border_color(theme::hairline())
            .text_size(px(11.))
            .text_color(theme::text_muted())
            .whitespace_nowrap()
            .child(
                item()
                    .child(div().size(px(6.)).rounded_full().bg(analysis_note.0))
                    .child(analysis_note.1),
            )
            .when_some(tour, |d, t| d.child(item().text_color(theme::teal()).child(t)))
            .child(div().flex_1())
            .when_some(file_info, |d, (lang, lines, churn)| {
                d.when(churn > 0, |d| d.child(item().child(format!("1 年で {churn} 回変更"))))
                    .when_some(lines, |d, lines| d.child(item().font_family(theme::CODE_FONT).child(lines)))
                    .child(item().child(lang))
            })
            .child(
                text_button("status-tree", "ツリー")
                    .child(kbd("⌘⇧E"))
                    .on_click(cx.listener(|this, _, window, cx| this.on_focus_tree(&FocusTree, window, cx))),
            )
    }
}

fn search_glyph() -> impl IntoElement {
    // 虫眼鏡を線で描く。書体に頼らず、どの環境でも同じ形にする
    div()
        .relative()
        .size(px(12.))
        .flex_none()
        .child(
            div()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size(px(9.))
                .rounded_full()
                .border_1()
                .border_color(theme::text_muted()),
        )
        .child(
            div()
                .absolute()
                .top(px(8.5))
                .left(px(8.))
                .w(px(4.))
                .h(px(1.5))
                .rounded_full()
                .bg(theme::text_muted()),
        )
}

fn pulse_dot(color: gpui::Rgba) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(7.))
        .rounded_full()
        .bg(color)
        .with_animation(
            "pulse",
            gpui::Animation::new(Duration::from_millis(1400)).repeat().with_easing(gpui::pulsating_between(0.35, 1.0)),
            |d, t| d.opacity(t),
        )
}

fn kbd_dark(label: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(4.))
        .bg(theme::alpha(theme::space(), 0.18))
        .text_size(px(10.5))
        .font_family(theme::CODE_FONT)
        .text_color(theme::alpha(theme::space(), 0.75))
        .child(label)
}

fn stat_grid(items: Vec<(&'static str, String)>) -> impl IntoElement {
    div()
        .flex()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::hairline())
        .children(items.into_iter().enumerate().map(|(i, (label, value))| {
            div()
                .flex_1()
                .px(px(12.))
                .py(px(10.))
                .when(i > 0, |d| d.border_l_1().border_color(theme::hairline()))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_family(theme::CODE_FONT)
                        .text_color(theme::white())
                        .child(value),
                )
                .child(div().mt(px(2.)).text_size(px(11.)).text_color(theme::text_muted()).child(label))
        }))
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let laid_out = self.loaded.as_ref().is_some_and(|l| l.map.read(cx).is_laid_out());
        if !laid_out && self.loaded.is_some() {
            window.request_animation_frame();
        }
        if laid_out && !self.demo_applied {
            self.demo_applied = true;
            self.apply_demo(window, cx);
        }
        // 検索を閉じた直後に別の入力欄を開いていたら、そちらの入力を奪わない
        if std::mem::take(&mut self.refocus) && !self.typing(cx) {
            window.focus(&self.focus);
        }
        self.viewport_width = f32::from(window.viewport_size().width);
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
            .on_action(cx.listener(Self::on_toggle_rail))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_show_map))
            .on_action(cx.listener(Self::on_show_blueprint))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_focus_tree))
            .on_action(cx.listener(Self::on_find))
            .on_action(cx.listener(Self::on_go_to_line))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::space())
            .text_color(theme::text())
            .when(self.resizing, |d| d.cursor_col_resize())
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if !this.resizing {
                    return;
                }
                if e.pressed_button != Some(MouseButton::Left) {
                    this.resizing = false;
                } else {
                    this.sidebar_width = f32::from(e.position.x).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing {
                        this.resizing = false;
                        cx.notify();
                    }
                }),
            )
            .child(titlebar);

        if self.loaded.is_none() {
            return root.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .child(pulse_dot(theme::violet()))
                    .child(div().text_size(px(15.)).text_color(theme::white()).child("星を数えています"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_family(theme::CODE_FONT)
                            .text_color(theme::text_muted())
                            .child(self.repo.display().to_string()),
                    ),
            );
        }

        self.place_map(cx);
        self.update_sky_mood(cx);
        let reading = self.reading();
        let viewport = f32::from(window.viewport_size().width);
        let center_width = viewport
            - if self.sidebar_open { self.sidebar_width } else { 0.0 }
            - if self.rail_visible() { RAIL_WIDTH } else { 0.0 };
        let dock = self.dock_visible().then(|| self.render_dock(cx));
        let tour_note = self.render_tour_note(center_width, window, cx);
        let sidebar = self.render_sidebar(cx);
        let tabs = self.render_tabs(cx).into_any_element();
        let rail = self.render_rail(window, cx);
        let hint = (!reading && !self.blueprint_open).then(|| self.render_map_hint(center_width, cx)).flatten();
        let status = self.render_statusbar(cx).into_any_element();
        let l = self.loaded.as_ref().expect("上で確認した");
        let (map, code) = (l.map.clone(), l.code.clone());
        // 星図は一つしかないので、読んでいる間は右のパネルに置く
        let blueprint_view = l.blueprint.clone();
        let center: gpui::AnyElement = if self.blueprint_open && !reading {
            blueprint_view.update(cx, |b, _| b.set_width(center_width));
            div().flex_1().min_h_0().child(blueprint_view).into_any_element()
        } else if reading {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(code))
                .children(tour_note)
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .bg(theme::space())
                .child(map)
                .children(hint)
                .when_some(dock, |d, dock| {
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
                .into_any_element()
        };
        let focus = self.focus.clone();

        root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .children(sidebar)
                .child(
                    div()
                        .id("center")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        // コードを触ったら、矢印キーをツアーへ戻す
                        .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                        .child(tabs)
                        .child(center),
                )
                .children(rail),
        )
        .child(status)
        .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |d, palette| {
            d.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(84.))
                    .bg(theme::alpha(theme::space(), 0.62))
                    // 後ろの星図へ入力を通さない。外側を押すと閉じる
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
                    )
                    .child(div().occlude().child(palette)),
            )
        })
    }
}
