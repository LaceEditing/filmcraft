//! Headless UI tests of moving panels around (Premiere's docking): dragging a tab into another
//! group or beside it, a group by its strip, the dock's outer edges, the tab ×, the panel
//! menu (Undock, Close Other Panels, Maximize Frame), Window ▸ checkmarks, and the arrangement
//! reopening after a restart until Reset to Saved Layout.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::dock::{DockNode, PanelKind};
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

fn data_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/docking-tests").join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Driver {
    fn new(dir: &Path) -> Self {
        let mut s = Session::default();
        s.prefs_path = Some(dir.join("preferences.json"));
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(6);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array().unwrap().iter().any(|e| e["id"] == json!(id))
    }

    fn drag(&mut self, from: Value, to: Value) {
        self.ok("ui.drag", json!({"from": from, "to": to, "steps": 10}));
        self.frames(4);
    }

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn dock(&mut self) -> DockNode {
        self.app().ui.dock.clone()
    }

    fn group(&mut self, p: PanelKind) -> Vec<PanelKind> {
        self.dock().group_of(p).map(|g| g.0).unwrap_or_default()
    }

    /// A tab of the Project group that is on screen (the strip may not show them all).
    fn visible_tab(&mut self, candidates: &[PanelKind]) -> PanelKind {
        for p in candidates {
            if self.has(&format!("panel.tab.{}", p.id())) {
                return *p;
            }
        }
        let tabs = self.ok("ui.elements", json!({"prefix": "panel."}));
        let ids: Vec<&str> = tabs.as_array().unwrap().iter().filter_map(|e| e["id"].as_str()).collect();
        panic!("none of {candidates:?} has a tab on screen: {ids:?}\n{:?} max {:?}", self.dock(), self.app().ui.keys.maximized);
    }
}

#[test]
fn tabs_move_into_groups_and_beside_them() {
    let dir = data_dir("move");
    let mut d = Driver::new(&dir);
    let p = d.visible_tab(&[PanelKind::Project, PanelKind::MediaBrowser, PanelKind::Libraries]);
    // into the Program monitor's group (its centre)
    d.drag(json!({"id": format!("panel.tab.{}", p.id())}), json!({"id": "panel.Program"}));
    assert_eq!(d.group(PanelKind::Program), [PanelKind::Program, p], "{:?}", d.dock());
    assert!(d.dock().is_visible(p), "the moved tab is shown");
    // beside the Timeline: dropped on its top quarter, the Timeline's place splits
    d.drag(json!({"id": format!("panel.tab.{}", p.id())}), json!({"id": "panel.Timeline", "fx": 0.5, "fy": 0.08}));
    assert_eq!(d.group(p), [p]);
    assert_eq!(d.group(PanelKind::Timeline), [PanelKind::Timeline]);
    // a whole group by the empty part of its strip, to the right edge of the dock area
    let (props, _) = d.dock().group_of(PanelKind::Properties).unwrap();
    d.drag(json!({"id": "panel.grip.Properties", "fx": 0.6}), json!({"x": 1596.0, "y": 500.0}));
    let dock = d.dock();
    let DockNode::Split { vertical: false, b, .. } = &dock else { panic!("{dock:?}") };
    assert_eq!(**b, DockNode::Tabs { panels: props, active: 0 }, "a full-height column at the right");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn close_reopen_undock_and_maximize() {
    let dir = data_dir("close");
    let mut d = Driver::new(&dir);
    let p = d.visible_tab(&[PanelKind::MediaBrowser, PanelKind::Libraries, PanelKind::Info]);
    // the tab's ×
    d.ok("ui.click", json!({"id": format!("panel.close.{}", p.id())}));
    d.frames(3);
    assert!(!d.app().panel_open(p));
    let menu = d.ok("ui.menu.list", json!({}));
    let item = menu.as_array().unwrap().iter().find(|i| i["id"] == json!(format!("window.panel.{}", p.id()))).cloned().unwrap();
    assert_eq!(item["checked"], json!(false), "Window ▸ unticked when closed");
    d.exec(&format!("window.panel.{}", p.id()), json!({}));
    d.frames(2);
    assert!(d.app().panel_open(p), "Window ▸ reopens it");
    let menu = d.ok("ui.menu.list", json!({}));
    let item = menu.as_array().unwrap().iter().find(|i| i["id"] == json!(format!("window.panel.{}", p.id()))).cloned().unwrap();
    assert_eq!(item["checked"], json!(true));
    // panel menu ▸ Undock Panel: a window of its own, out of the dock
    d.ok("ui.click", json!({"id": "panel.menu.Program"}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "panel.menu.Program.undock"}));
    d.frames(3);
    assert!(!d.dock().contains(PanelKind::Program));
    assert_eq!(d.app().ui.floating.len(), 1);
    assert!(d.app().panel_open(PanelKind::Program));
    // showing it (Window ▸ / a shortcut) does not add a second one to the dock
    d.exec("window.panel.Program", json!({}));
    assert!(!d.dock().contains(PanelKind::Program));
    // dock it back: where it was
    let docked = d.dock();
    d.exec("window.dock.redock", json!({"panel": "Program"}));
    assert!(d.dock().contains(PanelKind::Program) && d.app().ui.floating.is_empty());
    assert_eq!(d.group(PanelKind::Program), [PanelKind::Program], "its own group again, not a tab elsewhere: {:?}", d.dock());
    assert_ne!(d.dock(), docked);
    // Maximize Frame keeps the layout
    let before = d.dock();
    d.ok("ui.click", json!({"id": "panel.menu.Timeline"}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "panel.menu.Timeline.groupSettings"}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "panel.menu.Timeline.maximize"}));
    d.frames(2);
    assert_eq!(d.app().ui.keys.maximized, Some(PanelKind::Timeline));
    assert_eq!(d.dock(), before, "the layout is kept while maximized");
    // Ctrl at the drop undocks
    d.app().ui.keys.maximized = None;
    d.frames(2);
    let q = d.visible_tab(&[PanelKind::EffectControls, PanelKind::Source]);
    d.ok("ui.drag", json!({"from": {"id": format!("panel.tab.{}", q.id())}, "to": {"id": "panel.Program"}, "steps": 8, "modifiers": {"command": true}}));
    d.frames(4);
    assert!(d.app().ui.floating.iter().any(|f| f.panels == [q]), "{:?}", d.app().ui.floating);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_arrangement_reopens_until_reset() {
    let dir = data_dir("persist");
    let moved = {
        let mut d = Driver::new(&dir);
        d.exec("window.dock.move", json!({"panel": "Effects", "target": "Program", "zone": "left"}));
        d.exec("window.dock.undock", json!({"panel": "History"}));
        d.frames(70);
        let moved = d.dock();
        assert_eq!(d.group(PanelKind::Effects), [PanelKind::Effects]);
        moved
    };
    // a new app on the same data directory reopens it as it was left
    let mut d = Driver::new(&dir);
    d.frames(4);
    assert_eq!(d.dock(), moved);
    assert!(d.app().ui.floating.iter().any(|f| f.panels == [PanelKind::History]));
    // Reset to Saved Layout
    d.exec("window.workspace.reset", json!({}));
    assert_eq!(d.dock(), filmcraft_ui_egui::dock::workspace("Editing"));
    assert!(d.app().ui.floating.is_empty());
    d.frames(70);
    drop(d);
    let mut d = Driver::new(&dir);
    d.frames(4);
    assert_eq!(d.dock(), filmcraft_ui_egui::dock::workspace("Editing"), "and the reset sticks");
    let _ = std::fs::remove_dir_all(&dir);
}
