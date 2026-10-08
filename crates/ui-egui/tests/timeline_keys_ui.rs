//! Keyboard shortcuts as the Timeline panel sees them: every default key runs the command the
//! shortcut set resolves it to (no shadowed or swallowed keys), and gaps on the timeline are
//! selected by clicking empty track space and closed with Clear, Ripple Delete or the gap menu.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::shortcuts::{Chord, Platform};
use filmcraft_project::ClipId;
use filmcraft_time::Tick;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    /// The demo project; `fired` (when given) records the commands keys run instead of running them.
    fn new(fired: Option<Rc<RefCell<Vec<String>>>>) -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(s).with_control(rx);
        app.hooks.open_path = Some(Box::new(|_, _| Ok(())));
        if let Some(fired) = fired {
            app.hooks.shortcut = Some(Box::new(move |id| {
                fired.borrow_mut().push(id.to_string());
                false
            }));
        }
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
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

    fn key(&mut self, key: &str) {
        self.ok("ui.key", json!({"key": key}));
        self.frames(3);
    }

    fn focus(&mut self, panel: &str) {
        self.ok("ui.set", json!({"focused": panel}));
        self.frames(2);
    }

    fn app(&self) -> &FilmcraftApp {
        self.harness.state()
    }

    fn start(&self, c: ClipId) -> Tick {
        self.app().session.active_sequence().and_then(|q| q.find_item(c)).map(|(_, i)| i.start).unwrap()
    }
}

/// Every application and Timeline key of the default set, pressed with the Timeline focused, runs
/// the command the set resolves that key to there. Keys a panel or widget swallowed, or that
/// fired some other binding, used to go unnoticed.
#[test]
fn every_timeline_key_runs_its_command() {
    let fired = Rc::new(RefCell::new(Vec::new()));
    let mut d = Driver::new(Some(fired.clone()));
    d.focus("Timeline");
    let platform = Platform::current();
    let shortcuts = d.app().session.shortcuts.clone();
    let mut seen = Vec::new();
    let mut wrong = Vec::new();
    for b in shortcuts.bindings.iter().filter(|b| b.panel.is_none() || b.panel.as_deref() == Some("Timeline")) {
        let Ok(chord) = Chord::parse(&b.keys) else { panic!("unparsable default `{}`", b.keys) };
        let key = chord.effective(platform).canonical();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        let expected = shortcuts.resolve(&chord, Some("Timeline"), platform).map(|r| r.command.clone()).unwrap();
        fired.borrow_mut().clear();
        d.key(&b.keys);
        let got = fired.borrow().clone();
        if got != [expected.clone()] {
            wrong.push(format!("{} ({key}): expected {expected}, ran {got:?}", b.keys));
        }
    }
    assert!(seen.len() > 150, "the default set has its keys: {}", seen.len());
    assert!(wrong.is_empty(), "{} keys misfired:\n{}", wrong.len(), wrong.join("\n"));
}

/// Where Ctrl is the command key (Windows, Linux), a ⌃-spelled macOS extra and a Cmd binding land
/// on the same keys: the Cmd binding runs, as the shortcut editor shows. Ctrl+M added a range
/// marker instead of opening Export and Ctrl+T toggled the trim type instead of adding text.
#[test]
fn ctrl_keys_run_the_primary_binding_off_macos() {
    if Platform::current().is_mac() {
        return;
    }
    let fired = Rc::new(RefCell::new(Vec::new()));
    let mut d = Driver::new(Some(fired.clone()));
    d.focus("Timeline");
    for (key, id) in [("Ctrl+M", "mode.export"), ("Cmd+M", "mode.export"), ("Ctrl+T", "graphics.newText"), ("Ctrl+9", "timeline.toggleAllAudioTargets")] {
        fired.borrow_mut().clear();
        d.key(key);
        assert_eq!(*fired.borrow(), [id], "{key}");
    }
}

/// Two 48-frame clips on V1/A1 with a 48-frame gap between them, zoomed to fit: (first, second).
fn gap_sequence(d: &mut Driver) -> (ClipId, ClipId) {
    d.exec("file.newSequence", json!({"name": "Gaps", "video": 1, "audio": 1}));
    let rate = d.app().session.sequence_rate();
    let item = d.app().session.project.items.values().find(|i| i.has_video() && i.has_audio() && i.as_media().is_some()).map(|i| i.id).unwrap();
    for frame in [0, 96] {
        d.exec("timeline.place", json!({"item": item.0, "frame": frame, "sourceIn": rate.tick_of(48).0, "duration": rate.tick_of(48).0}));
    }
    d.focus("Timeline");
    d.key("\\");
    d.frames(20);
    let q = d.app().session.active_sequence().unwrap();
    (q.video_tracks[0].items[0].id, q.video_tracks[0].items[1].id)
}

/// The middle of the gap between `a` and `b` on screen.
fn gap_point(d: &mut Driver, a: ClipId, b: ClipId) -> (f64, f64) {
    let out = d.ok("ui.timeline.locate", json!({"clip": a.0, "edge": "out"}));
    let inn = d.ok("ui.timeline.locate", json!({"clip": b.0, "edge": "in"}));
    let (x0, x1) = (out["x"].as_f64().unwrap(), inn["x"].as_f64().unwrap());
    assert!(x1 - x0 > 40.0, "the gap is visible: {x0}..{x1}");
    ((x0 + x1) / 2.0, out["y"].as_f64().unwrap())
}

fn click_gap(d: &mut Driver, a: ClipId, b: ClipId, button: &str) {
    let (x, y) = gap_point(d, a, b);
    d.ok("ui.click", json!({"x": x, "y": y, "button": button}));
    d.frames(3);
}

/// Clicking empty space between clips selects the gap (it is drawn), and Clear (Backspace and
/// Delete), Ripple Delete (Shift+Delete, Alt+Backspace in the Timeline) and the gap's context menu
/// close it: the later clip moves up. Clear did nothing because nothing was selected.
#[test]
fn clicking_a_gap_and_clearing_it_ripples() {
    let mut d = Driver::new(None);
    let (first, second) = gap_sequence(&mut d);
    let rate = d.app().session.sequence_rate();
    let (f48, f96) = (rate.tick_of(48), rate.tick_of(96));
    d.exec("timeline.select", json!({"clips": [first.0]}));

    for key in ["Backspace", "Delete", "Shift+Delete", "Alt+Backspace"] {
        click_gap(&mut d, first, second, "left");
        assert!(d.app().session.state.selection.is_empty(), "{key}: clicking the gap deselects the clip");
        assert!(d.app().session.selected_gap().is_some(), "{key}: the gap is selected");
        let gap = d.ok("ui.elements", json!({"prefix": "timeline.gap"}));
        assert_eq!(gap.as_array().map(Vec::len), Some(1), "{key}: the selected gap is drawn: {gap}");
        d.key(key);
        assert_eq!(d.start(second), f48, "{key} closes the gap");
        assert_eq!(d.app().session.history.undo.last().map(|u| u.0.clone()).as_deref(), Some("Ripple Delete"));
        d.key("Cmd+Z");
        assert_eq!(d.start(second), f96, "{key}: undo brings the gap back");
    }

    // right-clicking the gap offers Ripple Delete
    click_gap(&mut d, first, second, "right");
    d.frames(2);
    d.ok("ui.click", json!({"id": "timeline.gapMenu.rippleDelete"}));
    d.frames(3);
    assert_eq!(d.start(second), f48, "the gap menu's Ripple Delete closes it");
    d.key("Cmd+Z");

    // clicking empty space after the last clip is no gap: it only deselects
    d.exec("timeline.select", json!({"clips": [first.0]}));
    let end = d.ok("ui.timeline.locate", json!({"clip": second.0, "edge": "out"}));
    d.ok("ui.click", json!({"x": end["x"].as_f64().unwrap() + 30.0, "y": end["y"]}));
    d.frames(3);
    assert!(d.app().session.state.selection.is_empty() && d.app().session.selected_gap().is_none());
    d.key("Backspace");
    assert_eq!(d.start(second), f96, "nothing to clear");
}

/// Clear acts on the focused panel's selection, whichever command the key is bound to: Delete
/// bound to the Project panel's Clear (as older defaults had it) clears the timeline when the
/// Timeline has focus and never deletes the project items selected out of sight; the timeline's
/// Clear key in the Project panel deletes project items.
#[test]
fn delete_keys_act_on_the_focused_panel() {
    let mut d = Driver::new(None);
    let (first, second) = gap_sequence(&mut d);
    let rate = d.app().session.sequence_rate();
    d.exec("shortcuts.set", json!({"command": "project.delete", "keys": "Delete"}));
    d.exec("shortcuts.set", json!({"command": "edit.clear", "keys": "F5"}));
    // a project item the gap sequence does not use, selected in the Project panel
    let used = d.app().session.active_sequence().and_then(|q| q.find_item(first)).map(|(_, i)| i.item).unwrap();
    let victim = d.app().session.project.items.values().find(|i| i.as_media().is_some() && i.id != used).map(|i| i.id).unwrap();
    d.exec("project.select", json!({"items": [victim.0]}));

    d.focus("Timeline");
    click_gap(&mut d, first, second, "left");
    d.key("Delete");
    assert_eq!(d.start(second), rate.tick_of(48), "Delete in the Timeline closed the gap");
    assert!(d.app().session.project.item(victim).is_some(), "and deleted no project item");
    d.key("Cmd+Z");
    click_gap(&mut d, first, second, "left");
    d.key("F5");
    assert_eq!(d.start(second), rate.tick_of(48), "so does Clear's own key");

    d.focus("Project");
    d.exec("project.select", json!({"items": [victim.0]}));
    d.key("F5");
    assert!(d.app().session.project.item(victim).is_none(), "Clear in the Project panel deletes the selected item");
}

/// The numeric keypad's + and − zoom the timeline by default; bound to a command of their own they
/// run it and leave = and - zooming.
#[test]
fn keypad_plus_and_minus_zoom_or_run_their_own_binding() {
    let mut d = Driver::new(None);
    d.focus("Timeline");
    d.frames(10);
    let z0 = d.app().ui.timeline.target_pps;
    d.key("KeypadPlus");
    let z1 = d.app().ui.timeline.target_pps;
    assert!(z1 > z0, "Keypad + zooms in: {z0} → {z1}");
    d.key("KeypadMinus");
    assert!(d.app().ui.timeline.target_pps < z1, "Keypad − zooms out");

    d.exec("shortcuts.set", json!({"command": "timeline.expandAllTracks", "keys": "KeypadPlus"}));
    let (h0, z) = (d.app().ui.timeline.video_track_h, d.app().ui.timeline.target_pps);
    d.key("KeypadPlus");
    assert!(d.app().ui.timeline.video_track_h > h0, "Keypad + runs its own binding");
    assert_eq!(d.app().ui.timeline.target_pps, z, "and does not zoom");
    d.key("=");
    assert!(d.app().ui.timeline.target_pps > z, "= still zooms in");
}

/// Recording in the Keyboard Shortcuts dialog: the keypad's keys record as themselves (they
/// recorded as the main - and + keys, or not at all), and shifted punctuation records as the
/// Shift chord the defaults use.
#[test]
fn the_shortcut_editor_records_keypad_keys() {
    let mut d = Driver::new(None);
    d.ok("ui.menu.invoke", json!({"id": "app.keyboardShortcuts"}));
    d.frames(2);
    for (key, recorded) in
        [("KeypadMinus", "KeypadMinus"), ("KeypadPlus", "KeypadPlus"), ("Keypad5", "Keypad5"), ("Shift+=", "Shift+="), ("Shift+;", "Shift+;")]
    {
        d.ok("ui.click", json!({"id": "shortcuts.search"}));
        d.ok("ui.key", json!({"key": "Cmd+A"}));
        d.ok("ui.type", json!({"text": "add edit"}));
        d.frames(2);
        d.ok("ui.click", json!({"id": "shortcuts.add.sequence.addEdit"}));
        d.frames(1);
        d.ok("ui.key", json!({"key": key}));
        d.frames(2);
        let keys: Vec<String> = d.app().session.shortcuts.for_command("sequence.addEdit").iter().map(|b| b.keys.clone()).collect();
        assert!(keys.iter().any(|k| k == recorded), "{key} recorded as {recorded}: {keys:?}");
    }
    // the main keys keep their own bindings
    let minus: Vec<String> = d.app().session.shortcuts.bindings.iter().filter(|b| b.keys == "-").map(|b| b.command.clone()).collect();
    assert!(minus.contains(&"view.zoomOut".to_string()), "{minus:?}");
}
