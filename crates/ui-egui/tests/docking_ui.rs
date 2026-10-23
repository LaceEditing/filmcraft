//! Headless UI tests of closing, reopening and undocking panels (Premiere's docking): the tab ×,
//! the panel menu (Undock, Maximize Frame), a compact panel's grip, and Window ▸ checkmarks.
//! Dragging panels into, beside and out of groups is tested with the shared docking model
//! (`it/shared_docking_ui.rs`).

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

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn dock(&mut self) -> DockNode {
        self.app().ui.dock.clone()
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
    // panel menu ▸ Undock Panel: it floats, out of the dock
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
    let menu = d.ok("ui.menu.list", json!({}));
    let item = menu.as_array().unwrap().iter().find(|i| i["id"] == json!("window.panel.Program")).cloned().unwrap();
    assert_eq!(item["checked"], json!(true), "Window ▸ ticked while floating");
    // dock it back, its own group beside the Source monitor (the floating panel is a window over
    // the dock, in front of the Timeline's ≡)
    d.exec("window.dockPanel", json!({"panel": "Program", "anchor": "Source", "zone": "right"}));
    d.frames(2);
    assert!(d.dock().contains(PanelKind::Program) && d.app().ui.floating.is_empty());
    assert_eq!(d.dock().group_of(PanelKind::Program).map(|(g, _)| g), Some(vec![PanelKind::Program]), "its own group: {:?}", d.dock());
    // Maximize Frame keeps the layout (the Timeline's menu is taller than the Program's: all of it shows)
    let before = d.dock();
    d.ok("ui.click", json!({"id": "panel.menu.Timeline"}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "panel.menu.Timeline.groupSettings"}));
    d.frames(2);
    for item in ["closeGroup", "undockGroup", "maximizeGroup"] {
        assert!(d.has(&format!("panel.menu.Timeline.{item}")), "Panel Group Settings ▸ {item}");
    }
    d.ok("ui.click", json!({"id": "panel.menu.Timeline.maximize"}));
    d.frames(2);
    assert_eq!(d.app().ui.keys.maximized, Some(PanelKind::Timeline));
    assert_eq!(d.dock(), before, "the layout is kept while maximized");
    // Panel Group Settings ▸ Undock Panel Group: the whole group floats, tabs in order, the shown one shown
    d.app().ui.keys.maximized = None;
    d.frames(2);
    let (group, active) = d.dock().group_of(p).unwrap();
    let shown = group[active];
    d.ok("ui.click", json!({"id": format!("panel.menu.{}", shown.id())}));
    d.frames(2);
    d.ok("ui.click", json!({"id": format!("panel.menu.{}.groupSettings", shown.id())}));
    d.frames(2);
    d.ok("ui.click", json!({"id": format!("panel.menu.{}.undockGroup", shown.id())}));
    d.frames(3);
    assert!(group.iter().all(|q| !d.dock().contains(*q)), "{:?}", d.dock());
    let floating = d.app().ui.floating.clone();
    assert_eq!(floating.len(), 1);
    assert_eq!(floating[0].panels, group);
    assert_eq!(floating[0].panels[floating[0].active], shown);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Panels without a tab (Tools, Audio Meters) close from their grip: right-click for the panel
/// menu, or its ×. Window ▸ a ticked panel closes it; the same command from a shortcut only
/// shows it.
#[test]
fn compact_panels_close_from_their_grip_and_the_window_menu_toggles() {
    let dir = data_dir("compact");
    let mut d = Driver::new(&dir);
    // right-click the Tools grip: its panel menu ▸ Close Panel
    d.ok("ui.click", json!({"id": "panel.drag.Tools", "button": "right"}));
    d.frames(2);
    assert!(d.has("panel.menu.Tools.close"), "the panel menu opened");
    d.ok("ui.click", json!({"id": "panel.menu.Tools.close"}));
    d.frames(2);
    assert!(!d.app().panel_open(PanelKind::Tools));
    // the meters' × on their grip
    d.ok("ui.click", json!({"id": "panel.close.AudioMeters"}));
    d.frames(2);
    assert!(!d.app().panel_open(PanelKind::AudioMeters));
    // Window ▸ Tools reopens it; choosing it again from the menu closes it
    d.ok("ui.menu.invoke", json!({"id": "window.panel.Tools", "params": {"fromMenu": true}}));
    assert!(d.app().panel_open(PanelKind::Tools));
    d.ok("ui.menu.invoke", json!({"id": "window.panel.Tools", "params": {"fromMenu": true}}));
    assert!(!d.app().panel_open(PanelKind::Tools), "a ticked panel closes from the Window menu");
    // a shortcut's (no fromMenu) only shows
    d.exec("window.panel.Program", json!({}));
    assert!(d.app().panel_open(PanelKind::Program));
    let _ = std::fs::remove_dir_all(&dir);
}
