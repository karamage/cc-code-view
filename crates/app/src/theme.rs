//! デザイントークン。色と時間の値はここだけに書く。
#![allow(dead_code)]
use gpui::{Rgba, rgb};

pub fn space() -> Rgba { rgb(0x05060c) }
pub fn nebula() -> Rgba { rgb(0x141633) }
pub fn panel() -> Rgba { rgb(0x0b0c18) }
pub fn hairline() -> Rgba { rgb(0x1a1c30) }
pub fn text() -> Rgba { rgb(0xdadcf0) }
/// space の上で 4.5:1 以上
pub fn text_muted() -> Rgba { rgb(0x8d91b0) }
pub fn violet() -> Rgba { rgb(0xa8a4ff) }
pub fn teal() -> Rgba { rgb(0x73e2d4) }
pub fn white() -> Rgba { rgb(0xffffff) }
pub const CODE_FONT: &str = "JetBrains Mono";
pub const TRANSITION_MS: u64 = 420;
