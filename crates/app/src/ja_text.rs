//! 禁則処理をした段落を gpui の要素にする。行の分け方は `ccv_core::text` にある。
use crate::theme;
use ccv_core::text::break_lines;
use gpui::{FontStyle, Hsla, Window, div, font, prelude::*, px};

/// 禁則処理をした段落を作る。
pub fn paragraph(
    text: &str,
    width: f32,
    size: f32,
    line_height: f32,
    color: impl Into<Hsla>,
    window: &mut Window,
) -> gpui::Div {
    let color = color.into();
    let system = window.text_system().clone();
    let mut ui_font = font(theme::UI_FONT);
    ui_font.style = FontStyle::Normal;
    let font_id = system.resolve_font(&ui_font);
    let advance = move |c: char| -> f32 {
        if c.is_ascii() {
            system.advance(font_id, px(size), c).map_or(size * 0.55, |a| f32::from(a.width))
        } else {
            size
        }
    };
    let lines = break_lines(text, width, &advance);
    div()
        .flex()
        .flex_col()
        .text_size(px(size))
        .line_height(px(line_height))
        .text_color(color)
        .children(lines.into_iter().map(|l| div().whitespace_nowrap().child(l)))
}

/// バッククォートの位置を表す文字。幅を持たず、行分けの規則にも影響しない
const CODE_MARK: char = '\u{E000}';

/// `名前` をコードの色で示す段落。AI の文章のうち、コードの名前だけを目立たせる。
pub fn rich_paragraph(
    text: &str,
    width: f32,
    size: f32,
    line_height: f32,
    color: impl Into<Hsla>,
    window: &mut Window,
) -> gpui::Div {
    let color = color.into();
    if !text.contains('`') {
        return paragraph(text, width, size, line_height, color, window);
    }
    let system = window.text_system().clone();
    let mut ui_font = font(theme::UI_FONT);
    ui_font.style = FontStyle::Normal;
    let font_id = system.resolve_font(&ui_font);
    let advance = move |c: char| -> f32 {
        if c == CODE_MARK {
            0.0
        } else if c.is_ascii() {
            system.advance(font_id, px(size), c).map_or(size * 0.55, |a| f32::from(a.width))
        } else {
            size
        }
    };
    let marked = text.replace('`', &CODE_MARK.to_string());
    let lines = break_lines(&marked, width, &advance);
    let code = gpui::HighlightStyle {
        color: Some(theme::teal().into()),
        background_color: Some(theme::alpha(theme::teal(), 0.06)),
        ..Default::default()
    };
    let mut inside = false;
    div()
        .flex()
        .flex_col()
        .text_size(px(size))
        .line_height(px(line_height))
        .text_color(color)
        .children(lines.into_iter().map(|line| {
            let mut plain = String::new();
            let mut ranges = Vec::new();
            let mut start = 0;
            for c in line.chars() {
                if c == CODE_MARK {
                    if inside {
                        ranges.push((start..plain.len(), code));
                    } else {
                        start = plain.len();
                    }
                    inside = !inside;
                } else {
                    plain.push(c);
                }
            }
            // 行をまたぐコードは、行末でいったん閉じる
            if inside && start < plain.len() {
                ranges.push((start..plain.len(), code));
            }
            ranges.retain(|(r, _)| !r.is_empty());
            div().whitespace_nowrap().child(gpui::StyledText::new(plain).with_highlights(ranges))
        }))
}
