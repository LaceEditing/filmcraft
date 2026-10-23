//! Headless UI tests of the Clip / Track Fx Editor windows (Parametric Equalizer, Graphic
//! Equalizer, Multiband Compressor, Dynamics): opened from Effect Controls ("Custom Setup ▸
//! Edit…") and from a mixer effect slot, driven by automation id and by pointer drags.
//!
//! Set `FILMCRAFT_UI_SNAPSHOT_DIR=<dir>` to also write PNGs of the windows (`fx-editor-*.png`).

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    snapshots: Option<std::path::PathBuf>,
}

impl Driver {
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let snapshots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000);
        if snapshots.is_some() {
            b = b.wgpu();
        }
        let harness = b.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, snapshots };
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

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(2);
    }

    fn element(&mut self, id: &str) -> Option<[f32; 4]> {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array()?.iter().find(|e| e["id"] == id).map(|e| {
            let r = &e["rect"];
            [r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32, r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32]
        })
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        let v = self.ok("ui.elements", json!({"prefix": prefix}));
        v.as_array().unwrap().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }

    /// Press on `id`, move by (dx, dy) over several frames, release.
    fn drag(&mut self, id: &str, dx: f32, dy: f32, steps: usize) {
        let r = self.element(id).unwrap_or_else(|| panic!("no element {id}"));
        let mut p = egui::pos2(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let push = |d: &mut Self, e: egui::Event| d.harness.input_mut().events.push(e);
        push(self, egui::Event::PointerMoved(p));
        self.frames(1);
        push(self, egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        self.frames(1);
        for _ in 0..steps {
            p.x += dx / steps as f32;
            p.y += dy / steps as f32;
            push(self, egui::Event::PointerMoved(p));
            self.frames(1);
        }
        push(self, egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        self.frames(3);
    }

    fn snapshot(&mut self, name: &str) {
        let Some(dir) = self.snapshots.clone() else { return };
        self.frames(2);
        match self.harness.render() {
            Ok(img) => {
                std::fs::create_dir_all(&dir).unwrap();
                img.save(dir.join(format!("{name}.png"))).unwrap();
            }
            Err(e) => eprintln!("snapshot {name} skipped: {e}"),
        }
    }

    fn undo_len(&mut self) -> usize {
        self.exec("history.list", json!({}))["undo"].as_array().unwrap().len()
    }

    /// Audio clip on A1 (id) with `effect` applied and selected; Effect Controls showing it.
    fn audio_clip_with(&mut self, effect: &str) -> u64 {
        self.ok("ui.set", json!({"workspace": "Effects"}));
        let seq = self.exec("sequence.inspect", json!({}));
        let clip = &seq["audio"][0]["items"][0];
        let (id, start) = (clip["clip"].as_u64().unwrap(), clip["start"].as_i64().unwrap());
        self.exec("playhead.set", json!({"time": start}));
        // only the audio clip (Effect Controls prefers a linked video clip)
        self.exec("sequence.linkedSelection", json!({"on": false}));
        self.exec("timeline.select", json!({"clips": [id]}));
        self.exec("effects.apply", json!({"clips": [id], "effect": effect}));
        self.frames(3);
        // fold the fixed effects so the applied one's rows are on screen
        for fx in ["volume", "channel_volume", "panner"] {
            self.click(&format!("effectControls.effect.{fx}.twirl"));
        }
        id
    }

    fn clip_param(&mut self, clip: u64, effect: &str, param: &str) -> Value {
        let seq = self.exec("sequence.inspect", json!({}));
        let items = seq["audio"][0]["items"].as_array().unwrap().clone();
        let it = items.iter().find(|i| i["clip"] == clip).unwrap().clone();
        let fx = it["effects"].as_array().unwrap().iter().find(|e| e["effect"] == effect).unwrap().clone();
        fx["params"][param]["value"].clone()
    }
}

/// A parameter value from `sequence.inspect` (`"Float(3.5)"`, `"Bool(true)"`) or
/// `mixer.inspect` (plain JSON).
fn num(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim_start_matches("Float(").trim_start_matches("Choice(").trim_end_matches(')').parse().ok()))
        .unwrap_or(f64::NAN)
}

fn truthy(v: &Value) -> bool {
    v.as_bool().unwrap_or(false) || v.as_str() == Some("Bool(true)")
}

#[test]
fn parametric_eq_editor_from_effect_controls() {
    let mut d = Driver::demo();
    let clip = d.audio_clip_with("parametric_eq");
    d.click("effectControls.effect.parametric_eq.edit");
    d.frames(3);
    let ids = d.ids("fxEditor.parametric_eq.");
    for id in [
        "plot",
        "node.low",
        "node.b1",
        "node.mid",
        "node.high",
        "toggle.hp_on",
        "toggle.low_steep",
        "value.mid_q",
        "choice.hp_slope",
        "range.30",
        "param.master_gain",
        "close",
    ] {
        assert!(ids.iter().any(|i| *i == format!("fxEditor.parametric_eq.{id}")), "{id} missing from {ids:?}");
    }
    // the pass filters are off, so they have no point (as in Premiere); switched on, they do
    assert!(!ids.iter().any(|i| i == "fxEditor.parametric_eq.node.hp"));
    d.click("fxEditor.parametric_eq.toggle.hp_on");
    assert!(truthy(&d.clip_param(clip, "parametric_eq", "hp_on")));
    assert!(d.element("fxEditor.parametric_eq.node.hp").is_some(), "HP point shown once on");
    // dragging band 3 up boosts it — one undo step for the whole drag (frequency and gain are
    // committed on release, in the same gesture)
    let undo0 = d.undo_len();
    let g0 = num(&d.clip_param(clip, "parametric_eq", "mid_gain"));
    d.drag("fxEditor.parametric_eq.node.mid", 0.0, -60.0, 6);
    let g = num(&d.clip_param(clip, "parametric_eq", "mid_gain"));
    assert!(g > 3.0, "mid gain after drag: {g}");
    assert_eq!(d.undo_len(), undo0 + 1, "the drag is one undo step");
    d.exec("edit.undo", json!({}));
    assert_eq!(num(&d.clip_param(clip, "parametric_eq", "mid_gain")), g0, "one undo takes the whole drag back");
    d.exec("edit.redo", json!({}));
    d.snapshot("fx-editor-parametric-eq");
    d.click("fxEditor.parametric_eq.close");
    assert!(d.ids("fxEditor.parametric_eq.").is_empty(), "window closed");
}

/// The graph behaves like Premiere's: a point grabbed off-centre stays under the cursor (no
/// jump), every band moves (bands 1, 2, 4, 5 too), points stop at the graph's edges (±48 dB at
/// Range 96, ±15 at Range 30), HP / LP move only sideways, a band switched off loses its point,
/// and the shelves' slope switches.
#[test]
fn parametric_eq_graph_follows_the_pointer() {
    let mut d = Driver::demo();
    let clip = d.audio_clip_with("parametric_eq");
    d.click("effectControls.effect.parametric_eq.edit");
    d.frames(3);
    let centre = |r: [f32; 4]| egui::pos2(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    // grabbed 4 px right of and 3 px below its centre, band 1 moves exactly as far as the pointer
    let before = centre(d.element("fxEditor.parametric_eq.node.b1").unwrap());
    let mut p = before + egui::vec2(4.0, 3.0);
    d.harness.input_mut().events.push(egui::Event::PointerMoved(p));
    d.frames(1);
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    d.frames(1);
    for _ in 0..5 {
        p += egui::vec2(10.0, -8.0);
        d.harness.input_mut().events.push(egui::Event::PointerMoved(p));
        // the move's frame and the repaint the editor asks for while dragging
        d.frames(2);
    }
    let during = centre(d.element("fxEditor.parametric_eq.node.b1").unwrap());
    assert!((during - (before + egui::vec2(50.0, -40.0))).length() < 1.0, "point follows the pointer live: {before:?} → {during:?}");
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    d.frames(3);
    let after = centre(d.element("fxEditor.parametric_eq.node.b1").unwrap());
    assert!((after - during).length() < 1.0, "no jump on release: {during:?} → {after:?}");
    assert!(num(&d.clip_param(clip, "parametric_eq", "b1_gain")) > 3.0, "band 1 is boosted");
    assert!(num(&d.clip_param(clip, "parametric_eq", "b1_freq")) > 50.0, "and moved up in frequency");
    // past the top-right corner: held at 24 kHz / +48 dB
    d.drag("fxEditor.parametric_eq.node.b5", 900.0, -900.0, 8);
    assert_eq!(num(&d.clip_param(clip, "parametric_eq", "b5_gain")), 48.0);
    assert_eq!(num(&d.clip_param(clip, "parametric_eq", "b5_freq")), 24000.0);
    // Range 30 dB: ±15
    d.click("fxEditor.parametric_eq.range.30");
    d.drag("fxEditor.parametric_eq.node.b4", 0.0, 900.0, 8);
    assert_eq!(num(&d.clip_param(clip, "parametric_eq", "b4_gain")), -15.0);
    // a pass filter moves sideways only
    d.click("fxEditor.parametric_eq.toggle.lp_on");
    d.drag("fxEditor.parametric_eq.node.lp", -120.0, 80.0, 6);
    let lp = num(&d.clip_param(clip, "parametric_eq", "lp_freq"));
    assert!(lp < 15000.0, "LP moved down: {lp}");
    let r = d.element("fxEditor.parametric_eq.node.lp").unwrap();
    let zero = d.element("fxEditor.parametric_eq.node.mid").unwrap();
    assert!((centre(r).y - centre(zero).y).abs() < 1.0, "LP stays on the 0 dB line");
    // band 2 off: no point
    d.click("fxEditor.parametric_eq.toggle.b2_on");
    assert!(!truthy(&d.clip_param(clip, "parametric_eq", "b2_on")));
    assert!(d.element("fxEditor.parametric_eq.node.b2").is_none(), "an off band has no point");
    // the low shelf's slope switch
    d.click("fxEditor.parametric_eq.toggle.low_steep");
    assert!(truthy(&d.clip_param(clip, "parametric_eq", "low_steep")));
    d.snapshot("fx-editor-parametric-eq-edited");
}

/// A window belongs to the effect it was opened on: when its slot comes to hold another effect
/// (the EQ removed and a Bass put in its place, or another project opened), it closes instead
/// of showing the other effect.
#[test]
fn fx_editor_closes_when_its_effect_is_replaced() {
    let mut d = Driver::demo();
    let clip = d.audio_clip_with("parametric_eq");
    d.click("effectControls.effect.parametric_eq.edit");
    d.frames(3);
    assert!(!d.ids("fxEditor.parametric_eq.").is_empty());
    let seq = d.exec("sequence.inspect", json!({}));
    let items = seq["audio"][0]["items"].as_array().unwrap().clone();
    let it = items.iter().find(|i| i["clip"] == clip).unwrap();
    let index = it["effects"].as_array().unwrap().iter().position(|e| e["effect"] == "parametric_eq").unwrap();
    d.exec("effects.remove", json!({"clip": clip, "index": index}));
    d.exec("effects.apply", json!({"clips": [clip], "effect": "bass"}));
    d.frames(3);
    assert!(d.ids("fxEditor.").iter().all(|i| !i.starts_with("fxEditor.bass.")), "no Bass editor took the EQ's window");
    assert!(d.ids("fxEditor.parametric_eq.").is_empty(), "the EQ's window closed");
}

#[test]
fn graphic_eq_and_dynamics_editors() {
    let mut d = Driver::demo();
    let clip = d.audio_clip_with("graphic_eq_30");
    d.click("effectControls.effect.graphic_eq_30.edit");
    d.frames(3);
    let ids = d.ids("fxEditor.graphic_eq_30.param.");
    assert_eq!(ids.iter().filter(|i| i.starts_with("fxEditor.graphic_eq_30.param.b")).count(), 30, "{ids:?}");
    d.drag("fxEditor.graphic_eq_30.param.b17", 0.0, -40.0, 5);
    let g = num(&d.clip_param(clip, "graphic_eq_30", "b17"));
    assert!(g > 3.0, "1 kHz band after drag: {g}");
    d.click("fxEditor.graphic_eq_30.reset");
    assert_eq!(num(&d.clip_param(clip, "graphic_eq_30", "b17")), 0.0);
    d.snapshot("fx-editor-graphic-eq-30");
    d.click("fxEditor.graphic_eq_30.close");
    // Dynamics: the transfer curve and section toggles
    d.exec("effects.apply", json!({"clips": [clip], "effect": "dynamics_rack"}));
    d.frames(3);
    d.click("effectControls.effect.graphic_eq_30.twirl");
    d.click("effectControls.effect.dynamics_rack.edit");
    d.frames(3);
    // Premiere's four sections: switches, knobs and meters
    let ids = d.ids("fxEditor.dynamics_rack.");
    for id in [
        "toggle.gate_on",
        "toggle.comp_on",
        "toggle.exp_on",
        "toggle.lim_on",
        "param.gate_threshold",
        "param.gate_hold",
        "param.comp_ratio",
        "param.comp_makeup",
        "param.exp_ratio",
        "param.lim_release",
        "value.comp_threshold",
        "meter.gate",
        "meter.comp",
        "meter.exp",
        "meter.lim",
        "bypass",
        "presets",
        "close",
    ] {
        assert!(ids.iter().any(|i| *i == format!("fxEditor.dynamics_rack.{id}")), "{id} missing from {ids:?}");
    }
    d.click("fxEditor.dynamics_rack.toggle.lim_on");
    assert!(truthy(&d.clip_param(clip, "dynamics_rack", "lim_on")));
    // a knob turned up and to the right raises the compressor threshold, in one undo step
    let undo0 = d.undo_len();
    let t0 = num(&d.clip_param(clip, "dynamics_rack", "comp_threshold"));
    d.drag("fxEditor.dynamics_rack.param.comp_threshold", 20.0, -40.0, 6);
    let t1 = num(&d.clip_param(clip, "dynamics_rack", "comp_threshold"));
    assert!(t1 > t0 + 5.0, "threshold {t0} → {t1}");
    assert_eq!(d.undo_len(), undo0 + 1);
    // double-click: back to the default
    d.ok("ui.click", json!({"id": "fxEditor.dynamics_rack.param.comp_threshold", "count": 2}));
    d.frames(3);
    assert_eq!(num(&d.clip_param(clip, "dynamics_rack", "comp_threshold")), -20.0);
    d.snapshot("fx-editor-dynamics");
}

#[test]
fn track_fx_editor_from_the_mixer_slot() {
    let mut d = Driver::demo();
    d.exec("mixer.addInsert", json!({"strip": "A1", "effect": "multiband_compressor"}));
    d.ok("ui.set", json!({"workspace": "Audio"}));
    d.frames(3);
    d.click("mixer.showEffects");
    // the slot's right-click menu ▸ Edit…
    d.ok("ui.click", json!({"id": "mixer.A1.fx.0", "button": "right"}));
    d.frames(2);
    d.click("mixer.A1.fx.0.edit");
    d.frames(3);
    let ids = d.ids("fxEditor.multiband_compressor.");
    for id in ["xo1", "xo2", "xo3", "curve.1", "curve.4", "toggle.b2_solo", "param.b3_threshold", "toggle.lim_on"] {
        assert!(ids.iter().any(|i| *i == format!("fxEditor.multiband_compressor.{id}")), "{id} missing from {ids:?}");
    }
    d.click("fxEditor.multiband_compressor.toggle.b2_solo");
    let m = d.exec("mixer.inspect", json!({}));
    let a1 = m["strips"].as_array().unwrap().iter().find(|s| s["ref"] == "A1").unwrap().clone();
    assert!(truthy(&a1["inserts"][0]["params"]["b2_solo"]), "solo set on the insert: {a1}");
    // dragging the mid crossover to the right raises it
    d.drag("fxEditor.multiband_compressor.xo2", 60.0, 0.0, 5);
    let m = d.exec("mixer.inspect", json!({}));
    let a1 = m["strips"].as_array().unwrap().iter().find(|s| s["ref"] == "A1").unwrap().clone();
    let xo2 = num(&a1["inserts"][0]["params"]["xo2"]);
    assert!(xo2 > 2200.0, "crossover after drag: {xo2} ({a1})");
    d.snapshot("fx-editor-multiband");
}

impl Driver {
    /// The effect ids in a strip's rack (`mixer.inspect`).
    fn rack(&mut self, strip: &str) -> Vec<String> {
        let m = self.exec("mixer.inspect", json!({}));
        let s = m["strips"].as_array().unwrap().iter().find(|s| s["ref"] == strip).unwrap().clone();
        s["inserts"].as_array().unwrap().iter().map(|i| i["effect"].as_str().unwrap().to_string()).collect()
    }

    /// An insert of a strip as `mixer.inspect` shows it.
    fn insert(&mut self, strip: &str, slot: usize) -> Value {
        let m = self.exec("mixer.inspect", json!({}));
        let s = m["strips"].as_array().unwrap().iter().find(|s| s["ref"] == strip).unwrap().clone();
        s["inserts"][slot].clone()
    }

    /// The Audio workspace with the mixer's effect racks open.
    fn mixer_racks(&mut self) {
        self.ok("ui.set", json!({"workspace": "Audio"}));
        self.frames(3);
        self.click("mixer.showEffects");
        self.frames(2);
    }

    /// Drag from one element to another with the control channel (modifiers held throughout).
    fn drag_to(&mut self, from: &str, to: &str, modifiers: Value) {
        self.ok("ui.drag", json!({"from": {"id": from}, "to": {"id": to}, "modifiers": modifiers}));
        self.frames(4);
    }
}

/// Premiere's Audio Track Mixer racks (measured in Premiere 26.5): an effect dragged to another
/// slot of its track moves there, onto another track it is copied with its settings; Ctrl swaps
/// the two (copy within the track, move to another); Alt and Shift change nothing. A drop on a
/// filled slot inserts there. Before, every drag moved and only Alt copied, which KDE takes for
/// moving the window.
#[test]
fn mixer_effects_drag_between_slots_and_tracks() {
    let mut d = Driver::demo();
    for fx in ["amplify", "bass", "treble"] {
        d.exec("mixer.addInsert", json!({"strip": "A1", "effect": fx}));
    }
    d.exec("mixer.setInsert", json!({"strip": "A1", "slot": 1, "params": {"boost": 7.5}}));
    d.mixer_racks();
    // reorder: Amplify onto the third slot
    d.drag_to("mixer.A1.fx.0", "mixer.A1.fx.2", json!({}));
    assert_eq!(d.rack("A1"), ["bass", "treble", "amplify"]);
    // onto another track: a copy, with its settings; the original stays
    d.drag_to("mixer.A1.fx.0", "mixer.A2.fx.0", json!({}));
    assert_eq!(d.rack("A1"), ["bass", "treble", "amplify"]);
    assert_eq!(d.rack("A2"), ["bass"]);
    assert_eq!(num(&d.insert("A2", 0)["params"]["boost"]), 7.5, "the copy keeps the setting");
    // Alt and Shift change nothing
    d.drag_to("mixer.A1.fx.1", "mixer.A2.fx.1", json!({"alt": true}));
    assert_eq!(d.rack("A1"), ["bass", "treble", "amplify"]);
    assert_eq!(d.rack("A2"), ["bass", "treble"]);
    // Ctrl onto another track moves
    d.drag_to("mixer.A1.fx.2", "mixer.A2.fx.0", json!({"command": true}));
    assert_eq!(d.rack("A1"), ["bass", "treble"]);
    assert_eq!(d.rack("A2"), ["amplify", "bass", "treble"], "inserted at the slot it was dropped on");
    // Ctrl within the track copies
    d.drag_to("mixer.A1.fx.0", "mixer.A1.fx.2", json!({"command": true}));
    assert_eq!(d.rack("A1"), ["bass", "treble", "bass"]);
    // dropping an effect on a send slot or outside the rack does nothing
    d.drag_to("mixer.A1.fx.0", "mixer.A1.send.0", json!({}));
    d.drag_to("mixer.A1.fx.0", "mixer.A1.fader", json!({}));
    assert_eq!(d.rack("A1"), ["bass", "treble", "bass"]);
    // each drag is one undo step
    d.exec("edit.undo", json!({}));
    assert_eq!(d.rack("A1"), ["bass", "treble"]);
}

/// The slot menus and the knob under the rack: the arrow replaces or empties a filled slot, the
/// right-click menu bypasses, chooses the knob's parameter and copies / pastes the rack, the knob
/// turns the parameter (one undo step), its fx button bypasses, a double-click edits.
#[test]
fn mixer_rack_menus_and_knob() {
    let mut d = Driver::demo();
    d.exec("mixer.addInsert", json!({"strip": "A1", "effect": "hard_limiter"}));
    d.exec("mixer.addInsert", json!({"strip": "A1", "effect": "bass"}));
    d.mixer_racks();
    // clicking the slot selects it: the knob shows its first parameter
    d.click("mixer.A1.fx.0");
    let knob = d.ids("mixer.A1.rack.");
    assert!(knob.iter().any(|i| i == "mixer.A1.rack.knob"), "{knob:?}");
    let v0 = d.insert("A1", 0)["params"]["max"].as_f64().unwrap();
    let undo0 = d.undo_len();
    d.drag("mixer.A1.rack.knob", -30.0, 30.0, 6);
    let v1 = d.insert("A1", 0)["params"]["max"].as_f64().unwrap();
    assert!(v1 < v0 - 5.0, "Maximum Amplitude turned down: {v0} → {v1}");
    assert_eq!(d.undo_len(), undo0 + 1, "one undo step for the turn");
    // the knob's parameter, from the slot's right-click menu
    d.ok("ui.click", json!({"id": "mixer.A1.fx.0", "button": "right"}));
    d.frames(2);
    d.click("mixer.A1.fx.0.knob.release");
    let label = d.ok("ui.elements", json!({"prefix": "mixer.A1.rack.knob"}))[0]["label"].clone();
    assert_eq!(label, "Release Time");
    // the fx button bypasses the selected effect
    d.click("mixer.A1.rack.bypass");
    assert_eq!(d.insert("A1", 0)["enabled"], false);
    d.click("mixer.A1.rack.bypass");
    assert_eq!(d.insert("A1", 0)["enabled"], true);
    // the arrow of a filled slot: another effect replaces it
    d.ok("ui.click", json!({"id": "mixer.A1.fx.1", "fx": 0.96}));
    d.frames(2);
    d.click("mixer.A1.fx.1.folder.Filter and EQ");
    d.click("mixer.A1.fx.1.treble");
    assert_eq!(d.rack("A1"), ["hard_limiter", "treble"]);
    // … or None empties it
    d.ok("ui.click", json!({"id": "mixer.A1.fx.1", "fx": 0.96}));
    d.frames(2);
    d.click("mixer.A1.fx.1.none");
    assert_eq!(d.rack("A1"), ["hard_limiter"]);
    // Copy Track Effects on A1, Paste Track Effects on A2
    d.ok("ui.click", json!({"id": "mixer.A1.fx.0", "button": "right"}));
    d.frames(2);
    d.click("mixer.A1.fx.copy");
    d.ok("ui.click", json!({"id": "mixer.A2.fx.0", "button": "right"}));
    d.frames(2);
    d.click("mixer.A2.fx.paste");
    assert_eq!(d.rack("A2"), ["hard_limiter"]);
    // double-click: the effect's editor
    d.ok("ui.click", json!({"id": "mixer.A2.fx.0", "count": 2}));
    d.frames(3);
    assert!(d.element("fxEditor.hard_limiter.close").is_some(), "the Hard Limiter editor opened");
}

/// The Hard Limiter editor as in Premiere: Peak / True Peak, scaled sliders, Link Channels; and
/// the frame every editor has: bypass, Presets (save, choose, delete), close.
#[test]
fn hard_limiter_editor_presets_and_bypass() {
    let mut d = Driver::demo();
    d.exec("mixer.addInsert", json!({"strip": "A1", "effect": "hard_limiter"}));
    d.mixer_racks();
    d.ok("ui.click", json!({"id": "mixer.A1.fx.0", "count": 2}));
    d.frames(3);
    let ids = d.ids("fxEditor.hard_limiter.");
    for id in [
        "bypass",
        "presets",
        "presets.save",
        "presets.delete",
        "choice.detect.0",
        "choice.detect.1",
        "param.max",
        "param.boost",
        "param.lookahead",
        "param.release",
        "value.max",
        "toggle.link",
        "meter.lim",
        "close",
    ] {
        assert!(ids.iter().any(|i| *i == format!("fxEditor.hard_limiter.{id}")), "{id} missing from {ids:?}");
    }
    // a slider dragged left lowers Maximum Amplitude
    d.drag("fxEditor.hard_limiter.param.max", -80.0, 0.0, 6);
    let max = d.insert("A1", 0)["params"]["max"].as_f64().unwrap();
    assert!(max < -10.0, "Maximum Amplitude {max}");
    d.click("fxEditor.hard_limiter.choice.detect.1");
    assert_eq!(d.insert("A1", 0)["params"]["detect"].as_f64(), Some(1.0), "True Peak");
    d.click("fxEditor.hard_limiter.toggle.link");
    assert_eq!(d.insert("A1", 0)["params"]["link"], false);
    // save these settings as a preset
    d.click("fxEditor.hard_limiter.presets.save");
    d.frames(2);
    d.ok("ui.type", json!({"text": "Broadcast"}));
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    let presets = d.exec("presets.list", json!({}))["presets"].as_array().unwrap().clone();
    assert!(presets.iter().any(|p| p["name"] == "Broadcast" && p["ids"] == json!(["hard_limiter"])), "{presets:?}");
    let shown = d.ok("ui.elements", json!({"prefix": "fxEditor.hard_limiter.presets"}));
    assert!(shown.as_array().unwrap().iter().any(|e| e["id"] == "fxEditor.hard_limiter.presets"), "{shown}");
    // change something, then choose the preset from the menu: the settings come back
    d.exec("mixer.setInsert", json!({"strip": "A1", "slot": 0, "params": {"max": -3.0}}));
    d.frames(2);
    d.click("fxEditor.hard_limiter.presets");
    d.click("fxEditor.hard_limiter.presets.Broadcast");
    assert_eq!(d.insert("A1", 0)["params"]["max"].as_f64(), Some(max));
    // (Default) resets it
    d.click("fxEditor.hard_limiter.presets");
    d.click("fxEditor.hard_limiter.presets.(Default)");
    assert_eq!(d.insert("A1", 0)["params"]["max"].as_f64(), Some(-0.1));
    assert_eq!(d.insert("A1", 0)["params"]["link"], true);
    // delete it (chosen first)
    d.click("fxEditor.hard_limiter.presets");
    d.click("fxEditor.hard_limiter.presets.Broadcast");
    d.click("fxEditor.hard_limiter.presets.delete");
    let presets = d.exec("presets.list", json!({}))["presets"].as_array().unwrap().clone();
    assert!(!presets.iter().any(|p| p["name"] == "Broadcast"));
    // the power button bypasses the effect
    d.click("fxEditor.hard_limiter.bypass");
    assert_eq!(d.insert("A1", 0)["enabled"], false);
    d.snapshot("fx-editor-hard-limiter");
    d.click("fxEditor.hard_limiter.close");
    assert!(d.ids("fxEditor.hard_limiter.").is_empty());
}

/// Every audio effect has an editor: a few numbers as scaled sliders, many as sections of knobs,
/// switches as checkboxes, choices as radio buttons.
#[test]
fn any_audio_effect_has_an_editor() {
    let mut d = Driver::demo();
    for fx in ["studio_reverb", "surround_reverb", "analog_delay"] {
        d.exec("mixer.addInsert", json!({"strip": "A1", "effect": fx}));
    }
    d.mixer_racks();
    // Studio Reverb: five sliders
    d.ok("ui.click", json!({"id": "mixer.A1.fx.0", "count": 2}));
    d.frames(3);
    let ids = d.ids("fxEditor.studio_reverb.param.");
    assert_eq!(ids.len(), 5, "{ids:?}");
    d.drag("fxEditor.studio_reverb.param.wet", 60.0, 0.0, 5);
    let wet = d.insert("A1", 0)["params"]["wet"].as_f64().unwrap();
    assert!(wet > 45.0, "Wet {wet}");
    d.snapshot("fx-editor-studio-reverb");
    d.click("fxEditor.studio_reverb.close");
    // Surround Reverb: eleven numbers, as knobs
    d.ok("ui.click", json!({"id": "mixer.A1.fx.1", "count": 2}));
    d.frames(3);
    let ids = d.ids("fxEditor.surround_reverb.param.");
    assert_eq!(ids.len(), 11, "{ids:?}");
    d.click("fxEditor.surround_reverb.close");
    // Analog Delay: its mode as radio buttons
    d.ok("ui.click", json!({"id": "mixer.A1.fx.2", "count": 2}));
    d.frames(3);
    d.click("fxEditor.analog_delay.choice.mode.2");
    assert_eq!(d.insert("A1", 2)["params"]["mode"].as_f64(), Some(2.0));
}
