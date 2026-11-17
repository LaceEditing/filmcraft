//! Effect Controls: the selected clip's effects (fixed Motion/Opacity/Time Remapping first, as in
//! Premiere, then standard effects), generated from parameter schemas, with stopwatches and a
//! keyframe lane on the right. Also hosts the Color Grading panel body (same editor, grouped).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::{ClipId, EffectInstance, ParamKind, ParamValue, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::panels::ec_keyframes::Lane;
use crate::state::{EcView, KfSel};
use crate::theme::Tokens;
use crate::widgets::HotNumberOptions;

const ROW_H: f32 = 22.0;
/// Most lines the Source Text box shows at once.
const TEXT_ROWS: usize = 6;
/// The narrowest the effect list and the keyframe area can be dragged (#643): the list's rows are
/// laid out for at least 260 points (values from 150 points in, the reset button at the end).
const MIN_LIST_W: f32 = 260.0;
const MIN_LANE_W: f32 = 60.0;
/// Keyframe lane header: the time ruler (beside the Source / Sequence pills) and the clip's bar.
const RULER_H: f32 = 24.0;
const CLIP_BAR_H: f32 = 18.0;
/// Distance between the keyframe navigator's buttons (◀ ◆ ▶).
const NAV_STEP: f32 = 16.0;
/// Properties panel: the keyframe diamonds' column, from a row's right edge (the arrows sit one
/// [`NAV_STEP`] to either side of it, and a section's reset button above it).
const PROPS_NAV_X: f32 = 26.0;
/// Properties panel: where a row's value fields end, left of the navigator.
const PROPS_VALUE_R: f32 = PROPS_NAV_X + NAV_STEP + 12.0;

/// Timeline time at which the clip shows media time `m` (where a keyframe sits in the sequence).
fn timeline_time_of(it: &TrackItem, m: Tick) -> Tick {
    let offset = if it.reverse { it.source_out() - m - Tick(1) } else { m - it.source_in };
    it.start + Tick((offset.0 as f64 / it.speed.abs().max(1e-6)).round() as i64)
}

/// The clip's media time the panel shows its values at: the playhead's, held inside the clip.
fn effect_time_now(it: &TrackItem, ph: Tick) -> Tick {
    it.effect_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)))
}

/// Whether `k` sits on a row the panel draws disabled at the playhead `ph` (Scale Width while
/// Uniform Scale is on, #4381). Such keyframes can't be selected, so a group drag or a selection's
/// interpolation change leaves them alone.
fn keyframe_disabled(it: &TrackItem, k: &KfSel, ph: Tick) -> bool {
    k.mask.is_none() && it.effects.get(k.effect).is_some_and(|e| ignored_by_uniform_scale(it, e, &k.param, effect_time_now(it, ph)))
}

/// Edit ▸ Select All (⌘A) with Effect Controls focused selects every keyframe the panel shows
/// instead of the Timeline's clips (#1241). False when the panel is not focused or shows none.
pub(crate) fn select_all_keyframes(app: &mut FilmcraftApp) -> bool {
    if app.ui.focused != crate::dock::PanelKind::EffectControls {
        return false;
    }
    let mut keys = Vec::new();
    let ph = app.session.playhead();
    for (clip, it, _) in selected_clips(app) {
        let mt = effect_time_now(&it, ph);
        for (effect, e) in it.effects.iter().enumerate() {
            let key = |param: &str, mask, time: Tick| KfSel { clip: clip.0, effect, param: param.to_string(), mask, time: time.0 };
            // not a disabled row's (Scale Width while Uniform Scale is on)
            for (param, p) in e.params.iter().filter(|(param, _)| !ignored_by_uniform_scale(&it, e, param, mt)) {
                keys.extend(p.keyframes.iter().map(|kf| key(param, None, kf.time)));
            }
            for (m, mask) in e.masks.iter().enumerate() {
                for param in filmcraft_project::mask::MASK_PARAMS {
                    keys.extend(mask.param(param).into_iter().flat_map(|p| &p.keyframes).map(|kf| key(param, Some(m), kf.time)));
                }
            }
        }
    }
    if keys.is_empty() {
        return false;
    }
    app.ui.kf_selected = keys;
    // effects and keyframes are not selected together (Delete acts on one or the other)
    app.ui.fx_selected.clear();
    true
}

fn selected_clip(app: &FilmcraftApp) -> Option<(ClipId, TrackItem, TrackKind)> {
    selected_clips(app).into_iter().next()
}

/// The selected clip (video first) and the audio linked to it when that audio is selected or
/// linked selection is on — so clip audio FX (Parametric EQ, Custom Setup) stay visible on A/V pairs.
pub(crate) fn selected_clips(app: &FilmcraftApp) -> Vec<(ClipId, TrackItem, TrackKind)> {
    let Some(seq) = app.session.active_sequence() else { return Vec::new() };
    let picked: Vec<(ClipId, &TrackItem, TrackKind)> = app
        .session
        .state
        .selection
        .iter()
        .filter_map(|c| {
            let (tid, it) = seq.find_item(*c)?;
            Some((*c, it, seq.track(tid)?.kind))
        })
        .collect();
    let Some(&(clip, it, kind)) = picked.iter().find(|p| p.2 == TrackKind::Video).or(picked.first()) else { return Vec::new() };
    let mut out = vec![(clip, it.clone(), kind)];
    if kind == TrackKind::Video {
        let linked = picked.iter().find(|p| p.2 == TrackKind::Audio && p.1.link.is_some() && p.1.link == it.link).copied().or_else(|| {
            if !app.session.state.linked_selection {
                return None;
            }
            let link = it.link?;
            seq.audio_tracks.iter().find_map(|t| t.items.iter().find(|i| i.link == Some(link)).map(|i| (i.id, i, TrackKind::Audio)))
        });
        if let Some((ac, ai, ak)) = linked {
            out.push((ac, ai.clone(), ak));
        }
    }
    out
}

/// Where the keyframe lane's diamonds were drawn this frame, with what they are (the keyframe box
/// selects those inside it).
fn kf_pos_id() -> egui::Id {
    egui::Id::unique("ec-kf-pos")
}

/// An effect (or preset) dragged from the Effects panel and released over this panel is applied to
/// the clip shown here, so it need not be aimed at the clip in the Timeline (#2326). Transitions
/// need an edge, so they stay Timeline drops.
fn drop_effect(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect, clips: &[(ClipId, TrackItem, TrackKind)]) {
    let Some(effect) = crate::panels::dragged_effect(ui) else { return };
    let is_transition = filmcraft_project::find_effect(&effect)
        .is_some_and(|d| matches!(d.kind, filmcraft_project::EffectKind::VideoTransition | filmcraft_project::EffectKind::AudioTransition));
    let ctx = ui.ctx().clone();
    let over = ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p) == Some(ui.layer_id()));
    if is_transition || !over {
        return;
    }
    app.auto.add("effectControls.dropTarget", rect, "drop an effect here");
    ui.painter().rect_stroke(rect.shrink(1.0), 3.0, Stroke::new(2.0, app.tokens.accent), egui::StrokeKind::Inside);
    if !ctx.input(|i| i.pointer.any_released()) {
        return;
    }
    // the video clip first; its linked audio clip takes audio effects the video clip refuses
    let mut result = Err(filmcraft_engine::EngineError::NoSequence);
    for (clip, _, _) in clips {
        let params_clips = json!([clip.0]);
        result = if let Some(name) = effect.strip_prefix("preset:") {
            app.session.execute("presets.apply", json!({"preset": name, "clips": params_clips}))
        } else if let Some(name) = effect.strip_prefix(crate::panels::LUMETRI_PRESET_PREFIX) {
            app.session.execute("lumetri.applyPreset", json!({"name": name, "clips": params_clips}))
        } else {
            app.session.execute("effects.apply", json!({"effect": effect, "clips": params_clips}))
        };
        if result.is_ok() {
            break;
        }
    }
    if let Err(e) = result {
        app.ui.status = e.to_string();
    }
    crate::panels::clear_drag(ui);
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let clips = selected_clips(app);
    // selected keyframes that are gone (undo, another clip) drop out, and so do those on a row
    // that is disabled now (selected before Uniform Scale was turned on, say)
    let ph = app.session.playhead();
    app.ui.kf_selected.retain(|k| clips.iter().any(|(c, it, _)| c.0 == k.clip && kf_exists(it, k) && !keyframe_disabled(it, k, ph)));
    let Some((clip, it, _)) = clips.first().cloned() else {
        // a transition clicked in the Timeline (#430)
        if let Some(id) = crate::panels::transition_controls::selected(app) {
            crate::panels::transition_controls::effect_controls(app, ui, rect, id);
            return;
        }
        crate::dock::placeholder(ui, rect, &t, tl!("(no clip selected)"));
        return;
    };
    drop_effect(app, ui, rect, &clips);
    // Keep the immutable project snapshot alive while UI commands mutate the session.
    // Cloning a sequence here copies every clip and effect on every redraw.
    let project = app.session.project.clone();
    let Some(seq) = app.session.state.active_sequence.and_then(|id| project.sequence(id)) else {
        crate::dock::placeholder(ui, rect, &t, tl!("(no sequences)"));
        return;
    };
    // the divider between the effect list and the keyframe area; dragging it sets the list's width
    // (#643), each side keeping a minimum width
    // (the keyframe area has 4 points of margin left of it and 6 right)
    let max_list = (rect.width() - MIN_LANE_W - 10.0).max(MIN_LIST_W);
    let list_w = if app.ui.effect_controls_split > 0.0 { app.ui.effect_controls_split } else { (rect.width() * 0.58).max(260.0) };
    let list_w = list_w.clamp(MIN_LIST_W, max_list);
    let split = rect.min.x + list_w;
    let head = Rect::from_min_size(rect.min + vec2(8.0, 4.0), vec2(split - rect.min.x - 12.0, 24.0));
    // Premiere: two pill tabs — "Source · clip" and "Sequence · clip" (active)
    let pill = |ui: &mut egui::Ui, r: Rect, text: &str, active: bool| {
        ui.painter().rect_filled(r, 4.0, if active { t.pill_active_bg } else { t.panel_bg });
        if !active {
            ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
        }
        let cp = ui.painter().with_clip_rect(r.shrink(2.0));
        cp.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, text, Tokens::ui(11.5), if active { t.text } else { t.text_dim });
    };
    let half = (head.width() - 6.0) / 2.0;
    pill(ui, Rect::from_min_size(head.min, vec2(half, 24.0)), &tlf!("Source · {name}", name = it.name), false);
    pill(ui, Rect::from_min_size(head.min + vec2(half + 6.0, 0.0), vec2(half, 24.0)), &format!("{} · {}", seq_name(app), it.name), true);
    // keyframe area, as Premiere's: a time ruler over the part of the clip it shows (which the
    // playhead's handle rides on), the clip's bar under it, and the zoom bar below
    let lane_rect = Rect::from_min_max(pos2(split + 4.0, rect.min.y + 4.0), pos2(rect.max.x - 6.0, rect.max.y - 26.0));
    ui.painter().rect_filled(lane_rect, 0.0, t.tl_bg);
    let rate = seq.settings.frame_rate;
    let fd = rate.frame_duration();
    let lane = Lane::new(lane_rect, &it, app.ui.ec_view, fd);
    crate::panels::ec_keyframes::wheel(app, ui, &lane, clip);
    let lane = Lane::new(lane_rect, &it, app.ui.ec_view, fd);
    // the linked audio's rows show the same stretch of the timeline
    let view = |c: ClipId| EcView { clip: c.0, start: lane.t0.0, end: lane.t1.0 };
    let lanes: Vec<Lane> = clips.iter().map(|(c, ci, _)| if *c == clip { lane } else { Lane::new(lane_rect, ci, Some(view(*c)), fd) }).collect();
    let ruler = Rect::from_min_max(lane_rect.min, pos2(lane_rect.max.x, lane_rect.min.y + RULER_H));
    let bar_row = Rect::from_min_max(pos2(lane_rect.min.x, ruler.max.y + 4.0), pos2(lane_rect.max.x, ruler.max.y + 4.0 + CLIP_BAR_H));
    let bar = Rect::from_min_max(pos2(lane.x(it.start).max(bar_row.min.x), bar_row.min.y), pos2(lane.x(it.end()).min(bar_row.max.x), bar_row.max.y));
    let scrub = Rect::from_min_max(ruler.min, bar_row.max);
    app.auto.add("effectControls.lane", lane_rect, "keyframe lane");
    app.auto.add("effectControls.ruler", scrub, "time ruler");
    let zbar = Rect::from_min_max(pos2(lane_rect.min.x + 6.0, rect.max.y - 22.0), pos2(lane_rect.max.x - 6.0, rect.max.y - 6.0));
    crate::panels::ec_keyframes::zoom_bar(app, ui, zbar, &lane, clip);
    let body = Rect::from_min_max(pos2(rect.min.x, head.max.y + 4.0), pos2(split, rect.max.y - 26.0));
    let mut actions: Vec<(String, Value)> = Vec::new();
    // the rows span the list and the keyframe area: the wheel scrolls anywhere over them and the
    // scrollbar sits at the panel's right edge (#643)
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(body.min, pos2(lane_rect.max.x, body.max.y))).id_salt("ec-body"));
    bui.set_clip_rect(Rect::from_min_max(body.min, pos2(rect.max.x, body.max.y)));
    bui.data_mut(|d| d.insert_temp::<Vec<(Pos2, KfSel)>>(kf_pos_id(), Vec::new()));
    let fx_drag_id = egui::Id::unique(("ec-fx-drag", clip.0));
    #[allow(clippy::cognitive_complexity)]
    let scroll_out = egui::ScrollArea::vertical().id_salt("ec-scroll").auto_shrink([false, false]).show(&mut bui, |bui| {
        // the lane behind the keyframes: a click clears the keyframe selection, a drag selects the
        // keyframes inside its box (Shift / Cmd adds them)
        let lane_body = Rect::from_min_max(pos2(lane_rect.min.x, bar_row.max.y), lane_rect.max);
        let marquee = bui.interact(lane_body, egui::Id::unique(("ec-marquee", clip.0)), Sense::click_and_drag());
        app.auto.add("effectControls.laneBody", lane_body, "keyframe lane");
        // effect headers (for dragging an effect to another place): clip, index, row, movable
        let mut headers: Vec<(u64, usize, Rect, bool)> = Vec::new();
        let mut fx_drop = false;
        for ((clip, it, kind), lane) in clips.iter().zip(&lanes) {
            let clip = *clip;
            let (hr, _) = bui.allocate_exact_size(vec2(body.width(), 18.0), Sense::hover());
            let heading = if *kind == TrackKind::Video { tl!("Video") } else { tl!("Audio") };
            bui.painter().text(pos2(hr.min.x + 8.0, hr.center().y), Align2::LEFT_CENTER, heading, Tokens::semibold(11.5), t.text_dim);
            // a graphic with Responsive Design – Time: its clip effects below keep media time (#3361)
            if *kind == TrackKind::Video && it.graphic.as_deref().is_some_and(|g| g.has_responsive_time()) {
                let hint = tl!("Responsive Design – Time remaps the graphic's layer keyframes; clip effects like Opacity and Motion keep media time");
                let ir = Rect::from_center_size(pos2(hr.max.x - 14.0, hr.center().y), vec2(18.0, 18.0));
                // a small button: keyboard-focusable, with an accessible name, tooltip on hover or focus
                let resp = bui.put(ir, egui::Button::new("").frame(false));
                icons::paint(bui.painter(), ir.shrink(2.0), Icon::Info, t.text_dim);
                if resp.ctx.has_accesskit_node(resp.id) {
                    resp.ctx.accesskit_node_builder(resp.id, |node| node.set_label(hint));
                }
                if resp.has_focus() {
                    resp.show_tooltip_text(hint);
                } else {
                    resp.on_hover_text(hint);
                }
                app.auto.add("effectControls.graphicTimeHint", ir, hint);
            }
            let mt_now = effect_time_now(it, ph);
            // Premiere lists the fixed effects (Motion, Opacity, Time Remapping / Volume…) first.
            let mut order: Vec<usize> = (0..it.effects.len()).collect();
            order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
            // the effects picked in this panel, else the one selected elsewhere (the Program
            // monitor's handles act on it)
            let picked_here = app.ui.fx_selected.iter().any(|(c, _)| *c == clip.0);
            let mut after_fixed = false;
            for idx in order {
                let e = &it.effects[idx];
                let Some(def) = e.def() else { continue };
                // the first effect added from the library after the fixed ones gets a line on top
                // (#2328); drawn on its edge, so no row moves
                let first_added = if def.intrinsic {
                    after_fixed = true;
                    false
                } else {
                    std::mem::take(&mut after_fixed)
                };
                let key = format!("{}:{}", clip.0, idx);
                let open = !app.ui.collapsed_fx.contains(&key);
                let selected = if picked_here {
                    app.ui.fx_selected.contains(&(clip.0, idx))
                } else {
                    app.session.state.selected_effect.as_ref().is_some_and(|s| s.clip == clip && s.index(&it.effects) == Some(idx))
                };
                let movable = filmcraft_engine::effect_edit::movable(e);
                let (r, resp) = bui.allocate_exact_size(vec2(body.width(), ROW_H), if movable { Sense::click_and_drag() } else { Sense::click() });
                headers.push((clip.0, idx, r, movable));
                if movable && resp.drag_started() {
                    bui.data_mut(|d| d.insert_temp(fx_drag_id, (clip.0, idx)));
                }
                if movable && resp.drag_stopped() {
                    fx_drop = true;
                }
                if selected {
                    bui.painter().rect_filled(row_fill(r), 0.0, t.row_selected);
                } else if resp.hovered() {
                    bui.painter().rect_filled(row_fill(r), 0.0, t.hover);
                }
                row_line(bui, r, lane, &t);
                if first_added {
                    let line = Rect::from_min_max(pos2(r.min.x, r.min.y - 1.0), pos2(r.max.x, r.min.y + 1.0));
                    bui.painter().rect_filled(line, 0.0, t.accent.gamma_multiply(0.8));
                    let which = if *kind == TrackKind::Video { "video" } else { "audio" };
                    app.auto.add(&format!("effectControls.addedEffects.{which}"), line, "Added effects");
                }
                // only the arrow twirls the effect open or shut; the rest of the header selects it
                let tw = Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(10.0, 10.0));
                icons::paint(bui.painter(), tw, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_dim);
                let twresp = bui.interact(tw.expand(3.0), egui::Id::unique(("fx-twirl", clip.0, idx)), Sense::click());
                app.auto.add(&format!("effectControls.effect.{}.twirl", e.effect), tw, "twirl");
                app.auto.add(&format!("effectControls.effect.{}.fold", e.effect), tw, "twirl");
                // fx enable toggle
                let fxr = Rect::from_center_size(pos2(r.min.x + 28.0, r.center().y), vec2(18.0, 14.0));
                let fxresp = bui.interact(fxr, egui::Id::unique(("fxen", clip.0, idx)), Sense::click());
                bui.painter().text(fxr.center(), Align2::CENTER_CENTER, "fx", Tokens::semibold(10.5), if e.enabled { t.text } else { t.text_faint });
                if !e.enabled {
                    bui.painter().line_segment([fxr.left_bottom(), fxr.right_top()], Stroke::new(1.0, t.text_faint));
                }
                if fxresp.clicked() {
                    actions.push(("effects.toggleEnabled".into(), json!({"clip": clip.0, "index": idx})));
                }
                // (the reset button sits at the row's right end)
                row_label(bui, pos2(r.min.x + 42.0, r.center().y), crate::i18n::t(def.name), r.max.x - 26.0 - (r.min.x + 42.0), Tokens::ui(12.0), t.text);
                // reset button
                let rr = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(16.0, 16.0));
                let rresp = bui.interact(rr, egui::Id::unique(("fxreset", clip.0, idx)), Sense::click()).on_hover_text(tl!("Reset Effect"));
                icons::paint(bui.painter(), rr.shrink(2.0), Icon::Reset, if rresp.hovered() { t.text } else { t.text_dim });
                if rresp.clicked() {
                    actions.push(("effects.reset".into(), json!({"clip": clip.0, "index": idx})));
                }
                app.auto.add(&format!("effectControls.effect.{}", e.effect), r, def.name);
                // summary keyframes: every time any parameter of the effect has one
                let mut times: Vec<Tick> = e.params.values().flat_map(|p| p.keyframes.iter().map(|k| k.time)).collect();
                times.sort_unstable();
                times.dedup();
                for mt in times {
                    let x = lane.x_of_media(mt);
                    if !lane.shows(x) || x < lane_rect.min.x || x > lane_rect.max.x {
                        continue;
                    }
                    let c = pos2(x, r.center().y);
                    bui.painter().add(egui::Shape::convex_polygon(
                        vec![c + vec2(0.0, -3.5), c + vec2(3.5, 0.0), c + vec2(0.0, 3.5), c + vec2(-3.5, 0.0)],
                        t.text_faint,
                        Stroke::NONE,
                    ));
                }
                if twresp.clicked() {
                    if open {
                        app.ui.collapsed_fx.push(key.clone());
                    } else {
                        app.ui.collapsed_fx.retain(|k| *k != key);
                    }
                } else if resp.clicked() && !fxresp.clicked() && !rresp.clicked() {
                    // the name selects the effect (Cmd/Ctrl adds or removes, Shift extends from
                    // the last one), as in Premiere; a preset can be saved from several at once
                    let mods = bui.input(|i| i.modifiers);
                    select_effect(app, clip, idx, mods, it, &mut actions);
                }
                // a right-click on an unselected effect selects it alone first
                if resp.secondary_clicked() && !selected {
                    select_effect(app, clip, idx, egui::Modifiers::NONE, it, &mut actions);
                }
                let mut save_preset = false;
                let fx_id = e.effect.clone();
                let picked: Vec<usize> = {
                    let mut v: Vec<usize> = app.ui.fx_selected.iter().filter(|(c, _)| *c == clip.0).map(|(_, i)| *i).collect();
                    v.sort_unstable();
                    if v.contains(&idx) { v } else { vec![idx] }
                };
                resp.context_menu(|ui| {
                    let label = if picked.len() > 1 { format!("{} ({})", tl!("Save Preset…"), picked.len()) } else { tl!("Save Preset…").to_string() };
                    let sp = ui.button(&label);
                    app.auto.add(&format!("effectControls.effect.{fx_id}.savePreset"), sp.rect, "Save Preset…");
                    if sp.clicked() {
                        save_preset = true;
                        ui.close();
                    }
                    if !def.intrinsic && ui.button(tl!("Clear")).clicked() {
                        actions.push(("effects.remove".into(), json!({"clip": clip.0, "index": idx})));
                        ui.close();
                    }
                });
                if save_preset {
                    let name = if picked.len() > 1 { format!("{} + {}", def.name, picked.len() - 1) } else { def.name.to_string() };
                    crate::panels::presets::open_save(app, clip.0, picked.clone(), &name);
                }
                if !open {
                    continue;
                }
                if crate::panels::audio_fx_editor::has_editor(&e.effect) {
                    custom_setup_row(app, bui, body, lane, clip, idx, &e.effect);
                }
                for pd in &def.params {
                    let mut rows = |bui: &mut egui::Ui| {
                        param_row(app, bui, body, clip, idx, e, None, pd, mt_now, &mut actions, lane, it);
                        if app.ui.expanded_fx.contains(&graph_key(clip, idx, pd.id))
                            && let Some(param) = e.params.get(pd.id)
                            && crate::panels::ec_keyframes::graphable(param)
                        {
                            crate::panels::ec_keyframes::graph_rows(app, bui, body, clip, idx, None, pd, param, lane, it, &mut actions);
                        }
                    };
                    // Scale Width while Uniform Scale is on: greyed out, neither its value nor its
                    // graphs dragged or typed (#4381). Only that row gets a scope: an empty one
                    // (a parameter the instance lacks) would still add a row gap.
                    if ignored_by_uniform_scale(it, e, pd.id, mt_now) {
                        bui.add_enabled_ui(false, rows);
                    } else {
                        rows(bui);
                    }
                }
                if crate::panels::masks::maskable(e) {
                    crate::panels::masks::effect_rows(app, bui, body, clip, idx, e, mt_now, &mut actions, lane, it);
                }
            }
        }
        // an effect dragged by its name goes between the standard effects of its clip it is
        // dropped between
        if let Some((owner, from)) = bui.data(|d| d.get_temp::<(u64, usize)>(fx_drag_id)) {
            let movable: Vec<(usize, Rect)> = headers.iter().filter(|h| h.0 == owner && h.3).map(|h| (h.1, h.2)).collect();
            if let (Some(p), Some(last)) = (bui.ctx().pointer_latest_pos(), movable.last()) {
                // the first standard effect whose middle is below the pointer, else after the last
                let before = movable.iter().find(|(_, r)| p.y < r.center().y);
                let (line_y, to) = match before {
                    Some((i, r)) => (r.min.y, if *i > from { i - 1 } else { *i }),
                    None => (last.1.max.y, last.0),
                };
                bui.painter().line_segment([pos2(body.min.x + 4.0, line_y), pos2(body.max.x - 4.0, line_y)], Stroke::new(2.0, t.accent));
                if fx_drop {
                    bui.data_mut(|d| d.remove::<(u64, usize)>(fx_drag_id));
                    if to != from {
                        actions.push(("effects.move".into(), json!({"clip": owner, "index": from, "to": to})));
                        app.ui.fx_selected = vec![(owner, to)];
                    }
                }
            } else if fx_drop {
                bui.data_mut(|d| d.remove::<(u64, usize)>(fx_drag_id));
            }
        }
        // the keyframe box (Escape abandons it, #580)
        let start_id = egui::Id::unique(("ec-marquee-start", clip.0));
        if marquee.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = bui.input(|i| i.pointer.press_origin())
        {
            bui.data_mut(|d| d.insert_temp(start_id, p));
        }
        if bui.input(|i| i.key_pressed(egui::Key::Escape)) {
            bui.data_mut(|d| d.remove::<Pos2>(start_id));
        }
        let start: Option<Pos2> = bui.data(|d| d.get_temp(start_id));
        if let (Some(a), Some(b)) = (start, marquee.interact_pointer_pos().or(bui.ctx().pointer_latest_pos())) {
            let boxr = Rect::from_two_pos(a, b);
            if marquee.dragged() {
                bui.painter().rect(boxr, 0.0, t.accent.gamma_multiply(0.12), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                app.auto.add("effectControls.marquee", boxr, "keyframe selection");
            }
            if marquee.drag_stopped() {
                bui.data_mut(|d| d.remove::<Pos2>(start_id));
                let inside: Vec<KfSel> = bui
                    .data(|d| d.get_temp::<Vec<(Pos2, KfSel)>>(kf_pos_id()))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(c, _)| boxr.contains(*c))
                    .map(|(_, k)| k)
                    .collect();
                let m = bui.input(|i| i.modifiers);
                if !(m.shift || m.command || m.ctrl) {
                    app.ui.kf_selected.clear();
                }
                for k in inside {
                    if !app.ui.kf_selected.contains(&k) {
                        app.ui.kf_selected.push(k);
                    }
                }
                app.ui.fx_selected.clear();
            }
        }
        if marquee.clicked() {
            app.ui.kf_selected.clear();
        }
    });
    let _ = scroll_out;
    // the selected keyframes, dragged along the lane, move as the pointer goes
    for ((_, it, _), lane) in clips.iter().zip(&lanes) {
        crate::panels::ec_keyframes::run_lane_drag(app, ui, it, lane, &mut actions);
    }
    // the ruler and the clip's bar, painted after the rows so rows scrolled up under them (their
    // lines and keyframes) stay hidden
    ui.painter().rect_filled(Rect::from_min_max(lane_rect.min, pos2(lane_rect.max.x, bar_row.max.y)), 0.0, t.tl_bg);
    paint_ruler(ui.painter(), ruler, lane.t0, lane.t1, rate, seq.settings.drop_frame, &t);
    ui.painter().rect_filled(bar.shrink2(vec2(0.0, 2.0)), 2.0, t.clip_bar_bg);
    ui.painter().with_clip_rect(bar).text(pos2(bar.min.x + 4.0, bar.center().y), Align2::LEFT_CENTER, &it.name, Tokens::ui(10.0), t.text);
    // click or drag anywhere on the ruler (or the bar) to move the playhead (after the rows, which
    // reach under it, so it gets the pointer first)
    let sresp = ui.interact(scrub, egui::Id::unique(("ec-scrub", clip.0)), Sense::click_and_drag());
    if (sresp.dragged() || sresp.clicked())
        && let Some(pos) = sresp.interact_pointer_pos()
    {
        let tk = rate.snap_nearest(lane.tl_at(pos.x.max(lane_rect.min.x).min(lane_rect.max.x))).clamp(it.start, it.end());
        app.stop_for_scrub();
        app.session.set_playhead(tk);
    }
    // the divider: a line the height of the panel, a resize cursor and a wider grab area
    let divider = Rect::from_min_max(pos2(split - 2.0, rect.min.y), pos2(split + 4.0, rect.max.y - 26.0));
    let dresp = ui.interact(divider, egui::Id::unique("ec-divider"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if dresp.dragged() {
        app.ui.effect_controls_split = (list_w + dresp.drag_delta().x).clamp(MIN_LIST_W, max_list);
    }
    let dcol = if dresp.hovered() || dresp.dragged() { t.accent } else { t.separator };
    ui.painter().line_segment([pos2(split + 1.5, rect.min.y), pos2(split + 1.5, rect.max.y - 26.0)], Stroke::new(1.0, dcol));
    app.auto.add("effectControls.divider", divider, "divider");
    // playhead: a handle on the ruler and a line down the lane, while it is on the part of the clip
    // the lane shows
    if ph >= it.start && ph <= it.end() && ph >= lane.t0 && ph <= lane.t1 {
        let px = lane.x(ph);
        let (top, tip) = (ruler.max.y - 13.0, ruler.max.y);
        let head = vec![pos2(px - 5.0, top), pos2(px + 5.0, top), pos2(px + 5.0, tip - 5.0), pos2(px, tip), pos2(px - 5.0, tip - 5.0)];
        ui.painter().add(egui::Shape::convex_polygon(head, t.playhead, Stroke::NONE));
        ui.painter().line_segment([pos2(px, tip), pos2(px, lane_rect.max.y)], Stroke::new(1.0, t.playhead));
        app.auto.add("effectControls.playhead", Rect::from_min_max(pos2(px - 5.0, top), pos2(px + 5.0, tip)), "playhead");
    }
    // footer timecode
    let tc = filmcraft_time::format_time(ph, seq.settings.frame_rate, seq.settings.drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
    ui.painter().text(pos2(rect.min.x + 10.0, rect.max.y - 13.0), Align2::LEFT_CENTER, tc, Tokens::mono(13.0), t.timecode);
    run(app, ui.ctx(), actions);
}

/// The thin line under a row, across the names, the values and the keyframe lane, so a row can be
/// followed across the panel, as in Premiere (#640). It runs through the middle of the spacing
/// between rows, so each row sits centred between its two lines.
pub(crate) fn row_line(ui: &egui::Ui, r: Rect, lane: &Lane, t: &Tokens) {
    let y = r.max.y + ui.spacing().item_spacing.y / 2.0;
    ui.painter().line_segment([pos2(r.min.x, y), pos2(lane.rect.max.x, y)], Stroke::new(1.0, t.separator));
}

/// A row's label, cut off with "…" where it would run into what follows it, as Premiere does when
/// the effect list is narrow (#643).
pub(crate) fn row_label(ui: &egui::Ui, left_center: Pos2, text: &str, max_w: f32, font: egui::FontId, color: Color32) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap.max_width = max_w.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(pos2(left_center.x, left_center.y - g.size().y / 2.0), g, color);
}

/// Make a dropdown or a checkbox in a row fit inside the row, centred between its row lines.
pub(crate) fn fit_to_row(ui: &mut egui::Ui) {
    ui.spacing_mut().interact_size.y = ROW_H - 4.0;
    ui.spacing_mut().button_padding.y = 1.0;
}

/// Effect Controls selection: a click selects the effect, Cmd/Ctrl-click toggles it, Shift-click
/// selects the range from the last picked effect (in the panel's order: fixed effects first). The
/// session's selected effect (`effects.select`: the Program monitor's handles act on its Motion or
/// Transform) is the one clicked while it is picked.
fn select_effect(app: &mut FilmcraftApp, clip: ClipId, idx: usize, mods: egui::Modifiers, it: &TrackItem, actions: &mut Vec<(String, Value)>) {
    // effects and keyframes are not selected together (Delete acts on one or the other)
    app.ui.kf_selected.clear();
    let sel = &mut app.ui.fx_selected;
    sel.retain(|(c, _)| *c == clip.0);
    if mods.command || mods.ctrl {
        if let Some(i) = sel.iter().position(|x| *x == (clip.0, idx)) {
            sel.remove(i);
        } else {
            sel.push((clip.0, idx));
        }
    } else if mods.shift && !sel.is_empty() {
        let mut order: Vec<usize> = (0..it.effects.len()).collect();
        order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
        let at = |i: usize| order.iter().position(|o| *o == i).unwrap_or(0);
        let last = sel.last().map_or(idx, |(_, i)| *i);
        let (a, b) = (at(last).min(at(idx)), at(last).max(at(idx)));
        for o in &order[a..=b] {
            if !sel.contains(&(clip.0, *o)) {
                sel.push((clip.0, *o));
            }
        }
    } else {
        *sel = vec![(clip.0, idx)];
    }
    let picked = sel.contains(&(clip.0, idx));
    actions.push(("effects.select".into(), if picked { json!({"clip": clip.0, "effect": idx}) } else { json!({"none": true}) }));
}

/// Premiere's "Custom Setup ▸ Edit…" row: opens the effect's Clip Fx Editor window.
fn custom_setup_row(app: &mut FilmcraftApp, ui: &mut egui::Ui, body: Rect, lane: &Lane, clip: ClipId, idx: usize, effect: &str) {
    let t = app.tokens;
    let (r, _) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::hover());
    row_line(ui, r, lane, &t);
    ui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, tl!("Custom Setup"), Tokens::ui(12.0), t.text_dim);
    let br = Rect::from_min_size(pos2(r.min.x + 160.0, r.min.y + 2.0), vec2(60.0, ROW_H - 4.0));
    let resp = ui.interact(br, egui::Id::unique(("fx-custom-setup", clip.0, idx)), Sense::click());
    ui.painter().rect_filled(br, 3.0, if resp.hovered() { t.hover } else { t.field_bg });
    ui.painter().rect_stroke(br, 3.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(br.center(), Align2::CENTER_CENTER, tl!("Edit…"), Tokens::ui(11.5), t.text);
    app.auto.add(&format!("effectControls.effect.{effect}.edit"), br, "Edit…");
    if resp.clicked() {
        crate::panels::audio_fx_editor::open(app, crate::panels::audio_fx_editor::FxTarget::Clip { clip: clip.0, index: idx });
    }
}

fn seq_name(app: &FilmcraftApp) -> String {
    app.session.state.active_sequence.and_then(|s| app.session.project.item(s)).map(|i| i.name.clone()).unwrap_or_default()
}

/// The keyframe lane's time ruler: ticks and sequence timecode across the stretch of the timeline
/// the lane shows, `start` to `end` (the clip, or the part of it the lane is zoomed in on). Steps
/// are whole frames or whole seconds of timecode, spaced so the labels never crowd.
fn paint_ruler(p: &egui::Painter, ruler: Rect, start: Tick, end: Tick, rate: filmcraft_time::FrameRate, drop_frame: bool, t: &Tokens) {
    let p = p.with_clip_rect(ruler);
    let dur = (end - start).0.max(1) as f64;
    let frame_px = ruler.width() as f64 * rate.frame_duration().0.max(1) as f64 / dur;
    let base = rate.timecode_base();
    let steps = [
        1,
        2,
        5,
        10,
        base / 2,
        base,
        base * 2,
        base * 5,
        base * 10,
        base * 15,
        base * 30,
        base * 60,
        base * 120,
        base * 300,
        base * 600,
        base * 1800,
        base * 3600,
    ];
    // labels far enough apart to read; the small ticks divide the labelled ones evenly
    let label_step = steps.iter().copied().find(|s| *s > 0 && *s as f64 * frame_px >= 80.0);
    let minor = steps.iter().copied().find(|s| *s > 0 && *s as f64 * frame_px >= 8.0 && label_step.is_none_or(|l| l % s == 0));
    let (f0, f1) = (rate.frame_at(start), rate.frame_at(end));
    let base_y = ruler.max.y - 1.0;
    for (step, h, labelled) in [(minor, 3.0, false), (label_step, 7.0, true)] {
        let Some(step) = step else { continue };
        let mut f = f0.div_euclid(step).saturating_mul(step);
        // a step is at least 8 px wide, so this covers any ruler; the cap is for damaged numbers
        for _ in 0..4096 {
            if f > f1 {
                break;
            }
            let x = ruler.min.x + ((rate.tick_of(f) - start).0 as f64 / dur) as f32 * ruler.width();
            p.line_segment([pos2(x, base_y - h), pos2(x, base_y)], Stroke::new(1.0, t.tl_ruler_tick));
            if labelled {
                let label = filmcraft_time::format_time(rate.tick_of(f), rate, drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
                p.text(pos2(x, ruler.min.y + 7.0), Align2::CENTER_CENTER, label, Tokens::ui(10.0), t.tl_ruler_text);
            }
            f = f.saturating_add(step);
        }
    }
}

/// Premiere's keyframe navigator, centred on `at`: a diamond that adds a keyframe at the playhead
/// or removes the one there (filled while the playhead is on a keyframe) and, once the parameter
/// is animated, arrows to the previous and the next keyframe. `param` is `None` while the effect
/// is not on the clip yet. Automation ids: `<auto>.addKeyframe`, `.prevKeyframe`, `.nextKeyframe`.
/// Returns whether the diamond was clicked; the arrows queue their own `playhead.set`.
#[allow(clippy::too_many_arguments)]
fn keyframe_nav(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    at: Pos2,
    param: Option<&filmcraft_project::Param>,
    mt: Tick,
    it: &TrackItem,
    id: egui::Id,
    auto: &str,
    tip: &str,
    actions: &mut Vec<(String, Value)>,
) -> bool {
    let t = app.tokens;
    if let Some(param) = param.filter(|p| p.is_animated()) {
        for (d, target, name, tip) in
            [(-1.0, param.prev_keyframe(mt), "prevKeyframe", "Go to previous keyframe"), (1.0, param.next_keyframe(mt), "nextKeyframe", "Go to next keyframe")]
        {
            // the arrows almost the row's height to click, as Premiere's
            let r = Rect::from_center_size(at + vec2(d * NAV_STEP, 0.0), vec2(14.0, 20.0));
            let resp = ui.interact(r, id.with(name), Sense::click()).on_hover_text(crate::i18n::t(tip));
            let c = r.center();
            let pts = vec![c + vec2(-3.5 * d, -5.5), c + vec2(-3.5 * d, 5.5), c + vec2(3.5 * d, 0.0)];
            ui.painter().add(egui::Shape::convex_polygon(pts, if target.is_some() { t.text } else { t.text_faint }, Stroke::NONE));
            app.auto.add(&format!("{auto}.{name}"), r, tip);
            if resp.clicked()
                && let Some(k) = target
            {
                actions.push(("playhead.set".into(), json!({"time": timeline_time_of(it, k).0})));
            }
        }
    }
    let r = Rect::from_center_size(at, vec2(15.0, 15.0));
    let resp = ui.interact(r.expand(1.5), id.with("key"), Sense::click()).on_hover_text(crate::i18n::t(tip));
    if param.is_some_and(|p| p.keyframes.iter().any(|k| k.time == mt)) {
        icons::paint(ui.painter(), r, Icon::Keyframe, t.hot_text);
    } else {
        let s = 4.7;
        let outline = vec![at + vec2(0.0, -s), at + vec2(s, 0.0), at + vec2(0.0, s), at + vec2(-s, 0.0)];
        ui.painter().add(egui::Shape::closed_line(outline, Stroke::new(1.2, if resp.hovered() { t.text } else { t.text_dim })));
    }
    app.auto.add(&format!("{auto}.addKeyframe"), r, tip);
    resp.clicked()
}

/// A Properties row's keyframe navigator for `effect`.`param`, with Premiere's tooltips. The
/// diamond is `effects.addKeyframe`: it adds a keyframe at the playhead (turning animation on) or
/// removes the one there, so the value falls back to what the remaining keyframes give. An effect
/// the clip does not have yet (Crop) is applied first.
#[allow(clippy::too_many_arguments)]
fn properties_nav(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    row: Rect,
    clip: ClipId,
    it: &TrackItem,
    mt: Tick,
    effect: &str,
    param: &str,
    actions: &mut Vec<(String, Value)>,
) {
    let prm = it.effect(effect).and_then(|e| e.param(param));
    let tip = match prm {
        Some(p) if p.keyframes.iter().any(|k| k.time == mt) => "Remove keyframe",
        Some(p) if p.is_animated() => "Add keyframe",
        _ => "Turn on animation and add keyframe",
    };
    let at = pos2(row.max.x - PROPS_NAV_X, row.center().y);
    let id = egui::Id::unique(("props-nav", effect, param, clip.0));
    if keyframe_nav(app, ui, at, prm, mt, it, id, &format!("properties.{effect}.{param}"), tip, actions) {
        if it.effect(effect).is_none() {
            actions.push(("effects.apply".into(), json!({"clips": [clip.0], "effect": effect})));
        }
        actions.push(("effects.addKeyframe".into(), json!({"clip": clip.0, "effect": effect, "param": param})));
    }
}

/// Whether `param` is Scale Width with the effect's Uniform Scale on at media time `mt` of `it`:
/// Motion, Transform and graphic layers then scale both axes by Scale (Transform's Scale Height)
/// and the render ignores Scale Width. Uniform Scale is read exactly as the render reads it, so the
/// row is greyed out exactly when Scale Width does nothing, even with a keyframe on it (Uniform
/// Scale can't be animated, but `effects.addKeyframe` adds one) or with it missing or non-boolean:
/// - Motion (`filmcraft_render::motion_matrix`): the stored value, keyframes aside; missing = on;
/// - Transform (`gpufx`, `vfx::distort`): the stored value, keyframes aside; missing = off;
/// - graphic layers (`filmcraft_project::graphic::eval_layer`): keyframes too, at the layer's time
///   (Responsive Design – Time); missing = the definition's default (on).
fn ignored_by_uniform_scale(it: &TrackItem, e: &EffectInstance, param: &str, mt: Tick) -> bool {
    if param != "scale_width" {
        return false;
    }
    let uniform = e.param("uniform_scale");
    if filmcraft_project::graphic::is_layer(e) {
        let t = filmcraft_render::graphic_clip::graphic_time(it, mt).0;
        let value = match uniform {
            Some(p) => Some(p.value_at(t)),
            None => e.def().and_then(|d| d.param("uniform_scale")).map(|d| d.default.clone()),
        };
        return value.and_then(|v| v.as_bool()).unwrap_or(false);
    }
    uniform.and_then(|p| p.value.as_bool()).unwrap_or(e.effect == "motion")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn param_row(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    body: Rect,
    clip: ClipId,
    idx: usize,
    e: &EffectInstance,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    mt: Tick,
    actions: &mut Vec<(String, Value)>,
    lane: &Lane,
    it: &TrackItem,
) {
    let t = app.tokens;
    let param = match mask {
        Some(k) => e.masks.get(k).and_then(|m| m.param(pd.id)),
        None => e.params.get(pd.id),
    };
    let Some(param) = param else { return };
    // mask parameters: automation ids / widget ids get a `m<k>.` prefix, commands a `mask` field
    let pkey: String = match mask {
        Some(k) => format!("mask{k}.{}", pd.id),
        None => pd.id.to_string(),
    };
    let pkey = pkey.as_str();
    let with_mask = |mut v: Value| -> Value {
        if let Some(k) = mask {
            v["mask"] = json!(k);
        }
        v
    };
    let row_h = match (&param.value, e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "text") {
        (ParamValue::Text(s), true) => ROW_H + 16.0 * (s.lines().count().clamp(1, TEXT_ROWS) as f32 - 1.0) + 6.0,
        _ => ROW_H,
    };
    let (r, row) = ui.allocate_exact_size(vec2(body.width(), row_h), if mask.is_none() { Sense::click() } else { Sense::hover() });
    row_line(ui, r, lane, &t);
    // clicking one of an effect's properties selects the effect, as clicking its header does
    if row.clicked() {
        select_effect(app, clip, idx, egui::Modifiers::NONE, it, actions);
    }
    // the label, stopwatch and keyframes sit on the first line of a taller row
    let cy = r.min.y + ROW_H / 2.0;
    let mut x = r.min.x + 26.0;
    // twirl-down for the value/velocity graphs (animated numbers and points)
    if crate::panels::ec_keyframes::graphable(param) {
        let key = graph_key(clip, idx, pkey);
        let open = app.ui.expanded_fx.contains(&key);
        let tw = Rect::from_center_size(pos2(r.min.x + 12.0, cy), vec2(12.0, 12.0));
        let tresp = ui.interact(tw.expand(2.0), egui::Id::unique(("twirl", clip.0, idx, pkey)), Sense::click()).on_hover_text(tl!("Show graphs"));
        icons::paint(ui.painter(), tw, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_dim);
        app.auto.add(&format!("effectControls.{}.{}.graphs", e.effect, pkey), tw, "Show graphs");
        if tresp.clicked() {
            if open {
                app.ui.expanded_fx.retain(|k| *k != key);
            } else {
                app.ui.expanded_fx.push(key);
            }
        }
    }
    if pd.animatable {
        let sw = Rect::from_center_size(pos2(x, cy), vec2(14.0, 14.0));
        let resp = ui.interact(sw, egui::Id::unique(("sw", clip.0, idx, pkey)), Sense::click()).on_hover_text(tl!("Toggle animation"));
        icons::paint(ui.painter(), sw, Icon::Stopwatch, if param.is_animated() { t.accent } else { t.text_dim });
        if resp.clicked() {
            actions.push(("effects.toggleAnimation".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id}))));
        }
        app.auto.add(&format!("effectControls.{}.{}.stopwatch", e.effect, pkey), sw, "Toggle animation");
    }
    x += 14.0;
    // the value: left of the keyframe navigator when there is one, starting early enough for a
    // point's two numbers to fit
    let value_end = if pd.animatable && param.is_animated() { r.max.x - 68.0 } else { r.max.x - 26.0 };
    let need = if matches!(pd.kind, ParamKind::Point) { 112.0 } else { 56.0 };
    let vx = (r.min.x + (r.width() * 0.42).max(130.0)).min(value_end - need).max(r.min.x + 96.0);
    row_label(ui, pos2(x, cy), crate::i18n::t(pd.label), vx - 6.0 - x, Tokens::ui(12.0), t.text);
    // double-clicking a property's name resets it, as the reset button at the row's end does (a
    // click on the value itself starts typing, so the name is the double-click target)
    if mask.is_none() {
        let name = Rect::from_min_max(pos2(x, r.min.y), pos2(vx - 6.0, r.max.y));
        // the instance index tells two copies of one effect apart; only the visible part of a row
        // scrolled partly out of view is registered
        let shown = name.intersect(ui.clip_rect());
        if shown.is_positive() {
            app.auto.add(&format!("effectControls.{}.{idx}.{}.name", e.effect, pkey), shown, crate::i18n::t(pd.label));
        }
        if row.double_clicked() && row.interact_pointer_pos().is_some_and(|p| name.contains(p)) {
            actions.push(("effects.resetParam".into(), json!({"clip": clip.0, "effect": idx, "param": pd.id})));
        }
    }
    let value = param.value_at(mt);
    let mut vui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(vx, r.min.y + 1.0), pos2(value_end.max(vx + 40.0), r.max.y - 1.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let id = egui::Id::unique(("pv", clip.0, idx, pkey));
    let set = param_value_widget(app, &mut vui, e, pd, &value, clip, idx, mask, pkey, id, t, mt, it, actions);
    if let Some(v) = set {
        actions.push(("effects.setParam".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": v}))));
    }
    // keyframe navigator ◀ ◆ ▶ (when animated)
    let eff_json = json!(idx);
    if pd.animatable && param.is_animated() {
        let at = pos2(r.max.x - 42.0, cy);
        let auto = format!("effectControls.{}.{}", e.effect, pkey);
        if keyframe_nav(app, ui, at, Some(param), mt, it, egui::Id::unique(("knav", clip.0, idx, pkey)), &auto, "Add/Remove Keyframe", actions) {
            actions.push(("effects.addKeyframe".into(), with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id}))));
        }
    }
    // reset to the default, under the effect's own reset button (an animated parameter keeps its
    // keyframes and gets one at the playhead)
    if mask.is_none() {
        let rr = Rect::from_center_size(pos2(r.max.x - 14.0, cy), vec2(12.0, 12.0));
        let rresp = ui.interact(rr, egui::Id::unique(("param-reset", clip.0, idx, pkey)), Sense::click()).on_hover_text(tl!("Reset Parameter"));
        icons::paint(ui.painter(), rr, Icon::Reset, if rresp.hovered() { t.text } else { t.text_dim });
        app.auto.add(&format!("effectControls.{}.{}.reset", e.effect, pkey), rr, "Reset Parameter");
        if rresp.clicked() {
            actions.push(("effects.resetParam".into(), json!({"clip": clip.0, "effect": eff_json, "param": pd.id})));
        }
    }
    if param.is_animated() {
        lane_keyframes(app, ui, clip, idx, e, mask, pd, param, pkey, mt, cy, actions, lane);
    }
}

/// A parameter's keyframes in the lane, on the line `y`: diamonds that drag (the selected ones
/// together, live), a line along each animated span, a right-click menu for interpolation.
#[allow(clippy::too_many_arguments)]
fn lane_keyframes(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    clip: ClipId,
    idx: usize,
    e: &EffectInstance,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    param: &filmcraft_project::Param,
    pkey: &str,
    mt: Tick,
    y: f32,
    actions: &mut Vec<(String, Value)>,
    lane: &Lane,
) {
    let t = app.tokens;
    let rate = app.session.sequence_rate();
    let lr = lane.rect;
    // the animated spans: a line from each keyframe to the next (dashed after a Hold key,
    // whose value does not change until the next one)
    let span = ui.painter().with_clip_rect(lr.intersect(ui.clip_rect()));
    for w in param.keyframes.windows(2) {
        let [first, second] = w else { continue };
        let (a, b) = (lane.x_of_media(first.time), lane.x_of_media(second.time));
        let (a, b) = (a.max(lr.min.x - 6.0), b.min(lr.max.x + 6.0));
        if b - a < 12.0 {
            continue;
        }
        let col = Color32::from_rgb(0x6a, 0x6a, 0x72);
        if first.interp == filmcraft_project::Interpolation::Hold {
            span.add(egui::Shape::dashed_line(&[pos2(a + 6.0, y), pos2(b - 6.0, y)], Stroke::new(1.0, col), 4.0, 3.0));
        } else {
            span.line_segment([pos2(a + 6.0, y), pos2(b - 6.0, y)], Stroke::new(1.0, col));
        }
    }
    let kp = ui.painter().with_clip_rect(lr.expand2(vec2(6.0, 0.0)).intersect(ui.clip_rect()));
    for k in &param.keyframes {
        let tl = lane.tl_of(k.time);
        let kx = lane.x_of_media(k.time);
        if !lane.shows(kx) {
            continue;
        }
        let me = KfSel { clip: clip.0, effect: idx, param: pd.id.to_string(), mask, time: k.time.0 };
        let selected = app.ui.kf_selected.contains(&me);
        let id = egui::Id::unique(("kf", clip.0, idx, pkey, k.time.0));
        let kr = Rect::from_center_size(pos2(kx, y), vec2(11.0, 11.0));
        let resp = ui.interact(kr.expand(2.0), id, Sense::click_and_drag());
        let resp = resp.on_hover_text(format!(
            "{}  ·  {}  ·  {}",
            filmcraft_time::format_time(tl, rate, false, filmcraft_time::TimeDisplay::Timecode, 48000),
            value_text(pd, &k.value),
            k.interp.label()
        ));
        app.auto.add(&format!("effectControls.{}.{}.keyframe.{}", e.effect, pkey, k.time.0), kr, if selected { "keyframe (selected)" } else { "keyframe" });
        // a disabled row's keyframes (Scale Width under Uniform Scale) can't be box-selected either
        if ui.is_enabled() {
            ui.data_mut(|d| d.get_temp_mut_or_default::<Vec<(Pos2, KfSel)>>(kf_pos_id()).push((kr.center(), me.clone())));
        }
        // selected: accent; at the playhead: highlighted; else grey
        let col = if selected || resp.dragged() {
            t.accent
        } else if k.time == mt {
            t.hot_text
        } else {
            Color32::from_rgb(0xb0, 0xb0, 0xb0)
        };
        match k.interp {
            filmcraft_project::Interpolation::Hold => {
                kp.rect_filled(Rect::from_center_size(kr.center(), vec2(8.0, 8.0)), 0.0, col);
            }
            filmcraft_project::Interpolation::Linear => icons::paint(&kp, kr, Icon::Keyframe, col),
            _ => {
                kp.circle_filled(kr.center(), 4.5, col);
            }
        }
        if selected {
            kp.rect_stroke(kr.expand(1.5), 2.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Outside);
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            // an unselected keyframe is picked up alone (Shift / Cmd adds it); the selected
            // ones then move together, live (`ec_keyframes::run_lane_drag`)
            let m = ui.input(|i| i.modifiers);
            if !selected {
                if !(m.shift || m.command || m.ctrl) {
                    app.ui.kf_selected.clear();
                }
                app.ui.kf_selected.push(me.clone());
            }
            app.ui.fx_selected.retain(|(c, _)| *c != clip.0);
            crate::panels::ec_keyframes::start_lane_drag(ui, clip);
        }
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            if m.shift || m.command || m.ctrl {
                if selected {
                    app.ui.kf_selected.retain(|s| *s != me);
                } else {
                    app.ui.kf_selected.push(me.clone());
                }
            } else {
                app.ui.kf_selected = vec![me.clone()];
                actions.push(("playhead.set".into(), json!({"time": tl.0})));
            }
            app.ui.fx_selected.retain(|(c, _)| *c != clip.0);
        }
        // the menu acts on the selected keyframes when this one is among them
        let menu_keys: Vec<Value> =
            if selected { app.ui.kf_selected.iter().filter(|s| s.clip == clip.0).map(KfSel::key_json).collect() } else { vec![me.key_json()] };
        resp.context_menu(|ui| keyframe_menu_checked(app, ui, clip.0, &menu_keys, Some(k.interp), actions));
    }
}

/// The value editor of one parameter row: number, point, checkbox, choice, colour (with the
/// eyedropper), path, a text layer's font and style pickers and its Source Text box, a LUT menu or
/// a text field. Returns the edited value, if any (Source Text is committed as it is typed, by
/// `graphics.setText`).
#[allow(clippy::too_many_arguments)]
fn param_value_widget(
    app: &mut FilmcraftApp,
    vui: &mut egui::Ui,
    e: &EffectInstance,
    pd: &filmcraft_project::ParamDef,
    value: &ParamValue,
    clip: ClipId,
    idx: usize,
    mask: Option<usize>,
    pkey: &str,
    id: egui::Id,
    t: Tokens,
    mt: Tick,
    it: &TrackItem,
    actions: &mut Vec<(String, Value)>,
) -> Option<Value> {
    let mut set: Option<Value> = None;
    let value_id = format!("effectControls.{}.{}.value", e.effect, pkey);
    match (&pd.kind, value) {
        (ParamKind::Float { min, max, soft_min, soft_max, unit, decimals }, ParamValue::Float(v)) => {
            let speed = ((soft_max - soft_min) / 400.0).max(0.01);
            let (vr, nv) =
                crate::widgets::hot_number(vui, crate::i18n::t(pd.label), id, *v, HotNumberOptions::new(speed, (*min, *max), *decimals as usize, unit, &t));
            app.auto.add(&value_id, vr.rect, pd.label);
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Angle, ParamValue::Float(v)) => {
            let (vr, nv) = crate::widgets::hot_number(vui, crate::i18n::t(pd.label), id, *v, HotNumberOptions::new(0.5, (-36000.0, 36000.0), 1, "°", &t));
            app.auto.add(&value_id, vr.rect, pd.label);
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Point, ParamValue::Vec2(p)) => {
            let (xr, nx) = crate::widgets::hot_number(
                vui,
                &format!("{} X", crate::i18n::t(pd.label)),
                id.with("x"),
                p.x,
                HotNumberOptions::new(1.0, (-100_000.0, 100_000.0), 1, "", &t),
            );
            let (yr, ny) = crate::widgets::hot_number(
                vui,
                &format!("{} Y", crate::i18n::t(pd.label)),
                id.with("y"),
                p.y,
                HotNumberOptions::new(1.0, (-100_000.0, 100_000.0), 1, "", &t),
            );
            app.auto.add(&format!("effectControls.{}.{}.x", e.effect, pkey), xr.rect, pd.label);
            app.auto.add(&format!("effectControls.{}.{}.y", e.effect, pkey), yr.rect, pd.label);
            if nx.is_some() || ny.is_some() {
                set = Some(json!([nx.unwrap_or(p.x), ny.unwrap_or(p.y)]));
            }
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut v = *b;
            fit_to_row(vui);
            let r = crate::widgets::accessible_name(vui.checkbox(&mut v, ""), crate::i18n::t(pd.label));
            app.auto.add(&value_id, r.rect, pd.label);
            if r.changed() {
                set = Some(json!(v));
            }
        }
        (ParamKind::Choice(opts), ParamValue::Choice(c)) => {
            let mut sel = *c as usize;
            fit_to_row(vui);
            // (narrower when the effect list is narrow, so it stays left of the divider)
            let w = vui.available_width().min(130.0);
            let shown = opts.get(sel).map_or("", |o| crate::i18n::t(o));
            let combo = craft_ui::ComboBox::from_id_salt(id).selected_text(shown).width(w).height(360.0).show_ui(vui, |ui| {
                for (i, o) in opts.iter().enumerate() {
                    let o_r = ui.selectable_value(&mut sel, i, crate::i18n::t(o));
                    app.auto.add(&format!("effectControls.{}.{}.option.{i}", e.effect, pkey), o_r.rect, o);
                    if o_r.changed() {
                        set = Some(json!(i));
                    }
                }
            });
            let r = crate::widgets::accessible_name(combo.response, crate::i18n::t(pd.label));
            app.auto.add(&value_id, r.rect, pd.label);
        }
        (ParamKind::Color, ParamValue::Color(c)) => {
            let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let r = egui::color_picker::color_edit_button_rgba(vui, &mut rgba, egui::color_picker::Alpha::Opaque);
            app.auto.add(&value_id, r.rect, pd.label);
            if r.changed() {
                set = Some(json!([rgba.r(), rgba.g(), rgba.b(), rgba.a()]));
            }
            // Eyedropper: arm it, then click a pixel in the Program monitor (Esc cancels)
            let target = crate::state::Eyedropper { clip: clip.0, effect: idx, param: pd.id.to_string(), mask };
            let armed = app.ui.eyedropper.as_ref() == Some(&target);
            let drop = icons::button(vui, Icon::Eyedropper, 18.0, armed, &t, tl!("Eyedropper (pick a color from the Program monitor)"));
            app.auto.add(&format!("effectControls.{}.{}.eyedropper", e.effect, pkey), drop.rect, "Eyedropper");
            if drop.clicked() {
                app.ui.eyedropper = if armed { None } else { Some(target) };
            }
        }
        (ParamKind::Path, _) => {
            crate::panels::masks::path_value(app, vui, clip, idx, mask, actions);
        }
        // a text layer's font and style: pickers of the system's fonts; its text: committed as it
        // is typed (so the field's own undo / redo work and the picture follows)
        (ParamKind::Text, ParamValue::Text(f)) if e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "font" => {
            let w = (vui.available_width() - 4.0).clamp(120.0, 220.0);
            if let Some(nf) = crate::panels::font_picker::font_picker(app, vui, ("ec", clip.0, idx), f, w, &format!("effectControls.{}.font", e.effect)) {
                set = Some(json!(nf));
            }
        }
        (ParamKind::Text, ParamValue::Text(st)) if e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "font_style" => {
            let family = match e.params.get("font").map(|p| p.value_at(mt)) {
                Some(ParamValue::Text(f)) => f,
                _ => String::new(),
            };
            let w = (vui.available_width() - 4.0).clamp(120.0, 220.0);
            if let Some(ns) =
                crate::panels::font_picker::style_picker(app, vui, ("ec", clip.0, idx), &family, st, w, &format!("effectControls.{}.font_style", e.effect))
            {
                set = Some(json!(ns));
            }
        }
        (ParamKind::Text, ParamValue::Text(s)) if e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "text" => {
            // a box of the text's lines (Return starts a new line), committed as typed: one undo
            // step per visit to the box
            let mut buf = s.clone();
            let rows = s.lines().count().clamp(1, TEXT_ROWS);
            let width = (vui.available_width() - 4.0).max(120.0);
            let r =
                crate::widgets::accessible_name(vui.add(egui::TextEdit::multiline(&mut buf).desired_rows(rows).desired_width(width)), crate::i18n::t(pd.label));
            app.auto.add(&format!("effectControls.text.{}.{}", idx, pd.id), r.rect, pd.label);
            let fresh_id = id.with("fresh");
            if r.gained_focus() {
                vui.data_mut(|d| d.insert_temp(fresh_id, true));
            }
            if buf != *s {
                let fresh = vui.data_mut(|d| d.remove_temp::<bool>(fresh_id)).unwrap_or(false);
                let layer = filmcraft_project::graphic::layer_indices(&it.effects).iter().position(|i| *i == idx).unwrap_or(0);
                actions.push(("graphics.setText".into(), json!({"clip": clip.0, "layer": layer, "text": buf, "merge": !fresh})));
            }
        }
        (ParamKind::Text, ParamValue::Text(s)) if !pd.id.ends_with("_lut") => {
            // edited in a buffer; committed (one undo step) when the field loses focus
            let mut buf = vui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| s.clone());
            let r = crate::widgets::accessible_name(vui.add(egui::TextEdit::singleline(&mut buf).desired_width(160.0)), crate::i18n::t(pd.label));
            app.auto.add(&format!("effectControls.text.{}.{}", idx, pd.id), r.rect, pd.label);
            crate::widgets::select_all_on_focus(vui, &r, &buf);
            if r.lost_focus() {
                if buf != *s {
                    set = Some(json!(buf));
                }
                vui.data_mut(|d| d.remove::<String>(id));
            } else if r.has_focus() {
                vui.data_mut(|d| d.insert_temp(id, buf));
            } else {
                vui.data_mut(|d| d.remove::<String>(id));
            }
        }
        // Color Grading's Input LUT / Look LUT: the Color Grading panel's menu (it showed only the name, #1123)
        (ParamKind::Text, ParamValue::Text(cur)) => {
            let input = pd.id == "input_lut";
            let auto_id = format!("effectControls.{}.{}", e.effect, pkey);
            let (rect, pick) = crate::panels::lumetri::lut_menu(vui, app, ("fx-lut", clip.0, idx, pkey), cur, input, 160.0, &auto_id);
            app.auto.add(&auto_id, rect, pd.label);
            match pick {
                Some(crate::panels::lumetri::LutPick::Lut(r)) => set = Some(json!(r)),
                Some(crate::panels::lumetri::LutPick::Browse) => {
                    let (param, clip, mask, label) = (pd.id.to_string(), clip.0, mask, pd.label);
                    app.pick_ui(crate::native_dialogs::FileDialog::open_file(tl!("LUT"), &["cube", "3dl"]), move |app, paths| {
                        let Some(path) = paths.into_iter().next() else { return };
                        // importing into the LUT library and setting the parameter undo as one step
                        let r = app.session.edit_group(label, |s| {
                            let r = s.execute("lut.import", json!({"path": path}))?["ref"].clone();
                            let mut p = json!({"clip": clip, "effect": idx, "param": param, "value": r});
                            if let Some(k) = mask {
                                p["mask"] = json!(k);
                            }
                            s.execute("effects.setParam", p)
                        });
                        if let Err(err) = r {
                            app.ui.status = err.to_string();
                        }
                    });
                }
                None => {}
            }
        }
        _ => {
            vui.label(param_text(&app.session.project, pd.id, value));
        }
    }
    set
}

/// A lane keyframe's or a graph point's menu, for it or for the selected keyframes of its clip
/// when it is one of them (`keys` as the `effects.*Keyframes` commands take them; a mask
/// parameter's keep their mask). The rows and their order are Premiere's, with no heading: Copy
/// and Clear, the five temporal interpolations, then Ease In and Ease Out (Cut, Paste and Select
/// All are the Edit menu's, by the focused panel).
pub(crate) fn keyframe_menu(app: &mut FilmcraftApp, ui: &mut egui::Ui, clip: u64, keys: &[Value], actions: &mut Vec<(String, Value)>) {
    keyframe_menu_checked(app, ui, clip, keys, None, actions);
}

/// [`keyframe_menu`] with the right-clicked keyframe's interpolation, `current`, as the checked
/// row (`None`: no row is reported as checkable).
pub(crate) fn keyframe_menu_checked(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    clip: u64,
    keys: &[Value],
    current: Option<filmcraft_project::Interpolation>,
    actions: &mut Vec<(String, Value)>,
) {
    use filmcraft_project::Interpolation;
    let response = crate::menus::checked_entry(ui, tl!("Copy"), true, None);
    app.auto.add("effectControls.keyframeMenu.copy", response.rect, "Copy");
    if response.clicked() {
        actions.push(("effects.copyKeyframes".into(), json!({"clip": clip, "keys": keys})));
        ui.close();
    }
    let response = crate::menus::checked_entry(ui, tl!("Clear"), true, None);
    app.auto.add("effectControls.keyframeMenu.clear", response.rect, "Clear");
    if response.clicked() {
        actions.push(("effects.deleteKeyframes".into(), json!({"clip": clip, "keys": keys})));
        ui.close();
    }
    for group in [
        &[
            (tl!("Linear"), "linear", Interpolation::Linear),
            (tl!("Bezier"), "bezier", Interpolation::Bezier),
            (tl!("Auto Bezier"), "autoBezier", Interpolation::AutoBezier),
            (tl!("Continuous Bezier"), "continuousBezier", Interpolation::ContinuousBezier),
            (tl!("Hold"), "hold", Interpolation::Hold),
        ][..],
        &[(tl!("Ease In"), "easeIn", Interpolation::EaseIn), (tl!("Ease Out"), "easeOut", Interpolation::EaseOut)],
    ] {
        ui.separator();
        for &(label, key, interpolation) in group {
            let response = crate::menus::checked_entry(ui, label, true, current.map(|c| c == interpolation));
            app.auto.add(&format!("effectControls.keyframeMenu.{key}"), response.rect, label);
            if response.clicked() {
                actions.push(("effects.setKeyframesInterpolation".into(), json!({"clip": clip, "keys": keys, "interpolation": key})));
                ui.close();
            }
        }
    }
}

/// Color Grading panel: edits (or adds) the Color Grading effect on the selected clip, grouped by section.
pub fn lumetri_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, _)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, tl!("Select a clip to grade"));
        return;
    };
    let idx = it.effects.iter().position(|e| e.effect == "lumetri");
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)).id_salt("lumetri"));
    bui.label(egui::RichText::new(tlf!("Master · {name}", name = it.name)).color(t.text_dim));
    let Some(idx) = idx else {
        if bui.button(tl!("Add Color Grading to clip")).clicked() {
            let _ = app.session.execute("effects.apply", json!({"clips": [clip.0], "effect": "lumetri"}));
        }
        return;
    };
    let e = it.effects[idx].clone();
    let Some(def) = e.def() else { return };
    let ph = app.session.playhead();
    let mt = it.effect_time_at(ph.clamp(it.start, it.end() - Tick(1)));
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut bui, |ui| {
        let mut groups: Vec<&str> = Vec::new();
        for p in &def.params {
            if let Some(g) = p.group
                && !groups.contains(&g)
            {
                groups.push(g);
            }
        }
        for g in groups {
            let key = format!("lumetri:{g}");
            let open = !app.ui.collapsed_fx.contains(&key);
            let (resp, now_open) = crate::widgets::section_header(ui, egui::Id::unique(&key), crate::i18n::t(g), open, &t, true);
            app.auto.add(&format!("lumetri.section.{g}"), resp.rect, g);
            if now_open != open {
                if now_open {
                    app.ui.collapsed_fx.retain(|k| *k != key);
                } else {
                    app.ui.collapsed_fx.push(key.clone());
                }
            }
            if !now_open {
                continue;
            }
            for pd in def.params.iter().filter(|p| p.group == Some(g)) {
                let v = e.params.get(pd.id).map(|p| p.value_at(mt)).unwrap_or(pd.default.clone());
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.add_sized(vec2(110.0, 18.0), egui::Label::new(egui::RichText::new(crate::i18n::t(pd.label)).size(12.0)));
                    if let (ParamKind::Float { min, max, soft_min, soft_max, .. }, ParamValue::Float(x)) = (&pd.kind, &v) {
                        let mut val = *x;
                        let s = ui.add(egui::Slider::new(&mut val, *soft_min..=*soft_max).show_value(false));
                        let (_, nv) = crate::widgets::hot_number(
                            ui,
                            crate::i18n::t(pd.label),
                            egui::Id::unique(("lum", pd.id)),
                            val,
                            HotNumberOptions::new((soft_max - soft_min) / 300.0, (*min, *max), 1, "", &t),
                        );
                        if s.changed() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": val}));
                        } else if let Some(nv) = nv {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": nv}));
                        }
                        if s.double_clicked() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": pd.default.as_f64().unwrap_or(0.0)}));
                        }
                    } else if let ParamValue::Color(c) = v {
                        let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                        if egui::color_picker::color_edit_button_rgba(ui, &mut rgba, egui::color_picker::Alpha::Opaque).changed() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": [rgba.r(), rgba.g(), rgba.b(), 1.0]}));
                        }
                    }
                });
            }
        }
    });
    run(app, ui.ctx(), actions.into_iter().map(|a| ("effects.setParam".to_string(), a)).collect());
}

/// Properties panel (Premiere 26): a compact inspector for the selected clip.
pub fn properties_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let clips = selected_clips(app);
    let Some((clip, it, kind)) = clips.first().cloned() else {
        crate::dock::placeholder(ui, rect, &t, tl!("Select a clip to see its properties"));
        return;
    };
    let ph = app.session.playhead();
    let mt = it.effect_time_at(ph.clamp(it.start, it.end() - Tick(1)));
    let mut actions: Vec<(String, Value)> = Vec::new();
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 8.0))).id_salt("props"));
    // header: clip name + menu
    let (hr, _) = bui.allocate_exact_size(vec2(bui.available_width(), 30.0), Sense::hover());
    let sw = Rect::from_center_size(pos2(hr.min.x + 8.0, hr.center().y), vec2(12.0, 12.0));
    let lc = app.session.prefs.labels.rgb(it.label);
    bui.painter().rect_filled(sw, 2.0, Color32::from_rgb(lc[0], lc[1], lc[2]));
    bui.painter().text(pos2(hr.min.x + 22.0, hr.center().y), Align2::LEFT_CENTER, &it.name, Tokens::semibold(12.5), t.text);
    let row = |ui: &mut egui::Ui, label: &str| -> Rect {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        ui.painter().text(pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(12.0), t.text_dim);
        r
    };
    let section = |ui: &mut egui::Ui, app: &mut FilmcraftApp, name: &str, reset: Option<(usize, u64)>, actions: &mut Vec<(String, Value)>| -> bool {
        ui.add_space(4.0);
        let key = format!("props:{name}");
        let open = !app.ui.collapsed_fx.contains(&key);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.min.x + 6.0, r.center().y), vec2(10.0, 10.0)),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            t.text_dim,
        );
        ui.painter().text(pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::t(name), Tokens::semibold(13.0), t.text);
        if let Some((idx, c)) = reset {
            let rr = Rect::from_center_size(pos2(r.max.x - PROPS_NAV_X, r.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), rr, Icon::Reset, t.text_dim);
            if ui.interact(rr, egui::Id::unique(("props-reset", name, c)), Sense::click()).clicked() {
                actions.push(("effects.reset".into(), json!({"clip": c, "index": idx})));
            }
        }
        if resp.clicked() {
            if open {
                app.ui.collapsed_fx.push(key);
            } else {
                app.ui.collapsed_fx.retain(|k| *k != key);
            }
        }
        open
    };
    let mut open_speed = false;
    // the sections scroll: a short panel would otherwise cut Audio and the speed button off (#1322)
    egui::ScrollArea::vertical().id_salt("props-scroll").auto_shrink([false, false]).show(&mut bui, |bui| {
        // keep the rows (and their keyframe arrows) clear of the scrollbar at the panel edge
        bui.set_max_width((bui.available_width() - 14.0).max(1.0));
        let eff_idx = |id: &str| it.effects.iter().position(|e| e.effect == id);
        let val = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).map(|p| p.value_at(mt));
        if kind == TrackKind::Video {
            if section(bui, app, "Transform", eff_idx("motion").map(|i| (i, clip.0)), &mut actions) {
                for (label, p, unit, speed, range) in [
                    (tl!("Position"), "position", "", 1.0, (-100_000.0, 100_000.0)),
                    (tl!("Anchor point"), "anchor", "", 1.0, (-100_000.0, 100_000.0)),
                    (tl!("Scale"), "scale", " %", 0.5, (0.0, 10000.0)),
                    (tl!("Rotation"), "rotation", " °", 0.5, (-36000.0, 36000.0)),
                ] {
                    let r = row(bui, label);
                    let mut vui = bui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - PROPS_VALUE_R, r.max.y - 4.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    match val("motion", p) {
                        Some(ParamValue::Vec2(v)) => {
                            let (xr, nx) = crate::widgets::hot_number(
                                &mut vui,
                                &format!("{label} X"),
                                egui::Id::unique(("pp", p, "x", clip.0)),
                                v.x,
                                HotNumberOptions::new(speed, range, 0, " X", &t),
                            );
                            vui.add_space(10.0);
                            let (yr, ny) = crate::widgets::hot_number(
                                &mut vui,
                                &format!("{label} Y"),
                                egui::Id::unique(("pp", p, "y", clip.0)),
                                v.y,
                                HotNumberOptions::new(speed, range, 0, " Y", &t),
                            );
                            app.auto.add(&format!("properties.motion.{p}.x"), xr.rect, label);
                            app.auto.add(&format!("properties.motion.{p}.y"), yr.rect, label);
                            if nx.is_some() || ny.is_some() {
                                actions.push((
                                    "effects.setParam".into(),
                                    json!({"clip": clip.0, "effect": "motion", "param": p, "value": [nx.unwrap_or(v.x), ny.unwrap_or(v.y)]}),
                                ));
                            }
                        }
                        Some(ParamValue::Float(v)) => {
                            let (vr, nv) = crate::widgets::hot_number(
                                &mut vui,
                                label,
                                egui::Id::unique(("pp", p, clip.0)),
                                v,
                                HotNumberOptions::new(speed, range, if p == "scale" { 0 } else { 1 }, unit, &t),
                            );
                            app.auto.add(&format!("properties.motion.{p}.value"), vr.rect, label);
                            if let Some(nv) = nv {
                                actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "motion", "param": p, "value": nv})));
                            }
                        }
                        _ => {}
                    }
                    properties_nav(app, bui, r, clip, &it, mt, "motion", p, &mut actions);
                }
                let r = row(bui, tl!("Opacity"));
                let mut vui = bui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - PROPS_VALUE_R, r.max.y - 4.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                if let Some(ParamValue::Float(v)) = val("opacity", "opacity") {
                    let (vr, nv) = crate::widgets::hot_number(
                        &mut vui,
                        tl!("Opacity"),
                        egui::Id::unique(("pp-op", clip.0)),
                        v,
                        HotNumberOptions::new(0.5, (0.0, 100.0), 0, " %", &t),
                    );
                    app.auto.add("properties.opacity.opacity.value", vr.rect, "Opacity");
                    if let Some(nv) = nv {
                        actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "opacity", "param": "opacity", "value": nv})));
                    }
                }
                properties_nav(app, bui, r, clip, &it, mt, "opacity", "opacity", &mut actions);
            }
            let crop = eff_idx("crop");
            if section(bui, app, "Crop", crop.map(|i| (i, clip.0)), &mut actions) {
                for (label, p) in [(tl!("Left"), "left"), (tl!("Top"), "top"), (tl!("Right"), "right"), (tl!("Bottom"), "bottom")] {
                    let r = row(bui, label);
                    let mut vui = bui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - PROPS_VALUE_R, r.max.y - 4.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let v = val("crop", p).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let (_, nv) = crate::widgets::hot_number(
                        &mut vui,
                        label,
                        egui::Id::unique(("pp-crop", p, clip.0)),
                        v,
                        HotNumberOptions::new(0.2, (0.0, 100.0), 1, " %", &t),
                    );
                    if let Some(nv) = nv {
                        if crop.is_none() {
                            actions.push(("effects.apply".into(), json!({"clips": [clip.0], "effect": "crop"})));
                        }
                        actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "crop", "param": p, "value": nv})));
                    }
                    properties_nav(app, bui, r, clip, &it, mt, "crop", p, &mut actions);
                }
            }
        }
        if let Some((clip, it, _)) = clips.iter().find(|c| c.2 == TrackKind::Audio)
            && section(bui, app, "Audio", it.effects.iter().position(|e| e.effect == "volume").map(|i| (i, clip.0)), &mut actions)
        {
            let clip = *clip;
            let mt = it.effect_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
            let val = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).map(|p| p.value_at(mt));
            let r = row(bui, tl!("Level"));
            let mut vui = bui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - PROPS_VALUE_R, r.max.y - 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let v = val("volume", "level").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let (_, nv) = crate::widgets::hot_number(
                &mut vui,
                tl!("Level"),
                egui::Id::unique(("pp-vol", clip.0)),
                v,
                HotNumberOptions::new(0.1, (-96.0, 15.0), 1, " dB", &t),
            );
            if let Some(nv) = nv {
                actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "volume", "param": "level", "value": nv})));
            }
            properties_nav(app, bui, r, clip, it, mt, "volume", "level", &mut actions);
            let r = row(bui, tl!("Pan"));
            let mut vui = bui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - PROPS_VALUE_R, r.max.y - 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let v = val("panner", "balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let (_, nv) = crate::widgets::hot_number(
                &mut vui,
                tl!("Pan"),
                egui::Id::unique(("pp-pan", clip.0)),
                v,
                HotNumberOptions::new(0.5, (-100.0, 100.0), 0, "", &t),
            );
            if let Some(nv) = nv {
                actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "panner", "param": "balance", "value": nv})));
            }
        }
        // speed footer
        bui.add_space(12.0);
        let (br, bresp) = bui.allocate_exact_size(vec2(118.0, 26.0), Sense::click());
        bui.painter().rect_filled(br, 4.0, if bresp.hovered() { t.hover } else { t.panel_bg });
        bui.painter().rect_stroke(br, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
        bui.painter().text(br.center(), Align2::CENTER_CENTER, tlf!("Speed {n}%", n = format!("{:.0}", it.speed * 100.0)), Tokens::ui(12.0), t.text);
        app.auto.add("properties.speed", br, "Speed");
        open_speed = bresp.clicked();
    });
    run(app, ui.ctx(), actions);
    // the footer button reads out the clip's speed and opens Clip ▸ Speed/Duration…, as in Premiere
    if open_speed && let Err(e) = crate::menus::invoke(app, ui.ctx(), "clip.speedDuration", json!({})) {
        app.ui.status = e;
    }
}

/// Where a row's highlight (hover, selection) is painted: the row less its first point, which is
/// where the focused panel's outline runs, so the highlight doesn't cut through the outline.
pub(crate) fn row_fill(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y), r.max)
}

/// Run the panel's actions (also the Color Grading panel's). Parameter changes made
/// while the mouse button is down (a drag) share one undo step, which the first change of each
/// press begins (#201); typed values and clicks stay separate steps. A drag value only changes once
/// the mouse moves, after the press frame, so the press that already began a step is remembered by
/// its start time. That memory is refreshed on every frame the button stays down — including frames
/// with no value change — so a Scale scrub that pauses between percents is still one undo (#887). A
/// change that brings its own `merge` key (a Program monitor handle changing several parameters)
/// keeps it.
pub(crate) fn run(app: &mut FilmcraftApp, ctx: &egui::Context, actions: Vec<(String, Value)>) {
    let (down, press) = ctx.input(|i| (i.pointer.any_down(), i.pointer.press_start_time()));
    let key = egui::Id::unique("effect-controls-drag-step");
    let begun = ctx.data(|d| d.get_temp::<Option<f64>>(key)).flatten();
    let mut wrote = false;
    for (cmd, mut p) in actions {
        if cmd == "effects.setParam" && down {
            if p.get("merge").is_none() {
                p["merge"] = json!(true);
            }
            // only the first write of a frame begins the step: a handle that changes several
            // parameters at once (position and anchor point) sends them together
            p["begin"] = json!(begun != press && !wrote);
            wrote = true;
        }
        if let Err(e) = app.session.execute(&cmd, p) {
            app.ui.status = e.to_string();
        }
    }
    if down && (wrote || begun.is_some()) {
        ctx.data_mut(|d| d.insert_temp(key, press));
    } else if !down {
        ctx.data_mut(|d| d.remove::<Option<f64>>(key));
    }
}

pub(crate) use crate::panels::ec_keyframes::graph_key;

/// Whether a selected keyframe still exists on `it`.
fn kf_exists(it: &TrackItem, k: &KfSel) -> bool {
    let Some(e) = it.effects.get(k.effect) else { return false };
    let p = match k.mask {
        Some(m) => e.masks.get(m).and_then(|x| x.param(&k.param)),
        None => e.params.get(&k.param),
    };
    p.is_some_and(|p| p.keyframes.iter().any(|kf| kf.time.0 == k.time))
}

/// Clear, Cut, Copy and Paste (without a target) by the focused panel: in Effect Controls the
/// selected keyframes, else the selected effects (of the clip they are on: the video clip or its
/// linked audio); in the Program monitor the selected graphic layers (Clear). None: the command
/// goes on as usual (the timeline selection).
pub fn route(app: &mut FilmcraftApp, id: &str) -> Option<Result<Value, String>> {
    let clear = matches!(id, "edit.clear" | "project.delete");
    match app.ui.focused {
        crate::dock::PanelKind::Program if clear => return crate::panels::graphics::clear_selected_layers(app),
        crate::dock::PanelKind::EffectControls => {}
        _ => return None,
    }
    if !matches!(id, "edit.clear" | "project.delete" | "edit.cut" | "edit.copy" | "edit.paste") {
        return None;
    }
    let clips = selected_clips(app);
    app.ui.kf_selected.retain(|k| clips.iter().any(|(c, it, _)| c.0 == k.clip && kf_exists(it, k)));
    let owner = app.ui.kf_selected.first().map(|k| k.clip).or_else(|| app.ui.fx_selected.iter().map(|(c, _)| *c).find(|c| clips.iter().any(|x| x.0.0 == *c)));
    let (clip, it, _) = match owner {
        Some(c) => clips.iter().find(|x| x.0.0 == c).cloned(),
        None => clips.first().cloned(),
    }?;
    let keys: Vec<Value> = app.ui.kf_selected.iter().filter(|k| k.clip == clip.0).map(KfSel::key_json).collect();
    let mut fx: Vec<usize> = app.ui.fx_selected.iter().filter(|(c, i)| *c == clip.0 && *i < it.effects.len()).map(|(_, i)| *i).collect();
    fx.sort_unstable();
    let exec = |app: &mut FilmcraftApp, cmd: &str, p: Value| app.session.execute(cmd, p).map_err(|e| e.to_string());
    let copy = |app: &mut FilmcraftApp| -> Option<Result<Value, String>> {
        if !keys.is_empty() {
            Some(exec(app, "effects.copyKeyframes", json!({"clip": clip.0, "keys": keys})))
        } else if !fx.is_empty() {
            Some(exec(app, "effects.copy", json!({"clip": clip.0, "indices": fx})))
        } else {
            None
        }
    };
    let delete = |app: &mut FilmcraftApp| -> Result<Value, String> {
        if !keys.is_empty() {
            let r = exec(app, "effects.deleteKeyframes", json!({"clip": clip.0, "keys": keys}));
            app.ui.kf_selected.clear();
            return r;
        }
        let removable: Vec<usize> = fx.iter().copied().filter(|i| !it.effects.get(*i).and_then(|e| e.def()).is_some_and(|d| d.intrinsic)).collect();
        if removable.is_empty() {
            return Err(if fx.is_empty() {
                "select keyframes or effects in Effect Controls to delete them (clips are deleted in the Timeline)".into()
            } else {
                "Motion, Opacity and the other fixed effects can't be removed (Reset sets them back)".into()
            });
        }
        let r = exec(app, "effects.remove", json!({"clip": clip.0, "indices": removable}));
        app.ui.fx_selected.retain(|(c, _)| *c != clip.0);
        r
    };
    match id {
        "edit.copy" => copy(app),
        "edit.cut" => match copy(app)? {
            Ok(_) => Some(delete(app)),
            Err(e) => Some(Err(e)),
        },
        "edit.paste" => match app.session.state.fx_clipboard {
            Some(filmcraft_engine::effect_edit::FxClipboard::Keyframes(_)) => {
                let r = exec(app, "effects.pasteKeyframes", json!({"clip": clip.0}));
                // the pasted keyframes become the selection
                if let Ok(v) = &r {
                    app.ui.kf_selected = v["keys"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|k| {
                                    Some(KfSel {
                                        clip: clip.0,
                                        effect: k["effect"].as_u64()? as usize,
                                        param: k["param"].as_str()?.to_string(),
                                        mask: k["mask"].as_u64().map(|m| m as usize),
                                        time: k["mediaTime"].as_i64()?,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Some(r)
            }
            Some(filmcraft_engine::effect_edit::FxClipboard::Effects(_)) => Some(exec(app, "effects.paste", json!({"clips": [clip.0]}))),
            None => None,
        },
        _ => Some(delete(app)),
    }
}

/// A keyframe's value as the panel shows it (tooltips).
fn value_text(pd: &filmcraft_project::ParamDef, v: &ParamValue) -> String {
    match (&pd.kind, v) {
        (ParamKind::Float { decimals, unit, .. }, ParamValue::Float(x)) => format!("{x:.prec$}{unit}", prec = *decimals as usize),
        (ParamKind::Angle, ParamValue::Float(x)) => format!("{x:.1}°"),
        (_, ParamValue::Float(x)) => format!("{x:.2}"),
        (_, ParamValue::Vec2(p)) => format!("{:.1}, {:.1}", p.x, p.y),
        (_, ParamValue::Bool(b)) => if *b { "On" } else { "Off" }.to_string(),
        (ParamKind::Choice(opts), ParamValue::Choice(c)) => opts.get(*c as usize).copied().unwrap_or("").to_string(),
        (_, other) => format!("{other:?}"),
    }
}

/// Read-only text for parameters without an inline editor (LUT references, free text, curves).
fn param_text(project: &filmcraft_project::Project, id: &str, value: &ParamValue) -> String {
    match value {
        ParamValue::Text(s) if id.ends_with("_lut") => filmcraft_render::luts::label(Some(project), s),
        ParamValue::Text(s) if s.is_empty() => tl!("None").into(),
        ParamValue::Text(s) => s.clone(),
        ParamValue::Curve(pts) if pts.is_empty() || *pts == [[0.0, 0.0], [1.0, 1.0]] => tl!("Default").into(),
        ParamValue::Curve(pts) => tlf!("Custom ({n} points)", n = pts.len()),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod param_text_tests {
    use super::*;

    #[test]
    fn keyframe_timeline_coordinates_follow_trim_speed_and_reverse() {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let mut it = session.active_sequence().unwrap().video_tracks[0].items[0].clone();
        it.start += Tick::from_seconds_f64(4.0);
        it.source_in += Tick::from_seconds_f64(3.0);
        for speed in [0.5, 1.0, 1.5, 2.0] {
            for reverse in [false, true] {
                it.speed = speed;
                it.reverse = reverse;
                for time in [it.start, it.start + Tick(it.duration.0 / 4), it.end() - session.sequence_rate().frame_duration()] {
                    let media = it.moving_source_time_at(time);
                    assert!((timeline_time_of(&it, media) - time).abs() <= Tick(2), "speed={speed}, reverse={reverse}");
                }
            }
        }
    }

    #[test]
    fn readable_values_for_text_lut_and_curve_params() {
        let p = filmcraft_project::Project::default();
        assert_eq!(param_text(&p, "input_lut", &ParamValue::Text(String::new())), "None");
        assert_eq!(param_text(&p, "look_lut", &ParamValue::Text("builtin:look-teal-orange".into())), "Teal & Orange");
        assert_eq!(param_text(&p, "label", &ParamValue::Text(String::new())), "None");
        assert_eq!(param_text(&p, "label", &ParamValue::Text("Reel 3".into())), "Reel 3");
        assert_eq!(param_text(&p, "curve_luma", &ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]])), "Default");
        assert_eq!(param_text(&p, "hue_vs_sat", &ParamValue::Curve(vec![])), "Default");
        assert_eq!(param_text(&p, "curve_luma", &ParamValue::Curve(vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]])), "Custom (3 points)");
    }
}

#[cfg(test)]
mod drag_undo_tests {
    use serde_json::json;

    /// #201: a drag value changes only once the mouse moves, after the press frame; two drags of
    /// the same parameter are still two undo steps.
    #[test]
    fn each_press_begins_its_own_undo_step() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        let mut app = crate::FilmcraftApp::new(s);
        let ctx = egui::Context::default();
        let opacity = |app: &crate::FilmcraftApp| {
            let q = app.session.active_sequence().unwrap();
            let it = q.find_item(filmcraft_project::ClipId(clip)).unwrap().1;
            it.effects.iter().find(|e| e.effect == "opacity").unwrap().params["opacity"].value.as_f64().unwrap()
        };
        let start = opacity(&app);
        let pos = egui::pos2(10.0, 10.0);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut time = 0.0;
        let mut frame = |app: &mut crate::FilmcraftApp, events: Vec<egui::Event>, value: Option<f64>| {
            time += 0.1;
            let raw = egui::RawInput { time: Some(time), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let actions = value.map(|v| ("effects.setParam".to_string(), json!({"clip": clip, "effect": "opacity", "param": "opacity", "value": v})));
                super::run(app, ui.ctx(), actions.into_iter().collect());
            });
            out.textures_delta.clear();
        };
        for values in [[90.0, 70.0], [50.0, 60.0]] {
            frame(&mut app, vec![egui::Event::PointerMoved(pos), button(true)], None); // the press: nothing changes yet
            for v in values {
                frame(&mut app, vec![egui::Event::PointerMoved(pos)], Some(v));
            }
            frame(&mut app, vec![button(false)], None);
        }
        assert_eq!(opacity(&app), 60.0);
        app.session.undo();
        assert_eq!(opacity(&app), 70.0, "undo takes back only the second drag");
        app.session.undo();
        assert_eq!(opacity(&app), start, "and then the first");
    }

    /// #887: a Scale hot-text drag that pauses between percents (no `setParam` that frame) is
    /// still one undo, not one step per 1%.
    #[test]
    fn a_scale_drag_with_idle_frames_is_one_undo_step() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        let mut app = crate::FilmcraftApp::new(s);
        let ctx = egui::Context::default();
        let scale = |app: &crate::FilmcraftApp| {
            let q = app.session.active_sequence().unwrap();
            let it = q.find_item(filmcraft_project::ClipId(clip)).unwrap().1;
            it.effects.iter().find(|e| e.effect == "motion").unwrap().params["scale"].value.as_f64().unwrap()
        };
        let start = scale(&app);
        let pos = egui::pos2(10.0, 10.0);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut time = 0.0;
        let mut frame = |app: &mut crate::FilmcraftApp, events: Vec<egui::Event>, value: Option<f64>| {
            time += 0.1;
            let raw = egui::RawInput { time: Some(time), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let actions = value.map(|v| ("effects.setParam".to_string(), json!({"clip": clip, "effect": "motion", "param": "scale", "value": v})));
                super::run(app, ui.ctx(), actions.into_iter().collect());
            });
            out.textures_delta.clear();
        };
        frame(&mut app, vec![egui::Event::PointerMoved(pos), button(true)], None);
        for v in [101.0, 150.0, 200.0, 249.0] {
            frame(&mut app, vec![egui::Event::PointerMoved(pos)], Some(v));
            frame(&mut app, vec![egui::Event::PointerMoved(pos)], None); // idle: mouse down, no 1% tick
        }
        frame(&mut app, vec![button(false)], None);
        assert_eq!(scale(&app), 249.0);
        app.session.undo();
        assert_eq!(scale(&app), start, "one undo restores the scale from before the drag");
        assert_eq!(app.session.undo(), None, "the drag must not have left extra 1% steps");
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    /// A context with the app's fonts (the panels use the semibold family).
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &Tokens::for_kind(Default::default()));
        ctx
    }

    /// A project file is only checked for overflow, so a damaged one can carry a clip with no
    /// (or a negative) duration. Selecting it must still draw both panels, ruler, playhead and
    /// keyframe navigators included.
    #[test]
    fn a_clip_without_a_duration_still_draws() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
        s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
        s.execute("effects.toggleAnimation", json!({"clip": clip.0, "effect": "motion", "param": "scale"})).unwrap();
        let seq = s.state.active_sequence.unwrap();
        let mut app = crate::FilmcraftApp::new(s);
        let ctx = context();
        for duration in [0, -5 * filmcraft_time::TICKS_PER_SECOND] {
            let project = std::sync::Arc::make_mut(&mut app.session.project);
            project.sequence_mut(seq).unwrap().find_item_mut(clip).unwrap().1.duration = Tick(duration);
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 500.0));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(&mut app, ui, rect);
                properties_panel(&mut app, ui, rect);
            });
            out.textures_delta.clear();
        }
    }

    /// The ruler takes its numbers from the project (clip times, the sequence's frame rate) and
    /// the panel's size. Anything a project that opens can carry (times within `Tick::MIN..=MAX`,
    /// a rate within `SequenceSettings::validate`, or an unset one) must not make it panic or
    /// tick forever.
    #[test]
    fn ruler_survives_damaged_numbers() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let mut it = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        let ctx = context();
        let t = Tokens::for_kind(Default::default());
        let big = i64::from(u32::MAX);
        let rates = [(24000, 1001), (30, 1), (1000, 1), (1, 1), (1, big), (big, big), (0, 0), (-25, 1)];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for width in [300.0, 0.0, -40.0, 0.001, 1.0e9, f32::NAN, f32::INFINITY] {
                for duration in [1, 0, -7, filmcraft_time::TICKS_PER_SECOND, Tick::MAX.0 / 2] {
                    for start in [0, Tick::MIN.0, Tick::MAX.0 / 2] {
                        for (num, den) in rates {
                            it.start = Tick(start);
                            it.duration = Tick(duration);
                            let ruler = Rect::from_min_size(pos2(10.0, 10.0), vec2(width, RULER_H));
                            paint_ruler(ui.painter(), ruler, it.start, it.end(), filmcraft_time::FrameRate { num, den }, false, &t);
                        }
                    }
                }
            }
        });
        out.textures_delta.clear();
    }

    /// A five-second lane 300 px wide is labelled every two seconds, in sequence timecode.
    #[test]
    fn ruler_labels_sequence_timecode() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let mut it = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        let rate = filmcraft_time::FrameRate { num: 24, den: 1 };
        it.start = rate.tick_of(24 * 60);
        it.duration = rate.tick_of(24 * 5);
        let ctx = context();
        let t = Tokens::for_kind(Default::default());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_ruler(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, RULER_H)), it.start, it.end(), rate, false, &t);
        });
        out.textures_delta.clear();
        fn texts(s: egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Vec(v) => v.into_iter().for_each(|s| texts(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        let mut labels = Vec::new();
        out.shapes.into_iter().for_each(|c| texts(c.shape, &mut labels));
        assert_eq!(labels, ["00:01:00:00", "00:01:02:00", "00:01:04:00"]);
    }
}

#[cfg(test)]
mod frame_hold_tests {
    //! #1675: on a Frame Hold clip with Hold Filters off the effects keep animating through the
    //! hold, so the panels that show effect values read them where the render does, as Effect
    //! Controls does, and not on the held frame.
    use super::*;
    use crate::panels::audio_fx_editor::FxTarget;
    use filmcraft_geom::Vec2;
    use filmcraft_project::{Keyframe, Param};

    #[derive(Clone, Copy, Debug)]
    enum Panel {
        Lumetri,
        Masks,
        TransformHandles,
        AudioFxEditor,
    }

    fn animate(it: &mut TrackItem, effect: &str, param: &str, from: ParamValue, to: ParamValue) {
        let (a, b) = (it.moving_source_time_at(it.start), it.moving_source_time_at(it.end() - Tick(1)));
        let e = it.effects.iter_mut().find(|e| e.effect == effect).unwrap();
        e.params.insert(param.into(), Param { value: from.clone(), keyframes: vec![Keyframe::new(a, from), Keyframe::new(b, to)] });
    }

    /// A V1 clip and an A1 clip under the playhead, their effects animated from
    /// their In to their Out, each held on its In (`Some(hold_filters)`) or not held.
    fn app(hold: Option<bool>) -> FilmcraftApp {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let v = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        // away from the In, where the held and the moving times meet
        let ph = v.start + Tick(v.duration.0 / 2);
        s.execute("playhead.set", json!({"time": ph.0})).unwrap();
        let a = s.active_sequence().unwrap().audio_tracks.iter().find_map(|t| t.item_at(ph)).unwrap().id;
        s.execute("effects.apply", json!({"clips": [v.id.0], "effect": "lumetri"})).unwrap();
        s.execute("effects.apply", json!({"clips": [a.0], "effect": "parametric_eq"})).unwrap();
        s.execute("timeline.select", json!({"clips": [v.id.0]})).unwrap();
        s.execute("masks.add", json!({"clip": v.id.0, "effect": "opacity", "shape": "ellipse"})).unwrap();
        s.execute("effects.select", json!({"clip": v.id.0, "effect": "motion"})).unwrap();
        if let Some(hold_filters) = hold {
            s.execute("clip.frameHoldOptions", json!({"clips": [v.id.0], "holdOn": "in", "holdFilters": hold_filters})).unwrap();
        }
        let seq = s.state.active_sequence.unwrap();
        let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap();
        let it = q.find_item_mut(v.id).unwrap().1;
        animate(it, "motion", "position", ParamValue::Vec2(Vec2::new(400.0, 300.0)), ParamValue::Vec2(Vec2::new(800.0, 500.0)));
        animate(it, "lumetri", "saturation", ParamValue::Float(50.0), ParamValue::Float(150.0));
        let it = q.find_item_mut(a).unwrap().1;
        animate(it, "parametric_eq", "b1_gain", ParamValue::Float(-12.0), ParamValue::Float(12.0));
        // Frame Hold Options takes video clips; an imported edit list can hold an audio clip
        if let Some(hold_filters) = hold {
            (it.frame_hold, it.hold_filters) = (Some(it.source_in), hold_filters);
        }
        let index = it.effects.iter().position(|e| e.effect == "parametric_eq").unwrap();
        let mut app = FilmcraftApp::new(s);
        app.ui.audio_fx_editors.push(FxTarget::Clip { clip: a.0, index });
        app
    }

    /// What `panel` draws, after a few frames (the editor window sizes itself on the first one).
    fn observe(mut app: FilmcraftApp, panel: Panel) -> String {
        let q = app.session.active_sequence().unwrap();
        let frame = (q.settings.width, q.settings.height);
        let pic = Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 360.0));
        // a selected mask and a selected effect exclude each other
        if let Panel::Masks = panel {
            let clip = app.session.state.selection[0];
            app.session.execute("masks.select", json!({"clip": clip.0, "effect": "opacity", "mask": 0})).unwrap();
        }
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &app.tokens);
        let mut shapes = String::new();
        for i in 0..3 {
            app.auto.begin_frame();
            let mut out = ctx.run_ui(egui::RawInput { time: Some(f64::from(i) * 0.1), ..Default::default() }, |ui| match panel {
                Panel::Lumetri => crate::panels::lumetri::show(&mut app, ui, Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 900.0))),
                Panel::Masks => crate::panels::masks::monitor_overlay(&mut app, ui, pic, frame),
                Panel::TransformHandles => crate::panels::transform_handles::monitor_overlay(&mut app, ui, pic, frame),
                Panel::AudioFxEditor => crate::panels::audio_fx_editor::show(&mut app, ui.ctx()),
            });
            out.textures_delta.clear();
            shapes = format!("{:?}", out.shapes);
        }
        shapes
    }

    #[test]
    fn panels_show_effect_values_where_the_render_takes_them_on_a_frame_hold() {
        for panel in [Panel::Lumetri, Panel::Masks, Panel::TransformHandles, Panel::AudioFxEditor] {
            let moving = observe(app(None), panel);
            let held = observe(app(Some(true)), panel);
            assert!(moving != held, "{panel:?}: with Hold Filters on it shows the held frame's values");
            assert!(observe(app(Some(false)), panel) == moving, "{panel:?}: with Hold Filters off it shows the values it shows without the hold");
        }
    }
}

#[cfg(test)]
mod uniform_scale_tests {
    //! #4381: with Uniform Scale on, Scale Width does nothing (Motion and Transform scale both
    //! axes by Scale / Scale Height), so its row is greyed out and its value can't be changed.
    use super::*;

    /// Drags the value of `effect`'s Scale Width in the panel, then clicks it, and returns its
    /// value afterwards and whether the click opened it for typing.
    fn drag_scale_width(effect: &str, uniform: bool) -> (f64, bool) {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
        if effect != "motion" {
            s.execute("effects.apply", json!({"clips": [clip.0], "effect": effect})).unwrap();
        }
        s.execute("effects.setParam", json!({"clip": clip.0, "effect": effect, "param": "uniform_scale", "value": uniform})).unwrap();
        s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
        let mut app = FilmcraftApp::new(s);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &app.tokens);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 1600.0));
        let mut time = 0.0;
        let mut frame = |app: &mut FilmcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            app.auto.begin_frame();
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), events, ..Default::default() }, |ui| show(app, ui, rect));
            out.textures_delta.clear();
        };
        let effects = |app: &FilmcraftApp| app.session.active_sequence().unwrap().find_item(clip).unwrap().1.effects.clone();
        let idx = effects(&app).iter().position(|e| e.effect == effect).unwrap();
        let width = |app: &FilmcraftApp| effects(app)[idx].params["scale_width"].value.as_f64().unwrap();
        frame(&mut app, vec![]);
        let [x, y, w, h] = app.auto.find(&format!("effectControls.{effect}.{idx}.scale_width.name")).expect("the Scale Width row is drawn").rect;
        // the value starts 6 points right of the name
        let start = pos2(x + w + 12.0, y + h / 2.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(start), button(start, true)]);
        for dx in [20.0, 40.0, 60.0] {
            frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(dx, 0.0))]);
        }
        frame(&mut app, vec![button(start + vec2(60.0, 0.0), false)]);
        frame(&mut app, vec![]);
        let dragged = width(&app);
        frame(&mut app, vec![egui::Event::PointerMoved(start), button(start, true)]);
        frame(&mut app, vec![button(start, false)]);
        frame(&mut app, vec![]);
        let pkey = egui::Id::unique(("pv", clip.0, idx, "scale_width"));
        let typing = ctx.data(|d| d.get_temp::<String>(pkey.with("editing")).is_some());
        (dragged, typing)
    }

    #[test]
    fn scale_width_is_greyed_out_while_uniform_scale_is_on() {
        for effect in ["motion", "transform"] {
            let (width, typing) = drag_scale_width(effect, false);
            assert!(width > 100.0, "{effect}: with Uniform Scale off, dragging Scale Width changes it");
            assert!(typing, "{effect}: with Uniform Scale off, a click on Scale Width types a value");
            assert_eq!(drag_scale_width(effect, true), (100.0, false), "{effect}: with Uniform Scale on, Scale Width can't be dragged or typed");
        }
    }

    /// Animates Motion's Scale Width, opens its graphs, drags its second keyframe (mid-clip, clear
    /// of the divider) in the value graph and returns that keyframe's value afterwards.
    fn drag_scale_width_graph(uniform: bool) -> f64 {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (clip, start, dur) = {
            let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
            (it.id, it.start, it.duration)
        };
        for at in [start, start + Tick(dur.0 / 2)] {
            s.execute("effects.addKeyframe", json!({"clip": clip.0, "effect": "motion", "param": "scale_width", "time": at.0})).unwrap();
        }
        s.execute("effects.setParam", json!({"clip": clip.0, "effect": "motion", "param": "uniform_scale", "value": uniform})).unwrap();
        s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
        let mut app = FilmcraftApp::new(s);
        let effects = |app: &FilmcraftApp| app.session.active_sequence().unwrap().find_item(clip).unwrap().1.effects.clone();
        let idx = effects(&app).iter().position(|e| e.effect == "motion").unwrap();
        let second = |app: &FilmcraftApp| effects(app)[idx].params["scale_width"].keyframes[1].clone();
        let key = second(&app).time;
        // opened before Uniform Scale was turned on
        app.ui.expanded_fx.push(graph_key(clip, idx, "scale_width"));
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &app.tokens);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 1600.0));
        let mut time = 0.0;
        let mut frame = |app: &mut FilmcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            app.auto.begin_frame();
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), events, ..Default::default() }, |ui| show(app, ui, rect));
            out.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        let [x, y, w, h] = app.auto.find(&format!("effectControls.scale_width.graph.keyframe.{}", key.0)).expect("the value graph is drawn").rect;
        let at = pos2(x + w / 2.0, y + h / 2.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        for dy in [10.0, 20.0, 30.0] {
            frame(&mut app, vec![egui::Event::PointerMoved(at - vec2(0.0, dy))]);
        }
        frame(&mut app, vec![button(at - vec2(0.0, 30.0), false)]);
        frame(&mut app, vec![]);
        second(&app).value.as_f64().unwrap()
    }

    #[test]
    fn scale_width_graph_is_greyed_out_while_uniform_scale_is_on() {
        assert!(drag_scale_width_graph(false) > 100.0, "with Uniform Scale off, the value graph changes Scale Width");
        assert_eq!(drag_scale_width_graph(true), 100.0, "with Uniform Scale on, the value graph can't change Scale Width");
    }

    /// Keys Scale and Scale Width with Uniform Scale off, selects every keyframe (Select All), turns
    /// Uniform Scale on and drags the selected second Scale keyframe right. Returns the app
    /// afterwards, whether that Scale keyframe moved and whether Scale Width's keyframes stayed put.
    /// (Keyframes move live while they are dragged here, so Uniform Scale turned on part-way through
    /// a drag leaves Scale Width's where the drag had taken them by then.)
    fn drag_scale_keyframe() -> (FilmcraftApp, bool, bool) {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (clip, start, dur) = {
            let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
            (it.id, it.start, it.duration)
        };
        for param in ["scale", "scale_width"] {
            for at in [start, start + Tick(dur.0 / 2)] {
                s.execute("effects.addKeyframe", json!({"clip": clip.0, "effect": "motion", "param": param, "time": at.0})).unwrap();
            }
        }
        let uniform = |on: bool| json!({"clip": clip.0, "effect": "motion", "param": "uniform_scale", "value": on});
        s.execute("effects.setParam", uniform(false)).unwrap();
        s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
        let mut app = FilmcraftApp::new(s);
        let effects = |app: &FilmcraftApp| app.session.active_sequence().unwrap().find_item(clip).unwrap().1.effects.clone();
        let idx = effects(&app).iter().position(|e| e.effect == "motion").unwrap();
        let times = |app: &FilmcraftApp, param: &str| effects(app)[idx].params[param].keyframes.iter().map(|k| k.time).collect::<Vec<_>>();
        app.ui.focused = crate::dock::PanelKind::EffectControls;
        assert!(select_all_keyframes(&mut app));
        assert_eq!(
            app.ui.kf_selected.iter().filter(|k| k.param == "scale_width").count(),
            2,
            "with Uniform Scale off, Select All takes Scale Width's keyframes"
        );
        app.session.execute("effects.setParam", uniform(true)).unwrap();
        let (before, mid) = (times(&app, "scale_width"), times(&app, "scale")[1]);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &app.tokens);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 1600.0));
        let mut time = 0.0;
        let mut frame = |app: &mut FilmcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            app.auto.begin_frame();
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), events, ..Default::default() }, |ui| show(app, ui, rect));
            out.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        let [x, y, w, h] = app.auto.find(&format!("effectControls.motion.scale.keyframe.{}", mid.0)).expect("Scale's keyframe is drawn").rect;
        let at = pos2(x + w / 2.0, y + h / 2.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        for dx in [10.0, 20.0, 40.0] {
            frame(&mut app, vec![egui::Event::PointerMoved(at + vec2(dx, 0.0))]);
        }
        frame(&mut app, vec![button(at + vec2(40.0, 0.0), false)]);
        frame(&mut app, vec![]);
        let moved = times(&app, "scale")[1] != mid;
        let stayed = times(&app, "scale_width") == before;
        (app, moved, stayed)
    }

    /// Scale Width keyframes selected while Uniform Scale was off (Select All) leave the selection
    /// once it's on: dragging a selected Scale keyframe doesn't move them, and Select All skips them.
    #[test]
    fn scale_width_keyframes_leave_the_selection_while_uniform_scale_is_on() {
        let (mut app, moved, stayed) = drag_scale_keyframe();
        assert!(moved, "the selected Scale keyframes move");
        assert!(stayed, "Scale Width's keyframes, selected before Uniform Scale was on, stay put");
        assert!(select_all_keyframes(&mut app));
        assert!(app.ui.kf_selected.iter().all(|k| k.param != "scale_width"), "with Uniform Scale on, Select All skips Scale Width's keyframes");
    }

    /// Whether the render ignores the Scale Width of effect `idx` of `it` at media time `mt`: the
    /// result is the same with Scale Width at 50 % as at 200 %. Motion through
    /// `filmcraft_render::motion_matrix`, graphic layers through `item_layer_specs`.
    fn render_ignores_scale_width(seq: &filmcraft_project::Sequence, it: &TrackItem, idx: usize, mt: Tick) -> bool {
        let with = |width: f64| {
            let mut it = it.clone();
            let p = it.effects[idx].params.get_mut("scale_width").unwrap();
            (p.value, p.keyframes) = (ParamValue::Float(width), Vec::new());
            if it.effects[idx].effect == "motion" {
                format!("{:?}", filmcraft_render::motion_matrix(seq, &it, (1920, 1080), None, mt))
            } else {
                let specs = filmcraft_render::graphic_clip::item_layer_specs(&it, mt, (1920, 1080));
                format!("{:?}", specs.iter().find(|(i, _)| *i == idx).unwrap().1.transform)
            }
        };
        with(50.0) == with(200.0)
    }

    /// Uniform Scale is read as the render reads it, so Scale Width is greyed out exactly when the
    /// render ignores it: Motion and Transform read the stored value even when `effects.addKeyframe`
    /// gave Uniform Scale a keyframe that differs from it, graphic layers read the keyframe; a
    /// missing or non-boolean Uniform Scale (an older or damaged project) counts as each reads it.
    #[test]
    fn uniform_scale_counts_as_the_render_reads_it() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (clip, start) = {
            let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
            (it.id, it.start)
        };
        // Motion's Uniform Scale on, with a keyframe that says off (the command allows it)
        let added = s.execute("effects.addKeyframe", json!({"clip": clip.0, "effect": "motion", "param": "uniform_scale", "time": start.0})).unwrap();
        let mt = Tick(added["mediaTime"].as_i64().unwrap());
        s.execute("effects.setKeyframe", json!({"clip": clip.0, "effect": "motion", "param": "uniform_scale", "mediaTime": mt.0, "value": false})).unwrap();
        let seq = s.active_sequence().unwrap().clone();
        let it = seq.find_item(clip).unwrap().1.clone();
        let motion = it.effects.iter().position(|e| e.effect == "motion").unwrap();
        let uniform = |it: &TrackItem, idx: usize| it.effects[idx].params["uniform_scale"].clone();
        assert_eq!(uniform(&it, motion).value, ParamValue::Bool(true));
        assert_eq!(uniform(&it, motion).value_at(mt), ParamValue::Bool(false), "the keyframe differs from the stored value");
        // the same on a graphic layer and on Transform, each also with Uniform Scale missing and non-boolean
        let mut it = it;
        it.effects.push(filmcraft_project::graphic::new_text_layer("A", Default::default(), 20.0));
        it.effects.push(filmcraft_project::find_effect("transform").unwrap().instance());
        let (layer, transform) = (it.effects.len() - 2, it.effects.len() - 1);
        for idx in [layer, transform] {
            let p = it.effects[idx].params.get_mut("uniform_scale").unwrap();
            p.add_keyframe(mt);
            p.keyframes[0].value = ParamValue::Bool(false);
        }
        let variants = |idx: usize| {
            let mut missing = it.clone();
            missing.effects[idx].params.remove("uniform_scale");
            let mut odd = it.clone();
            odd.effects[idx].params.get_mut("uniform_scale").unwrap().value = ParamValue::Float(1.0);
            [("keyframed", it.clone()), ("missing", missing), ("non-boolean", odd)]
        };
        for idx in [motion, layer] {
            for (case, it) in variants(idx) {
                let e = &it.effects[idx];
                let render = render_ignores_scale_width(&seq, &it, idx, mt);
                assert_eq!(ignored_by_uniform_scale(&it, e, "scale_width", mt), render, "{} ({case})", e.effect);
            }
        }
        // the render ignores Motion's Scale Width by its stored value, a graphic layer's by the keyframe
        assert!(render_ignores_scale_width(&seq, &variants(motion)[0].1, motion, mt));
        assert!(!render_ignores_scale_width(&seq, &variants(layer)[0].1, layer, mt));
        // Transform (`gpufx`, `vfx::distort`): the stored value, missing or non-boolean = off
        for ((case, it), render) in variants(transform).into_iter().zip([true, false, false]) {
            assert_eq!(ignored_by_uniform_scale(&it, &it.effects[transform], "scale_width", mt), render, "transform ({case})");
        }
    }
}
