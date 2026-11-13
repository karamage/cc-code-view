//! デザイントークン。色、時間、書体の値はここだけに書く。
#![allow(dead_code)]
use gpui::{Hsla, Rgba, rgb};

pub fn space() -> Rgba { rgb(0x05060c) }
pub fn nebula() -> Rgba { rgb(0x141633) }
/// コードの面。周りの枠より一段明るくし、読む場所を示す
pub fn panel() -> Rgba { rgb(0x0b0c18) }
pub fn panel_raised() -> Rgba { rgb(0x12142a) }
/// ツリー、タブ、タイトル、状態の帯。星空とコードの間の色
pub fn chrome() -> Rgba { rgb(0x07080f) }
pub fn hairline() -> Rgba { rgb(0x1a1d33) }
pub fn hairline_strong() -> Rgba { rgb(0x2e3254) }
pub fn text() -> Rgba { rgb(0xe4e6f6) }
/// space の上で 4.5:1 以上
pub fn text_muted() -> Rgba { rgb(0xa5a9c8) }
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

/// 星の種類の色。意味を持たせ、彩度は抑える。
pub fn star_test() -> Rgba { rgb(0x8db4ff) }
pub fn star_doc() -> Rgba { rgb(0xe9c49f) }

/// 星座ごとの色。ツリーの点、タブ、星図の星座名を同じ色でつなぐ。
/// 彩度と明るさを揃え、どれも暗い面の上で読める
const HUES: [u32; 10] = [
    0xa8a4ff, 0x73e2d4, 0xe9c49f, 0xf2a7c3, 0x8db4ff, 0xa9d99a, 0xf4a38c, 0xe8d583, 0xc9a0f0,
    0x7fd0ef,
];

pub fn hue(constellation: usize) -> Rgba { rgb(HUES[constellation % HUES.len()]) }

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
