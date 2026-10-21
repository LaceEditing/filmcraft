//! Effect Controls: the selected clip's effects (fixed Motion/Opacity/Time Remapping first, as in
//! Premiere, then standard effects), generated from parameter schemas, with stopwatches and a
//! keyframe lane on the right. Also hosts the Lumetri Color panel body (same editor, grouped).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::{ClipId, EffectInstance, ParamKind, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::state::KfSel;
use crate::theme::Tokens;

const ROW_H: f32 = 22.0;
/// Most lines the Source Text box shows at once.
const TEXT_ROWS: usize = 6;
/// Height of the keyframe lane's time ruler.
const RULER_H: f32 = 16.0;

/// The keyframe lane's time ruler: ticks and sequence timecodes over the clip `start` +
/// `duration`. Steps are whole frames or whole seconds of timecode (so labels read 00:00:02:00,
/// not a rounded 00:00:01:23), spaced so the labels never crowd; a label that would run off the
/// lane is left out.
fn lane_ruler(ui: &egui::Ui, r: Rect, start: Tick, duration: Tick, rate: filmcraft_time::FrameRate, drop_frame: bool, t: &Tokens) {
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 0.0, Color32::from_rgb(0x26, 0x26, 0x26));
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
    let span = duration.0.max(1) as f64;
    let x_of = |tk: Tick| r.min.x + ((tk - start).0 as f64 / span) as f32 * r.width();
    let nominal = rate.as_f64().round().max(1.0) as i64;
    let total_frames = (rate.frame_at(start + duration) - rate.frame_at(start)).max(1);
    let px_per_frame = r.width() as f64 / total_frames as f64;
    // label every `step` frames: 1, 2, 5, 10 frames, then 1, 2, 5, 10, 15, 30 s, 1, 2, 5, 10, 15, 30 min, 1 h
    let steps = [1, 2, 5, 10].into_iter().chain([1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600].into_iter().map(|s| s * nominal));
    let step = steps.clone().find(|s| *s as f64 * px_per_frame >= 90.0).unwrap_or(3600 * nominal);
    let minor = match step {
        s if s >= 5 && s % 5 == 0 => s / 5,
        s if s % 2 == 0 => s / 2,
        s => s,
    }
    .max(1);
    let (f0, f1) = (rate.frame_at(start), rate.frame_at(start + duration));
    let mut f = (f0 + minor - 1).div_euclid(minor) * minor;
    let mut n = 0;
    while f <= f1 && n < 4000 {
        let x = x_of(rate.tick_of(f));
        let major = f % step == 0;
        let h = if major { 7.0 } else { 3.0 };
        p.line_segment([pos2(x, r.max.y - h), pos2(x, r.max.y)], Stroke::new(1.0, if major { t.text_dim } else { t.text_faint }));
        if major {
            let tc = filmcraft_time::format_time(rate.tick_of(f), rate, drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
            let g = p.layout_no_wrap(tc, Tokens::ui(9.5), t.text_dim);
            if x + 3.0 + g.size().x <= r.max.x {
                p.galley(pos2(x + 3.0, r.min.y + 1.0), g, t.text_dim);
            }
        }
        f += minor;
        n += 1;
    }
}

fn selected_clip(app: &FilmcraftApp) -> Option<(ClipId, TrackItem, filmcraft_project::TrackKind)> {
    let seq = app.session.active_sequence()?;
    let mut best = None;
    for c in &app.session.state.selection {
        if let Some((tid, it)) = seq.find_item(*c) {
            let kind = seq.track(tid)?.kind;
            if kind == filmcraft_project::TrackKind::Video {
                return Some((*c, it.clone(), kind));
            }
            best.get_or_insert((*c, it.clone(), kind));
        }
    }
    best
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, kind)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "(no clip selected)");
        return;
    };
    let Some(seq) = app.session.active_sequence().cloned() else {
        crate::dock::placeholder(ui, rect, &t, "(no sequences)");
        return;
    };
    // the divider between the property list and the keyframe area drags
    let split = rect.min.x + (rect.width() * app.ui.ec_split.clamp(0.2, 0.85)).max(200.0).min(rect.width() - 80.0);
    let head = Rect::from_min_size(rect.min + vec2(8.0, 4.0), vec2(split - rect.min.x - 12.0, 24.0));
    // Premiere: two pill tabs — "Source · clip" and "Sequence · clip" (active)
    let pill = |ui: &mut egui::Ui, r: Rect, text: &str, active: bool| {
        ui.painter().rect_filled(r, 4.0, if active { Color32::from_rgb(0x3a, 0x3a, 0x3a) } else { t.panel_bg });
        if !active {
            ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
        }
        let cp = ui.painter().with_clip_rect(r.shrink(2.0));
        cp.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, text, Tokens::ui(11.5), if active { t.text } else { t.text_dim });
    };
    let half = (head.width() - 6.0) / 2.0;
    pill(ui, Rect::from_min_size(head.min, vec2(half, 24.0)), &format!("Source · {}", it.name), false);
    pill(ui, Rect::from_min_size(head.min + vec2(half + 6.0, 0.0), vec2(half, 24.0)), &format!("{} · {}", seq_name(app), it.name), true);
    let divider = Rect::from_min_max(pos2(split - 2.0, rect.min.y), pos2(split + 4.0, rect.max.y));
    let dresp = ui.interact(divider, egui::Id::new("ec-divider"), Sense::drag());
    if dresp.hovered() || dresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().line_segment([pos2(split + 1.0, rect.min.y), pos2(split + 1.0, rect.max.y)], Stroke::new(2.0, t.accent));
    }
    if dresp.dragged() {
        app.ui.ec_split = (app.ui.ec_split + dresp.drag_delta().x / rect.width().max(1.0)).clamp(0.2, 0.85);
    }
    app.auto.add("effectControls.divider", divider, "divider");
    // keyframe area: a ruler over the part of the clip it shows, the clip bar, the zoom bar
    let lane_rect = Rect::from_min_max(pos2(split + 4.0, rect.min.y + 4.0), pos2(rect.max.x - 6.0, rect.max.y - 26.0));
    ui.painter().rect_filled(lane_rect, 0.0, t.tl_bg);
    let ph = app.session.playhead();
    let rate = seq.settings.frame_rate;
    let lane = crate::panels::ec_keyframes::Lane::new(lane_rect, &it, app.ui.ec_view, rate.frame_duration());
    crate::panels::ec_keyframes::wheel(app, ui, &lane, clip);
    let lane = crate::panels::ec_keyframes::Lane::new(lane_rect, &it, app.ui.ec_view, rate.frame_duration());
    let ruler = Rect::from_min_max(lane_rect.min, pos2(lane_rect.max.x, lane_rect.min.y + RULER_H));
    lane_ruler(ui, ruler, lane.t0, lane.t1 - lane.t0, rate, seq.settings.drop_frame, &t);
    let bar_row = Rect::from_min_max(pos2(lane_rect.min.x, ruler.max.y + 1.0), pos2(lane_rect.max.x, ruler.max.y + 13.0));
    let bar = Rect::from_min_max(pos2(lane.x(it.start).max(bar_row.min.x), bar_row.min.y), pos2(lane.x(it.end()).min(bar_row.max.x), bar_row.max.y));
    ui.painter().rect_filled(bar, 2.0, Color32::from_rgb(58, 58, 70));
    ui.painter().with_clip_rect(bar).text(pos2(bar.min.x + 4.0, bar.center().y), Align2::LEFT_CENTER, &it.name, Tokens::ui(10.0), t.text);
    let scrub = Rect::from_min_max(lane_rect.min, pos2(lane_rect.max.x, bar_row.max.y));
    let sresp = ui.interact(scrub, egui::Id::new(("ec-scrub", clip.0)), Sense::click_and_drag());
    app.auto.add("effectControls.lane", lane_rect, "keyframe lane");
    if (sresp.dragged() || sresp.clicked())
        && let Some(pos) = sresp.interact_pointer_pos()
    {
        let tk = lane.tl_at(pos.x).clamp(it.start, it.end() - Tick(1));
        app.stop();
        app.session.set_playhead(tk);
    }
    let zbar = Rect::from_min_max(pos2(lane_rect.min.x + 6.0, rect.max.y - 22.0), pos2(lane_rect.max.x - 6.0, rect.max.y - 6.0));
    crate::panels::ec_keyframes::zoom_bar(app, ui, zbar, &lane, clip);
    let lane_r = lane_rect;
    let body = Rect::from_min_max(pos2(rect.min.x, head.max.y + 4.0), pos2(split, rect.max.y - 26.0));
    let mut actions: Vec<(String, Value)> = Vec::new();
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("ec-body"));
    bui.set_clip_rect(Rect::from_min_max(body.min, pos2(rect.max.x, body.max.y)));
    let mt_now = it.source_time_at(ph.clamp(it.start, it.end() - Tick(1)));
    let heading = if kind == filmcraft_project::TrackKind::Video { "Video" } else { "Audio" };
    bui.painter().text(pos2(body.min.x + 8.0, body.min.y + 8.0), Align2::LEFT_CENTER, heading, Tokens::semibold(11.5), t.text_dim);
    bui.add_space(18.0);
    // Premiere lists the fixed effects (Motion, Opacity, Time Remapping / Volume…) first.
    let mut order: Vec<usize> = (0..it.effects.len()).collect();
    order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
    // selected keyframes that are gone (undo, another clip) drop out
    app.ui.kf_selected.retain(|k| k.clip == clip.0 && kf_exists(&it, k));
    let pos_id = egui::Id::new(("ec-kf-pos", clip.0));
    bui.data_mut(|d| d.insert_temp::<Vec<(Pos2, KfSel)>>(pos_id, Vec::new()));
    let fx_drag_id = egui::Id::new(("ec-fx-drag", clip.0));
    let scroll_out = egui::ScrollArea::vertical().id_salt("ec-scroll").auto_shrink([false, false]).show(&mut bui, |bui| {
        // the lane behind the keyframes: a click clears the keyframe selection, a drag selects the
        // keyframes inside its box (Shift / Cmd adds them)
        let lane_body = Rect::from_min_max(pos2(lane_r.min.x, bar_row.max.y + 1.0), lane_r.max);
        let marquee = bui.interact(lane_body, egui::Id::new(("ec-marquee", clip.0)), Sense::click_and_drag());
        app.auto.add("effectControls.laneBody", lane_body, "keyframe lane");
        // effect headers (for dragging an effect to another place)
        let mut headers: Vec<(usize, Rect, bool)> = Vec::new();
        let mut fx_drop = false;
        for idx in order {
            let e = &it.effects[idx];
            let Some(def) = e.def() else { continue };
            let key = format!("{}:{}", clip.0, idx);
            let open = !app.ui.collapsed_fx.contains(&key);
            let movable = filmcraft_engine::effect_edit::movable(e);
            let (r, resp) = bui.allocate_exact_size(vec2(body.width(), ROW_H), if movable { Sense::click_and_drag() } else { Sense::click() });
            headers.push((idx, r, movable));
            if movable && resp.drag_started() {
                bui.data_mut(|d| d.insert_temp(fx_drag_id, idx));
            }
            if movable && resp.drag_stopped() {
                fx_drop = true;
            }
            let selected = app.ui.fx_selected.contains(&(clip.0, idx));
            if selected {
                bui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                bui.painter().rect_filled(r, 0.0, t.hover);
            }
            icons::paint(
                bui.painter(),
                Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(10.0, 10.0)),
                if open { Icon::ChevronDown } else { Icon::ChevronRight },
                t.text_dim,
            );
            // fx enable toggle
            let fxr = Rect::from_center_size(pos2(r.min.x + 28.0, r.center().y), vec2(18.0, 14.0));
            let fxresp = bui.interact(fxr, egui::Id::new(("fxen", clip.0, idx)), Sense::click());
            bui.painter().text(fxr.center(), Align2::CENTER_CENTER, "fx", Tokens::semibold(10.5), if e.enabled { t.text } else { t.text_faint });
            if !e.enabled {
                bui.painter().line_segment([fxr.left_bottom(), fxr.right_top()], Stroke::new(1.0, t.text_faint));
            }
            if fxresp.clicked() {
                actions.push(("effects.toggleEnabled".into(), json!({"clip": clip.0, "index": idx})));
            }
            bui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, def.name, Tokens::ui(12.0), t.text);
            // reset button
            let rr = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(16.0, 16.0));
            let rresp = bui.interact(rr, egui::Id::new(("fxreset", clip.0, idx)), Sense::click()).on_hover_text("Reset Effect");
            icons::paint(bui.painter(), rr.shrink(2.0), Icon::Reset, if rresp.hovered() { t.text } else { t.text_dim });
            if rresp.clicked() {
                actions.push(("effects.reset".into(), json!({"clip": clip.0, "index": idx})));
            }
            app.auto.add(&format!("effectControls.effect.{}", e.effect), r, def.name);
            app.auto.add(
                &format!("effectControls.effect.{}.fold", e.effect),
                Rect::from_min_size(r.min, vec2(20.0, r.height())),
                if open { "fold" } else { "unfold" },
            );
            // summary keyframes: every time any parameter of the effect has one
            let mut times: Vec<Tick> = e.params.values().flat_map(|p| p.keyframes.iter().map(|k| k.time)).collect();
            times.sort_unstable();
            times.dedup();
            for mt in times {
                let x = lane.x_of_media(mt);
                if !lane.shows(x) || x < lane_r.min.x || x > lane_r.max.x {
                    continue;
                }
                let c = pos2(x, r.center().y);
                bui.painter().add(egui::Shape::convex_polygon(
                    vec![c + vec2(0.0, -3.5), c + vec2(3.5, 0.0), c + vec2(0.0, 3.5), c + vec2(-3.5, 0.0)],
                    t.text_faint,
                    Stroke::NONE,
                ));
            }
            if resp.clicked() && !fxresp.clicked() && !rresp.clicked() {
                let mods = bui.input(|i| i.modifiers);
                let on_chevron = resp.interact_pointer_pos().is_some_and(|p| p.x < r.min.x + 20.0);
                if on_chevron {
                    if open {
                        app.ui.collapsed_fx.push(key.clone());
                    } else {
                        app.ui.collapsed_fx.retain(|k| *k != key);
                    }
                } else {
                    // the name selects the effect (Cmd/Ctrl adds or removes, Shift extends from
                    // the last one), as in Premiere; a preset can be saved from several at once
                    select_effect(app, clip.0, idx, mods, &it);
                }
            }
            // a right-click on an unselected effect selects it alone first
            if resp.secondary_clicked() && !selected {
                app.ui.fx_selected = vec![(clip.0, idx)];
            }
            let mut save_preset = false;
            let fx_id = e.effect.clone();
            let picked: Vec<usize> = {
                let mut v: Vec<usize> = app.ui.fx_selected.iter().filter(|(c, _)| *c == clip.0).map(|(_, i)| *i).collect();
                v.sort_unstable();
                if v.contains(&idx) { v } else { vec![idx] }
            };
            resp.context_menu(|ui| {
                let label = if picked.len() > 1 { format!("Save Preset… ({} effects)", picked.len()) } else { "Save Preset…".to_string() };
                let sp = ui.button(&label);
                app.auto.add(&format!("effectControls.effect.{fx_id}.savePreset"), sp.rect, &label);
                if sp.clicked() {
                    save_preset = true;
                    ui.close();
                }
                if !def.intrinsic && ui.button("Clear").clicked() {
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
                custom_setup_row(app, bui, body, clip, idx, &e.effect);
            }
            for pd in &def.params {
                param_row(app, bui, body, clip, idx, e, None, pd, mt_now, &mut actions, &lane, &it);
                if app.ui.expanded_fx.contains(&graph_key(clip, idx, pd.id))
                    && let Some(param) = e.params.get(pd.id)
                    && crate::panels::ec_keyframes::graphable(param)
                {
                    crate::panels::ec_keyframes::graph_rows(app, bui, body, clip, idx, None, pd, param, &lane, &it, &mut actions);
                }
            }
            if crate::panels::masks::maskable(e) {
                crate::panels::masks::effect_rows(app, bui, body, clip, idx, e, mt_now, &mut actions, &lane, &it);
            }
        }
        // an effect dragged by its name goes between the standard effects it is dropped between
        if let Some(from) = bui.data(|d| d.get_temp::<usize>(fx_drag_id)) {
            let movable: Vec<(usize, Rect)> = headers.iter().filter(|h| h.2).map(|h| (h.0, h.1)).collect();
            if let (Some(p), Some(last)) = (bui.ctx().pointer_latest_pos(), movable.last()) {
                // the first standard effect whose middle is below the pointer, else after the last
                let before = movable.iter().find(|(_, r)| p.y < r.center().y);
                let (line_y, to) = match before {
                    Some((i, r)) => (r.min.y, if *i > from { i - 1 } else { *i }),
                    None => (last.1.max.y, last.0),
                };
                bui.painter().line_segment([pos2(body.min.x + 4.0, line_y), pos2(body.max.x - 4.0, line_y)], Stroke::new(2.0, t.accent));
                if fx_drop {
                    bui.data_mut(|d| d.remove::<usize>(fx_drag_id));
                    if to != from {
                        actions.push(("effects.move".into(), json!({"clip": clip.0, "index": from, "to": to})));
                        app.ui.fx_selected = vec![(clip.0, to)];
                    }
                }
            } else if fx_drop {
                bui.data_mut(|d| d.remove::<usize>(fx_drag_id));
            }
        }
        // the keyframe box
        let start_id = egui::Id::new(("ec-marquee-start", clip.0));
        if marquee.drag_started()
            && let Some(p) = bui.input(|i| i.pointer.press_origin())
        {
            bui.data_mut(|d| d.insert_temp(start_id, p));
        }
        let start: Option<Pos2> = bui.data(|d| d.get_temp(start_id));
        if let (Some(a), Some(b)) = (start, marquee.interact_pointer_pos().or(bui.ctx().pointer_latest_pos())) {
            let boxr = Rect::from_two_pos(a, b);
            if marquee.dragged() {
                bui.painter().rect(boxr, 0.0, t.accent.gamma_multiply(0.12), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
            if marquee.drag_stopped() {
                bui.data_mut(|d| d.remove::<Pos2>(start_id));
                let inside: Vec<KfSel> = bui
                    .data(|d| d.get_temp::<Vec<(Pos2, KfSel)>>(pos_id))
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
                app.ui.fx_selected.retain(|(c, _)| *c != clip.0);
            }
        }
        if marquee.clicked() {
            app.ui.kf_selected.clear();
        }
    });
    let _ = scroll_out;
    crate::panels::ec_keyframes::run_lane_drag(app, ui, &it, &lane, &mut actions);
    // playhead in lane, with its head on the ruler (only while it is over the shown part of the clip)
    if ph >= it.start && ph < it.end() && ph >= lane.t0 && ph <= lane.t1 {
        let px = lane.x(ph);
        ui.painter().line_segment([pos2(px, lane_r.min.y + 6.0), pos2(px, lane_r.max.y)], Stroke::new(1.0, t.playhead));
        let y0 = lane_r.min.y + 1.0;
        let head = vec![pos2(px - 5.0, y0), pos2(px + 5.0, y0), pos2(px + 5.0, y0 + 7.0), pos2(px, y0 + 12.0), pos2(px - 5.0, y0 + 7.0)];
        ui.painter().add(egui::Shape::convex_polygon(head, t.playhead, Stroke::NONE));
    }
    // footer timecode
    let tc = filmcraft_time::format_time(ph, seq.settings.frame_rate, seq.settings.drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
    ui.painter().text(pos2(rect.min.x + 10.0, rect.max.y - 13.0), Align2::LEFT_CENTER, tc, Tokens::mono(13.0), t.timecode);
    for (cmd, p) in actions {
        if let Err(e) = app.session.execute(&cmd, p) {
            app.ui.status = e.to_string();
        }
    }
}

/// Effect Controls selection: a click selects the effect, Cmd/Ctrl-click toggles it, Shift-click
/// selects the range from the last picked effect (in the panel's order: fixed effects first).
fn select_effect(app: &mut FilmcraftApp, clip: u64, idx: usize, mods: egui::Modifiers, it: &TrackItem) {
    // effects and keyframes are not selected together (Delete acts on one or the other)
    app.ui.kf_selected.clear();
    let sel = &mut app.ui.fx_selected;
    sel.retain(|(c, _)| *c == clip);
    if mods.command || mods.ctrl {
        if let Some(i) = sel.iter().position(|x| *x == (clip, idx)) {
            sel.remove(i);
        } else {
            sel.push((clip, idx));
        }
    } else if mods.shift && !sel.is_empty() {
        let mut order: Vec<usize> = (0..it.effects.len()).collect();
        order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
        let at = |i: usize| order.iter().position(|o| *o == i).unwrap_or(0);
        let last = sel.last().map_or(idx, |(_, i)| *i);
        let (a, b) = (at(last).min(at(idx)), at(last).max(at(idx)));
        for o in &order[a..=b] {
            if !sel.contains(&(clip, *o)) {
                sel.push((clip, *o));
            }
        }
    } else {
        *sel = vec![(clip, idx)];
    }
}

/// Premiere's "Custom Setup ▸ Edit…" row: opens the effect's Clip Fx Editor window.
fn custom_setup_row(app: &mut FilmcraftApp, ui: &mut egui::Ui, body: Rect, clip: ClipId, idx: usize, effect: &str) {
    let t = app.tokens;
    let (r, _) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::hover());
    ui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, "Custom Setup", Tokens::ui(12.0), t.text_dim);
    let br = Rect::from_min_size(pos2(r.min.x + 160.0, r.min.y + 2.0), vec2(60.0, ROW_H - 4.0));
    let resp = ui.interact(br, egui::Id::new(("fx-custom-setup", clip.0, idx)), Sense::click());
    ui.painter().rect_filled(br, 3.0, if resp.hovered() { t.hover } else { t.field_bg });
    ui.painter().rect_stroke(br, 3.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(br.center(), Align2::CENTER_CENTER, "Edit…", Tokens::ui(11.5), t.text);
    app.auto.add(&format!("effectControls.effect.{effect}.edit"), br, "Edit…");
    if resp.clicked() {
        crate::panels::audio_fx_editor::open(app, crate::panels::audio_fx_editor::FxTarget::Clip { clip: clip.0, index: idx });
    }
}

fn seq_name(app: &FilmcraftApp) -> String {
    app.session.state.active_sequence.and_then(|s| app.session.project.item(s)).map(|i| i.name.clone()).unwrap_or_default()
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
    lane: &crate::panels::ec_keyframes::Lane,
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
    let (r, _) = ui.allocate_exact_size(vec2(body.width(), row_h), Sense::hover());
    // the label, stopwatch and keyframes sit on the first line of a taller row
    let cy = r.min.y + ROW_H / 2.0;
    let mut x = r.min.x + 26.0;
    // twirl-down for the value/velocity graphs (animated numbers and points)
    if crate::panels::ec_keyframes::graphable(param) {
        let key = graph_key(clip, idx, pkey);
        let open = app.ui.expanded_fx.contains(&key);
        let tw = Rect::from_center_size(pos2(r.min.x + 12.0, cy), vec2(12.0, 12.0));
        let tresp = ui.interact(tw.expand(2.0), egui::Id::new(("twirl", clip.0, idx, pkey)), Sense::click()).on_hover_text("Show graphs");
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
        let resp = ui.interact(sw, egui::Id::new(("sw", clip.0, idx, pkey)), Sense::click()).on_hover_text("Toggle animation");
        icons::paint(ui.painter(), sw, Icon::Stopwatch, if param.is_animated() { t.accent } else { t.text_dim });
        if resp.clicked() {
            actions.push(("effects.toggleAnimation".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id}))));
        }
        app.auto.add(&format!("effectControls.{}.{}.stopwatch", e.effect, pkey), sw, "Toggle animation");
    }
    x += 14.0;
    ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, pd.label, Tokens::ui(12.0), t.text);
    // the value: left of the keyframe navigator when there is one, starting early enough for a
    // point's two numbers to fit
    let value_end = if pd.animatable && param.is_animated() { r.max.x - 68.0 } else { r.max.x - 26.0 };
    let need = if matches!(pd.kind, ParamKind::Point) { 112.0 } else { 56.0 };
    let vx = (r.min.x + (r.width() * 0.42).max(130.0)).min(value_end - need).max(r.min.x + 96.0);
    let value = param.value_at(mt);
    let mut vui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(vx, r.min.y + 1.0), pos2(value_end.max(vx + 40.0), r.max.y - 1.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let id = egui::Id::new(("pv", clip.0, idx, pkey));
    let mut set: Option<Value> = None;
    match (&pd.kind, &value) {
        (ParamKind::Float { min, max, soft_min, soft_max, unit, decimals }, ParamValue::Float(v)) => {
            let speed = ((soft_max - soft_min) / 400.0).max(0.01);
            let (vr, nv) = crate::widgets::hot_number(&mut vui, id, *v, speed, (*min, *max), *decimals as usize, unit, &t);
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), vr.rect, pd.label);
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Angle, ParamValue::Float(v)) => {
            let (vr, nv) = crate::widgets::hot_number(&mut vui, id, *v, 0.5, (-36000.0, 36000.0), 1, "°", &t);
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), vr.rect, pd.label);
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Point, ParamValue::Vec2(p)) => {
            let (xr, nx) = crate::widgets::hot_number(&mut vui, id.with("x"), p.x, 1.0, (-100_000.0, 100_000.0), 1, "", &t);
            let (yr, ny) = crate::widgets::hot_number(&mut vui, id.with("y"), p.y, 1.0, (-100_000.0, 100_000.0), 1, "", &t);
            app.auto.add(&format!("effectControls.{}.{}.x", e.effect, pkey), xr.rect, pd.label);
            app.auto.add(&format!("effectControls.{}.{}.y", e.effect, pkey), yr.rect, pd.label);
            if nx.is_some() || ny.is_some() {
                set = Some(json!([nx.unwrap_or(p.x), ny.unwrap_or(p.y)]));
            }
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut v = *b;
            let r = vui.checkbox(&mut v, "");
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), r.rect, pd.label);
            if r.changed() {
                set = Some(json!(v));
            }
        }
        (ParamKind::Choice(opts), ParamValue::Choice(c)) => {
            let mut sel = *c as usize;
            let r = egui::ComboBox::from_id_salt(id).selected_text(opts.get(sel).copied().unwrap_or("")).width(130.0).height(360.0).show_ui(&mut vui, |ui| {
                for (i, o) in opts.iter().enumerate() {
                    let o_r = ui.selectable_value(&mut sel, i, *o);
                    app.auto.add(&format!("effectControls.{}.{}.option.{i}", e.effect, pkey), o_r.rect, o);
                    if o_r.changed() {
                        set = Some(json!(i));
                    }
                }
            });
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), r.response.rect, pd.label);
        }
        (ParamKind::Color, ParamValue::Color(c)) => {
            let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let r = egui::color_picker::color_edit_button_rgba(&mut vui, &mut rgba, egui::color_picker::Alpha::Opaque);
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), r.rect, pd.label);
            if r.changed() {
                set = Some(json!([rgba.r(), rgba.g(), rgba.b(), rgba.a()]));
            }
        }
        (ParamKind::Path, _) => {
            crate::panels::masks::path_value(app, &mut vui, clip, idx, mask, actions);
        }
        // a text layer's font and style: pickers of the system's fonts; its text: committed as it
        // is typed (so the field's own undo / redo work and the picture follows)
        (ParamKind::Text, ParamValue::Text(f)) if e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "font" => {
            let w = (vui.available_width() - 4.0).clamp(120.0, 220.0);
            if let Some(nf) = crate::panels::font_picker::font_picker(app, &mut vui, ("ec", clip.0, idx), f, w, &format!("effectControls.{}.font", e.effect)) {
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
                crate::panels::font_picker::style_picker(app, &mut vui, ("ec", clip.0, idx), &family, st, w, &format!("effectControls.{}.font_style", e.effect))
            {
                set = Some(json!(ns));
            }
        }
        (ParamKind::Text, ParamValue::Text(s)) if e.effect == filmcraft_project::graphic::TEXT_LAYER && pd.id == "text" => {
            // a box of the text's lines (Return starts a new line), committed as typed: one undo
            // step per visit to the box
            let mut buf = s.clone();
            let rows = s.lines().count().clamp(1, TEXT_ROWS);
            let r = vui.add(egui::TextEdit::multiline(&mut buf).desired_rows(rows).desired_width((vui.available_width() - 4.0).max(120.0)));
            app.auto.add(&format!("effectControls.text.{}.{}", idx, pd.id), r.rect, pd.label);
            let fresh_id = id.with("fresh");
            if r.gained_focus() {
                vui.data_mut(|d| d.insert_temp(fresh_id, true));
            }
            if buf != *s {
                let fresh = vui.data_mut(|d| d.remove_temp::<bool>(fresh_id)).unwrap_or(false);
                actions.push((
                    "graphics.setText".into(),
                    json!({"clip": clip.0, "layer": filmcraft_project::graphic::layer_indices(&it.effects).iter().position(|i| *i == idx).unwrap_or(0), "text": buf, "merge": !fresh}),
                ));
            }
        }
        (ParamKind::Text, ParamValue::Text(s)) if !pd.id.ends_with("_lut") => {
            // edited in a buffer; committed (one undo step) when the field loses focus
            let mut buf = vui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| s.clone());
            let r = vui.add(egui::TextEdit::singleline(&mut buf).desired_width(160.0));
            app.auto.add(&format!("effectControls.text.{}.{}", idx, pd.id), r.rect, pd.label);
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
        _ => {
            vui.label(param_text(&app.session.project, pd.id, &value));
        }
    }
    if let Some(v) = set {
        actions.push(("effects.setParam".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": v}))));
    }
    // keyframe navigator ◀ ◆ ▶ (when animated)
    let eff_json = json!(idx);
    if pd.animatable && param.is_animated() {
        let nx = r.max.x - 58.0;
        let at_key = param.keyframes.iter().any(|k| k.time == mt);
        let prev = param.prev_keyframe(mt);
        let next = param.next_keyframe(mt);
        let to_tl = |k: Tick| it.start + Tick(((k - it.source_in).0 as f64 / it.speed.abs().max(1e-6)) as i64);
        let pr = Rect::from_center_size(pos2(nx, cy), vec2(12.0, 14.0));
        let kr = Rect::from_center_size(pos2(nx + 16.0, cy), vec2(12.0, 12.0));
        let nr = Rect::from_center_size(pos2(nx + 32.0, cy), vec2(12.0, 14.0));
        let arrow = |p: &egui::Painter, r: Rect, left: bool, on: bool| {
            let c = r.center();
            let pts = if left {
                vec![c + vec2(3.0, -4.0), c + vec2(3.0, 4.0), c + vec2(-3.0, 0.0)]
            } else {
                vec![c + vec2(-3.0, -4.0), c + vec2(-3.0, 4.0), c + vec2(3.0, 0.0)]
            };
            p.add(egui::Shape::convex_polygon(pts, if on { t.text } else { t.text_faint }, Stroke::NONE));
        };
        arrow(ui.painter(), pr, true, prev.is_some());
        arrow(ui.painter(), nr, false, next.is_some());
        icons::paint(ui.painter(), kr, Icon::Keyframe, if at_key { t.hot_text } else { t.text_dim });
        if ui.interact(pr, egui::Id::new(("kprev", clip.0, idx, pkey)), Sense::click()).clicked()
            && let Some(k) = prev
        {
            actions.push(("playhead.set".into(), json!({"time": to_tl(k).0})));
        }
        if ui.interact(nr, egui::Id::new(("knext", clip.0, idx, pkey)), Sense::click()).clicked()
            && let Some(k) = next
        {
            actions.push(("playhead.set".into(), json!({"time": to_tl(k).0})));
        }
        if ui.interact(kr, egui::Id::new(("kadd", clip.0, idx, pkey)), Sense::click()).on_hover_text("Add/Remove Keyframe").clicked() {
            actions.push(("effects.addKeyframe".into(), with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id}))));
        }
        app.auto.add(&format!("effectControls.{}.{}.addKeyframe", e.effect, pkey), kr, "Add/Remove Keyframe");
    }
    // keyframes in the lane: diamonds that drag (the selected ones together, live), right-click
    // for interpolation
    if param.is_animated() {
        let y = cy;
        let rate = app.session.sequence_rate();
        let lr = lane.rect;
        // the animated spans: a line from each keyframe to the next (dashed after a Hold key,
        // whose value does not change until the next one)
        let span = ui.painter().with_clip_rect(lr.intersect(ui.clip_rect()));
        for w in param.keyframes.windows(2) {
            let (a, b) = (lane.x_of_media(w[0].time), lane.x_of_media(w[1].time));
            let (a, b) = (a.max(lr.min.x - 6.0), b.min(lr.max.x + 6.0));
            if b - a < 12.0 {
                continue;
            }
            let col = Color32::from_rgb(0x6a, 0x6a, 0x72);
            if w[0].interp == filmcraft_project::Interpolation::Hold {
                span.add(egui::Shape::dashed_line(&[pos2(a + 6.0, y), pos2(b - 6.0, y)], Stroke::new(1.0, col), 4.0, 3.0));
            } else {
                span.line_segment([pos2(a + 6.0, y), pos2(b - 6.0, y)], Stroke::new(1.0, col));
            }
        }
        let pos_id = egui::Id::new(("ec-kf-pos", clip.0));
        let kp = ui.painter().with_clip_rect(lr.expand2(vec2(6.0, 0.0)).intersect(ui.clip_rect()));
        for k in &param.keyframes {
            let tl = lane.tl_of(k.time);
            let kx = lane.x_of_media(k.time);
            if !lane.shows(kx) {
                continue;
            }
            let me = KfSel { clip: clip.0, effect: idx, param: pd.id.to_string(), mask, time: k.time.0 };
            let selected = app.ui.kf_selected.contains(&me);
            let id = egui::Id::new(("kf", clip.0, idx, pkey, k.time.0));
            let kr = Rect::from_center_size(pos2(kx, y), vec2(11.0, 11.0));
            let resp = ui.interact(kr.expand(2.0), id, Sense::click_and_drag());
            let resp = resp.on_hover_text(format!(
                "{}  ·  {}  ·  {}",
                filmcraft_time::format_time(tl, rate, false, filmcraft_time::TimeDisplay::Timecode, 48000),
                value_text(pd, &k.value),
                k.interp.label()
            ));
            app.auto.add(&format!("effectControls.{}.{}.keyframe.{}", e.effect, pkey, k.time.0), kr, if selected { "keyframe (selected)" } else { "keyframe" });
            ui.data_mut(|d| d.get_temp_mut_or_default::<Vec<(Pos2, KfSel)>>(pos_id).push((kr.center(), me.clone())));
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
            if resp.drag_started() {
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
            let n = menu_keys.len();
            resp.context_menu(|ui| {
                let head = if n > 1 { format!("Temporal Interpolation ({n} keyframes)") } else { "Temporal Interpolation".to_string() };
                ui.label(egui::RichText::new(head).color(t.text_dim));
                for (label, key) in [
                    ("Linear", "linear"),
                    ("Bezier", "bezier"),
                    ("Auto Bezier", "autoBezier"),
                    ("Continuous Bezier", "continuousBezier"),
                    ("Hold", "hold"),
                    ("Ease In", "easeIn"),
                    ("Ease Out", "easeOut"),
                ] {
                    if ui.button(label).clicked() {
                        actions.push(("effects.setKeyframesInterpolation".into(), json!({"clip": clip.0, "keys": menu_keys, "interpolation": key})));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Copy").clicked() {
                    actions.push(("effects.copyKeyframes".into(), json!({"clip": clip.0, "keys": menu_keys})));
                    ui.close();
                }
                if ui.button("Clear").clicked() {
                    actions.push(("effects.deleteKeyframes".into(), json!({"clip": clip.0, "keys": menu_keys})));
                    ui.close();
                }
            });
        }
    }
}

/// Lumetri Color panel: edits (or adds) the Lumetri effect on the selected clip, grouped by section.
pub fn lumetri_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, _)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "Select a clip to grade");
        return;
    };
    let idx = it.effects.iter().position(|e| e.effect == "lumetri");
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)).id_salt("lumetri"));
    bui.label(egui::RichText::new(format!("Master · {}", it.name)).color(t.text_dim));
    let Some(idx) = idx else {
        if bui.button("Add Lumetri Color to clip").clicked() {
            let _ = app.session.execute("effects.apply", json!({"clips": [clip.0], "effect": "lumetri"}));
        }
        return;
    };
    let e = it.effects[idx].clone();
    let Some(def) = e.def() else { return };
    let ph = app.session.playhead();
    let mt = it.source_time_at(ph.clamp(it.start, it.end() - Tick(1)));
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
            let (resp, now_open) = crate::widgets::section_header(ui, egui::Id::new(&key), g, open, &t, true);
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
                    ui.add_sized(vec2(110.0, 18.0), egui::Label::new(egui::RichText::new(pd.label).size(12.0)));
                    if let (ParamKind::Float { min, max, soft_min, soft_max, .. }, ParamValue::Float(x)) = (&pd.kind, &v) {
                        let mut val = *x;
                        let s = ui.add(egui::Slider::new(&mut val, *soft_min..=*soft_max).show_value(false));
                        let (_, nv) =
                            crate::widgets::hot_number(ui, egui::Id::new(("lum", pd.id)), val, (soft_max - soft_min) / 300.0, (*min, *max), 1, "", &t);
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
    for a in actions {
        if let Err(e) = app.session.execute("effects.setParam", a) {
            app.ui.status = e.to_string();
        }
    }
}

/// Properties panel (Premiere 26): a compact inspector for the selected clip.
pub fn properties_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, kind)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "Select a clip to see its properties");
        return;
    };
    let ph = app.session.playhead();
    let mt = it.source_time_at(ph.clamp(it.start, it.end() - Tick(1)));
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
        ui.painter().text(pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, name, Tokens::semibold(13.0), t.text);
        if let Some((idx, c)) = reset {
            let rr = Rect::from_center_size(pos2(r.max.x - 10.0, r.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), rr, Icon::Reset, t.text_dim);
            if ui.interact(rr, egui::Id::new(("props-reset", name, c)), Sense::click()).clicked() {
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
    let diamond = |ui: &mut egui::Ui, r: Rect, animated: bool, id: egui::Id| -> bool {
        let dr = Rect::from_center_size(pos2(r.max.x - 10.0, r.center().y), vec2(10.0, 10.0));
        let resp = ui.interact(dr.expand(3.0), id, Sense::click());
        if animated {
            icons::paint(ui.painter(), dr, Icon::Keyframe, t.hot_text);
        } else {
            let c = dr.center();
            let s = 4.5;
            ui.painter()
                .add(egui::Shape::closed_line(vec![c + vec2(0.0, -s), c + vec2(s, 0.0), c + vec2(0.0, s), c + vec2(-s, 0.0)], Stroke::new(1.2, t.text_dim)));
        }
        resp.clicked()
    };
    let eff_idx = |id: &str| it.effects.iter().position(|e| e.effect == id);
    let val = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).map(|p| p.value_at(mt));
    let anim = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).is_some_and(|p| p.is_animated());
    if kind == filmcraft_project::TrackKind::Video {
        if section(&mut bui, app, "Transform", eff_idx("motion").map(|i| (i, clip.0)), &mut actions) {
            for (label, p, unit, speed, range) in [
                ("Position", "position", "", 1.0, (-100_000.0, 100_000.0)),
                ("Anchor point", "anchor", "", 1.0, (-100_000.0, 100_000.0)),
                ("Scale", "scale", " %", 0.5, (0.0, 10000.0)),
                ("Rotation", "rotation", " °", 0.5, (-36000.0, 36000.0)),
            ] {
                let r = row(&mut bui, label);
                let mut vui = bui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                match val("motion", p) {
                    Some(ParamValue::Vec2(v)) => {
                        let (xr, nx) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, "x", clip.0)), v.x, speed, range, 0, " X", &t);
                        vui.add_space(10.0);
                        let (yr, ny) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, "y", clip.0)), v.y, speed, range, 0, " Y", &t);
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
                        let (vr, nv) =
                            crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, clip.0)), v, speed, range, if p == "scale" { 0 } else { 1 }, unit, &t);
                        app.auto.add(&format!("properties.motion.{p}.value"), vr.rect, label);
                        if let Some(nv) = nv {
                            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "motion", "param": p, "value": nv})));
                        }
                    }
                    _ => {}
                }
                if diamond(&mut bui, r, anim("motion", p), egui::Id::new(("pd", p, clip.0))) {
                    actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "motion", "param": p})));
                }
            }
            let r = row(&mut bui, "Opacity");
            let mut vui = bui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            if let Some(ParamValue::Float(v)) = val("opacity", "opacity") {
                let (vr, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-op", clip.0)), v, 0.5, (0.0, 100.0), 0, " %", &t);
                app.auto.add("properties.opacity.opacity.value", vr.rect, "Opacity");
                if let Some(nv) = nv {
                    actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "opacity", "param": "opacity", "value": nv})));
                }
            }
            if diamond(&mut bui, r, anim("opacity", "opacity"), egui::Id::new(("pd-op", clip.0))) {
                actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "opacity", "param": "opacity"})));
            }
        }
        let crop = eff_idx("crop");
        if section(&mut bui, app, "Crop", crop.map(|i| (i, clip.0)), &mut actions) {
            for (label, p) in [("Left", "left"), ("Top", "top"), ("Right", "right"), ("Bottom", "bottom")] {
                let r = row(&mut bui, label);
                let mut vui = bui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let v = val("crop", p).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-crop", p, clip.0)), v, 0.2, (0.0, 100.0), 1, " %", &t);
                if let Some(nv) = nv {
                    if crop.is_none() {
                        actions.push(("effects.apply".into(), json!({"clips": [clip.0], "effect": "crop"})));
                    }
                    actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "crop", "param": p, "value": nv})));
                }
                diamond(&mut bui, r, anim("crop", p), egui::Id::new(("pd-crop", p, clip.0)));
            }
        }
    } else if section(&mut bui, app, "Audio", eff_idx("volume").map(|i| (i, clip.0)), &mut actions) {
        let r = row(&mut bui, "Level");
        let mut vui = bui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let v = val("volume", "level").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-vol", clip.0)), v, 0.1, (-96.0, 15.0), 1, " dB", &t);
        if let Some(nv) = nv {
            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "volume", "param": "level", "value": nv})));
        }
        if diamond(&mut bui, r, anim("volume", "level"), egui::Id::new(("pd-vol", clip.0))) {
            actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "volume", "param": "level"})));
        }
        let r = row(&mut bui, "Pan");
        let mut vui = bui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let v = val("panner", "balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-pan", clip.0)), v, 0.5, (-100.0, 100.0), 0, "", &t);
        if let Some(nv) = nv {
            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "panner", "param": "balance", "value": nv})));
        }
    }
    // speed footer
    bui.add_space(12.0);
    let (br, bresp) = bui.allocate_exact_size(vec2(118.0, 26.0), Sense::click());
    bui.painter().rect_filled(br, 4.0, if bresp.hovered() { t.hover } else { t.panel_bg });
    bui.painter().rect_stroke(br, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    bui.painter().text(br.center(), Align2::CENTER_CENTER, format!("Speed {:.0}%", it.speed * 100.0), Tokens::ui(12.0), t.text);
    for (cmd, p) in actions {
        if let Err(e) = app.session.execute(&cmd, p) {
            app.ui.status = e.to_string();
        }
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
/// selected keyframes, else the selected effects; in the Program monitor the selected graphic
/// layers (Clear). None: the command goes on as usual (the timeline selection).
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
    let (clip, it, _) = selected_clip(app)?;
    app.ui.kf_selected.retain(|k| k.clip == clip.0 && kf_exists(&it, k));
    let keys: Vec<Value> = app.ui.kf_selected.iter().map(KfSel::key_json).collect();
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
        ParamValue::Text(s) if s.is_empty() => "None".into(),
        ParamValue::Text(s) => s.clone(),
        ParamValue::Curve(pts) if pts.is_empty() || *pts == [[0.0, 0.0], [1.0, 1.0]] => "Default".into(),
        ParamValue::Curve(pts) => format!("Custom ({} points)", pts.len()),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod param_text_tests {
    use super::*;

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
