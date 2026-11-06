//! 星図のビュー。カメラの操作と飛行を管理する。
pub mod camera;
pub mod paint;
pub mod sky;

use camera::{Camera, MAX_ZOOM, ease_in_out_cubic, hit_star};
use gpui::{
    Bounds, Context, EventEmitter, Pixels, Render, ScrollDelta, ScrollWheelEvent, Window, canvas,
    div, prelude::*,
};
use paint::Frame;
use sky::Sky;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::theme;

pub enum StarMapEvent {
    OpenFile(usize),
    OpenConstellation(usize),
}

struct Flight {
    from: Camera,
    to: Camera,
    start: Instant,
    duration: Duration,
}

struct Morph {
    from: Vec<(f32, f32)>,
    start: Instant,
    duration: Duration,
}

pub struct StarMap {
    sky: Arc<Sky>,
    positions: Rc<Vec<(f32, f32)>>,
    camera: Camera,
    flight: Option<Flight>,
    morph: Option<Morph>,
    reveal_start: Option<Instant>,
    reveal_duration: Duration,
    hovered: Option<usize>,
    selected: Option<usize>,
    lit: Vec<usize>,
    route: Vec<usize>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<(f32, f32)>,
    dragged: bool,
    needs_fit: bool,
    reduce_motion: bool,
    /// 上右下左。パネルに隠れない範囲へ全景を収める
    insets: (f32, f32, f32, f32),
    /// 全景を見ている。余白が変わったら合わせ直す
    at_overview: bool,
    /// `CCV_FRAME_LOG` を付けたときだけ、フレーム間隔を出す
    frame_log: Option<Instant>,
    orbit: bool,
    last_vp: (f32, f32),
}

impl EventEmitter<StarMapEvent> for StarMap {}

fn positions_of(sky: &Sky) -> Vec<(f32, f32)> {
    sky.stars.iter().map(|s| (s.x, s.y)).collect()
}

impl StarMap {
    pub fn new(sky: Arc<Sky>, reduce_motion: bool) -> StarMap {
        StarMap {
            positions: Rc::new(positions_of(&sky)),
            sky,
            camera: Camera { center: (0.0, 0.0), zoom: 1.0 },
            flight: None,
            morph: None,
            reveal_start: Some(Instant::now()),
            reveal_duration: Duration::from_millis(1400),
            hovered: None,
            selected: None,
            lit: Vec::new(),
            route: Vec::new(),
            viewport: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            dragged: false,
            needs_fit: true,
            reduce_motion,
            insets: (24.0, 24.0, 96.0, 24.0),
            at_overview: true,
            frame_log: std::env::var_os("CCV_FRAME_LOG").map(|_| Instant::now()),
            orbit: std::env::var_os("CCV_FRAME_ORBIT").is_some(),
            last_vp: (0.0, 0.0),
        }
    }

    pub fn sky(&self) -> &Arc<Sky> {
        &self.sky
    }

    /// 一度でも配置されたか。カメラの計算には画面の大きさが要る。
    pub fn is_laid_out(&self) -> bool {
        !self.needs_fit
    }

    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// 解析の完了時などに星座を組み替える。星は新しい位置へ移動する。
    pub fn set_sky(&mut self, sky: Arc<Sky>, cx: &mut Context<Self>) {
        let from = (*self.positions).clone();
        let same_len = from.len() == sky.stars.len();
        self.positions = Rc::new(positions_of(&sky));
        self.sky = sky;
        let (morph_ms, reveal_ms) =
            if self.reduce_motion { (theme::REDUCED_MS, theme::REDUCED_MS) } else { (1100, 1700) };
        if same_len {
            self.morph = Some(Morph {
                from,
                start: Instant::now(),
                duration: Duration::from_millis(morph_ms),
            });
        }
        self.reveal_start = Some(Instant::now());
        self.reveal_duration = Duration::from_millis(reveal_ms);
        self.fit_all(cx);
    }

    fn vp(&self) -> (f32, f32) {
        let s = self.viewport.get().size;
        (f32::from(s.width).max(1.0), f32::from(s.height).max(1.0))
    }

    fn fit_camera(&self) -> Camera {
        let (w, h) = self.vp();
        let (top, right, bottom, left) = self.insets;
        let usable = ((w - left - right).max(120.0), (h - top - bottom).max(120.0));
        let mut cam = Camera::fit(self.label_bounds(), usable, 1.6);
        // 使える範囲の中心へ全景の中心を合わせる
        cam.center.0 -= (left - right) / 2.0 / cam.zoom;
        cam.center.1 -= (top - bottom) / 2.0 / cam.zoom;
        cam
    }

    /// 星座名は星の外に出るので、その分を含めた範囲を返す。
    fn label_bounds(&self) -> (f32, f32, f32, f32) {
        let (min_x, min_y, max_x, max_y) = self.sky.bounds;
        let margin = (max_x - min_x).max(max_y - min_y) * 0.05;
        (min_x - margin, min_y - margin * 1.6, max_x + margin, max_y + margin)
    }

    pub fn set_insets(&mut self, insets: (f32, f32, f32, f32), cx: &mut Context<Self>) {
        if self.insets != insets {
            self.insets = insets;
            if !self.needs_fit && self.at_overview {
                self.fit_all(cx);
            }
        }
    }

    fn fly(&mut self, to: Camera, cx: &mut Context<Self>) {
        let ms = if self.reduce_motion { theme::REDUCED_MS } else { theme::FLIGHT_MS };
        self.flight = Some(Flight {
            from: self.camera,
            to,
            start: Instant::now(),
            duration: Duration::from_millis(ms),
        });
        cx.notify();
    }

    pub fn fit_all(&mut self, cx: &mut Context<Self>) {
        self.at_overview = true;
        let to = self.fit_camera();
        self.fly(to, cx);
    }

    pub fn fly_to_file(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(s) = self.sky.stars.get(file) else { return };
        self.at_overview = false;
        let to = Camera { center: (s.x, s.y), zoom: self.camera.zoom.clamp(1.1, 2.0) };
        self.fly(to, cx);
    }

    pub fn fly_to_constellation(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(c) = self.sky.constellations.get(index) else { return };
        let (w, h) = self.vp();
        let zoom = ((w.min(h) * 0.7) / (c.extent * 2.0 + 120.0)).clamp(0.6, 3.5);
        self.at_overview = false;
        // 左のパネルに隠れないよう、右へ寄せて映す
        let center = (c.center.0 - 180.0 / zoom, c.center.1);
        self.fly(Camera { center, zoom }, cx);
    }

    pub fn zoom_step(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.at_overview = false;
        let mut to = self.flight.as_ref().map_or(self.camera, |f| f.to);
        to.zoom = (to.zoom * factor).clamp(camera::MIN_ZOOM, MAX_ZOOM);
        let ms = if self.reduce_motion { 0 } else { 180 };
        self.flight = Some(Flight {
            from: self.camera,
            to,
            start: Instant::now(),
            duration: Duration::from_millis(ms),
        });
        cx.notify();
    }

    pub fn set_selected(&mut self, file: Option<usize>, cx: &mut Context<Self>) {
        self.selected = file;
        cx.notify();
    }

    pub fn set_lit(&mut self, files: Vec<usize>, cx: &mut Context<Self>) {
        self.lit = files;
        cx.notify();
    }

    pub fn set_route(&mut self, route: Vec<usize>, cx: &mut Context<Self>) {
        self.route = route;
        cx.notify();
    }

    fn local(&self, position: gpui::Point<Pixels>) -> (f32, f32) {
        let o = self.viewport.get().origin;
        (f32::from(position.x - o.x), f32::from(position.y - o.y))
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let (dx, dy) = match e.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
        };
        self.flight = None;
        self.at_overview = false;
        let vp = self.vp();
        if e.modifiers.platform || e.modifiers.control {
            let cursor = self.local(e.position);
            self.camera.zoom_at(cursor, (dy * 0.005).exp(), vp);
        } else {
            self.camera.pan((dx, dy));
        }
        cx.notify();
    }

    fn star_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let world = self.camera.screen_to_world(self.local(position), self.vp());
        hit_star(&self.sky.stars, &self.positions, world, self.camera.zoom)
    }

    fn constellation_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        // 名前のラベルの周辺、または星座の中で星のない場所
        let (x, y) = self.camera.screen_to_world(self.local(position), self.vp());
        let zoom = self.camera.zoom;
        self.sky.constellations.iter().position(|c| {
            let d = ((c.center.0 - x).powi(2) + (c.center.1 - y).powi(2)).sqrt();
            d <= c.extent + 34.0 / zoom
        })
    }
}

impl Render for StarMap {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let mut animating = false;
        if let Some(last) = self.frame_log.as_mut() {
            eprintln!("CCV_FRAME {:.2}", now.duration_since(*last).as_secs_f32() * 1000.0);
            *last = now;
            // 計測中は動かし続ける
            window.request_animation_frame();
            if self.orbit {
                self.flight = None;
                self.camera.zoom = (self.camera.zoom * 1.002).min(camera::MAX_ZOOM);
                self.camera.pan((0.6, 0.3));
            }
        }

        // ペインの幅が変わったら、全景を合わせ直す
        let vp = self.vp();
        if vp != self.last_vp {
            self.last_vp = vp;
            if self.at_overview && !self.needs_fit {
                let to = self.fit_camera();
                self.fly(to, cx);
            }
        }

        if self.needs_fit && f32::from(self.viewport.get().size.width) > 1.0 {
            self.camera = self.fit_camera();
            self.needs_fit = false;
        } else if self.needs_fit {
            animating = true;
        }

        if let Some(f) = &self.flight {
            let t = now.duration_since(f.start).as_secs_f32() / f.duration.as_secs_f32().max(0.001);
            self.camera = Camera::lerp(f.from, f.to, t);
            if t >= 1.0 {
                self.flight = None;
            } else {
                animating = true;
            }
        }

        let mut morph_positions = None;
        if let Some(m) = &self.morph {
            let t = now.duration_since(m.start).as_secs_f32() / m.duration.as_secs_f32().max(0.001);
            if t >= 1.0 {
                self.morph = None;
            } else {
                animating = true;
                let e = ease_in_out_cubic(t);
                let list: Vec<(f32, f32)> = m
                    .from
                    .iter()
                    .zip(self.positions.iter())
                    .map(|(a, b)| (a.0 + (b.0 - a.0) * e, a.1 + (b.1 - a.1) * e))
                    .collect();
                morph_positions = Some(Rc::new(list));
            }
        }

        let reveal = match self.reveal_start {
            Some(start) => {
                let t = now.duration_since(start).as_secs_f32()
                    / self.reveal_duration.as_secs_f32().max(0.001);
                if t >= 1.0 {
                    self.reveal_start = None;
                    1.0
                } else {
                    animating = true;
                    t
                }
            }
            None => 1.0,
        };
        if animating {
            window.request_animation_frame();
        }

        let sky = self.sky.clone();
        let positions = morph_positions.unwrap_or_else(|| self.positions.clone());
        let camera = self.camera;
        let hovered = self.hovered;
        let selected = self.selected;
        let lit = self.lit.clone();
        let route = self.route.clone();
        let viewport = self.viewport.clone();

        div()
            .id("starmap")
            .size_full()
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.on_scroll(e, cx)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, _, _| {
                    this.drag = Some((f32::from(e.position.x), f32::from(e.position.y)));
                    this.dragged = false;
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
                    if let Some(i) = this.star_at(e.position) {
                        cx.emit(StarMapEvent::OpenFile(i));
                    } else if let Some(c) = this.constellation_at(e.position) {
                        cx.emit(StarMapEvent::OpenConstellation(c));
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if let Some(last) = this.drag {
                    let now = (f32::from(e.position.x), f32::from(e.position.y));
                    let delta = (now.0 - last.0, now.1 - last.1);
                    if delta.0.abs() + delta.1.abs() > 1.0 {
                        this.dragged = true;
                    }
                    this.drag = Some(now);
                    this.flight = None;
                    this.at_overview = false;
                    this.camera.pan(delta);
                    cx.notify();
                    return;
                }
                let hover = this.star_at(e.position);
                if hover != this.hovered {
                    this.hovered = hover;
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| viewport.set(bounds),
                    move |bounds, _, window, cx| {
                        let frame = Frame {
                            sky: &sky,
                            positions: &positions,
                            camera,
                            hovered,
                            lit: &lit,
                            selected,
                            route: &route,
                            reveal,
                        };
                        paint::paint(bounds, &frame, window, cx);
                    },
                )
                .size_full(),
            )
    }
}
