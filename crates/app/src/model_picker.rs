//! モデルの選択。隠さず並べて、実行前に一目で確認できるようにする。
use crate::theme;
use ccv_core::model::ModelChoice;
use gpui::{App, Window, div, prelude::*, px};
use std::rc::Rc;

pub fn model_picker(
    id: &'static str,
    current: ModelChoice,
    on_pick: impl Fn(ModelChoice, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(8.))
        .bg(theme::alpha(theme::white(), 0.04))
        .children(ModelChoice::ALL.iter().enumerate().map(|(i, model)| {
            let model = *model;
            let selected = model == current;
            let on_pick = on_pick.clone();
            div()
                .id((id, i))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(11.5))
                // 実行ボタンと競わないよう、選択は中立の色で示す
                .text_color(if selected { theme::white() } else { theme::text_muted() })
                .when(selected, |d| d.bg(theme::alpha(theme::white(), 0.17)))
                .when(!selected, |d| {
                    d.hover(|h| h.bg(theme::alpha(theme::white(), 0.07)).text_color(theme::text()))
                })
                // 狭い場所でも一行に収まるよう、既定は短く書く
                .child(if model == ModelChoice::Default { "既定" } else { model.label() })
                .on_click(move |_, window, cx| on_pick(model, window, cx))
        }))
}
