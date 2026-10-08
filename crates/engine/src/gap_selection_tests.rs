//! Selecting a gap (clicking empty track space between clips) and closing it with Clear or
//! Ripple Delete, as in Premiere: clearing a gap has nothing to lift, so it ripples.

use filmcraft_project::ClipId;
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::Session;

fn range(s: &Session, c: ClipId) -> (Tick, Tick) {
    s.active_sequence().unwrap().find_item(c).map(|(_, i)| (i.start, i.end())).unwrap()
}

/// Two 48-frame clips (pictures on V1, linked sounds on A1) with a 48-frame gap between them, and
/// an empty V2: [(picture, sound); 2].
fn two_clips_with_a_gap(s: &mut Session) -> [(ClipId, ClipId); 2] {
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("file.newSequence", json!({"name": "Gaps", "video": 2, "audio": 1})).unwrap();
    let rate = s.sequence_rate();
    let item = s.project.items.values().find(|i| i.has_video() && i.has_audio() && i.as_media().is_some()).map(|i| i.id).unwrap();
    for frame in [0, 96] {
        s.execute("timeline.place", json!({"item": item.0, "frame": frame, "sourceIn": rate.tick_of(48).0, "duration": rate.tick_of(48).0})).unwrap();
    }
    let q = s.active_sequence().unwrap();
    [0, 1].map(|n| (q.video_tracks[0].items[n].id, q.audio_tracks[0].items[n].id))
}

#[test]
fn clearing_a_selected_gap_closes_it() {
    for cmd in ["edit.clear", "edit.rippleDelete"] {
        let mut s = Session::default();
        let [(picture1, sound1), (picture2, sound2)] = two_clips_with_a_gap(&mut s);
        let rate = s.sequence_rate();
        let f = |n: i64| rate.tick_of(n);
        let (f48, f96) = (f(48), f(96));
        s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
        let before = s.project.clone();

        let r = s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
        assert_eq!(r, json!({"track": s.active_sequence().unwrap().video_tracks[0].id.0, "start": f48.0, "end": f96.0}));
        assert!(s.state.selection.is_empty(), "selecting a gap deselects the clips");
        assert!(s.selected_gap().is_some());
        s.execute(cmd, json!({})).unwrap();
        assert_eq!(range(&s, picture2), (f48, f96), "{cmd}: the later clip moves up to close the gap");
        assert_eq!(range(&s, sound2), (f48, f96), "{cmd}: and its sound moves with it (A1 is sync-locked)");
        assert_eq!((range(&s, picture1), range(&s, sound1)), ((Tick::ZERO, f48), (Tick::ZERO, f48)));
        assert_eq!(s.history.undo.last().map(|u| u.0.as_str()), Some("Ripple Delete"));
        assert!(s.state.selected_gap.is_none(), "the closed gap is no longer selected");
        assert!(s.execute(cmd, json!({})).is_err(), "nothing is selected now");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.project, *before, "{cmd}: one undo step restores the gap");
    }
}

#[test]
fn the_gap_before_the_first_clip_closes_to_zero() {
    let mut s = Session::default();
    let [_, (picture2, _)] = two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.select", json!({"clips": [s.active_sequence().unwrap().video_tracks[0].items[0].id.0]})).unwrap();
    s.execute("edit.rippleDelete", json!({})).unwrap();
    assert_eq!(range(&s, picture2), (f(48), f(96)), "the first clip's ripple delete left a leading gap");
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(10).0})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(range(&s, picture2), (Tick::ZERO, f(48)));
}

#[test]
fn only_empty_space_with_a_clip_after_it_is_a_gap() {
    let mut s = Session::default();
    let [(picture1, _), _] = two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    let before = s.project.clone();
    for (track, frame) in [("V1", 10), ("V1", 200), ("V2", 60), ("A1", 20)] {
        s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
        let r = s.execute("timeline.selectGap", json!({"track": track, "time": f(frame).0})).unwrap();
        assert_eq!(r, Value::Null, "{track} at {frame}: no gap");
        assert!(s.state.selection.is_empty(), "clicking empty space still deselects");
        assert!(s.execute("edit.clear", json!({})).is_err(), "{track} at {frame}: Clear has nothing to act on");
    }
    assert_eq!(*s.project, *before);
    // hostile parameters are errors, never a panic
    assert!(s.execute("timeline.selectGap", json!({"track": "V9", "time": 0})).is_err());
    assert!(s.execute("timeline.selectGap", json!({"track": "V1"})).is_err());
    assert_eq!(s.execute("timeline.selectGap", json!({"track": "V1", "time": i64::MIN})).unwrap(), Value::Null);
}

#[test]
fn a_clip_selection_wins_over_the_gap() {
    let mut s = Session::default();
    let [(picture1, sound1), (picture2, _)] = two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
    assert!(s.selected_gap().is_none(), "selecting a clip leaves the gap");
    s.execute("edit.clear", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(q.find_item(picture1).is_none() && q.find_item(sound1).is_none(), "Clear lifted the selected clip");
    assert_eq!(range(&s, picture2).0, f(96), "and left the gap");
    // a gap in another sequence is not this sequence's selection
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    assert!(s.selected_gap().is_some());
    s.execute("file.newSequence", json!({"name": "Other", "video": 1, "audio": 1})).unwrap();
    assert!(s.selected_gap().is_none());
}

/// A clip on another sync-locked track over the gap would fall out of sync: the gap stays and the
/// message says why. With that track's sync lock off it closes and the clip stays put.
#[test]
fn a_sync_locked_clip_over_the_gap_keeps_it() {
    let mut s = Session::default();
    let [_, (picture2, sound2)] = two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    let item = s.project.items.values().find(|i| i.has_video() && !i.has_audio()).map(|i| i.id).unwrap();
    let title = s.execute("timeline.place", json!({"item": item.0, "frame": 40, "duration": f(30).0, "track": "V2"})).unwrap()["clips"][0]
        .as_u64()
        .map(ClipId)
        .unwrap();
    let over = range(&s, title);
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    let before = s.project.clone();
    let e = s.execute("edit.clear", json!({})).unwrap_err().to_string();
    assert!(e.contains("sync-locked") && e.contains("Sync Lock"), "{e}");
    assert_eq!(*s.project, *before, "nothing moved");
    assert!(s.selected_gap().is_some(), "the gap is still selected");

    s.execute("timeline.setTrack", json!({"track": "V2", "syncLock": false})).unwrap();
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!((range(&s, picture2).0, range(&s, sound2).0), (f(48), f(48)));
    assert_eq!(range(&s, title), over, "the clip on the unlocked track stays");
}
