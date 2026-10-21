//! Effect Controls' keyframe area, as in Premiere: the lane's view of the clip (the zoom bar under
//! it, Ctrl / Alt + wheel to zoom about the pointer, Shift + wheel to pan, a double-click on the
//! bar shows the whole clip), keyframes dragged along the lane, and a parameter's value and
//! velocity graphs (keyframes dragged in time and value, Bezier influence handles, the graph's
//! height dragged by its bottom edge).
//!
//! Every drag changes the project as it goes, so the Program monitor follows it, and is one undo
//! step (the session merges a gesture's edits). A drag is a session kept apart from the widgets
//! (whose ids carry keyframe times that change as they move): its start, what it started from,
//! and what it has applied so far. Keyframes never cross their neighbours: a drag stops a frame
//! short of the next keyframe (moving onto one would replace it).
//!
//! Automation ids: `effectControls.zoomBar`, `…zoomBar.start`, `…zoomBar.end`,
//! `effectControls.<param>.graph.keyframe.<media ticks>[.x|.y]`, `…graph.in.<t>`, `…graph.out.<t>`,
//! `effectControls.graphResize`.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::{ClipId, Interpolation, ParamKind, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::state::{EcView, KfSel};
use crate::theme::Tokens;

/// The keyframe lane's mapping between timeline time and x, over the part of the clip it shows.
#[derive(Clone, Copy, Debug)]
pub struct Lane {
    pub rect: Rect,
    /// The timeline range shown.
    pub t0: Tick,
    pub t1: Tick,
    start: Tick,
    end: Tick,
    source_in: Tick,
    speed: f64,
    /// A timeline frame.
    pub fd: Tick,
}

impl Lane {
    /// The lane of `it` in `rect`: the stored view when it is this clip's and inside it, else
    /// the whole clip.
    pub fn new(rect: Rect, it: &TrackItem, view: Option<EcView>, fd: Tick) -> Self {
        let (start, end) = (it.start, it.end());
        let (t0, t1) = match view.filter(|v| v.clip == it.id.0) {
            Some(v) => {
                let a = Tick(v.start).clamp(start, end);
                let b = Tick(v.end).clamp(start, end);
                if b.0 - a.0 >= fd.0.max(1) { (a, b) } else { (start, end) }
            }
            None => (start, end),
        };
        Lane { rect, t0, t1, start, end, source_in: it.source_in, speed: it.speed.abs().max(1e-6), fd: Tick(fd.0.max(1)) }
    }
    fn span(&self) -> f64 {
        (self.t1 - self.t0).0.max(1) as f64
    }
    pub fn x(&self, tl: Tick) -> f32 {
        self.rect.min.x + ((tl - self.t0).0 as f64 / self.span()) as f32 * self.rect.width()
    }
    pub fn tl_at(&self, x: f32) -> Tick {
        self.t0 + Tick((((x - self.rect.min.x) / self.rect.width().max(1.0)) as f64 * self.span()) as i64)
    }
    /// Timeline time of a media time (keyframes are stored at media times).
    pub fn tl_of(&self, mt: Tick) -> Tick {
        self.start + Tick(((mt - self.source_in).0 as f64 / self.speed) as i64)
    }
    pub fn mt_of(&self, tl: Tick) -> Tick {
        self.source_in + Tick(((tl - self.start).0 as f64 * self.speed).round() as i64)
    }
    pub fn x_of_media(&self, mt: Tick) -> f32 {
        self.x(self.tl_of(mt))
    }
    /// Timeline ticks per pixel.
    pub fn ticks_per_px(&self) -> f64 {
        self.span() / self.rect.width().max(1.0) as f64
    }
    /// Whether x is in the lane (with room for a keyframe's half width).
    pub fn shows(&self, x: f32) -> bool {
        x >= self.rect.min.x - 6.0 && x <= self.rect.max.x + 6.0
    }
    /// A timeline duration rounded to whole frames.
    pub fn frames(&self, ticks: f64) -> i64 {
        (ticks / self.fd.0 as f64).round() as i64 * self.fd.0
    }
}

// ----------------------------------------------------------------------------------- the view

/// Ctrl / Alt + wheel over the lane zooms about the pointer, Shift + wheel (or a sideways wheel)
/// pans. Consumed, so the property list does not scroll too.
pub(crate) fn wheel(app: &mut FilmcraftApp, ui: &egui::Ui, lane: &Lane, clip: ClipId) {
    // the wheel events themselves: their modifiers and where the pointer was (scripted input
    // sets them on the event alone)
    let (zoom, mut m, mut at) = ui.input(|i| (i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
    let mut wheel = egui::Vec2::ZERO;
    ui.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::PointerMoved(p) => at = Some(*p),
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let k = match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => 40.0,
                        egui::MouseWheelUnit::Page => 400.0,
                    };
                    wheel += *delta * k;
                    m |= *modifiers;
                }
                _ => {}
            }
        }
    });
    // (each wheel step at once, not spread over frames as smooth scrolling does)
    let scroll = wheel;
    let Some(p) = at.filter(|p| lane.rect.contains(*p)) else { return };
    let mut span = lane.span();
    let mut t0 = lane.t0.0 as f64;
    let mut changed = false;
    // Ctrl + wheel arrives as zoom; Alt + wheel as a scroll
    let factor = if (zoom - 1.0).abs() > 1e-4 {
        1.0 / zoom as f64
    } else if m.alt && scroll.y.abs() > 0.1 {
        (-scroll.y as f64 * 0.004).exp()
    } else {
        1.0
    };
    if (factor - 1.0).abs() > 1e-6 {
        let at = lane.tl_at(p.x).0 as f64;
        let ns = (span * factor).clamp(lane.fd.0 as f64 * 4.0, (lane.end - lane.start).0.max(1) as f64);
        t0 = at - (at - t0) * ns / span;
        span = ns;
        changed = true;
    }
    let pan = if m.shift && !m.alt { scroll.y + scroll.x } else { scroll.x };
    if !changed && pan.abs() > 0.1 {
        t0 -= pan as f64 * lane.ticks_per_px();
        changed = true;
    }
    if changed {
        set_view(app, lane, clip, t0, span);
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    }
}

/// Show `span` ticks of the clip from `t0` (kept inside the clip).
fn set_view(app: &mut FilmcraftApp, lane: &Lane, clip: ClipId, t0: f64, span: f64) {
    let whole = (lane.end - lane.start).0.max(1) as f64;
    let span = span.clamp(lane.fd.0 as f64 * 4.0, whole);
    let t0 = t0.clamp(lane.start.0 as f64, lane.end.0 as f64 - span);
    let (a, b) = (t0.round() as i64, (t0 + span).round() as i64);
    app.ui.ec_view = if a <= lane.start.0 && b >= lane.end.0 { None } else { Some(EcView { clip: clip.0, start: a, end: b }) };
}

/// The zoom bar under the lane: the shown part of the clip as a thumb; its body pans, its round
/// ends zoom, a double-click shows the whole clip.
pub(crate) fn zoom_bar(app: &mut FilmcraftApp, ui: &mut egui::Ui, bar: Rect, lane: &Lane, clip: ClipId) {
    let t = app.tokens;
    let whole = (lane.end - lane.start).0.max(1) as f64;
    let fx = |tk: Tick| bar.min.x + ((tk - lane.start).0 as f64 / whole) as f32 * bar.width();
    let track = Rect::from_min_max(pos2(bar.min.x, bar.center().y - 3.0), pos2(bar.max.x, bar.center().y + 3.0));
    ui.painter().rect_filled(track, 3.0, Color32::from_gray(0x2a));
    // (wide enough to grab its middle between the ends)
    let a = fx(lane.t0).min(bar.max.x - 28.0);
    let b = fx(lane.t1).max(a + 28.0);
    let thumb = Rect::from_min_max(pos2(a, bar.center().y - 4.0), pos2(b, bar.center().y + 4.0));
    let id = egui::Id::new(("ec-zoombar", clip.0));
    let body = ui.interact(thumb, id.with("body"), Sense::click_and_drag());
    let ends = [(a, "start", id.with("start")), (b, "end", id.with("end"))];
    let hover = body.hovered() || body.dragged();
    ui.painter().rect_filled(thumb, 4.0, if hover { Color32::from_gray(0x8a) } else { Color32::from_gray(0x6a) });
    let span = lane.span();
    let per_px = whole / bar.width().max(1.0) as f64;
    if body.dragged() {
        let dt = body.drag_delta().x as f64 * per_px;
        set_view(app, lane, clip, lane.t0.0 as f64 + dt, span);
    }
    if body.double_clicked() {
        app.ui.ec_view = None;
    }
    app.auto.add("effectControls.zoomBar", thumb, "zoom bar");
    for (x, name, hid) in ends {
        let c = pos2(x, bar.center().y);
        // each end's grip reaches outward, leaving the thumb's middle to the thumb
        let grip = if name == "start" {
            Rect::from_min_max(pos2(x - 9.0, bar.min.y), pos2(x + 4.0, bar.max.y))
        } else {
            Rect::from_min_max(pos2(x - 4.0, bar.min.y), pos2(x + 9.0, bar.max.y))
        };
        let r = ui.interact(grip, hid, Sense::drag());
        ui.painter().circle_filled(c, 5.0, if r.hovered() || r.dragged() { Color32::WHITE } else { Color32::from_gray(0xc8) });
        ui.painter().circle_stroke(c, 5.0, Stroke::new(1.0, t.panel_bg));
        app.auto.add(&format!("effectControls.zoomBar.{name}"), grip, name);
        if r.dragged() {
            let dt = r.drag_delta().x as f64 * per_px;
            let (mut s0, mut s1) = (lane.t0.0 as f64, lane.t1.0 as f64);
            if name == "start" {
                s0 = (s0 + dt).min(s1 - lane.fd.0 as f64 * 4.0);
            } else {
                s1 = (s1 + dt).max(s0 + lane.fd.0 as f64 * 4.0);
            }
            set_view(app, lane, clip, s0, s1 - s0);
        }
    }
}

// -------------------------------------------------------------------------- lane keyframe drags

/// A drag of the selected keyframes along the lane.
#[derive(Clone, Debug)]
struct LaneDrag {
    clip: u64,
    start_x: f32,
    /// Timeline ticks moved so far.
    applied: i64,
    /// How far they may go (timeline ticks): not past a neighbour, not out of the clip.
    lo: i64,
    hi: i64,
}

fn lane_drag_id(clip: ClipId) -> egui::Id {
    egui::Id::new(("ec-lane-drag", clip.0))
}

/// A keyframe in the lane started a drag (the selection is already updated).
pub(crate) fn start_lane_drag(ui: &egui::Ui, clip: ClipId) {
    let x = ui.input(|i| i.pointer.press_origin().map(|p| p.x));
    if let Some(x) = x {
        ui.data_mut(|d| d.insert_temp(lane_drag_id(clip).with("start"), x));
    }
}

/// Run the lane drag: started this frame (bounds computed from the selection), moved as the
/// pointer goes (whole frames), ended when the button is released.
pub(crate) fn run_lane_drag(app: &mut FilmcraftApp, ui: &egui::Ui, it: &TrackItem, lane: &Lane, actions: &mut Vec<(String, Value)>) {
    let id = lane_drag_id(it.id);
    if let Some(x) = ui.data_mut(|d| d.remove_temp::<f32>(id.with("start"))) {
        let (lo, hi) = lane_bounds(app, it, lane);
        ui.data_mut(|d| d.insert_temp(id, LaneDrag { clip: it.id.0, start_x: x, applied: 0, lo, hi }));
    }
    let Some(mut drag) = ui.data(|d| d.get_temp::<LaneDrag>(id)) else { return };
    let (down, px, shift) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos().map(|p| p.x), i.modifiers.shift));
    if !down {
        ui.data_mut(|d| d.remove::<LaneDrag>(id));
        return;
    }
    let Some(px) = px else { return };
    let mut want = lane.frames((px - drag.start_x) as f64 * lane.ticks_per_px());
    // Shift: snap the dragged keyframes' first one to the playhead (as Premiere)
    if shift && let Some(first) = app.ui.kf_selected.iter().filter(|s| s.clip == drag.clip).map(|s| lane.tl_of(Tick(s.time)).0).min() {
        let ph = app.session.playhead().0;
        if (first + want - ph).abs() as f64 <= 8.0 * lane.ticks_per_px() {
            want = ph - first;
        }
    }
    let want = if drag.lo <= drag.hi { want.clamp(drag.lo, drag.hi) } else { 0 };
    let delta = want - drag.applied;
    if delta == 0 {
        return;
    }
    let by_media = (delta as f64 * it.speed.abs()).round() as i64;
    let keys: Vec<Value> = app.ui.kf_selected.iter().filter(|s| s.clip == drag.clip).map(KfSel::key_json).collect();
    if by_media != 0 && !keys.is_empty() {
        actions.push(("effects.moveKeyframes".into(), json!({"clip": drag.clip, "keys": keys, "by": by_media})));
        for s in app.ui.kf_selected.iter_mut().filter(|s| s.clip == drag.clip) {
            s.time += by_media;
        }
    }
    drag.applied = want;
    ui.data_mut(|d| d.insert_temp(id, drag));
}

/// How far the selected keyframes of `it` may move (timeline ticks): each stays a frame short of
/// the nearest unselected keyframe of its parameter and inside the clip.
fn lane_bounds(app: &FilmcraftApp, it: &TrackItem, lane: &Lane) -> (i64, i64) {
    let sel: Vec<&KfSel> = app.ui.kf_selected.iter().filter(|s| s.clip == it.id.0).collect();
    let (mut lo, mut hi) = (i64::MIN / 4, i64::MAX / 4);
    let fd = lane.fd.0;
    for s in &sel {
        let Some(e) = it.effects.get(s.effect) else { continue };
        let p = match s.mask {
            Some(m) => e.masks.get(m).and_then(|x| x.param(&s.param)),
            None => e.params.get(&s.param),
        };
        let Some(p) = p else { continue };
        let me = lane.tl_of(Tick(s.time)).0;
        lo = lo.max(lane.start.0 - me);
        hi = hi.min(lane.end.0 - fd - me);
        let moving = |t: Tick| sel.iter().any(|o| o.effect == s.effect && o.mask == s.mask && o.param == s.param && o.time == t.0);
        for k in p.keyframes.iter().filter(|k| !moving(k.time)) {
            let other = lane.tl_of(k.time).0;
            if other < me {
                lo = lo.max(other - me + fd);
            } else if other > me {
                hi = hi.min(other - me - fd);
            }
        }
    }
    (lo, hi)
}

// ----------------------------------------------------------------------------------- graphs

/// What a graph drag holds.
#[derive(Clone, Debug)]
struct GraphDrag {
    start: Pos2,
    /// The keyframe's media time when the drag began, and now.
    mt0: i64,
    cur: i64,
    /// Its value (components) when the drag began, and which component is dragged.
    v0: Vec<f64>,
    comp: usize,
    /// Value units per pixel (from the graph's range, frozen while dragging).
    upp: f64,
    /// Media times it may move between (exclusive: its neighbours).
    lo: i64,
    hi: i64,
    /// A Bezier handle (out: true) and its influence when the drag began, its segment's width.
    handle: Option<(bool, f64, f32)>,
}

pub(crate) fn graph_key(clip: ClipId, idx: usize, pid: &str) -> String {
    format!("graph:{}:{}:{}", clip.0, idx, pid)
}

/// Whether a parameter has graphs (an animated number or point).
pub(crate) fn graphable(param: &filmcraft_project::Param) -> bool {
    param.is_animated() && matches!(param.value, ParamValue::Float(_) | ParamValue::Vec2(_))
}

fn comps(v: &ParamValue) -> Vec<f64> {
    match v {
        ParamValue::Float(x) => vec![*x],
        ParamValue::Vec2(p) => vec![p.x, p.y],
        _ => Vec::new(),
    }
}

fn value_json(v0: &ParamValue, c: &[f64]) -> Value {
    match v0 {
        ParamValue::Vec2(_) => json!([c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)]),
        _ => json!(c.first().copied().unwrap_or(0.0)),
    }
}

const COMP_COLORS: [Color32; 2] = [Color32::from_rgb(0xe0, 0x70, 0x70), Color32::from_rgb(0x70, 0xc8, 0x80)];

/// Value and velocity graphs of an animated number or point, across the keyframe lane.
#[allow(clippy::too_many_arguments)]
pub(crate) fn graph_rows(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    body: Rect,
    clip: ClipId,
    idx: usize,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    param: &filmcraft_project::Param,
    lane: &Lane,
    it: &TrackItem,
    actions: &mut Vec<(String, Value)>,
) {
    let t = app.tokens;
    let with_mask = |mut v: Value| -> Value {
        if let Some(k) = mask {
            v["mask"] = json!(k);
        }
        v
    };
    let pkey = match mask {
        Some(k) => format!("mask{k}.{}", pd.id),
        None => pd.id.to_string(),
    };
    let gkey = graph_key(clip, idx, &pkey);
    let gh = app.ui.ec_graph_h.clamp(60.0, 600.0);
    let (vr, _) = ui.allocate_exact_size(vec2(body.width(), gh), Sense::hover());
    let (velr, _) = ui.allocate_exact_size(vec2(body.width(), 64.0), Sense::hover());
    let point = matches!(param.value, ParamValue::Vec2(_));
    let ncomp = if point { 2 } else { 1 };
    let area = Rect::from_min_max(pos2(lane.rect.min.x, vr.min.y + 4.0), pos2(lane.rect.max.x, vr.max.y - 6.0));
    // samples over the shown range
    let n = (lane.rect.width() / 2.0).max(8.0) as usize;
    let at = |i: usize| lane.mt_of(lane.t0 + Tick(((lane.t1 - lane.t0).0 as f64 * i as f64 / n as f64) as i64));
    let samples: Vec<Vec<f64>> = (0..=n).map(|i| comps(&param.value_at(at(i)))).collect();
    // the value range: what is shown and the keyframes, padded; frozen while a drag runs
    let drag_id = egui::Id::new(("ec-graph-drag", &gkey));
    let range_id = egui::Id::new(("ec-graph-range", &gkey));
    let dragging: Option<GraphDrag> = ui.data(|d| d.get_temp(drag_id));
    let (lo, hi) = match ui.data(|d| d.get_temp::<(f64, f64)>(range_id)).filter(|_| dragging.is_some()) {
        Some(r) => r,
        None => {
            let mut lo = f64::MAX;
            let mut hi = f64::MIN;
            for v in samples.iter().flatten().chain(param.keyframes.iter().flat_map(|k| comps(&k.value)).collect::<Vec<_>>().iter()) {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
            if hi < lo {
                (lo, hi) = (0.0, 1.0);
            }
            if hi - lo < 1e-6 {
                lo -= 1.0;
                hi += 1.0;
            }
            let pad = (hi - lo) * 0.12;
            (lo - pad, hi + pad)
        }
    };
    let y_of = |v: f64| area.max.y - ((v - lo) / (hi - lo)) as f32 * area.height();
    let p = ui.painter().with_clip_rect(Rect::from_min_max(pos2(body.min.x, vr.min.y), pos2(lane.rect.max.x, velr.max.y)));
    p.rect_filled(area, 0.0, Color32::from_rgb(0x19, 0x19, 0x19));
    for g in 1..4 {
        let y = area.min.y + area.height() * g as f32 / 4.0;
        p.line_segment([pos2(area.min.x, y), pos2(area.max.x, y)], Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2a, 0x2a)));
    }
    // the playhead through the graphs
    let ph = app.session.playhead();
    if ph >= lane.t0 && ph <= lane.t1 {
        let x = lane.x(ph);
        p.line_segment([pos2(x, area.min.y), pos2(x, velr.max.y - 4.0)], Stroke::new(1.0, t.playhead.gamma_multiply(0.6)));
    }
    let dec = if let ParamKind::Float { decimals, .. } = pd.kind { decimals as usize } else { 1 };
    p.text(pos2(vr.max.x - 30.0, area.min.y + 6.0), Align2::RIGHT_CENTER, format!("{hi:.dec$}"), Tokens::ui(10.0), t.text_dim);
    p.text(pos2(vr.max.x - 30.0, area.max.y - 6.0), Align2::RIGHT_CENTER, format!("{lo:.dec$}"), Tokens::ui(10.0), t.text_dim);
    p.text(pos2(vr.min.x + 44.0, area.center().y), Align2::LEFT_CENTER, "Value", Tokens::ui(11.0), t.text_dim);
    if point {
        for (c, name) in ["X", "Y"].iter().enumerate() {
            p.text(pos2(vr.min.x + 44.0, area.center().y + 16.0 + 14.0 * c as f32), Align2::LEFT_CENTER, *name, Tokens::ui(10.5), COMP_COLORS[c]);
        }
    }
    let x_at = |i: usize| lane.rect.min.x + lane.rect.width() * i as f32 / n as f32;
    for c in 0..ncomp {
        let line: Vec<Pos2> = samples.iter().enumerate().filter_map(|(i, v)| v.get(c).map(|v| pos2(x_at(i), y_of(*v)))).collect();
        p.add(egui::Shape::line(line, Stroke::new(1.6, if point { COMP_COLORS[c] } else { t.accent })));
    }
    // velocity: the value's rate of change per second (a point's speed)
    let secs = (lane.t1 - lane.t0).0 as f64 * it.speed.abs() / filmcraft_time::TICKS_PER_SECOND as f64 / n as f64;
    let vel: Vec<f64> = (0..n)
        .map(|i| {
            let (a, b) = (&samples[i], &samples[i + 1]);
            if point {
                a.iter().zip(b).map(|(x, y)| (y - x) * (y - x)).sum::<f64>().sqrt() / secs.max(1e-9)
            } else {
                (b.first().copied().unwrap_or(0.0) - a.first().copied().unwrap_or(0.0)) / secs.max(1e-9)
            }
        })
        .collect();
    let vmax = vel.iter().fold(1e-6f64, |a, v| a.max(v.abs())) * 1.15;
    let varea = Rect::from_min_max(pos2(lane.rect.min.x, velr.min.y + 2.0), pos2(lane.rect.max.x, velr.max.y - 4.0));
    p.rect_filled(varea, 0.0, Color32::from_rgb(0x19, 0x19, 0x19));
    let vy = |v: f64| if point { varea.max.y - (v / vmax) as f32 * varea.height() } else { varea.center().y - (v / vmax) as f32 * varea.height() / 2.0 };
    if !point {
        p.line_segment([pos2(varea.min.x, varea.center().y), pos2(varea.max.x, varea.center().y)], Stroke::new(1.0, Color32::from_rgb(0x33, 0x33, 0x33)));
    }
    let vline: Vec<Pos2> = vel.iter().enumerate().map(|(i, v)| pos2(lane.rect.min.x + lane.rect.width() * (i as f32 + 0.5) / n as f32, vy(*v))).collect();
    p.add(egui::Shape::line(vline, Stroke::new(1.2, Color32::from_rgb(0xd0, 0xa0, 0x40))));
    p.text(pos2(velr.min.x + 44.0, varea.center().y), Align2::LEFT_CENTER, "Velocity", Tokens::ui(11.0), t.text_dim);
    p.text(pos2(velr.max.x - 30.0, varea.min.y + 6.0), Align2::RIGHT_CENTER, format!("{vmax:.1}/s"), Tokens::ui(10.0), t.text_dim);
    // the graph's height: its bottom edge drags
    let edge = Rect::from_min_max(pos2(area.min.x, vr.max.y - 5.0), pos2(area.max.x, vr.max.y + 1.0));
    let er = ui.interact(edge, egui::Id::new(("ec-graph-h", &gkey)), Sense::drag());
    if er.hovered() || er.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        p.line_segment([pos2(edge.min.x, edge.center().y), pos2(edge.max.x, edge.center().y)], Stroke::new(2.0, t.accent));
    }
    if er.dragged() {
        app.ui.ec_graph_h = (app.ui.ec_graph_h + er.drag_delta().y).clamp(60.0, 600.0);
    }
    app.auto.add("effectControls.graphResize", edge, "graph height");
    // keyframes and their handles
    let fd_media = ((lane.fd.0 as f64) * it.speed.abs()).round().max(1.0) as i64;
    let ks = &param.keyframes;
    let units_per_px = (hi - lo) / area.height().max(1.0) as f64;
    let mut started: Option<GraphDrag> = None;
    let rate = app.session.sequence_rate();
    for (i, k) in ks.iter().enumerate() {
        let vals = comps(&k.value);
        let x = lane.x_of_media(k.time);
        if !lane.shows(x) {
            continue;
        }
        let neighbours = (ks.get(i.wrapping_sub(1)).map_or(i64::MIN / 4, |o| o.time.0), ks.get(i + 1).map_or(i64::MAX / 4, |o| o.time.0));
        // Bezier influence handles (numbers): flat tangents, length ∝ influence × the segment
        if !point && let Some(v) = vals.first() {
            let c = pos2(x, y_of(*v));
            let eases_out = !matches!(k.interp, Interpolation::Linear | Interpolation::Hold | Interpolation::EaseIn);
            let eases_in = !matches!(k.interp, Interpolation::Linear | Interpolation::Hold | Interpolation::EaseOut);
            for (out, on, nb) in [(true, eases_out, ks.get(i + 1)), (false, eases_in && i > 0, if i > 0 { ks.get(i - 1) } else { None })] {
                let (true, Some(nb)) = (on, nb) else { continue };
                let seg = (lane.x_of_media(nb.time) - x).abs();
                let infl = if out { k.out_influence } else { k.in_influence } as f32;
                let side = if out { 1.0 } else { -1.0 };
                let hp = pos2(x + side * infl * seg, c.y);
                p.line_segment([c, hp], Stroke::new(1.0, Color32::from_rgb(0x90, 0x90, 0x90)));
                let hr = Rect::from_center_size(hp, vec2(12.0, 12.0));
                let resp = ui.interact(hr, egui::Id::new(("ec-graph-handle", &gkey, k.time.0, out)), Sense::drag());
                p.circle_filled(hp, if resp.hovered() { 4.5 } else { 3.5 }, if resp.hovered() { Color32::WHITE } else { Color32::from_rgb(0xd0, 0xd0, 0xd0) });
                app.auto.add(&format!("effectControls.{pkey}.graph.{}.{}", if out { "out" } else { "in" }, k.time.0), hr, "influence");
                if resp.drag_started() {
                    started = Some(GraphDrag {
                        start: ui.input(|i| i.pointer.press_origin()).unwrap_or(hp),
                        mt0: k.time.0,
                        cur: k.time.0,
                        v0: vals.clone(),
                        comp: 0,
                        upp: units_per_px,
                        lo: neighbours.0,
                        hi: neighbours.1,
                        handle: Some((out, infl as f64, seg.max(1.0))),
                    });
                }
            }
        }
        for (c, v) in vals.iter().enumerate() {
            let kp = pos2(x, y_of(*v));
            let r = Rect::from_center_size(kp, vec2(12.0, 12.0));
            let resp = ui.interact(r, egui::Id::new(("ec-graph-key", &gkey, k.time.0, c)), Sense::drag());
            let hot = resp.hovered() || dragging.as_ref().is_some_and(|g| g.cur == k.time.0 && g.comp == c && g.handle.is_none());
            let col = if point { COMP_COLORS[c] } else { Color32::from_rgb(0xe0, 0xe0, 0xe0) };
            p.circle_filled(kp, if hot { 5.5 } else { 4.5 }, if hot { Color32::WHITE } else { col });
            let suffix = if point { if c == 0 { ".x" } else { ".y" } } else { "" };
            app.auto.add(&format!("effectControls.{pkey}.graph.keyframe.{}{suffix}", k.time.0), r, "keyframe value");
            let resp = resp.on_hover_text(format!(
                "{}  ·  {v:.dec$}",
                filmcraft_time::format_time(lane.tl_of(k.time), rate, false, filmcraft_time::TimeDisplay::Timecode, 48000)
            ));
            if resp.drag_started() {
                started = Some(GraphDrag {
                    start: ui.input(|i| i.pointer.press_origin()).unwrap_or(kp),
                    mt0: k.time.0,
                    cur: k.time.0,
                    v0: vals.clone(),
                    comp: c,
                    upp: units_per_px,
                    lo: neighbours.0,
                    hi: neighbours.1,
                    handle: None,
                });
            }
        }
    }
    if let Some(g) = started {
        ui.data_mut(|d| {
            d.insert_temp(range_id, (lo, hi));
            d.insert_temp(drag_id, g);
        });
        return;
    }
    // the running drag: applied as the pointer moves
    let Some(mut g) = dragging else { return };
    let (down, pos, shift) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.modifiers.shift));
    if !down {
        ui.data_mut(|d| {
            d.remove::<GraphDrag>(drag_id);
            d.remove::<(f64, f64)>(range_id);
        });
        return;
    }
    let Some(pos) = pos else { return };
    let mut d = pos - g.start;
    let Some(k) = param.keyframes.iter().find(|k| k.time.0 == g.cur) else {
        ui.data_mut(|dd| dd.remove::<GraphDrag>(drag_id));
        return;
    };
    let base = with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "mediaTime": g.cur}));
    match g.handle {
        Some((out, infl0, seg)) => {
            let side = if out { 1.0 } else { -1.0 };
            let ni = (infl0 + (d.x * side / seg) as f64).clamp(0.01, 1.0);
            let key = if out { "outInfluence" } else { "inInfluence" };
            let cur = if out { k.out_influence } else { k.in_influence };
            if (ni - cur).abs() > 1e-4 {
                let mut v = base;
                v[key] = json!(ni);
                actions.push(("effects.setKeyframe".into(), v));
            }
            p.text(pos + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, format!("{:.0} %", ni * 100.0), Tokens::ui(10.5), t.hot_text);
        }
        None => {
            // Shift: one direction only (the one moved more)
            if shift {
                if d.x.abs() > d.y.abs() {
                    d.y = 0.0;
                } else {
                    d.x = 0.0;
                }
            }
            let dt_media = ((lane.frames(d.x as f64 * lane.ticks_per_px()) as f64) * it.speed.abs()).round() as i64;
            let want_t = (g.mt0 + dt_media).clamp(g.lo + fd_media, g.hi - fd_media);
            let want_t = if g.lo + fd_media > g.hi - fd_media { g.cur } else { want_t };
            let mut vals = g.v0.clone();
            let nv = g.v0.get(g.comp).copied().unwrap_or(0.0) - d.y as f64 * g.upp;
            let nv = match pd.kind {
                ParamKind::Float { min, max, .. } => nv.clamp(min, max),
                _ => nv,
            };
            if let Some(x) = vals.get_mut(g.comp) {
                *x = nv;
            }
            let now = comps(&k.value);
            let moved_value = now.iter().zip(&vals).any(|(a, b)| (a - b).abs() > 1e-9);
            if want_t != g.cur || moved_value {
                let mut v = base;
                v["value"] = value_json(&k.value, &vals);
                if want_t != g.cur {
                    v["newTime"] = json!(want_t);
                }
                actions.push(("effects.setKeyframe".into(), v));
                // the keyframe selection follows it
                for s in app.ui.kf_selected.iter_mut().filter(|s| s.clip == clip.0 && s.effect == idx && s.mask == mask && s.param == pd.id && s.time == g.cur)
                {
                    s.time = want_t;
                }
                g.cur = want_t;
            }
            p.text(pos + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, format!("{nv:.dec$}"), Tokens::ui(10.5), t.hot_text);
        }
    }
    ui.data_mut(|dd| dd.insert_temp(drag_id, g));
}
