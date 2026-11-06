//! 星図の描画。画面内の要素だけを描く。
use super::camera::{Camera, Lod, lod};
use super::sky::Sky;
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

fn constellation_of(sky: &Sky, star: usize) -> Option<usize> {
    let module = sky.stars.get(star)?.module;
    sky.constellations.iter().position(|c| match c.module_index {
        Some(m) => m == module,
        None => module >= sky.constellations.iter().filter(|x| x.module_index.is_some()).count(),
    })
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
pub fn paint_backdrop(bounds: Bounds<Pixels>, t: f32, window: &mut Window) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let vp = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        window.paint_quad(fill(bounds, theme::space()));
        let at = |x: f32, y: f32| (origin.0 + vp.0 * x, origin.1 + vp.1 * y);
        glow(window, at(0.86, 0.3), 220.0, 300.0, theme::alpha(theme::nebula(), 0.95));
        glow(window, at(0.72, 0.84), 150.0, 240.0, theme::alpha(theme::nebula(), 0.7));
        paint_field(window, origin, vp, Camera { center: (0.0, 0.0), zoom: 1.0 });

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
                let color = theme::alpha(theme::nebula(), 0.55);
                glow(window, center, radius * 0.72, radius * 0.9, color);
            }
        }

        // 星座間の依存。普段は太いものだけを淡く、注目中の星座は全部を描く
        {
            let focus = f.hovered_constellation.or(f.hovered.and_then(|i| constellation_of(sky, i)));
            let max_links = sky.module_links.iter().map(|l| l.2).max().unwrap_or(1) as f32;
            let resting = sky.constellations.len();
            const BANDS: usize = 3;
            let mut bands: Vec<PathBuilder> =
                (0..BANDS).map(|b| PathBuilder::stroke(px(0.6 + b as f32 * 0.35))).collect();
            let mut used = [false; BANDS];
            let mut lit_links = PathBuilder::stroke(px(1.1));
            let mut any_lit = false;
            for (order, &(a, b, n)) in sky.module_links.iter().enumerate() {
                let touches = focus.is_some_and(|c| a as usize == c || b as usize == c);
                if !touches && (order >= resting || level == Lod::Near) {
                    continue;
                }
                let (ca, cb) = (&sky.constellations[a as usize], &sky.constellations[b as usize]);
                let (p, q) = (to_screen(ca.center), to_screen(cb.center));
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
                    let weight = (n as f32 / max_links).sqrt();
                    let band = ((weight * BANDS as f32) as usize).min(BANDS - 1);
                    bands[band].move_to(point(px(p.0), px(p.1)));
                    bands[band].curve_to(point(px(q.0), px(q.1)), ctrl);
                    used[band] = true;
                }
            }
            let fade = if focus.is_some() { 0.45 } else { 1.0 };
            for (i, path) in bands.into_iter().enumerate() {
                if used[i] && let Ok(path) = path.build() {
                    let alpha = 0.05 + (i as f32 + 0.5) / BANDS as f32 * 0.14;
                    window.paint_path(path, theme::alpha(theme::violet(), alpha * fade));
                }
            }
            if any_lit && let Ok(path) = lit_links.build() {
                window.paint_path(path, theme::alpha(theme::teal(), 0.55));
            }
        }

        // 星座の形
        let mut figure = PathBuilder::stroke(px(0.9));
        let mut any_figure = false;
        for &(a, b) in &sky.figure_lines {
            let (p, q) = (to_screen(f.positions[a as usize]), to_screen(f.positions[b as usize]));
            if on_screen(p, 60.0) || on_screen(q, 60.0) {
                figure.move_to(point(px(p.0), px(p.1)));
                figure.line_to(point(px(q.0), px(q.1)));
                any_figure = true;
            }
        }
        if any_figure && let Ok(path) = figure.build() {
            window.paint_path(path, theme::alpha(theme::violet(), 0.62));
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

        // ツアーの航路
        if f.route.len() > 1 {
            let mut path = PathBuilder::stroke(px(1.6));
            for (i, star) in f.route.iter().enumerate() {
                let p = to_screen(f.positions[*star]);
                if i == 0 {
                    path.move_to(point(px(p.0), px(p.1)));
                } else {
                    path.line_to(point(px(p.0), px(p.1)));
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, theme::alpha(theme::teal(), 0.9));
            }
            for star in f.route {
                let p = to_screen(f.positions[*star]);
                if on_screen(p, 12.0) {
                    circle(window, p, 4.0, theme::teal().into());
                    circle(window, p, 2.2, theme::space().into());
                }
            }
        }

        // 星
        let lit: HashSet<usize> = f.lit.iter().copied().collect();
        // 引いても星が消えないよう、大きさはズームほどには縮めない
        let scale = cam.zoom.powf(0.55);
        let mut glows = 0;
        for (i, s) in sky.stars.iter().enumerate() {
            let p = to_screen(f.positions[i]);
            let figure = sky.figure[i];
            // 形を作る星を主役にし、ほかは背景の塵にする
            let r = if figure {
                // 極端に大きなファイルが塊に見えないよう、引きでは上限を設ける
                (s.radius * scale * 1.35).clamp(1.5, 2.8 + cam.zoom * 2.4)
            } else {
                (s.radius * scale * 0.72).max(0.65)
            };
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
            if figure {
                if glows < 240 {
                    glows += 1;
                    let halo = theme::alpha(theme::violet(), 0.28 + 0.4 * s.brightness);
                    glow(window, p, r * 0.8, r * 3.0, halo);
                }
                circle(window, p, r, theme::star_color(0.55 + 0.45 * s.brightness));
            } else {
                let mut dust = theme::star_color(s.brightness * 0.6);
                dust.a *= 0.62;
                circle(window, p, r, dust);
            }
        }

        // ラベル: 遠景は星座名、中景は主要ファイル、近景は全ファイル
        let mut placed: Vec<(f32, f32, f32, f32)> = Vec::new();
        let mut try_place = |x: f32, y: f32, w: f32, h: f32| {
            let hit = placed.iter().any(|(px_, py, pw, ph)| {
                x < px_ + pw && x + w > *px_ && y < py + ph && y + h > *py
            });
            if !hit {
                placed.push((x, y, w, h));
            }
            !hit
        };

        // 光っている星が多いとき（検索結果）は、名前を出すと読めなくなる
        let lit_labels: &[usize] = if f.lit.len() <= 8 { f.lit } else { &[] };
        for &i in lit_labels.iter().chain(f.selected.iter()).chain(f.hovered.iter()) {
            let p = to_screen(f.positions[i]);
            let label = sky.labels[i].clone();
            let w = label.chars().map(|c| if c.is_ascii() { 7.0 } else { 12.0 }).sum::<f32>();
            try_place(p.0 + 8.0, p.1 - 10.0, w + 12.0, 20.0);
            text(window, cx, label, (p.0 + 12.0, p.1 - 8.0), 12.0, theme::teal().into(), FontWeight::MEDIUM, false);
        }

        // 大きな星座から順に、重ならない分だけ名前を出す。大きさで階層を付ける
        let reveal = f.reveal;
        let total = sky.constellations.len().max(1) as f32;
        let biggest = sky.constellations.iter().map(|c| c.lines).max().unwrap_or(1).max(1) as f32;
        let mut order: Vec<usize> = (0..sky.constellations.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(sky.constellations[*i].lines));
        for (position, ci) in order.into_iter().enumerate() {
            let c = &sky.constellations[ci];
            // 星座名を順に灯す
            let start = position as f32 / total * 0.6;
            let lit_amount = ((reveal - start) / 0.4).clamp(0.0, 1.0);
            if lit_amount <= 0.0 {
                continue;
            }
            let share = (c.lines as f32 / biggest).sqrt();
            let size_px = 12.0 + share * 8.0;
            let center = to_screen(c.center);
            let y = center.1 - c.extent * cam.zoom - size_px * 2.0;
            // 全角を含むので、バイト数ではなく見た目の幅で見積もる
            let w: f32 = c
                .name
                .chars()
                .map(|ch| if ch.is_ascii() { size_px * 0.58 } else { size_px })
                .sum();
            const PAD: f32 = 8.0;
            let left = center.0 - w / 2.0;
            let inside = left > origin.0 + 6.0 && left + w < origin.0 + vp.0 - 6.0;
            if !inside
                || y < origin.1 + 4.0
                || !try_place(left - PAD, y - 3.0, w + PAD * 2.0, size_px * 1.3 + 6.0)
            {
                continue;
            }
            let focused = f.hovered_constellation == Some(ci);
            let strength = if focused { 1.0 } else { 0.62 + 0.38 * share };
            let name_color = if focused {
                theme::alpha(theme::teal(), lit_amount)
            } else {
                theme::alpha(theme::white(), strength * lit_amount)
            };
            let weight = if share > 0.5 { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM };
            text(window, cx, c.name.clone(), (center.0, y), size_px, name_color, weight, true);
        }

        if level != Lod::Far {
            let keep = if level == Lod::Near { u32::MAX } else { 3 };
            let mut drawn = 0;
            for (i, s) in sky.stars.iter().enumerate() {
                let special = lit.contains(&i) || f.selected == Some(i) || f.hovered == Some(i);
                if sky.rank[i] >= keep || !sky.labeled[i] || drawn > 160 || special {
                    continue;
                }
                let p = to_screen(f.positions[i]);
                let r = (s.radius * scale).max(0.7);
                if !on_screen(p, 0.0) {
                    continue;
                }
                let label = sky.labels[i].clone();
                let w = label.chars().map(|c| if c.is_ascii() { 6.4 } else { 11.0 }).sum::<f32>();
                // 右に置けなければ左へ。枠の外へはみ出すものは出さない
                let right = p.0 + r + 7.0;
                let left = p.0 - r - 7.0 - w;
                let y = p.1 - 7.5;
                let fits = |x: f32| x > origin.0 + 4.0 && x + w < origin.0 + vp.0 - 4.0;
                let x = if fits(right) && try_place(right - 4.0, y, w + 8.0, 16.0) {
                    right
                } else if fits(left) && try_place(left - 4.0, y, w + 8.0, 16.0) {
                    left
                } else {
                    continue;
                };
                drawn += 1;
                text(window, cx, label, (x, y), 11.0, theme::alpha(theme::text_muted(), 0.95), FontWeight::NORMAL, false);
            }
        }
    });
}
