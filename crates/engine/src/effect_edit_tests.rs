//! Effect Controls editing of several keyframes / effects at once: delete, move and interpolate
//! keyframes, copy / paste keyframes and effects, remove and reorder effects; each one undo step.

use serde_json::{Value, json};

use crate::Session;
use filmcraft_project::{ClipId, Interpolation, ParamValue, TrackItem};
use filmcraft_time::{TICKS_PER_SECOND, Tick};

fn demo() -> (Session, Vec<ClipId>) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let clips: Vec<ClipId> = q.video_tracks[0].items.iter().map(|i| i.id).collect();
    (s, clips)
}

fn item(s: &Session, c: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(c).unwrap().1.clone()
}

fn secs(x: f64) -> Tick {
    Tick((x * TICKS_PER_SECOND as f64).round() as i64)
}

fn motion(it: &TrackItem) -> usize {
    it.effects.iter().position(|e| e.effect == "motion").unwrap()
}

/// Keyframes on Motion ▸ Scale at 0.5, 1 and 1.5 s into the clip (values 100, 150, 120); their
/// media times.
fn keyed(s: &mut Session, c: ClipId) -> Vec<Tick> {
    let it = item(s, c);
    let mut times = Vec::new();
    for (i, (at, v)) in [(0.5, 100.0), (1.0, 150.0), (1.5, 120.0)].into_iter().enumerate() {
        // at the playhead (on a frame)
        s.set_playhead(it.start + secs(at));
        let tl = s.playhead();
        if i == 0 {
            // the stopwatch turned on at the first one
            s.execute("effects.toggleAnimation", json!({"clip": c.0, "effect": "motion", "param": "scale"})).unwrap();
        }
        s.execute("effects.setParam", json!({"clip": c.0, "effect": "motion", "param": "scale", "value": v})).unwrap();
        times.push(it.source_time_at(tl));
    }
    times
}

fn scale_keys(s: &Session, c: ClipId) -> Vec<(Tick, f64)> {
    let it = item(s, c);
    it.effects[motion(&it)].params["scale"].keyframes.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect()
}

fn key(effect: usize, t: Tick) -> Value {
    json!({"effect": effect, "param": "scale", "mediaTime": t.0})
}

#[test]
fn several_keyframes_delete_move_and_interpolate_in_one_step_each() {
    let (mut s, clips) = demo();
    let c = clips[0];
    let times = keyed(&mut s, c);
    let m = motion(&item(&s, c));
    assert_eq!(scale_keys(&s, c).len(), 3, "{:?}", scale_keys(&s, c));
    let undo_before = s.history.undo.len();

    // move the last two 0.25 s later: the spacing stays
    let r = s.execute("effects.moveKeyframes", json!({"clip": c.0, "keys": [key(m, times[1]), key(m, times[2])], "by": secs(0.25).0})).unwrap();
    assert_eq!(r["keys"].as_array().unwrap().len(), 2);
    assert_eq!(scale_keys(&s, c), vec![(times[0], 100.0), (times[1] + secs(0.25), 150.0), (times[2] + secs(0.25), 120.0)]);
    assert_eq!(s.history.undo.len(), undo_before + 1, "one undo step");
    // moving onto another keyframe replaces it
    s.execute("effects.moveKeyframes", json!({"clip": c.0, "keys": [key(m, times[1] + secs(0.25))], "by": (times[0] - times[1] - secs(0.25)).0})).unwrap();
    assert_eq!(scale_keys(&s, c), vec![(times[0], 150.0), (times[2] + secs(0.25), 120.0)]);
    s.undo();
    s.undo();
    assert_eq!(scale_keys(&s, c).len(), 3);

    // interpolation of two at once
    s.execute("effects.setKeyframesInterpolation", json!({"clip": c.0, "keys": [key(m, times[0]), key(m, times[1])], "interpolation": "hold"})).unwrap();
    let it = item(&s, c);
    let interps: Vec<Interpolation> = it.effects[m].params["scale"].keyframes.iter().map(|k| k.interp).collect();
    assert_eq!(interps[..2], [Interpolation::Hold, Interpolation::Hold]);
    assert_ne!(interps[2], Interpolation::Hold);

    // delete two: one step; deleting what is not there says so
    let before = s.history.undo.len();
    s.execute("effects.deleteKeyframes", json!({"clip": c.0, "keys": [key(m, times[0]), key(m, times[2])]})).unwrap();
    assert_eq!(scale_keys(&s, c), vec![(times[1], 150.0)]);
    assert_eq!(s.history.undo.len(), before + 1);
    assert!(s.execute("effects.deleteKeyframes", json!({"clip": c.0, "keys": [key(m, times[0])]})).is_err());
    // hostile input: no keys, bad keys
    assert!(s.execute("effects.deleteKeyframes", json!({"clip": c.0, "keys": []})).is_err());
    assert!(s.execute("effects.moveKeyframes", json!({"clip": c.0, "keys": [{"effect": 9999, "param": "scale", "mediaTime": 0}], "by": 5})).is_err());
    assert!(s.execute("effects.moveKeyframes", json!({"clip": c.0, "keys": [key(m, times[1])], "by": i64::MAX})).is_ok(), "saturates, no panic");
}

#[test]
fn keyframes_paste_at_the_playhead_with_their_spacing() {
    let (mut s, clips) = demo();
    let (a, b) = (clips[0], clips[1]);
    let times = keyed(&mut s, a);
    let m = motion(&item(&s, a));
    s.execute("effects.copyKeyframes", json!({"clip": a.0, "keys": [key(m, times[1]), key(m, times[2])]})).unwrap();
    // onto clip B, 0.2 s into it
    let ib = item(&s, b);
    let at = ib.start + secs(0.2);
    s.set_playhead(at);
    let r = s.execute("effects.pasteKeyframes", json!({"clip": b.0})).unwrap();
    assert_eq!(r["keys"].as_array().unwrap().len(), 2);
    let mt = item(&s, b).source_time_at(s.playhead());
    assert_eq!(scale_keys(&s, b), vec![(mt, 150.0), (mt + (times[2] - times[1]), 120.0)]);
    // nothing on the clipboard of that kind: an error, not a crash
    s.state.fx_clipboard = None;
    assert!(s.execute("effects.pasteKeyframes", json!({"clip": b.0})).is_err());
}

#[test]
fn effects_copy_paste_remove_and_reorder() {
    let (mut s, clips) = demo();
    let (a, b) = (clips[0], clips[1]);
    s.state.selection = vec![a];
    for fx in ["gaussian_blur", "tint", "invert"] {
        s.execute("effects.apply", json!({"effect": fx})).unwrap();
    }
    let ia = item(&s, a);
    let idx = |it: &TrackItem, id: &str| it.effects.iter().position(|e| e.effect == id).unwrap();
    let (blur, tint) = (idx(&ia, "gaussian_blur"), idx(&ia, "tint"));
    s.execute("effects.setParam", json!({"clip": a.0, "effect": blur, "param": "blurriness", "value": 33.0})).unwrap();

    // copy Blur and Tint, paste on B: both arrive before B's fixed effects, with their values
    s.execute("effects.copy", json!({"clip": a.0, "indices": [blur, tint]})).unwrap();
    let before = s.history.undo.len();
    s.execute("effects.paste", json!({"clips": [b.0]})).unwrap();
    assert_eq!(s.history.undo.len(), before + 1);
    let ib = item(&s, b);
    assert!(idx(&ib, "gaussian_blur") < idx(&ib, "motion") && idx(&ib, "tint") < idx(&ib, "motion"));
    assert_eq!(ib.effects[idx(&ib, "gaussian_blur")].params["blurriness"].value, ParamValue::Float(33.0));

    // reorder on A: Invert to the front of the standard effects; fixed effects stay put
    let ia = item(&s, a);
    let inv = idx(&ia, "invert");
    s.execute("effects.move", json!({"clip": a.0, "index": inv, "to": blur})).unwrap();
    let ia = item(&s, a);
    assert_eq!(idx(&ia, "invert"), blur);
    assert!(s.execute("effects.move", json!({"clip": a.0, "index": idx(&ia, "motion"), "to": 0})).is_err(), "Motion stays");
    assert!(s.execute("effects.move", json!({"clip": a.0, "index": 0, "to": idx(&ia, "opacity")})).is_err(), "not among the fixed ones");
    assert!(s.execute("effects.move", json!({"clip": a.0, "index": 9999, "to": 0})).is_err());

    // remove two at once (Motion in the list stays); only fixed ones: an error
    let ia = item(&s, a);
    let n = ia.effects.len();
    let r = s.execute("effects.remove", json!({"clip": a.0, "indices": [idx(&ia, "invert"), idx(&ia, "tint"), idx(&ia, "motion")]})).unwrap();
    assert_eq!(r["removed"], json!(2));
    let ia = item(&s, a);
    assert_eq!(ia.effects.len(), n - 2);
    assert!(ia.effects.iter().any(|e| e.effect == "motion"));
    assert!(s.execute("effects.remove", json!({"clip": a.0, "indices": [idx(&ia, "motion")]})).is_err());
    assert!(s.execute("effects.remove", json!({"clip": a.0, "indices": [9999]})).is_err());
}
