//! Headless UI tests of Effect Controls: dragging a value is one undo step (Ctrl+Z goes back
//! to before the press), and keyframes set through the panel interpolate between them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

fn data_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/effect-controls-tests").join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new(dir: &Path) -> Self {
        let mut s = Session::default();
        s.prefs_path = Some(dir.join("preferences.json"));
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        // (frames 1/60 s apart, so two clicks can be a double-click)
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000).build_eframe(move |_cc| app);
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

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }
}

impl Driver {
    /// The first clip of V1 selected, Effect Controls showing it.
    fn with_clip(dir: &Path) -> (Self, u64) {
        let mut d = Driver::new(dir);
        let clip = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        d.ok("ui.click", json!({"id": format!("timeline.clip.{clip}")}));
        d.frames(2);
        d.exec("window.panel.EffectControls", json!({}));
        d.frames(3);
        (d, clip)
    }

    fn scale_at(&mut self, secs: f64) -> f64 {
        self.scale_at_tick(filmcraft_time::Tick::from_seconds_f64(secs))
    }

    fn scale_at_tick(&mut self, t: filmcraft_time::Tick) -> f64 {
        let it = self.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone();
        it.effect("motion").unwrap().f64_at("scale", it.source_time_at(t))
    }
}

#[test]
fn dragging_a_value_is_one_undo_step() {
    let dir = data_dir("undo");
    let (mut d, _) = Driver::with_clip(&dir);
    assert_eq!(d.scale_at(0.0), 100.0);
    let steps = d.app().session.history.undo.len();
    d.ok("ui.drag", json!({"from": {"id": "effectControls.motion.scale.value"}, "to": {"id": "effectControls.motion.scale.value", "fx": 4.0}, "steps": 30}));
    d.frames(3);
    let after = d.scale_at(0.0);
    assert!(after > 110.0, "the drag changed the scale ({after})");
    assert_eq!(d.app().session.history.undo.len(), steps + 1, "one undo step for the whole drag");
    d.ok("ui.key", json!({"key": "Cmd+Z"}));
    d.frames(2);
    assert_eq!(d.scale_at(0.0), 100.0, "undo goes back to before the press");
    // a second drag is a step of its own
    d.ok("ui.drag", json!({"from": {"id": "effectControls.motion.scale.value"}, "to": {"id": "effectControls.motion.scale.value", "fx": 3.0}, "steps": 10}));
    d.frames(2);
    d.ok("ui.drag", json!({"from": {"id": "effectControls.motion.scale.value"}, "to": {"id": "effectControls.motion.scale.value", "fx": 3.0}, "steps": 10}));
    d.frames(2);
    let twice = d.scale_at(0.0);
    d.ok("ui.key", json!({"key": "Cmd+Z"}));
    d.frames(2);
    let once = d.scale_at(0.0);
    assert!(once > 100.0 && once < twice, "undo took back the second drag only ({once} vs {twice})");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keyframes_set_in_the_panel_interpolate() {
    let dir = data_dir("keys");
    let (mut d, _) = Driver::with_clip(&dir);
    d.exec("playhead.set", json!({"seconds": 1.0}));
    d.frames(2);
    let t1 = d.app().session.playhead();
    d.ok("ui.click", json!({"id": "effectControls.motion.scale.stopwatch"}));
    d.frames(2);
    d.exec("playhead.set", json!({"seconds": 3.0}));
    d.frames(2);
    let t3 = d.app().session.playhead();
    d.ok("ui.drag", json!({"from": {"id": "effectControls.motion.scale.value"}, "to": {"id": "effectControls.motion.scale.value", "fx": 5.0}, "steps": 20}));
    d.frames(3);
    let (a, b) = (d.scale_at_tick(t1), d.scale_at_tick(t3));
    assert_eq!(a, 100.0);
    assert!(b > 120.0, "a second keyframe at 3 s ({b})");
    let mid = d.scale_at(2.0);
    assert!(mid > a + 1.0 && mid < b - 1.0, "interpolated at 2 s: {mid} between {a} and {b}");
    // the lane shows both keyframes on the Scale row
    let keys: Vec<String> = d
        .ok("ui.elements", json!({"prefix": "effectControls.motion.scale.keyframe."}))
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str().map(String::from))
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Effect Controls: clicking an effect's name selects it, Cmd/Ctrl-click adds another, and
/// Save Preset… on the selection saves them together (it used to take one effect only).
#[test]
fn several_effects_select_and_save_as_one_preset() {
    let dir = data_dir("multi-select");
    let (mut d, clip) = Driver::with_clip(&dir);
    d.exec("effects.apply", json!({"clips": [clip], "effect": "Gaussian Blur"}));
    d.exec("effects.apply", json!({"clips": [clip], "effect": "Tint"}));
    // fold the fixed effects so both rows are in view
    let fixed: Vec<String> = d.app().session.active_sequence().unwrap().video_tracks[0].items[0]
        .effects
        .iter()
        .enumerate()
        .filter(|(_, e)| e.def().is_some_and(|d| d.intrinsic))
        .map(|(i, _)| format!("{clip}:{i}"))
        .collect();
    d.app().ui.collapsed_fx.extend(fixed);
    d.frames(3);
    d.ok("ui.click", json!({"id": "effectControls.effect.gaussian_blur"}));
    d.frames(2);
    assert_eq!(d.app().ui.fx_selected.len(), 1);
    d.ok("ui.click", json!({"id": "effectControls.effect.tint", "modifiers": {"command": true}}));
    d.frames(2);
    assert_eq!(d.app().ui.fx_selected.len(), 2, "Cmd-click added the second effect");
    d.ok("ui.click", json!({"id": "effectControls.effect.tint", "button": "right"}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "effectControls.effect.tint.savePreset"}));
    d.frames(2);
    let draft = d.app().ui.save_preset.clone().expect("the Save Preset dialog");
    assert_eq!(draft.effects.len(), 2, "both effects go into the preset");
    d.app().ui.save_preset = None;
    d.frames(2);
    // a plain click selects one again
    d.ok("ui.click", json!({"id": "effectControls.effect.gaussian_blur"}));
    d.frames(2);
    assert_eq!(d.app().ui.fx_selected.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

impl Driver {
    /// Media times of the Scale keyframes shown in the lane.
    fn scale_keys(&mut self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .ok("ui.elements", json!({"prefix": "effectControls.motion.scale.keyframe."}))
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["id"].as_str()?.rsplit('.').next()?.parse().ok())
            .collect();
        v.sort_unstable();
        v
    }

    /// Scale keyframes at 1, 2 and 3 s (100, 150, 120).
    fn three_scale_keys(&mut self) -> Vec<i64> {
        self.exec("playhead.set", json!({"seconds": 1.0}));
        self.frames(2);
        self.ok("ui.click", json!({"id": "effectControls.motion.scale.stopwatch"}));
        self.frames(2);
        let clip = self.app().session.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        for (s, v) in [(2.0, 150.0), (3.0, 120.0)] {
            self.exec("playhead.set", json!({"seconds": s}));
            self.exec("effects.setParam", json!({"clip": clip, "effect": "motion", "param": "scale", "value": v}));
        }
        self.frames(3);
        let keys = self.scale_keys();
        assert_eq!(keys.len(), 3, "{keys:?}");
        keys
    }

    fn key_id(t: i64) -> String {
        format!("effectControls.motion.scale.keyframe.{t}")
    }

    fn selected_times(&mut self) -> Vec<i64> {
        let mut v: Vec<i64> = self.app().ui.kf_selected.iter().map(|k| k.time).collect();
        v.sort_unstable();
        v
    }

    fn clip_count(&mut self) -> usize {
        self.app().session.active_sequence().unwrap().video_tracks[0].items.len()
    }
}

/// Keyframes in the lane: click selects one, Shift-click adds, a drag of a selected one moves them
/// all together (one undo step; this deadlocked once), Delete in Effect Controls deletes the
/// selected keyframes and never the clip, a box dragged in the lane selects what it covers, and
/// Copy / Paste puts them at the playhead.
#[test]
fn keyframes_select_move_delete_and_paste_together() {
    let dir = data_dir("kf-select");
    let (mut d, _) = Driver::with_clip(&dir);
    let k = d.three_scale_keys();
    d.ok("ui.click", json!({"id": Driver::key_id(k[0])}));
    d.frames(2);
    d.ok("ui.click", json!({"id": Driver::key_id(k[2]), "modifiers": {"shift": true}}));
    d.frames(2);
    assert_eq!(d.selected_times(), vec![k[0], k[2]]);
    // drag the first selected keyframe 30 px right: both selected ones move, the middle one stays
    let steps = d.app().session.history.undo.len();
    d.ok("ui.drag", json!({"from": {"id": Driver::key_id(k[0])}, "to": {"id": Driver::key_id(k[0]), "fx": 3.2}, "steps": 12}));
    d.frames(3);
    let moved = d.scale_keys();
    assert_eq!(moved.len(), 3, "{moved:?}");
    assert!(moved.contains(&k[1]), "the unselected keyframe stayed: {moved:?}");
    let by = moved[0] - k[0];
    assert!(by > 0, "moved right: {moved:?} from {k:?}");
    assert_eq!(moved[2] - k[2], by, "both by the same amount");
    assert_eq!(d.selected_times(), vec![moved[0], moved[2]], "the selection follows");
    assert_eq!(d.app().session.history.undo.len(), steps + 1);
    // Delete with Effect Controls focused: the two selected keyframes go, the clip stays
    let clips = d.clip_count();
    d.ok("ui.key", json!({"key": "Delete"}));
    d.frames(2);
    assert_eq!(d.scale_keys(), vec![k[1]]);
    assert_eq!(d.clip_count(), clips, "Delete in Effect Controls never deletes the clip");
    d.ok("ui.key", json!({"key": "Cmd+Z"}));
    d.frames(2);
    assert_eq!(d.scale_keys(), moved, "one undo brings both back");
    // nothing selected: Delete there does nothing to the clip either
    d.app().ui.kf_selected.clear();
    d.ok("ui.key", json!({"key": "Delete"}));
    d.frames(2);
    assert_eq!(d.clip_count(), clips);
    assert_eq!(d.scale_keys(), moved);
    // a box over the first two keyframes selects them
    let r0 = d.ok("ui.elements", json!({"prefix": Driver::key_id(moved[0])}))[0]["rect"].clone();
    let r1 = d.ok("ui.elements", json!({"prefix": Driver::key_id(moved[1])}))[0]["rect"].clone();
    let (x0, y0) = (r0[0].as_f64().unwrap() - 8.0, r0[1].as_f64().unwrap() - 6.0);
    let (x1, y1) = (r1[0].as_f64().unwrap() + r1[2].as_f64().unwrap() + 6.0, r1[1].as_f64().unwrap() + r1[3].as_f64().unwrap() + 6.0);
    d.ok("ui.drag", json!({"from": {"x": x0, "y": y0}, "to": {"x": x1, "y": y1}, "steps": 8}));
    d.frames(2);
    assert_eq!(d.selected_times(), vec![moved[0], moved[1]], "the box selected the keyframes in it");
    // Copy, playhead to 4 s, Paste: two keyframes there with the same spacing, selected
    d.ok("ui.key", json!({"key": "Cmd+C"}));
    d.exec("playhead.set", json!({"seconds": 4.0}));
    d.frames(2);
    d.ok("ui.key", json!({"key": "Cmd+V"}));
    d.frames(2);
    let after = d.scale_keys();
    assert_eq!(after.len(), 5, "{after:?}");
    let pasted = d.selected_times();
    assert_eq!(pasted.len(), 2);
    assert_eq!(pasted[1] - pasted[0], moved[1] - moved[0], "spacing kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Effects: an effect dragged by its name moves among the standard effects, Copy / Paste of the
/// selected effects adds copies, Delete removes the selected effect (Motion and Opacity stay).
#[test]
fn effects_reorder_copy_paste_and_delete() {
    let dir = data_dir("fx-order");
    let (mut d, clip) = Driver::with_clip(&dir);
    d.exec("effects.apply", json!({"clips": [clip], "effect": "Gaussian Blur"}));
    d.exec("effects.apply", json!({"clips": [clip], "effect": "Tint"}));
    let fixed: Vec<String> = d.app().session.active_sequence().unwrap().video_tracks[0].items[0]
        .effects
        .iter()
        .enumerate()
        .filter(|(_, e)| e.def().is_some_and(|d| d.intrinsic))
        .map(|(i, _)| format!("{clip}:{i}"))
        .collect();
    d.app().ui.collapsed_fx.extend(fixed);
    d.frames(3);
    let order = |d: &mut Driver| -> Vec<String> {
        d.app().session.active_sequence().unwrap().video_tracks[0].items[0]
            .effects
            .iter()
            .map(|e| e.effect.clone())
            .filter(|e| e == "gaussian_blur" || e == "tint")
            .collect()
    };
    assert_eq!(order(&mut d), ["gaussian_blur", "tint"]);
    let clips = d.clip_count();
    // Tint dragged above Gaussian Blur
    d.ok("ui.drag", json!({"from": {"id": "effectControls.effect.tint", "fx": 0.3}, "to": {"id": "effectControls.effect.gaussian_blur", "fx": 0.3, "fy": 0.1}, "steps": 16}));
    d.frames(3);
    assert_eq!(order(&mut d), ["tint", "gaussian_blur"]);
    // Copy Tint, Paste: a second Tint
    d.ok("ui.click", json!({"id": "effectControls.effect.tint", "fx": 0.3}));
    d.frames(2);
    d.ok("ui.key", json!({"key": "Cmd+C"}));
    d.ok("ui.key", json!({"key": "Cmd+V"}));
    d.frames(2);
    assert_eq!(order(&mut d), ["tint", "gaussian_blur", "tint"]);
    // Delete the selected Gaussian Blur; the clip and its fixed effects stay
    d.ok("ui.click", json!({"id": "effectControls.effect.gaussian_blur", "fx": 0.3}));
    d.frames(2);
    d.ok("ui.key", json!({"key": "Delete"}));
    d.frames(2);
    assert_eq!(order(&mut d), ["tint", "tint"]);
    assert_eq!(d.clip_count(), clips, "the clip stays");
    // Motion selected: Delete refuses (it is fixed)
    d.ok("ui.click", json!({"id": "effectControls.effect.motion", "fx": 0.3}));
    d.frames(2);
    let n = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].effects.len();
    d.ok("ui.key", json!({"key": "Delete"}));
    d.frames(2);
    assert_eq!(d.app().session.active_sequence().unwrap().video_tracks[0].items[0].effects.len(), n);
    let _ = std::fs::remove_dir_all(&dir);
}

impl Driver {
    fn center(&mut self, id: &str) -> egui::Pos2 {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == json!(id)).unwrap_or_else(|| panic!("no element {id}")).clone();
        let r = &e["rect"];
        let f = |i: usize| r[i].as_f64().unwrap() as f32;
        egui::pos2(f(0) + f(2) / 2.0, f(1) + f(3) / 2.0)
    }
    fn press(&mut self, p: egui::Pos2) {
        self.harness.input_mut().events.push(egui::Event::PointerMoved(p));
        self.frames(1);
        self.harness.input_mut().events.push(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        });
        self.frames(1);
    }
    /// Move the held pointer from `from` by `by` in `steps`.
    fn move_by(&mut self, from: egui::Pos2, by: egui::Vec2, steps: usize) -> egui::Pos2 {
        let mut p = from;
        for _ in 0..steps {
            p += by / steps as f32;
            self.harness.input_mut().events.push(egui::Event::PointerMoved(p));
            self.frames(2);
        }
        p
    }
    fn release(&mut self, p: egui::Pos2) {
        self.harness.input_mut().events.push(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        });
        self.frames(3);
    }
    fn undo_len(&mut self) -> usize {
        self.app().session.history.undo.len()
    }
    fn motion(&mut self, param: &str) -> filmcraft_project::ParamValue {
        let ph = self.app().session.playhead();
        let it = self.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone();
        it.effect("motion").unwrap().param(param).unwrap().value_at(it.source_time_at(ph))
    }
}

/// The value graph: a keyframe dragged up raises its value while the drag goes (the picture
/// follows), sideways moves it in time but never past its neighbours, and each drag is one undo
/// step. Before, the value changed only on release.
#[test]
fn graph_keyframes_drag_live_in_time_and_value() {
    let dir = data_dir("graph-live");
    let (mut d, _) = Driver::with_clip(&dir);
    let k = d.three_scale_keys();
    d.ok("ui.click", json!({"id": "effectControls.motion.scale.graphs"}));
    d.frames(3);
    let id = format!("effectControls.scale.graph.keyframe.{}", k[1]);
    let undo = d.undo_len();
    let p = d.center(&id);
    d.press(p);
    let p = d.move_by(p, egui::vec2(0.0, -40.0), 8);
    let it = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone();
    let live = it.effect("motion").unwrap().params["scale"].keyframes[1].value.as_f64().unwrap();
    assert!(live > 150.0, "raised while dragging: {live}");
    assert_eq!(d.scale_keys(), k, "straight up: the same time");
    d.release(p);
    assert_eq!(d.undo_len(), undo + 1, "one undo step");
    // sideways, far past the third keyframe: it stops a frame short of it
    let p = d.center(&id);
    d.press(p);
    let p = d.move_by(p, egui::vec2(600.0, 0.0), 10);
    d.release(p);
    let after = d.scale_keys();
    assert_eq!(after.len(), 3, "none replaced: {after:?}");
    assert!(after[1] > k[1] && after[1] < k[2], "moved right, short of its neighbour: {after:?} from {k:?}");
    assert_eq!(d.undo_len(), undo + 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The lane: a keyframe dragged moves live and stops short of the next one; the zoom bar's end
/// zooms in (keyframes spread out), a double-click on it shows the whole clip again, Alt + wheel
/// zooms about the pointer, and the divider between the properties and the lane drags.
#[test]
fn lane_drags_live_and_zooms() {
    let dir = data_dir("lane-zoom");
    let (mut d, _) = Driver::with_clip(&dir);
    let k = d.three_scale_keys();
    let p = d.center(&Driver::key_id(k[0]));
    d.ok("ui.click", json!({"id": Driver::key_id(k[0])}));
    d.frames(2);
    d.press(p);
    let q = d.move_by(p, egui::vec2(400.0, 0.0), 10);
    let mid = d.scale_keys();
    assert!(mid[0] > k[0], "moving while dragged: {mid:?}");
    d.release(q);
    let after = d.scale_keys();
    assert_eq!(after.len(), 3, "{after:?}");
    assert!(after[0] < after[1] && after[1] == k[1], "stopped short of the second keyframe: {after:?}");
    // Alt + wheel between two keyframes zooms in about the pointer: they spread out
    let gap = |d: &mut Driver, a: i64, b: i64| d.center(&Driver::key_id(b)).x - d.center(&Driver::key_id(a)).x;
    let before = gap(&mut d, after[1], after[2]);
    let (a, b) = (d.center(&Driver::key_id(after[1])), d.center(&Driver::key_id(after[2])));
    let lane = d.center("effectControls.laneBody");
    d.ok("ui.scroll", json!({"x": (a.x + b.x) / 2.0, "y": lane.y, "dy": 200.0, "modifiers": {"alt": true}}));
    d.frames(3);
    assert!(d.app().ui.ec_view.is_some(), "Alt + wheel zooms");
    let zoomed = gap(&mut d, after[1], after[2]);
    assert!(zoomed > before * 1.5, "spread out: {before} → {zoomed}");
    d.ok("ui.click", json!({"id": "effectControls.zoomBar", "count": 2}));
    d.frames(2);
    assert!(d.app().ui.ec_view.is_none(), "a double-click on the zoom bar shows the whole clip");
    // the bar's end zooms too
    let end = d.center("effectControls.zoomBar.end");
    let bar = d.center("effectControls.zoomBar.start");
    d.ok("ui.drag", json!({"from": {"x": end.x, "y": end.y}, "to": {"x": bar.x + (end.x - bar.x) * 0.5, "y": end.y}, "steps": 10}));
    d.frames(3);
    assert!(d.app().ui.ec_view.is_some(), "zoomed with the bar");
    // the divider
    let split = d.app().ui.ec_split;
    let dv = d.center("effectControls.divider");
    d.ok("ui.drag", json!({"from": {"x": dv.x, "y": dv.y}, "to": {"x": dv.x - 80.0, "y": dv.y}, "steps": 6}));
    d.frames(2);
    assert!(d.app().ui.ec_split < split, "the divider moved left");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Program monitor: the selected clip shows its box; dragging inside it moves the clip as the
/// pointer goes, a corner handle scales it, just outside a corner rotates it, the anchor point's
/// crosshair moves the anchor (the picture stays put); each drag is one undo step.
#[test]
fn program_monitor_moves_scales_and_rotates_the_selected_clip() {
    let dir = data_dir("motion-overlay");
    let (mut d, _) = Driver::with_clip(&dir);
    d.exec("playhead.set", json!({"seconds": 1.0}));
    d.frames(4);
    let pos = |v: filmcraft_project::ParamValue| v.as_vec2().unwrap();
    let p0 = pos(d.motion("position"));
    let undo = d.undo_len();
    let c = d.center("program.motion.box");
    d.press(c);
    let c2 = d.move_by(c, egui::vec2(40.0, 20.0), 8);
    let live = pos(d.motion("position"));
    assert!(live.x > p0.x + 10.0 && live.y > p0.y + 5.0, "moving while dragged: {p0:?} → {live:?}");
    d.release(c2);
    assert_eq!(d.undo_len(), undo + 1, "one undo step");
    // a corner dragged in: smaller
    let h = d.center("program.motion.handle.2");
    d.press(h);
    let h2 = d.move_by(h, egui::vec2(-60.0, -40.0), 6);
    d.release(h2);
    let sc = d.motion("scale").as_f64().unwrap();
    assert!(sc < 95.0 && sc > 20.0, "scaled down: {sc}");
    // just outside a corner: rotated
    let r = d.center("program.motion.rotate.1");
    d.press(r);
    let r2 = d.move_by(r, egui::vec2(30.0, 60.0), 6);
    d.release(r2);
    assert!(d.motion("rotation").as_f64().unwrap().abs() > 5.0, "rotated");
    // the anchor point: anchor and position move together
    let (a0, q0) = (pos(d.motion("anchor")), pos(d.motion("position")));
    let a = d.center("program.motion.anchor");
    d.press(a);
    let a2 = d.move_by(a, egui::vec2(-30.0, 0.0), 5);
    d.release(a2);
    let (a1, q1) = (pos(d.motion("anchor")), pos(d.motion("position")));
    assert!((a1 - a0).length() > 5.0 && (q1 - q0).length() > 5.0, "anchor {a0:?}→{a1:?}, position {q0:?}→{q1:?}");
    assert_eq!(d.undo_len(), undo + 4);
    let _ = std::fs::remove_dir_all(&dir);
}
