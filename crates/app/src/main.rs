mod ai_task;
mod analysis;
mod blueprint;
mod diagram_view;
mod args;
mod code;
mod home;
mod ja_text;
mod model_picker;
mod palette;
mod starmap;
mod state;
mod text_input;
mod theme;
mod tour;
mod tree_view;
mod ui;
mod workspace;

use args::{DemoState, parse_args};
use ccv_ai::analyze::analyze_repo;
use ccv_ai::preflight::login_shell_path;
use ccv_ai::runner::{ClaudeConfig, RunEvent};
use ccv_core::index::build_index;
use ccv_core::model::ModelChoice;
use ccv_core::store::Store;
use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, Focusable, KeyBinding, Menu, MenuItem, Render,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, point,
    prelude::*, px, size,
};
use home::{Home, HomeEvent};
use state::{AppState, ClaudeStatus, check_claude, read_reduce_motion};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use workspace::{Workspace, WorkspaceEvent};

actions!(app, [Quit]);

enum Screen {
    Home(Entity<Home>),
    Workspace(Entity<Workspace>),
}

struct Root {
    screen: Screen,
    _subscription: Option<Subscription>,
}

impl Root {
    fn show_home(&mut self, cx: &mut Context<Self>) {
        let reduce_motion = cx.global::<AppState>().reduce_motion;
        let home = cx.new(|_| Home::new(reduce_motion));
        self._subscription =
            Some(cx.subscribe(&home, |this, _, e: &HomeEvent, cx| match e {
                HomeEvent::Open(path) => this.show_repo(path.clone(), cx),
            }));
        self.screen = Screen::Home(home);
        cx.notify();
    }

    fn show_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let workspace = cx.new(|cx| Workspace::new(path, cx));
        self._subscription = Some(cx.subscribe(&workspace, |this, _, e: &WorkspaceEvent, cx| {
            match e {
                WorkspaceEvent::GoHome => this.show_home(cx),
            }
        }));
        self.screen = Screen::Workspace(workspace);
        cx.notify();
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Screen::Workspace(w) = &self.screen
            && window.focused(cx).is_none()
        {
            let handle = w.focus_handle(cx);
            window.focus(&handle);
        }
        div().size_full().font_family(theme::UI_FONT).bg(theme::space()).child(match &self.screen {
            Screen::Home(h) => h.clone().into_any_element(),
            Screen::Workspace(w) => w.clone().into_any_element(),
        })
    }
}

fn bind_keys(cx: &mut App) {
    use workspace::*;
    text_input::bind_keys(cx);
    let w = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-p", OpenFiles, w),
        KeyBinding::new("cmd-shift-f", OpenSearch, w),
        KeyBinding::new("right", TourNext, w),
        KeyBinding::new("left", TourPrev, w),
        KeyBinding::new("t", TourToggle, w),
        KeyBinding::new("escape", CloseTop, w),
        KeyBinding::new("cmd-[", NavBack, w),
        KeyBinding::new("cmd-]", NavForward, w),
        KeyBinding::new("enter", StartAnalysis, w),
        KeyBinding::new("cmd-shift-o", GoHome, w),
        KeyBinding::new("cmd-=", ZoomIn, w),
        KeyBinding::new("cmd--", ZoomOut, w),
        KeyBinding::new("cmd-0", FitAll, w),
        KeyBinding::new("cmd-shift-l", ToggleRail, w),
        KeyBinding::new("cmd-alt-b", ToggleRail, w),
        KeyBinding::new("cmd-b", ToggleSidebar, w),
        KeyBinding::new("cmd-w", CloseTab, w),
        KeyBinding::new("cmd-1", ShowMap, w),
        KeyBinding::new("cmd-2", ShowBlueprint, w),
        KeyBinding::new("ctrl-tab", NextTab, w),
        KeyBinding::new("ctrl-shift-tab", PrevTab, w),
        KeyBinding::new("cmd-shift-]", NextTab, w),
        KeyBinding::new("cmd-shift-[", PrevTab, w),
        KeyBinding::new("cmd-shift-e", FocusTree, w),
        KeyBinding::new("cmd-f", FindInFile, w),
        KeyBinding::new("cmd-l", GoToLine, w),
        KeyBinding::new("ctrl-g", GoToLine, w),
        KeyBinding::new("enter", code::FindNext, Some("CodeFind")),
        KeyBinding::new("cmd-g", code::FindNext, Some("CodeFind")),
        KeyBinding::new("shift-enter", code::FindPrev, Some("CodeFind")),
        KeyBinding::new("cmd-shift-g", code::FindPrev, Some("CodeFind")),
        KeyBinding::new("escape", code::FindClose, Some("CodeFind")),
        KeyBinding::new("up", tree_view::Up, Some("FileTree")),
        KeyBinding::new("down", tree_view::Down, Some("FileTree")),
        KeyBinding::new("left", tree_view::Collapse, Some("FileTree")),
        KeyBinding::new("right", tree_view::Expand, Some("FileTree")),
        KeyBinding::new("enter", tree_view::Activate, Some("FileTree")),
        KeyBinding::new("space", tree_view::Activate, Some("FileTree")),
        KeyBinding::new("cmd-c", CopySelection, w),
        KeyBinding::new("down", palette::SelectNext, Some("Palette")),
        KeyBinding::new("up", palette::SelectPrev, Some("Palette")),
        KeyBinding::new("enter", palette::Confirm, Some("Palette")),
        KeyBinding::new("escape", palette::Dismiss, Some("Palette")),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(vec![Menu {
        name: "CC Code View".into(),
        items: vec![MenuItem::action("CC Code View を終了", Quit)],
    }]);
}

fn parse_model(name: Option<&str>) -> ModelChoice {
    ModelChoice::ALL
        .into_iter()
        .find(|m| m.cli_alias() == name)
        .unwrap_or(ModelChoice::Default)
}

/// 画面を出さずに解析して保存する。デモデータの作成と動作確認に使う。
fn analyze_headless(repo: PathBuf, model: ModelChoice, store: &Store) -> i32 {
    let repo = repo.canonicalize().unwrap_or(repo);
    let index = build_index(&repo);
    eprintln!("{} files", index.files.len());
    let cfg = ClaudeConfig { bin: "claude".into(), path_env: login_shell_path() };
    let result = analyze_repo(&cfg, &repo, &index, model, Default::default(), |e| match e {
        RunEvent::Reading(p) => eprintln!("reading {p}"),
        RunEvent::Tool(t) => eprintln!("tool {t}"),
    });
    match result {
        Ok(a) => {
            eprintln!("modules={} tour={}", a.modules.len(), a.tour.len());
            match store.save_analysis(&repo, &a) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// 画面を出さずに設計図を書いて保存する。3 章ずつ並べて動かす。
fn blueprint_headless(repo: PathBuf, model: ModelChoice, chapters: Option<&str>, store: &Store) -> i32 {
    use ccv_ai::blueprint::{ChapterContext, write_chapter};
    use ccv_core::model::ChapterKind;
    let repo = repo.canonicalize().unwrap_or(repo);
    let index = Arc::new(build_index(&repo));
    let kinds: Vec<ChapterKind> = match chapters {
        Some(list) => ChapterKind::ALL.into_iter().filter(|k| list.split(',').any(|s| s.trim() == k.slug())).collect(),
        None => ChapterKind::ALL.to_vec(),
    };
    let context = Arc::new(ChapterContext::new(&index, store.load_analysis(&repo)));
    let cfg = ClaudeConfig { bin: "claude".into(), path_env: login_shell_path() };
    let queue = Arc::new(std::sync::Mutex::new(kinds));
    let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let (queue, context, index, cfg, repo, failures) =
                (queue.clone(), context.clone(), index.clone(), cfg.clone(), repo.clone(), failures.clone());
            let store = Store::new(store_root(store));
            std::thread::spawn(move || {
                while let Some(kind) = queue.lock().unwrap().pop() {
                    eprintln!("[{}] start", kind.slug());
                    let r = write_chapter(&cfg, &repo, &index, &context, kind, model, Default::default(), |e| {
                        if let RunEvent::Reading(p) = e {
                            eprintln!("[{}] reading {p}", kind.slug());
                        }
                    });
                    match r {
                        Ok(c) => {
                            eprintln!("[{}] done diagrams={} points={}", kind.slug(), c.diagrams.len(), c.points.len());
                            if let Err(e) = store.save_chapter(&repo, &c) {
                                eprintln!("{e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("[{}] failed: {e}", kind.slug());
                            failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    i32::from(failures.load(std::sync::atomic::Ordering::Relaxed) > 0)
}

fn store_root(store: &Store) -> PathBuf {
    store.root().to_path_buf()
}

fn main() {
    state::mark_start();
    let args = parse_args(std::env::args().skip(1));
    let root_dir = args.data_dir.clone().unwrap_or_else(Store::default_root);
    let store = Arc::new(Store::new(root_dir));

    if let Some(repo) = args.analyze.clone() {
        std::process::exit(analyze_headless(repo, parse_model(args.model.as_deref()), &store));
    }
    if let Some(repo) = args.blueprint.clone() {
        std::process::exit(blueprint_headless(repo, parse_model(args.model.as_deref()), args.chapters.as_deref(), &store));
    }

    Application::new().run(move |cx: &mut App| {
        let fonts: Vec<Cow<'static, [u8]>> = vec![
            Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")),
        ];
        if let Err(e) = cx.text_system().add_fonts(fonts) {
            eprintln!("書体を読み込めませんでした: {e}");
        }
        let mut settings = store.load_settings();
        let open = args.open.clone().and_then(|p| p.canonicalize().ok());
        if let Some(path) = &open {
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            if !settings.repos.iter().any(|r| &r.path == path) {
                settings.repos.push(ccv_core::model::RepoEntry {
                    path: path.clone(),
                    name,
                    tour_position: 0,
                });
            }
        }
        cx.set_global(AppState {
            store: store.clone(),
            settings,
            claude: ClaudeConfig { bin: "claude".into(), path_env: None },
            claude_status: ClaudeStatus::Checking,
            reduce_motion: read_reduce_motion(),
            demo: args.demo,
        });
        bind_keys(cx);
        check_claude(cx);

        let open_repo = if args.demo == Some(DemoState::Home) { None } else { open };
        // 撮影で最小の大きさを確かめられるよう、環境変数で窓の大きさを変えられる
        let (w, h) = std::env::var("CCV_WINDOW")
            .ok()
            .and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
            })
            .unwrap_or((1280., 800.));
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(900.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("CC Code View".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.), px(14.))),
                }),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let reduce_motion = cx.global::<AppState>().reduce_motion;
                    let home = cx.new(|_| Home::new(reduce_motion));
                    let mut root = Root { screen: Screen::Home(home), _subscription: None };
                    match open_repo {
                        Some(path) => root.show_repo(path, cx),
                        None => root.show_home(cx),
                    }
                    root
                })
            },
        )
        .expect("ウィンドウを開けませんでした");
        cx.activate(true);
    });
}
