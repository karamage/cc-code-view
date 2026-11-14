//! 配置を終えた図。画面の座標で持ち、描く側はこれをなぞるだけにする。
use super::ast::{Kind, Line, Marker, Shape};
use crate::text::break_lines;

pub const NODE_TEXT: f32 = 13.0;
pub const SMALL_TEXT: f32 = 11.5;
pub const MONO_TEXT: f32 = 11.5;

pub fn line_height(size: f32) -> f32 {
    (size * 1.45).round()
}

/// 文字幅を測る。図の配置と描画で同じ値を使う。
pub trait Measure {
    fn advance(&self, c: char, size: f32, mono: bool, bold: bool) -> f32;

    fn width(&self, text: &str, size: f32, mono: bool, bold: bool) -> f32 {
        text.chars().map(|c| self.advance(c, size, mono, bold)).sum()
    }

    /// 禁則処理をして折り返す。明示した改行は保つ。
    fn wrap(&self, text: &str, size: f32, max: f32, bold: bool) -> Vec<String> {
        let advance = |c: char| self.advance(c, size, false, bold);
        let lines = break_lines(text, max, &advance);
        let lines: Vec<String> = lines.into_iter().filter(|l| !l.is_empty()).collect();
        if lines.is_empty() { vec![String::new()] } else { lines }
    }
}

/// テストと見積もりに使う固定幅の測り方。
pub struct FixedMeasure;

impl Measure for FixedMeasure {
    fn advance(&self, c: char, size: f32, mono: bool, _: bool) -> f32 {
        if c.is_ascii() { size * if mono { 0.6 } else { 0.55 } } else { size }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn centered(cx: f32, cy: f32, w: f32, h: f32) -> Rect {
        Rect { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: self.right().max(o.right()) - x, h: self.bottom().max(o.bottom()) - y }
    }
    fn shift(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub id: String,
    pub rect: Rect,
    pub shape: Shape,
    pub title: Vec<String>,
    pub stereotype: Option<String>,
    /// 区画 → 行 → 列。ER 図の列はタブ区切りから作る
    pub sections: Vec<Vec<Vec<String>>>,
    /// 列の左端。箱の左端からの距離
    pub columns: Vec<f32>,
    /// 色を決める最上位の囲み
    pub hue: Option<usize>,
    /// 現れる順番。上流から順に灯す
    pub rank: usize,
    /// シーケンス図の下端に繰り返す参加者。元の要素の番号
    pub mirror_of: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneLabel {
    pub rect: Rect,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEdge {
    pub from: usize,
    pub to: usize,
    /// 3 次ベジェの列。始点に続けて、制御点 2 つと終点を繰り返す
    pub path: Vec<(f32, f32)>,
    pub line: Line,
    pub start: Marker,
    pub end: Marker,
    pub label: Option<SceneLabel>,
    pub start_label: Option<SceneLabel>,
    pub end_label: Option<SceneLabel>,
    /// シーケンス図の通し番号
    pub number: Option<usize>,
    pub rank: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneGroup {
    pub rect: Rect,
    pub label: String,
    /// シーケンス図の枠の種類（loop、alt など）
    pub keyword: Option<String>,
    pub depth: usize,
    pub hue: usize,
    /// 枠を区切る線の高さと、その区画の条件
    pub dividers: Vec<(f32, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneNote {
    pub rect: Rect,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lifeline {
    pub node: usize,
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub kind: Kind,
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<SceneNode>,
    pub edges: Vec<SceneEdge>,
    pub groups: Vec<SceneGroup>,
    pub notes: Vec<SceneNote>,
    pub lifelines: Vec<Lifeline>,
    /// 階層の数。灯す演出の長さを決める
    pub ranks: usize,
}

impl Scene {
    /// 余白を足して左上を原点に寄せ、大きさを決める。
    pub(crate) fn normalize(&mut self, margin: f32) {
        let mut bounds: Option<Rect> = None;
        let mut add = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
        for n in &self.nodes {
            add(n.rect);
        }
        for g in &self.groups {
            add(g.rect);
        }
        for n in &self.notes {
            add(n.rect);
        }
        for e in &self.edges {
            for p in &e.path {
                add(Rect { x: p.0, y: p.1, w: 0.0, h: 0.0 });
            }
            for l in [&e.label, &e.start_label, &e.end_label].into_iter().flatten() {
                add(l.rect);
            }
        }
        let Some(b) = bounds else {
            self.width = margin * 2.0;
            self.height = margin * 2.0;
            return;
        };
        let (dx, dy) = (margin - b.x, margin - b.y);
        for n in &mut self.nodes {
            n.rect.shift(dx, dy);
        }
        for g in &mut self.groups {
            g.rect.shift(dx, dy);
            for d in &mut g.dividers {
                d.0 += dy;
            }
        }
        for n in &mut self.notes {
            n.rect.shift(dx, dy);
        }
        for e in &mut self.edges {
            for p in &mut e.path {
                p.0 += dx;
                p.1 += dy;
            }
            for l in [&mut e.label, &mut e.start_label, &mut e.end_label].into_iter().flatten() {
                l.rect.shift(dx, dy);
            }
        }
        for l in &mut self.lifelines {
            l.x += dx;
            l.top += dy;
            l.bottom += dy;
        }
        self.width = b.w + margin * 2.0;
        self.height = b.h + margin * 2.0;
    }

    /// 点の下の要素。小さな要素を優先する。
    pub fn node_at(&self, x: f32, y: f32) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.rect.contains(x, y))
            .min_by(|a, b| (a.1.rect.w * a.1.rect.h).total_cmp(&(b.1.rect.w * b.1.rect.h)))
            .map(|(i, n)| n.mirror_of.unwrap_or(i))
    }
}

/// 3 次ベジェ上の点。
pub fn bezier_point(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p1: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (a * p0.0 + b * c1.0 + c * c2.0 + d * p1.0, a * p0.1 + b * c1.1 + c * c2.1 + d * p1.1)
}

/// 線の長さに沿った位置 `t`（0.0〜1.0）の点と向き。粒を流すのに使う。
pub fn point_along(path: &[(f32, f32)], t: f32) -> Option<((f32, f32), (f32, f32))> {
    let segments = (path.len().saturating_sub(1)) / 3;
    if segments == 0 {
        return None;
    }
    // 各区間の長さを折れ線で見積もる
    let mut lengths = Vec::with_capacity(segments);
    for s in 0..segments {
        let (p0, c1, c2, p1) = (path[s * 3], path[s * 3 + 1], path[s * 3 + 2], path[s * 3 + 3]);
        let mut len = 0.0;
        let mut prev = p0;
        for k in 1..=8 {
            let q = bezier_point(p0, c1, c2, p1, k as f32 / 8.0);
            len += ((q.0 - prev.0).powi(2) + (q.1 - prev.1).powi(2)).sqrt();
            prev = q;
        }
        lengths.push(len);
    }
    let total: f32 = lengths.iter().sum();
    let mut want = t.clamp(0.0, 1.0) * total;
    for (s, len) in lengths.iter().enumerate() {
        if want <= *len || s == segments - 1 {
            let local = if *len > 0.0 { (want / len).clamp(0.0, 1.0) } else { 0.0 };
            let (p0, c1, c2, p1) = (path[s * 3], path[s * 3 + 1], path[s * 3 + 2], path[s * 3 + 3]);
            let at = bezier_point(p0, c1, c2, p1, local);
            let ahead = bezier_point(p0, c1, c2, p1, (local + 0.02).min(1.0));
            let behind = bezier_point(p0, c1, c2, p1, (local - 0.02).max(0.0));
            return Some((at, (ahead.0 - behind.0, ahead.1 - behind.1)));
        }
        want -= len;
    }
    None
}

/// 線の先頭から `t` までを切り出す。線を引いていく演出に使う。
pub fn trim_path(path: &[(f32, f32)], t: f32) -> Vec<(f32, f32)> {
    let segments = (path.len().saturating_sub(1)) / 3;
    if segments == 0 || t >= 1.0 {
        return path.to_vec();
    }
    let t = t.max(0.0) * segments as f32;
    let whole = t.floor() as usize;
    let mut out = path[..=whole * 3].to_vec();
    let frac = t - whole as f32;
    if whole < segments && frac > 0.0 {
        let (p0, c1, c2, p1) =
            (path[whole * 3], path[whole * 3 + 1], path[whole * 3 + 2], path[whole * 3 + 3]);
        // de Casteljau で前半を取り出す
        let lerp =
            |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * frac, a.1 + (b.1 - a.1) * frac);
        let (a, b, c) = (lerp(p0, c1), lerp(c1, c2), lerp(c2, p1));
        let (d, e) = (lerp(a, b), lerp(b, c));
        let f = lerp(d, e);
        out.extend([a, d, f]);
    }
    out
}

/// 直線を 3 次ベジェの形にする。
pub(crate) fn straight(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let third = |k: f32| (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k);
    vec![a, third(1.0 / 3.0), third(2.0 / 3.0), b]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_keeps_the_start_and_ends_on_the_curve() {
        let path = straight((0.0, 0.0), (90.0, 0.0));
        let half = trim_path(&path, 0.5);
        assert_eq!(half[0], (0.0, 0.0));
        let end = *half.last().unwrap();
        assert!((end.0 - 45.0).abs() < 0.01, "{end:?}");
        assert_eq!(trim_path(&path, 1.0), path);
    }

    #[test]
    fn points_along_follow_length() {
        let mut path = straight((0.0, 0.0), (10.0, 0.0));
        path.extend(straight((10.0, 0.0), (10.0, 30.0)).into_iter().skip(1));
        let (p, dir) = point_along(&path, 0.5).unwrap();
        assert!((p.0 - 10.0).abs() < 0.5 && (p.1 - 10.0).abs() < 0.5, "{p:?}");
        assert!(dir.1 > 0.0);
    }

    #[test]
    fn wraps_with_kinsoku() {
        let lines = FixedMeasure.wrap("設定を読み込み、画面を描く", 13.0, 13.0 * 6.0, false);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| !l.starts_with('、')));
    }
}
