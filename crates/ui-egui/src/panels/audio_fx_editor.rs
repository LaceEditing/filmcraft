//! Clip / Track Fx Editor windows: Premiere-style settings windows for every audio effect.
//! Opened from Effect Controls ("Custom Setup ▸ Edit…") and from a track's effect slot in the
//! Audio Track Mixer (double-click, or right-click ▸ Edit…).
//!
//! Every window has the same frame: a title bar ("Track Fx Editor - Dynamics: Audio 3, Slot 2"),
//! a header with the effect's bypass (power) button and its Presets menu ("(Default)" and the
//! settings saved for that effect, with save and delete buttons), and a footer naming the
//! channels it processes ("In: L, R | Out: L, R"). In between:
//!
//! - **Dynamics**: Premiere's four sections side by side (AutoGate, Compressor, Expander,
//!   Limiter), each switched by its checkbox, with knobs and live meters — the gate's
//!   open / closing / closed lights, gain-reduction bars, the Limit light — fed by the playing
//!   mix for track effects (`LiveMix::take_fx_meters`).
//! - **Hard Limiter**: Peak / True Peak, scaled sliders for Maximum Amplitude, Input Boost,
//!   Look-Ahead and Release Time, Link Channels.
//! - **Parametric Equalizer**: Premiere's graph (a point per band that is on, dragged where
//!   grabbed, held at the edges; Range 30 / 96 dB) over its Frequency / Gain / Q / Band table.
//! - **Graphic Equalizer, Multiband Compressor**: their band graphs.
//!
//! Every curve comes from the real DSP (`render::audio_fx::configured` → `response_db` /
//! `transfer_db`), so it shows what playback does.
//! - **Any other audio effect**: scaled sliders for a few parameters, or sections of knobs for
//!   many (one per parameter group), checkboxes for switches and radio buttons for choices.
//!
//! Knobs and sliders drag (Shift: finer); their blue values scrub and take typed numbers;
//! double-clicking a knob resets it. Drags preview locally and commit one `effects.setParam` /
//! `mixer.setInsert` on release (one undo step).
//!
//! Automation ids (`<fx>` = effect id): `fxEditor.<fx>.param.<param>` (knobs, sliders, EQ bands),
//! `…value.<param>` (their numbers), `…toggle.<param>` (checkboxes), `…choice.<param>.<i>`
//! (radio buttons) or `…choice.<param>` (menus), `…meter.<gate|comp|exp|lim>`, the EQ graph's
//! `…plot`, `…node.<band>` (hp, low, b1, b2, mid, b4, b5, high, lp; only bands that are on),
//! `…toggle.<band>_on` (Band buttons), `…toggle.low_steep` / `high_steep`, `…range.30|96`, the multiband
//! `…xo1…3` and `…curve.<band>`, `…reset`, the frame's `…bypass`, `…presets`,
//! `…presets.<name>`, `…presets.save`, `…presets.name`, `…presets.delete` and `…close`.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_engine::presets::DEFAULT_SETTINGS;
use filmcraft_project::{AudioChannels, ClipId, EffectInstance, ParamDef, ParamKind, ParamValue, TrackId};
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

/// What an editor window edits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FxTarget {
    /// Clip effect `index` of clip `clip` (Clip Fx Editor).
    Clip { clip: u64, index: usize },
    /// Insert `slot` of mixer strip `strip` (track id; Track Fx Editor).
    Insert { strip: u64, slot: usize },
}

// Premiere's editor palette: dark panel, light labels, blue values.
const BG: Color32 = Color32::from_gray(0x23);
const TITLE_BG: Color32 = Color32::from_gray(0x2e);
const LINE: Color32 = Color32::from_gray(0x3c);
const LABEL: Color32 = Color32::from_gray(0xcc);
const DIM: Color32 = Color32::from_gray(0x80);
const KNOB: Color32 = Color32::from_gray(0xb8);
const LED_OFF: Color32 = Color32::from_gray(0x0a);
const SEG_OFF: Color32 = Color32::from_gray(0x3a);
const GREEN: Color32 = Color32::from_rgb(0x3c, 0xc8, 0x5a);
const AMBER: Color32 = Color32::from_rgb(0xe6, 0xb4, 0x2d);
const RED: Color32 = Color32::from_rgb(0xe0, 0x3a, 0x32);
/// Knob cell size (knob, value, label).
const CELL: egui::Vec2 = egui::Vec2::new(104.0, 104.0);

/// Every audio effect with parameters to set (not Volume / Channel Volume / Panner, which the
/// clip itself carries) has an editor.
pub fn has_editor(effect: &str) -> bool {
    filmcraft_project::find_effect(effect).is_some_and(|d| d.kind == filmcraft_project::EffectKind::Audio && !d.intrinsic)
}

/// Open (or raise) the editor for a target.
pub fn open(app: &mut FilmcraftApp, target: FxTarget) {
    if let Some(res) = resolve(app, &target) {
        app.ui.audio_fx_editor_effects.insert(key(&target), res.inst.effect);
    }
    if !app.ui.audio_fx_editors.contains(&target) {
        app.ui.audio_fx_editors.push(target);
    }
}

/// The edited effect (current project state), the time its values are read at, the place in the
/// title ("Audio 3, Slot 2"), and the channels it processes.
struct Resolved {
    inst: EffectInstance,
    mt: Tick,
    place: String,
    channels: AudioChannels,
}

fn resolve(app: &FilmcraftApp, target: &FxTarget) -> Option<Resolved> {
    let seq = app.session.active_sequence()?;
    let ph = app.session.playhead();
    match target {
        FxTarget::Clip { clip, index } => {
            let (tid, it) = seq.find_item(ClipId(*clip))?;
            let inst = it.effects.get(*index)?.clone();
            let tr = seq.track(tid);
            let mt = it.source_time_at(ph.clamp(it.start, it.end() - Tick(1)));
            Some(Resolved {
                inst,
                mt,
                place: format!("{}, {}", tr.map(|t| t.name.clone()).unwrap_or_default(), it.name),
                channels: tr.map_or(AudioChannels::Stereo, |t| t.channels),
            })
        }
        FxTarget::Insert { strip, slot } => {
            let tr = seq.strip(TrackId(*strip))?;
            let inst = tr.effects.get(*slot)?.clone();
            Some(Resolved { inst, mt: ph, place: format!("{}, Slot {}", tr.name, slot + 1), channels: tr.channels })
        }
    }
}

fn key(target: &FxTarget) -> String {
    match target {
        FxTarget::Clip { clip, index } => format!("clip-{clip}-{index}"),
        FxTarget::Insert { strip, slot } => format!("insert-{strip}-{slot}"),
    }
}

/// Command that sets one parameter on the target.
fn set_cmd(target: &FxTarget, pid: &str, v: Value) -> (String, Value) {
    match target {
        FxTarget::Clip { clip, index } => ("effects.setParam".into(), json!({"clip": clip, "effect": index, "param": pid, "value": v})),
        FxTarget::Insert { strip, slot } => ("mixer.setInsert".into(), json!({"strip": strip, "slot": slot, "params": {pid: v}})),
    }
}

/// Command that bypasses / enables the effect.
fn bypass_cmd(target: &FxTarget, enabled: bool) -> (String, Value) {
    match target {
        FxTarget::Clip { clip, index } => ("effects.toggleEnabled".into(), json!({"clip": clip, "index": index})),
        FxTarget::Insert { strip, slot } => ("mixer.setInsert".into(), json!({"strip": strip, "slot": slot, "enabled": !enabled})),
    }
}

/// The target as `presets.saveSettings` / `presets.applySettings` address it.
fn preset_target(target: &FxTarget) -> Value {
    match target {
        FxTarget::Clip { clip, index } => json!({"clip": clip, "index": index}),
        FxTarget::Insert { strip, slot } => json!({"strip": strip, "slot": slot}),
    }
}

/// "In: L, R | Out: L, R".
fn channel_text(c: AudioChannels) -> String {
    let ch = match c {
        AudioChannels::Mono => "M",
        AudioChannels::Surround51 => "L, R, C, LFE, Ls, Rs",
        AudioChannels::Stereo | AudioChannels::Adaptive => "L, R",
    };
    format!("In: {ch} | Out: {ch}")
}

/// Per-window editing context.
struct Ed<'a> {
    t: Tokens,
    fx: &'a str,
    target: &'a FxTarget,
    inst: EffectInstance,
    mt: Tick,
    /// Uncommitted values (live drags), by parameter id.
    drafts: HashMap<String, f64>,
    acts: Vec<(String, Value)>,
    autos: Vec<(String, Rect, String)>,
    /// Live gain reduction of the effect's stages (dB ≤ 0; 0 when idle or not metered).
    meters: [f32; 4],
    /// Size of a knob cell (smaller where many knobs share the window).
    cell: egui::Vec2,
}

impl Ed<'_> {
    fn def(&self, pid: &str) -> Option<&'static ParamDef> {
        self.inst.def().and_then(|d| d.param(pid))
    }
    fn v(&self, pid: &str) -> f64 {
        self.drafts.get(pid).copied().unwrap_or_else(|| self.inst.f64_at(pid, self.mt))
    }
    fn on(&self, pid: &str) -> bool {
        self.v(pid) >= 0.5
    }
    /// (min, max, unit) of a number: the hard range.
    fn range(&self, pid: &str) -> (f64, f64, &'static str) {
        match self.def(pid).map(|p| &p.kind) {
            Some(ParamKind::Float { min, max, unit, .. }) => (*min, *max, unit),
            _ => (0.0, 1.0, ""),
        }
    }
    /// The slider / knob range (Premiere's scale) and decimals.
    fn scale(&self, pid: &str) -> (f64, f64, usize) {
        match self.def(pid).map(|p| &p.kind) {
            Some(ParamKind::Float { soft_min, soft_max, decimals, .. }) if soft_max > soft_min => (*soft_min, *soft_max, *decimals as usize),
            Some(ParamKind::Float { min, max, decimals, .. }) => (*min, *max, *decimals as usize),
            _ => (0.0, 1.0, 1),
        }
    }
    /// Frequencies and wide time ranges turn logarithmically.
    fn log(&self, pid: &str) -> bool {
        let (lo, hi, _) = self.scale(pid);
        let unit = self.range(pid).2;
        lo > 0.0 && ((unit == "Hz" && hi / lo > 10.0) || ((unit == "ms" || unit == "s") && hi / lo >= 100.0))
    }
    fn to_u(&self, pid: &str, v: f64) -> f32 {
        let (lo, hi, _) = self.scale(pid);
        let u = if self.log(pid) { (v.max(lo) / lo).ln() / (hi / lo).ln() } else { (v - lo) / (hi - lo).max(1e-12) };
        u.clamp(0.0, 1.0) as f32
    }
    fn value_of_u(&self, pid: &str, u: f32) -> f64 {
        let (lo, hi, dec) = self.scale(pid);
        let u = u.clamp(0.0, 1.0) as f64;
        let v = if self.log(pid) { lo * (hi / lo).powf(u) } else { lo + (hi - lo) * u };
        let k = 10f64.powi(dec as i32);
        (v * k).round() / k
    }
    fn default(&self, pid: &str) -> f64 {
        self.def(pid).and_then(|p| p.default.as_f64()).unwrap_or(0.0)
    }
    fn commit(&mut self, pid: &str, v: f64) {
        let val = match self.def(pid).map(|p| &p.kind) {
            Some(ParamKind::Bool) => json!(v >= 0.5),
            Some(ParamKind::Choice(_)) => json!(v.max(0.0).round() as u64),
            _ => json!(v),
        };
        self.acts.push(set_cmd(self.target, pid, val));
        // keep showing the new value until the project catches up this frame
        self.drafts.insert(pid.to_string(), v);
    }
    fn auto(&mut self, id: String, r: Rect, label: &str) {
        self.autos.push((id, r, label.to_string()));
    }
    /// The effect as currently edited (drafts applied), for curve evaluation.
    fn preview(&self) -> EffectInstance {
        let mut e = self.inst.clone();
        for (k, v) in &self.drafts {
            if let Some(p) = e.params.get_mut(k) {
                p.keyframes.clear();
                p.value = match p.value {
                    ParamValue::Bool(_) => ParamValue::Bool(*v >= 0.5),
                    ParamValue::Choice(_) => ParamValue::Choice(*v as u32),
                    _ => ParamValue::Float(*v),
                };
            } else if let Some(d) = e.def().and_then(|d| d.param(k)) {
                let mut p = filmcraft_project::Param::new(d.default.clone());
                p.value = ParamValue::Float(*v);
                e.params.insert(k.clone(), p);
            }
        }
        e
    }

    /// The blue number of a parameter (scrub or type it); drags preview, release commits.
    fn value_text(&mut self, ui: &mut egui::Ui, pid: &str, r: Rect, align: egui::Align) {
        let (min, max, unit) = self.range(pid);
        let (lo, hi, dec) = self.scale(pid);
        let v = self.v(pid);
        let suffix = if unit.is_empty() || unit.starts_with(':') { unit.to_string() } else { format!(" {unit}") };
        let mut vui = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(align)));
        let id = egui::Id::new(("fx-value", self.target.clone(), pid));
        let dec = shown_decimals(v, dec);
        let (resp, nv) = crate::widgets::hot_number(&mut vui, id, v, (hi - lo).abs().max(1e-6) / 250.0, (min, max), dec, &suffix, &self.t);
        self.auto(format!("fxEditor.{}.value.{pid}", self.fx), resp.rect, &format!("{v:.dec$}{suffix}"));
        if let Some(n) = nv {
            if resp.dragged() {
                self.drafts.insert(pid.to_string(), n);
            } else {
                self.commit(pid, n);
            }
        }
        if resp.drag_stopped() {
            let c = self.v(pid);
            self.commit(pid, c);
        }
    }

    /// A knob cell: the knob, its blue value and its name under it (Premiere's Dynamics look).
    fn knob(&mut self, ui: &mut egui::Ui, pid: &str, label: &str) {
        let size = self.cell;
        let compact = size.y < CELL.y;
        let (cell, _) = ui.allocate_exact_size(size, Sense::hover());
        if self.def(pid).is_none() {
            return;
        }
        let radius = if compact { 15.0 } else { 18.0 };
        let c = pos2(cell.center().x, cell.min.y + radius + 6.0);
        let kr = Rect::from_center_size(c, vec2(radius * 2.0 + 6.0, radius * 2.0 + 6.0));
        let resp = ui.interact(kr, egui::Id::new(("fx-knob", self.target.clone(), pid)), Sense::click_and_drag());
        let v = self.v(pid);
        let u = self.to_u(pid, v);
        let col = if resp.hovered() || resp.dragged() { Color32::WHITE } else { KNOB };
        let ang = (-135.0 + 270.0 * u).to_radians();
        let p = ui.painter();
        p.circle_stroke(c, radius, Stroke::new(1.6, col));
        p.line_segment([c, c + vec2(ang.sin(), -ang.cos()) * (radius - 1.0)], Stroke::new(1.8, col));
        self.auto(format!("fxEditor.{}.param.{pid}", self.fx), kr, label);
        if resp.dragged() {
            let d = resp.drag_delta();
            let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
            let nu = u + (d.x - d.y) / 200.0 * fine;
            self.drafts.insert(pid.to_string(), self.value_of_u(pid, nu));
        }
        if resp.drag_stopped() {
            let nv = self.v(pid);
            self.commit(pid, nv);
        }
        // (egui counts a click elsewhere just before as the first of three)
        if resp.double_clicked() || resp.triple_clicked() {
            let d = self.default(pid);
            self.commit(pid, d);
        }
        let resp = resp.on_hover_text(format!("{label} (drag; Shift: finer; double-click: default)"));
        let _ = resp;
        let vy = c.y + radius + 16.0;
        self.value_text(ui, pid, Rect::from_center_size(pos2(cell.center().x, vy), vec2(size.x, 18.0)), egui::Align::Center);
        ui.painter().text(pos2(cell.center().x, vy + 24.0), Align2::CENTER_CENTER, label, FontId::proportional(if compact { 12.0 } else { 13.0 }), LABEL);
    }

    /// A row of up to two knobs.
    fn knobs(&mut self, ui: &mut egui::Ui, row: &[(&str, &str)]) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (pid, label) in row {
                self.knob(ui, pid, label);
            }
        });
    }

    /// A scaled slider row: the name right-aligned, a track with Premiere's tick scale above it,
    /// and the blue value at the right (Hard Limiter).
    fn slider_row(&mut self, ui: &mut egui::Ui, pid: &str, label: &str) {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width().max(520.0), 56.0), Sense::hover());
        if self.def(pid).is_none() {
            return;
        }
        let (lo, hi, _) = self.scale(pid);
        let ty = row.min.y + 36.0;
        let label_w = ui.painter().layout_no_wrap(format!("{label}:"), FontId::proportional(13.5), LABEL).size().x;
        let track = Rect::from_min_max(pos2(row.min.x + (label_w + 28.0).max(172.0), ty - 1.0), pos2(row.max.x - 110.0, ty + 1.0));
        let p = ui.painter();
        p.text(pos2(track.min.x - 14.0, ty), Align2::RIGHT_CENTER, format!("{label}:"), FontId::proportional(13.5), LABEL);
        // the scale: labelled major ticks, minor ticks between
        let (major, minor) = ticks(lo, hi);
        let x_of = |v: f64| track.min.x + self.to_u(pid, v) * track.width();
        let mut m = (lo / minor).ceil() * minor;
        while m <= hi + 1e-9 {
            let x = x_of(m);
            let is_major = ((m / major).round() * major - m).abs() < minor * 0.01;
            let h = if is_major { 8.0 } else { 4.0 };
            p.line_segment([pos2(x, ty - 3.0 - h), pos2(x, ty - 3.0)], Stroke::new(1.0, DIM));
            if is_major {
                p.text(pos2(x, ty - 14.0), Align2::CENTER_BOTTOM, fmt_tick(m), FontId::proportional(11.5), LABEL);
            }
            m += minor;
        }
        p.line_segment([pos2(track.min.x, ty), pos2(track.max.x, ty)], Stroke::new(1.0, DIM));
        p.line_segment([pos2(track.min.x, ty - 6.0), pos2(track.min.x, ty + 3.0)], Stroke::new(1.0, DIM));
        p.line_segment([pos2(track.max.x, ty - 6.0), pos2(track.max.x, ty + 3.0)], Stroke::new(1.0, DIM));
        let hit = track.expand2(vec2(8.0, 12.0));
        let resp = ui.interact(hit, egui::Id::new(("fx-slider", self.target.clone(), pid)), Sense::click_and_drag());
        let v = self.v(pid);
        let x = track.min.x + self.to_u(pid, v) * track.width();
        let col = if resp.hovered() || resp.dragged() { Color32::WHITE } else { KNOB };
        ui.painter().circle_filled(pos2(x, ty), 7.0, BG);
        ui.painter().circle_stroke(pos2(x, ty), 7.0, Stroke::new(1.6, col));
        self.auto(format!("fxEditor.{}.param.{pid}", self.fx), hit, label);
        if (resp.dragged() || resp.is_pointer_button_down_on())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let u = (pos.x - track.min.x) / track.width().max(1.0);
            self.drafts.insert(pid.to_string(), self.value_of_u(pid, u));
        }
        if resp.drag_stopped() || resp.clicked() {
            let nv = self.v(pid);
            self.commit(pid, nv);
        }
        if resp.double_clicked() || resp.triple_clicked() {
            let d = self.default(pid);
            self.commit(pid, d);
        }
        self.value_text(ui, pid, Rect::from_min_max(pos2(track.max.x + 12.0, ty - 9.0), pos2(row.max.x, ty + 9.0)), egui::Align::Max);
    }

    /// A checkbox for a switch (a section's title when `strong`).
    fn check(&mut self, ui: &mut egui::Ui, pid: &str, label: &str, strong: bool) -> bool {
        let on = self.on(pid);
        let font = FontId::proportional(if strong { 14.0 } else { 13.0 });
        let galley = ui.painter().layout_no_wrap(label.to_string(), font, LABEL);
        let (r, resp) = ui.allocate_exact_size(vec2(22.0 + galley.size().x, 20.0f32.max(galley.size().y)), Sense::click());
        let b = Rect::from_min_size(pos2(r.min.x + 1.0, r.center().y - 7.0), vec2(14.0, 14.0));
        let p = ui.painter();
        if on {
            p.rect_filled(b, 2.0, Color32::from_gray(0xd8));
            p.line_segment([b.min + vec2(3.0, 7.5), b.min + vec2(6.0, 10.5)], Stroke::new(2.0, BG));
            p.line_segment([b.min + vec2(6.0, 10.5), b.min + vec2(11.0, 3.5)], Stroke::new(2.0, BG));
        } else {
            p.rect_stroke(b, 2.0, Stroke::new(1.4, if resp.hovered() { Color32::WHITE } else { KNOB }), StrokeKind::Inside);
        }
        p.galley(pos2(b.max.x + 7.0, r.center().y - galley.size().y / 2.0), galley, LABEL);
        self.auto(format!("fxEditor.{}.toggle.{pid}", self.fx), r, label);
        if resp.clicked() {
            self.commit(pid, if on { 0.0 } else { 1.0 });
        }
        on
    }

    /// A choice: radio buttons for a few options, a menu for more.
    fn choice(&mut self, ui: &mut egui::Ui, pid: &str, label: &str) {
        let Some(ParamKind::Choice(opts)) = self.def(pid).map(|p| &p.kind) else { return };
        let cur = self.v(pid).max(0.0).round() as usize;
        if opts.len() <= 3 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                for (i, o) in opts.iter().enumerate() {
                    let galley = ui.painter().layout_no_wrap(o.to_string(), FontId::proportional(13.5), LABEL);
                    let (r, resp) = ui.allocate_exact_size(vec2(24.0 + galley.size().x, 22.0), Sense::click());
                    let c = pos2(r.min.x + 9.0, r.center().y);
                    let p = ui.painter();
                    p.circle_stroke(c, 7.5, Stroke::new(1.5, if resp.hovered() { Color32::WHITE } else { KNOB }));
                    if i == cur {
                        p.circle_filled(c, 4.0, Color32::from_gray(0xe0));
                    }
                    p.galley(pos2(c.x + 14.0, r.center().y - galley.size().y / 2.0), galley, LABEL);
                    self.auto(format!("fxEditor.{}.choice.{pid}.{i}", self.fx), r, o);
                    if resp.clicked() && i != cur {
                        self.commit(pid, i as f64);
                    }
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{label}:")).color(LABEL).size(13.0));
                let mut sel = cur;
                let r = egui::ComboBox::from_id_salt(("fx-choice", self.target.clone(), pid))
                    .width(170.0)
                    .selected_text(opts.get(cur).copied().unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for (i, o) in opts.iter().enumerate() {
                            ui.selectable_value(&mut sel, i, *o);
                        }
                    });
                self.auto(format!("fxEditor.{}.choice.{pid}", self.fx), r.response.rect, label);
                if sel != cur {
                    self.commit(pid, sel as f64);
                }
            });
        }
    }

    /// A section title that switches it (when the effect has a switch for it).
    fn title(&mut self, ui: &mut egui::Ui, pid: Option<&str>, label: &str) {
        ui.vertical_centered(|ui| match pid {
            Some(pid) => {
                self.check(ui, pid, label, true);
            }
            None => {
                ui.label(egui::RichText::new(label).color(LABEL).size(14.0));
            }
        });
        ui.add_space(6.0);
    }
}

/// Major and minor tick spacing for a slider scale.
fn ticks(lo: f64, hi: f64) -> (f64, f64) {
    let span = (hi - lo).abs().max(1e-9);
    // at most five labelled intervals: Premiere labels −100 … 0 every 20, −100 … 50 every 50
    let major = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0]
        .into_iter()
        .find(|m| span / m <= 5.0 + 1e-9)
        .unwrap_or(10_000.0);
    let minor = if (major / 5.0).fract() == 0.0 || major < 5.0 { major / 5.0 } else { major / 2.0 };
    (major, minor.max(major / 10.0))
}

fn fmt_tick(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

/// Decimals to show `v` with: none for whole numbers ("1 ms", "-20 dB", as Premiere shows them).
fn shown_decimals(v: f64, dec: usize) -> usize {
    if (v - v.round()).abs() < 0.5 * 10f64.powi(-(dec as i32)) { 0 } else { dec }
}

/// The live gain reduction of every metered track effect, decaying back to zero (20 dB/s) once
/// the mix stops reporting it. Read once per frame.
fn live_meters(app: &FilmcraftApp, ctx: &egui::Context) -> HashMap<(u64, usize), [f32; 4]> {
    let id = egui::Id::new("fx-editor-meters");
    let frame = ctx.cumulative_frame_nr();
    let (last, mut st): (u64, HashMap<(u64, usize), [f32; 4]>) = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if last == frame && frame != 0 {
        return st;
    }
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    for v in st.values_mut() {
        for m in v.iter_mut() {
            *m = (*m + 20.0 * dt).min(0.0);
        }
    }
    for ((strip, slot), m) in app.session.previews.live.take_fx_meters() {
        let e = st.entry((strip.0, slot)).or_insert([0.0; 4]);
        for (a, b) in e.iter_mut().zip(m) {
            *a = a.min(b);
        }
    }
    st.retain(|_, v| v.iter().any(|x| *x < -0.01));
    if !st.is_empty() {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, (frame, st.clone())));
    st
}

/// Whether `inst`'s values at `mt` are those of `values` (a preset's or the defaults).
fn same_settings(inst: &EffectInstance, mt: Tick, values: &EffectInstance) -> bool {
    let Some(def) = inst.def() else { return false };
    def.params.iter().all(|p| {
        let a = inst.param(p.id).map(|x| x.value_at(mt)).unwrap_or_else(|| p.default.clone());
        let b = values.param(p.id).map(|x| x.value.clone()).unwrap_or_else(|| p.default.clone());
        match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-6,
            _ => a == b,
        }
    })
}

/// Show every open editor window.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    // while a knob, slider or point is dragged, one more frame shows where the last pointer move
    // put it (egui draws only on input, so the window was left a move behind when the mouse stopped)
    if !app.ui.audio_fx_editors.is_empty() && ctx.dragged_id().is_some() {
        ctx.request_repaint();
    }
    let meters = live_meters(app, ctx);
    let targets = app.ui.audio_fx_editors.clone();
    let mut close = Vec::new();
    for target in targets {
        let Some(res) = resolve(app, &target) else {
            close.push(target);
            continue;
        };
        // the window belongs to the effect it was opened on, not to its slot
        let opened_on = app.ui.audio_fx_editor_effects.entry(key(&target)).or_insert_with(|| res.inst.effect.clone());
        if !has_editor(&res.inst.effect) || *opened_on != res.inst.effect {
            close.push(target);
            continue;
        }
        let name = res.inst.def().map(|d| d.name).unwrap_or("Effect");
        let kind = if matches!(target, FxTarget::Clip { .. }) { "Clip Fx Editor" } else { "Track Fx Editor" };
        let title = format!("{kind} - {name}: {}", res.place);
        let k = key(&target);
        let drafts_id = egui::Id::new(("fx-editor-drafts", &k));
        let drafts: HashMap<String, f64> = ctx.data(|d| d.get_temp(drafts_id)).unwrap_or_default();
        let fx_id = res.inst.effect.clone();
        let enabled = res.inst.enabled;
        // presets of this effect alone, and which one the current settings are
        let presets: Vec<(String, EffectInstance)> = app
            .session
            .presets
            .all()
            .into_iter()
            .filter_map(|p| match p.effects.as_slice() {
                [e] if e.effect == fx_id => Some((p.name.clone(), e.clone())),
                _ => None,
            })
            .collect();
        let user: Vec<String> = app.session.presets.user.iter().map(|p| p.name.clone()).collect();
        let current = if res.inst.def().is_some_and(|d| same_settings(&res.inst, res.mt, &d.instance())) {
            DEFAULT_SETTINGS.to_string()
        } else {
            presets.iter().find(|(_, e)| same_settings(&res.inst, res.mt, e)).map(|(n, _)| n.clone()).unwrap_or_else(|| "(Custom)".into())
        };
        let live = match &target {
            FxTarget::Insert { strip, slot } => meters.get(&(*strip, *slot)).copied().unwrap_or([0.0; 4]),
            FxTarget::Clip { .. } => [0.0; 4],
        };
        let mut ed = Ed {
            t: app.tokens,
            fx: &fx_id,
            target: &target,
            inst: res.inst,
            mt: res.mt,
            drafts,
            acts: Vec::new(),
            autos: Vec::new(),
            meters: live,
            cell: CELL,
        };
        let mut closed = false;
        let frame = egui::Frame::new().fill(BG).stroke(Stroke::new(1.0, LINE)).corner_radius(3.0).shadow(ctx.global_style().visuals.window_shadow);
        egui::Window::new(title.clone()).id(egui::Id::new(("fx-editor", &k))).title_bar(false).collapsible(false).resizable(false).frame(frame).show(
            ctx,
            |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.set_min_width(body_width(ed.fx));
                title_bar(ui, &mut ed, &title, &mut closed);
                header(ui, &mut ed, enabled, &presets, &user, &current);
                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| match ed.fx {
                    "parametric_eq" => parametric(ui, &mut ed),
                    "graphic_eq" | "graphic_eq_20" | "graphic_eq_30" => graphic(ui, &mut ed),
                    "multiband_compressor" => multiband(ui, &mut ed),
                    "dynamics_rack" => dynamics(ui, &mut ed),
                    "hard_limiter" => hard_limiter(ui, &mut ed),
                    _ => generic(ui, &mut ed),
                });
                footer(ui, &channel_text(res.channels));
            },
        );
        // drafts that were committed are dropped once the project shows them
        let committed: Vec<String> = ed
            .acts
            .iter()
            .filter_map(|(_, p)| {
                p.get("param")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| p.get("params").and_then(Value::as_object).and_then(|o| o.keys().next().cloned()))
            })
            .collect();
        let mut keep = ed.drafts.clone();
        for c in &committed {
            keep.remove(c);
        }
        if ed.acts.iter().any(|(c, _)| c.starts_with("presets.")) {
            keep.clear();
        }
        ctx.data_mut(|d| d.insert_temp(drafts_id, keep));
        for (id, r, label) in std::mem::take(&mut ed.autos) {
            app.auto.add(&id, r, &label);
        }
        for (cmd, p) in std::mem::take(&mut ed.acts) {
            if let Err(e) = app.session.execute(&cmd, p) {
                app.ui.status = e.to_string();
            }
        }
        if closed {
            close.push(target);
        }
    }
    app.ui.audio_fx_editors.retain(|t| !close.contains(t));
    let open: std::collections::HashSet<String> = app.ui.audio_fx_editors.iter().map(key).collect();
    app.ui.audio_fx_editor_effects.retain(|k, _| open.contains(k));
}

/// Width of an editor's body.
fn body_width(fx: &str) -> f32 {
    match fx {
        "dynamics_rack" => 690.0,
        "hard_limiter" => 600.0,
        "parametric_eq" => 760.0,
        "multiband_compressor" => 640.0,
        _ => 560.0,
    }
}

/// The title bar: the window's name and its close button (the window moves by its title).
fn title_bar(ui: &mut egui::Ui, ed: &mut Ed, title: &str, closed: &mut bool) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    ui.painter().rect_filled(r, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, TITLE_BG);
    let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x - 40.0, r.max.y)));
    clip.text(pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER, title, FontId::proportional(14.0), Color32::from_gray(0xe6));
    let xr = Rect::from_min_max(pos2(r.max.x - 36.0, r.min.y + 4.0), pos2(r.max.x - 6.0, r.max.y - 4.0));
    let resp = ui.interact(xr, egui::Id::new(("fx-editor-close", ed.target.clone())), Sense::click()).on_hover_text("Close");
    if resp.hovered() {
        ui.painter().rect_filled(xr, 2.0, RED);
    }
    icons::paint(ui.painter(), xr.shrink(8.0), Icon::Close, Color32::from_gray(0xe6));
    ed.auto(format!("fxEditor.{}.close", ed.fx), xr, "Close");
    if resp.clicked() {
        *closed = true;
    }
}

/// The header: bypass (power) button, the Presets menu, save and delete.
fn header(ui: &mut egui::Ui, ed: &mut Ed, enabled: bool, presets: &[(String, EffectInstance)], user: &[String], current: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, LINE));
    // power button: lit when the effect plays, dark and struck when bypassed
    let pc = pos2(r.min.x + 22.0, r.center().y);
    let pr = Rect::from_center_size(pc, vec2(24.0, 24.0));
    let presp = ui.interact(pr, egui::Id::new(("fx-editor-power", ed.target.clone())), Sense::click()).on_hover_text(if enabled { "Bypass" } else { "Enable" });
    let col = if !enabled {
        RED
    } else if presp.hovered() {
        Color32::WHITE
    } else {
        KNOB
    };
    let p = ui.painter();
    let pts: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = (40.0 + 280.0 * i as f32 / 24.0).to_radians();
            pc + vec2(a.sin(), -a.cos()) * 7.5
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.6, col)));
    p.line_segment([pc - vec2(0.0, 9.0), pc - vec2(0.0, 2.0)], Stroke::new(1.6, col));
    ed.auto(format!("fxEditor.{}.bypass", ed.fx), pr, if enabled { "Bypass" } else { "Enable" });
    if presp.clicked() {
        let c = bypass_cmd(ed.target, enabled);
        ed.acts.push(c);
    }
    p.text(pos2(r.min.x + 48.0, r.center().y), Align2::LEFT_CENTER, "Presets:", FontId::proportional(13.5), LABEL);
    // saving: a name field in place of the menu
    let naming_id = egui::Id::new(("fx-preset-name", ed.target.clone()));
    let mut naming: Option<String> = ui.data(|d| d.get_temp(naming_id));
    let field = Rect::from_min_max(pos2(r.min.x + 112.0, r.center().y - 12.0), pos2(r.max.x - 70.0, r.center().y + 12.0));
    let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if let Some(buf) = naming.as_mut() {
        let te = fui.add(egui::TextEdit::singleline(buf).hint_text("Preset name").desired_width(field.width()));
        ed.auto(format!("fxEditor.{}.presets.name", ed.fx), te.rect, "Preset name");
        if te.lost_focus() {
            let name = buf.trim().to_string();
            if fui.input(|i| i.key_pressed(egui::Key::Enter)) && !name.is_empty() {
                let mut pa = preset_target(ed.target);
                pa["name"] = json!(name);
                ed.acts.push(("presets.saveSettings".into(), pa));
            }
            naming = None;
        } else {
            te.request_focus();
        }
    } else {
        let resp = egui::ComboBox::from_id_salt(("fx-presets", ed.target.clone())).width(field.width() - 8.0).selected_text(current).show_ui(&mut fui, |ui| {
            let names = std::iter::once(DEFAULT_SETTINGS.to_string()).chain(presets.iter().map(|(n, _)| n.clone()));
            for n in names {
                let r = ui.selectable_label(n == current, &n);
                ed.auto(format!("fxEditor.{}.presets.{n}", ed.fx), r.rect, &n);
                if r.clicked() {
                    let mut pa = preset_target(ed.target);
                    pa["preset"] = json!(n);
                    ed.acts.push(("presets.applySettings".into(), pa));
                }
            }
        });
        ed.auto(format!("fxEditor.{}.presets", ed.fx), resp.response.rect, "Presets");
    }
    // save the current settings as a preset; delete the chosen one (your presets only)
    let sr = Rect::from_center_size(pos2(r.max.x - 50.0, r.center().y), vec2(24.0, 24.0));
    let sresp = ui.interact(sr, egui::Id::new(("fx-preset-save", ed.target.clone())), Sense::click()).on_hover_text("Save Preset…");
    icons::paint(ui.painter(), sr.shrink(4.0), Icon::NewItem, if sresp.hovered() { Color32::WHITE } else { KNOB });
    ed.auto(format!("fxEditor.{}.presets.save", ed.fx), sr, "Save Preset…");
    if sresp.clicked() {
        naming = Some(String::new());
    }
    let can_delete = user.iter().any(|u| u == current);
    let dr = Rect::from_center_size(pos2(r.max.x - 22.0, r.center().y), vec2(24.0, 24.0));
    let dresp = ui.interact(dr, egui::Id::new(("fx-preset-delete", ed.target.clone())), Sense::click()).on_hover_text("Delete Preset");
    icons::paint(
        ui.painter(),
        dr.shrink(4.0),
        Icon::Trash,
        if !can_delete {
            Color32::from_gray(0x55)
        } else if dresp.hovered() {
            Color32::WHITE
        } else {
            KNOB
        },
    );
    ed.auto(format!("fxEditor.{}.presets.delete", ed.fx), dr, "Delete Preset");
    if dresp.clicked() && can_delete {
        ed.acts.push(("presets.delete".into(), json!({"name": current})));
    }
    ui.data_mut(|d| match naming {
        Some(n) => {
            d.insert_temp(naming_id, n);
        }
        None => d.remove::<String>(naming_id),
    });
}

/// The footer: the channels the effect processes.
fn footer(ui: &mut egui::Ui, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, LINE));
    ui.painter().text(pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), LABEL);
}

/// A vertical separator between sections.
fn vline(ui: &mut egui::Ui, h: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, h), Sense::hover());
    ui.painter().line_segment([pos2(r.center().x, r.min.y), pos2(r.center().x, r.max.y)], Stroke::new(1.0, LINE));
}

// ------------------------------------------------------------------------------- meters

/// A gain-reduction meter: thin segments over 40 dB, lit from the 0 dB end as the stage reduces
/// the level, with its dB scale under it.
fn gr_meter(ui: &mut egui::Ui, ed: &mut Ed, gr_db: f32, stage: &str) {
    let (cell, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
    let bar = Rect::from_center_size(pos2(cell.center().x, cell.min.y + 9.0), vec2(170.0, 18.0));
    let p = ui.painter();
    p.rect_filled(bar, 1.0, LED_OFF);
    let n = 40;
    let w = (bar.width() - 4.0) / n as f32;
    let lit = (-gr_db).clamp(0.0, 40.0);
    for i in 0..n {
        // segment i covers −(n − i) … −(n − i − 1) dB
        let db_left = -((n - i) as f32);
        let x = bar.min.x + 2.0 + i as f32 * w;
        let seg = Rect::from_min_max(pos2(x, bar.min.y + 2.0), pos2(x + w - 1.2, bar.max.y - 2.0));
        let col = if i == n - 1 {
            RED
        } else if -db_left <= lit + 1e-3 && lit > 0.05 {
            AMBER
        } else {
            SEG_OFF
        };
        p.rect_filled(seg, 0.0, col);
    }
    let x_of = |db: f32| bar.max.x - 2.0 - (-db / 40.0) * (bar.width() - 4.0);
    for (db, label) in [(-34.0, "dB"), (-24.0, "-24"), (-12.0, "-12"), (0.0, "0")] {
        p.text(pos2(x_of(db), bar.max.y + 3.0), Align2::CENTER_TOP, label, FontId::proportional(11.5), LABEL);
    }
    for db in [-36.0f32, -30.0, -24.0, -18.0, -12.0, -6.0, 0.0] {
        p.line_segment([pos2(x_of(db), bar.max.y), pos2(x_of(db), bar.max.y + 2.0)], Stroke::new(1.0, DIM));
    }
    ed.auto(format!("fxEditor.{}.meter.{stage}", ed.fx), bar, &format!("{stage} gain reduction {gr_db:.1} dB"));
}

/// The AutoGate lights: open (green), closing (amber), closed (red).
fn gate_lights(ui: &mut egui::Ui, ed: &mut Ed, gr_db: f32, on: bool) {
    let (cell, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
    let state = if !on {
        None
    } else if gr_db > -1.0 {
        Some(0)
    } else if gr_db > -79.0 {
        Some(1)
    } else {
        Some(2)
    };
    let x0 = cell.center().x - 63.0;
    for (i, col) in [GREEN, AMBER, RED].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * 43.0, cell.min.y), vec2(40.0, 18.0));
        ui.painter().rect_filled(r, 1.0, if state == Some(i) { col } else { LED_OFF });
    }
    let label = match state {
        Some(0) => "open",
        Some(1) => "closing",
        Some(2) => "closed",
        _ => "off",
    };
    ed.auto(format!("fxEditor.{}.meter.gate", ed.fx), Rect::from_min_size(pos2(x0, cell.min.y), vec2(126.0, 18.0)), &format!("Gate {label}"));
}

/// The Limit light: red while the limiter reduces the level.
fn limit_light(ui: &mut egui::Ui, ed: &mut Ed, gr_db: f32) {
    let (cell, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let r = Rect::from_min_size(pos2(cell.center().x - 36.0, cell.min.y), vec2(34.0, 18.0));
    let on = gr_db < -0.1;
    ui.painter().rect_filled(r, 1.0, if on { RED } else { LED_OFF });
    ui.painter().text(pos2(r.max.x + 10.0, r.center().y), Align2::LEFT_CENTER, "Limit", FontId::proportional(13.5), LABEL);
    ed.auto(format!("fxEditor.{}.meter.lim", ed.fx), r, if on { "Limiting" } else { "Not limiting" });
}

// ------------------------------------------------------------------------------- Dynamics

fn dynamics(ui: &mut egui::Ui, ed: &mut Ed) {
    let m = ed.meters;
    let col_w = 2.0 * CELL.x + 4.0;
    let h = 470.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.vertical(|ui| {
            ui.set_width(col_w);
            ed.title(ui, Some("gate_on"), "AutoGate");
            let on = ed.on("gate_on");
            gate_lights(ui, ed, m[0], on);
            ed.knobs(ui, &[("gate_threshold", "Threshold"), ("gate_attack", "Attack")]);
            ed.knobs(ui, &[("gate_release", "Release"), ("gate_hold", "Hold")]);
        });
        vline(ui, h);
        ui.vertical(|ui| {
            ui.set_width(col_w);
            ed.title(ui, Some("comp_on"), "Compressor");
            gr_meter(ui, ed, m[1], "comp");
            ed.knobs(ui, &[("comp_threshold", "Threshold"), ("comp_ratio", "Ratio")]);
            ed.knobs(ui, &[("comp_attack", "Attack"), ("comp_release", "Release")]);
            ed.knobs(ui, &[("comp_makeup", "MakeUp")]);
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ed.check(ui, "comp_auto", "Auto MakeUp", false);
            });
        });
        vline(ui, h);
        ui.vertical(|ui| {
            ui.set_width(col_w);
            ed.title(ui, Some("exp_on"), "Expander");
            gr_meter(ui, ed, m[2], "exp");
            ed.knobs(ui, &[("exp_threshold", "Threshold"), ("exp_ratio", "Ratio")]);
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(vec2(col_w, 12.0), Sense::hover());
            ui.painter().line_segment([pos2(r.min.x + 8.0, r.center().y), pos2(r.max.x - 8.0, r.center().y)], Stroke::new(1.0, LINE));
            ed.title(ui, Some("lim_on"), "Limiter");
            limit_light(ui, ed, m[3]);
            ed.knobs(ui, &[("lim_threshold", "Threshold"), ("lim_release", "Release")]);
            ed.knobs(ui, &[("output", "Output")]);
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ed.check(ui, "soft_clip", "Soft Clip", false);
            });
        });
    });
}

// --------------------------------------------------------------------------- Hard Limiter

fn hard_limiter(ui: &mut egui::Ui, ed: &mut Ed) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ed.choice(ui, "detect", "Detection");
        ui.add_space(30.0);
        // the Limit light: lit while it reduces the level
        let on = ed.meters[0] < -0.1;
        let (r, _) = ui.allocate_exact_size(vec2(34.0, 18.0), Sense::hover());
        ui.painter().rect_filled(r, 1.0, if on { RED } else { LED_OFF });
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Limit").color(LABEL).size(13.5));
        ed.auto(format!("fxEditor.{}.meter.lim", ed.fx), r, if on { "Limiting" } else { "Not limiting" });
    });
    ui.add_space(6.0);
    ed.slider_row(ui, "max", "Maximum Amplitude");
    ed.slider_row(ui, "boost", "Input Boost");
    ed.slider_row(ui, "lookahead", "Look-Ahead Time");
    ed.slider_row(ui, "release", "Release Time");
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ed.check(ui, "link", "Link Channels", false);
    });
}

// ------------------------------------------------------------------------- any other effect

/// The effect's parameters laid out like Premiere's own editors: scaled sliders for a few
/// numbers, sections of knobs (one per parameter group) for many; switches as checkboxes,
/// choices as radio buttons or menus.
fn generic(ui: &mut egui::Ui, ed: &mut Ed) {
    let Some(def) = ed.inst.def() else { return };
    let params: Vec<&'static ParamDef> = def.params.iter().collect();
    if params.is_empty() {
        ui.label(egui::RichText::new("This effect has no settings.").color(LABEL).size(13.5));
        return;
    }
    let floats = params.iter().filter(|p| matches!(p.kind, ParamKind::Float { .. })).count();
    let grouped = params.iter().any(|p| p.group.is_some());
    if floats <= 5 && !grouped {
        for p in &params {
            if let ParamKind::Choice(_) = p.kind {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ed.choice(ui, p.id, p.label);
                });
                ui.add_space(6.0);
            }
        }
        for p in params.iter().filter(|p| matches!(p.kind, ParamKind::Float { .. })) {
            ed.slider_row(ui, p.id, p.label);
        }
        let bools: Vec<_> = params.iter().filter(|p| p.kind == ParamKind::Bool).collect();
        if !bools.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                for p in bools {
                    ed.check(ui, p.id, p.label, false);
                    ui.add_space(18.0);
                }
            });
        }
        return;
    }
    // sections: the ungrouped parameters, then each group in order
    let mut groups: Vec<Option<&'static str>> = Vec::new();
    for p in &params {
        if !groups.contains(&p.group) {
            groups.push(p.group);
        }
    }
    for (gi, g) in groups.iter().enumerate() {
        let ps: Vec<&&'static ParamDef> = params.iter().filter(|p| p.group == *g).collect();
        // a group's own switch ("On") titles it
        let switch = ps.iter().find(|p| p.kind == ParamKind::Bool && (p.label == "On" || Some(p.label) == *g)).map(|p| p.id);
        if gi > 0 {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
            ui.painter().line_segment([pos2(r.min.x, r.center().y), pos2(r.max.x, r.center().y)], Stroke::new(1.0, LINE));
        }
        if let Some(name) = g {
            ed.title(ui, switch, name);
        }
        for p in ps.iter().filter(|p| matches!(p.kind, ParamKind::Choice(_))) {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ed.choice(ui, p.id, p.label);
            });
            ui.add_space(6.0);
        }
        let knobs: Vec<(&str, &str)> = ps.iter().filter(|p| matches!(p.kind, ParamKind::Float { .. })).map(|p| (p.id, p.label)).collect();
        for row in knobs.chunks(5) {
            ed.knobs(ui, row);
        }
        let bools: Vec<_> = ps.iter().filter(|p| p.kind == ParamKind::Bool && Some(p.id) != switch).collect();
        if !bools.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                for p in bools {
                    ed.check(ui, p.id, p.label, false);
                    ui.add_space(18.0);
                }
            });
        }
    }
}

// ------------------------------------------------------------------------------- plotting

const F_MIN: f64 = 20.0;
const F_MAX: f64 = 24000.0;

fn fx_of(x: f32, r: Rect) -> f64 {
    let u = ((x - r.min.x) / r.width()).clamp(0.0, 1.0) as f64;
    F_MIN * (F_MAX / F_MIN).powf(u)
}
fn x_of(f: f64, r: Rect) -> f32 {
    r.min.x + ((f.clamp(F_MIN, F_MAX) / F_MIN).ln() / (F_MAX / F_MIN).ln()) as f32 * r.width()
}
fn y_of(db: f64, r: Rect, range: f64) -> f32 {
    r.center().y - (db.clamp(-range, range) / range) as f32 * r.height() * 0.5
}
fn db_of(y: f32, r: Rect, range: f64) -> f64 {
    ((r.center().y - y) / (r.height() * 0.5)) as f64 * range
}

/// Frequency-response plot with a log-frequency grid; returns the plot rect and response.
fn response_plot(ui: &mut egui::Ui, ed: &mut Ed, size: egui::Vec2, range: f64) -> (Rect, egui::Response) {
    let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, Color32::from_gray(0x16));
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = x_of(f, r);
        p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, Color32::from_gray(0x2c)));
        let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(pos2(x + 2.0, r.max.y - 2.0), Align2::LEFT_BOTTOM, label, Tokens::ui(9.5), DIM);
    }
    let step = if range > 30.0 { 12.0 } else { 6.0 };
    let mut d = -range;
    while d <= range + 1e-9 {
        let y = y_of(d, r, range);
        let col = if d.abs() < 1e-9 { Color32::from_gray(0x48) } else { Color32::from_gray(0x2c) };
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, col));
        p.text(pos2(r.min.x + 2.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{d:+.0}"), Tokens::ui(9.5), DIM);
        d += step;
    }
    // curve from the DSP
    if let Some(dsp) = filmcraft_render::audio_fx::configured(&ed.preview(), ed.mt, 48000) {
        let pts: Vec<Pos2> = (0..=240)
            .filter_map(|i| {
                let x = r.min.x + r.width() * i as f32 / 240.0;
                dsp.response_db(fx_of(x, r)).map(|db| pos2(x, y_of(db, r, range)))
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, ed.t.hot_text)));
    }
    ed.auto(format!("fxEditor.{}.plot", ed.fx), r, "Response");
    (r, resp)
}

// ------------------------------------------------------------------------- parametric EQ
//
// Premiere's layout (measured in Premiere 26.5): a log-frequency graph (20 Hz – 24 kHz, bold lines
// at 100 Hz / 1 kHz / 10 kHz) over ±48 dB (Range 96 dB) or ±15 dB (Range 30 dB), a square point
// per band that is on (HP / LP only when on, and only horizontally), then a table: Frequency,
// Gain (the pass filters' slope menus), Q / Width (the shelves' slope switch) and the Band
// buttons. A point stays under the cursor where it was grabbed, the curve and the numbers follow
// live, and the point stops at the graph's edges. Q changes in its number only (scrub or type);
// the master Gain is not part of the curve.

/// (node label, parameter prefix, kind)
const PEQ_NODES: [(&str, &str, PeqKind); 9] = [
    ("HP", "hp", PeqKind::Pass),
    ("L", "low", PeqKind::Shelf),
    ("1", "b1", PeqKind::Band),
    ("2", "b2", PeqKind::Band),
    ("3", "mid", PeqKind::Band),
    ("4", "b4", PeqKind::Band),
    ("5", "b5", PeqKind::Band),
    ("H", "high", PeqKind::Shelf),
    ("LP", "lp", PeqKind::Pass),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum PeqKind {
    Pass,
    Shelf,
    Band,
}

const PEQ_BLUE: Color32 = Color32::from_rgb(0x55, 0x6e, 0xff);
const PEQ_LABEL_W: f32 = 92.0;

/// The graph's half height in dB for Range 96 dB (default) or 30 dB.
fn peq_half_range(range_30: bool) -> f64 {
    if range_30 { 15.0 } else { 48.0 }
}

/// Where a dragged point goes: the pointer less the grab offset, held inside the graph.
fn peq_drag_target(pointer: Pos2, grab: egui::Vec2, r: Rect, half: f64) -> (f64, f64) {
    let p = pointer - grab;
    let f = fx_of(p.x, r);
    let db = db_of(p.y.clamp(r.min.y, r.max.y), r, half).clamp(-half, half);
    (f, db)
}

fn parametric(ui: &mut egui::Ui, ed: &mut Ed) {
    let k = key(ed.target);
    let range_id = egui::Id::new(("peq-range30", &k));
    let sel_id = egui::Id::new(("peq-selected", &k));
    let grab_id = egui::Id::new(("peq-grab", &k));
    let range_30: bool = ui.ctx().data(|d| d.get_temp(range_id)).unwrap_or(false);
    let mut selected: Option<String> = ui.ctx().data(|d| d.get_temp(sel_id));
    let mut grab: egui::Vec2 = ui.ctx().data(|d| d.get_temp(grab_id)).unwrap_or_default();
    let half = peq_half_range(range_30);

    let (r, plot_resp) = ui.allocate_exact_size(vec2(body_width(ed.fx) - 28.0, 300.0), Sense::click());
    ed.auto(format!("fxEditor.{}.plot", ed.fx), r, "Response");
    if plot_resp.clicked() || plot_resp.double_clicked() {
        selected = None;
    }
    // 1. the points take the pointer first, so the curve below shows this frame's values
    let mut dragging: Option<(Pos2, &str)> = None;
    for (_, pre, kind) in PEQ_NODES {
        let on_id = format!("{pre}_on");
        if !ed.on(&on_id) {
            continue;
        }
        let (f_id, g_id) = (format!("{pre}_freq"), format!("{pre}_gain"));
        let g = if kind == PeqKind::Pass { 0.0 } else { ed.v(&g_id) };
        let c = pos2(x_of(ed.v(&f_id), r), y_of(g, r, half));
        let nr = Rect::from_center_size(c, vec2(16.0, 16.0));
        let resp = ui.interact(nr, egui::Id::new(("peq-node", ed.target.clone(), pre)), Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            selected = Some(pre.to_string());
        }
        // the offset from where the button went down: egui starts a drag only after the pointer
        // has moved a few pixels, and measuring from there left the point trailing the pointer
        if resp.drag_started()
            && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
        {
            grab = pos - c;
        }
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let (f, db) = peq_drag_target(pos, grab, r, half);
            let (lo, hi, _) = ed.range(&f_id);
            ed.drafts.insert(f_id.clone(), ((f * 10.0).round() / 10.0).clamp(lo, hi));
            if kind != PeqKind::Pass {
                let (lo, hi, _) = ed.range(&g_id);
                ed.drafts.insert(g_id.clone(), ((db * 10.0).round() / 10.0).clamp(lo, hi));
            }
            dragging = Some((pos, pre));
            // one more frame shows where the last pointer move put the point (egui draws only on
            // input, and the graph was left a move behind when the mouse stopped)
            ui.ctx().request_repaint();
        }
        if resp.drag_stopped() {
            let fv = ed.v(&f_id);
            ed.commit(&f_id, fv);
            if kind != PeqKind::Pass {
                let gv = ed.v(&g_id);
                ed.commit(&g_id, gv);
            }
            ui.ctx().request_repaint();
        }
    }
    // 2. the graph, the curve and the points
    peq_grid(ui, r, half);
    let mut shown = ed.preview();
    // the master Gain is not part of Premiere's curve
    shown.params.remove("master_gain");
    if let Some(dsp) = filmcraft_render::audio_fx::configured(&shown, ed.mt, 48000) {
        let pts: Vec<Pos2> = (0..=320)
            .filter_map(|i| {
                let x = r.min.x + r.width() * i as f32 / 320.0;
                dsp.response_db(fx_of(x, r)).map(|db| pos2(x, y_of(db, r, half)))
            })
            .collect();
        ui.painter_at(r).add(egui::Shape::line(pts, Stroke::new(1.6, PEQ_BLUE)));
    }
    for (label, pre, kind) in PEQ_NODES {
        if !ed.on(&format!("{pre}_on")) {
            continue;
        }
        let g = if kind == PeqKind::Pass { 0.0 } else { ed.v(&format!("{pre}_gain")) };
        let c = pos2(x_of(ed.v(&format!("{pre}_freq")), r), y_of(g, r, half));
        let sel = selected.as_deref() == Some(pre);
        let sq = Rect::from_center_size(c, vec2(if sel { 10.0 } else { 7.0 }, if sel { 10.0 } else { 7.0 }));
        let p = ui.painter_at(r.expand(8.0));
        p.rect_filled(sq, 1.0, if sel { PEQ_BLUE } else { Color32::from_rgb(0xe8, 0xf4, 0xff) });
        if sel {
            p.rect_stroke(sq, 1.0, Stroke::new(1.2, Color32::WHITE), StrokeKind::Outside);
        }
        p.text(pos2(c.x, c.y - 9.0), Align2::CENTER_BOTTOM, label, Tokens::ui(11.0), Color32::WHITE);
        ed.auto(format!("fxEditor.{}.node.{pre}", ed.fx), Rect::from_center_size(c, vec2(16.0, 16.0)), label);
    }
    if let Some((pos, pre)) = dragging {
        let f = ed.v(&format!("{pre}_freq"));
        let g = if pre == "hp" || pre == "lp" { 0.0 } else { ed.v(&format!("{pre}_gain")) };
        let tip = format!("{f:.1} Hz --> {g:.1} dB");
        let at = pos2(pos.x.clamp(r.min.x + 60.0, r.max.x - 60.0), (pos.y + 22.0).clamp(r.min.y + 10.0, r.max.y - 4.0));
        let galley = ui.painter().layout_no_wrap(tip, Tokens::ui(11.0), Color32::BLACK);
        let tr = Rect::from_center_size(at, galley.size() + vec2(8.0, 4.0));
        ui.painter().rect_filled(tr, 2.0, Color32::from_rgb(0xf4, 0xf4, 0xe8));
        ui.painter().galley(tr.min + vec2(4.0, 2.0), galley, Color32::BLACK);
    }
    ui.add_space(12.0);
    peq_table(ui, ed);
    ui.add_space(8.0);
    // Range (a view setting, like Premiere's)
    ui.horizontal(|ui| {
        ui.add_space(PEQ_LABEL_W);
        ui.label(egui::RichText::new("Range:").color(LABEL).size(13.0));
        for (val, text, id) in [(true, "30 dB", "30"), (false, "96 dB", "96")] {
            let rr = ui.radio(range_30 == val, egui::RichText::new(text).color(LABEL).size(13.0));
            ed.auto(format!("fxEditor.{}.range.{id}", ed.fx), rr.rect, text);
            if rr.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(range_id, val));
            }
        }
    });
    ui.add_space(4.0);
    ed.slider_row(ui, "master_gain", "Gain");
    ui.ctx().data_mut(|d| {
        d.insert_temp(grab_id, grab);
        match &selected {
            Some(s) => {
                d.insert_temp(sel_id, s.clone());
            }
            None => d.remove::<String>(sel_id),
        }
    });
}

/// Premiere's grid: thin lines at every 1…9 × decade, bold at 100 Hz, 1 kHz and 10 kHz; dB
/// lines every 2 dB (Range 96) or 1 dB (Range 30), labelled every 10 / 5 dB at the right.
fn peq_grid(ui: &mut egui::Ui, r: Rect, half: f64) {
    let p = ui.painter_at(r.expand2(vec2(30.0, 0.0)));
    p.rect_filled(r, 0.0, Color32::from_gray(0x0e));
    let mut decade = 10.0;
    while decade <= 10000.0 {
        for m in 1..=9 {
            let f = decade * m as f64;
            if !(F_MIN..=F_MAX).contains(&f) {
                continue;
            }
            let x = x_of(f, r);
            let bold = m == 1 && (100.0..=10000.0).contains(&f);
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, Color32::from_gray(if bold { 0x6a } else { 0x2a })));
            let label = match f as u32 {
                20 => Some("Hz".to_string()),
                30 | 40 | 50 | 60 | 70 | 80 | 100 | 200 | 300 | 400 | 500 | 600 | 800 => Some(format!("{}", f as u32)),
                1000 | 2000 | 3000 | 4000 | 5000 | 6000 | 7000 | 8000 | 10000 | 20000 => Some(format!("{}k", f as u32 / 1000)),
                _ => None,
            };
            if let Some(l) = label {
                let col = if bold { LABEL } else { DIM };
                p.text(pos2(x + 2.0, r.max.y - 2.0), Align2::LEFT_BOTTOM, l, Tokens::ui(9.5), col);
            }
        }
        decade *= 10.0;
    }
    let (minor, major) = if half > 20.0 { (2.0, 10.0) } else { (1.0, 5.0) };
    let mut d = -half;
    while d <= half + 1e-9 {
        let y = y_of(d, r, half);
        let is_major = (d / major).round() * major == d;
        let col = if d.abs() < 1e-9 {
            Color32::from_gray(0x60)
        } else if is_major {
            Color32::from_gray(0x34)
        } else {
            Color32::from_gray(0x1e)
        };
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, col));
        if is_major && d.abs() < half - 1e-9 {
            p.text(pos2(r.max.x - 3.0, y), Align2::RIGHT_CENTER, format!("{d:.0}"), Tokens::ui(9.5), DIM);
        }
        d += minor;
    }
    p.text(pos2(r.max.x - 3.0, r.min.y + 2.0), Align2::RIGHT_TOP, "dB", Tokens::ui(9.5), DIM);
}

/// The table under the graph: Frequency, Gain, Q / Width and Band rows, one column per band.
fn peq_table(ui: &mut egui::Ui, ed: &mut Ed) {
    let w = body_width(ed.fx) - 28.0;
    let col_w = (w - PEQ_LABEL_W) / PEQ_NODES.len() as f32;
    let row = |ui: &mut egui::Ui, label: &str| -> Rect {
        let (rr, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
        ui.painter().text(pos2(rr.min.x + PEQ_LABEL_W - 12.0, rr.center().y), Align2::RIGHT_CENTER, label, FontId::proportional(13.0), LABEL);
        rr
    };
    let cell = |rr: Rect, i: usize| Rect::from_min_size(pos2(rr.min.x + PEQ_LABEL_W + col_w * i as f32, rr.min.y + 4.0), vec2(col_w - 6.0, 20.0));
    // Frequency
    let rr = row(ui, "Frequency");
    for (i, (_, pre, _)) in PEQ_NODES.iter().enumerate() {
        ed.value_text(ui, &format!("{pre}_freq"), cell(rr, i), egui::Align::Max);
    }
    // Gain (the pass filters' slopes)
    let rr = row(ui, "Gain");
    for (i, (_, pre, kind)) in PEQ_NODES.iter().enumerate() {
        if *kind == PeqKind::Pass {
            peq_slope(ui, ed, &format!("{pre}_slope"), cell(rr, i));
        } else {
            ed.value_text(ui, &format!("{pre}_gain"), cell(rr, i), egui::Align::Max);
        }
    }
    // Q / Width (the shelves' slope switch)
    let rr = row(ui, "Q / Width");
    for (i, (_, pre, kind)) in PEQ_NODES.iter().enumerate() {
        match kind {
            PeqKind::Band => ed.value_text(ui, &format!("{pre}_q"), cell(rr, i), egui::Align::Max),
            PeqKind::Shelf => peq_shelf_switch(ui, ed, pre, cell(rr, i)),
            PeqKind::Pass => {}
        }
    }
    // Band buttons
    let rr = row(ui, "Band");
    for (i, (label, pre, _)) in PEQ_NODES.iter().enumerate() {
        let b = cell(rr, i).shrink2(vec2(4.0, 0.0));
        let pid = format!("{pre}_on");
        let resp = ui.interact(b, egui::Id::new(("peq-band", ed.target.clone(), *pre)), Sense::click());
        let on = ed.on(&pid);
        let fill = if on { Color32::from_gray(0x50) } else { Color32::from_gray(0x0c) };
        ui.painter().rect_filled(b, 3.0, fill);
        if resp.hovered() {
            ui.painter().rect_stroke(b, 3.0, Stroke::new(1.0, Color32::from_gray(0x90)), StrokeKind::Inside);
        }
        ui.painter().text(b.center(), Align2::CENTER_CENTER, *label, FontId::proportional(12.5), Color32::WHITE);
        ed.auto(format!("fxEditor.{}.toggle.{pid}", ed.fx), b, label);
        if resp.clicked() {
            ed.commit(&pid, if on { 0.0 } else { 1.0 });
        }
    }
}

/// A pass filter's slope menu (6 … 48 dB/Oct).
fn peq_slope(ui: &mut egui::Ui, ed: &mut Ed, pid: &str, r: Rect) {
    let Some(ParamKind::Choice(opts)) = ed.def(pid).map(|p| &p.kind) else { return };
    let cur = ed.v(pid).max(0.0).round() as usize;
    let mut sel = cur;
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(r));
    let resp = egui::ComboBox::from_id_salt(("peq-slope", ed.target.clone(), pid))
        .width(r.width())
        .selected_text(egui::RichText::new(opts.get(cur).copied().unwrap_or("")).size(11.5))
        .show_ui(&mut cui, |ui| {
            for (i, o) in opts.iter().enumerate() {
                ui.selectable_value(&mut sel, i, *o);
            }
        });
    ed.auto(format!("fxEditor.{}.choice.{pid}", ed.fx), resp.response.rect, pid);
    if sel != cur {
        ed.commit(pid, sel as f64);
    }
}

/// A shelf's slope switch: gentle (first order) or steep (second order), drawn as its shape.
fn peq_shelf_switch(ui: &mut egui::Ui, ed: &mut Ed, pre: &str, r: Rect) {
    let pid = format!("{pre}_steep");
    let b = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(26.0, 20.0));
    let resp = ui.interact(b, egui::Id::new(("peq-steep", ed.target.clone(), pre)), Sense::click());
    let steep = ed.on(&pid);
    if resp.hovered() {
        ui.painter().rect_stroke(b, 3.0, Stroke::new(1.0, Color32::from_gray(0x90)), StrokeKind::Inside);
    }
    // the shelf's outline: high on the boosted side, a gentle or steep step between
    let (lo_y, hi_y) = (b.center().y + 4.0, b.center().y - 4.0);
    let (x0, x3) = (b.min.x + 4.0, b.max.x - 4.0);
    let (x1, x2) = if steep { (b.center().x - 2.0, b.center().x + 2.0) } else { (b.center().x - 6.0, b.center().x + 6.0) };
    let (y_left, y_right) = if pre == "low" { (hi_y, lo_y) } else { (lo_y, hi_y) };
    let pts = vec![pos2(x0, y_left), pos2(x1, y_left), pos2(x2, y_right), pos2(x3, y_right)];
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.6, LABEL)));
    ed.auto(format!("fxEditor.{}.toggle.{pid}", ed.fx), b, if steep { "Steep" } else { "Gentle" });
    let resp = resp.on_hover_text(if steep { "Steep shelf (click: gentle)" } else { "Gentle shelf (click: steep)" });
    if resp.clicked() {
        ed.commit(&pid, if steep { 0.0 } else { 1.0 });
    }
}

// --------------------------------------------------------------------------- graphic EQ

fn graphic(ui: &mut egui::Ui, ed: &mut Ed) {
    let n = match ed.fx {
        "graphic_eq" => 10,
        "graphic_eq_20" => 20,
        _ => 30,
    };
    let labels: &[&str] = match n {
        10 => &filmcraft_project::effect::GEQ10_LABELS,
        20 => &filmcraft_project::effect::GEQ20_LABELS,
        _ => &filmcraft_project::effect::GEQ30_LABELS,
    };
    let w = (n as f32 * 26.0).max(520.0);
    response_plot(ui, ed, vec2(w, 140.0), 24.0);
    ui.add_space(6.0);
    let colw = w / n as f32;
    let (area, _) = ui.allocate_exact_size(vec2(w, 170.0), Sense::hover());
    for (i, label) in labels.iter().enumerate() {
        let pid = format!("b{}", i + 1);
        let x = area.min.x + colw * (i as f32 + 0.5);
        let track = Rect::from_center_size(pos2(x, area.min.y + 70.0), vec2(4.0, 130.0));
        ui.painter().rect_filled(track, 2.0, Color32::from_gray(0x3a));
        ui.painter().line_segment([pos2(x - 6.0, track.center().y), pos2(x + 6.0, track.center().y)], Stroke::new(1.0, DIM));
        let v = ed.v(&pid);
        let ky = track.center().y - (v / 24.0) as f32 * track.height() * 0.5;
        let knob = Rect::from_center_size(pos2(x, ky), vec2(colw.min(18.0), 8.0));
        let id = egui::Id::new(("geq-band", ed.target.clone(), i));
        let resp = ui.interact(knob.union(track).expand(3.0), id, Sense::click_and_drag());
        ui.painter().rect_filled(knob, 2.0, if resp.hovered() || resp.dragged() { Color32::WHITE } else { KNOB });
        ui.painter().text(pos2(x, area.max.y - 22.0), Align2::CENTER_CENTER, format!("{v:+.0}"), Tokens::ui(9.0), ed.t.hot_text);
        let short = label.replace(" Hz", "").replace(" kHz", "k");
        ui.painter().text(pos2(x, area.max.y - 8.0), Align2::CENTER_CENTER, short, Tokens::ui(9.0), LABEL);
        ed.auto(format!("fxEditor.{}.param.{pid}", ed.fx), track.union(knob), label);
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let db = (((track.center().y - pos.y) / (track.height() * 0.5)) as f64 * 24.0).clamp(-24.0, 24.0);
            ed.drafts.insert(pid.clone(), (db * 2.0).round() / 2.0);
        }
        if resp.drag_stopped() {
            let nv = ed.v(&pid);
            ed.commit(&pid, nv);
        }
        if resp.double_clicked() {
            ed.commit(&pid, 0.0);
        }
    }
    ed.slider_row(ui, "gain", "Master Gain");
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let r = ui.button("Reset Bands");
        ed.auto(format!("fxEditor.{}.reset", ed.fx), r.rect, "Reset");
        if r.clicked() {
            for i in 0..n {
                let pid = format!("b{}", i + 1);
                if ed.v(&pid) != 0.0 {
                    ed.commit(&pid, 0.0);
                }
            }
        }
    });
}

// --------------------------------------------------------------------- transfer curves

fn transfer_plot(ui: &mut egui::Ui, ed: &mut Ed, size: f32, band: usize, auto: String) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, Color32::from_gray(0x16));
    let lo = -60.0;
    let map = |x: f64, y: f64| pos2(r.min.x + ((x - lo) / -lo) as f32 * r.width(), r.max.y - (((y - lo) / -lo).clamp(-0.05, 1.05)) as f32 * r.height());
    for d in [-48.0, -36.0, -24.0, -12.0] {
        p.line_segment([map(d, lo), map(d, 0.0)], Stroke::new(1.0, Color32::from_gray(0x2c)));
        p.line_segment([map(lo, d), map(0.0, d)], Stroke::new(1.0, Color32::from_gray(0x2c)));
    }
    p.line_segment([map(lo, lo), map(0.0, 0.0)], Stroke::new(1.0, Color32::from_gray(0x44)));
    if let Some(dsp) = filmcraft_render::audio_fx::configured(&ed.preview(), ed.mt, 48000) {
        let pts: Vec<Pos2> = (0..=120)
            .filter_map(|i| {
                let x = lo + -lo * i as f64 / 120.0;
                dsp.transfer_db(band, x as f32).map(|y| map(x, y as f64))
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, AMBER)));
    }
    p.text(r.left_top() + vec2(3.0, 2.0), Align2::LEFT_TOP, "out", Tokens::ui(9.0), DIM);
    p.text(r.right_bottom() - vec2(3.0, 2.0), Align2::RIGHT_BOTTOM, "in", Tokens::ui(9.0), DIM);
    ed.auto(auto, r, "Transfer curve");
}

// ------------------------------------------------------------------- multiband compressor

fn multiband(ui: &mut egui::Ui, ed: &mut Ed) {
    ed.cell = vec2(80.0, 88.0);
    // Spectrum strip with draggable crossover handles.
    let (r, _) = ui.allocate_exact_size(vec2(4.0 * 2.0 * ed.cell.x, 70.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, Color32::from_gray(0x16));
    let cols =
        [Color32::from_rgb(0x3d, 0x6e, 0xb4), Color32::from_rgb(0x3d, 0xa0, 0x6e), Color32::from_rgb(0xb4, 0x9a, 0x3d), Color32::from_rgb(0xb4, 0x4f, 0x3d)];
    let xo = [ed.v("xo1"), ed.v("xo2"), ed.v("xo3")];
    let edges = [r.min.x, x_of(xo[0], r), x_of(xo[1], r), x_of(xo[2], r), r.max.x];
    for b in 0..4 {
        let br = Rect::from_min_max(pos2(edges[b], r.min.y), pos2(edges[b + 1], r.max.y));
        ui.painter().rect_filled(br.shrink2(vec2(0.0, 8.0)), 0.0, cols[b].gamma_multiply(0.35));
        ui.painter().text(br.center(), Align2::CENTER_CENTER, format!("Band {}", b + 1), Tokens::ui(10.5), LABEL);
    }
    for (k, pid) in ["xo1", "xo2", "xo3"].iter().enumerate() {
        let x = edges[k + 1];
        let hr = Rect::from_center_size(pos2(x, r.center().y), vec2(10.0, r.height()));
        let resp = ui.interact(hr, egui::Id::new(("mb-xo", ed.target.clone(), k)), Sense::drag());
        ui.painter().line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(if resp.hovered() || resp.dragged() { 2.5 } else { 1.5 }, Color32::WHITE));
        ui.painter().text(pos2(x + 3.0, r.min.y + 2.0), Align2::LEFT_TOP, format!("{:.0} Hz", xo[k]), Tokens::ui(9.5), ed.t.hot_text);
        ed.auto(format!("fxEditor.{}.{pid}", ed.fx), hr, "Crossover");
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let (lo, hi, _) = ed.range(pid);
            ed.drafts.insert(pid.to_string(), fx_of(pos.x, r).clamp(lo, hi).round());
        }
        if resp.drag_stopped() {
            let v = ed.v(pid);
            ed.commit(pid, v);
        }
    }
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        for b in 1..=4 {
            ui.vertical(|ui| {
                ui.set_width(2.0 * ed.cell.x);
                ui.vertical_centered(|ui| ui.label(egui::RichText::new(format!("Band {b}")).color(LABEL).size(14.0)));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(ed.cell.x - 55.0);
                    transfer_plot(ui, ed, 110.0, b - 1, format!("fxEditor.{}.curve.{b}", ed.fx));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ed.check(ui, &format!("b{b}_solo"), "Solo", false);
                    ui.add_space(8.0);
                    ed.check(ui, &format!("b{b}_bypass"), "Bypass", false);
                });
                let k = |p: &str| format!("b{b}_{p}");
                let (th, ra, at, re, ga) = (k("threshold"), k("ratio"), k("attack"), k("release"), k("gain"));
                ed.knobs(ui, &[(&th, "Threshold"), (&ra, "Ratio")]);
                ed.knobs(ui, &[(&at, "Attack"), (&re, "Release")]);
                ed.knobs(ui, &[(&ga, "Gain")]);
            });
        }
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x, r.center().y), pos2(r.max.x, r.center().y)], Stroke::new(1.0, LINE));
    ui.horizontal_top(|ui| {
        ed.knobs(ui, &[("output", "Output")]);
        ui.add_space(2.0 * ed.cell.x);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ed.check(ui, "lim_on", "Limiter", true);
            ui.add_space(8.0);
            ed.check(ui, "link", "Link Channels", false);
        });
        ui.add_space(20.0);
        ed.knobs(ui, &[("lim_threshold", "Threshold"), ("lim_release", "Release")]);
    });
}
