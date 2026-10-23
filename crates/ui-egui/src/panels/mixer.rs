//! Audio Track Mixer and Audio Clip Mixer.
//!
//! Track Mixer strips (audio tracks, submixes, Mix): effect and send slots (behind the ▸
//! disclosure) with the knob of the selected slot's parameter under them (see [`rack`]: drag
//! an effect within a strip to move it, onto another strip to copy it, Ctrl-drag for the other
//! one, as in Premiere; double-click to edit), input mapping, output assignment, pan knob, automation mode, M / S / R, fader with
//! a dB scale, stereo meters fed by the playing mix, the fader value and the strip name. Every
//! control goes through `mixer.*` commands: fader and knob drags are `mixer.touch` while held and
//! `mixer.release` when let go, so they are heard live during playback and recorded as automation
//! in Latch / Touch / Write mode.
//!
//! Automation ids: `mixer.<A1|S1|Mix>.<fader|value|pan|panValue|mode|mute|solo|record|output|input|
//! fx.<slot>|send.<slot>|rack.knob|rack.value|rack.bypass|meter|name>` (effect slot menus:
//! `…fx.<slot>.<effect id|none|folder.<name>>`, right-click `…fx.<slot>.<edit|bypass|postFader|
//! knob.<param>|remove>`, `…fx.copy`, `…fx.paste`), popup entries below them (`mixer.A1.mode.Touch`…), the
//! effects disclosure `mixer.showEffects`, and the footer transport `mixer.transport.<command>`.
//! The Clip Mixer uses `clipMixer.A1.<fader|value|pan|mute|solo|keyframe|mode>`, and `clipMixer.scroll`
//! when its strips, narrowed to fit the panel, still do not fit.
//!
//! Strips feeding a 5.1 submix or Mix show the 5.1 panner instead of the pan knob: a puck in the
//! listening square (`mixer.A1.pan51`, drag; double-click: front centre) with Center % and LFE
//! controls (`mixer.A1.pan51.center`, `mixer.A1.pan51.lfe`). Meters show one bar per channel
//! (six for 5.1 strips). The panel menu (`mixer.menu`) holds Show/Hide Tracks (`mixer.menu.showHide`,
//! then `mixer.showHide.<A1|S1>` per strip) and Meter Input(s) Only (`mixer.menu.meterInputOnly`).

use std::collections::HashMap;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_engine::mixer::strip_label;
use filmcraft_project::mixer::{
    FADER_MAX_DB, FADER_MIN_DB, LANE_MUTE, LANE_PAN, LANE_PAN51_CENTER, LANE_PAN51_LFE, LANE_PAN51_X, LANE_PAN51_Y, LANE_VOLUME, MASTER_STRIP,
};
use filmcraft_project::{AutomationMode, InputMap, Sequence, Track, TrackId};
use filmcraft_time::{TimeDisplay, format_time};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

// ------------------------------------------------------------------------------------- fader law

/// Fader taper: (dB, position 0 … 1). Our own law: most of the travel around unity gain.
const TAPER: [(f64, f32); 10] =
    [(-96.0, 0.0), (-60.0, 0.05), (-40.0, 0.14), (-30.0, 0.24), (-20.0, 0.38), (-12.0, 0.52), (-6.0, 0.64), (0.0, 0.76), (6.0, 0.88), (15.0, 1.0)];
/// Labels on the fader scale.
const SCALE: [f64; 10] = [15.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -60.0, -96.0];
const STRIP_W: f32 = 98.0;
/// Narrowest Clip Mixer strip: the M / S / keyframe buttons side by side.
const CLIP_STRIP_MIN_W: f32 = 66.0;
const SLOT_H: f32 = 15.0;
const RED: Color32 = Color32::from_rgb(0xd8, 0x50, 0x3f);
const MUTE_ON: Color32 = Color32::from_rgb(0x2d, 0x9d, 0x78);
const SOLO_ON: Color32 = Color32::from_rgb(0xf0, 0xf0, 0x4f);

/// Fader position (0 bottom … 1 top) of a dB value.
pub fn db_to_pos(db: f64) -> f32 {
    if db <= TAPER[0].0 {
        return 0.0;
    }
    for w in TAPER.windows(2) {
        let ((d0, p0), (d1, p1)) = (w[0], w[1]);
        if db <= d1 {
            return p0 + (p1 - p0) * ((db - d0) / (d1 - d0)) as f32;
        }
    }
    1.0
}

/// dB value of a fader position.
pub fn pos_to_db(p: f32) -> f64 {
    let p = p.clamp(0.0, 1.0);
    for w in TAPER.windows(2) {
        let ((d0, p0), (d1, p1)) = (w[0], w[1]);
        if p <= p1 {
            let db = d0 + (d1 - d0) * ((p - p0) / (p1 - p0)) as f64;
            return if db <= FADER_MIN_DB + 0.5 { FADER_MIN_DB } else { db };
        }
    }
    FADER_MAX_DB
}

fn db_text(db: f64) -> String {
    if db <= FADER_MIN_DB { "-∞".into() } else { format!("{db:.1}") }
}

// ------------------------------------------------------------------------------------- meters

/// Meter key of the recording input (Meter Input(s) Only).
pub const INPUT_METER: u64 = u64::MAX - 1;

/// Per-strip meter state, `strip id → [level, peak]` per channel (dBFS; 2 channels, or 6 for 5.1
/// strips), fed by the playing mix ([`filmcraft_render::mixer::LiveMix`]) with fast attack and a
/// 20 dB/s fall. [`INPUT_METER`] holds the voice-over input.
pub fn poll_meters(app: &mut FilmcraftApp, ui: &egui::Ui) -> HashMap<u64, Vec<[f32; 2]>> {
    let id = egui::Id::new("mixer-meters");
    let frame = ui.ctx().cumulative_frame_nr();
    let (last, mut st): (u64, HashMap<u64, Vec<[f32; 2]>>) = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    if last == frame && frame != 0 {
        return st;
    }
    // without an audio device, the loudness feed is what mixes the programme (and posts meters)
    super::meters::feed_loudness(app);
    let mut fresh: Vec<(u64, Vec<f32>)> = app.session.previews.live.take_meters().into_iter().map(|(k, v)| (k.0, v)).collect();
    let input = app.session.previews.live.take_input_meter();
    if !input.is_empty() {
        fresh.push((INPUT_METER, input));
    }
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    for v in st.values_mut() {
        for m in v.iter_mut() {
            m[0] = (m[0] - 20.0 * dt).max(-90.0);
            m[1] = (m[1] - 6.0 * dt).max(-90.0);
        }
    }
    // scrubbing plays through the same mixer, so its grains move the meters too
    let live = app.playback.playing || app.session.voiceover.recording() || app.scrub.is_open();
    if live {
        for (k, p) in fresh {
            let e = st.entry(k).or_default();
            if e.len() != p.len() {
                *e = vec![[-90.0; 2]; p.len()];
            }
            for (m, x) in e.iter_mut().zip(&p) {
                let l = 20.0 * x.max(1e-6).log10();
                m[0] = m[0].max(l);
                m[1] = m[1].max(l);
            }
        }
    }
    // After Stop, levels and peak holds still fall. Keep animating without pointer input until
    // both reach the floor, then let the idle window sleep (#1062).
    if live || st.values().flatten().any(|meter| meter.iter().any(|db| db.is_finite() && *db > -90.0)) {
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(id, (frame, st.clone())));
    st
}

/// One bar per channel (level, peak hold) with a clip light on top; 5.1 meters carry channel names.
fn draw_meters(ui: &egui::Ui, r: Rect, m: &[[f32; 2]], channels: usize, t: &Tokens) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, t.meter_bg);
    let n = channels.max(1);
    let gap = if n > 2 { 1.0 } else { 2.0 };
    let w = (r.width() - gap * (n as f32 - 1.0)) / n as f32;
    for c in 0..n {
        let v = m.get(c).copied().unwrap_or([-90.0; 2]);
        let br = Rect::from_min_size(pos2(r.min.x + c as f32 * (w + gap), r.min.y + 6.0), vec2(w, r.height() - 6.0));
        crate::widgets::meter_bar(p, br, v[0], v[1], t);
        // clip light
        let lr = Rect::from_min_size(pos2(br.min.x, r.min.y), vec2(w, 4.0));
        p.rect_filled(lr, 0.0, if v[1] >= -0.1 { RED } else { t.meter_clip_off });
    }
}

/// Meter levels for the automation label, one per channel (silent channels read -90.0).
fn meter_text(m: &[[f32; 2]], channels: usize) -> String {
    (0..channels).map(|c| format!("{:.1}", m.get(c).map_or(-90.0, |v| v[0]))).collect::<Vec<_>>().join(" / ")
}

// ------------------------------------------------------------------------------------- helpers

struct Ctx<'a> {
    t: Tokens,
    seq: &'a Sequence,
    now: filmcraft_time::Tick,
    recording: bool,
    acts: Vec<(String, Value)>,
    /// The strip whose rack slot a dragged effect is over (for the copy / move cue).
    drop_strip: Option<u64>,
}

impl Ctx<'_> {
    fn cmd(&mut self, id: &str, p: Value) {
        self.acts.push((id.to_string(), p));
    }
}

/// The value a control shows: a held override, else the automation (or static value) at the playhead.
fn shown(app: &FilmcraftApp, tr: &Track, lane: &str, now: filmcraft_time::Tick) -> f64 {
    app.session.previews.live.get(tr.id, lane).map(|o| o.value).unwrap_or_else(|| tr.lane_value(lane, now))
}

fn small_text(ui: &egui::Ui, pos: egui::Pos2, align: Align2, s: &str, size: f32, c: Color32) {
    ui.painter().text(pos, align, s, Tokens::ui(size), c);
}

/// Width of the dropdown arrow at the right of a rack slot: clicking there opens the effect (or
/// send) list; clicking the rest of a filled slot selects it.
const ARROW_W: f32 = 14.0;
/// Height of the parameter knob area under the rack.
const KNOB_AREA_H: f32 = 78.0;

/// An effect being dragged out of a rack slot. As in Premiere: dropped in its own strip it moves,
/// on another strip it is copied; Ctrl (Cmd) swaps the two. It lands at the slot it is dropped
/// on, pushing that slot's effect and the ones below down.
#[derive(Clone, Debug)]
struct RackDrag {
    strip: u64,
    slot: usize,
    name: String,
}

/// Whether dropping a dragged effect copies it (else it moves): Premiere moves within a strip
/// and copies to another one, and Ctrl (Cmd) swaps the two. Alt does nothing (and on Linux
/// desktops Alt-drag moves the window).
fn drop_copies(same_strip: bool, ctrl: bool) -> bool {
    same_strip == ctrl
}

/// Paint a rack slot: its text, the dropdown arrow, the selection and a drop marker.
#[allow(clippy::too_many_arguments)]
fn paint_slot(ui: &egui::Ui, r: Rect, text: &str, filled: bool, dim: bool, selected: bool, hovered: bool, drop: bool, t: &Tokens) {
    let bg = if selected {
        t.accent.gamma_multiply(0.35)
    } else if hovered {
        t.hover
    } else {
        t.field_bg
    };
    ui.painter().rect_filled(r, 2.0, bg);
    let border = if selected { t.accent } else { t.field_border };
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let col = if !filled || dim { t.text_faint } else { t.text };
    let galley = ui.painter().layout_no_wrap(text.to_string(), Tokens::ui(10.0), col);
    let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x - ARROW_W, r.max.y)).shrink(1.0).intersect(ui.clip_rect()));
    clip.galley(pos2(r.min.x + 4.0, r.center().y - galley.size().y / 2.0), galley, col);
    if dim && filled {
        // bypassed: struck through, as the effect is not heard
        let y = r.center().y;
        clip.line_segment([pos2(r.min.x + 3.0, y), pos2(r.max.x - ARROW_W - 2.0, y)], Stroke::new(1.0, t.text_faint));
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 7.0, r.center().y), vec2(7.0, 7.0)), Icon::ChevronDown, t.text_dim);
    if drop {
        // where a dragged effect lands: at this slot, the ones below move down
        ui.painter().line_segment([pos2(r.min.x, r.min.y - 1.0), pos2(r.max.x, r.min.y - 1.0)], Stroke::new(2.0, t.accent));
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
}

/// The parameter a rack knob shows for `e`: the one chosen in the slot's menu, else the effect's
/// first adjustable number.
fn knob_param(app: &FilmcraftApp, strip: TrackId, slot: usize, e: &filmcraft_project::EffectInstance) -> Option<&'static filmcraft_project::ParamDef> {
    let def = e.def()?;
    let numeric = |p: &&filmcraft_project::ParamDef| matches!(p.kind, filmcraft_project::ParamKind::Float { .. });
    let chosen = app.ui.mixer_knob_param.get(&knob_key(strip, slot, &e.effect)).and_then(|id| def.params.iter().filter(numeric).find(|p| p.id == id));
    chosen.or_else(|| def.params.iter().find(|p| numeric(p) && p.animatable)).or_else(|| def.params.iter().find(numeric))
}

/// Key of a rack knob's parameter choice: strip, slot and the effect there.
fn knob_key(strip: TrackId, slot: usize, effect: &str) -> String {
    format!("{}/{slot}/{effect}", strip.0)
}

/// The DSP-backed audio effects that can go in an insert slot.
fn insert_effects() -> Vec<&'static filmcraft_project::EffectDef> {
    filmcraft_project::effect_defs()
        .iter()
        .filter(|d| d.kind == filmcraft_project::EffectKind::Audio && !d.intrinsic && filmcraft_render::audio_fx::supported(d.id))
        .collect()
}

/// Submixes a strip may route to (all of them for tracks; only later ones for submixes).
fn targets(seq: &Sequence, id: TrackId) -> Vec<&Track> {
    let me = seq.submix_tracks.iter().position(|t| t.id == id);
    seq.submix_tracks.iter().enumerate().filter(|(i, _)| me.is_none_or(|m| *i > m)).map(|(_, t)| t).collect()
}

// ------------------------------------------------------------------------------------- track mixer

pub fn track_mixer(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    // Keep the immutable project snapshot alive while UI commands mutate the session.
    // Cloning a sequence here copies every clip and effect on every redraw.
    let project = app.session.project.clone();
    let Some(seq) = app.session.state.active_sequence.and_then(|id| project.sequence(id)) else {
        crate::dock::placeholder(ui, rect, &t, tl!("(no sequence)"));
        return;
    };
    let meters = poll_meters(app, ui);
    let mut cx = Ctx { t, seq, now: app.session.playhead(), recording: app.session.mixrec.active(), acts: Vec::new(), drop_strip: None };
    let footer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - 34.0), rect.max);
    let body = Rect::from_min_max(pos2(rect.min.x, rect.min.y + 2.0), pos2(rect.max.x, footer.min.y));
    // effects / sends disclosure at the left edge
    let arrow = Rect::from_min_size(pos2(body.min.x + 2.0, body.min.y + 4.0), vec2(14.0, 18.0));
    let aresp = ui.interact(arrow, egui::Id::new("mixer-fx-toggle"), Sense::click()).on_hover_text(tl!("Show/Hide Effects and Sends"));
    icons::paint(ui.painter(), arrow.shrink(2.0), if app.ui.mixer_fx_open { Icon::ChevronDown } else { Icon::ChevronRight }, t.icon);
    app.auto.add("mixer.showEffects", arrow, "Show/Hide Effects and Sends");
    if aresp.clicked() {
        app.ui.mixer_fx_open = !app.ui.mixer_fx_open;
    }
    // panel menu: Show/Hide Tracks, Meter Input(s) Only
    let menu_r = Rect::from_min_size(pos2(body.min.x + 2.0, body.min.y + 26.0), vec2(14.0, 18.0));
    let mresp = ui.interact(menu_r, egui::Id::new("mixer-menu"), Sense::click()).on_hover_text(tl!("Audio Track Mixer menu"));
    icons::paint(ui.painter(), menu_r.shrink(2.0), Icon::Hamburger, if mresp.hovered() { t.tab_text_active } else { t.icon });
    app.auto.add("mixer.menu", menu_r, "Audio Track Mixer menu");
    let show_hide_id = egui::Id::new("mixer-show-hide-open");
    let mut show_hide: bool = ui.data(|d| d.get_temp(show_hide_id)).unwrap_or(false);
    egui::Popup::menu(&mresp).show(|ui| {
        let r = ui.button(tl!("Show/Hide Tracks…"));
        app.auto.add("mixer.menu.showHide", r.rect, "Show/Hide Tracks…");
        if r.clicked() {
            show_hide = true;
        }
        let on = app.ui.mixer_meter_input_only;
        let r = ui.selectable_label(on, tl!("Meter Input(s) Only"));
        app.auto.add("mixer.menu.meterInputOnly", r.rect, "Meter Input(s) Only");
        if r.clicked() {
            app.ui.mixer_meter_input_only = !on;
        }
    });
    if show_hide {
        let mut open = true;
        let mut close = false;
        egui::Window::new(tl!("Show/Hide Tracks")).id(egui::Id::new("mixer-show-hide")).collapsible(false).resizable(false).open(&mut open).show(
            ui.ctx(),
            |ui| {
                for id in seq.audio_tracks.iter().chain(&seq.submix_tracks).map(|t| t.id) {
                    let label = strip_label(seq, id);
                    let name = seq.mix_track(id).map(|t| t.name.clone()).unwrap_or_default();
                    let mut shown = !app.ui.mixer_hidden.contains(&id.0);
                    let r = ui.checkbox(&mut shown, format!("{label}  {name}"));
                    app.auto.add(&format!("mixer.showHide.{label}"), r.rect, &format!("Show {label}"));
                    if r.changed() {
                        if shown {
                            app.ui.mixer_hidden.retain(|h| *h != id.0);
                        } else if !app.ui.mixer_hidden.contains(&id.0) {
                            app.ui.mixer_hidden.push(id.0);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    let r = ui.button(tl!("Show All"));
                    app.auto.add("mixer.showHide.all", r.rect, "Show All");
                    if r.clicked() {
                        app.ui.mixer_hidden.clear();
                    }
                    let r = ui.button(tl!("OK"));
                    app.auto.add("mixer.showHide.ok", r.rect, "OK");
                    if r.clicked() {
                        close = true;
                    }
                });
            },
        );
        show_hide = open && !close;
    }
    ui.data_mut(|d| d.insert_temp(show_hide_id, show_hide));
    let strips: Vec<TrackId> = seq.strip_ids().into_iter().filter(|id| !app.ui.mixer_hidden.contains(&id.0)).collect();
    let x0 = body.min.x + 18.0;
    let total = strips.len() as f32 * (STRIP_W + 2.0);
    let avail = body.max.x - x0 - 2.0;
    let (off, sb_h) = strip_scroll(app, ui, body, x0, total, avail, "mixer");
    // strips never get shorter than their controls plus a usable fader: scroll vertically instead
    let need = 4.0 + if app.ui.mixer_fx_open { 10.0 * SLOT_H + 6.0 + KNOB_AREA_H } else { 0.0 } + 112.0 + 26.0 + 24.0 + 6.0 + 170.0 + 42.0;
    let clip = Rect::from_min_max(pos2(x0, body.min.y), pos2(body.max.x - 8.0, body.max.y - sb_h));
    let voff_id = egui::Id::new("mixer-vscroll");
    let max_voff = (need - (clip.height() - 4.0)).max(0.0);
    let mut voff: f32 = ui.data(|d| d.get_temp(voff_id)).unwrap_or(0.0);
    if ui.rect_contains_pointer(body) && !ui.input(|i| i.modifiers.shift) {
        voff -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    voff = voff.clamp(0.0, max_voff);
    if max_voff > 0.0 {
        let bar = Rect::from_min_max(pos2(body.max.x - 7.0, clip.min.y + 2.0), pos2(body.max.x - 2.0, clip.max.y - 2.0));
        ui.painter().rect_filled(bar, 3.0, t.field_bg);
        let kh = (bar.height() * clip.height() / (need + 4.0)).max(20.0);
        let ky = bar.min.y + (bar.height() - kh) * voff / max_voff;
        let knob = Rect::from_min_size(pos2(bar.min.x, ky), vec2(bar.width(), kh));
        let kr = ui.interact(knob, egui::Id::new("mixer-vscroll-knob"), Sense::drag());
        ui.painter().rect_filled(knob, 3.0, if kr.dragged() { t.accent } else { t.fader_knob });
        if kr.dragged() {
            voff = (voff + kr.drag_delta().y * (need + 4.0) / bar.height()).clamp(0.0, max_voff);
        }
        app.auto.add("mixer.vscroll", knob, "Scroll strips vertically");
    }
    ui.data_mut(|d| d.insert_temp(voff_id, voff));
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(clip));
    cui.set_clip_rect(clip.intersect(ui.clip_rect()));
    for (i, id) in strips.iter().enumerate() {
        let sx = x0 + i as f32 * (STRIP_W + 2.0) - off;
        if sx + STRIP_W < clip.min.x || sx > clip.max.x {
            continue;
        }
        let sr = Rect::from_min_size(pos2(sx, clip.min.y + 2.0 - voff), vec2(STRIP_W, (clip.height() - 4.0).max(need)));
        let is_track = seq.audio_tracks.iter().any(|t| t.id == *id && t.mixer.record_arm);
        let m = if app.ui.mixer_meter_input_only {
            // Meter Input(s) Only: armed tracks show the recording input; other meters are idle
            if is_track { meters.get(&INPUT_METER).cloned().unwrap_or_default() } else { Vec::new() }
        } else {
            meters.get(&id.0).cloned().unwrap_or_default()
        };
        strip(app, &mut cui, &mut cx, *id, sr, &m);
    }
    // the effect being dragged follows the pointer ("+" where the drop makes a copy)
    if let (Some(d), Some(p)) = (egui::DragAndDrop::payload::<RackDrag>(ui.ctx()), ui.ctx().pointer_interact_pos()) {
        let ctrl = ui.input(|i| i.modifiers.command);
        let copy = cx.drop_strip.is_some_and(|s| drop_copies(s == d.strip, ctrl));
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("mixer-rack-drag")));
        let text = if copy { format!("+ {}", d.name) } else { d.name.clone() };
        let galley = painter.layout_no_wrap(text, Tokens::ui(11.0), t.text);
        let r = Rect::from_min_size(p + vec2(12.0, 8.0), galley.size() + vec2(10.0, 6.0));
        painter.rect_filled(r, 3.0, Color32::from_black_alpha(210));
        painter.rect_stroke(r, 3.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        painter.galley(r.min + vec2(5.0, 3.0), galley, t.text);
        ui.ctx().set_cursor_icon(if copy { egui::CursorIcon::Copy } else { egui::CursorIcon::Grabbing });
    }
    transport(app, ui, footer, &mut cx);
    let acts = std::mem::take(&mut cx.acts);
    run_actions(app, &ui.ctx().clone(), acts);
}

/// Horizontal scrolling of a row of strips `total` wide shown in `avail` from `x0`: the wheel (and
/// Shift + wheel) over `body`, and a knob along its bottom edge (`<key>.scroll`). Returns the offset
/// and the height the scrollbar takes (0 when everything fits).
fn strip_scroll(app: &mut FilmcraftApp, ui: &egui::Ui, body: Rect, x0: f32, total: f32, avail: f32, key: &str) -> (f32, f32) {
    let t = app.tokens;
    let max_off = (total - avail).max(0.0);
    let off_id = egui::Id::new(format!("{key}-hscroll"));
    let mut off: f32 = ui.data(|d| d.get_temp(off_id)).unwrap_or(0.0);
    if ui.rect_contains_pointer(body) {
        let sd = ui.input(|i| i.smooth_scroll_delta);
        off -= sd.x + if ui.input(|i| i.modifiers.shift) { sd.y } else { 0.0 };
    }
    off = off.clamp(0.0, max_off);
    if max_off > 0.0 {
        let bar = Rect::from_min_max(pos2(x0, body.max.y - 10.0), pos2(body.max.x - 4.0, body.max.y - 2.0));
        ui.painter().rect_filled(bar, 4.0, t.field_bg);
        let kw = (bar.width() * avail / total).max(20.0);
        let kx = bar.min.x + (bar.width() - kw) * off / max_off;
        let knob = Rect::from_min_size(pos2(kx, bar.min.y), vec2(kw, bar.height()));
        let kr = ui.interact(knob, egui::Id::new(format!("{key}-hscroll-knob")), Sense::drag());
        ui.painter().rect_filled(knob, 4.0, if kr.dragged() { t.accent } else { t.fader_knob });
        if kr.dragged() {
            off = (off + kr.drag_delta().x * total / bar.width()).clamp(0.0, max_off);
        }
        app.auto.add(&format!("{key}.scroll"), knob, "Scroll strips");
    }
    ui.data_mut(|d| d.insert_temp(off_id, off));
    (off, if max_off > 0.0 { 12.0 } else { 0.0 })
}

fn run_actions(app: &mut FilmcraftApp, ctx: &egui::Context, acts: Vec<(String, Value)>) {
    for (id, p) in acts {
        let r = if filmcraft_engine::find_command(&id).is_some() {
            app.session.execute(&id, p).map(|_| ()).map_err(|e| e.to_string())
        } else {
            crate::menus::invoke(app, ctx, &id, p).map(|_| ())
        };
        if let Err(e) = r {
            app.ui.status = e;
        }
    }
}

fn strip(app: &mut FilmcraftApp, ui: &mut egui::Ui, cx: &mut Ctx, id: TrackId, sr: Rect, meter: &[[f32; 2]]) {
    let t = cx.t;
    let seq = cx.seq;
    let Some(tr) = seq.strip(id).map(|c| c.into_owned()) else { return };
    let label = strip_label(seq, id);
    let ap = format!("mixer.{label}");
    let is_master = id == MASTER_STRIP;
    let sid = json!(id.0);
    ui.painter().rect_filled(sr, 3.0, t.control_surface);
    ui.painter().line_segment([pos2(sr.max.x + 1.0, sr.min.y), pos2(sr.max.x + 1.0, sr.max.y)], Stroke::new(1.0, t.separator));
    let x = sr.min.x;
    let w = sr.width();
    let mut y = sr.min.y + 4.0;

    // ---- effect and send slots, and the parameter knob of the selected one
    if app.ui.mixer_fx_open {
        y = rack(app, ui, cx, &tr, id, &ap, x, w, y);
    }

    // ---- input / output
    if !is_master {
        let ir = Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, 20.0));
        let iresp = crate::widgets::dropdown_text(ui, ir, crate::i18n::t(tr.mixer.input_map.label()), &t, egui::Id::new((&ap, "input")));
        app.auto.add(&format!("{ap}.input"), ir, "Input channel mapping");
        egui::Popup::menu(&iresp).show(|ui| {
            for m in InputMap::ALL {
                let r = ui.selectable_label(m == tr.mixer.input_map, crate::i18n::t(m.label()));
                app.auto.add(&format!("{ap}.input.{}", m.label()), r.rect, m.label());
                if r.clicked() {
                    cx.cmd("mixer.setStrip", json!({"strip": sid, "inputMap": m.label()}));
                }
            }
        });
        y += 24.0;
        let or = Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, 20.0));
        let out_label =
            tr.mixer.output.map(|o| seq.mix_track(o).map(|s| s.name.clone()).unwrap_or_else(|| tl!("Mix").into())).unwrap_or_else(|| tl!("Mix").into());
        let oresp = crate::widgets::dropdown_text(ui, or, &out_label, &t, egui::Id::new((&ap, "output")));
        app.auto.add(&format!("{ap}.output"), or, "Track output assignment");
        let tg = targets(seq, id);
        egui::Popup::menu(&oresp).show(|ui| {
            let r = ui.selectable_label(tr.mixer.output.is_none(), tl!("Mix"));
            app.auto.add(&format!("{ap}.output.Mix"), r.rect, "Mix");
            if r.clicked() {
                cx.cmd("mixer.setStrip", json!({"strip": sid, "output": "Mix"}));
            }
            for s in &tg {
                let r = ui.selectable_label(tr.mixer.output == Some(s.id), &s.name);
                app.auto.add(&format!("{ap}.output.{}", strip_label(seq, s.id)), r.rect, &s.name);
                if r.clicked() {
                    cx.cmd("mixer.setStrip", json!({"strip": sid, "output": s.id.0}));
                }
            }
        });
        y += 26.0;
        if seq.pans_51(id) {
            pan51_panner(app, ui, cx, &tr, &ap, &sid, Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, 62.0)));
            y += 62.0;
        } else {
            // ---- pan knob
            let pan = shown(app, &tr, LANE_PAN, cx.now);
            let kc = pos2(sr.center().x, y + 20.0);
            let kr = Rect::from_center_size(kc, vec2(38.0, 38.0));
            let kresp = ui.interact(kr, egui::Id::new((&ap, "pan")), Sense::click_and_drag()).on_hover_text(tl!("Pan / balance (drag; double-click: centre)"));
            let ring = if kresp.hovered() || kresp.dragged() { t.tab_text_active } else { t.control_handle_dim };
            ui.painter().circle_stroke(kc, 16.0, Stroke::new(2.0, ring));
            let ang = (pan / 100.0) as f32 * 135f32.to_radians();
            ui.painter().line_segment([kc, kc + vec2(ang.sin(), -ang.cos()) * 14.0], Stroke::new(2.0, ring));
            small_text(ui, pos2(kc.x - 22.0, kc.y + 17.0), Align2::CENTER_CENTER, "L", 11.0, t.hot_text);
            small_text(ui, pos2(kc.x + 22.0, kc.y + 17.0), Align2::CENTER_CENTER, "R", 11.0, t.hot_text);
            app.auto.add(&format!("{ap}.pan"), kr, "Pan");
            if kresp.dragged() {
                let d = kresp.drag_delta();
                let np = (pan + (d.x - d.y) as f64).clamp(-100.0, 100.0);
                if np != pan {
                    cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_PAN, "value": np, "time": cx.now.0}));
                }
            }
            if kresp.drag_stopped() {
                cx.cmd("mixer.release", json!({"strip": sid, "lane": LANE_PAN, "time": cx.now.0}));
            }
            if kresp.double_clicked() {
                cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN, "value": 0.0}));
            }
            let vr = Rect::from_center_size(pos2(kc.x, y + 48.0), vec2(56.0, 16.0));
            let mut vui = ui.new_child(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::top_down(egui::Align::Center)));
            let (vresp, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new((&ap, "panValue")), pan, 1.0, (-100.0, 100.0), 1, "", &t);
            app.auto.add(&format!("{ap}.panValue"), vresp.rect, &format!("{pan:.1}"));
            if let Some(v) = nv {
                cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN, "value": v}));
            }
            y += 62.0;
        }
    } else {
        y += 112.0;
    }

    // ---- automation mode
    let mr = Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, 20.0));
    let mode = tr.mixer.mode;
    let mresp = crate::widgets::dropdown_text(ui, mr, crate::i18n::t(mode.label()), &t, egui::Id::new((&ap, "mode")));
    if cx.recording && mode.writes() {
        ui.painter().rect_stroke(mr, 4.0, Stroke::new(1.0, RED), StrokeKind::Inside);
    }
    app.auto.add(&format!("{ap}.mode"), mr, &format!("Automation mode: {}", mode.label()));
    egui::Popup::menu(&mresp).show(|ui| {
        for m in AutomationMode::ALL {
            let r = ui.selectable_label(m == mode, crate::i18n::t(m.label()));
            app.auto.add(&format!("{ap}.mode.{}", m.label()), r.rect, m.label());
            if r.clicked() {
                cx.cmd("mixer.setStrip", json!({"strip": sid, "mode": m.label()}));
            }
        }
    });
    y += 26.0;

    // ---- M / S / R
    if !is_master {
        let muted = shown(app, &tr, LANE_MUTE, cx.now) >= 0.5;
        let bw = 18.0;
        let bx = sr.center().x - bw * 1.5 - 6.0;
        let b = |i: f32| Rect::from_min_size(pos2(bx + i * (bw + 6.0), y), vec2(bw, 16.0));
        let m_r = crate::widgets::letter_toggle(ui, b(0.0), "M", muted, MUTE_ON, &t, egui::Id::new((&ap, "M")));
        app.auto.add(&format!("{ap}.mute"), b(0.0), "Mute Track");
        if m_r.clicked() {
            if cx.recording && mode.writes() {
                let v = if muted { 0.0 } else { 1.0 };
                cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_MUTE, "value": v, "time": cx.now.0}));
                cx.cmd("mixer.release", json!({"strip": sid, "lane": LANE_MUTE, "time": cx.now.0}));
            } else {
                cx.cmd("mixer.setStrip", json!({"strip": sid, "muted": !tr.muted}));
            }
        }
        let s_r = crate::widgets::letter_toggle(ui, b(1.0), "S", tr.solo, SOLO_ON, &t, egui::Id::new((&ap, "S")));
        app.auto.add(&format!("{ap}.solo"), b(1.0), "Solo Track");
        if s_r.clicked() {
            cx.cmd("mixer.setStrip", json!({"strip": sid, "solo": !tr.solo}));
        }
        let is_sub = seq.submix_tracks.iter().any(|s| s.id == id);
        if is_sub {
            let r_r = crate::widgets::letter_toggle(ui, b(2.0), "◆", tr.mixer.solo_safe, Color32::from_gray(0xc8), &t, egui::Id::new((&ap, "safe")))
                .on_hover_text(tl!("Solo safe"));
            app.auto.add(&format!("{ap}.soloSafe"), b(2.0), "Solo Safe");
            if r_r.clicked() {
                cx.cmd("mixer.setStrip", json!({"strip": sid, "soloSafe": !tr.mixer.solo_safe}));
            }
        } else {
            let r_r = crate::widgets::letter_toggle(ui, b(2.0), "R", tr.mixer.record_arm, RED, &t, egui::Id::new((&ap, "R")))
                .on_hover_text(tl!("Enable track for recording"));
            app.auto.add(&format!("{ap}.record"), b(2.0), "Enable Track for Recording");
            if r_r.clicked() {
                cx.cmd("mixer.setStrip", json!({"strip": sid, "recordArm": !tr.mixer.record_arm}));
            }
        }
    }
    y += 24.0;

    // ---- fader + meters
    let name_h = 20.0;
    let value_h = 18.0;
    let area = Rect::from_min_max(pos2(x, y + 6.0), pos2(sr.max.x, sr.max.y - name_h - value_h - 4.0));
    if area.height() > 40.0 {
        let track = Rect::from_min_max(pos2(x + 33.0, area.min.y + 8.0), pos2(x + 37.0, area.max.y - 8.0));
        let ypos = |db: f64| track.max.y - db_to_pos(db) * track.height();
        let tall = track.height() > 150.0;
        // scale
        small_text(ui, pos2(x + 26.0, area.min.y - 2.0), Align2::RIGHT_CENTER, "dB", 9.0, t.text_dim);
        for d in SCALE.iter().copied().filter(|d| tall || [15.0, 0.0, -12.0, -30.0, -96.0].contains(d)) {
            let yy = ypos(d);
            let s = if d <= FADER_MIN_DB { "-∞".to_string() } else { format!("{}", d as i32) };
            small_text(ui, pos2(x + 26.0, yy), Align2::RIGHT_CENTER, &s, 9.0, t.text_dim);
            ui.painter().line_segment([pos2(x + 28.0, yy), pos2(x + 31.0, yy)], Stroke::new(1.0, t.text_faint));
        }
        ui.painter().rect_filled(track, 2.0, t.control_well);
        ui.painter().rect_stroke(track, 2.0, Stroke::new(1.0, t.control_border), StrokeKind::Inside);
        let vol = shown(app, &tr, LANE_VOLUME, cx.now);
        let cy = ypos(vol);
        let cap = Rect::from_center_size(pos2(track.center().x, cy), vec2(18.0, 26.0));
        let fresp =
            ui.interact(cap.expand(3.0), egui::Id::new((&ap, "fader")), Sense::click_and_drag()).on_hover_text(tl!("Volume (drag; double-click: 0 dB)"));
        let held = app.session.previews.live.get(id, LANE_VOLUME).is_some();
        let col = if fresp.dragged() || held { t.accent } else { t.control_handle };
        ui.painter().rect_filled(cap, 3.0, t.control_handle_bg);
        ui.painter().rect_stroke(cap, 3.0, Stroke::new(2.0, col), StrokeKind::Inside);
        ui.painter().line_segment([pos2(cap.min.x + 5.0, cy), pos2(cap.max.x - 5.0, cy)], Stroke::new(2.0, col));
        app.auto.add(&format!("{ap}.fader"), cap, &format!("Volume {}", db_text(vol)));
        if fresp.dragged() {
            let dy = fresp.drag_delta().y;
            if dy != 0.0 {
                let fine = if ui.input(|i| i.modifiers.command) { 0.1 } else { 1.0 };
                let nv = pos_to_db(db_to_pos(vol) - dy * fine / track.height());
                cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_VOLUME, "value": nv, "time": cx.now.0}));
            } else if fresp.drag_started() {
                cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_VOLUME, "value": vol, "time": cx.now.0}));
            }
        }
        if fresp.drag_stopped() {
            cx.cmd("mixer.release", json!({"strip": sid, "lane": LANE_VOLUME, "time": cx.now.0}));
        }
        if fresp.double_clicked() {
            cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_VOLUME, "value": 0.0}));
        }
        // meters with their own scale (0 … −54 dB)
        let nch = filmcraft_render::mixer::width_of(tr.channels);
        let mrect = Rect::from_min_max(pos2(x + 46.0, area.min.y), pos2(x + 70.0, area.max.y));
        let nshow = if meter.is_empty() { nch } else { meter.len() };
        draw_meters(ui, mrect, meter, nshow, &t);
        app.auto.add(&format!("{ap}.meter"), mrect, &format!("Meter {} dB", meter_text(meter, nshow)));
        let bar_top = mrect.min.y + 6.0;
        let bar_h = mrect.height() - 6.0;
        let mut d = 0;
        let step = if tall { 6 } else { 18 };
        while d >= -54 {
            let yy = bar_top + bar_h * (-d as f32 / 60.0);
            small_text(ui, pos2(x + 91.0, yy), Align2::RIGHT_CENTER, &format!("{d}"), 8.5, t.text_dim);
            ui.painter().line_segment([pos2(x + 71.0, yy), pos2(x + 74.0, yy)], Stroke::new(1.0, t.text_faint));
            d -= step;
        }
        small_text(ui, pos2(x + 91.0, mrect.max.y + 6.0), Align2::RIGHT_CENTER, "dB", 8.5, t.text_dim);
        // fader value (hot text)
        let vr = Rect::from_min_size(pos2(x + 4.0, sr.max.y - name_h - value_h), vec2(56.0, value_h));
        let mut vui = ui.new_child(egui::UiBuilder::new().max_rect(vr));
        let (vresp, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new((&ap, "value")), vol.max(-96.0), 0.1, (FADER_MIN_DB, FADER_MAX_DB), 1, "", &t);
        app.auto.add(&format!("{ap}.value"), vresp.rect, &db_text(vol));
        if let Some(v) = nv {
            cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_VOLUME, "value": v}));
        }
    }
    // ---- name
    let nr = Rect::from_min_max(pos2(x, sr.max.y - name_h), sr.max);
    ui.painter().line_segment([pos2(nr.min.x + 4.0, nr.min.y), pos2(nr.max.x - 4.0, nr.min.y)], Stroke::new(1.0, t.separator));
    if is_master {
        small_text(ui, nr.center(), Align2::CENTER_CENTER, tl!("Mix"), 12.0, t.text);
    } else {
        small_text(ui, pos2(nr.min.x + 6.0, nr.center().y), Align2::LEFT_CENTER, &label, 12.0, t.text);
        let name = ui.painter().with_clip_rect(Rect::from_min_max(pos2(nr.min.x + 26.0, nr.min.y), nr.max).intersect(ui.clip_rect()));
        name.text(pos2(nr.min.x + 30.0, nr.center().y), Align2::LEFT_CENTER, &tr.name, Tokens::ui(12.0), t.text);
    }
    app.auto.add(&format!("{ap}.name"), nr, &tr.name);
}

/// The 5.1 panner: a puck in the listening square (x left … right, y rear … front; speakers drawn
/// at L, C, R, Ls, Rs), and the Center % and LFE values. Drags are `mixer.touch` / `mixer.release`
/// on the `pan51.x` / `pan51.y` lanes, so they are heard live and recorded like the pan knob.
/// The effect rack of a strip (Premiere's Audio Track Mixer): five effect slots, five send slots
/// and the knob of the selected slot's parameter. Returns the y below it.
///
/// An effect slot is selected by a click (its knob shows below), opens the effect list from its
/// arrow (or anywhere when empty; on a filled slot the list replaces it, None empties it), opens
/// its editor on a double-click and its menu on a right-click (Edit…, Bypass, Post-Fader, the
/// knob's parameter, Copy / Paste Track Effects, Remove). Dragging an effect onto another slot
/// of this strip moves it there, onto another strip copies it (with its settings); Ctrl (Cmd)
/// swaps the two ([`drop_copies`]).
#[allow(clippy::too_many_arguments)]
fn rack(app: &mut FilmcraftApp, ui: &mut egui::Ui, cx: &mut Ctx, tr: &Track, id: TrackId, ap: &str, x: f32, w: f32, mut y: f32) -> f32 {
    use crate::state::RackSel;
    use egui::SetOpenCommand;
    let t = cx.t;
    let seq = cx.seq;
    let sid = json!(id.0);
    let is_master = id == MASTER_STRIP;
    let sel = app.ui.mixer_rack.get(&id.0).copied();
    let mut select: Option<Option<RackSel>> = None;
    let mut open_editor = None;
    let can_paste = app.session.state.rack_clipboard.is_some();
    let dragging = egui::DragAndDrop::has_payload_of_type::<RackDrag>(ui.ctx());
    for k in 0..filmcraft_project::mixer::MAX_INSERTS {
        let r = Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, SLOT_H - 2.0));
        let fx = tr.effects.get(k);
        let name = fx.map(|e| e.def().map(|d| d.name).unwrap_or(&e.effect).to_string());
        let text = match (fx, &name) {
            (Some(e), Some(n)) => format!("{}{n}", if e.post_fader { "» " } else { "" }),
            _ => String::new(),
        };
        let resp = ui.interact(r, egui::Id::new((ap, "fx", k)), Sense::click_and_drag());
        let arrow = Rect::from_min_max(pos2(r.max.x - ARROW_W, r.min.y), r.max);
        let hover_drop = dragging && resp.dnd_hover_payload::<RackDrag>().is_some();
        if hover_drop {
            cx.drop_strip = Some(id.0);
        }
        paint_slot(ui, r, &text, fx.is_some(), fx.is_some_and(|e| !e.enabled), sel == Some(RackSel::Effect(k)) && fx.is_some(), resp.hovered(), hover_drop, &t);
        app.auto.add(&format!("{ap}.fx.{k}"), r, &if text.is_empty() { format!("Effect slot {}", k + 1) } else { text.clone() });
        if let (Some(_), Some(n)) = (fx, &name) {
            resp.dnd_set_drag_payload(RackDrag { strip: id.0, slot: k, name: n.clone() });
        }
        if let Some(d) = resp.dnd_release_payload::<RackDrag>() {
            let copy = drop_copies(d.strip == id.0, ui.input(|i| i.modifiers.command));
            cx.cmd("mixer.moveInsert", json!({"strip": d.strip, "slot": d.slot, "to": id.0, "toSlot": k, "copy": copy}));
            let landed = if !copy && d.strip == id.0 { k.min(tr.effects.len().saturating_sub(1)) } else { k.min(tr.effects.len()) };
            select = Some(Some(RackSel::Effect(landed)));
        }
        let on_arrow = resp.interact_pointer_pos().is_some_and(|p| arrow.contains(p));
        if resp.clicked() && fx.is_some() && !on_arrow {
            select = Some(Some(RackSel::Effect(k)));
        }
        let open_list = resp.clicked() && (fx.is_none() || on_arrow);
        // (its own id: the right-click menu below is the response's default popup)
        let list = egui::Popup::menu(&resp).id(egui::Id::new((ap, "fx-list", k)));
        list.open_memory(open_list.then_some(SetOpenCommand::Toggle)).show(|ui| {
            ui.set_min_width(180.0);
            if fx.is_some() {
                // a filled slot: another effect replaces it, None empties it
                let r = ui.button("None");
                app.auto.add(&format!("{ap}.fx.{k}.none"), r.rect, "None");
                if r.clicked() {
                    cx.cmd("mixer.removeInsert", json!({"strip": sid, "slot": k}));
                    ui.close();
                }
                ui.separator();
            }
            // Premiere's effect-slot menu: one submenu per Audio Effects folder
            let list = insert_effects();
            let mut folders: Vec<&str> = Vec::new();
            for d in &list {
                let f = d.category.get(1).copied().unwrap_or("");
                if !folders.contains(&f) {
                    folders.push(f);
                }
            }
            for folder in folders {
                let items: Vec<_> = list.iter().filter(|d| d.category.get(1).copied().unwrap_or("") == folder).collect();
                let mut add = |ui: &mut egui::Ui| {
                    for d in &items {
                        let r = ui.button(d.name);
                        app.auto.add(&format!("{ap}.fx.{k}.{}", d.id), r.rect, d.name);
                        if r.clicked() {
                            if fx.is_some() {
                                cx.cmd("mixer.replaceInsert", json!({"strip": sid, "slot": k, "effect": d.id}));
                            } else {
                                cx.cmd("mixer.addInsert", json!({"strip": sid, "effect": d.id}));
                            }
                            ui.close();
                        }
                    }
                };
                if folder.is_empty() {
                    add(ui);
                } else {
                    let mr = ui.menu_button(folder, |ui| add(ui));
                    app.auto.add(&format!("{ap}.fx.{k}.folder.{folder}"), mr.response.rect, folder);
                }
            }
        });
        resp.context_menu(|ui| {
            ui.set_min_width(190.0);
            if let Some(e) = fx {
                let r0 = ui.button(tl!("Edit…"));
                app.auto.add(&format!("{ap}.fx.{k}.edit"), r0.rect, "Edit…");
                if r0.clicked() {
                    open_editor = Some(crate::panels::audio_fx_editor::FxTarget::Insert { strip: id.0, slot: k });
                    ui.close();
                }
                ui.separator();
                let on = e.enabled;
                let r1 = ui.selectable_label(!on, tl!("Bypass"));
                app.auto.add(&format!("{ap}.fx.{k}.bypass"), r1.rect, "Bypass");
                if r1.clicked() {
                    cx.cmd("mixer.setInsert", json!({"strip": sid, "slot": k, "enabled": !on}));
                }
                let r2 = ui.selectable_label(e.post_fader, tl!("Post-Fader"));
                app.auto.add(&format!("{ap}.fx.{k}.postFader"), r2.rect, "Post-Fader");
                if r2.clicked() {
                    cx.cmd("mixer.setInsert", json!({"strip": sid, "slot": k, "postFader": !e.post_fader}));
                }
                // the parameter the knob under the rack shows
                let shown_param = knob_param(app, id, k, e).map(|p| p.id);
                let params: Vec<&filmcraft_project::ParamDef> =
                    e.def().map(|d| d.params.iter().filter(|p| matches!(p.kind, filmcraft_project::ParamKind::Float { .. })).collect()).unwrap_or_default();
                if !params.is_empty() {
                    ui.separator();
                    for p in params {
                        let label = match p.group {
                            Some(g) if !p.label.starts_with(g) => format!("{g}: {}", p.label),
                            _ => p.label.to_string(),
                        };
                        let r = ui.selectable_label(shown_param == Some(p.id), label);
                        app.auto.add(&format!("{ap}.fx.{k}.knob.{}", p.id), r.rect, p.label);
                        if r.clicked() {
                            app.ui.mixer_knob_param.insert(knob_key(id, k, &e.effect), p.id.to_string());
                            select = Some(Some(RackSel::Effect(k)));
                            ui.close();
                        }
                    }
                }
                ui.separator();
            }
            let r = ui.button("Copy Track Effects");
            app.auto.add(&format!("{ap}.fx.copy"), r.rect, "Copy Track Effects");
            if r.clicked() {
                cx.cmd("mixer.copyInserts", json!({"strip": sid}));
                ui.close();
            }
            let r = ui.add_enabled(can_paste, egui::Button::new("Paste Track Effects"));
            app.auto.add(&format!("{ap}.fx.paste"), r.rect, "Paste Track Effects");
            if r.clicked() {
                cx.cmd("mixer.pasteInserts", json!({"strip": sid}));
                ui.close();
            }
            if fx.is_some() {
                ui.separator();
                let r3 = ui.button(tl!("Remove Effect"));
                app.auto.add(&format!("{ap}.fx.{k}.remove"), r3.rect, "Remove Effect");
                if r3.clicked() {
                    cx.cmd("mixer.removeInsert", json!({"strip": sid, "slot": k}));
                    ui.close();
                }
            }
        });
        // (egui counts a click elsewhere just before a double-click as a triple-click)
        if (resp.double_clicked() || resp.triple_clicked()) && fx.is_some() {
            open_editor = Some(crate::panels::audio_fx_editor::FxTarget::Insert { strip: id.0, slot: k });
        }
        y += SLOT_H;
    }
    if let Some(target) = open_editor {
        crate::panels::audio_fx_editor::open(app, target);
    }
    y += 4.0;
    for k in 0..filmcraft_project::mixer::MAX_SENDS {
        let r = Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, SLOT_H - 2.0));
        if is_master {
            y += SLOT_H;
            continue;
        }
        let snd = tr.mixer.sends.get(k);
        let text = match snd {
            Some(s) => format!("{}{} {}", if s.pre_fader { tl!("pre ") } else { "" }, strip_label(seq, s.target), db_text(s.level_db)),
            None => String::new(),
        };
        let resp = ui.interact(r, egui::Id::new((ap, "send", k)), Sense::click());
        let arrow = Rect::from_min_max(pos2(r.max.x - ARROW_W, r.min.y), r.max);
        paint_slot(ui, r, &text, snd.is_some(), snd.is_some_and(|s| s.muted), sel == Some(RackSel::Send(k)) && snd.is_some(), resp.hovered(), false, &t);
        app.auto.add(&format!("{ap}.send.{k}"), r, &if text.is_empty() { format!("Send slot {}", k + 1) } else { text.clone() });
        let on_arrow = resp.interact_pointer_pos().is_some_and(|p| arrow.contains(p));
        if resp.clicked() && snd.is_some() && !on_arrow {
            select = Some(Some(RackSel::Send(k)));
        }
        let tg = targets(seq, id);
        let open_list = resp.clicked() && (snd.is_none() || on_arrow);
        let list = egui::Popup::menu(&resp).id(egui::Id::new((ap, "send-list", k)));
        list.open_memory(open_list.then_some(SetOpenCommand::Toggle)).show(|ui| {
            ui.set_min_width(180.0);
            for s in &tg {
                let lbl = format!("{} ({})", strip_label(seq, s.id), s.name);
                let r = ui.selectable_label(snd.is_some_and(|x| x.target == s.id), &lbl);
                app.auto.add(&format!("{ap}.send.{k}.{}", strip_label(seq, s.id)), r.rect, &lbl);
                if r.clicked() {
                    if snd.is_some() {
                        cx.cmd("mixer.setSend", json!({"strip": sid, "send": k, "target": s.id.0}));
                    } else {
                        cx.cmd("mixer.addSend", json!({"strip": sid, "target": s.id.0}));
                    }
                    ui.close();
                }
            }
            if snd.is_none() && seq.submix_tracks.iter().all(|s| s.id != id) {
                let r = ui.button(tl!("New Submix"));
                app.auto.add(&format!("{ap}.send.{k}.newSubmix"), r.rect, "New Submix");
                if r.clicked() {
                    let n = seq.submix_tracks.len();
                    cx.cmd("mixer.addSubmix", json!({}));
                    cx.cmd("mixer.addSend", json!({"strip": sid, "target": format!("S{}", n + 1)}));
                    ui.close();
                }
            }
            if tg.is_empty() && snd.is_none() && seq.submix_tracks.iter().any(|s| s.id == id) {
                ui.label("No submix after this one");
            }
        });
        if let Some(s) = snd {
            resp.context_menu(|ui| {
                ui.set_min_width(180.0);
                let mut lvl = s.level_db;
                let sl = ui.add(egui::Slider::new(&mut lvl, FADER_MIN_DB..=FADER_MAX_DB).text("dB"));
                app.auto.add(&format!("{ap}.send.{k}.level"), sl.rect, "Send level");
                if sl.changed() {
                    cx.cmd("mixer.setSend", json!({"strip": sid, "send": k, "levelDb": lvl}));
                }
                let r1 = ui.selectable_label(s.pre_fader, tl!("Pre-Fader"));
                app.auto.add(&format!("{ap}.send.{k}.preFader"), r1.rect, "Pre-Fader");
                if r1.clicked() {
                    cx.cmd("mixer.setSend", json!({"strip": sid, "send": k, "preFader": !s.pre_fader}));
                }
                let r2 = ui.selectable_label(s.muted, tl!("Mute Send"));
                app.auto.add(&format!("{ap}.send.{k}.mute"), r2.rect, "Mute Send");
                if r2.clicked() {
                    cx.cmd("mixer.setSend", json!({"strip": sid, "send": k, "muted": !s.muted}));
                }
                ui.separator();
                let r3 = ui.button(tl!("Remove Send"));
                app.auto.add(&format!("{ap}.send.{k}.remove"), r3.rect, "Remove Send");
                if r3.clicked() {
                    cx.cmd("mixer.removeSend", json!({"strip": sid, "send": k}));
                    ui.close();
                }
            });
        }
        y += SLOT_H;
    }
    if let Some(s) = select {
        match s {
            Some(v) => app.ui.mixer_rack.insert(id.0, v),
            None => app.ui.mixer_rack.remove(&id.0),
        };
    }
    y += 2.0;
    rack_knob(app, ui, cx, tr, id, ap, Rect::from_min_size(pos2(x + 4.0, y), vec2(w - 8.0, KNOB_AREA_H - 6.0)));
    y + KNOB_AREA_H
}

/// The knob area under the rack: the selected effect's chosen parameter (or the selected send's
/// level) as a knob with its value and name, and the bypass button (effect bypass / send mute).
/// Drags are `mixer.touch` / `mixer.release` on the parameter's lane (`fx.<slot>.<param>`,
/// `send.<i>.level`), so they are heard live and recorded as automation like the fader.
#[allow(clippy::too_many_arguments)]
fn rack_knob(app: &mut FilmcraftApp, ui: &mut egui::Ui, cx: &mut Ctx, tr: &Track, id: TrackId, ap: &str, r: Rect) {
    use crate::state::RackSel;
    let t = cx.t;
    let sid = json!(id.0);
    ui.painter().rect_filled(r, 3.0, Color32::from_gray(0x1b));
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    // (lane, value, (min, max), unit, decimals, name, default, bypassed, bypass command)
    struct Knob {
        lane: String,
        value: f64,
        range: (f64, f64),
        unit: &'static str,
        decimals: usize,
        name: String,
        default: f64,
        bypassed: bool,
        bypass: (&'static str, Value),
    }
    let knob = match app.ui.mixer_rack.get(&id.0).copied() {
        Some(RackSel::Effect(k)) => tr.effects.get(k).and_then(|e| {
            let p = knob_param(app, id, k, e)?;
            let filmcraft_project::ParamKind::Float { soft_min, soft_max, unit, decimals, .. } = p.kind else { return None };
            let lane = filmcraft_project::mixer::fx_lane(k, p.id);
            Some(Knob {
                value: shown(app, tr, &lane, cx.now),
                lane,
                range: (soft_min, soft_max),
                unit,
                decimals: decimals as usize,
                name: p.label.to_string(),
                default: p.default.as_f64().unwrap_or(soft_min),
                bypassed: !e.enabled,
                bypass: ("mixer.setInsert", json!({"strip": sid, "slot": k, "enabled": !e.enabled})),
            })
        }),
        Some(RackSel::Send(k)) => tr.mixer.sends.get(k).map(|s| {
            let lane = format!("send.{k}.level");
            Knob {
                value: shown(app, tr, &lane, cx.now),
                lane,
                range: (FADER_MIN_DB, FADER_MAX_DB),
                unit: "dB",
                decimals: 1,
                name: format!("Send to {}", strip_label(cx.seq, s.target)),
                default: 0.0,
                bypassed: s.muted,
                bypass: ("mixer.setSend", json!({"strip": sid, "send": k, "muted": !s.muted})),
            }
        }),
        None => None,
    };
    // bypass button at the right (Premiere's fx circle; struck through when bypassed)
    let bc = pos2(r.max.x - 12.0, r.min.y + 13.0);
    let br = Rect::from_center_size(bc, vec2(18.0, 18.0));
    let bresp = ui.interact(br, egui::Id::new((ap, "rack-bypass")), Sense::click()).on_hover_text("Bypass");
    let active = knob.is_some();
    let ring = if !active {
        Color32::from_gray(0x55)
    } else if bresp.hovered() {
        t.tab_text_active
    } else {
        Color32::from_gray(0xa8)
    };
    ui.painter().circle_stroke(bc, 8.0, Stroke::new(1.2, ring));
    ui.painter().text(bc, Align2::CENTER_CENTER, "fx", egui::FontId::new(9.0, egui::FontFamily::Proportional), ring);
    if knob.as_ref().is_some_and(|k| k.bypassed) {
        ui.painter().line_segment([bc + vec2(-6.0, 6.0), bc + vec2(6.0, -6.0)], Stroke::new(1.5, RED));
    }
    app.auto.add(&format!("{ap}.rack.bypass"), br, "Bypass");
    let Some(k) = knob else { return };
    if bresp.clicked() {
        cx.cmd(k.bypass.0, k.bypass.1.clone());
    }
    let kc = pos2(r.center().x - 4.0, r.min.y + 22.0);
    let kr = Rect::from_center_size(kc, vec2(34.0, 34.0));
    let kresp = ui.interact(kr, egui::Id::new((ap, "rack-knob")), Sense::click_and_drag()).on_hover_text(format!("{} (drag; double-click: default)", k.name));
    let ring = if kresp.hovered() || kresp.dragged() { t.tab_text_active } else { Color32::from_gray(0xd0) };
    let span = (k.range.1 - k.range.0).max(1e-9);
    let u = ((k.value - k.range.0) / span).clamp(0.0, 1.0) as f32;
    let ang = (-135.0 + 270.0 * u).to_radians();
    ui.painter().circle_filled(kc, 13.0, Color32::from_gray(0x2a));
    ui.painter().circle_stroke(kc, 13.0, Stroke::new(1.5, ring));
    ui.painter().line_segment([kc, kc + vec2(ang.sin(), -ang.cos()) * 11.0], Stroke::new(2.0, ring));
    app.auto.add(&format!("{ap}.rack.knob"), kr, &k.name);
    if kresp.dragged() {
        let d = kresp.drag_delta();
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        let nv = (k.value + (d.x - d.y) as f64 * span / 150.0 * fine).clamp(k.range.0, k.range.1);
        if nv != k.value {
            cx.cmd("mixer.touch", json!({"strip": sid, "lane": k.lane, "value": nv, "time": cx.now.0}));
        }
    }
    if kresp.drag_stopped() {
        cx.cmd("mixer.release", json!({"strip": sid, "lane": k.lane, "time": cx.now.0}));
    }
    if kresp.double_clicked() {
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": k.lane, "value": k.default}));
    }
    let vr = Rect::from_center_size(pos2(r.center().x, r.min.y + 50.0), vec2(r.width(), 16.0));
    let mut vui = ui.new_child(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::top_down(egui::Align::Center)));
    let suffix = if k.unit.is_empty() { String::new() } else { format!(" {}", k.unit) };
    let (vresp, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new((ap, "rack-value")), k.value, span / 300.0, k.range, k.decimals, &suffix, &t);
    app.auto.add(&format!("{ap}.rack.value"), vresp.rect, &format!("{:.*}{suffix}", k.decimals, k.value));
    if let Some(v) = nv {
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": k.lane, "value": v}));
    }
    let galley = ui.painter().layout_no_wrap(k.name.clone(), Tokens::ui(10.0), t.text_dim);
    let clip = ui.painter().with_clip_rect(r.shrink(2.0).intersect(ui.clip_rect()));
    let gx = if galley.size().x < r.width() - 6.0 { r.center().x - galley.size().x / 2.0 } else { r.min.x + 3.0 };
    clip.galley(pos2(gx, r.max.y - galley.size().y - 3.0), galley, t.text_dim);
}

fn pan51_panner(app: &mut FilmcraftApp, ui: &mut egui::Ui, cx: &mut Ctx, tr: &Track, ap: &str, sid: &Value, r: Rect) {
    let t = cx.t;
    let px = shown(app, tr, LANE_PAN51_X, cx.now);
    let py = shown(app, tr, LANE_PAN51_Y, cx.now);
    let sq = Rect::from_min_size(pos2(r.min.x, r.min.y), vec2(46.0, 46.0));
    let resp =
        ui.interact(sq, egui::Id::new((ap, "pan51")), Sense::click_and_drag()).on_hover_text(tl!("5.1 panner (drag the puck; double-click: front centre)"));
    let p = ui.painter();
    p.rect_filled(sq, 3.0, t.panner_bg);
    p.rect_stroke(sq, 3.0, Stroke::new(1.0, if resp.hovered() || resp.dragged() { t.tab_text_active } else { t.panner_border }), StrokeKind::Inside);
    let inner = sq.shrink(5.0);
    let to_pos =
        |x: f64, y: f64| pos2(inner.center().x + (x / 100.0) as f32 * inner.width() / 2.0, inner.center().y - (y / 100.0) as f32 * inner.height() / 2.0);
    // speakers
    for (x, y) in [(-100.0, 100.0), (0.0, 100.0), (100.0, 100.0), (-100.0, -100.0), (100.0, -100.0)] {
        p.rect_filled(Rect::from_center_size(to_pos(x, y), vec2(4.0, 4.0)), 1.0, t.panner_speaker);
    }
    let puck = to_pos(px, py);
    p.circle_filled(puck, 4.0, if resp.dragged() { t.accent } else { t.hot_text });
    app.auto.add(&format!("{ap}.pan51"), sq, &format!("5.1 panner x {px:.0} y {py:.0}"));
    if resp.dragged() {
        let d = resp.drag_delta();
        let nx = (px + (d.x / (inner.width() / 2.0) * 100.0) as f64).clamp(-100.0, 100.0);
        let ny = (py - (d.y / (inner.height() / 2.0) * 100.0) as f64).clamp(-100.0, 100.0);
        if nx != px || resp.drag_started() {
            cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_PAN51_X, "value": nx, "time": cx.now.0}));
        }
        if ny != py || resp.drag_started() {
            cx.cmd("mixer.touch", json!({"strip": sid, "lane": LANE_PAN51_Y, "value": ny, "time": cx.now.0}));
        }
    }
    if resp.drag_stopped() {
        cx.cmd("mixer.release", json!({"strip": sid, "lane": LANE_PAN51_X, "time": cx.now.0}));
        cx.cmd("mixer.release", json!({"strip": sid, "lane": LANE_PAN51_Y, "time": cx.now.0}));
    }
    if resp.double_clicked() {
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN51_X, "value": 0.0}));
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN51_Y, "value": 100.0}));
    }
    // Center % and LFE (hot numbers)
    let center = shown(app, tr, LANE_PAN51_CENTER, cx.now);
    let lfe = shown(app, tr, LANE_PAN51_LFE, cx.now);
    let col_x = sq.max.x + 4.0;
    small_text(ui, pos2(col_x, r.min.y + 6.0), Align2::LEFT_CENTER, tl!("Center"), 9.0, t.text_dim);
    let cr = Rect::from_min_size(pos2(col_x, r.min.y + 12.0), vec2(r.max.x - col_x, 14.0));
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(cr));
    let (cresp, nc) = crate::widgets::hot_number(&mut cui, egui::Id::new((ap, "pan51.center")), center, 1.0, (0.0, 100.0), 0, "%", &t);
    app.auto.add(&format!("{ap}.pan51.center"), cresp.rect, &format!("Center {center:.0} %"));
    if let Some(v) = nc {
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN51_CENTER, "value": v}));
    }
    small_text(ui, pos2(col_x, r.min.y + 32.0), Align2::LEFT_CENTER, "LFE", 9.0, t.text_dim);
    let lr = Rect::from_min_size(pos2(col_x, r.min.y + 38.0), vec2(r.max.x - col_x, 14.0));
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(lr));
    let (lresp, nl) =
        crate::widgets::hot_number(&mut lui, egui::Id::new((ap, "pan51.lfe")), lfe.max(FADER_MIN_DB), 0.5, (FADER_MIN_DB, FADER_MAX_DB), 1, "", &t);
    app.auto.add(&format!("{ap}.pan51.lfe"), lresp.rect, &format!("LFE {}", db_text(lfe)));
    if let Some(v) = nl {
        cx.cmd("mixer.setValue", json!({"strip": sid, "lane": LANE_PAN51_LFE, "value": v}));
    }
}

fn transport(app: &mut FilmcraftApp, ui: &mut egui::Ui, row: Rect, cx: &mut Ctx) {
    let t = cx.t;
    let seq = cx.seq;
    ui.painter().line_segment([pos2(row.min.x, row.min.y + 0.5), pos2(row.max.x, row.min.y + 0.5)], Stroke::new(1.0, t.separator));
    let tc = format_time(cx.now, seq.settings.frame_rate, seq.settings.drop_frame, TimeDisplay::Timecode, seq.settings.sample_rate as i64);
    ui.painter().text(pos2(row.min.x + 12.0, row.center().y), Align2::LEFT_CENTER, tc, Tokens::mono(13.0), t.timecode);
    let dur = format_time(seq.duration(), seq.settings.frame_rate, seq.settings.drop_frame, TimeDisplay::Timecode, seq.settings.sample_rate as i64);
    ui.painter().text(pos2(row.max.x - 12.0, row.center().y), Align2::RIGHT_CENTER, dur, Tokens::mono(12.0), t.text_dim);
    let playing = app.playback.playing;
    let buttons: [(Icon, &str, &str); 6] = [
        (Icon::GoToIn, "markers.goToIn", tl!("Go to In Point")),
        (Icon::GoToOut, "markers.goToOut", tl!("Go to Out Point")),
        (if playing { Icon::Pause } else { Icon::Play }, "playback.toggle", tl!("Play-Stop Toggle (Space)")),
        (Icon::Play, "playback.inToOut", tl!("Play In to Out")),
        (Icon::Loop, "playback.loop", tl!("Loop")),
        (Icon::Mic, "voiceover.recordToggle", tl!("Record (voice-over on the record-enabled track)")),
    ];
    let bw = 30.0;
    let mut x = row.center().x - bw * buttons.len() as f32 / 2.0;
    for (icon, cmd, tip) in buttons {
        let r = Rect::from_min_size(pos2(x, row.min.y + 4.0), vec2(bw - 2.0, 26.0));
        let resp = ui.interact(r, egui::Id::new(("mixer-transport", cmd)), Sense::click()).on_hover_text(tip);
        app.auto.add(&format!("mixer.transport.{cmd}"), r, tip);
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        let on = cmd == "playback.loop" && app.playback.looping;
        let col = if on {
            t.accent
        } else if resp.hovered() {
            t.tab_text_active
        } else {
            t.icon
        };
        if cmd == "voiceover.recordToggle" {
            let on = app.session.voiceover.recording();
            ui.painter().circle_filled(r.center(), 6.0, if on { RED } else { RED.gamma_multiply(0.55) });
        } else {
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, col);
            if cmd == "playback.inToOut" {
                // braces around the play triangle
                let c = r.center();
                for s in [-1.0f32, 1.0] {
                    ui.painter().line_segment([pos2(c.x + s * 10.0, c.y - 6.0), pos2(c.x + s * 10.0, c.y + 6.0)], Stroke::new(1.5, col));
                }
            }
        }
        if resp.clicked() {
            cx.cmd(cmd, json!({}));
        }
        x += bw;
    }
}

// ------------------------------------------------------------------------------------- clip mixer

/// Audio Clip Mixer: one strip per audio track, controlling the clip under the playhead (clip
/// Volume and Panner). With the keyframe button on, changes add keyframes at the playhead.
pub fn clip_mixer(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    // Keep the immutable project snapshot alive while UI commands mutate the session.
    // Cloning a sequence here copies every clip and effect on every redraw.
    let project = app.session.project.clone();
    let Some(seq) = app.session.state.active_sequence.and_then(|id| project.sequence(id)) else {
        crate::dock::placeholder(ui, rect, &t, tl!("(no sequence)"));
        return;
    };
    let meters = poll_meters(app, ui);
    let now = app.session.playhead();
    let mut acts: Vec<(String, Value)> = Vec::new();
    let kf_id = egui::Id::new("clip-mixer-keyframes");
    let mut write_kf: HashMap<u64, bool> = ui.data(|d| d.get_temp(kf_id)).unwrap_or_default();
    // Like Premiere, the strips narrow so that every track fits the panel; below the narrowest
    // usable strip the row scrolls instead, so no track is out of reach (#1922).
    let x0 = rect.min.x + 6.0;
    let avail = rect.max.x - x0 - 2.0;
    let n = seq.audio_tracks.len();
    let sw = (avail / n.max(1) as f32 - 2.0).clamp(CLIP_STRIP_MIN_W, STRIP_W);
    let (off, sb_h) = strip_scroll(app, ui, rect, x0, n as f32 * (sw + 2.0), avail, "clipMixer");
    let view = Rect::from_min_max(pos2(x0, rect.min.y), pos2(rect.max.x - 2.0, rect.max.y - sb_h));
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(view));
    cui.set_clip_rect(view.intersect(ui.clip_rect()));
    let ui = &mut cui;
    for (i, tr) in seq.audio_tracks.iter().enumerate() {
        let x = x0 + i as f32 * (sw + 2.0) - off;
        if x + sw < view.min.x || x > view.max.x {
            continue;
        }
        let sr = Rect::from_min_max(pos2(x, rect.min.y + 4.0), pos2(x + sw, view.max.y - 4.0));
        let label = format!("A{}", i + 1);
        let ap = format!("clipMixer.{label}");
        ui.painter().rect_filled(sr, 3.0, t.control_surface);
        let clip = tr.item_at(now).filter(|c| c.enabled);
        let active = clip.is_some();
        let dimc = |c: Color32| if active { c } else { c.gamma_multiply(0.4) };
        let mt = clip.map(|c| c.source_time_at(now));
        let vol = clip.and_then(|c| c.effect("volume")).map(|e| e.f64_at("level", mt.unwrap_or_default())).unwrap_or(0.0);
        let pan = clip.and_then(|c| c.effect("panner")).map(|e| e.f64_at("balance", mt.unwrap_or_default())).unwrap_or(0.0);
        let kf = *write_kf.get(&tr.id.0).unwrap_or(&false);
        // during an automation pass, moves are live gestures (recorded in Latch / Touch / Write)
        let gestures = app.session.mixrec.active();
        let lane_of = |effect: &str| if effect == "panner" { "pan" } else { "volume" };
        let set = |acts: &mut Vec<(String, Value)>, effect: &str, param: &str, v: f64, begin: bool| {
            if gestures {
                acts.push(("clipMixer.touch".into(), json!({"track": tr.id.0, "lane": lane_of(effect), "value": v, "time": now.0})));
            } else if let Some(c) = clip {
                acts.push(("clipMixer.set".into(), json!({"clip": c.id.0, "effect": effect, "param": param, "value": v, "keyframe": kf, "begin": begin})));
            }
        };
        let release = |acts: &mut Vec<(String, Value)>, effect: &str| {
            if gestures {
                acts.push(("clipMixer.release".into(), json!({"track": tr.id.0, "lane": lane_of(effect), "time": now.0})));
            }
        };
        let live = &app.session.previews.live;
        let vol = live.get(tr.id, filmcraft_render::audio::CLIP_LANE_VOLUME).map(|o| o.value).unwrap_or(vol);
        let pan = live.get(tr.id, filmcraft_render::audio::CLIP_LANE_PAN).map(|o| o.value).unwrap_or(pan);
        // pan knob
        let kc = pos2(sr.center().x, sr.min.y + 36.0);
        let kr = Rect::from_center_size(kc, vec2(30.0, 30.0));
        let kresp = ui.interact(kr, egui::Id::new((&ap, "pan")), Sense::click_and_drag());
        ui.painter().circle_stroke(kc, 14.0, Stroke::new(2.0, dimc(t.control_handle_dim)));
        let ang = (pan / 100.0) as f32 * 135f32.to_radians();
        ui.painter().line_segment([kc, kc + vec2(ang.sin(), -ang.cos()) * 12.0], Stroke::new(2.0, dimc(t.control_handle_dim)));
        small_text(ui, pos2(kc.x - 18.0, kc.y + 16.0), Align2::CENTER_CENTER, "L", 11.0, dimc(t.hot_text));
        small_text(ui, pos2(kc.x + 18.0, kc.y + 16.0), Align2::CENTER_CENTER, "R", 11.0, dimc(t.hot_text));
        small_text(ui, pos2(kc.x, kc.y + 26.0), Align2::CENTER_CENTER, &format!("{pan:.1}"), 11.5, dimc(t.hot_text));
        app.auto.add(&format!("{ap}.pan"), kr, "Clip pan");
        if kresp.dragged() && active {
            let d = kresp.drag_delta();
            set(&mut acts, "panner", "balance", (pan + (d.x - d.y) as f64).clamp(-100.0, 100.0), kresp.drag_started());
        }
        if kresp.drag_stopped() {
            release(&mut acts, "panner");
        }
        if kresp.double_clicked() {
            set(&mut acts, "panner", "balance", 0.0, true);
        }
        // automation mode (Clip Mixer)
        let mode = filmcraft_engine::mixer::clip_mode(&app.session, tr.id);
        let mdr = Rect::from_min_size(pos2(sr.min.x + 4.0, sr.min.y + 2.0), vec2(sr.width() - 8.0, 16.0));
        let mdresp = crate::widgets::dropdown_text(ui, mdr, crate::i18n::t(mode.label()), &t, egui::Id::new((&ap, "mode")));
        app.auto.add(&format!("{ap}.mode"), mdr, &format!("Clip automation mode: {}", mode.label()));
        egui::Popup::menu(&mdresp).show(|ui| {
            for m in AutomationMode::ALL {
                let r = ui.selectable_label(m == mode, crate::i18n::t(m.label()));
                app.auto.add(&format!("{ap}.mode.{}", m.label()), r.rect, m.label());
                if r.clicked() {
                    acts.push(("clipMixer.setMode".into(), json!({"track": tr.id.0, "mode": m.label()})));
                }
            }
        });
        // M S ◇
        let y = sr.min.y + 72.0;
        let step = ((sr.width() - 6.0 - 18.0) / 2.0).min(24.0);
        let b = |k: f32| Rect::from_min_size(pos2(sr.center().x - step - 9.0 + k * step, y), vec2(18.0, 16.0));
        let mr = crate::widgets::letter_toggle(ui, b(0.0), "M", tr.muted, MUTE_ON, &t, egui::Id::new((&ap, "M")));
        app.auto.add(&format!("{ap}.mute"), b(0.0), "Mute Track");
        if mr.clicked() {
            acts.push(("mixer.setStrip".into(), json!({"strip": tr.id.0, "muted": !tr.muted})));
        }
        let so = crate::widgets::letter_toggle(ui, b(1.0), "S", tr.solo, SOLO_ON, &t, egui::Id::new((&ap, "S")));
        app.auto.add(&format!("{ap}.solo"), b(1.0), "Solo Track");
        if so.clicked() {
            acts.push(("mixer.setStrip".into(), json!({"strip": tr.id.0, "solo": !tr.solo})));
        }
        let kresp =
            crate::widgets::icon_toggle(ui, b(2.0), Icon::Keyframe, kf, &t, egui::Id::new((&ap, "kf")), Some(t.accent)).on_hover_text(tl!("Write keyframes"));
        app.auto.add(&format!("{ap}.keyframe"), b(2.0), "Write Keyframes");
        if kresp.clicked() {
            write_kf.insert(tr.id.0, !kf);
        }
        // fader + meter
        let area = Rect::from_min_max(pos2(sr.min.x, y + 30.0), pos2(sr.max.x, sr.max.y - 46.0));
        if area.height() > 40.0 {
            let track = Rect::from_min_max(pos2(sr.min.x + 33.0, area.min.y + 8.0), pos2(sr.min.x + 37.0, area.max.y - 8.0));
            let ypos = |db: f64| track.max.y - db_to_pos(db) * track.height();
            for d in SCALE {
                let s = if d <= FADER_MIN_DB { "-∞".to_string() } else { format!("{}", d as i32) };
                small_text(ui, pos2(sr.min.x + 26.0, ypos(d)), Align2::RIGHT_CENTER, &s, 9.0, t.text_dim);
            }
            ui.painter().rect_filled(track, 2.0, t.control_well);
            let cy = ypos(vol);
            let cap = Rect::from_center_size(pos2(track.center().x, cy), vec2(18.0, 26.0));
            let fresp = ui.interact(cap.expand(3.0), egui::Id::new((&ap, "fader")), Sense::click_and_drag());
            ui.painter().rect_filled(cap, 3.0, t.control_handle_bg);
            ui.painter().rect_stroke(cap, 3.0, Stroke::new(2.0, dimc(t.control_handle)), StrokeKind::Inside);
            ui.painter().line_segment([pos2(cap.min.x + 5.0, cy), pos2(cap.max.x - 5.0, cy)], Stroke::new(2.0, dimc(t.control_handle)));
            app.auto.add(&format!("{ap}.fader"), cap, &format!("Clip volume {}", db_text(vol)));
            if fresp.dragged() && active && fresp.drag_delta().y != 0.0 {
                set(&mut acts, "volume", "level", pos_to_db(db_to_pos(vol) - fresp.drag_delta().y / track.height()), fresp.drag_started());
            }
            if fresp.drag_stopped() {
                release(&mut acts, "volume");
            }
            if fresp.double_clicked() {
                set(&mut acts, "volume", "level", 0.0, true);
            }
            let mrect = Rect::from_min_max(pos2(sr.min.x + 46.0, area.min.y), pos2(sr.max.x - 6.0, area.max.y));
            let m = meters.get(&tr.id.0).cloned().unwrap_or_default();
            draw_meters(ui, mrect, &m, if m.is_empty() { filmcraft_render::mixer::width_of(tr.channels) } else { m.len() }, &t);
        }
        // dB readout: drag to scrub, click to type a value (clamped to the fader range), like the track mixer
        let vr = Rect::from_min_size(pos2(sr.min.x + 4.0, sr.max.y - 42.0), vec2(50.0, 16.0));
        let mut vui = ui.new_child(egui::UiBuilder::new().max_rect(vr));
        if !active {
            vui.set_opacity(0.4);
        }
        let (vresp, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new((&ap, "value")), vol.max(-96.0), 0.1, (FADER_MIN_DB, FADER_MAX_DB), 1, "", &t);
        app.auto.add(&format!("{ap}.value"), vresp.rect, &db_text(vol));
        if let (Some(v), true) = (nv, active) {
            set(&mut acts, "volume", "level", v, !vresp.dragged() || vresp.drag_started());
            if !vresp.dragged() {
                release(&mut acts, "volume");
            }
        }
        if vresp.drag_stopped() {
            release(&mut acts, "volume");
        }
        let name = ui.painter().with_clip_rect(sr.intersect(ui.clip_rect()));
        name.text(pos2(sr.min.x + 6.0, sr.max.y - 10.0), Align2::LEFT_CENTER, &label, Tokens::ui(12.0), t.text);
        name.text(pos2(sr.min.x + 30.0, sr.max.y - 10.0), Align2::LEFT_CENTER, &tr.name, Tokens::ui(12.0), t.text);
    }
    ui.data_mut(|d| d.insert_temp(kf_id, write_kf));
    run_actions(app, &ui.ctx().clone(), acts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_well_and_inactive_clip_light_follow_theme() {
        for kind in [crate::theme::ThemeKind::Dark, crate::theme::ThemeKind::Medium, crate::theme::ThemeKind::Light] {
            let t = Tokens::for_kind(kind);
            let ctx = egui::Context::default();
            let area = Rect::from_min_size(pos2(20.0, 20.0), vec2(40.0, 100.0));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw_meters(ui, area, &[[-90.0; 2]], 1, &t));
            out.textures_delta.clear();
            let rectangles: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => Some(r),
                    _ => None,
                })
                .collect();
            let well = rectangles.iter().find(|r| r.rect == area).expect("painted meter well");
            assert_eq!(well.fill, t.meter_bg, "meter well did not follow {kind:?}");
            let light = rectangles.iter().find(|r| r.rect.height() == 4.0).expect("painted clip light");
            assert_eq!(light.fill, t.meter_clip_off, "clip light did not follow {kind:?}");
        }
    }

    #[test]
    fn taper_roundtrips_and_is_monotonic() {
        let mut last = f64::NEG_INFINITY;
        for i in 0..=100 {
            let p = i as f32 / 100.0;
            let db = pos_to_db(p);
            assert!(db >= last - 1e-9);
            last = db;
            if db > FADER_MIN_DB {
                assert!((db_to_pos(db) - p).abs() < 1e-4, "{p} → {db}");
            }
        }
        assert_eq!(db_to_pos(0.0), 0.76);
        assert_eq!(pos_to_db(1.0), 15.0);
        assert_eq!(pos_to_db(0.0), FADER_MIN_DB);
    }
}
