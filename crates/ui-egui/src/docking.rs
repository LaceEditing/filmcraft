//! Moving panels around (Premiere's docking): drag a tab — or a whole group by the empty part of
//! its tab strip — onto another group's tabs or centre (it becomes a tab there), onto one of its
//! four sides (the group splits), or onto an outer edge of the dock area (a full-height column or
//! full-width row). A blue overlay shows where it will land; Escape cancels; Ctrl (Cmd) at the
//! drop undocks instead. Undocked groups are windows of their own (they can go to another
//! screen), with a panel menu to dock them back.
//!
//! The arrangement is kept per workspace and reopened at launch, until Window ▸ Workspaces ▸
//! Reset to Saved Layout or Save Changes ([`dock::WorkspacePrefs::live`]).
//!
//! Everything is also a command (`window.dock.*`, see [`route`]) for the control channel.

use egui::{Align2, Color32, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::dock::{self, DockAction, DockNode, DropTarget, FloatingGroup, PanelKind, Zone};
use crate::theme::Tokens;

/// A panel or group being dragged to a new place.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub panels: Vec<PanelKind>,
    pub active: usize,
}

/// Docking state that lives only while the app runs.
#[derive(Default)]
pub struct DockingState {
    pub drag: Option<Drag>,
    /// The tabs drawn this frame per group path (drop positions between tabs).
    pub tab_rects: Vec<(String, Vec<(PanelKind, Rect)>)>,
    /// An undocked group's panel menu: (group id, panel, where).
    pub float_menu: Option<(u64, PanelKind, egui::Pos2)>,
    /// The layout last written to the workspaces file, and frames since the last check.
    persisted: Option<(DockNode, Vec<FloatingGroup>)>,
    frames: u64,
    /// Groups undocked during this frame: their panels were already drawn in the dock, so their
    /// windows open from the next frame.
    fresh: Vec<u64>,
    /// Where closed or undocked panels were (this session), to put them back there.
    homes: std::collections::HashMap<PanelKind, (PanelKind, Zone)>,
}

impl DockingState {
    /// The layout as it now is in the workspaces file (a workspace just opened).
    pub fn set_persisted(&mut self, dock: &DockNode, floating: &[FloatingGroup]) {
        self.persisted = Some((dock.clone(), floating.to_vec()));
    }
}

impl FilmcraftApp {
    /// Whether `p` is open: docked or in an undocked window.
    pub fn panel_open(&self, p: PanelKind) -> bool {
        self.ui.dock.contains(p) || self.ui.floating.iter().any(|f| f.panels.contains(&p))
    }

    /// Back to the workspace's saved layout (Window ▸ Workspaces ▸ Reset to Saved Layout).
    pub fn reset_workspace(&mut self, name: &str) {
        // the changes being dropped must not be saved again by the switch below
        if self.ui.workspace == name {
            let (dock, floating) = (self.ui.dock.clone(), self.ui.floating.clone());
            self.docking.set_persisted(&dock, &floating);
        }
        if self.workspaces.live.iter().any(|s| s.name == name) {
            let mut next = self.workspaces.clone();
            next.live.retain(|s| s.name != name);
            if let Err(e) = self.set_workspaces(next) {
                self.ui.status = e;
            }
        }
        self.set_workspace(name);
    }

    /// Take `panels` out of the dock (or their window) into a window of their own.
    pub fn undock(&mut self, panels: &[PanelKind], active: usize, size: [f32; 2]) -> Option<u64> {
        let panels: Vec<PanelKind> = panels.iter().copied().filter(|p| self.panel_open(*p)).collect();
        if panels.is_empty() {
            return None;
        }
        self.take_out(&panels);
        let id = self.ui.floating.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        let active = active.min(panels.len() - 1);
        self.ui.keys.maximized = None;
        self.ui.floating.push(FloatingGroup { id, panels, active, size: [size[0].clamp(240.0, 3000.0), size[1].clamp(160.0, 2000.0)] });
        self.docking.fresh.push(id);
        Some(id)
    }

    /// Remove `panels` from wherever they are (dock or undocked windows; empty windows close),
    /// remembering where docked ones were.
    fn take_out(&mut self, panels: &[PanelKind]) {
        for p in panels {
            // a home outside the moved set (a group leaving together goes back beside its neighbour)
            let mut probe = self.ui.dock.clone();
            for q in panels.iter().filter(|q| *q != p) {
                probe.close(*q);
            }
            if let Some(h) = probe.home_of(*p) {
                self.docking.homes.insert(*p, h);
            }
        }
        for p in panels {
            self.ui.dock.close(*p);
        }
        for f in &mut self.ui.floating {
            let shown = f.panels.get(f.active).copied();
            f.panels.retain(|p| !panels.contains(p));
            f.active = shown.and_then(|s| f.panels.iter().position(|p| *p == s)).unwrap_or(0);
        }
        self.ui.floating.retain(|f| !f.panels.is_empty());
    }

    /// Dock an undocked group back where it came from (or where its panels normally live).
    pub fn redock(&mut self, id: u64) -> bool {
        let Some(i) = self.ui.floating.iter().position(|f| f.id == id) else { return false };
        let f = self.ui.floating.remove(i);
        if self.ui.dock.is_empty() {
            self.ui.dock.move_to_edge(&f.panels, f.active, Zone::Center);
        } else if let Some(&(target, zone)) = f.panels.iter().find_map(|p| self.docking.homes.get(p)).filter(|(t, _)| self.ui.dock.contains(*t)) {
            let mut d = self.ui.dock.clone();
            if d.move_panels(&f.panels, f.active, target, zone, None) {
                self.ui.dock = d;
            }
        }
        // whatever could not go home goes where it normally lives
        for p in &f.panels {
            if !self.ui.dock.contains(*p) {
                self.show_panel(*p);
            }
        }
        if let Some(p) = f.panels.get(f.active) {
            self.ui.dock.activate(*p);
            self.ui.focused = *p;
        }
        true
    }

    /// Put a closed panel back where it was closed from, if that place is still there.
    pub(crate) fn reopen_at_home(&mut self, p: PanelKind) -> bool {
        let Some(&(target, zone)) = self.docking.homes.get(&p) else { return false };
        if !self.ui.dock.contains(target) {
            return false;
        }
        let mut d = self.ui.dock.clone();
        if d.move_panels(&[p], 0, target, zone, None) {
            self.ui.dock = d;
            true
        } else {
            false
        }
    }

    /// Close panels wherever they are.
    pub fn close_panels(&mut self, panels: &[PanelKind]) {
        self.take_out(panels);
        if self.ui.keys.maximized.is_some_and(|m| panels.contains(&m)) {
            self.ui.keys.maximized = None;
        }
    }

    /// Dock `drag` at `target` (panels from an undocked window too).
    fn drop_at(&mut self, drag: &Drag, target: DropTarget) -> bool {
        // undocked panels join the dock tree first so the tree move finds them
        let floating: Vec<PanelKind> = drag.panels.iter().copied().filter(|p| !self.ui.dock.contains(*p)).collect();
        let mut dock = self.ui.dock.clone();
        let ok = match target {
            DropTarget::Group { target, zone, index, .. } => dock.move_panels(&drag.panels, drag.active, target, zone, index),
            DropTarget::Edge { zone, .. } => dock.move_to_edge(&drag.panels, drag.active, zone),
        };
        if ok {
            self.ui.dock = dock;
            if !floating.is_empty() {
                for f in &mut self.ui.floating {
                    f.panels.retain(|p| !floating.contains(p));
                }
                self.ui.floating.retain(|f| !f.panels.is_empty());
            }
            if let Some(p) = drag.panels.get(drag.active) {
                self.ui.focused = *p;
            }
        }
        ok
    }

    /// While a panel is dragged over the dock: show where it would land, and dock it on release.
    pub(crate) fn dock_drag(&mut self, ui: &egui::Ui, body: Rect, groups: &[dock::Group]) {
        let Some(drag) = self.docking.drag.clone() else { return };
        let t = self.tokens;
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.docking.drag = None;
            return;
        }
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let target = pos.and_then(|p| dock::drop_target(groups, body, p, t.tab_h, &self.docking.tab_rects, &drag.panels));
        // Ctrl (Cmd) held: undock instead (also read from the release itself: scripted input
        // carries its modifiers on the button event)
        let undock = ctx.input(|i| {
            i.modifiers.command || i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, modifiers, .. } if modifiers.command))
        });
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("dock-drop")));
        if let Some(target) = target.filter(|_| !undock) {
            let h = match target {
                DropTarget::Group { highlight, .. } | DropTarget::Edge { highlight, .. } => highlight,
            };
            painter.rect_filled(h, t.radius, t.focus.gamma_multiply(0.22));
            painter.rect_stroke(h, t.radius, Stroke::new(2.0, t.focus), StrokeKind::Inside);
        }
        if let Some(p) = pos {
            let title = drag.panels.get(drag.active).map(|p| p.title()).unwrap_or("");
            let label = if drag.panels.len() > 1 { format!("{title} (+{})", drag.panels.len() - 1) } else { title.to_string() };
            let label = if undock { format!("{label} — undock") } else { label };
            let g = painter.layout_no_wrap(label, Tokens::ui(12.0), Color32::WHITE);
            let r = Rect::from_min_size(p + vec2(14.0, 10.0), g.size() + vec2(16.0, 8.0));
            painter.rect_filled(r, 4.0, Color32::from_black_alpha(210));
            painter.galley(r.min + vec2(8.0, 4.0), g, Color32::WHITE);
        }
        ctx.request_repaint();
        if ctx.input(|i| !i.pointer.primary_down()) {
            self.docking.drag = None;
            if undock {
                let size = groups
                    .iter()
                    .find(|g| g.panels.iter().any(|p| drag.panels.contains(p)))
                    .map(|g| [g.rect.width(), g.rect.height()])
                    .unwrap_or([640.0, 420.0]);
                self.undock(&drag.panels, drag.active, size);
            } else if let Some(target) = target {
                self.drop_at(&drag, target);
            }
        }
    }

    /// Draw every undocked group in a window of its own.
    pub(crate) fn floating_windows(&mut self, ctx: &egui::Context) {
        let fresh = std::mem::take(&mut self.docking.fresh);
        let groups = self.ui.floating.clone();
        for fg in groups.into_iter().filter(|f| !fresh.contains(&f.id)) {
            let title = format!("{} — FilmCraft", fg.panels.get(fg.active).map(|p| p.title()).unwrap_or("Panel"));
            let vid = egui::ViewportId::from_hash_of(("filmcraft-panel-window", fg.id));
            let builder = egui::ViewportBuilder::default().with_title(title.clone()).with_inner_size(fg.size).with_app_id("ai.storyteller.filmcraft");
            ctx.show_viewport_immediate(vid, builder, |vctx, class| {
                if vctx.input(|i| i.viewport().close_requested()) {
                    // closing the window closes its panels (Window ▸ reopens them, docked)
                    let panels = fg.panels.clone();
                    self.close_panels(&panels);
                    return;
                }
                if let Some(r) = vctx.input(|i| i.viewport().inner_rect)
                    && let Some(f) = self.ui.floating.iter_mut().find(|f| f.id == fg.id)
                {
                    f.size = [r.width(), r.height()];
                }
                let bg = self.tokens.app_bg;
                // (without native windows, e.g. on the web, egui wraps this in an in-app window)
                let _ = class;
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(bg).inner_margin(4.0)).show(vctx, |ui| self.floating_group(ui, fg.id));
            });
        }
    }

    /// One undocked group: its tab strip and the shown panel.
    fn floating_group(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(f) = self.ui.floating.iter().find(|f| f.id == id).cloned() else { return };
        let t = self.tokens;
        let rect = ui.max_rect();
        let tab_h = if f.panels.len() == 1 && f.panels[0].compact() { 8.0 } else { t.tab_h };
        let content = Rect::from_min_max(pos2(rect.min.x, rect.min.y + tab_h), rect.max);
        let g = dock::Group { path: format!("float{id}"), rect, content, panels: f.panels.clone(), active: f.active };
        let seqs = self.seq_tabs();
        let (actions, _) = dock::draw_group_chrome(ui, &g, self.ui.focused, &seqs, &t, &mut self.auto);
        if let Some(p) = f.panels.get(f.active).copied() {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).id_salt(("float-panel", id, p.id())));
            child.set_clip_rect(content);
            crate::panels::show(self, &mut child, p, content);
        }
        for a in actions {
            match a {
                DockAction::Activate(p) => {
                    if let Some(f) = self.ui.floating.iter_mut().find(|f| f.id == id)
                        && let Some(i) = f.panels.iter().position(|x| *x == p)
                    {
                        f.active = i;
                    }
                }
                DockAction::Focus(p) => self.ui.focused = p,
                DockAction::Close(p) => self.close_panels(&[p]),
                DockAction::PanelMenu(p, at) => {
                    self.docking.float_menu = Some((id, p, at));
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("float-menu-opened"), ui.ctx().cumulative_frame_nr()));
                }
                other => self.sequence_tab_action(&ui.ctx().clone(), other),
            }
        }
        self.float_menu_popup(ui, id);
    }

    /// The panel menu of an undocked group.
    fn float_menu_popup(&mut self, ui: &egui::Ui, id: u64) {
        let Some((gid, p, at)) = self.docking.float_menu.filter(|m| m.0 == id) else { return };
        let mut done = false;
        let area = egui::Area::new(egui::Id::new(("float-menu", gid))).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(190.0);
                if ui.button("Dock Panel Group").clicked() {
                    self.redock(gid);
                    done = true;
                }
                if ui.button("Close Panel").clicked() {
                    self.close_panels(&[p]);
                    done = true;
                }
                let others: Vec<PanelKind> =
                    self.ui.floating.iter().find(|f| f.id == gid).map(|f| f.panels.iter().copied().filter(|x| *x != p).collect()).unwrap_or_default();
                if ui.add_enabled(!others.is_empty(), egui::Button::new("Close Other Panels in Group")).clicked() {
                    self.close_panels(&others);
                    done = true;
                }
                if ui.button("Close Panel Group").clicked() {
                    let all: Vec<PanelKind> = self.ui.floating.iter().find(|f| f.id == gid).map(|f| f.panels.clone()).unwrap_or_default();
                    self.close_panels(&all);
                    done = true;
                }
            });
        });
        let fresh = ui.ctx().data(|d| d.get_temp::<u64>(egui::Id::new("float-menu-opened"))) == Some(ui.ctx().cumulative_frame_nr());
        if done || (!fresh && area.response.clicked_elsewhere()) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.docking.float_menu = None;
        }
    }

    /// Write the arrangement to the workspaces file when it changed (checked about once a
    /// second, and at exit with `force`), so the workspace reopens as it was left.
    pub(crate) fn persist_layout(&mut self, force: bool) {
        self.docking.frames += 1;
        if !force && (!self.docking.frames.is_multiple_of(60) || self.docking.drag.is_some()) {
            return;
        }
        let now = (self.ui.dock.clone(), self.ui.floating.clone());
        if self.docking.persisted.as_ref() == Some(&now) {
            return;
        }
        let name = self.ui.workspace.clone();
        let mut next = self.workspaces.clone();
        next.live.retain(|s| s.name != name);
        next.live.push(dock::SavedWorkspace::new(&name, &now.0, &now.1));
        next.current = name;
        match self.set_workspaces(next) {
            Ok(()) => self.docking.persisted = Some(now),
            Err(e) => self.ui.status = e,
        }
    }
}

/// `window.dock.*` commands (control channel / MCP):
///
/// - `window.dock.layout` → `{workspace, dock, floating}`
/// - `window.dock.move {panel | panels: [..], active?, target?, zone: center|left|right|top|bottom, index?}`:
///   onto the group holding `target`, or without `target` onto that outer edge of the dock area
/// - `window.dock.undock {panel, group?: bool}`: the panel, or its whole group, into a window
/// - `window.dock.redock {panel}`: dock the undocked group holding `panel` back
/// - `window.dock.close {panel, group?: bool, others?: bool}`: close the panel, its group, or the
///   other panels of its group
/// - `window.dock.maximize {panel}`: toggle Maximize Frame
pub fn route(app: &mut FilmcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let cmd = id.strip_prefix("window.dock.")?;
    let panel = |k: &str| -> Result<PanelKind, String> {
        let name = p.get(k).and_then(Value::as_str).ok_or_else(|| format!("need `{k}`"))?;
        PanelKind::from_name(name).ok_or_else(|| format!("unknown panel `{name}`"))
    };
    let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
    let group_of = |app: &FilmcraftApp, x: PanelKind| -> (Vec<PanelKind>, usize) {
        app.ui.dock.group_of(x).or_else(|| app.ui.floating.iter().find(|f| f.panels.contains(&x)).map(|f| (f.panels.clone(), f.active))).unwrap_or((vec![x], 0))
    };
    let r = (|| -> Result<Value, String> {
        match cmd {
            "layout" => Ok(json!({"workspace": app.ui.workspace, "dock": app.ui.dock, "floating": app.ui.floating})),
            "move" => {
                let panels: Vec<PanelKind> = match p.get("panels").and_then(Value::as_array) {
                    Some(a) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|n| PanelKind::from_name(n).ok_or_else(|| format!("unknown panel `{n}`")))
                        .collect::<Result<_, _>>()?,
                    None => vec![panel("panel")?],
                };
                if let Some(x) = panels.iter().find(|x| !app.panel_open(**x)) {
                    return Err(format!("{} is not open", x.title()));
                }
                let zone =
                    p.get("zone").and_then(Value::as_str).map_or(Some(Zone::Center), Zone::from_name).ok_or("`zone` is center, left, right, top or bottom")?;
                let active = p.get("active").and_then(Value::as_u64).unwrap_or(0) as usize;
                let drag = Drag { panels: panels.clone(), active };
                let r = Rect::NOTHING;
                let target = match p.get("target") {
                    Some(_) => {
                        DropTarget::Group { target: panel("target")?, zone, index: p.get("index").and_then(Value::as_u64).map(|i| i as usize), highlight: r }
                    }
                    None => DropTarget::Edge { zone, highlight: r },
                };
                if !app.drop_at(&drag, target) {
                    return Err("nothing to move there (the target is one of the moved panels, or not docked)".into());
                }
                Ok(json!({"dock": app.ui.dock}))
            }
            "undock" => {
                let x = panel("panel")?;
                let (panels, active) = if flag("group") { group_of(app, x) } else { (vec![x], 0) };
                let id = app.undock(&panels, active, [640.0, 420.0]).ok_or_else(|| format!("{} is not open", x.title()))?;
                Ok(json!({"window": id, "floating": app.ui.floating}))
            }
            "redock" => {
                let x = panel("panel")?;
                let id = app.ui.floating.iter().find(|f| f.panels.contains(&x)).map(|f| f.id).ok_or_else(|| format!("{} is not undocked", x.title()))?;
                app.redock(id);
                Ok(json!({"dock": app.ui.dock}))
            }
            "close" => {
                let x = panel("panel")?;
                let (group, _) = group_of(app, x);
                let panels: Vec<PanelKind> = if flag("group") {
                    group
                } else if flag("others") {
                    group.into_iter().filter(|g| *g != x).collect()
                } else {
                    vec![x]
                };
                app.close_panels(&panels);
                Ok(json!({"closed": panels.iter().map(|p| p.title()).collect::<Vec<_>>()}))
            }
            "maximize" => {
                let x = panel("panel")?;
                app.ui.keys.maximized = if app.ui.keys.maximized == Some(x) || !app.ui.dock.contains(x) { None } else { Some(x) };
                Ok(json!({"maximized": app.ui.keys.maximized.map(|p| p.title())}))
            }
            other => Err(format!("unknown command `window.dock.{other}`")),
        }
    })();
    Some(r)
}

/// Paint a drop hint for the panel menu (unused shapes kept out of the dock module).
pub(crate) fn hint(painter: &egui::Painter, rect: Rect, t: &Tokens, text: &str) {
    painter.text(rect.center(), Align2::CENTER_CENTER, text, Tokens::ui(12.0), t.text_faint);
}
