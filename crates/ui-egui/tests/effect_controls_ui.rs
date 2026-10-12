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
