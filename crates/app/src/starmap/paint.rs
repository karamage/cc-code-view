//! 星図の描画。画面内の要素だけを描く。
use super::camera::{Camera, Lod, lod};
use super::sky::{Sky, StarKind};
use crate::theme;
use gpui::{
    App, Bounds, BoxShadow, ContentMask, Font, FontFeatures, FontStyle, FontWeight, Hsla,
    PathBuilder, Pixels, TextRun, Window, fill, point, px, quad, size,
};
use std::collections::HashSet;

/// 描画ごとに変わる状態。
pub struct Frame<'a> {
    pub sky: &'a Sky,
    pub positions: &'a [(f32, f32)],
    pub camera: Camera,
    pub hovered: Option<usize>,
    /// カーソルの下の星座。その依存だけを濃く描く
    pub hovered_constellation: Option<usize>,
    /// 実行中に読んでいるファイル、ツアーの通過点など、光らせる星
    pub lit: &'a [usize],
    pub selected: Option<usize>,
    pub route: &'a [usize],
    /// これから通る停留所
    pub upcoming: &'a [usize],
    /// AI が読み終えた星。空が変わって進み具合が見える
    pub read: &'a [usize],
    /// 空全体の明るさ。エラー時や解析中に下げる
    pub dim: f32,
    /// 選ばれた星座。ほかを暗くする
    pub focus_constellation: Option<usize>,
    /// 全景の中の現在地を隅に出す
    pub minimap: bool,
    /// 0.0〜1.0。星座名を灯す演出の進行
    pub reveal: f32,
}

fn circle(window: &mut Window, center: (f32, f32), radius: f32, color: Hsla) {
    let b = Bounds {
        origin: point(px(center.0 - radius), px(center.1 - radius)),
        size: size(px(radius * 2.0), px(radius * 2.0)),
    };
    window.paint_quad(quad(b, px(radius), color, px(0.), gpui::transparent_black(), Default::default()));
}

/// 画面上の星の半径。形を作る星を主役にし、ほかは背景の塵にする。
fn star_screen_radius(radius: f32, figure: bool, zoom: f32) -> f32 {
    let scale = zoom.powf(0.55);
    if figure {
        // 極端に大きなファイルが塊に見えないよう、引きでは上限を設ける
        (radius * scale * 1.35).clamp(1.5, 2.8 + zoom * 2.4)
    } else {
        (radius * scale * 0.72).max(0.65)
    }
}

fn ring(window: &mut Window, center: (f32, f32), radius: f32, color: Hsla) {
    let b = Bounds {
        origin: point(px(center.0 - radius), px(center.1 - radius)),
        size: size(px(radius * 2.0), px(radius * 2.0)),
    };
    window.paint_quad(quad(b, px(radius), gpui::transparent_black(), px(1.2), color, Default::default()));
}

fn glow(window: &mut Window, center: (f32, f32), radius: f32, blur: f32, color: Hsla) {
    let b = Bounds {
        origin: point(px(center.0 - radius), px(center.1 - radius)),
        size: size(px(radius * 2.0), px(radius * 2.0)),
    };
    window.paint_shadows(
        b,
        gpui::Corners::all(px(radius)),
        &[BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(blur), spread_radius: px(0.) }],
    );
}

#[allow(clippy::too_many_arguments)]
fn text(
    window: &mut Window,
    cx: &mut App,
    label: gpui::SharedString,
    at: (f32, f32),
    size_px: f32,
    color: Hsla,
    weight: FontWeight,
    centered: bool,
) {
    if label.is_empty() {
        return;
    }
    let font = Font {
        family: theme::UI_FONT.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    };
    let run = TextRun {
        len: label.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(label, px(size_px), &[run], None);
    let x = if centered { at.0 - f32::from(line.width) / 2.0 } else { at.0 };
    // 星や線の上でも読めるよう、文字の下を暗く敷く
    let backing = Bounds {
        origin: point(px(x - 4.0), px(at.1 - 1.0)),
        size: size(line.width + px(8.0), px(size_px * 1.3 + 2.0)),
    };
    window.paint_quad(quad(
        backing,
        px(3.0),
        theme::alpha(theme::space(), 0.86 * color.a),
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
    let _ = line.paint(point(px(x), px(at.1)), px(size_px * 1.3), window, cx);
}

/// 視差で動く遠景の星。決定的に作る。
fn paint_field(window: &mut Window, origin: (f32, f32), vp: (f32, f32), cam: Camera) {
    const COUNT: u32 = 260;
    let mut seed: u32 = 0x9e37_79b9;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for _ in 0..COUNT {
        let (fx, fy, r, a) = (next(), next(), next(), next());
        // 深さごとに視差を変える
        let depth = 0.012 + r * 0.03;
        let x = (fx * vp.0 - cam.center.0 * cam.zoom.min(1.0) * depth).rem_euclid(vp.0);
        let y = (fy * vp.1 - cam.center.1 * cam.zoom.min(1.0) * depth).rem_euclid(vp.1);
        let color = theme::alpha(theme::white(), 0.08 + a * 0.28);
        circle(window, (origin.0 + x, origin.1 + y), 0.45 + r * 0.55, color);
    }
}

/// ホーム画面の背景。`t` は 0.0〜1.0 の入場の進行で、星座が右上から順に灯る。
pub fn paint_backdrop(
    bounds: Bounds<Pixels>,
    t: f32,
    preview: Option<&ccv_core::model::SkyPreview>,
    window: &mut Window,
) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let vp = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        // 奥の星雲は写真で敷くので、ここでは塗りつぶさない
        let at = |x: f32, y: f32| (origin.0 + vp.0 * x, origin.1 + vp.1 * y);
        paint_field(window, origin, vp, Camera { center: (0.0, 0.0), zoom: 1.0 });

        // 登録済みなら、飾りではなく利用者自身の空を見せる
        if let Some(preview) = preview {
            let area = Bounds {
                origin: point(px(origin.0 + vp.0 * 0.5), px(origin.1 + vp.1 * 0.05)),
                size: size(px(vp.0 * 0.48), px(vp.1 * 0.9)),
            };
            paint_preview(area, preview, 1.0, t, window);
            return;
        }

        // 右側に、航路をなす星座を置く。座標はウィンドウに対する割合
        const STARS: [(f32, f32, f32); 14] = [
            (0.655, 0.16, 2.2),
            (0.735, 0.1, 3.4),
            (0.81, 0.2, 5.4),
            (0.905, 0.13, 2.8),
            (0.955, 0.27, 3.6),
            (0.87, 0.36, 7.2),
            (0.76, 0.42, 3.0),
            (0.695, 0.54, 4.2),
            (0.79, 0.63, 2.4),
            (0.9, 0.56, 4.6),
            (0.965, 0.68, 2.6),
            (0.85, 0.78, 3.8),
            (0.745, 0.86, 5.0),
            (0.64, 0.8, 2.2),
        ];
        const LINES: [(usize, usize); 14] = [
            (0, 1), (1, 2), (2, 3), (3, 4), (2, 5), (4, 5), (5, 6), (6, 7),
            (7, 8), (8, 5), (8, 9), (9, 10), (8, 11), (11, 12),
        ];
        let lit = |i: usize| ((t * 1.5 - i as f32 / 14.0 * 0.5) * 2.0).clamp(0.0, 1.0);
        let mut drawn = PathBuilder::stroke(px(0.8));
        let mut any = false;
        for (i, (a, b)) in LINES.iter().enumerate() {
            let amount = lit(i);
            if amount <= 0.0 {
                continue;
            }
            let (pa, pb) = (STARS[*a], STARS[*b]);
            let (p, q) = (at(pa.0, pa.1), at(pb.0, pb.1));
            let end = (p.0 + (q.0 - p.0) * amount, p.1 + (q.1 - p.1) * amount);
            drawn.move_to(point(px(p.0), px(p.1)));
            drawn.line_to(point(px(end.0), px(end.1)));
            any = true;
        }
        if any && let Ok(path) = drawn.build() {
            window.paint_path(path, theme::alpha(theme::violet(), 0.42));
        }
        for (i, (x, y, r)) in STARS.iter().enumerate() {
            let amount = lit(i);
            if amount <= 0.0 {
                continue;
            }
            let p = at(*x, *y);
            if *r > 3.5 {
                glow(window, p, r * 0.8, r * 3.2, theme::alpha(theme::violet(), 0.6 * amount));
            }
            let brightness = (r / 7.2).clamp(0.45, 1.0);
            let mut color = theme::star_color(brightness);
            color.a *= amount;
            circle(window, p, *r * (0.6 + 0.4 * amount), color);
        }
    });
}

const MINIMAP_W: f32 = 112.0;
const MINIMAP_H: f32 = 74.0;
const MINIMAP_MARGIN: f32 = 10.0;

/// 全景の中の現在地を隅に描く。読んでいる間も全体を見失わないように。
fn paint_minimap(origin: (f32, f32), vp: (f32, f32), f: &Frame<'_>, window: &mut Window) {
    const W: f32 = MINIMAP_W;
    const H: f32 = MINIMAP_H;
    const MARGIN: f32 = MINIMAP_MARGIN;
    let area = Bounds {
        origin: point(px(origin.0 + vp.0 - W - MARGIN), px(origin.1 + MARGIN)),
        size: size(px(W), px(H)),
    };
    window.paint_quad(quad(
        area,
        px(8.),
        theme::alpha(theme::panel_raised(), 0.94),
        px(1.),
        theme::hairline_strong(),
        Default::default(),
    ));
    let (min_x, min_y, max_x, max_y) = f.sky.bounds;
    let span = ((max_x - min_x) / (W - 16.0)).max((max_y - min_y) / (H - 16.0)).max(0.001);
    let (cx_world, cy_world) = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
    let (ax, ay) = (origin.0 + vp.0 - W - MARGIN + W / 2.0, origin.1 + MARGIN + H / 2.0);
    let to_map = |w: (f32, f32)| (ax + (w.0 - cx_world) / span, ay + (w.1 - cy_world) / span);

    for (i, position) in f.positions.iter().enumerate() {
        if !f.sky.figure[i] {
            continue;
        }
        let focused = f
            .focus_constellation
            .is_some_and(|c| f.sky.star_constellation[i] as usize == c);
        let color = if focused {
            theme::teal().into()
        } else {
            theme::alpha(theme::violet(), 0.55)
        };
        circle(window, to_map(*position), if focused { 1.1 } else { 0.7 }, color);
    }
    if f.route.len() > 1 {
        let mut path = PathBuilder::stroke(px(1.0));
        for (i, star) in f.route.iter().enumerate() {
            let p = to_map(f.positions[*star]);
            if i == 0 {
                path.move_to(point(px(p.0), px(p.1)));
            } else {
                path.line_to(point(px(p.0), px(p.1)));
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, theme::alpha(theme::teal(), 0.85));
        }
    }
    // 今見ている範囲
    let top_left = f.camera.screen_to_world((0.0, 0.0), vp);
    let bottom_right = f.camera.screen_to_world(vp, vp);
    let (a, b) = (to_map(top_left), to_map(bottom_right));
    let inner = (area.origin.x, area.origin.y, area.origin.x + area.size.width, area.origin.y + area.size.height);
    let clamp = |x: f32, lo: Pixels, hi: Pixels| x.clamp(f32::from(lo), f32::from(hi));
    let (x0, y0) = (clamp(a.0, inner.0, inner.2), clamp(a.1, inner.1, inner.3));
    let (x1, y1) = (clamp(b.0, inner.0, inner.2), clamp(b.1, inner.1, inner.3));
    if x1 - x0 > 2.0 && y1 - y0 > 2.0 {
        window.paint_quad(quad(
            Bounds { origin: point(px(x0), px(y0)), size: size(px(x1 - x0), px(y1 - y0)) },
            px(2.),
            theme::alpha(theme::white(), 0.05),
            px(1.),
            theme::alpha(theme::white(), 0.6),
            Default::default(),
        ));
    }
}

/// リポジトリの実際の星図を、枠に収めて描く。`t` は入場の進行。
pub fn paint_preview(
    bounds: Bounds<Pixels>,
    preview: &ccv_core::model::SkyPreview,
    star_scale: f32,
    t: f32,
    window: &mut Window,
) {
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let side = w.min(h);
    let at = |x: f32, y: f32| (ox + (w - side) / 2.0 + x * side, oy + (h - side) / 2.0 + y * side);
    let mut path = PathBuilder::stroke(px((star_scale * 0.5).clamp(0.5, 0.9)));
    let mut any = false;
    for (a, b) in &preview.lines {
        let (pa, pb) = (preview.stars[*a as usize], preview.stars[*b as usize]);
        let (p, q) = (at(pa.0, pa.1), at(pb.0, pb.1));
        path.move_to(point(px(p.0), px(p.1)));
        path.line_to(point(px(q.0), px(q.1)));
        any = true;
    }
    if any && t > 0.05 && let Ok(path) = path.build() {
        window.paint_path(path, theme::alpha(theme::violet(), 0.5 * t));
    }
    for (i, (x, y, radius, brightness)) in preview.stars.iter().enumerate() {
        let amount = (t * 1.6 - i as f32 / preview.stars.len().max(1) as f32 * 0.6).clamp(0.0, 1.0);
        if amount <= 0.0 {
            continue;
        }
        let p = at(*x, *y);
        if *radius > 0.0 {
            let r = (radius * star_scale).clamp(0.8, 5.5);
            if star_scale > 0.6 {
                glow(window, p, r * 0.8, r * 3.0, theme::alpha(theme::violet(), 0.4 * amount));
            }
            let mut c = theme::star_color(0.6 + 0.4 * brightness);
            c.a *= amount;
            circle(window, p, r, c);
        } else {
            let mut c = theme::star_color(brightness * 0.6);
            c.a *= 0.45 * amount;
            circle(window, p, (star_scale * 0.7).max(0.45), c);
        }
    }
}

pub fn paint(bounds: Bounds<Pixels>, f: &Frame<'_>, window: &mut Window, cx: &mut App) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let vp = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        window.paint_quad(fill(bounds, theme::space()));
        paint_field(window, origin, vp, f.camera);

        let sky = f.sky;
        let cam = f.camera;
        let level = lod(cam.zoom);
        let to_screen = |world: (f32, f32)| {
            let s = cam.world_to_screen(world, vp);
            (s.0 + origin.0, s.1 + origin.1)
        };
        let on_screen = |p: (f32, f32), margin: f32| {
            p.0 > origin.0 - margin
                && p.0 < origin.0 + vp.0 + margin
                && p.1 > origin.1 - margin
                && p.1 < origin.1 + vp.1 + margin
        };

        // 星雲
        for c in &sky.constellations {
            let center = to_screen(c.center);
            let radius = (c.extent * cam.zoom).max(10.0);
            if on_screen(center, radius * 2.0) {
                let color = theme::alpha(theme::nebula(), 0.55 * f.dim);
                glow(window, center, radius * 0.72, radius * 0.9, color);
            }
        }

        // 星座間の依存。普段は太いものだけを淡く、注目中の星座は全部を描く
        {
            let focus = f
                .hovered_constellation
                .or(f.focus_constellation)
                .or(f.hovered.map(|i| sky.star_constellation[i] as usize));
            let max_links = sky.module_links.iter().map(|l| l.2).max().unwrap_or(1) as f32;
            let resting = sky.constellations.len();
            const BANDS: usize = 3;
            let mut bands: Vec<PathBuilder> =
                (0..BANDS).map(|b| PathBuilder::stroke(px(0.6 + b as f32 * 0.35))).collect();
            let mut used = [false; BANDS];
            let hovering = f.hovered_constellation.is_some() || f.hovered.is_some();
            let mut lit_links = PathBuilder::stroke(px(if hovering { 1.0 } else { 0.7 }));
            let mut any_lit = false;
            for (order, &(a, b, n)) in sky.module_links.iter().enumerate() {
                let touches = focus.is_some_and(|c| a as usize == c || b as usize == c);
                if !touches && (order >= resting || level == Lod::Near) {
                    continue;
                }
                let (ca, cb) = (&sky.constellations[a as usize], &sky.constellations[b as usize]);
                // 線は星座の中心ではなく、代表の星で終わらせる
                let (p, q) = (to_screen(f.positions[ca.anchor]), to_screen(f.positions[cb.anchor]));
                if !on_screen(p, vp.0) && !on_screen(q, vp.0) {
                    continue;
                }
                let mid = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                let ctrl = point(px(mid.0 - dy * 0.14), px(mid.1 + dx * 0.14));
                if touches {
                    lit_links.move_to(point(px(p.0), px(p.1)));
                    lit_links.curve_to(point(px(q.0), px(q.1)), ctrl);
                    any_lit = true;
                } else {
                    // 主要な航路は上位の数本だけにする。全部が強いと何も伝わらない
                    let strong = order < 4 && (n as f32 / max_links) > 0.4;
                    let band = if strong { 2 } else if order < resting / 2 { 1 } else { 0 };
                    bands[band].move_to(point(px(p.0), px(p.1)));
                    bands[band].curve_to(point(px(q.0), px(q.1)), ctrl);
                    used[band] = true;
                }
            }
            let fade = if focus.is_some() { 0.45 } else { 1.0 };
            for (i, path) in bands.into_iter().enumerate() {
                if used[i] && let Ok(path) = path.build() {
                    // 太い依存を主要な航路として際立たせる
                    let alpha = [0.07, 0.15, 0.42][i.min(2)];
                    window.paint_path(path, theme::alpha(theme::violet(), alpha * fade));
                }
            }
            if any_lit && let Ok(path) = lit_links.build() {
                // 選ばれた星座の依存は控えめに。カーソルを乗せたときだけ強くする
                let color = if hovering {
                    theme::alpha(theme::teal(), 0.5)
                } else {
                    theme::alpha(theme::violet(), 0.3)
                };
                window.paint_path(path, color);
            }
        }

        // 星座の形。選ばれた星座があれば、ほかは沈める
        let mut figure = PathBuilder::stroke(px(0.9));
        let mut any_figure = false;
        let mut quiet = PathBuilder::stroke(px(0.7));
        let mut any_quiet = false;
        for &(a, b) in &sky.figure_lines {
            let (p, q) = (to_screen(f.positions[a as usize]), to_screen(f.positions[b as usize]));
            if !on_screen(p, 60.0) && !on_screen(q, 60.0) {
                continue;
            }
            let selected = f
                .focus_constellation
                .is_none_or(|c| sky.star_constellation[a as usize] as usize == c);
            let (builder, flag) =
                if selected { (&mut figure, &mut any_figure) } else { (&mut quiet, &mut any_quiet) };
            builder.move_to(point(px(p.0), px(p.1)));
            builder.line_to(point(px(q.0), px(q.1)));
            *flag = true;
        }
        if any_quiet && let Ok(path) = quiet.build() {
            window.paint_path(path, theme::alpha(theme::violet(), 0.16 * f.dim));
        }
        if any_figure && let Ok(path) = figure.build() {
            window.paint_path(path, theme::alpha(theme::violet(), 0.62 * f.dim));
        }

        // カーソルを乗せた星の依存（ファイル単位）
        if let Some(focus) = f.hovered {
            let mut path = PathBuilder::stroke(px(0.8));
            let mut any = false;
            for &(a, b) in &sky.edges {
                if a as usize == focus || b as usize == focus {
                    let (p, q) = (to_screen(f.positions[a as usize]), to_screen(f.positions[b as usize]));
                    path.move_to(point(px(p.0), px(p.1)));
                    path.line_to(point(px(q.0), px(q.1)));
                    any = true;
                }
            }
            if any && let Ok(path) = path.build() {
                window.paint_path(path, theme::alpha(theme::teal(), 0.5));
            }
        }

        // これから通る航路を淡く描く。全景で読む順番が見えるように
        if f.route.is_empty() && f.upcoming.len() > 1 {
            let mut path = PathBuilder::stroke(px(1.0));
            let mut started = false;
            for pair in f.upcoming.windows(2) {
                let (p, q) = (to_screen(f.positions[pair[0]]), to_screen(f.positions[pair[1]]));
                let mid = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                if !started {
                    path.move_to(point(px(p.0), px(p.1)));
                    started = true;
                }
                path.curve_to(point(px(q.0), px(q.1)), point(px(mid.0 - dy * 0.12), px(mid.1 + dx * 0.12)));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, theme::alpha(theme::teal(), 0.5));
            }
        }

        // ツアーの航路。古い区間は細く薄くし、直近の移動だけを際立たせる
        if f.route.len() > 1 {
            let last = f.route.len() - 2;
            for (i, pair) in f.route.windows(2).enumerate() {
                let (p, q) = (to_screen(f.positions[pair[0]]), to_screen(f.positions[pair[1]]));
                let current = i == last;
                let mut path = PathBuilder::stroke(px(if current { 1.6 } else { 0.9 }));
                let mid = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                path.move_to(point(px(p.0), px(p.1)));
                path.curve_to(point(px(q.0), px(q.1)), point(px(mid.0 - dy * 0.12), px(mid.1 + dx * 0.12)));
                if let Ok(path) = path.build() {
                    let alpha = if current { 0.9 } else { 0.32 };
                    window.paint_path(path, theme::alpha(theme::teal(), alpha));
                }
            }
        }

        // 星
        let lit: HashSet<usize> = f.lit.iter().copied().collect();
        let read: HashSet<usize> = f.read.iter().copied().collect();
        let mut glows = 0;
        // 引きで見える塵は 1 画素に満たない。描く数を間引いて、描画の負荷を一定に保つ
        const MAX_DUST: usize = 2500;
        let dust_stride = if level == Lod::Near { 1 } else { (sky.stars.len() / MAX_DUST).max(1) };
        for (i, s) in sky.stars.iter().enumerate() {
            let figure = sky.figure[i];
            if !figure
                && i % dust_stride != 0
                && !lit.contains(&i)
                && !read.contains(&i)
                && f.selected != Some(i)
                && f.hovered != Some(i)
            {
                continue;
            }
            let p = to_screen(f.positions[i]);
            // 形を作る星を主役にし、ほかは背景の塵にする
            let r = star_screen_radius(s.radius, figure, cam.zoom);
            if !on_screen(p, r + 24.0) {
                continue;
            }
            let special = lit.contains(&i) || f.selected == Some(i) || f.hovered == Some(i);
            if special {
                let accent = theme::alpha(theme::teal(), 0.9);
                glow(window, p, r + 1.0, r * 2.0 + 16.0, accent);
                circle(window, p, r + 0.8, theme::white().into());
                continue;
            }
            let in_focus = f
                .focus_constellation
                .is_none_or(|c| sky.star_constellation[i] as usize == c);
            let was_read = read.contains(&i);
            let mood = if in_focus { f.dim } else { f.dim * 0.42 };
            let tint = |color: Hsla| -> Hsla {
                let mut c = color;
                c.a *= mood;
                c
            };
            let base = match sky.kinds[i] {
                StarKind::Code => theme::star_color(0.55 + 0.45 * s.brightness),
                StarKind::Test => {
                    let mut c: Hsla = theme::star_test().into();
                    c.a = 0.72 + 0.28 * s.brightness;
                    c
                }
                StarKind::Doc => {
                    let mut c: Hsla = theme::star_doc().into();
                    c.a = 0.72 + 0.28 * s.brightness;
                    c
                }
            };
            if was_read {
                // 読み終えた星は、解析の間ずっと灯したままにする
                glow(window, p, r.max(2.4), 9.0, theme::alpha(theme::teal(), 0.6));
                circle(window, p, r.max(2.4), theme::teal().into());
            } else if figure {
                if glows < 140 && in_focus {
                    glows += 1;
                    let halo = theme::alpha(theme::violet(), (0.28 + 0.4 * s.brightness) * mood);
                    glow(window, p, r * 0.8, r * 3.0, halo);
                }
                circle(window, p, r, tint(base));
            } else {
                let mut dust = base;
                dust.a *= 0.46;
                circle(window, p, r, tint(dust));
            }
        }

        // 停留所の印。通過した場所には番号を添える
        for (k, star) in f.upcoming.iter().enumerate() {
            let p = to_screen(f.positions[*star]);
            if on_screen(p, 8.0) {
                let color = theme::alpha(theme::teal(), 0.8);
                ring(window, p, 5.5, color);
                text(
                    window,
                    cx,
                    (f.route.len() + k + 1).to_string().into(),
                    (p.0 - 19.0, p.1 - 8.0),
                    12.0,
                    color,
                    FontWeight::NORMAL,
                    false,
                );
            }
        }
        let current_stop = f.route.last().copied();
        for (n, star) in f.route.iter().enumerate() {
            let p = to_screen(f.positions[*star]);
            if !on_screen(p, 8.0) {
                continue;
            }
            if Some(*star) != current_stop {
                ring(window, p, 6.5, theme::teal().into());
                text(
                    window,
                    cx,
                    (n + 1).to_string().into(),
                    (p.0 - 20.0, p.1 - 8.0),
                    12.0,
                    theme::teal().into(),
                    FontWeight::SEMIBOLD,
                    false,
                );
            }
        }

        // ラベル: 遠景は星座名、中景は主要ファイル、近景は全ファイル
        // 現在地の小窓を障害物として、先に場所を取る
        let mut placed: Vec<(f32, f32, f32, f32)> = Vec::new();
        if f.minimap {
            let (w, h, m) = (MINIMAP_W, MINIMAP_H, MINIMAP_MARGIN);
            placed.push((origin.0 + vp.0 - w - m * 2.0, origin.1, w + m * 2.0, h + m * 2.0));
        }
        fn place(placed: &mut Vec<(f32, f32, f32, f32)>, x: f32, y: f32, w: f32, h: f32) -> bool {
            let hit = placed.iter().any(|(px_, py, pw, ph)| {
                x < px_ + pw && x + w > *px_ && y < py + ph && y + h > *py
            });
            if !hit {
                placed.push((x, y, w, h));
            }
            !hit
        }

        // 光っている星が多いとき（検索結果）は、名前を出すと読めなくなる
        let lit_labels: &[usize] = if f.lit.len() <= 8 { f.lit } else { &[] };
        for &i in lit_labels.iter().chain(f.selected.iter()).chain(f.hovered.iter()) {
            let p = to_screen(f.positions[i]);
            let label = sky.labels[i].clone();
            let w = label.chars().map(|c| if c.is_ascii() { 7.0 } else { 12.0 }).sum::<f32>();
            let r = star_screen_radius(sky.stars[i].radius, sky.figure[i], cam.zoom) + 8.0;
            // 右に入らなければ左へ。枠の外へは出さない
            let x = if p.0 + r + w < origin.0 + vp.0 - 4.0 { p.0 + r } else { p.0 - r - w };
            // 重なるときは、選択中とカーソルの下だけを優先して出す
            let priority = f.selected == Some(i) || f.hovered == Some(i);
            if !place(&mut placed, x - 4.0, p.1 - 10.0, w + 8.0, 20.0) && !priority {
                continue;
            }
            text(window, cx, label, (x, p.1 - 8.0), 12.0, theme::teal().into(), FontWeight::MEDIUM, false);
        }

        // 大きな星座から順に、重ならない分だけ名前を出す。大きさで階層を付ける
        let reveal = f.reveal;
        let total = sky.constellations.len().max(1) as f32;
        let biggest = sky.constellations.iter().map(|c| c.lines).max().unwrap_or(1).max(1) as f32;
        let mut order: Vec<usize> = (0..sky.constellations.len()).collect();
        // 選ばれた星座の名前を最優先で置く
        order.sort_by_key(|i| {
            (f.focus_constellation != Some(*i), std::cmp::Reverse(sky.constellations[*i].lines))
        });
        for (position, ci) in order.into_iter().enumerate() {
            let c = &sky.constellations[ci];
            // 星座名を順に灯す
            let start = position as f32 / total * 0.6;
            let lit_amount = ((reveal - start) / 0.4).clamp(0.0, 1.0);
            if lit_amount <= 0.0 {
                continue;
            }
            let share = (c.lines as f32 / biggest).sqrt();
            // 小さな窓では名前も小さくし、星を隠さない
            let fit = (vp.0.min(vp.1) / 520.0).clamp(0.62, 1.0);
            let size_px = ((12.0 + share * 8.0) * fit).max(10.5);
            let center = to_screen(c.center);
            let screen_radius = c.extent * cam.zoom;
            // 遠景で出す名前の数を画面の広さで制限する。残りは近づくか、カーソルを乗せると出る
            let budget = ((vp.0 * vp.1 / 42_000.0) as usize).max(6);
            let focused = f.hovered_constellation == Some(ci) || f.focus_constellation == Some(ci);
            if level == Lod::Far && position >= budget && !focused {
                continue;
            }
            let mut y = center.1 - screen_radius - size_px * 1.5;
            // 全角を含むので、バイト数ではなく見た目の幅で見積もる
            let w: f32 = c
                .name
                .chars()
                .map(|ch| if ch.is_ascii() { size_px * 0.58 } else { size_px })
                .sum();
            const PAD: f32 = 8.0;
            let pinned = f.focus_constellation == Some(ci);
            let mut center_x = center.0;
            if pinned {
                // 読んでいる星座の名前は、画面の端に寄せてでも見せ続ける
                center_x = center_x
                    .clamp(origin.0 + w / 2.0 + 10.0, (origin.0 + vp.0 - w / 2.0 - 10.0).max(origin.0 + w / 2.0 + 10.0));
                y = y.clamp(origin.1 + 8.0, origin.1 + vp.1 - size_px * 2.0);
            }
            let left = center_x - w / 2.0;
            let inside = left > origin.0 + 6.0 && left + w < origin.0 + vp.0 - 6.0;
            if !pinned
                && (!inside
                    || y < origin.1 + 4.0
                    || !place(&mut placed, left - PAD, y - 3.0, w + PAD * 2.0, size_px * 1.3 + 6.0))
            {
                continue;
            }
            if pinned {
                place(&mut placed, left - PAD, y - 3.0, w + PAD * 2.0, size_px * 1.3 + 6.0);
            }
            let strength = if f.focus_constellation.is_some() && !focused {
                0.35
            } else {
                0.8 + 0.2 * share
            };
            let name_color = if focused {
                theme::alpha(theme::hue(ci), lit_amount)
            } else {
                theme::alpha(theme::white(), strength * lit_amount)
            };
            let weight = if share > 0.5 { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM };
            text(window, cx, c.name.clone(), (center_x, y), size_px, name_color, weight, true);
            // ファイルツリーの点と同じ色で、どの星座かを結びつける
            let dot = (left - size_px * 0.55, y + size_px * 0.62);
            circle(window, dot, (size_px * 0.17).max(2.2), theme::alpha(theme::hue(ci), strength * lit_amount));
        }

        // 名前を置いたあとで、星を障害物に加える。ファイル名が星に重ならないように
        for (i, s) in sky.stars.iter().enumerate() {
            if !sky.figure[i] {
                continue;
            }
            let p = to_screen(f.positions[i]);
            if on_screen(p, 20.0) {
                let r = star_screen_radius(s.radius, true, cam.zoom) + 2.0;
                placed.push((p.0 - r, p.1 - r, r * 2.0, r * 2.0));
            }
        }
        if level != Lod::Far {
            let keep = if level == Lod::Near { u32::MAX } else { 6 };
            // 大きな星から順に名前を付ける
            let mut candidates: Vec<usize> = (0..sky.stars.len())
                .filter(|i| sky.rank[*i] < keep && sky.labeled[*i] && !lit.contains(i))
                .filter(|i| f.selected != Some(*i) && f.hovered != Some(*i))
                .filter(|i| on_screen(to_screen(f.positions[*i]), 0.0))
                .collect();
            candidates.sort_by_key(|i| sky.rank[*i]);
            let mut drawn = 0;
            for i in candidates {
                if drawn >= 70 {
                    break;
                }
                let s = sky.stars[i];
                let p = to_screen(f.positions[i]);
                let r = star_screen_radius(s.radius, sky.figure[i], cam.zoom);
                let label = sky.labels[i].clone();
                let w = label.chars().map(|c| if c.is_ascii() { 6.4 } else { 11.0 }).sum::<f32>();
                // 右、左、上、下の順に空いている場所を探す
                let spots = [
                    (p.0 + r + 7.0, p.1 - 7.5),
                    (p.0 - r - 7.0 - w, p.1 - 7.5),
                    (p.0 - w / 2.0, p.1 - r - 18.0),
                    (p.0 - w / 2.0, p.1 + r + 3.0),
                ];
                let fits = |x: f32| x > origin.0 + 4.0 && x + w < origin.0 + vp.0 - 4.0;
                let Some((x, y)) = spots
                    .into_iter()
                    .find(|(x, y)| fits(*x) && place(&mut placed, *x - 4.0, *y, w + 8.0, 16.0))
                else {
                    continue;
                };
                drawn += 1;
                text(window, cx, label, (x, y), 11.0, theme::alpha(theme::text_muted(), 0.95), FontWeight::NORMAL, false);
            }
        }

        if f.minimap {
            paint_minimap(origin, vp, f, window);
        }
    });
}
