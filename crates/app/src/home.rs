//! リポジトリの一覧と登録。
use crate::code::icon_button;
use crate::state::AppState;
use crate::theme;
use ccv_core::churn::is_git_repo;
use ccv_core::model::{Analysis, SkyPreview};
use gpui::{
    Context, EventEmitter, ExternalPaths, FontWeight, PathPromptOptions, Render, SharedString,
    Window, canvas, div, prelude::*, px,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub enum HomeEvent {
    Open(PathBuf),
}

/// 保存された情報。描画のたびにファイルを読まないよう、一度だけ読む。
#[derive(Clone)]
struct RepoInfo {
    status: String,
    preview: Option<Arc<SkyPreview>>,
}

pub struct Home {
    error: Option<SharedString>,
    opened: Instant,
    reduce_motion: bool,
    info: HashMap<PathBuf, RepoInfo>,
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

fn status_line(analysis: Option<&Analysis>) -> String {
    match analysis {
        Some(a) => format!(
            "{} で解析済み　星座 {}　停留所 {}",
            a.model.label(),
            a.modules.len(),
            a.tour.len()
        ),
        None => "まだ解析していません".to_string(),
    }
}

/// 背景の星雲。一度だけ読み込み、描画のたびに作り直さない
fn nebula() -> Arc<gpui::Image> {
    static IMAGE: std::sync::OnceLock<Arc<gpui::Image>> = std::sync::OnceLock::new();
    IMAGE
        .get_or_init(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Jpeg,
                include_bytes!("../assets/images/nebula.jpg").to_vec(),
            ))
        })
        .clone()
}

impl Home {
    pub fn new(reduce_motion: bool) -> Home {
        Home { error: None, opened: Instant::now(), reduce_motion, info: HashMap::new() }
    }

    fn info_for(&mut self, path: &Path, cx: &Context<Self>) -> RepoInfo {
        if let Some(info) = self.info.get(path) {
            return info.clone();
        }
        let store = cx.global::<AppState>().store.clone();
        let info = RepoInfo {
            status: status_line(store.load_analysis(path).as_ref()),
            preview: store.load_preview(path).map(Arc::new),
        };
        self.info.insert(path.to_path_buf(), info.clone());
        info
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
                            this.info.clear();
                        }
                        Err(message) => this.error = Some(message.into()),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn render_hero(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::teal())
                    .child("CC Code View"),
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
                    .child("いつものファイルツリーとコードで読みながら、星図で"),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(px(24.))
                    .text_color(theme::text_muted())
                    .child("全体の中の位置を確かめる。読む順番は Claude が案内します。"),
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
                            .child("フォルダーをここへドロップしても登録できます"),
                    ),
            )
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let intro = if self.reduce_motion {
            1.0
        } else {
            self.opened.elapsed().as_secs_f32() / 2.2
        };
        if intro < 1.0 {
            window.request_animation_frame();
        }
        let intro = intro.min(1.0);
        let repos = cx.global::<AppState>().settings.repos.clone();
        let empty = repos.is_empty();
        let infos: Vec<RepoInfo> = repos.iter().map(|r| self.info_for(&r.path, cx)).collect();
        // 最後に開いた順の先頭のリポジトリの空を背景にする
        let backdrop = infos.iter().find_map(|i| i.preview.clone());

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
            // 星雲の写真を一番奥に敷き、手描きの星図をその上に重ねる
            .child(
                gpui::img(nebula())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover)
                    .opacity(0.25 + 0.6 * intro),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        crate::starmap::paint::paint_backdrop(
                            bounds,
                            intro,
                            backdrop.as_deref(),
                            window,
                        );
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
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                        window.start_window_move()
                    }),
            )
            .child(
                div()
                    .id("home-content")
                    .relative()
                    .w(px(560.))
                    .ml(px(72.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .pt(px(if empty { 210. } else { 76. }))
                    .pb(px(40.))
                    .overflow_y_scroll()
                    .when(empty, |d| d.child(self.render_hero(cx)))
                    // 再訪のときは宣伝を繰り返さず、一覧を主役にする
                    .when(!empty, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_end()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.))
                                        .flex_1()
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(theme::teal())
                                                .child("CC Code View"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(26.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child("どの空を読みますか"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("add-repo-compact")
                                        .px(px(12.))
                                        .py(px(6.))
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .border_1()
                                        .border_color(theme::hairline_strong())
                                        .text_size(px(12.5))
                                        .hover(|h| h.bg(theme::alpha(theme::white(), 0.07)))
                                        .child("＋ 登録")
                                        .on_click(cx.listener(|this, _, _, cx| this.choose(cx))),
                                ),
                        )
                    })
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
                    .when(!empty, |d| d.child(div().h(px(22.))))
                    .children(repos.into_iter().zip(infos).enumerate().map(|(i, (repo, info))| {
                        let exists = repo.path.exists();
                        let path = repo.path.clone();
                        let path_for_remove = repo.path.clone();
                        let path_for_relocate = repo.path.clone();
                        let line = if exists {
                            info.status.clone()
                        } else {
                            "見つかりません。移動した場合は場所を指定してください。".to_string()
                        };
                        let preview = info.preview.clone();
                        div()
                            .id(("repo", i))
                            .group("repo")
                            .mb(px(8.))
                            .p(px(10.))
                            .rounded(px(12.))
                            .bg(theme::alpha(theme::panel_raised(), 0.82))
                            .border_1()
                            .border_color(theme::hairline())
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .when(exists, |d| {
                                d.cursor_pointer()
                                    .hover(|h| h.border_color(theme::violet()))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(HomeEvent::Open(path.clone()))
                                    }))
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(112.))
                                    .h(px(72.))
                                    .rounded(px(8.))
                                    .overflow_hidden()
                                    .bg(theme::space())
                                    .child(
                                        canvas(
                                            |_, _, _| {},
                                            move |bounds, _, window, _| {
                                                if let Some(p) = &preview {
                                                    crate::starmap::paint::paint_preview(
                                                        bounds, p, 0.26, 1.0, window,
                                                    );
                                                }
                                            },
                                        )
                                        .size_full(),
                                    ),
                            )
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
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(repo.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .font_family(theme::CODE_FONT)
                                            .text_color(theme::text_muted())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(crate::palette::middle_ellipsis(&repo.path.display().to_string(), 46)),
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
                                d.child(icon_button("relocate", "場所を指定").on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.relocate(path_for_relocate.clone(), cx)
                                    },
                                )))
                            })
                            .child(
                                // 普段は隠し、行に乗せたときだけ出す
                                div().invisible().group_hover("repo", |s| s.visible()).child(
                                    icon_button("remove", "登録解除").on_click(cx.listener(
                                        move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            AppState::unregister(cx, &path_for_remove);
                                            cx.notify();
                                        },
                                    )),
                                ),
                            )
                            .when(exists, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .pr(px(6.))
                                        .text_size(px(16.))
                                        .text_color(theme::text_muted())
                                        .child("›"),
                                )
                            })
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
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(validate_repo(dir.path()).is_ok());
    }
}
