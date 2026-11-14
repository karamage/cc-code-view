//! 配置済みの図を描き、触れるようにする。要素に触れるとつながる線が灯り、
//! データの向きに光の粒が流れる。ファイルに結び付いた要素を押すとコードを開く。
use crate::theme;
use ccv_core::diagram::ast::{Kind, Line, Marker, Shape};
use ccv_core::diagram::scene::{
    MONO_TEXT, Measure, NODE_TEXT, Rect, SMALL_TEXT, Scene, SceneEdge, SceneNode, line_height,
    point_along, trim_path,
};
use gpui::{
    App, BorderStyle, Bounds, BoxShadow, ContentMask, Context, EventEmitter, Font, FontFeatures,
    FontStyle, FontWeight, Hsla, PathBuilder, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,
    TextRun, Window, canvas, div, point, prelude::*, px, quad, size,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PAD: f32 = 20.0;
const MAX_BASE_SCALE: f32 = 1.2;
const APPEAR: Duration = Duration::from_millis(1100);

pub enum DiagramEvent {
    Open { path: String, line: u32 },
    Hover(Option<String>),
    Expand,
}

/// gpui の書体で文字幅を測る。和文は全角で数える。
pub struct GpuiMeasure {
    system: Arc<gpui::WindowTextSystem>,
    ui: gpui::FontId,
    bold: gpui::FontId,
    mono: gpui::FontId,
}

impl GpuiMeasure {
    pub fn new(window: &Window) -> GpuiMeasure {
        let system = window.text_system().clone();
        let ui = system.resolve_font(&ui_font(FontWeight::NORMAL));
        let bold = system.resolve_font(&ui_font(FontWeight::SEMIBOLD));
        let mono = system.resolve_font(&mono_font());
        GpuiMeasure { system, ui, bold, mono }
    }
}

impl Measure for GpuiMeasure {
    fn advance(&self, c: char, size: f32, mono: bool, bold: bool) -> f32 {
        if !c.is_ascii() {
            return size;
        }
        let font = if mono {
            self.mono
        } else if bold {
            self.bold
        } else {
            self.ui
        };
        self.system.advance(font, px(size), c).map_or(size * 0.56, |a| f32::from(a.width))
    }
}

fn ui_font(weight: FontWeight) -> Font {
    Font {
        family: theme::UI_FONT.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

fn mono_font() -> Font {
    Font {
        family: theme::CODE_FONT.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

pub struct DiagramView {
    scene: Arc<Scene>,
    /// 要素の番号 → ファイルと行
    links: HashMap<usize, (String, u32)>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    appear: Rc<Cell<Option<Instant>>>,
    zoom: f32,
    pan: (f32, f32),
    hovered: Option<usize>,
    pointer_inside: bool,
    drag: Option<(f32, f32)>,
    dragged: bool,
    reduce_motion: bool,
    /// データの流れを表す図。触れている間、すべての線に粒を流す
    flow: bool,
    /// 拡大表示の中で使う。拡大のボタンを出さない
    expanded: bool,
}

impl EventEmitter<DiagramEvent> for DiagramView {}

impl DiagramView {
    pub fn new(
        scene: Arc<Scene>,
        links: HashMap<usize, (String, u32)>,
        flow: bool,
        reduce_motion: bool,
    ) -> DiagramView {
        DiagramView {
            scene,
            links,
            viewport: Rc::new(Cell::new(Bounds::default())),
            appear: Rc::new(Cell::new(None)),
            zoom: 1.0,
            pan: (0.0, 0.0),
            hovered: None,
            pointer_inside: false,
            drag: None,
            dragged: false,
            reduce_motion,
            flow,
            expanded: false,
        }
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.expanded = expanded;
        self.reset(cx);
    }

    pub fn scene(&self) -> &Arc<Scene> {
        &self.scene
    }

    /// 本文の幅に置いたときの高さ。図の縦横比を保ち、極端に高くしない。
    pub fn natural_height(&self, width: f32) -> f32 {
        let s = &self.scene;
        let scale = ((width - PAD * 2.0) / s.width.max(1.0)).clamp(0.55, MAX_BASE_SCALE);
        (s.height * scale + PAD * 2.0).clamp(220.0, 640.0)
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        cx.notify();
    }

    fn zoom_by(&mut self, factor: f32, around: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let b = self.viewport.get();
        let before = transform(&self.scene, b, self.zoom, self.pan, self.expanded);
        let new_zoom = (self.zoom * factor).clamp(0.4, 6.0);
        let cursor = around.unwrap_or((
            f32::from(b.origin.x + b.size.width / 2.0),
            f32::from(b.origin.y + b.size.height / 2.0),
        ));
        let world = before.to_world(cursor);
        self.zoom = new_zoom;
        let after = transform(&self.scene, b, self.zoom, (0.0, 0.0), self.expanded);
        let screen = after.to_screen(world);
        self.pan = (cursor.0 - screen.0, cursor.1 - screen.1);
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let (dx, dy) = match e.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
        };
        if e.modifiers.platform || e.modifiers.control {
            let at = (f32::from(e.position.x), f32::from(e.position.y));
            self.zoom_by((dy * 0.006).exp(), Some(at), cx);
            cx.stop_propagation();
        } else if self.expanded || self.zoom > 1.01 {
            // 拡大しているときは図を動かす。そうでなければ本文のスクロールに任せる
            self.pan.0 += dx;
            self.pan.1 += dy;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn node_at(&self, position: Point<Pixels>) -> Option<usize> {
        let t = transform(&self.scene, self.viewport.get(), self.zoom, self.pan, self.expanded);
        let (x, y) = t.to_world((f32::from(position.x), f32::from(position.y)));
        self.scene.node_at(x, y)
    }

    fn set_hovered(&mut self, hovered: Option<usize>, cx: &mut Context<Self>) {
        if hovered != self.hovered {
            self.hovered = hovered;
            let path = hovered.and_then(|i| self.links.get(&i)).map(|(p, _)| p.clone());
            cx.emit(DiagramEvent::Hover(path));
            cx.notify();
        }
    }
}

#[derive(Clone, Copy)]
struct Transform {
    scale: f32,
    origin: (f32, f32),
}

impl Transform {
    fn to_screen(self, p: (f32, f32)) -> (f32, f32) {
        (self.origin.0 + p.0 * self.scale, self.origin.1 + p.1 * self.scale)
    }
    fn to_world(self, p: (f32, f32)) -> (f32, f32) {
        ((p.0 - self.origin.0) / self.scale, (p.1 - self.origin.1) / self.scale)
    }
    fn rect(self, r: &Rect) -> Bounds<Pixels> {
        let (x, y) = self.to_screen((r.x, r.y));
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(r.w * self.scale), px(r.h * self.scale)),
        }
    }
}

fn transform(
    scene: &Scene,
    b: Bounds<Pixels>,
    zoom: f32,
    pan: (f32, f32),
    expanded: bool,
) -> Transform {
    let (vw, vh) = (f32::from(b.size.width), f32::from(b.size.height));
    // 文字が読めないほど縮めない。はみ出した分はドラッグで動かす
    let base = ((vw - PAD * 2.0) / scene.width.max(1.0))
        .min((vh - PAD * 2.0) / scene.height.max(1.0))
        .min(MAX_BASE_SCALE)
        .max(if expanded { 0.25 } else { 0.55 });
    let scale = base * zoom;
    // 収まるときは中央に、はみ出すときは左上をそろえて始まりを見せる
    let place = |view: f32, content: f32| {
        if content <= view - PAD * 2.0 { (view - content) / 2.0 } else { PAD }
    };
    let origin = (
        f32::from(b.origin.x) + place(vw, scene.width * scale) + pan.0,
        f32::from(b.origin.y) + place(vh, scene.height * scale) + pan.1,
    );
    Transform { scale, origin }
}

/// 図の中の色。囲みの色は星座の色と同じ並びを使う
fn node_hue(scene: &Scene, n: &SceneNode) -> Hsla {
    match (n.hue, scene.kind) {
        (Some(h), _) => theme::hue(h + 1).into(),
        (None, Kind::Entity) => theme::teal().into(),
        (None, Kind::Sequence) => theme::tok_type().into(),
        _ => theme::violet().into(),
    }
}

struct Frame<'a> {
    scene: &'a Scene,
    t: Transform,
    links: &'a HashMap<usize, (String, u32)>,
    hovered: Option<usize>,
    /// 0.0〜1.0。現れる演出の進み
    appear: f32,
    now: f32,
    particles: bool,
    flow_all: bool,
}

fn shape_text(
    window: &mut Window,
    text: &str,
    size_px: f32,
    color: Hsla,
    weight: FontWeight,
    mono: bool,
) -> gpui::ShapedLine {
    let font = if mono { mono_font() } else { ui_font(weight) };
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text.to_string().into(), px(size_px), &[run], None)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    at: (f32, f32),
    size_px: f32,
    color: Hsla,
    weight: FontWeight,
    mono: bool,
    centered: bool,
) {
    if text.is_empty() || color.a <= 0.01 {
        return;
    }
    let line = shape_text(window, text, size_px, color, weight, mono);
    let x = if centered { at.0 - f32::from(line.width) / 2.0 } else { at.0 };
    let _ = line.paint(point(px(x), px(at.1)), px(size_px * 1.45), window, cx);
}

fn with_alpha(c: Hsla, a: f32) -> Hsla {
    Hsla { a: c.a * a, ..c }
}

fn pt(p: (f32, f32)) -> Point<Pixels> {
    point(px(p.0), px(p.1))
}

fn polygon(
    window: &mut Window,
    points: &[(f32, f32)],
    fill: Option<Hsla>,
    stroke: Option<(Hsla, f32)>,
) {
    if let Some(color) = fill {
        let mut b = PathBuilder::fill();
        b.add_polygon(&points.iter().map(|p| pt(*p)).collect::<Vec<_>>(), true);
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    }
    if let Some((color, width)) = stroke {
        let mut b = PathBuilder::stroke(px(width));
        b.add_polygon(&points.iter().map(|p| pt(*p)).collect::<Vec<_>>(), true);
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    }
}

fn segment(window: &mut Window, a: (f32, f32), b: (f32, f32), color: Hsla, width: f32) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(pt(a));
    path.line_to(pt(b));
    if let Ok(p) = path.build() {
        window.paint_path(p, color);
    }
}

fn disc(window: &mut Window, c: (f32, f32), r: f32, fill: Hsla, border: Option<(Hsla, f32)>) {
    let b =
        Bounds { origin: point(px(c.0 - r), px(c.1 - r)), size: size(px(r * 2.0), px(r * 2.0)) };
    let (bc, bw) = border.unwrap_or((gpui::transparent_black(), 0.0));
    window.paint_quad(quad(b, px(r), fill, px(bw), bc, BorderStyle::Solid));
}

fn glow(window: &mut Window, c: (f32, f32), r: f32, blur: f32, color: Hsla) {
    let b =
        Bounds { origin: point(px(c.0 - r), px(c.1 - r)), size: size(px(r * 2.0), px(r * 2.0)) };
    window.paint_shadows(
        b,
        gpui::Corners::all(px(r)),
        &[BoxShadow {
            color,
            offset: point(px(0.), px(0.)),
            blur_radius: px(blur),
            spread_radius: px(0.),
        }],
    );
}

/// 端の向きの単位ベクトル。`tip` が端の点、`from` はその手前の点。
fn direction(tip: (f32, f32), from: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 { (0.0, 1.0) } else { (dx / len, dy / len) }
}

fn marker(
    window: &mut Window,
    m: Marker,
    tip: (f32, f32),
    dir: (f32, f32),
    color: Hsla,
    bg: Hsla,
    s: f32,
) {
    let (ux, uy) = dir;
    let (nx, ny) = (-uy, ux);
    let at = |back: f32, side: f32| {
        (tip.0 - ux * back * s + nx * side * s, tip.1 - uy * back * s + ny * side * s)
    };
    let w = 1.25 * s.max(0.6);
    match m {
        Marker::None => {}
        Marker::Arrow => {
            polygon(window, &[tip, at(9.0, 4.0), at(7.0, 0.0), at(9.0, -4.0)], Some(color), None)
        }
        Marker::OpenArrow => {
            segment(window, tip, at(9.0, 4.5), color, w);
            segment(window, tip, at(9.0, -4.5), color, w);
        }
        Marker::Triangle => {
            polygon(window, &[tip, at(12.0, 6.5), at(12.0, -6.5)], Some(bg), Some((color, w)))
        }
        Marker::Diamond => polygon(
            window,
            &[tip, at(7.0, 4.5), at(14.0, 0.0), at(7.0, -4.5)],
            Some(color),
            Some((color, w)),
        ),
        Marker::DiamondOpen => polygon(
            window,
            &[tip, at(7.0, 4.5), at(14.0, 0.0), at(7.0, -4.5)],
            Some(bg),
            Some((color, w)),
        ),
        Marker::Circle => disc(window, at(4.5, 0.0), 4.0 * s, bg, Some((color, w))),
        Marker::Cross => {
            segment(window, at(2.0, 4.0), at(10.0, -4.0), color, w);
            segment(window, at(2.0, -4.0), at(10.0, 4.0), color, w);
        }
        Marker::One | Marker::ZeroOrOne | Marker::OneOrMany | Marker::ZeroOrMany => {
            let many = matches!(m, Marker::OneOrMany | Marker::ZeroOrMany);
            let zero = matches!(m, Marker::ZeroOrOne | Marker::ZeroOrMany);
            if many {
                segment(window, at(0.0, 6.0), at(11.0, 0.0), color, w);
                segment(window, at(0.0, -6.0), at(11.0, 0.0), color, w);
                segment(window, at(0.0, 0.0), at(11.0, 0.0), color, w);
            } else {
                segment(window, at(5.0, 6.0), at(5.0, -6.0), color, w);
            }
            if zero {
                disc(window, at(17.0, 0.0), 4.0 * s, bg, Some((color, w)));
            } else {
                segment(window, at(13.0, 6.0), at(13.0, -6.0), color, w);
            }
        }
    }
}

fn stroke_path(
    window: &mut Window,
    f: &Frame,
    path: &[(f32, f32)],
    line: Line,
    width: f32,
    color: Hsla,
) {
    if path.len() < 4 {
        return;
    }
    let s = f.t.scale.max(0.5);
    let mut b = PathBuilder::stroke(px(width));
    if line == Line::Dotted {
        b = b.dash_array(&[px(4.0 * s), px(4.0 * s)]);
    }
    b.move_to(pt(f.t.to_screen(path[0])));
    for k in (1..path.len() - 2).step_by(3) {
        b.cubic_bezier_to(
            pt(f.t.to_screen(path[k + 2])),
            pt(f.t.to_screen(path[k])),
            pt(f.t.to_screen(path[k + 1])),
        );
    }
    if let Ok(p) = b.build() {
        window.paint_path(p, color);
    }
}

fn related(f: &Frame, e: &SceneEdge) -> Option<bool> {
    f.hovered.map(|h| e.from == h || e.to == h)
}

/// 要素ごとの現れる進み。上流から順に灯す。
fn reveal(f: &Frame, rank: usize) -> f32 {
    if f.appear >= 1.0 {
        return 1.0;
    }
    let ranks = f.scene.ranks.max(1) as f32;
    let at = rank as f32 / ranks;
    ((f.appear * 1.35 - at * 0.75) / 0.35).clamp(0.0, 1.0)
}

fn paint_edge(window: &mut Window, cx: &mut App, f: &Frame, e: &SceneEdge) {
    let bg: Hsla = theme::panel().into();
    let rel = related(f, e);
    let base: Hsla = theme::text_muted().into();
    let (color, alpha) = match rel {
        Some(true) => (theme::teal().into(), 1.0),
        Some(false) => (base, 0.16),
        None => (base, 0.62),
    };
    let progress = reveal(f, e.rank);
    if progress <= 0.0 {
        return;
    }
    let path = trim_path(&e.path, progress);
    let s = f.t.scale;
    let width = match e.line {
        Line::Thick => 2.4,
        _ => 1.25,
    } * s.clamp(0.6, 1.6)
        + if rel == Some(true) { 0.5 } else { 0.0 };
    let color = with_alpha(color, alpha);
    stroke_path(window, f, &path, e.line, width, color);
    if progress >= 1.0 {
        let n = e.path.len();
        let tip = f.t.to_screen(e.path[n - 1]);
        let before = f.t.to_screen(if e.path[n - 2] != e.path[n - 1] {
            e.path[n - 2]
        } else {
            e.path[n.saturating_sub(4)]
        });
        marker(window, e.end, tip, direction(tip, before), color, bg, s.clamp(0.6, 1.6));
        let start = f.t.to_screen(e.path[0]);
        let after =
            f.t.to_screen(if e.path[1] != e.path[0] { e.path[1] } else { e.path[3.min(n - 1)] });
        marker(window, e.start, start, direction(start, after), color, bg, s.clamp(0.6, 1.6));
        if let Some(number) = e.number {
            let r = 8.0 * s.clamp(0.7, 1.4);
            let c = (
                start.0 - if e.path[3].0 >= e.path[0].0 { -r - 4.0 } else { r + 4.0 } * 0.0 + 0.0,
                start.1,
            );
            disc(
                window,
                (c.0, c.1 - r - 2.0),
                r,
                with_alpha(theme::teal().into(), if rel == Some(false) { 0.3 } else { 0.9 }),
                None,
            );
            let size_px = 9.5 * s.clamp(0.7, 1.4);
            draw_text(
                window,
                cx,
                &number.to_string(),
                (c.0, c.1 - r - 2.0 - size_px * 0.72),
                size_px,
                theme::space().into(),
                FontWeight::BOLD,
                true,
                true,
            );
        }
    }
    let label_alpha =
        ((progress - 0.6) / 0.4).clamp(0.0, 1.0) * if rel == Some(false) { 0.3 } else { 1.0 };
    for (label, small) in [(&e.label, false), (&e.start_label, true), (&e.end_label, true)] {
        let Some(label) = label else { continue };
        let b = f.t.rect(&label.rect);
        if !small {
            window.paint_quad(quad(
                b,
                px(5.0 * s),
                with_alpha(theme::panel().into(), 0.94 * label_alpha),
                px(1.0),
                with_alpha(
                    if rel == Some(true) {
                        theme::teal().into()
                    } else {
                        theme::hairline_strong().into()
                    },
                    label_alpha,
                ),
                BorderStyle::Solid,
            ));
        }
        let size_px = SMALL_TEXT * s;
        let lh = line_height(SMALL_TEXT) * s;
        let text_color = with_alpha(
            if rel == Some(true) { theme::white().into() } else { theme::text_muted().into() },
            label_alpha,
        );
        let top = f32::from(b.origin.y)
            + (f32::from(b.size.height) - lh * label.lines.len() as f32) / 2.0;
        for (i, l) in label.lines.iter().enumerate() {
            draw_text(
                window,
                cx,
                l,
                (f32::from(b.origin.x + b.size.width / 2.0), top + i as f32 * lh),
                size_px,
                text_color,
                FontWeight::NORMAL,
                false,
                true,
            );
        }
    }
}

fn paint_node(window: &mut Window, cx: &mut App, f: &Frame, index: usize, n: &SceneNode) {
    let reveal_t = reveal(f, n.rank);
    if reveal_t <= 0.0 {
        return;
    }
    let source = n.mirror_of.unwrap_or(index);
    let hot = f.hovered == Some(source);
    let dimmed = f.hovered.is_some_and(|h| {
        h != source
            && !f
                .scene
                .edges
                .iter()
                .any(|e| (e.from == h && e.to == source) || (e.to == h && e.from == source))
    });
    let fade = reveal_t * if dimmed { 0.38 } else { 1.0 };
    let hue = node_hue(f.scene, n);
    let s = f.t.scale;
    let mut rect = n.rect;
    rect.y += (1.0 - reveal_t) * 8.0;
    let b = f.t.rect(&rect);
    let (x, y, w, h) = (
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    );
    let fill: Hsla = theme::panel_raised().into();
    let fill = with_alpha(fill, fade.max(0.0));
    let tint = with_alpha(hue, 0.075 * fade);
    let border =
        if hot { with_alpha(theme::teal().into(), fade) } else { with_alpha(hue, 0.58 * fade) };
    let bw = if hot { 1.6 } else { 1.0 };
    if hot {
        glow(
            window,
            (x + w / 2.0, y + h / 2.0),
            w.max(h) * 0.4,
            28.0 * s,
            with_alpha(theme::teal().into(), 0.22),
        );
    }
    let text_color = with_alpha(theme::text().into(), fade);
    let rounded = |window: &mut Window, radius: f32| {
        window.paint_quad(quad(
            b,
            px(radius),
            fill,
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
        window.paint_quad(quad(b, px(radius), tint, px(bw), border, BorderStyle::Solid));
    };
    let poly = |window: &mut Window, pts: &[(f32, f32)]| {
        polygon(window, pts, Some(fill), None);
        polygon(window, pts, Some(tint), Some((border, bw)));
    };
    match n.shape {
        Shape::Rect | Shape::Participant => rounded(window, 6.0 * s),
        Shape::Round => rounded(window, 14.0 * s),
        Shape::Stadium => rounded(window, h / 2.0),
        Shape::Circle => rounded(window, w.min(h) / 2.0),
        Shape::Subroutine => {
            rounded(window, 4.0 * s);
            segment(window, (x + 7.0 * s, y), (x + 7.0 * s, y + h), border, bw);
            segment(window, (x + w - 7.0 * s, y), (x + w - 7.0 * s, y + h), border, bw);
        }
        Shape::Diamond => poly(
            window,
            &[(x + w / 2.0, y), (x + w, y + h / 2.0), (x + w / 2.0, y + h), (x, y + h / 2.0)],
        ),
        Shape::Hexagon => {
            let k = (h * 0.32).min(w * 0.25);
            poly(
                window,
                &[
                    (x + k, y),
                    (x + w - k, y),
                    (x + w, y + h / 2.0),
                    (x + w - k, y + h),
                    (x + k, y + h),
                    (x, y + h / 2.0),
                ],
            );
        }
        Shape::Parallelogram => {
            let k = 10.0 * s;
            poly(window, &[(x + k, y), (x + w, y), (x + w - k, y + h), (x, y + h)]);
        }
        Shape::Flag => {
            let k = 12.0 * s;
            poly(window, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x + k, y + h / 2.0)]);
        }
        Shape::Cylinder => {
            let e = 7.0 * s;
            let mut body = PathBuilder::fill();
            body.move_to(pt((x, y + e)));
            body.cubic_bezier_to(
                pt((x + w, y + e)),
                pt((x, y - e / 3.0)),
                pt((x + w, y - e / 3.0)),
            );
            body.line_to(pt((x + w, y + h - e)));
            body.cubic_bezier_to(
                pt((x, y + h - e)),
                pt((x + w, y + h + e / 3.0)),
                pt((x, y + h + e / 3.0)),
            );
            body.close();
            if let Ok(p) = body.build() {
                window.paint_path(p, fill);
            }
            let mut outline = PathBuilder::stroke(px(bw));
            outline.move_to(pt((x, y + e)));
            outline.cubic_bezier_to(
                pt((x + w, y + e)),
                pt((x, y - e / 3.0)),
                pt((x + w, y - e / 3.0)),
            );
            outline.cubic_bezier_to(
                pt((x, y + e)),
                pt((x + w, y + e * 2.33)),
                pt((x, y + e * 2.33)),
            );
            outline.move_to(pt((x + w, y + e)));
            outline.line_to(pt((x + w, y + h - e)));
            outline.cubic_bezier_to(
                pt((x, y + h - e)),
                pt((x + w, y + h + e / 3.0)),
                pt((x, y + h + e / 3.0)),
            );
            outline.line_to(pt((x, y + e)));
            if let Ok(p) = outline.build() {
                window.paint_path(p, border);
            }
        }
        Shape::Table => {
            rounded(window, 8.0 * s);
        }
        Shape::Start => disc(
            window,
            (x + w / 2.0, y + h / 2.0),
            w / 2.0,
            with_alpha(theme::text().into(), fade),
            None,
        ),
        Shape::End => {
            disc(
                window,
                (x + w / 2.0, y + h / 2.0),
                w / 2.0,
                with_alpha(theme::panel().into(), fade),
                Some((with_alpha(theme::text().into(), fade), 1.4)),
            );
            disc(
                window,
                (x + w / 2.0, y + h / 2.0),
                w / 2.0 - 4.0 * s,
                with_alpha(theme::text().into(), fade),
                None,
            );
        }
        Shape::Bar => window.paint_quad(quad(
            b,
            px(2.0 * s),
            with_alpha(theme::text().into(), fade),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        )),
        Shape::Actor => {
            // 人の形を線で描く
            let cxp = x + w / 2.0;
            let top = y + 4.0 * s;
            let c = with_alpha(hue, fade);
            disc(window, (cxp, top + 6.0 * s), 6.0 * s, gpui::transparent_black(), Some((c, 1.3)));
            segment(window, (cxp, top + 12.0 * s), (cxp, top + 24.0 * s), c, 1.3);
            segment(
                window,
                (cxp - 9.0 * s, top + 16.0 * s),
                (cxp + 9.0 * s, top + 16.0 * s),
                c,
                1.3,
            );
            segment(window, (cxp, top + 24.0 * s), (cxp - 7.0 * s, top + 32.0 * s), c, 1.3);
            segment(window, (cxp, top + 24.0 * s), (cxp + 7.0 * s, top + 32.0 * s), c, 1.3);
        }
    }

    // 文字
    let size_px = NODE_TEXT * s;
    let lh = line_height(NODE_TEXT) * s;
    if n.shape == Shape::Table {
        let mut cy = y + 10.0 * s;
        let header_h = 10.0 * s
            + n.title.len() as f32 * lh
            + 10.0 * s
            + if n.stereotype.is_some() { line_height(SMALL_TEXT) * s } else { 0.0 };
        let band = Bounds { origin: b.origin, size: size(b.size.width, px(header_h)) };
        let r = px(8.0 * s);
        window.paint_quad(quad(
            band,
            gpui::Corners { top_left: r, top_right: r, bottom_left: px(0.), bottom_right: px(0.) },
            with_alpha(hue, 0.16 * fade),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
        if let Some(st) = &n.stereotype {
            draw_text(
                window,
                cx,
                &format!("«{st}»"),
                (x + w / 2.0, cy),
                SMALL_TEXT * s,
                with_alpha(theme::text_muted().into(), fade),
                FontWeight::NORMAL,
                false,
                true,
            );
            cy += line_height(SMALL_TEXT) * s;
        }
        for l in &n.title {
            draw_text(
                window,
                cx,
                l,
                (x + w / 2.0, cy),
                size_px,
                with_alpha(theme::white().into(), fade),
                FontWeight::SEMIBOLD,
                false,
                true,
            );
            cy += lh;
        }
        cy = y + header_h;
        let row_h = (line_height(MONO_TEXT) + 3.0) * s;
        for section in n.sections.iter().filter(|s| !s.is_empty()) {
            segment(window, (x, cy), (x + w, cy), with_alpha(hue, 0.35 * fade), 1.0);
            cy += 7.0 * s;
            for row in section {
                for (c, cell) in row.iter().enumerate() {
                    let col_x = x + n.columns.get(c).copied().unwrap_or(12.0) * s;
                    let color: Hsla = match c {
                        0 if row.len() > 1 => theme::tok_type().into(),
                        2 => theme::tok_number().into(),
                        3.. => theme::tok_comment().into(),
                        _ => theme::tok_plain().into(),
                    };
                    draw_text(
                        window,
                        cx,
                        cell,
                        (col_x, cy + 1.5 * s),
                        MONO_TEXT * s,
                        with_alpha(color, fade),
                        FontWeight::NORMAL,
                        true,
                        false,
                    );
                }
                cy += row_h;
            }
            cy += 7.0 * s;
        }
    } else if !n.title.is_empty() {
        let block = n.title.len() as f32 * lh;
        let top = match n.shape {
            Shape::Actor => y + h - block - 6.0 * s,
            Shape::Cylinder => y + (h - block) / 2.0 + 4.0 * s,
            _ => y + (h - block) / 2.0,
        };
        let weight = if matches!(n.shape, Shape::Participant | Shape::Actor) {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        let color = if hot { with_alpha(theme::white().into(), fade) } else { text_color };
        for (i, l) in n.title.iter().enumerate() {
            draw_text(
                window,
                cx,
                l,
                (x + w / 2.0, top + i as f32 * lh),
                size_px,
                color,
                weight,
                false,
                true,
            );
        }
    }

    // ファイルに結び付いた要素には、右上に小さな印を付ける
    if f.links.contains_key(&source)
        && n.mirror_of.is_none()
        && !matches!(n.shape, Shape::Start | Shape::End | Shape::Bar)
    {
        let c = with_alpha(theme::teal().into(), if hot { 1.0 } else { 0.75 } * fade);
        let (ax, ay) = match n.shape {
            Shape::Diamond => (x + w * 0.75, y + h * 0.18),
            Shape::Circle => (x + w * 0.82, y + h * 0.16),
            _ => (x + w - 8.0 * s, y + 8.0 * s),
        };
        let k = 3.2 * s.clamp(0.7, 1.4);
        segment(window, (ax - k, ay + k), (ax + k, ay - k), c, 1.2);
        segment(window, (ax, ay - k), (ax + k, ay - k), c, 1.2);
        segment(window, (ax + k, ay - k), (ax + k, ay), c, 1.2);
    }
}

fn paint_scene(bounds: Bounds<Pixels>, f: &Frame, window: &mut Window, cx: &mut App) {
    // 方眼の点。設計図の紙を思わせつつ、主張しない
    let step = 22.0 * f.t.scale.clamp(0.6, 2.4);
    let (bx, by) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let ox = (f.t.origin.0 - bx).rem_euclid(step);
    let oy = (f.t.origin.1 - by).rem_euclid(step);
    let dot: Hsla = theme::alpha(theme::white(), 0.05);
    let mut yy = by + oy;
    while yy < by + bh {
        let mut xx = bx + ox;
        while xx < bx + bw {
            window.paint_quad(quad(
                Bounds { origin: point(px(xx - 0.6), px(yy - 0.6)), size: size(px(1.2), px(1.2)) },
                px(0.6),
                dot,
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
            xx += step;
        }
        yy += step;
    }

    let s = f.t.scale;
    let scene = f.scene;
    // 囲みと枠
    for g in &scene.groups {
        let b = f.t.rect(&g.rect);
        let appear = reveal(f, 0);
        let fade = appear * if f.hovered.is_some() { 0.6 } else { 1.0 };
        let hue: Hsla = match g.keyword {
            Some(_) => theme::text_muted().into(),
            None => theme::hue(g.hue + 1).into(),
        };
        let (x, y) = (f32::from(b.origin.x), f32::from(b.origin.y));
        match &g.keyword {
            None => {
                window.paint_quad(quad(
                    b,
                    px(10.0 * s),
                    with_alpha(hue, 0.035 * fade),
                    px(1.0),
                    with_alpha(hue, 0.3 * fade),
                    BorderStyle::Solid,
                ));
                draw_text(
                    window,
                    cx,
                    &g.label,
                    (x + 12.0 * s, y + 6.0 * s),
                    SMALL_TEXT * s,
                    with_alpha(hue, 0.95 * fade),
                    FontWeight::SEMIBOLD,
                    false,
                    false,
                );
            }
            Some(keyword) => {
                window.paint_quad(quad(
                    b,
                    px(6.0 * s),
                    with_alpha(hue, 0.025 * fade),
                    px(1.0),
                    with_alpha(hue, 0.32 * fade),
                    BorderStyle::Solid,
                ));
                let tag = shape_text(
                    window,
                    keyword,
                    SMALL_TEXT * s,
                    with_alpha(theme::white().into(), fade),
                    FontWeight::SEMIBOLD,
                    false,
                );
                let tw = f32::from(tag.width) + 14.0 * s;
                let th = 20.0 * s;
                window.paint_quad(quad(
                    Bounds { origin: b.origin, size: size(px(tw), px(th)) },
                    gpui::Corners {
                        top_left: px(6.0 * s),
                        top_right: px(0.),
                        bottom_left: px(0.),
                        bottom_right: px(6.0 * s),
                    },
                    with_alpha(theme::violet().into(), 0.28 * fade),
                    px(0.0),
                    gpui::transparent_black(),
                    BorderStyle::Solid,
                ));
                let _ = tag.paint(
                    point(px(x + 7.0 * s), px(y + 2.5 * s)),
                    px(SMALL_TEXT * s * 1.45),
                    window,
                    cx,
                );
                if !g.label.is_empty() {
                    draw_text(
                        window,
                        cx,
                        &format!("[{}]", g.label),
                        (x + tw + 8.0 * s, y + 2.5 * s),
                        SMALL_TEXT * s,
                        with_alpha(theme::text_muted().into(), fade),
                        FontWeight::NORMAL,
                        false,
                        false,
                    );
                }
                for (dy, label) in &g.dividers {
                    let yy = f.t.to_screen((0.0, *dy)).1;
                    let mut path =
                        PathBuilder::stroke(px(1.0)).dash_array(&[px(5.0 * s), px(4.0 * s)]);
                    path.move_to(point(b.origin.x, px(yy)));
                    path.line_to(point(b.origin.x + b.size.width, px(yy)));
                    if let Ok(p) = path.build() {
                        window.paint_path(p, with_alpha(hue, 0.4 * fade));
                    }
                    if !label.is_empty() {
                        draw_text(
                            window,
                            cx,
                            &format!("[{label}]"),
                            (x + 10.0 * s, yy + 3.0 * s),
                            SMALL_TEXT * s,
                            with_alpha(theme::text_muted().into(), fade),
                            FontWeight::NORMAL,
                            false,
                            false,
                        );
                    }
                }
            }
        }
    }
    // 生存線
    for l in &scene.lifelines {
        let a = f.t.to_screen((l.x, l.top));
        let b = f.t.to_screen((l.x, l.bottom));
        let hot = f.hovered == Some(l.node);
        let mut path = PathBuilder::stroke(px(if hot { 1.4 } else { 1.0 }))
            .dash_array(&[px(3.0 * s), px(4.0 * s)]);
        path.move_to(pt(a));
        path.line_to(pt(b));
        if let Ok(p) = path.build() {
            window.paint_path(
                p,
                if hot {
                    theme::alpha(theme::teal(), 0.7)
                } else {
                    theme::alpha(theme::text_muted(), 0.28 * reveal(f, 0))
                },
            );
        }
    }
    // 注釈
    for n in &scene.notes {
        let b = f.t.rect(&n.rect);
        let fade = reveal(f, 1);
        window.paint_quad(quad(
            b,
            px(4.0 * s),
            with_alpha(theme::tok_number().into(), 0.1 * fade),
            px(1.0),
            with_alpha(theme::tok_number().into(), 0.42 * fade),
            BorderStyle::Solid,
        ));
        let lh = line_height(SMALL_TEXT) * s;
        let top =
            f32::from(b.origin.y) + (f32::from(b.size.height) - lh * n.lines.len() as f32) / 2.0;
        for (i, l) in n.lines.iter().enumerate() {
            draw_text(
                window,
                cx,
                l,
                (f32::from(b.origin.x + b.size.width / 2.0), top + i as f32 * lh),
                SMALL_TEXT * s,
                with_alpha(theme::tok_number().into(), fade),
                FontWeight::NORMAL,
                false,
                true,
            );
        }
    }
    // 線は要素の下に。触れている要素につながる線は最後に重ねる
    for e in scene.edges.iter().filter(|e| related(f, e) != Some(true)) {
        paint_edge(window, cx, f, e);
    }
    for e in scene.edges.iter().filter(|e| related(f, e) == Some(true)) {
        paint_edge(window, cx, f, e);
    }
    for (i, n) in scene.nodes.iter().enumerate() {
        paint_node(window, cx, f, i, n);
    }
    // 光の粒。データの向き（線の始点から終点）へ流す
    if f.particles {
        for (k, e) in scene.edges.iter().enumerate() {
            let on = related(f, e) == Some(true) || (f.flow_all && f.hovered.is_none());
            if !on || e.line == Line::Invisible {
                continue;
            }
            let speed = 0.42;
            for j in 0..2 {
                let phase = (f.now * speed + j as f32 * 0.5 + k as f32 * 0.137).fract();
                if let Some((p, _)) = point_along(&e.path, phase) {
                    let q = f.t.to_screen(p);
                    let alpha = (phase * std::f32::consts::PI).sin();
                    glow(
                        window,
                        q,
                        2.5 * s.clamp(0.7, 1.4),
                        9.0,
                        theme::alpha(theme::teal(), 0.55 * alpha),
                    );
                    disc(
                        window,
                        q,
                        1.9 * s.clamp(0.7, 1.4),
                        theme::alpha(theme::white(), 0.95 * alpha),
                        None,
                    );
                }
            }
        }
    }
}

impl Render for DiagramView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let start = self.appear.get();
        let appear = match (start, self.reduce_motion) {
            (_, true) => 1.0,
            (None, false) => 0.0,
            (Some(s), false) => (s.elapsed().as_secs_f32() / APPEAR.as_secs_f32()).min(1.0),
        };
        let particles =
            !self.reduce_motion && (self.hovered.is_some() || (self.flow && self.pointer_inside));
        // 描画結果は使い回されるので、動いている間は次のフレームで描き直しを頼む
        if (start.is_some() && appear < 1.0) || particles {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        let entity = cx.entity_id();
        let scene = self.scene.clone();
        let links = self.links.clone();
        let hovered = self.hovered;
        let viewport = self.viewport.clone();
        let appear_cell = self.appear.clone();
        let zoom = self.zoom;
        let pan = self.pan;
        let flow_all = self.flow;
        let now = crate::diagram_view::clock();
        let pointer = hovered.is_some_and(|h| self.links.contains_key(&h));

        // ファイルに結び付いた要素の説明
        let tooltip = hovered.and_then(|h| {
            let (path, line) = self.links.get(&h)?.clone();
            let b = self.viewport.get();
            let t = transform(&self.scene, b, self.zoom, self.pan, self.expanded);
            let r = t.rect(&self.scene.nodes[h].rect);
            let left = f32::from(r.origin.x - b.origin.x);
            let top = f32::from(r.origin.y - b.origin.y + r.size.height) + 8.0;
            let max_left = f32::from(b.size.width) - 280.0;
            Some(
                div()
                    .absolute()
                    .left(px(left.min(max_left).max(8.0)))
                    .top(px(top.min(f32::from(b.size.height) - 48.0)))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .bg(theme::alpha(theme::panel_raised(), 0.97))
                    .border_1()
                    .border_color(theme::alpha(theme::teal(), 0.5))
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_family(theme::CODE_FONT)
                            .text_color(theme::teal())
                            .child(if line > 0 { format!("{path}:{line}") } else { path }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child("押してコードを開く"),
                    ),
            )
        });

        let control = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(24.))
                .min_w(px(24.))
                .px(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::text_muted())
                .hover(|h| h.bg(theme::alpha(theme::white(), 0.08)).text_color(theme::white()))
                .child(label)
        };
        let expanded = self.expanded;
        div()
            .id("diagram")
            .relative()
            .size_full()
            .overflow_hidden()
            .when(pointer, |d| d.cursor_pointer())
            .when(self.drag.is_some() && self.dragged, |d| d.cursor_grabbing())
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.on_scroll(e, cx)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                    this.drag = Some((f32::from(e.position.x), f32::from(e.position.y)));
                    this.dragged = false;
                    if e.click_count == 2 && this.node_at(e.position).is_none() {
                        this.reset(cx);
                    }
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseUpEvent, _, cx| {
                    let was_drag = this.dragged;
                    this.drag = None;
                    if was_drag {
                        return;
                    }
                    if let Some(i) = this.node_at(e.position)
                        && let Some((path, line)) = this.links.get(&i).cloned()
                    {
                        cx.emit(DiagramEvent::Open { path, line });
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if !this.pointer_inside {
                    this.pointer_inside = true;
                    cx.notify();
                }
                if let Some(last) = this.drag
                    && e.pressed_button == Some(gpui::MouseButton::Left)
                {
                    let now = (f32::from(e.position.x), f32::from(e.position.y));
                    let delta = (now.0 - last.0, now.1 - last.1);
                    if delta.0.abs() + delta.1.abs() > 1.0 {
                        this.dragged = true;
                    }
                    this.drag = Some(now);
                    this.pan.0 += delta.0;
                    this.pan.1 += delta.1;
                    cx.notify();
                    return;
                }
                let hover = this.node_at(e.position);
                this.set_hovered(hover, cx);
            }))
            .on_hover(cx.listener(|this, inside: &bool, _, cx| {
                this.pointer_inside = *inside;
                if !*inside {
                    this.set_hovered(None, cx);
                }
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, _, _| viewport.set(bounds),
                    move |bounds, _, window, cx| {
                        // 画面に入ったときに灯し始める
                        if appear_cell.get().is_none() {
                            let vp = window.viewport_size();
                            let visible = f32::from(bounds.origin.y) < f32::from(vp.height) - 40.0
                                && f32::from(bounds.origin.y + bounds.size.height) > 40.0;
                            if visible {
                                appear_cell.set(Some(Instant::now()));
                                // 描画の最中の通知は捨てられるので、次のフレームで頼む
                                window.on_next_frame(move |_, cx| cx.notify(entity));
                            }
                        }
                        let t = transform(&scene, bounds, zoom, pan, expanded);
                        let frame = Frame {
                            scene: &scene,
                            t,
                            links: &links,
                            hovered,
                            appear,
                            now,
                            particles,
                            flow_all,
                        };
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            paint_scene(bounds, &frame, window, cx);
                        });
                    },
                )
                .size_full(),
            )
            .children(tooltip)
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(8.))
                    .bg(theme::alpha(theme::chrome(), 0.85))
                    .border_1()
                    .border_color(theme::hairline())
                    .occlude()
                    .child(
                        control("zoom-out", "−").on_click(
                            cx.listener(|this, _, _, cx| this.zoom_by(1.0 / 1.3, None, cx)),
                        ),
                    )
                    .child(
                        control("zoom-in", "+")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_by(1.3, None, cx))),
                    )
                    .child(
                        control("zoom-fit", "全体")
                            .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
                    )
                    .when(!expanded, |d| {
                        d.child(div().w(px(1.)).h(px(14.)).bg(theme::hairline_strong())).child(
                            control("expand", "拡大")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DiagramEvent::Expand))),
                        )
                    }),
            )
    }
}

/// 粒の位置を決める時計。図ごとにずれないよう、アプリで一つにする。
pub fn clock() -> f32 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}
