mod analysis;
mod args;
mod code;
mod home;
mod model_picker;
mod palette;
mod starmap;
mod state;
mod text_input;
mod theme;
mod tour;
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
        let home = cx.new(|_| Home::new());
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
        KeyBinding::new("cmd-shift-l", ToggleSymbols, w),
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

fn main() {
    let args = parse_args(std::env::args().skip(1));
    let root_dir = args.data_dir.clone().unwrap_or_else(Store::default_root);
    let store = Arc::new(Store::new(root_dir));

    if let Some(repo) = args.analyze.clone() {
        std::process::exit(analyze_headless(repo, parse_model(args.model.as_deref()), &store));
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
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
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
                    let home = cx.new(|_| Home::new());
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
        eprintln!("CCV_READY");
    });
}
