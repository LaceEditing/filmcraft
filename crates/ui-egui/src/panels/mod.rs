//! Panel bodies. `show` dispatches on [`PanelKind`]; drag-and-drop between panels (project items,
//! effects) is carried in egui temp data so the timeline/monitors can accept drops.

pub mod audio_fx_editor;
pub mod clip_dialogs;
pub mod color_dialogs;
pub mod dialogs;
pub mod ec_keyframes;
pub mod effect_controls;
pub mod effects;
pub mod essential_sound;
pub mod events;
pub mod export_mode;
pub mod file_dialogs;
pub mod font_picker;
pub mod frame_export;
pub mod graphics;
pub mod graphics_templates;
pub mod import_mode;
pub mod interchange_export;
pub mod keyboard;
pub mod lumetri;
pub mod masks;
pub mod media_browser;
pub mod media_dialogs;
pub mod menu_dialogs;
pub mod metadata;
pub mod meters;
pub mod misc;
pub mod mixer;
pub mod monitor;
pub mod monitor_view;
pub mod motion_overlay;
pub mod multicam;
pub mod notes;
pub mod panel_state;
pub mod presets;
pub mod project;
pub mod project_dialogs;
pub mod project_views;
pub mod reference;
pub mod remix;
pub mod scopes;
pub mod sequence_settings;
pub mod settings;
pub mod shortcuts_dialog;
pub mod text;
pub mod timecode;
pub mod timeline;
pub mod timeline_automation;
pub mod timeline_band;
pub mod timeline_captions;
pub mod timeline_hit;
pub mod timeline_volume;
pub mod tools;
pub mod transform_handles;
pub mod transition_controls;
pub mod trim_monitor;
pub mod tts;
pub mod voiceover;
pub mod workspaces;

use egui::{Align2, Color32, Rect};
use filmcraft_project::ItemId;

use crate::FilmcraftApp;
use crate::dock::PanelKind;
use crate::theme::Tokens;

/// ` · 2:05 left`: the time a job has left, as it follows a percentage.
pub fn left_text(d: std::time::Duration) -> String {
    tlf!(" · {time} left", time = filmcraft_engine::export::format_eta(d))
}

/// [`left_text`] for a job or queue item whose JSON has `etaSeconds`, nothing while it is null.
pub fn eta_suffix(job: &serde_json::Value) -> String {
    job["etaSeconds"].as_f64().and_then(|s| std::time::Duration::try_from_secs_f64(s).ok()).map(left_text).unwrap_or_default()
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, p: PanelKind, rect: Rect) {
    match p {
        PanelKind::Program if !app.session.state.edit_points.is_empty() => trim_monitor::show(app, ui, rect),
        PanelKind::Program => monitor::show(app, ui, rect, monitor::Which::Program),
        PanelKind::Source => monitor::show(app, ui, rect, monitor::Which::Source),
        PanelKind::Timeline => timeline::show(app, ui, rect),
        PanelKind::Project => project::show(app, ui, rect),
        PanelKind::Tools => tools::show(app, ui, rect),
        PanelKind::Effects => effects::show(app, ui, rect),
        PanelKind::EffectControls => effect_controls::show(app, ui, rect),
        PanelKind::AudioMeters => meters::show(app, ui, rect),
        PanelKind::LumetriColor => lumetri::show(app, ui, rect),
        PanelKind::Properties if graphics::graphic_selected(app) => graphics::properties(app, ui, rect),
        PanelKind::Properties if text::caption_selected(app) => text::caption_properties(app, ui, rect),
        PanelKind::Properties => effect_controls::properties_panel(app, ui, rect),
        PanelKind::EssentialGraphics => graphics_templates::essential_graphics(app, ui, rect),
        PanelKind::History => misc::history(app, ui, rect),
        PanelKind::Markers => misc::markers(app, ui, rect),
        PanelKind::Info => misc::info(app, ui, rect),
        PanelKind::MediaBrowser => media_browser::show(app, ui, rect),
        PanelKind::AudioTrackMixer => mixer::track_mixer(app, ui, rect),
        PanelKind::AudioClipMixer => mixer::clip_mixer(app, ui, rect),
        PanelKind::LumetriScopes => scopes::show(app, ui, rect),
        PanelKind::Metadata => metadata::show(app, ui, rect),
        PanelKind::Timecode => timecode::show(app, ui, rect),
        PanelKind::Events => events::events(app, ui, rect),
        PanelKind::Progress => events::progress(app, ui, rect),
        PanelKind::ReferenceMonitor => reference::show(app, ui, rect),
        PanelKind::Text => text::show(app, ui, rect),
        PanelKind::EssentialSound => essential_sound::show(app, ui, rect),
        PanelKind::TextToSpeech => tts::show(app, ui, rect),
        PanelKind::ProjectNotes => notes::show(app, ui, rect),
        other => crate::dock::placeholder(ui, rect, &app.tokens, &tlf!("{panel} — coming in a later milestone", panel = crate::i18n::t(other.title()))),
    }
}

pub mod source_drag;
pub mod source_range;

#[derive(Clone, Debug)]
enum DragPayload {
    Item(ItemId),
    /// A bin (by id) dragged in the Project panel.
    Bin(u64),
    Source(source_drag::SourceDrag),
    Effect(String),
    /// A graphics template (id, name) from Essential Graphics ▸ Browse.
    Template(String, String),
}

fn payload_id() -> egui::Id {
    egui::Id::new("filmcraft-drag-payload")
}

pub fn start_drag_source(ui: &egui::Ui, source: source_drag::SourceDrag) {
    ui.ctx().data_mut(|d| d.insert_temp(payload_id(), Some(DragPayloadBox(DragPayload::Source(source)))));
}
pub fn dragged_source(ui: &egui::Ui) -> Option<source_drag::SourceDrag> {
    match payload(ui) {
        Some(DragPayload::Source(s)) => Some(s),
        _ => None,
    }
}
pub fn start_drag_item(ui: &egui::Ui, item: ItemId) {
    ui.ctx().data_mut(|d| d.insert_temp(payload_id(), Some(DragPayloadBox(DragPayload::Item(item)))));
}
pub fn start_drag_bin(ui: &egui::Ui, bin: u64) {
    ui.ctx().data_mut(|d| d.insert_temp(payload_id(), Some(DragPayloadBox(DragPayload::Bin(bin)))));
}
pub fn dragged_bin(ui: &egui::Ui) -> Option<u64> {
    match payload(ui) {
        Some(DragPayload::Bin(b)) => Some(b),
        _ => None,
    }
}
pub fn start_drag_template(ui: &egui::Ui, id: &str, name: &str) {
    ui.ctx().data_mut(|d| d.insert_temp(payload_id(), Some(DragPayloadBox(DragPayload::Template(id.to_string(), name.to_string())))));
}
pub fn dragged_template(ui: &egui::Ui) -> Option<String> {
    match payload(ui) {
        Some(DragPayload::Template(id, _)) => Some(id),
        _ => None,
    }
}
pub fn start_drag_effect(ui: &egui::Ui, id: &str) {
    ui.ctx().data_mut(|d| d.insert_temp(payload_id(), Some(DragPayloadBox(DragPayload::Effect(id.to_string())))));
}
/// Prefix of a dragged effect payload that names a Lumetri preset rather than an effect id.
pub const LUMETRI_PRESET_PREFIX: &str = "lumetri:";
/// A Lumetri preset (Effects ▸ Lumetri Presets) dragged onto a clip: an effect payload
/// `lumetri:<name>` the Timeline applies with `lumetri.applyPreset`.
pub fn start_drag_lumetri_preset(ui: &egui::Ui, name: &str) {
    start_drag_effect(ui, &format!("{LUMETRI_PRESET_PREFIX}{name}"));
}
#[derive(Clone, Debug)]
struct DragPayloadBox(DragPayload);

fn payload(ui: &egui::Ui) -> Option<DragPayload> {
    ui.ctx().data(|d| d.get_temp::<Option<DragPayloadBox>>(payload_id())).flatten().map(|b| b.0)
}
pub fn dragged_project_item(ui: &egui::Ui) -> Option<ItemId> {
    match payload(ui) {
        Some(DragPayload::Item(i)) => Some(i),
        Some(DragPayload::Source(s)) => Some(s.item),
        _ => None,
    }
}
/// An item dragged from the Project panel (or the Media Browser), but not a Source Monitor drag,
/// whose In/Out range and video/audio choice only a timeline drop keeps.
pub fn dragged_panel_item(ui: &egui::Ui) -> Option<ItemId> {
    match payload(ui) {
        Some(DragPayload::Item(i)) => Some(i),
        _ => None,
    }
}
/// The items a drag from the Project panel carries: the whole selection when the dragged item is
/// one of several selected, otherwise just the dragged item.
pub fn dragged_selection(app: &FilmcraftApp, item: ItemId) -> Vec<ItemId> {
    let sel = &app.session.state.project_selection;
    // the selection can hold bins beside items (one id space): only items can be placed
    if sel.len() > 1 && sel.contains(&item) { sel.iter().copied().filter(|i| app.session.project.item(*i).is_some()).collect() } else { vec![item] }
}
pub fn dragged_effect(ui: &egui::Ui) -> Option<String> {
    match payload(ui) {
        Some(DragPayload::Effect(e)) => Some(e),
        _ => None,
    }
}
pub fn clear_drag(ui: &egui::Ui) {
    ui.ctx().data_mut(|d| d.insert_temp::<Option<DragPayloadBox>>(payload_id(), None));
}

fn cancelled_id() -> egui::Id {
    egui::Id::new("filmcraft-drag-cancelled")
}

/// Abandon the drag in progress (Escape, #580): nothing is dropped, and nothing restarts it until
/// the button comes up.
pub fn cancel_drag(ui: &egui::Ui) {
    clear_drag(ui);
    ui.ctx().data_mut(|d| d.insert_temp(cancelled_id(), true));
}

/// Whether the press in progress had its drag cancelled with Escape.
pub fn drag_cancelled(ui: &egui::Ui) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(cancelled_id())).unwrap_or(false)
}

/// Draw the drag ghost near the pointer and clear the payload after release. Escape cancels it.
pub fn drag_ghost(app: &FilmcraftApp, ui: &egui::Ui) {
    if !ui.ctx().input(|i| i.pointer.any_down()) {
        ui.ctx().data_mut(|d| d.remove::<bool>(cancelled_id()));
    }
    let Some(pl) = payload(ui) else { return };
    let ctx = ui.ctx();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel_drag(ui);
        return;
    }
    if let Some(p) = ctx.pointer_hover_pos() {
        let label = match &pl {
            DragPayload::Item(i) => match dragged_selection(app, *i).len() {
                1 => app.session.project.item(*i).map(|x| x.name.clone()).unwrap_or_default(),
                n => format!("{n} items"),
            },
            DragPayload::Bin(b) => app.session.project.root.find_bin(filmcraft_project::BinId(*b)).map(|x| x.name.clone()).unwrap_or_default(),
            DragPayload::Source(s) => {
                let name = app.session.project.item(s.item).map(|i| i.name.as_str()).unwrap_or("Source");
                let mode = match (s.video, s.audio) {
                    (true, true) => "video + audio",
                    (true, false) => "video",
                    _ => "audio",
                };
                format!("{name} · {mode} · {:.2} s", s.range.duration.seconds())
            }
            DragPayload::Template(_, name) => name.clone(),
            DragPayload::Effect(e) => match (e.strip_prefix("preset:"), e.strip_prefix(LUMETRI_PRESET_PREFIX)) {
                (Some(name), _) => name.to_string(),
                (_, Some(name)) => crate::i18n::t(name).to_string(),
                _ => filmcraft_project::find_effect(e).map(|d| d.name.to_string()).unwrap_or_default(),
            },
        };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag-ghost")));
        let r = Rect::from_min_size(p + egui::vec2(12.0, 8.0), egui::vec2(label.len() as f32 * 7.0 + 16.0, 20.0));
        painter.rect_filled(r, 4.0, Color32::from_black_alpha(200));
        painter.text(r.center(), Align2::CENTER_CENTER, label, Tokens::ui(11.5), Color32::WHITE);
    }
    if ctx.input(|i| i.pointer.any_released()) {
        // cleared next frame so drop targets see the release this frame
        let f = ctx.cumulative_frame_nr();
        let k = egui::Id::new("drag-release-frame");
        match ctx.data(|d| d.get_temp::<u64>(k)) {
            Some(prev) if prev < f => {
                clear_drag(ui);
                ctx.data_mut(|d| d.remove::<u64>(k));
            }
            None => {
                ctx.data_mut(|d| d.insert_temp(k, f));
            }
            Some(_) => {}
        }
    } else if !ctx.input(|i| i.pointer.any_down()) {
        clear_drag(ui);
    }
}

/// The panel "≡" menu.
pub fn panel_menu_popup(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    drag_ghost(app, ui);
    // bins opened in new windows (Project panel)
    project::floating(app, ui.ctx());
    let id = egui::Id::new("panel-menu");
    let Some((p, pos)) = ui.ctx().data(|d| d.get_temp::<(PanelKind, egui::Pos2)>(id)) else { return };
    let mut close = false;
    // kept inside the window: moved up (by its size last frame) when it would run off the bottom
    let size_id = id.with("size");
    let last = ui.ctx().data(|d| d.get_temp::<egui::Vec2>(size_id)).unwrap_or_default();
    let screen = ui.ctx().content_rect();
    let pos = egui::pos2(pos.x.min(screen.max.x - last.x - 4.0).max(screen.min.x), pos.y.min(screen.max.y - last.y - 4.0).max(screen.min.y));
    // opened this frame (by a click over the tab, not the menu)
    let fresh = ui.ctx().data(|d| d.get_temp::<u64>(egui::Id::new("panel-menu-opened"))) == Some(ui.ctx().cumulative_frame_nr());
    // the area is the root of its submenus (Panel Group Settings ▸): shown this frame, so egui
    // keeps an open submenu open
    egui::containers::menu::MenuState::mark_shown(ui.ctx(), id.with("area"));
    // Measured afresh (an invisible sizing pass) each time it opens: one area serves every
    // panel's menu, and it keeps its last size, so the scroll area inside would otherwise stay as
    // short as the previous, shorter menu and hide the items below.
    let menu = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(pos).default_height(screen.height()).sizing_pass(fresh);
    let area = menu.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let scroll =
                egui::ScrollArea::vertical().id_salt("panel-menu-scroll").max_height((ui.ctx().content_rect().height() - 16.0).max(0.0)).show(ui, |ui| {
                    ui.set_min_width(190.0);
                    // the Timeline's tabs are its open sequences: Close Panel closes the active one and
                    // keeps the panel (Premiere's wording and behaviour)
                    if p == PanelKind::Timeline && app.session.state.active_sequence.is_some() {
                        let r = ui.button(tl!("Sequence Settings…"));
                        app.auto.add("panel.menu.Timeline.sequenceSettings", r.rect, "Sequence Settings…");
                        if r.clicked() {
                            if let Err(e) = crate::menus::invoke(app, ui.ctx(), "sequence.settings", serde_json::json!({})) {
                                app.ui.status = e;
                            }
                            close = true;
                        }
                        ui.separator();
                        let r = ui.button(tl!("Close Panel"));
                        app.auto.add("panel.menu.Timeline.close", r.rect, "Close Panel");
                        if r.clicked() {
                            let _ = app.session.execute("sequence.close", serde_json::json!({}));
                            close = true;
                        }
                        let r = ui.add_enabled(app.session.state.open_sequences.len() > 1, egui::Button::new(tl!("Close Other Timeline Panels")));
                        app.auto.add("panel.menu.Timeline.closeOthers", r.rect, "Close Other Timeline Panels");
                        if r.clicked() {
                            let _ = app.session.execute("sequence.closeOthers", serde_json::json!({}));
                            close = true;
                        }
                    } else {
                        let r = ui.button(tl!("Close Panel"));
                        app.auto.add(&format!("panel.menu.{}.close", p.id()), r.rect, "Close Panel");
                        if r.clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "window.closePanel", serde_json::json!({"panel": p.id()}));
                            close = true;
                        }
                    }
                    let undock = ui.button(tl!("Undock Panel"));
                    app.auto.add_response(&format!("panel.menu.{}.undock", p.id()), &undock, "Undock Panel");
                    if undock.clicked() {
                        let _ = crate::menus::invoke(app, ui.ctx(), "window.floatPanel", serde_json::json!({"panel": p.id()}));
                        close = true;
                    }
                    // Premiere's Panel Group Settings: close the group's other tabs or the whole
                    // group, undock the whole (docked) group, maximize it
                    let floating = app.ui.floating.iter().find(|g| g.panels.contains(&p)).map(|g| g.panels.clone());
                    let docked = app.ui.dock.group_of(p).map(|(g, _)| g);
                    let undockable = docked.is_some();
                    let group = docked.or(floating).unwrap_or_else(|| vec![p]);
                    let others: Vec<PanelKind> = group.iter().copied().filter(|x| *x != p).collect();
                    let maximized = app.ui.keys.maximized.is_some_and(|q| app.ui.dock.contains(q) || app.ui.floating.iter().any(|g| g.panels.contains(&q)));
                    // a submenu at the right (▸), as in Premiere: a drop-down would cover Maximize
                    // Frame and Restore Workspace below it
                    let (gs, _) = egui::containers::menu::SubMenuButton::new(tl!("Panel Group Settings")).ui(ui, |ui| {
                        let r = ui.add_enabled(!others.is_empty(), egui::Button::new(tl!("Close Other Panels in Group")));
                        app.auto.add(&format!("panel.menu.{}.closeOthers", p.id()), r.rect, "Close Other Panels in Group");
                        let mut closing: &[PanelKind] = &[];
                        if r.clicked() {
                            closing = &others;
                        }
                        let r = ui.button(tl!("Close Panel Group"));
                        app.auto.add(&format!("panel.menu.{}.closeGroup", p.id()), r.rect, "Close Panel Group");
                        if r.clicked() {
                            closing = &group;
                        }
                        for q in closing {
                            let _ = crate::menus::invoke(app, ui.ctx(), "window.closePanel", serde_json::json!({"panel": q.id()}));
                            close = true;
                        }
                        let r = ui.add_enabled(undockable, egui::Button::new(tl!("Undock Panel Group")));
                        app.auto.add_response(&format!("panel.menu.{}.undockGroup", p.id()), &r, "Undock Panel Group");
                        if r.clicked() {
                            let rect = crate::dock_ui::default_float_rect(ui.ctx().content_rect());
                            if let Err(e) = app.undock_group(p, rect) {
                                app.ui.status = e;
                            }
                            close = true;
                        }
                        let label = if maximized { "Restore Panel Group" } else { "Maximize Panel Group" };
                        let r = ui.button(if maximized { tl!("Restore Panel Group") } else { tl!("Maximize Panel Group") });
                        app.auto.add(&format!("panel.menu.{}.maximizeGroup", p.id()), r.rect, label);
                        if r.clicked() {
                            keyboard::toggle_maximize(app, p);
                            close = true;
                        }
                    });
                    app.auto.add(&format!("panel.menu.{}.groupSettings", p.id()), gs.rect, "Panel Group Settings");
                    let r = ui.button(if maximized { tl!("Restore Frame") } else { tl!("Maximize Frame") });
                    app.auto.add(&format!("panel.menu.{}.maximize", p.id()), r.rect, if maximized { "Restore Frame" } else { "Maximize Frame" });
                    if r.clicked() {
                        keyboard::toggle_maximize(app, p);
                        close = true;
                    }
                    let r = ui.button(tl!("Restore Workspace"));
                    app.auto.add(&format!("panel.menu.{}.restoreWorkspace", p.id()), r.rect, "Restore Workspace");
                    if r.clicked() {
                        let w = app.ui.workspace.clone();
                        app.set_workspace(&w);
                        close = true;
                    }
                    if p == PanelKind::Timeline {
                        ui.separator();
                        let r = ui.add_enabled(app.session.state.active_sequence.is_some(), egui::Button::new(tl!("Reveal Sequence in Project")));
                        app.auto.add("panel.menu.Timeline.revealSequence", r.rect, "Reveal Sequence in Project");
                        if r.clicked() {
                            let _ = app.session.execute("sequence.revealInProject", serde_json::json!({}));
                            close = true;
                        }
                        ui.checkbox(&mut app.ui.timeline.show_thumbnails, tl!("Video Thumbnails"));
                        ui.checkbox(&mut app.ui.timeline.show_waveforms, tl!("Audio Waveforms"));
                    }
                    if p == PanelKind::Project {
                        ui.separator();
                        close |= project::panel_menu(app, ui);
                    }
                    if p == PanelKind::MediaBrowser {
                        ui.separator();
                        close |= media_browser::panel_menu(app, ui);
                    }
                    if p == PanelKind::Source {
                        ui.separator();
                        close |= monitor::source_panel_menu(app, ui);
                    }
                });
            app.auto.add("panel.menu.scroll", scroll.inner_rect, "Panel menu");
        });
    });
    let size = area.response.rect.size();
    ui.ctx().data_mut(|d| d.insert_temp(size_id, size));
    // A click elsewhere or Escape closes the menu (not the click that opened it, `fresh`).
    if close || (!fresh && area.response.clicked_elsewhere()) || crate::widgets::escape_closes(ui.ctx()) {
        ui.ctx().data_mut(|d| d.remove::<(PanelKind, egui::Pos2)>(id));
    }
}

pub(crate) mod transcript_cache;
