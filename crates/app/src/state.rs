//! アプリ全体の状態。
use crate::args::DemoState;
use ccv_ai::preflight::{login_shell_path, preflight};
use ccv_ai::runner::ClaudeConfig;
use ccv_core::model::{ModelChoice, RepoEntry, Settings};
use ccv_core::store::Store;
use gpui::{App, Global};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once, OnceLock};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum ClaudeStatus {
    Checking,
    Ready,
    Problem(String),
}

pub struct AppState {
    pub store: Arc<Store>,
    pub settings: Settings,
    pub claude: ClaudeConfig,
    pub claude_status: ClaudeStatus,
    pub reduce_motion: bool,
    pub demo: Option<DemoState>,
}

impl Global for AppState {}

impl AppState {
    pub fn update_settings(cx: &mut App, f: impl FnOnce(&mut Settings)) {
        let state = cx.global_mut::<AppState>();
        f(&mut state.settings);
        // 設定を保存できなくても閲覧は続けられる
        if let Err(e) = state.store.save_settings(&state.settings) {
            eprintln!("設定を保存できませんでした: {e}");
        }
    }

    pub fn set_model(cx: &mut App, model: ModelChoice) {
        Self::update_settings(cx, |s| s.last_model = model);
    }

    /// 登録する。すでにあれば何もしない。
    pub fn register(cx: &mut App, path: PathBuf) {
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Self::update_settings(cx, |s| {
            if !s.repos.iter().any(|r| r.path == path) {
                s.repos.insert(0, RepoEntry { path, name, tour_position: 0 });
            }
        });
    }

    pub fn unregister(cx: &mut App, path: &Path) {
        Self::update_settings(cx, |s| s.repos.retain(|r| r.path != path));
    }

    pub fn relocate(cx: &mut App, old: &Path, new: PathBuf) {
        Self::update_settings(cx, |s| {
            if let Some(r) = s.repos.iter_mut().find(|r| r.path == old) {
                r.path = new;
            }
        });
    }

    pub fn tour_position(cx: &App, repo: &Path) -> usize {
        let s = &cx.global::<AppState>().settings;
        s.repos.iter().find(|r| r.path == repo).map_or(0, |r| r.tour_position)
    }

    pub fn set_tour_position(cx: &mut App, repo: &Path, position: usize) {
        Self::update_settings(cx, |s| {
            if let Some(r) = s.repos.iter_mut().find(|r| r.path == repo) {
                r.tour_position = position;
            }
        });
    }
}

/// claude の確認はシェルの起動を伴い遅いので、起動処理の後ろで行う。
pub fn check_claude(cx: &mut App) {
    let task = cx.background_executor().spawn(async {
        let cfg = ClaudeConfig { bin: "claude".into(), path_env: login_shell_path() };
        let status = match preflight(&cfg) {
            Ok(()) => ClaudeStatus::Ready,
            Err(e) => ClaudeStatus::Problem(e.to_string()),
        };
        (cfg, status)
    });
    cx.spawn(async move |cx| {
        let (cfg, status) = task.await;
        let _ = cx.update(|cx| {
            let state = cx.global_mut::<AppState>();
            state.claude = cfg;
            state.claude_status = status;
            cx.refresh_windows();
        });
    })
    .detach();
}

static START: OnceLock<Instant> = OnceLock::new();
static FIRST_PAINT: Once = Once::new();

pub fn mark_start() {
    START.get_or_init(Instant::now);
}

/// 最初の描画で起動時間を出す。性能の実測に使う。
pub fn mark_first_paint() {
    FIRST_PAINT.call_once(|| {
        if let Some(start) = START.get() {
            eprintln!("CCV_READY {}", start.elapsed().as_millis());
        }
    });
}

/// 「視差効果を減らす」の設定を読む。
pub fn read_reduce_motion() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}
