//! 画面で共通に使う小さな部品。ボタンとキーの表示の形をここで揃える。
use crate::theme;
use gpui::{FontWeight, SharedString, Stateful, Div, div, prelude::*, px};

/// いちばん勧める操作。一つの面に一つだけ置く
pub fn button_primary(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(12.))
        .rounded(px(7.))
        .cursor_pointer()
        .bg(theme::violet())
        .text_color(theme::space())
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .hover(|h| h.bg(theme::white()))
        .child(label.into())
}

pub fn button_secondary(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(11.))
        .rounded(px(7.))
        .cursor_pointer()
        .border_1()
        .border_color(theme::hairline_strong())
        .text_color(theme::text())
        .text_size(px(12.5))
        .hover(|h| h.bg(theme::alpha(theme::white(), 0.06)))
        .child(label.into())
}

/// 枠のない控えめな操作。帯や見出しの横に置く
pub fn text_button(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(7.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(11.5))
        .text_color(theme::text_muted())
        .hover(|h| h.bg(theme::alpha(theme::white(), 0.06)).text_color(theme::white()))
        .child(label.into())
}

/// 押すキーの表示。
pub fn kbd(label: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(5.))
        .h(px(17.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(theme::hairline_strong())
        .text_size(px(10.5))
        .font_family(theme::CODE_FONT)
        .text_color(theme::text_muted())
        .child(label)
}

/// 区画の見出し。大文字や記号で飾らず、小さく静かに置く
pub fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_muted())
        .child(label)
}

/// 左右のパネルの開閉。窓の形を線で描き、開いている側を塗る
pub fn panel_icon(id: &'static str, left: bool, open: bool) -> Stateful<Div> {
    let color = if open { theme::text() } else { theme::text_muted() };
    let pane = div()
        .w(px(5.))
        .h_full()
        .when(open, |d| d.bg(theme::alpha(color, 0.75)))
        .when(left, |d| d.border_r_1())
        .when(!left, |d| d.border_l_1())
        .border_color(color);
    div()
        .id(id)
        .size(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|h| h.bg(theme::alpha(theme::white(), 0.07)))
        .child(
            div()
                .w(px(16.))
                .h(px(12.))
                .rounded(px(2.5))
                .border_1()
                .border_color(color)
                .overflow_hidden()
                .flex()
                .when(!left, |d| d.justify_end())
                .child(pane),
        )
}
