//! Clip keyframes in the timeline (Premiere's rubber band): a video clip's Opacity and an audio
//! clip's Volume drawn across the clip as a line with its keyframe diamonds.
//!
//! Dragging the line changes its value (with keyframes, the two either side of the grabbed
//! segment move together), ⌘-click or the Pen tool adds a keyframe on it, a diamond drags in time
//! and value, and right-click deletes it — how fades and dips are drawn on clips in Premiere.
//! Everything goes through `effects.setParam` / `effects.addKeyframe` / `effects.setKeyframe` /
//! `effects.deleteKeyframe`. Timeline ▸ settings ▸ Show Video / Audio Keyframes hide them.
//!
//! Presses on the line go through the Timeline's own hit-testing ([`on_line`], as the audio
//! clips' Volume line does), so a click there still selects the clip (Alt: one side of a link),
//! a right-click opens the clip menu and an Alt-drag copies the clip; only a plain drag, or a
//! ⌘-click / Pen click, is the band's.
//!
//! Automation ids: `timeline.clip.<id>.band` (the clip's band) and
//! `timeline.clip.<id>.band.kf.<n>` (its diamonds, in time order).

use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::{ClipId, Param, ParamValue, Sequence, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use super::timeline::{Drag, Layout};
use crate::FilmcraftApp;
use crate::icons::{self, Icon};

/// The parameter a clip's band shows: (effect, param).
fn target(kind: TrackKind) -> (&'static str, &'static str) {
    match kind {
        TrackKind::Video => ("opacity", "opacity"),
        TrackKind::Audio => ("volume", "level"),
    }
}

/// 0 (bottom) … 1 (top) for a value: opacity %, or volume dB on the fader's taper.
fn norm(kind: TrackKind, v: f64) -> f32 {
    match kind {
        TrackKind::Video => (v / 100.0).clamp(0.0, 1.0) as f32,
        TrackKind::Audio => super::mixer::db_to_pos(v),
    }
}

fn denorm(kind: TrackKind, n: f32) -> f64 {
    match kind {
        TrackKind::Video => (f64::from(n.clamp(0.0, 1.0)) * 100.0 * 10.0).round() / 10.0,
        TrackKind::Audio => (super::mixer::pos_to_db(n.clamp(0.0, 1.0)) * 100.0).round() / 100.0,
    }
}

/// Timeline time of a clip's media time `mt` (the inverse of `TrackItem::source_time_at`).
fn timeline_of(it: &TrackItem, mt: Tick) -> Tick {
    let speed = it.speed.abs().max(1e-9);
    let rel = if it.reverse { it.source_out() - Tick(1) - mt } else { mt - it.source_in };
    it.start + Tick((rel.0 as f64 / speed).round() as i64)
}

/// The band's area of a clip: below its name band.
fn band_rect(body: Rect) -> Rect {
    Rect::from_min_max(pos2(body.min.x, body.min.y + 17.0), pos2(body.max.x, body.max.y - 3.0))
}

/// The value a clip's band shows: its effect's parameter (the default when unset), None when the
/// effect is missing or off.
fn shown_param(it: &TrackItem, kind: TrackKind) -> Option<Param> {
    let (effect, pid) = target(kind);
    let e = it.effect(effect).filter(|e| e.enabled)?;
    Some(e.param(pid).cloned().unwrap_or_else(|| Param::new(ParamValue::Float(if kind == TrackKind::Video { 100.0 } else { 0.0 }))))
}

/// Whether `pos` is on `clip`'s Opacity line, within 4 px (video clips; the Timeline asks for
/// [`super::timeline_hit::Grab::Opacity`]). Not on a locked track or a track too low to show it.
pub fn on_line(seq: &Sequence, layout: &Layout, clip: ClipId, pos: Pos2) -> bool {
    let Some((tid, it)) = seq.find_item(clip) else { return false };
    let Some(r) = layout.rows.iter().find(|r| r.track == tid && !r.lane) else { return false };
    if r.kind != TrackKind::Video || r.rect.height() < 30.0 || seq.track(tid).is_none_or(|t| t.locked) {
        return false;
    }
    let (x0, x1) = (layout.x_of(it.start), layout.x_of(it.end()));
    if x1 - x0 < 6.0 || it.duration <= Tick::ZERO {
        return false;
    }
    let b = band_rect(Rect::from_min_max(pos2(x0, r.rect.min.y + 1.0), pos2(x1, r.rect.max.y - 1.0)));
    let vis = b.intersect(layout.content);
    if vis.height() < 6.0 || vis.width() < 2.0 || !vis.contains(pos) {
        return false;
    }
    let Some(p) = shown_param(it, TrackKind::Video) else { return false };
    let v = p.f64_at(it.source_time_at(layout.tick_at(pos.x).clamp(it.start, it.end() - Tick(1))));
    (pos.y - (b.max.y - norm(TrackKind::Video, v) * b.height())).abs() <= 4.0
}

/// A press on `clip`'s Opacity line at `at` that became a drag: the keyframes either side of the
/// grabbed point (or the static value) follow the pointer until the button comes up.
pub fn start_line_drag(ctx: &egui::Context, seq: &Sequence, layout: &Layout, clip: ClipId, at: Pos2) {
    let Some((_, it)) = seq.find_item(clip) else { return };
    let Some(base) = shown_param(it, TrackKind::Video) else { return };
    if it.duration <= Tick::ZERO {
        return;
    }
    let mt = it.source_time_at(layout.tick_at(at.x).clamp(it.start, it.end() - Tick(1)));
    let right = base.keyframes.iter().position(|k| k.time > mt);
    let left = match right {
        Some(0) => None,
        Some(r) => Some(r - 1),
        None => base.keyframes.len().checked_sub(1),
    };
    ctx.data_mut(|m| m.insert_temp(drag_id(), BandDrag::Line { clip, left, right, from_y: at.y }));
}

/// The Pen tool or a ⌘-click on `clip`'s Opacity line at `x`: a keyframe there.
pub fn add_keyframe(app: &mut FilmcraftApp, seq: &Sequence, layout: &Layout, clip: ClipId, x: f32) {
    let Some((_, it)) = seq.find_item(clip) else { return };
    if it.duration <= Tick::ZERO {
        return;
    }
    let (effect, pid) = target(TrackKind::Video);
    let t = layout.tick_at(x).clamp(it.start, it.end() - Tick(1));
    if let Err(e) = app.session.execute("effects.addKeyframe", json!({"clip": clip.0, "effect": effect, "param": pid, "time": t.0})) {
        app.ui.status = e.to_string();
    }
}

/// A drag on a band in progress.
#[derive(Clone, Copy, Debug)]
enum BandDrag {
    /// A keyframe (by index), grabbed this far from its centre.
    Key { clip: ClipId, index: usize, grab: egui::Vec2 },
    /// The line: the keyframes either side of the grabbed point (or the static value), from where
    /// the pointer went down.
    Line { clip: ClipId, left: Option<usize>, right: Option<usize>, from_y: f32 },
}

fn drag_id() -> egui::Id {
    egui::Id::new("tl-band-drag")
}

/// The parameter with a drag in progress applied: a keyframe moved to (`time`, `value`), or the
/// grabbed segment's keyframes (or the static value) moved by `dn` of the band's height.
fn previewed(p: &Param, kind: TrackKind, drag: BandDrag, key_to: Option<(Tick, f64)>, dn: f32) -> Param {
    let mut q = p.clone();
    match drag {
        BandDrag::Key { index, .. } => {
            if let (Some((t, v)), Some(k)) = (key_to, q.keyframes.get_mut(index)) {
                k.time = t;
                k.value = ParamValue::Float(v);
            }
            q.keyframes.sort_by_key(|k| k.time);
        }
        BandDrag::Line { left, right, .. } => {
            if q.keyframes.is_empty() {
                let v = q.value.as_f64().unwrap_or(0.0);
                q.value = ParamValue::Float(denorm(kind, norm(kind, v) + dn));
            }
            for i in [left, right].into_iter().flatten() {
                if let Some(k) = q.keyframes.get_mut(i) {
                    let v = k.value.as_f64().unwrap_or(0.0);
                    k.value = ParamValue::Float(denorm(kind, norm(kind, v) + dn));
                }
            }
        }
    }
    q
}

/// Draw the bands of the clips shown and handle their editing (after the timeline's own
/// interaction, so they sit on top of it).
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, seq: &Sequence, layout: &Layout, vclip: Rect, aclip: Rect) {
    // (not while the timeline drags something of its own)
    if app.tl.drag.as_ref().is_some_and(|d| !matches!(d, Drag::Scrub { .. })) {
        return;
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut drag: Option<BandDrag> = ui.data(|d| d.get_temp(drag_id()));
    let mut acts: Vec<(&str, Value)> = Vec::new();
    let visible = (layout.tick_at(layout.content.min.x - 2.0), layout.tick_at(layout.content.max.x + 2.0));
    for r in &layout.rows {
        let shown = match r.kind {
            TrackKind::Video => app.ui.timeline.show_video_keyframes,
            TrackKind::Audio => app.ui.timeline.show_audio_keyframes && !app.ui.timeline.track_lanes.contains_key(&r.track.0),
        };
        let Some(tr) = seq.track(r.track) else { continue };
        if !shown || r.rect.height() < 30.0 {
            continue;
        }
        let area = if r.kind == TrackKind::Video { vclip } else { aclip };
        let (effect, pid) = target(r.kind);
        for it in tr.items.iter().filter(|i| i.end() >= visible.0 && i.start <= visible.1) {
            let (x0, x1) = (layout.x_of(it.start), layout.x_of(it.end()));
            if x1 - x0 < 6.0 {
                continue;
            }
            let body = Rect::from_min_max(pos2(x0, r.rect.min.y + 1.0), pos2(x1, r.rect.max.y - 1.0));
            let b = band_rect(body);
            let vis = b.intersect(area).intersect(layout.content);
            if vis.height() < 6.0 || vis.width() < 2.0 {
                continue;
            }
            let Some(e) = it.effect(effect).filter(|e| e.enabled) else { continue };
            let base = e.param(pid).cloned().unwrap_or_else(|| Param::new(ParamValue::Float(if r.kind == TrackKind::Video { 100.0 } else { 0.0 })));
            let mine = drag.filter(|d| match d {
                BandDrag::Key { clip, .. } | BandDrag::Line { clip, .. } => *clip == it.id,
            });
            let y_of = |v: f64| b.max.y - norm(r.kind, v) * b.height();
            let n_of = |y: f32| (b.max.y - y) / b.height().max(1.0);
            // where a dragged keyframe is going: inside the clip, in the band
            let key_to = match (mine, pointer) {
                (Some(BandDrag::Key { grab, .. }), Some(pp)) => {
                    let c = pp - grab;
                    let t = layout.tick_at(c.x.clamp(x0, x1)).clamp(it.start, it.end() - Tick(1));
                    Some((it.source_time_at(t), denorm(r.kind, n_of(c.y.clamp(b.min.y, b.max.y)))))
                }
                _ => None,
            };
            let dn = match (mine, pointer) {
                (Some(BandDrag::Line { from_y, .. }), Some(pp)) => (from_y - pp.y) / b.height().max(1.0),
                _ => 0.0,
            };
            let p = match mine {
                Some(d) => previewed(&base, r.kind, d, key_to, dn),
                None => base.clone(),
            };
            let paint = ui.painter().with_clip_rect(vis.expand2(vec2(0.0, 5.0)).intersect(area));
            // the line, sampled across the visible part of the clip
            let line_at = |x: f32| y_of(p.f64_at(it.source_time_at(layout.tick_at(x).clamp(it.start, it.end() - Tick(1)))));
            let mut pts = Vec::new();
            let mut x = vis.min.x;
            while x < vis.max.x {
                pts.push(pos2(x, line_at(x)));
                x += 2.0;
            }
            pts.push(pos2(vis.max.x, line_at(vis.max.x)));
            paint.add(egui::Shape::line(pts.iter().map(|q| *q + vec2(0.0, 1.0)).collect(), Stroke::new(1.0, Color32::BLACK)));
            paint.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::WHITE)));
            app.auto.add(
                &format!("timeline.clip.{}.band", it.id.0),
                vis,
                &format!("{} keyframes", if r.kind == TrackKind::Video { "Opacity" } else { "Volume" }),
            );
            // keyframes
            let keys: Vec<(usize, Tick, f64)> = base.keyframes.iter().enumerate().map(|(i, k)| (i, k.time, k.value.as_f64().unwrap_or(0.0))).collect();
            for (i, mt, v) in keys {
                let (mt, v) = match (mine, key_to) {
                    (Some(BandDrag::Key { index, .. }), Some(to)) if index == i => to,
                    _ => (mt, v),
                };
                let tl = timeline_of(it, mt);
                if tl < it.start || tl > it.end() {
                    continue;
                }
                let v = match mine {
                    Some(BandDrag::Line { left, right, .. }) if left == Some(i) || right == Some(i) => denorm(r.kind, norm(r.kind, v) + dn),
                    _ => v,
                };
                let c = pos2(layout.x_of(tl), y_of(v));
                if c.x < vis.min.x - 6.0 || c.x > vis.max.x + 6.0 {
                    continue;
                }
                let hr = Rect::from_center_size(c, vec2(11.0, 11.0));
                let resp = ui.interact(hr, egui::Id::new(("band-kf", it.id.0, i)), Sense::click_and_drag());
                let hot = resp.hovered() || matches!(mine, Some(BandDrag::Key { index, .. }) if index == i);
                icons::paint(
                    &paint,
                    Rect::from_center_size(c, vec2(9.0, 9.0)),
                    Icon::Keyframe,
                    if hot { Color32::from_rgb(0x2d, 0x8c, 0xeb) } else { Color32::WHITE },
                );
                app.auto.add(&format!("timeline.clip.{}.band.kf.{i}", it.id.0), hr, &format!("keyframe {v:.1}"));
                if resp.drag_started()
                    && let Some(pp) = pointer
                {
                    let d = BandDrag::Key { clip: it.id, index: i, grab: pp - c };
                    drag = Some(d);
                    ui.data_mut(|m| m.insert_temp(drag_id(), d));
                }
                if resp.drag_stopped()
                    && let (Some(BandDrag::Key { index, .. }), Some((to_t, to_v))) = (mine, key_to)
                    && index == i
                {
                    let orig = base.keyframes.get(i).map_or(to_t, |k| k.time);
                    let mut params = json!({"clip": it.id.0, "effect": effect, "param": pid, "mediaTime": orig.0, "value": to_v});
                    if to_t != orig {
                        params["newTime"] = json!(to_t.0);
                    }
                    acts.push(("effects.setKeyframe", params));
                    ui.data_mut(|d| d.remove::<BandDrag>(drag_id()));
                }
                if hot && matches!(mine, Some(BandDrag::Key { .. })) {
                    let label = if r.kind == TrackKind::Video { format!("Opacity {v:.1} %") } else { format!("Volume {v:.2} dB") };
                    egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("band-kf-tip"), egui::PopupAnchor::Pointer).show(|ui| {
                        ui.label(label);
                    });
                }
                let at = base.keyframes.get(i).map_or(mt, |k| k.time);
                resp.context_menu(|ui| {
                    let d = ui.button("Delete");
                    app.auto.add(&format!("timeline.clip.{}.band.kf.{i}.delete", it.id.0), d.rect, "Delete");
                    if d.clicked() {
                        acts.push(("effects.deleteKeyframe", json!({"clip": it.id.0, "effect": effect, "param": pid, "mediaTime": at.0})));
                        ui.close();
                    }
                });
            }
            // the line being dragged (the Timeline started it, see `on_line`): it follows the
            // pointer, and the button coming up sets the value
            if let Some(BandDrag::Line { left, right, .. }) = mine {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
                if pointer.is_some() && ui.input(|i| i.pointer.any_released() || !i.pointer.any_down()) {
                    if base.keyframes.is_empty() {
                        let v = p.value.as_f64().unwrap_or(0.0);
                        acts.push(("effects.setParam", json!({"clip": it.id.0, "effect": effect, "param": pid, "value": v})));
                    }
                    for i in [left, right].into_iter().flatten() {
                        if let (Some(k), Some(moved)) = (base.keyframes.get(i), p.keyframes.get(i)) {
                            acts.push((
                                "effects.setKeyframe",
                                json!({"clip": it.id.0, "effect": effect, "param": pid, "mediaTime": k.time.0, "value": moved.value.as_f64().unwrap_or(0.0)}),
                            ));
                        }
                    }
                    ui.data_mut(|d| d.remove::<BandDrag>(drag_id()));
                } else {
                    let v = left_or_static(&p, mine).unwrap_or(0.0);
                    let label = if r.kind == TrackKind::Video { format!("Opacity {v:.1} %") } else { format!("Volume {v:.2} dB") };
                    egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("band-line-tip"), egui::PopupAnchor::Pointer).show(|ui| {
                        ui.label(label);
                    });
                }
            }
        }
    }
    if drag.is_some() && !ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.remove::<BandDrag>(drag_id()));
    }
    for (id, p) in acts {
        if let Err(e) = app.session.execute(id, p) {
            app.ui.status = e.to_string();
        }
    }
}

/// The value a line drag shows: the moved static value, or the left keyframe's.
fn left_or_static(p: &Param, d: Option<BandDrag>) -> Option<f64> {
    match d {
        Some(BandDrag::Line { left, right, .. }) => match left.or(right) {
            Some(i) => p.keyframes.get(i).and_then(|k| k.value.as_f64()),
            None => p.value.as_f64(),
        },
        _ => None,
    }
}
