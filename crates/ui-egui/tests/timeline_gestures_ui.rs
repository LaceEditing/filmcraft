//! Timeline mouse gestures and editing keys driven as a user does, through real pointer and key
//! events: select, move, insert-move, trim, ripple, roll, razor, slip, slide, rate stretch,
//! marquee, track select and scrubbing; and the editing keys acting on what they should.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::ClipId;
use filmcraft_time::Tick;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    /// Screen x of frame 0 and pixels per frame, once the sequence is fitted.
    x0: f64,
    ppf: f64,
}

impl Driver {
    fn new() -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(s).with_control(rx);
        app.hooks.open_path = Some(Box::new(|_, _| Ok(())));
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, x0: 0.0, ppf: 0.0 };
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

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn key(&mut self, key: &str) {
        self.ok("ui.key", json!({"key": key}));
        self.frames(3);
    }

    fn app(&self) -> &FilmcraftApp {
        self.harness.state()
    }

    fn f(&self, n: i64) -> Tick {
        self.app().session.sequence_rate().tick_of(n)
    }

    /// (start, end) of a clip in frames.
    fn span(&self, c: ClipId) -> (i64, i64) {
        let rate = self.app().session.sequence_rate();
        let (_, it) = self.app().session.active_sequence().and_then(|q| q.find_item(c)).unwrap_or_else(|| panic!("clip {c:?} is gone"));
        (rate.frame_at(it.start), rate.frame_at(it.end()))
    }

    fn v1(&self) -> Vec<ClipId> {
        self.app().session.active_sequence().map(|q| q.video_tracks[0].items.iter().map(|i| i.id).collect()).unwrap_or_default()
    }

    fn selected(&self) -> Vec<ClipId> {
        let mut v = self.app().session.state.selection.clone();
        v.sort();
        v
    }

    fn y(&mut self, c: ClipId) -> f64 {
        self.ok("ui.timeline.locate", json!({"clip": c.0}))["y"].as_f64().unwrap()
    }

    fn x(&self, frame: f64) -> f64 {
        self.x0 + frame * self.ppf
    }

    fn click(&mut self, x: f64, y: f64, mods: Value) {
        self.ok("ui.click", json!({"x": x, "y": y, "modifiers": mods}));
        self.frames(3);
    }

    fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: Value) {
        self.ok("ui.drag", json!({"from": {"x": from.0, "y": from.1}, "to": {"x": to.0, "y": to.1}, "modifiers": mods, "steps": 16}));
        self.frames(4);
    }

    fn undo(&mut self) {
        self.key("Cmd+Z");
    }
}

/// Three adjacent 48-frame clips A, B, C on V1 (sounds on A1), an empty V2 above, fitted in the
/// Timeline with snapping off: their V1 ids.
fn three_clips(d: &mut Driver) -> [ClipId; 3] {
    d.exec("file.newSequence", json!({"name": "Gestures", "video": 2, "audio": 1}));
    let f48 = d.f(48).0;
    let item = d.app().session.project.items.values().find(|i| i.has_video() && i.has_audio() && i.as_media().is_some()).map(|i| i.id).unwrap();
    for frame in [0, 48, 96] {
        d.exec("timeline.place", json!({"item": item.0, "frame": frame, "sourceIn": f48, "duration": f48}));
    }
    d.exec("sequence.snap", json!({"on": false}));
    d.ok("ui.set", json!({"focused": "Timeline"}));
    d.key("\\");
    d.frames(20);
    let v = d.v1();
    let [a, b, c] = [v[0], v[1], v[2]];
    measure(d, a, c);
    assert!(d.ppf > 4.0, "the clips are wide enough to grab: {} px per frame", d.ppf);
    [a, b, c]
}

/// Frame 0's x and the pixels per frame, from where A starts and C ends on screen.
fn measure(d: &mut Driver, a: ClipId, c: ClipId) {
    d.frames(20);
    let x_in = d.ok("ui.timeline.locate", json!({"clip": a.0, "edge": "in"}))["x"].as_f64().unwrap() - 2.0;
    let x_out = d.ok("ui.timeline.locate", json!({"clip": c.0, "edge": "out"}))["x"].as_f64().unwrap() + 2.0;
    d.x0 = x_in;
    d.ppf = (x_out - x_in) / 144.0;
}

#[test]
fn selecting_with_the_mouse() {
    let mut d = Driver::new();
    let [a, b, c] = three_clips(&mut d);
    let y = d.y(b);
    let (sound_b, sound_c) = {
        let q = d.app().session.active_sequence().unwrap();
        (q.audio_tracks[0].items[1].id, q.audio_tracks[0].items[2].id)
    };
    // click selects the clip and its linked sound
    d.click(d.x(72.0), y, json!({}));
    assert_eq!(d.selected(), vec![b, sound_b]);
    // Shift-click adds
    d.click(d.x(120.0), y, json!({"shift": true}));
    let mut both = vec![b, c, sound_b, sound_c];
    both.sort();
    assert_eq!(d.selected(), both);
    // Alt-click selects one side of a link
    d.click(d.x(24.0), y, json!({"alt": true}));
    assert_eq!(d.selected(), vec![a]);
    // clicking empty track space deselects
    d.click(d.x(24.0), y - 50.0, json!({}));
    assert!(d.selected().is_empty(), "empty V2 deselects");
    d.click(d.x(72.0), y, json!({}));
    d.key("-");
    measure(&mut d, a, c);
    d.click(d.x(160.0), y, json!({}));
    assert!(d.selected().is_empty(), "empty space after the clips deselects");

    // a marquee from empty V2 over A and B selects them (and their sounds, linked selection)
    let v2 = d.app().session.active_sequence().unwrap().video_tracks[1].id;
    let row = d.ok("ui.timeline.hit", json!({"x": d.x(30.0), "y": y - 50.0}));
    assert_eq!(row["track"].as_u64(), Some(v2.0), "y - 50 is on V2: {row}");
    d.drag((d.x(30.0), y - 50.0), (d.x(70.0), y), json!({}));
    let sel = d.selected();
    assert!(sel.contains(&a) && sel.contains(&b) && !sel.contains(&c), "{sel:?}");

    // Track Select Forward (A) selects from the clicked clip on
    d.key("A");
    d.click(d.x(72.0), y, json!({}));
    let sel = d.selected();
    assert!(!sel.contains(&a) && sel.contains(&b) && sel.contains(&c), "{sel:?}");
    d.key("V");

    // dragging on the ruler moves the playhead
    let ruler = d.ok("ui.elements", json!({"prefix": "timeline.ruler"}));
    let r = &ruler[0]["rect"];
    let ruler_y = r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0;
    d.drag((d.x(10.0), ruler_y), (d.x(60.0), ruler_y), json!({}));
    let ph = d.app().session.playhead();
    assert_eq!(d.app().session.sequence_rate().frame_at(ph), 60, "scrubbing on the ruler");
}

#[test]
fn moving_clips_with_the_mouse() {
    let mut d = Driver::new();
    let [a, b, c] = three_clips(&mut d);
    // room after the clips to drop on
    d.key("-");
    measure(&mut d, a, c);
    let y = d.y(c);
    // drag C right by 24 frames
    let steps = d.app().session.history.undo.len();
    d.drag((d.x(120.0), y), (d.x(144.0), y), json!({}));
    assert_eq!(d.span(c), (120, 168), "C moved by the whole drag");
    assert_eq!(d.app().session.history.undo.len(), steps + 1, "in one undo step");
    d.undo();
    assert_eq!(d.span(c), (96, 144));
    // Cmd-drag C onto B's start inserts it there: B moves after it
    d.drag((d.x(120.0), y), (d.x(72.0), y), json!({"command": true}));
    assert_eq!((d.span(a), d.span(c), d.span(b)), ((0, 48), (48, 96), (96, 144)), "C inserted before B");
    d.undo();
    assert_eq!((d.span(b), d.span(c)), ((48, 96), (96, 144)));
    // Alt-drag copies: the selected C (picture and sound) gets a copy right after it, C stays
    d.click(d.x(120.0), y, json!({}));
    d.drag((d.x(120.0), y), (d.x(168.0), y), json!({"alt": true}));
    assert_eq!(d.span(c), (96, 144), "the original stays");
    let v1 = d.v1();
    assert_eq!(v1.len(), 4, "a copy was added");
    let copy = *v1.last().unwrap();
    assert_eq!(d.span(copy), (144, 192));
    let sounds = d.app().session.active_sequence().unwrap().audio_tracks[0].items.len();
    assert_eq!(sounds, 4, "with its sound");
    assert!(d.selected().contains(&copy), "the copies are selected");
    d.undo();
    assert_eq!(d.v1().len(), 3);
    // Alt on an unselected clip takes one side of the link: only B's picture is copied, onto V2
    d.exec("edit.deselectAll", json!({}));
    d.drag((d.x(72.0), y), (d.x(72.0), y - 50.0), json!({"alt": true}));
    let q = d.app().session.active_sequence().unwrap();
    assert_eq!((q.video_tracks[1].items.len(), q.audio_tracks[0].items.len()), (1, 3), "one picture copied up to V2");
    assert_eq!(q.video_tracks[0].items.len(), 3, "B stays on V1");
    d.undo();

    // drag B up to V2: it moves track, its sound stays on A1
    let v2 = d.app().session.active_sequence().unwrap().video_tracks[1].id;
    d.drag((d.x(72.0), y), (d.x(72.0), y - 50.0), json!({}));
    let on = d.app().session.active_sequence().and_then(|q| q.find_item(b)).map(|(t, _)| t);
    assert_eq!(on, Some(v2), "B moved up a track");
    d.undo();
}

#[test]
fn trimming_with_the_mouse() {
    let mut d = Driver::new();
    let [a, b, c] = three_clips(&mut d);
    let y = d.y(b);
    // selection tool: C's out edge in by 12 (nothing after it: a plain trim)
    d.drag((d.x(144.0) - 2.0, y), (d.x(132.0) - 2.0, y), json!({}));
    assert_eq!(d.span(c), (96, 132), "trimmed C's out point");
    d.undo();
    // ripple tool: B's out edge in by 12 pulls C in
    d.key("B");
    d.drag((d.x(96.0) - 2.0, y), (d.x(84.0) - 2.0, y), json!({}));
    assert_eq!((d.span(b), d.span(c)), ((48, 84), (84, 132)), "ripple trim");
    d.undo();
    // rolling tool: the A|B cut right by 6
    d.key("N");
    d.drag((d.x(48.0) - 2.0, y), (d.x(54.0) - 2.0, y), json!({}));
    assert_eq!((d.span(a), d.span(b)), ((0, 54), (54, 96)), "rolled the cut");
    d.undo();
    // razor: click in B splits it
    d.key("C");
    d.click(d.x(72.0), y, json!({}));
    assert_eq!(d.v1().len(), 4, "B was cut in two");
    assert_eq!(d.span(b), (48, 72));
    d.undo();
    // slip: B's content moves, its place doesn't
    d.key("Y");
    let src = |d: &Driver| d.app().session.active_sequence().and_then(|q| q.find_item(b)).map(|(_, i)| i.source_in).unwrap();
    let s0 = src(&d);
    d.drag((d.x(72.0), y), (d.x(66.0), y), json!({}));
    assert_eq!(d.span(b), (48, 96), "slip keeps the clip in place");
    assert_eq!(src(&d) - s0, d.f(6), "and shows 6 frames later in its media");
    d.undo();
    // slide: B moves between A and C, which trim to fit
    d.key("U");
    d.drag((d.x(72.0), y), (d.x(78.0), y), json!({}));
    assert_eq!((d.span(a), d.span(b), d.span(c)), ((0, 54), (54, 102), (102, 144)), "slide");
    d.undo();
    // rate stretch: C's out edge out by 12 slows it down (zoomed out: the sequence end of a
    // fitted view is where dragging scrolls the timeline)
    d.key("-");
    measure(&mut d, a, c);
    d.key("R");
    d.drag((d.x(144.0) - 2.0, y), (d.x(156.0) - 2.0, y), json!({}));
    assert_eq!(d.span(c), (96, 156), "stretched");
    let speed = d.app().session.active_sequence().and_then(|q| q.find_item(c)).map(|(_, i)| i.speed).unwrap();
    assert!((speed - 48.0 / 60.0).abs() < 0.01, "speed {speed}");
    d.undo();
    d.key("V");
    assert_eq!((d.span(a), d.span(b), d.span(c)), ((0, 48), (48, 96), (96, 144)), "every gesture undid in one step");
}

/// The editing keys act on the selection and the playhead as their names say.
#[test]
fn editing_keys_do_what_they_say() {
    let mut d = Driver::new();
    let [a, b, c] = three_clips(&mut d);
    let rate = d.app().session.sequence_rate();
    let ph = |d: &Driver| rate.frame_at(d.app().session.playhead());
    // Home / End / Down / Up / Right / Left / Shift+Right
    d.key("End");
    assert_eq!(ph(&d), 144);
    d.key("Home");
    assert_eq!(ph(&d), 0);
    d.key("Down");
    assert_eq!(ph(&d), 48, "next edit");
    d.key("Down");
    assert_eq!(ph(&d), 96);
    d.key("Up");
    assert_eq!(ph(&d), 48, "previous edit");
    d.key("Right");
    assert_eq!(ph(&d), 49);
    d.key("Shift+Right");
    assert_eq!(ph(&d), 54);
    d.key("Left");
    assert_eq!(ph(&d), 53);

    // Cmd+K adds an edit at the playhead on the targeted tracks
    d.key("Cmd+K");
    assert_eq!(d.v1().len(), 4, "B was cut at the playhead");
    assert_eq!(d.span(b), (48, 53));
    d.undo();

    // D selects the clip at the playhead; Backspace lifts it (a gap stays)
    d.key("D");
    assert!(d.selected().contains(&b));
    d.key("Backspace");
    assert_eq!(d.v1(), vec![a, c], "B lifted");
    assert_eq!(d.span(c), (96, 144), "a gap stays");
    d.undo();
    // Shift+Delete ripple deletes it
    d.key("D");
    d.key("Shift+Delete");
    assert_eq!(d.span(c), (48, 96), "C closed up");
    d.undo();

    // I / O mark, ; lifts and ' extracts the marked range
    d.exec("playhead.set", json!({"frame": 60}));
    d.key("I");
    d.exec("playhead.set", json!({"frame": 71}));
    d.key("O");
    let q = d.app().session.active_sequence().unwrap();
    assert_eq!((q.mark_in.map(|t| rate.frame_at(t)), q.mark_out.map(|t| rate.frame_at(t))), (Some(60), Some(71)));
    d.key("'");
    assert_eq!(d.span(c), (84, 132), "extract closed the 12 marked frames");
    d.undo();
    d.key(";");
    assert_eq!(d.span(c), (96, 144), "lift leaves the gap");
    d.undo();
    d.key("Cmd+Shift+X");

    // Q / W ripple trim to the playhead
    d.exec("playhead.set", json!({"frame": 60}));
    d.key("Q");
    assert_eq!((d.v1().len(), d.span(c)), (3, (84, 132)), "Q trimmed B's head to the playhead and rippled");
    d.undo();
    d.exec("playhead.set", json!({"frame": 60}));
    d.key("W");
    assert_eq!(d.span(c), (60, 108), "W trimmed B's tail to the playhead and rippled");
    d.undo();

    // Cmd+C / Cmd+V paste a copy at the playhead
    d.exec("timeline.select", json!({"clips": [a.0]}));
    d.key("Cmd+C");
    d.exec("playhead.set", json!({"frame": 144}));
    d.key("Cmd+V");
    assert_eq!(d.v1().len(), 4, "pasted a copy");
    let last = *d.v1().last().unwrap();
    assert_eq!(d.span(last), (144, 192));
    d.undo();

    // M adds a marker; = and - zoom; S toggles snapping; Cmd+A / Cmd+Shift+A select and deselect
    let markers = d.app().session.active_sequence().unwrap().markers.len();
    d.key("M");
    assert_eq!(d.app().session.active_sequence().unwrap().markers.len(), markers + 1);
    let z = d.app().ui.timeline.target_pps;
    d.key("=");
    assert!(d.app().ui.timeline.target_pps > z);
    d.key("-");
    d.key("-");
    assert!(d.app().ui.timeline.target_pps < z);
    let snap = d.app().session.state.snapping;
    d.key("S");
    assert_ne!(d.app().session.state.snapping, snap);
    d.key("Cmd+A");
    assert_eq!(d.selected().len(), 6);
    d.key("Cmd+Shift+A");
    assert!(d.selected().is_empty());

    // Space plays, K stops; L plays forward, J backward
    let play = |d: &Driver| (d.app().playback.playing, d.app().playback.speed);
    d.key("Space");
    assert!(play(&d).0, "Space plays");
    d.key("K");
    assert!(!play(&d).0, "K stops");
    d.key("L");
    assert!(play(&d).0 && play(&d).1 > 0.0, "L plays forward: {:?}", play(&d));
    d.key("K");
    d.key("J");
    assert!(play(&d).0 && play(&d).1 < 0.0, "J plays backward: {:?}", play(&d));
    d.key("K");
    assert!(!play(&d).0);
}
