//! Direct manipulation of a clip's Motion in the Program monitor, as in Premiere: with the
//! Selection tool, a click on a clip in the picture selects it (the topmost one there that is not
//! a title; titles' layers have their own handles, `panels::graphics`), and the selected clip shows
//! its bounding box. Dragging inside the box moves it (Position; Shift: one direction), a corner
//! handle scales it about its anchor point (uniformly while Uniform Scale is on), an edge handle
//! scales its width or height (both, with Uniform Scale on), just outside a corner rotates it
//! (Shift: 15° steps), and the anchor point's crosshair moves the anchor point with the picture
//! staying where it is.
//!
//! A selected mask (or the mask pen) takes the picture over instead.
//!
//! Each drag changes Motion as it goes (`effects.setParam`, so the picture follows and an animated
//! parameter gets a keyframe at the playhead) and is one undo step.
//!
//! Automation ids: `program.motion.box`, `program.motion.handle.<0-7>` (corners from the top
//! left clockwise, then the top, right, bottom and left edges), `program.motion.rotate.<0-3>`,
//! `program.motion.anchor`.

use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ItemKind, TrackItem, TrackKind};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::state::Tool;

/// A visible clip's picture on the monitor.
struct ClipView {
    clip: u64,
    /// Source pixels → screen.
    m: Affine,
    src: (f64, f64),
    item: TrackItem,
}

impl ClipView {
    fn corners(&self) -> [Pos2; 4] {
        let (w, h) = self.src;
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| sp(&self.m, x, y))
    }
    /// Corners, then the middles of the top, right, bottom and left edges.
    fn handles(&self) -> [Pos2; 8] {
        let q = self.corners();
        let mid = |a: Pos2, b: Pos2| pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        [q[0], q[1], q[2], q[3], mid(q[0], q[1]), mid(q[1], q[2]), mid(q[2], q[3]), mid(q[3], q[0])]
    }
    fn hit(&self, p: Pos2) -> bool {
        let q = self.corners();
        // inside the (convex) quad: the same side of every edge
        let side = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        let s: Vec<f32> = (0..4).map(|i| side(q[i], q[(i + 1) % 4])).collect();
        s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
    }
    fn anchor_screen(&self, anchor: Vec2) -> Pos2 {
        sp(&self.m, anchor.x, anchor.y)
    }
}

fn sp(m: &Affine, x: f64, y: f64) -> Pos2 {
    let v = m.apply(Vec2::new(x, y));
    pos2(v.x as f32, v.y as f32)
}

/// What a Motion drag changes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Move,
    Scale(usize),
    Rotate,
    Anchor,
}

#[derive(Clone, Debug)]
struct MotionDrag {
    clip: u64,
    kind: Kind,
    start: Pos2,
    pos0: Vec2,
    anchor0: Vec2,
    /// Scale (height) and Scale Width, %.
    scale0: (f64, f64),
    uniform: bool,
    rot0: f64,
    /// Source pixels → screen when the drag began.
    m0: Affine,
    /// Screen pixels per sequence pixel.
    k: f64,
}

/// Whether a Motion drag holds the pointer (the graphics overlay then leaves the press alone).
pub fn claimed(ui: &egui::Ui) -> bool {
    ui.data(|d| d.get_temp::<MotionDrag>(drag_id()).is_some())
}

fn drag_id() -> egui::Id {
    egui::Id::new("program-motion-drag")
}

/// The Motion values of a clip at media time `mt`: position, anchor (unset: the centre of the
/// sequence / of the source), scale, scale width, uniform, rotation.
fn motion_values(it: &TrackItem, mt: filmcraft_time::Tick, seq: (f64, f64), src: (f64, f64)) -> (Vec2, Vec2, f64, f64, bool, f64) {
    let Some(m) = it.effect("motion") else { return (Vec2::new(seq.0 / 2.0, seq.1 / 2.0), Vec2::new(src.0 / 2.0, src.1 / 2.0), 100.0, 100.0, true, 0.0) };
    let p = m.vec2_at("position", mt);
    let pos = if p.x.is_nan() || p.y.is_nan() || m.param("position").is_none() { Vec2::new(seq.0 / 2.0, seq.1 / 2.0) } else { p };
    let a = m.vec2_at("anchor", mt);
    let anchor = if a.x.is_nan() || a.y.is_nan() || m.param("anchor").is_none() { Vec2::new(src.0 / 2.0, src.1 / 2.0) } else { a };
    let uniform = m.param("uniform_scale").and_then(|p| p.value.as_bool()).unwrap_or(true);
    (pos, anchor, m.f64_at("scale", mt), m.f64_at("scale_width", mt), uniform, m.f64_at("rotation", mt))
}

/// The visible non-title video clips at the playhead, topmost first.
fn clip_views(app: &FilmcraftApp, pic: Rect, frame: (u32, u32)) -> Vec<ClipView> {
    let Some(seq) = app.session.active_sequence() else { return Vec::new() };
    let ph = app.session.playhead();
    let k = pic.width() as f64 / frame.0.max(1) as f64;
    let to_screen = Affine::translate(pic.min.x as f64, pic.min.y as f64).then_apply(&Affine::scale(k, k));
    let mut out = Vec::new();
    for t in seq.tracks(TrackKind::Video).iter().rev().filter(|t| t.enabled) {
        for it in t.items.iter().filter(|c| c.enabled && c.start <= ph && ph < c.end()) {
            let kind = app.session.project.item(it.item).map(|i| &i.kind);
            if matches!(kind, Some(ItemKind::Graphic { .. } | ItemKind::AdjustmentLayer { .. }) | None) {
                continue;
            }
            let Some(src) = filmcraft_render::source_size(&app.session.project, it.item) else { continue };
            let m = filmcraft_render::motion_matrix(seq, it, src, app.session.project.source_par(it.item), it.effect_time_at(ph));
            out.push(ClipView { clip: it.id.0, m: to_screen.then_apply(&m), src: (src.0 as f64, src.1 as f64), item: it.clone() });
        }
    }
    out
}

/// Draw the selected clip's box and run Motion drags on the Program monitor picture `pic` (drawn
/// inside the monitor's video `area`).
pub fn overlay(app: &mut FilmcraftApp, ui: &mut egui::Ui, pic: Rect, area: Rect, frame: (u32, u32)) {
    // (a selected mask or the mask pen edits on the picture instead, as in Premiere)
    let masking = app.session.state.selected_mask.is_some() || app.ui.mask_pen.is_some();
    if app.ui.tool != Tool::Selection || app.playback.playing || app.ui.gfx_edit.is_some() || masking {
        ui.data_mut(|d| d.remove::<MotionDrag>(drag_id()));
        return;
    }
    let Some(seq) = app.session.active_sequence() else { return };
    let seq_size = (seq.settings.width as f64, seq.settings.height as f64);
    let ph = app.session.playhead();
    let views = clip_views(app, pic, frame);
    let selected: Vec<u64> = app.session.state.selection.iter().map(|c| c.0).collect();
    let sel = views.iter().find(|v| selected.contains(&v.clip));
    // (inside the monitor's video area: never over its transport)
    let painter = ui.painter().with_clip_rect(area);
    let k = pic.width() as f64 / frame.0.max(1) as f64;
    let accent = app.tokens.accent;
    // the selected clip's box, handles and anchor point
    let mut anchor_at = None;
    if let Some(v) = sel {
        let q = v.corners();
        painter.add(egui::Shape::closed_line(q.to_vec(), Stroke::new(1.0, Color32::from_white_alpha(200))));
        app.auto.add("program.motion.box", Rect::from_points(&q), "clip box");
        for (n, h) in v.handles().iter().enumerate() {
            let r = Rect::from_center_size(*h, vec2(7.0, 7.0));
            painter.rect_filled(r, 0.0, Color32::WHITE);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(0x30)), egui::StrokeKind::Middle);
            app.auto.add(&format!("program.motion.handle.{n}"), r.expand(3.0), "scale handle");
        }
        let (_, anchor, ..) = motion_values(&v.item, v.item.effect_time_at(ph), seq_size, v.src);
        let a = v.anchor_screen(anchor);
        painter.circle_stroke(a, 5.0, Stroke::new(1.2, accent));
        painter.line_segment([a - vec2(8.0, 0.0), a + vec2(8.0, 0.0)], Stroke::new(1.2, accent));
        painter.line_segment([a - vec2(0.0, 8.0), a + vec2(0.0, 8.0)], Stroke::new(1.2, accent));
        app.auto.add("program.motion.anchor", Rect::from_center_size(a, vec2(14.0, 14.0)), "anchor point");
        for (n, c) in q.iter().enumerate() {
            // the rotation zone: just outside the corner, away from the box's middle
            let out = (*c - Rect::from_points(&q).center()).normalized() * 14.0;
            app.auto.add(&format!("program.motion.rotate.{n}"), Rect::from_center_size(*c + out, vec2(10.0, 10.0)), "rotate");
        }
        anchor_at = Some(a);
    }
    // what the pointer is over (for the cursor and a press)
    let over = |p: Pos2| -> Option<(u64, Kind)> {
        if let (Some(v), Some(a)) = (sel, anchor_at) {
            if (p - a).length() <= 7.0 {
                return Some((v.clip, Kind::Anchor));
            }
            if let Some(n) = v.handles().iter().position(|h| (*h - p).length() <= 7.0) {
                return Some((v.clip, Kind::Scale(n)));
            }
            let q = v.corners();
            let mid = Rect::from_points(&q).center();
            if !v.hit(p) && q.iter().any(|c| (*c - p).length() <= 22.0 && (p - mid).length() > (*c - mid).length()) {
                return Some((v.clip, Kind::Rotate));
            }
            if v.hit(p) {
                return Some((v.clip, Kind::Move));
            }
        }
        views.iter().find(|v| v.hit(p)).map(|v| (v.clip, Kind::Move))
    };
    let on_title_layer = |app: &FilmcraftApp, p: Pos2| crate::panels::graphics::layer_at(app, pic, frame, p);
    let hover = ui.ctx().pointer_hover_pos().filter(|p| pic.expand(24.0).contains(*p));
    let mut drag: Option<MotionDrag> = ui.data(|d| d.get_temp(drag_id()));
    if drag.is_none()
        && let Some(p) = hover
        && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
    {
        let target = over(p).filter(|(c, kind)| *kind != Kind::Move || Some(*c) == sel.map(|v| v.clip) || !on_title_layer(app, p));
        if let Some((_, kind)) = target {
            let icon = match kind {
                Kind::Move => egui::CursorIcon::Move,
                Kind::Anchor => egui::CursorIcon::Crosshair,
                Kind::Rotate => egui::CursorIcon::Alias,
                Kind::Scale(0 | 2) => egui::CursorIcon::ResizeNwSe,
                Kind::Scale(1 | 3) => egui::CursorIcon::ResizeNeSw,
                Kind::Scale(5 | 7) => egui::CursorIcon::ResizeHorizontal,
                Kind::Scale(_) => egui::CursorIcon::ResizeVertical,
            };
            // a title layer under the pointer belongs to the graphics overlay
            if !(kind == Kind::Move && on_title_layer(app, p)) {
                ui.ctx().set_cursor_icon(icon);
            }
        }
        // a press: select the clip under it, start the drag
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        if pressed
            && pic.expand(24.0).contains(p)
            && let Some((clip, kind)) = target
        {
            let title = kind == Kind::Move && on_title_layer(app, p);
            if !title && let Some(v) = views.iter().find(|v| v.clip == clip) {
                if !selected.contains(&clip) {
                    let _ = app.session.execute("timeline.select", json!({"clips": [clip]}));
                }
                let (pos0, anchor0, s, sw, uniform, rot0) = motion_values(&v.item, v.item.effect_time_at(ph), seq_size, v.src);
                let d = MotionDrag { clip, kind, start: p, pos0, anchor0, scale0: (s, sw), uniform, rot0, m0: v.m, k };
                ui.data_mut(|dd| dd.insert_temp(drag_id(), d.clone()));
                drag = Some(d);
            }
        }
    }
    let Some(d) = drag else { return };
    let (down, cur, shift) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.modifiers.shift));
    if !down {
        ui.data_mut(|dd| dd.remove::<MotionDrag>(drag_id()));
        return;
    }
    let Some(cur) = cur else { return };
    let Some(v) = views.iter().find(|v| v.clip == d.clip) else {
        ui.data_mut(|dd| dd.remove::<MotionDrag>(drag_id()));
        return;
    };
    let mut set: Vec<(&str, Value)> = Vec::new();
    let delta = cur - d.start;
    let a0 = sp(&d.m0, d.anchor0.x, d.anchor0.y);
    match d.kind {
        Kind::Move => {
            let mut dl = delta;
            if shift {
                if dl.x.abs() > dl.y.abs() {
                    dl.y = 0.0;
                } else {
                    dl.x = 0.0;
                }
            }
            set.push(("position", json!([d.pos0.x + dl.x as f64 / d.k, d.pos0.y + dl.y as f64 / d.k])));
        }
        Kind::Anchor => {
            let lin = Affine { e: 0.0, f: 0.0, ..d.m0 };
            let da = lin.inverse().map(|i| i.apply(Vec2::new(delta.x as f64, delta.y as f64))).unwrap_or_default();
            set.push(("anchor", json!([d.anchor0.x + da.x, d.anchor0.y + da.y])));
            set.push(("position", json!([d.pos0.x + delta.x as f64 / d.k, d.pos0.y + delta.y as f64 / d.k])));
        }
        Kind::Rotate => {
            let ang = |p: Pos2| ((p.y - a0.y) as f64).atan2((p.x - a0.x) as f64).to_degrees();
            let mut r = d.rot0 + ang(cur) - ang(d.start);
            if shift {
                r = (r / 15.0).round() * 15.0;
            }
            set.push(("rotation", json!((r * 10.0).round() / 10.0)));
        }
        Kind::Scale(n) => {
            // in the clip's own pixels, about its anchor point
            let inv = d.m0.inverse();
            let local = |p: Pos2| inv.map(|i| i.apply(Vec2::new(p.x as f64, p.y as f64))).unwrap_or_default();
            let (ls, lc) = (local(d.start), local(cur));
            let ratio = |now: f64, was: f64, a: f64| if (was - a).abs() > 1e-6 { ((now - a) / (was - a)).max(0.0) } else { 1.0 };
            let fx = ratio(lc.x, ls.x, d.anchor0.x);
            let fy = ratio(lc.y, ls.y, d.anchor0.y);
            let (s0, sw0) = d.scale0;
            let (nh, nw) = match n {
                // corners: uniform keeps the proportions (the larger change), else each axis
                0..=3 if d.uniform || shift => {
                    let f = if (fx - 1.0).abs() > (fy - 1.0).abs() { fx } else { fy };
                    (s0 * f, sw0 * f)
                }
                0..=3 => (s0 * fy, sw0 * fx),
                // edges: one axis (both with Uniform Scale on)
                5 | 7 if d.uniform => (s0 * fx, sw0 * fx),
                4 | 6 if d.uniform => (s0 * fy, sw0 * fy),
                5 | 7 => (s0, sw0 * fx),
                _ => (s0 * fy, sw0),
            };
            let round = |x: f64| (x * 10.0).round() / 10.0;
            set.push(("scale", json!(round(nh))));
            if !d.uniform {
                set.push(("scale_width", json!(round(nw))));
            }
        }
    }
    // what changed since the last frame (the merged edit makes the gesture one undo step)
    let (pos, anchor, s, sw, _, rot) = motion_values(&v.item, v.item.effect_time_at(ph), seq_size, v.src);
    let differs = |id: &str, val: &Value| -> bool {
        let num = |i: usize| val.get(i).and_then(Value::as_f64).unwrap_or(0.0);
        match id {
            "position" => (num(0) - pos.x).abs() > 1e-6 || (num(1) - pos.y).abs() > 1e-6,
            "anchor" => (num(0) - anchor.x).abs() > 1e-6 || (num(1) - anchor.y).abs() > 1e-6,
            "scale" => (val.as_f64().unwrap_or(s) - s).abs() > 1e-6,
            "scale_width" => (val.as_f64().unwrap_or(sw) - sw).abs() > 1e-6,
            _ => (val.as_f64().unwrap_or(rot) - rot).abs() > 1e-6,
        }
    };
    for (id, val) in set {
        if differs(id, &val)
            && let Err(e) = app.session.execute("effects.setParam", json!({"clip": d.clip, "effect": "motion", "param": id, "value": val}))
        {
            app.ui.status = e.to_string();
        }
    }
    ui.ctx().request_repaint();
}
