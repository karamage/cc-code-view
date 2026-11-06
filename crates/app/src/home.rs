//! リポジトリの一覧と登録。
use crate::state::AppState;
use crate::theme;
use ccv_core::churn::is_git_repo;
use ccv_core::model::Analysis;
use gpui::{
    Context, EventEmitter, ExternalPaths, FontWeight, PathPromptOptions, Render, SharedString,
    Window, canvas, div, prelude::*, px,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub enum HomeEvent {
    Open(PathBuf),
}

pub struct Home {
    error: Option<SharedString>,
    opened: Instant,
    reduce_motion: bool,
}

impl EventEmitter<HomeEvent> for Home {}

pub fn validate_repo(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!("{} はフォルダーではありません。", path.display()));
    }
    if !is_git_repo(path) {
        return Err(format!(
            "{} は Git リポジトリではありません。クローンしたフォルダーを選んでください。",
            path.display()
        ));
    }
    path.canonicalize().map_err(|e| e.to_string())
}

impl Home {
    pub fn new(reduce_motion: bool) -> Home {
        Home { error: None, opened: Instant::now(), reduce_motion }
    }

    fn add(&mut self, path: &Path, cx: &mut Context<Self>) {
        match validate_repo(path) {
            Ok(path) => {
                self.error = None;
                AppState::register(cx, path.clone());
                cx.emit(HomeEvent::Open(path));
            }
            Err(message) => self.error = Some(message.into()),
        }
        cx.notify();
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("登録".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.add(&path, cx));
            }
        })
        .detach();
    }

    fn relocate(&mut self, old: PathBuf, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("この場所にする".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    match validate_repo(&path) {
                        Ok(path) => {
                            this.error = None;
                            AppState::relocate(cx, &old, path);
                        }
                        Err(message) => this.error = Some(message.into()),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

fn status_line(analysis: Option<&Analysis>) -> String {
    match analysis {
        Some(a) => format!("{} で解析済み · 星座 {} · 停留所 {}", a.model.label(), a.modules.len(), a.tour.len()),
        None => "未解析".to_string(),
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let intro = if self.reduce_motion { 1.0 } else { self.opened.elapsed().as_secs_f32() / 2.2 };
        if intro < 1.0 {
            window.request_animation_frame();
        }
        let intro = intro.min(1.0);
        let state = cx.global::<AppState>();
        let store = state.store.clone();
        let repos = state.settings.repos.clone();
        let empty = repos.is_empty();

        div()
            .id("home")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::space())
            .text_color(theme::text())
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(path) = paths.paths().first() {
                    this.add(path, cx);
                }
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        crate::starmap::paint::paint_backdrop(bounds, intro, window);
                        crate::state::mark_first_paint();
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("home-window-drag")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(44.))
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .child(
                div()
                    .id("home-content")
                    .relative()
                    .w(px(600.))
                    .ml(px(88.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .pt(px(if empty { 210. } else { 96. }))
                    .pb(px(40.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::teal())
                            .font_family(theme::CODE_FONT)
                            .child("CC CODE VIEW"),
                    )
                    .child(
                        div()
                            .mt(px(14.))
                            .text_size(px(38.))
                            .line_height(px(50.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("コードベースの星図を、"),
                    )
                    .child(
                        div()
                            .text_size(px(38.))
                            .line_height(px(50.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("AI の案内で航海する。"),
                    )
                    .child(
                        div()
                            .mt(px(16.))
                            .text_size(px(14.))
                            .line_height(px(24.))
                            .text_color(theme::text_muted())
                            .child("クローンしたリポジトリを登録すると、ファイルが星になり、Claude が読む順番を案内します。"),
                    )
                    .child(
                        div()
                            .mt(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(
                                div()
                                    .id("add-repo")
                                    .px(px(16.))
                                    .py(px(9.))
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .bg(theme::violet())
                                    .text_color(theme::space())
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .hover(|h| h.bg(theme::white()))
                                    .child("リポジトリを登録")
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(cx))),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::text_muted())
                                    .child("フォルダーをここにドロップしても登録できます"),
                            ),
                    )
                    .when_some(self.error.clone(), |d, message| {
                        d.child(
                            div()
                                .mt(px(16.))
                                .text_size(px(13.))
                                .line_height(px(20.))
                                .text_color(theme::danger())
                                .child(message),
                        )
                    })
                    .when(!empty, |d| {
                        d.child(
                            div()
                                .mt(px(44.))
                                .mb(px(6.))
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child("登録済みのリポジトリ"),
                        )
                    })
                    .children(repos.into_iter().enumerate().map(|(i, repo)| {
                        let exists = repo.path.exists();
                        let analysis = store.load_analysis(&repo.path);
                        let path = repo.path.clone();
                        let path_for_remove = repo.path.clone();
                        let path_for_relocate = repo.path.clone();
                        let line = if exists {
                            status_line(analysis.as_ref())
                        } else {
                            "見つかりません。移動した場合は場所を指定してください。".to_string()
                        };
                        div()
                            .id(("repo", i))
                            .group("repo")
                            .py(px(14.))
                            .px(px(12.))
                            .mx(px(-12.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .when(exists, |d| {
                                d.cursor_pointer()
                                    .hover(|h| h.bg(theme::alpha(theme::violet(), 0.06)))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(HomeEvent::Open(path.clone()))
                                    }))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(repo.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_family(theme::CODE_FONT)
                                            .text_color(theme::text_muted())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(repo.path.display().to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(if exists {
                                                theme::violet()
                                            } else {
                                                theme::danger()
                                            })
                                            .child(line),
                                    ),
                            )
                            .when(!exists, |d| {
                                d.child(
                                    crate::code::icon_button("relocate", "場所を指定").on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.relocate(path_for_relocate.clone(), cx)
                                        }),
                                    ),
                                )
                            })
                            .child(
                                // 普段は隠し、行に乗せたときだけ出す
                                div().invisible().group_hover("repo", |s| s.visible()).child(
                                    crate::code::icon_button("remove", "登録解除").on_click(
                                        cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            AppState::unregister(cx, &path_for_remove);
                                            cx.notify();
                                        }),
                                    ),
                                ),
                            )
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_files_and_non_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(validate_repo(&file).unwrap_err().contains("フォルダー"));
        assert!(validate_repo(dir.path()).unwrap_err().contains("Git リポジトリ"));
        std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        assert!(validate_repo(dir.path()).is_ok());
    }
}
