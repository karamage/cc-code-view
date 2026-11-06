//! デザイントークン。色、時間、書体の値はここだけに書く。
#![allow(dead_code)]
use gpui::{Hsla, Rgba, rgb};

pub fn space() -> Rgba { rgb(0x05060c) }
pub fn nebula() -> Rgba { rgb(0x141633) }
pub fn panel() -> Rgba { rgb(0x0a0b16) }
pub fn panel_raised() -> Rgba { rgb(0x101222) }
pub fn hairline() -> Rgba { rgb(0x20233c) }
pub fn hairline_strong() -> Rgba { rgb(0x34385c) }
pub fn text() -> Rgba { rgb(0xe4e6f6) }
/// space の上で 4.5:1 以上
pub fn text_muted() -> Rgba { rgb(0x9397b8) }
pub fn violet() -> Rgba { rgb(0xa8a4ff) }
pub fn teal() -> Rgba { rgb(0x73e2d4) }
pub fn white() -> Rgba { rgb(0xffffff) }
pub fn danger() -> Rgba { rgb(0xff8f8f) }

pub fn alpha(color: Rgba, a: f32) -> Hsla {
    let mut h: Hsla = color.into();
    h.a = a;
    h
}

/// 暗い星は青紫、明るい星は白へ寄せる。
pub fn star_color(brightness: f32) -> Hsla {
    let b = brightness.clamp(0.0, 1.0);
    let lerp = |a: f32, c: f32| a + (c - a) * b;
    Rgba { r: lerp(0.42, 1.0), g: lerp(0.44, 0.98), b: lerp(0.78, 1.0), a: lerp(0.62, 1.0) }.into()
}

pub const UI_FONT: &str = ".SystemUIFont";
pub const CODE_FONT: &str = "JetBrains Mono";

pub const TRANSITION_MS: u64 = 420;
pub const FLIGHT_MS: u64 = 900;
pub const REDUCED_MS: u64 = 150;

// トークン色（コード）
pub fn tok_keyword() -> Rgba { violet() }
pub fn tok_function() -> Rgba { white() }
pub fn tok_type() -> Rgba { rgb(0xc9c7ff) }
pub fn tok_string() -> Rgba { teal() }
pub fn tok_number() -> Rgba { rgb(0xf0c9a8) }
pub fn tok_comment() -> Rgba { rgb(0x7d81a3) }
pub fn tok_constant() -> Rgba { rgb(0xf0c9a8) }
pub fn tok_plain() -> Rgba { rgb(0xc2c5de) }
