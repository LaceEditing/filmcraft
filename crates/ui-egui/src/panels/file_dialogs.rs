//! The crash-recovery prompt and the Revert confirmation (Settings ▸ Auto Save lives in
//! `panels::settings`).
//!
//! Automation ids: `recovery.item.<n>`, `recovery.recover`, `recovery.discard`, `recovery.later`;
//! `revert.yes`, `revert.no`; `saveNotes.note.<n>`, `saveNotes.ok`.

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, vec2};
use serde_json::json;

use crate::theme::Tokens;
use crate::{Dialog, FilmcraftApp};

/// Dialog-local state.
#[derive(Default)]
pub struct FileDialogState {
    pub recovery_choice: usize,
    /// What the last Save could not write into the Premiere Pro project (path, notes).
    pub save_notes: Option<(String, Vec<String>)>,
    /// The notes last shown, by path: the same ones are not shown again on every Save.
    pub save_notes_seen: std::collections::HashMap<String, Vec<String>>,
}

impl FileDialogState {
    /// Whether these notes are news (not the ones last shown for this file).
    pub fn notes_are_new(&self, path: &str, notes: &[String]) -> bool {
        self.save_notes_seen.get(path).is_none_or(|seen| seen.as_slice() != notes)
    }
}

fn modal_frame(t: &Tokens) -> Frame {
    Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.separator)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(0))
}

fn button(app: &mut FilmcraftApp, ui: &mut Ui, id: &str, label: &str, primary: bool) -> bool {
    let t = app.tokens;
    let text = RichText::new(label).size(13.0).color(if primary { Color32::WHITE } else { t.text });
    let b = egui::Button::new(text)
        .min_size(vec2(88.0, 30.0))
        .corner_radius(CornerRadius::same(15))
        .fill(if primary { t.accent } else { Color32::TRANSPARENT })
        .stroke(if primary { Stroke::NONE } else { Stroke::new(1.5, t.text_faint) });
    let r = ui.add(b);
    app.auto.add(id, r.rect, label);
    r.clicked()
}

pub fn show_recovery(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    let t = app.tokens;
    let list = app.session.execute("file.recoveryList", json!({})).unwrap_or_default();
    let items = list.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return false;
    }
    let mut choice = app.file_dialogs.recovery_choice.min(items.len() - 1);
    let mut open = true;
    egui::Modal::new(egui::Id::new("recovery-modal")).frame(modal_frame(&t)).show(ctx, |ui| {
        ui.set_width(520.0);
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }).show(ui, |ui| {
            ui.label(RichText::new("Recover Unsaved Changes").size(18.0).strong().color(t.text));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            let c = &items[choice];
            let name = c["projectName"].as_str().unwrap_or("Untitled");
            let when = c["savedAt"].as_str().unwrap_or("");
            let why = if c["cleanExit"].as_bool() == Some(true) {
                format!("FilmCraft was closed while “{name}” had unsaved changes.")
            } else {
                format!("FilmCraft quit unexpectedly while “{name}” had unsaved changes.")
            };
            ui.label(RichText::new(why).size(13.5).color(t.text));
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Recover unsaved changes from {when}?")).size(13.5).color(t.text));
            if let Some(p) = c["projectPath"].as_str() {
                ui.label(RichText::new(p).size(11.5).color(t.text_dim));
            } else {
                ui.label(RichText::new("The project had not been saved yet.").size(11.5).color(t.text_dim));
            }
            if items.len() > 1 {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} sessions have unsaved changes:", items.len())).size(12.0).color(t.text_dim));
                for (i, it) in items.iter().enumerate() {
                    let label = format!("{} — {}", it["projectName"].as_str().unwrap_or("Untitled"), it["savedAt"].as_str().unwrap_or(""));
                    let r = ui.radio(choice == i, RichText::new(&label).size(12.5));
                    app.auto.add(&format!("recovery.item.{i}"), r.rect, &label);
                    if r.clicked() {
                        choice = i;
                    }
                }
            }
            ui.add_space(18.0);
            let id = items[choice]["id"].as_str().unwrap_or("").to_string();
            ui.horizontal(|ui| {
                if button(app, ui, "recovery.discard", "Discard", false) {
                    if let Err(e) = app.session.execute("file.discardRecovery", json!({ "id": id })) {
                        app.ui.status = e.to_string();
                    }
                    choice = 0;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(app, ui, "recovery.recover", "Recover", true) {
                        match app.session.execute("file.recover", json!({ "id": id })) {
                            Ok(_) => open = false,
                            Err(e) => app.ui.status = e.to_string(),
                        }
                    }
                    if button(app, ui, "recovery.later", "Not Now", false) {
                        open = false;
                    }
                });
            });
        });
    });
    app.file_dialogs.recovery_choice = choice;
    open && !app.session.recovery_candidates().is_empty()
}

/// "Save changes to '<project>' before closing?": Save (then close once the save went through),
/// Don't Save (close; the changes stay in the recovery journal), Cancel. Automation ids
/// `closeConfirm.save`, `closeConfirm.dontSave`, `closeConfirm.cancel`.
pub fn show_close(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    let t = app.tokens;
    let name = app.session.project.name.clone();
    let mut open = true;
    let mut close_now = false;
    let r = egui::Modal::new(egui::Id::new("close-modal")).frame(modal_frame(&t)).show(ctx, |ui| {
        ui.set_width(460.0);
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }).show(ui, |ui| {
            ui.label(RichText::new("Close").size(18.0).strong().color(t.text));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Save changes to '{name}' before closing?")).size(13.5).color(t.text));
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(app, ui, "closeConfirm.save", "Save", true) {
                    // a save that left something out says so before the window goes
                    let news = |app: &FilmcraftApp| {
                        app.session
                            .events
                            .iter()
                            .any(|e| matches!(e, filmcraft_engine::Event::SaveNotes { path, notes } if app.file_dialogs.notes_are_new(path, notes)))
                    };
                    match crate::menus::invoke(app, ctx, "file.save", json!({})) {
                        // a save that needed a dialog and was cancelled leaves the project dirty
                        Ok(_) if !app.session.is_dirty() && !news(app) => close_now = true,
                        Ok(_) => {}
                        Err(e) => app.ui.status = e,
                    }
                    open = false;
                }
                if button(app, ui, "closeConfirm.dontSave", "Don't Save", false) {
                    close_now = true;
                    open = false;
                }
                if button(app, ui, "closeConfirm.cancel", "Cancel", false) {
                    open = false;
                }
            });
        });
    });
    if close_now {
        app.close_confirmed = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    open && !r.should_close()
}

pub fn show_revert(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    let t = app.tokens;
    let file = app.session.path.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut open = true;
    let r = egui::Modal::new(egui::Id::new("revert-modal")).frame(modal_frame(&t)).show(ctx, |ui| {
        ui.set_width(460.0);
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }).show(ui, |ui| {
            ui.label(RichText::new("Revert").size(18.0).strong().color(t.text));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Are you sure you want to discard your changes to '{file}'?")).size(13.5).color(t.text));
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(app, ui, "revert.yes", "Yes", true) {
                    if let Err(e) = app.session.execute("file.revert", json!({})) {
                        app.ui.status = e.to_string();
                    }
                    open = false;
                }
                if button(app, ui, "revert.no", "No", false) {
                    open = false;
                }
            });
        });
    });
    open && !r.should_close()
}

/// "Saved, but not everything is in the Premiere Pro project": what Save could not write (the same
/// list is in the Events panel). Shown when the list changes, not on every Save.
pub fn show_save_notes(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    let t = app.tokens;
    let Some((path, notes)) = app.file_dialogs.save_notes.clone() else { return false };
    let file = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    let mut open = true;
    let r = egui::Modal::new(egui::Id::new("save-notes-modal")).frame(modal_frame(&t)).show(ctx, |ui| {
        ui.set_width(560.0);
        Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }).show(ui, |ui| {
            ui.label(RichText::new("Not everything was saved").size(18.0).strong().color(t.text));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "'{file}' was saved, but these changes cannot be written into a Premiere Pro project yet, so they will be missing when it is opened again. To keep them too, also save a FilmCraft copy (File \u{25b8} Save a Copy\u{2026}, as .fcproj)."
                ))
                .size(13.0)
                .color(t.text),
            );
            ui.add_space(10.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for (i, n) in notes.iter().enumerate() {
                    let r = ui.label(RichText::new(format!("\u{2022} {n}")).size(12.5).color(t.text_dim));
                    app.auto.add(&format!("saveNotes.note.{i}"), r.rect, n);
                }
            });
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(app, ui, "saveNotes.ok", "OK", true) {
                    open = false;
                }
            });
        });
    });
    let open = open && !r.should_close();
    if !open {
        app.file_dialogs.save_notes_seen.insert(path, notes);
        app.file_dialogs.save_notes = None;
    }
    open
}

/// Draw the dialog if it is one of ours; returns Some(still open).
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context, d: Dialog) -> Option<bool> {
    match d {
        Dialog::Preferences => Some(crate::panels::settings::show(app, ctx)),
        Dialog::Recovery => Some(show_recovery(app, ctx)),
        Dialog::RevertConfirm => Some(show_revert(app, ctx)),
        Dialog::CloseConfirm => Some(show_close(app, ctx)),
        Dialog::SaveNotes => Some(show_save_notes(app, ctx)),
        _ => None,
    }
}
