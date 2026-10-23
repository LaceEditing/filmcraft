//! Headless UI tests of the Timeline's sequence tabs, the clip context menu and the Nest… name
//! dialog.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::project::ItemKind;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            // the control channel's clicks and keys enter through the input hook
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

    fn menu(&mut self, id: &str) -> Value {
        let r = self.ok("ui.menu.invoke", json!({"id": id}));
        self.frames(3);
        r
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array().unwrap().iter().any(|e| e["id"] == json!(id))
    }

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn v1(&mut self, i: usize) -> filmcraft_engine::project::TrackItem {
        self.app().session.active_sequence().unwrap().video_tracks[0].items[i].clone()
    }
}

impl Driver {
    fn sequences(&mut self) -> Vec<(u64, String)> {
        self.app().session.project.items.values().filter(|i| matches!(i.kind, ItemKind::Sequence(_))).map(|i| (i.id.0, i.name.clone())).collect()
    }

    fn has_timeline(&mut self) -> bool {
        self.app().ui.dock.contains(filmcraft_ui_egui::dock::PanelKind::Timeline)
    }
}

#[test]
fn nest_asks_for_a_name_and_numbers_the_default() {
    let mut d = Driver::new();
    let main = d.app().session.state.active_sequence.unwrap();
    let a = d.v1(0);
    d.exec("timeline.select", json!({"clips": [a.id.0]}));
    assert_eq!(d.menu("clip.nest")["dialog"], "nest");
    assert_eq!(d.app().ui.clip_dialog.as_ref().unwrap().params["name"], "Nested Sequence 01");
    assert!(d.has("nest.name") && d.has("nest.cancel"));
    d.click("nest.ok");
    assert!(d.app().ui.clip_dialog.is_none(), "closed on OK");
    assert!(d.sequences().iter().any(|s| s.1 == "Nested Sequence 01"));
    // like Premiere, nesting stays in the sequence being edited
    assert_eq!(d.app().session.state.active_sequence, Some(main));
    // the next one is numbered on; Cancel nests nothing
    let b = d.v1(1);
    d.exec("timeline.select", json!({"clips": [b.id.0]}));
    d.menu("clip.nest");
    assert_eq!(d.app().ui.clip_dialog.as_ref().unwrap().params["name"], "Nested Sequence 02");
    d.click("nest.cancel");
    assert!(!d.sequences().iter().any(|s| s.1 == "Nested Sequence 02"));
    // without a name the command still picks an unused one
    d.exec("clip.nest", json!({"name": ""}));
    assert!(d.sequences().iter().any(|s| s.1 == "Nested Sequence 02"));
}

#[test]
fn clip_context_menu_offers_nest_and_make_subsequence() {
    let mut d = Driver::new();
    let a = d.v1(0);
    d.ok("ui.click", json!({"id": format!("timeline.clip.{}", a.id.0), "button": "right"}));
    d.frames(3);
    assert!(d.app().session.state.selection.contains(&a.id), "right-click selects the clip");
    for id in ["edit.cut", "edit.pasteAttributes", "clip.enable", "clip.group", "clip.nest", "sequence.makeSubsequence", "clip.frameHoldOptions"] {
        assert!(d.has(&format!("timeline.clipMenu.{id}")), "{id}");
    }
    d.click("timeline.clipMenu.clip.nest");
    assert_eq!(d.app().ui.clip_dialog.as_ref().map(|c| c.command.clone()).as_deref(), Some("clip.nest"), "Nest… asks for the name");
    d.click("nest.cancel");
    let before = d.sequences().len();
    d.ok("ui.click", json!({"id": format!("timeline.clip.{}", a.id.0), "button": "right"}));
    d.frames(3);
    d.click("timeline.clipMenu.sequence.makeSubsequence");
    assert_eq!(d.sequences().len(), before + 1);
}

#[test]
fn timeline_has_a_tab_per_open_sequence() {
    let mut d = Driver::new();
    let main = d.app().session.state.active_sequence.unwrap().0;
    let a = d.v1(0);
    d.exec("timeline.select", json!({"clips": [a.id.0]}));
    let nested = d.exec("clip.nest", json!({"name": "Inner"}))["sequence"].as_u64().unwrap();
    assert!(d.has(&format!("timeline.tab.{main}")));
    assert!(!d.has(&format!("timeline.tab.{nested}")), "not opened until asked");
    // double-clicking the nested clip opens it in a second tab
    let clip = {
        let q = d.app().session.active_sequence().unwrap();
        q.video_tracks[0].items.iter().find(|i| i.item.0 == nested).unwrap().id.0
    };
    d.ok("ui.click", json!({"id": format!("timeline.clip.{clip}"), "count": 2}));
    d.frames(4);
    assert_eq!(d.app().session.state.active_sequence.map(|i| i.0), Some(nested));
    assert!(d.has(&format!("timeline.tab.{main}")) && d.has(&format!("timeline.tab.{nested}")));
    // tabs switch sequences; × closes one and the panel stays
    d.click(&format!("timeline.tab.{main}"));
    assert_eq!(d.app().session.state.active_sequence.map(|i| i.0), Some(main));
    d.click(&format!("timeline.tab.{main}.close"));
    assert_eq!(d.app().session.state.active_sequence.map(|i| i.0), Some(nested));
    assert!(!d.has(&format!("timeline.tab.{main}")));
    d.click(&format!("timeline.tab.{nested}.close"));
    assert_eq!(d.app().session.state.active_sequence, None);
    assert!(d.has_timeline(), "closing the last sequence keeps the Timeline panel");
    assert!(d.has("panel.tab.Timeline"));
    // reopening from the project brings it back, even if the panel itself was closed
    d.app().ui.dock.close(filmcraft_ui_egui::dock::PanelKind::Timeline);
    d.frames(2);
    assert!(!d.has_timeline());
    d.exec("sequence.open", json!({"item": main}));
    d.frames(3);
    assert!(d.has_timeline());
    assert!(d.has(&format!("timeline.tab.{main}")));
}

/// Sequence ▸ Sequence Settings… (also on a sequence's Project panel and tab menus) opens a
/// dialog with the active sequence's settings; OK applies them. There was no way to change
/// them in the app before.
#[test]
fn sequence_settings_dialog_edits_the_active_sequence() {
    let mut d = Driver::new();
    let before = d.app().session.active_sequence().unwrap().settings.clone();
    d.menu("sequence.settings");
    assert!(matches!(d.app().dialog, Some(filmcraft_ui_egui::Dialog::SequenceSettings)), "the dialog opened");
    assert!(d.has("sequenceSettings.ok"));
    assert_eq!((d.app().ui.sequence_settings.width, d.app().ui.sequence_settings.height), (before.width, before.height), "filled from the sequence");
    d.app().ui.sequence_settings.width = 1280;
    d.app().ui.sequence_settings.height = 720;
    (d.app().ui.sequence_settings.fps_num, d.app().ui.sequence_settings.fps_den) = (25, 1);
    d.app().ui.sequence_settings.name = "Renamed".into();
    d.frames(2);
    assert!(d.has("sequenceSettings.scaleMotion"), "offered for a new frame size");
    d.click("sequenceSettings.ok");
    assert!(d.app().dialog.is_none(), "closed");
    let app = d.app();
    let active = app.session.state.active_sequence.unwrap();
    let q = app.session.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height, q.settings.frame_rate.as_f64()), (1280, 720, 25.0));
    assert_eq!(app.session.project.item(active).unwrap().name, "Renamed");
    // the sequence tab's right-click menu (the panel menu, as in Premiere) reaches it too
    let tab = format!("timeline.tab.{}", active.0);
    assert!(d.has(&tab));
    d.ok("ui.click", json!({"id": tab, "button": "right"}));
    d.frames(2);
    d.click("panel.menu.Timeline.sequenceSettings");
    assert!(matches!(d.app().dialog, Some(filmcraft_ui_egui::Dialog::SequenceSettings)));
    d.click("sequenceSettings.cancel");
    assert!(d.app().dialog.is_none());
}

/// Closing the window with unsaved changes asks first (Save / Don't Save / Cancel); it used to
/// close at once, leaving the changes to the recovery journal and the next launch's prompt.
#[test]
fn closing_with_unsaved_changes_asks_first() {
    let mut d = Driver::new();
    d.exec("playhead.set", json!({"seconds": 2.0}));
    d.exec("sequence.addEditAllTracks", json!({}));
    assert!(d.app().session.is_dirty());
    let close = |d: &mut Driver| {
        // the window's one close request (the harness keeps viewport events: clear it after)
        d.harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        d.frames(2);
        if let Some(v) = d.harness.input_mut().viewports.get_mut(&egui::ViewportId::ROOT) {
            v.events.clear();
        }
        d.frames(1);
    };
    close(&mut d);
    assert!(d.has("quit.save") && d.has("quit.dontSave") && d.has("quit.cancel"), "the prompt opened");
    assert!(!d.app().quit_confirmed);
    d.click("quit.cancel");
    assert!(d.app().ui.clip_dialog.is_none() && !d.app().quit_confirmed, "Cancel keeps the window");
    close(&mut d);
    d.click("quit.dontSave");
    assert!(d.app().quit_confirmed, "Don't Save lets the window close");
    assert!(d.app().ui.clip_dialog.is_none());
    // a clean project closes without asking
    let mut d = Driver::new();
    assert!(!d.app().session.is_dirty());
    close(&mut d);
    assert!(d.app().ui.clip_dialog.is_none() && !d.has("quit.save"));
}
