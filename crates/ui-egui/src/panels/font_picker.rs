//! The font picker of text layers (Effect Controls and Properties): a field showing the family; a
//! click opens a list of every font on the system with a search field (type to narrow it down),
//! the fonts used recently first, arrow keys / Return / Esc, opening upwards when there is no room
//! below. The system's fonts are scanned in the background when the app starts.
//!
//! Automation ids: `<id>` (the field), `<id>.search`, `<id>.option.<family>`.

use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, CornerRadius, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::FilmcraftApp;
use crate::theme::Tokens;

/// Font families and their styles, cached (the list changes only when fonts are scanned).
pub fn families() -> Arc<Vec<(String, Vec<String>)>> {
    static CACHE: Mutex<Option<(bool, std::time::Instant, Arc<Vec<(String, Vec<String>)>>)>> = Mutex::new(None);
    let scanned = filmcraft_text::fonts::system_scanned();
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, at, list)) = c.as_ref()
        && *s == scanned
        && at.elapsed().as_secs() < 30
    {
        return list.clone();
    }
    let list = Arc::new(filmcraft_text::families());
    *c = Some((scanned, std::time::Instant::now(), list.clone()));
    list
}

/// Scan the system's fonts on a background thread (once), so the picker opens at once.
pub fn scan_in_background() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("font-scan".into()).spawn(|| {
            filmcraft_text::fonts::scan_system();
        });
    });
}

#[derive(Clone, Default)]
struct PickerState {
    open: bool,
    query: String,
    /// Highlighted row of the filtered list.
    hi: usize,
    /// Scroll the highlighted row into view on the next frame.
    reveal: bool,
}

const ROW_H: f32 = 22.0;
const RECENT: usize = 8;

/// The font field; returns the family picked this frame.
pub fn font_picker(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    family: &str,
    width: f32,
    auto_id: &str,
) -> Option<String> {
    let t = app.tokens;
    let id = egui::Id::new(("font-picker", salt));
    let mut st: PickerState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // the field, drawn like a combo box
    let (r, resp) = ui.allocate_exact_size(vec2(width, ui.spacing().interact_size.y.max(22.0)), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 3.0, if resp.hovered() { t.hover } else { t.field_bg });
    p.rect_stroke(r, 3.0, Stroke::new(1.0, if st.open { t.accent } else { t.field_border }), StrokeKind::Inside);
    let shown = if family.is_empty() { "(none)" } else { family };
    p.with_clip_rect(r.shrink2(vec2(6.0, 0.0))).text(pos2(r.min.x + 7.0, r.center().y), Align2::LEFT_CENTER, shown, Tokens::ui(12.0), t.text);
    let c = pos2(r.max.x - 10.0, r.center().y);
    p.add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)], t.text_dim, Stroke::NONE));
    app.auto.add(auto_id, r, family);
    if resp.clicked() {
        st.open = !st.open;
        st.query.clear();
        st.hi = 0;
        st.reveal = true;
        if st.open {
            scan_in_background();
        }
    }
    let mut picked = None;
    if st.open {
        let all = families();
        let q = st.query.trim().to_lowercase();
        // recently used first (when not searching), then every family
        let mut rows: Vec<(String, bool)> = Vec::new();
        if q.is_empty() {
            for f in app.ui.recent_fonts.iter().take(RECENT) {
                if all.iter().any(|(a, _)| a.eq_ignore_ascii_case(f)) {
                    rows.push((f.clone(), true));
                }
            }
        }
        rows.extend(all.iter().filter(|(f, _)| q.is_empty() || f.to_lowercase().contains(&q)).map(|(f, _)| (f.clone(), false)));
        // the current family highlighted when the list opens
        if st.reveal
            && q.is_empty()
            && let Some(i) = rows.iter().position(|(f, recent)| !recent && f == family)
        {
            st.hi = i;
        }
        st.hi = st.hi.min(rows.len().saturating_sub(1));
        // room below the field, else above it
        let screen = ui.ctx().content_rect();
        let want_h = 420.0f32;
        let below = screen.max.y - r.max.y - 8.0;
        let above = r.min.y - screen.min.y - 8.0;
        let (h, open_up) = if below >= want_h.min(300.0) || below >= above { (want_h.min(below), false) } else { (want_h.min(above), true) };
        let pw = width.max(260.0);
        let pos = if open_up { pos2(r.min.x, r.min.y - h) } else { pos2(r.min.x, r.max.y + 2.0) };
        let area = egui::Area::new(id.with("popup")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true);
        let ar = area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(6)).show(ui, |ui| {
                ui.set_width(pw);
                ui.set_max_height(h);
                let sr = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Search fonts").desired_width(pw - 12.0));
                app.auto.add(&format!("{auto_id}.search"), sr.rect, "Search fonts");
                if st.reveal {
                    sr.request_focus();
                }
                if sr.changed() {
                    st.hi = 0;
                    st.reveal = true;
                }
                // keys while the list is open (the search field keeps the focus)
                let (up, down, enter, esc) = ui.input(|i| {
                    (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
                });
                if down && st.hi + 1 < rows.len() {
                    st.hi += 1;
                    st.reveal = true;
                }
                if up && st.hi > 0 {
                    st.hi -= 1;
                    st.reveal = true;
                }
                if enter && let Some((f, _)) = rows.get(st.hi) {
                    picked = Some(f.clone());
                }
                if esc {
                    st.open = false;
                }
                if !filmcraft_text::fonts::system_scanned() {
                    ui.label(egui::RichText::new("Loading the system's fonts…").size(11.5).color(t.text_dim));
                }
                if rows.is_empty() {
                    ui.label(egui::RichText::new("No font matches").size(12.0).color(t.text_dim));
                }
                let mut sa = egui::ScrollArea::vertical().max_height(h - 56.0).auto_shrink([false, true]);
                if st.reveal {
                    // keep the highlighted row in view
                    let top = st.hi as f32 * ROW_H;
                    sa = sa.vertical_scroll_offset((top - (h - 56.0) / 2.0).max(0.0));
                }
                sa.show_rows(ui, ROW_H, rows.len(), |ui, range| {
                    for i in range {
                        let Some((f, recent)) = rows.get(i) else { continue };
                        let (rr, rresp) = ui.allocate_exact_size(vec2(pw - 12.0, ROW_H), Sense::click());
                        let on = f == family;
                        let bg = if i == st.hi || rresp.hovered() {
                            t.hover
                        } else if on {
                            t.row_selected
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rr, 3.0, bg);
                        ui.painter().text(pos2(rr.min.x + 8.0, rr.center().y), Align2::LEFT_CENTER, f, Tokens::ui(12.5), if on { t.accent } else { t.text });
                        if *recent {
                            ui.painter().text(pos2(rr.max.x - 6.0, rr.center().y), Align2::RIGHT_CENTER, "recent", Tokens::ui(10.5), t.text_faint);
                        }
                        app.auto.add(&format!("{auto_id}.option.{f}"), rr, f);
                        if rresp.clicked() {
                            picked = Some(f.clone());
                        }
                    }
                });
            });
        });
        st.reveal = false;
        // a click elsewhere closes the list
        let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !ar.response.rect.contains(p) && !r.contains(p)));
        if outside {
            st.open = false;
        }
    }
    if let Some(f) = &picked {
        st.open = false;
        app.ui.recent_fonts.retain(|x| x != f);
        app.ui.recent_fonts.insert(0, f.clone());
        app.ui.recent_fonts.truncate(RECENT);
    }
    ui.data_mut(|d| d.insert_temp(id, st));
    picked
}

/// The style field of a family (Regular, Bold, …, and the synthetic Bold / Italic).
pub fn style_picker(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    family: &str,
    style: &str,
    width: f32,
    auto_id: &str,
) -> Option<String> {
    let all = families();
    let styles: Vec<String> = all.iter().find(|(f, _)| f.eq_ignore_ascii_case(family)).map(|(_, s)| s.clone()).unwrap_or_default();
    let mut list: Vec<&str> = Vec::new();
    for s in styles.iter().map(String::as_str).chain(["Regular", "Bold", "Italic", "Bold Italic"]) {
        if !list.contains(&s) {
            list.push(s);
        }
    }
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(("font-style", salt)).selected_text(style).width(width).height(320.0).show_ui(ui, |ui| {
        for s in &list {
            let o = ui.selectable_label(*s == style, *s);
            app.auto.add(&format!("{auto_id}.option.{s}"), o.rect, s);
            if o.clicked() {
                picked = Some(s.to_string());
            }
        }
    });
    app.auto.add(auto_id, r.response.rect, style);
    picked
}
