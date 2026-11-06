//! 星図のカメラ。描画に依存しない純粋な計算。
use ccv_core::layout::Star;

pub const MIN_ZOOM: f32 = 0.01;
pub const MAX_ZOOM: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: (f32, f32),
    pub zoom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lod {
    Far,
    Mid,
    Near,
}

pub fn lod(zoom: f32) -> Lod {
    if zoom < 1.0 {
        Lod::Far
    } else if zoom < 2.6 {
        Lod::Mid
    } else {
        Lod::Near
    }
}

impl Camera {
    /// `vp` はビューポートの大きさ。スクリーン座標はビューポートの左上が原点。
    pub fn world_to_screen(&self, world: (f32, f32), vp: (f32, f32)) -> (f32, f32) {
        (
            (world.0 - self.center.0) * self.zoom + vp.0 / 2.0,
            (world.1 - self.center.1) * self.zoom + vp.1 / 2.0,
        )
    }

    pub fn screen_to_world(&self, screen: (f32, f32), vp: (f32, f32)) -> (f32, f32) {
        (
            (screen.0 - vp.0 / 2.0) / self.zoom + self.center.0,
            (screen.1 - vp.1 / 2.0) / self.zoom + self.center.1,
        )
    }

    /// カーソルの下の点を動かさずにズームする。
    pub fn zoom_at(&mut self, cursor: (f32, f32), factor: f32, vp: (f32, f32)) {
        let before = self.screen_to_world(cursor, vp);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.screen_to_world(cursor, vp);
        self.center.0 += before.0 - after.0;
        self.center.1 += before.1 - after.1;
    }

    pub fn pan(&mut self, screen_delta: (f32, f32)) {
        self.center.0 -= screen_delta.0 / self.zoom;
        self.center.1 -= screen_delta.1 / self.zoom;
    }

    /// `bounds` (min_x, min_y, max_x, max_y) がビューポートに収まるカメラ。
    pub fn fit(bounds: (f32, f32, f32, f32), vp: (f32, f32), max_zoom: f32) -> Camera {
        let (w, h) = ((bounds.2 - bounds.0).max(1.0), (bounds.3 - bounds.1).max(1.0));
        let zoom = (vp.0 / w).min(vp.1 / h) * 0.96;
        Camera {
            center: ((bounds.0 + bounds.2) / 2.0, (bounds.1 + bounds.3) / 2.0),
            zoom: zoom.clamp(MIN_ZOOM, max_zoom),
        }
    }

    /// ズームは対数で補間する。線形だと、引きの動きが急に見える。
    pub fn lerp(a: Camera, b: Camera, t: f32) -> Camera {
        let t = ease_in_out_cubic(t);
        Camera {
            center: (
                a.center.0 + (b.center.0 - a.center.0) * t,
                a.center.1 + (b.center.1 - a.center.1) * t,
            ),
            zoom: (a.zoom.ln() + (b.zoom.ln() - a.zoom.ln()) * t).exp(),
        }
    }
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// 最も近い星を返す。小さく見える星にも指しやすいよう、画面上で 8px の余裕を持たせる。
pub fn hit_star(
    stars: &[Star],
    positions: &[(f32, f32)],
    world: (f32, f32),
    zoom: f32,
) -> Option<usize> {
    let tolerance = 8.0 / zoom;
    let mut best: Option<(usize, f32)> = None;
    for (i, (s, p)) in stars.iter().zip(positions).enumerate() {
        let d = ((p.0 - world.0).powi(2) + (p.1 - world.1).powi(2)).sqrt();
        if d <= s.radius.max(tolerance) && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_point_under_cursor_fixed() {
        let mut c = Camera { center: (0.0, 0.0), zoom: 1.0 };
        let vp = (800.0, 600.0);
        let cursor = (100.0, 50.0);
        let before = c.screen_to_world(cursor, vp);
        c.zoom_at(cursor, 2.0, vp);
        let after = c.screen_to_world(cursor, vp);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
    }

    #[test]
    fn zoom_is_clamped_and_lod_changes() {
        let mut c = Camera { center: (0.0, 0.0), zoom: 1.0 };
        c.zoom_at((0.0, 0.0), 1e9, (800.0, 600.0));
        assert!(c.zoom <= MAX_ZOOM);
        assert_eq!(lod(MIN_ZOOM), Lod::Far);
        assert_eq!(lod(MAX_ZOOM), Lod::Near);
    }

    #[test]
    fn screen_and_world_round_trip_and_fit_contains_bounds() {
        let vp = (800.0, 600.0);
        let c = Camera::fit((-100.0, -50.0, 300.0, 250.0), vp, MAX_ZOOM);
        let (x, y) = c.world_to_screen((-100.0, -50.0), vp);
        assert!(x >= 0.0 && y >= 0.0);
        let (x, y) = c.world_to_screen((300.0, 250.0), vp);
        assert!(x <= vp.0 && y <= vp.1);
        let back = c.screen_to_world(c.world_to_screen((12.0, 34.0), vp), vp);
        assert!((back.0 - 12.0).abs() < 1e-3 && (back.1 - 34.0).abs() < 1e-3);
    }

    #[test]
    fn lerp_hits_endpoints() {
        let (a, b) = (Camera { center: (0.0, 0.0), zoom: 0.5 }, Camera { center: (10.0, 20.0), zoom: 4.0 });
        assert_eq!(Camera::lerp(a, b, 0.0).center, a.center);
        assert_eq!(Camera::lerp(a, b, 1.0).center, b.center);
        assert!((Camera::lerp(a, b, 1.0).zoom - 4.0).abs() < 1e-4);
    }

    #[test]
    fn hit_test_prefers_nearest_star_with_screen_tolerance() {
        let s = |x, y| Star { x, y, radius: 2.0, brightness: 1.0, module: 0 };
        let stars = [s(0.0, 0.0), s(10.0, 0.0)];
        let pos = [(0.0, 0.0), (10.0, 0.0)];
        assert_eq!(hit_star(&stars, &pos, (9.0, 0.0), 1.0), Some(1));
        assert_eq!(hit_star(&stars, &pos, (100.0, 0.0), 1.0), None);
    }
}
