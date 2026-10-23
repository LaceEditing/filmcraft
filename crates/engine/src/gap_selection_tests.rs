//! Selecting a gap between clips and closing it with Delete / Backspace / Ripple Delete (#648, #668):
//! `timeline.selectGap` selects the empty time on one track, `edit.clear` and `edit.rippleDelete`
//! close it as one "Ripple Delete" undo step (as in Premiere: clearing a gap has nothing to lift,
//! so it ripples).

use filmcraft_project::ClipId;
use filmcraft_time::Tick;
use serde_json::json;

use crate::{GapSelection, Session};

fn range(s: &Session, c: ClipId) -> (Tick, Tick) {
    s.active_sequence().unwrap().find_item(c).map(|(_, i)| (i.start, i.end())).unwrap()
}

/// Two 48-frame clips (pictures on V1, linked sounds on A1) placed at `frames`.
fn two_clips(s: &mut Session, frames: [i64; 2]) -> [(ClipId, ClipId); 2] {
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("file.newSequence", json!({"name": "Gaps", "video": 1, "audio": 1})).unwrap();
    let rate = s.sequence_rate();
    let item = s.project.items.values().find(|i| i.has_video() && i.has_audio() && i.as_media().is_some()).map(|i| i.id).unwrap();
    for f in frames {
        s.execute("timeline.place", json!({"item": item.0, "frame": f, "sourceIn": rate.tick_of(48).0, "duration": rate.tick_of(48).0})).unwrap();
    }
    s.execute("edit.deselectAll", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    [0, 1].map(|n| (q.video_tracks[0].items[n].id, q.audio_tracks[0].items[n].id))
}

#[test]
fn clicking_between_two_clips_selects_the_gap_and_delete_closes_it() {
    let mut s = Session::default();
    let [(picture1, _), (picture2, sound2)] = two_clips(&mut s, [0, 96]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    let (f48, f60, f96) = (f(48), f(60), f(96));
    s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
    let before = s.project.clone();

    let r = s.execute("timeline.selectGap", json!({"track": "V1", "time": f60.0})).unwrap();
    assert_eq!(r, json!({"track": s.active_sequence().unwrap().video_tracks[0].id.0, "start": f48.0, "end": f96.0}));
    assert!(s.state.selection.is_empty(), "the gap replaces the clip selection");
    assert!(s.is_enabled("edit.clear") && s.is_enabled("edit.rippleDelete"), "Delete and Ripple Delete apply to the gap");

    // Delete / Backspace
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(range(&s, picture2), (f48, f96), "the next clip moves up into the gap");
    assert_eq!(range(&s, sound2), (f48, f96), "and its sound with it");
    assert_eq!(range(&s, picture1).0, Tick::ZERO, "the clip before stays");
    assert!(s.state.gap_selection.is_none(), "the gap is gone");
    s.active_sequence().unwrap().check().unwrap();

    assert_eq!(s.undo().as_deref(), Some("Ripple Delete"), "one undo step");
    assert_eq!(*s.project, *before);
}

#[test]
fn ripple_delete_closes_a_selected_gap_too() {
    let mut s = Session::default();
    let [_, (picture2, _)] = two_clips(&mut s, [0, 96]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.selectGap", json!({"track": "A1", "frame": 50})).unwrap();
    s.execute("edit.rippleDelete", json!({})).unwrap();
    assert_eq!(range(&s, picture2).0, f(48));
}

#[test]
fn the_empty_time_before_the_first_clip_is_a_gap() {
    let mut s = Session::default();
    let [(picture1, _), (picture2, _)] = two_clips(&mut s, [24, 72]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    let r = s.execute("timeline.selectGap", json!({"track": "V1", "frame": 5})).unwrap();
    assert_eq!((r["start"].clone(), r["end"].clone()), (json!(0), json!(f(24).0)));
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!((range(&s, picture1).0, range(&s, picture2).0), (Tick::ZERO, f(48)));
}

#[test]
fn empty_time_after_the_last_clip_is_not_a_gap() {
    let mut s = Session::default();
    two_clips(&mut s, [0, 96]);
    assert!(s.execute("timeline.selectGap", json!({"track": "V1", "frame": 400})).is_err());
    assert!(s.execute("timeline.selectGap", json!({"track": "V1", "frame": 10})).is_err(), "inside a clip");
    assert!(s.state.gap_selection.is_none());
    assert!(!s.is_enabled("edit.clear"), "nothing to delete");
}

#[test]
fn selecting_a_clip_or_deselecting_all_drops_the_gap() {
    let mut s = Session::default();
    let [(picture1, _), _] = two_clips(&mut s, [0, 96]);
    s.execute("timeline.selectGap", json!({"track": "V1", "frame": 60})).unwrap();
    s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
    assert!(s.state.gap_selection.is_none());
    s.execute("timeline.selectGap", json!({"track": "V1", "frame": 60})).unwrap();
    s.execute("edit.deselectAll", json!({})).unwrap();
    assert!(s.state.gap_selection.is_none());
}

/// An edit that fills the gap leaves no gap selected; Delete then removes nothing it should not.
#[test]
fn an_edit_that_changes_the_gap_drops_it() {
    let mut s = Session::default();
    let [(picture1, _), (picture2, _)] = two_clips(&mut s, [0, 96]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.selectGap", json!({"track": "V1", "frame": 60})).unwrap();
    s.execute("timeline.trim", json!({"clip": picture1.0, "edge": "out", "mode": "regular", "deltaFrames": 12})).unwrap();
    assert!(s.state.gap_selection.is_none(), "the gap changed");
    assert!(s.execute("edit.clear", json!({})).is_err());
    assert_eq!(range(&s, picture2).0, f(96), "nothing moved");
}

/// A stale selection (set from outside, as the control channel can) is refused, not acted on.
#[test]
fn a_stale_gap_is_refused() {
    let mut s = Session::default();
    let [_, (picture2, _)] = two_clips(&mut s, [0, 96]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    let track = s.active_sequence().unwrap().video_tracks[0].id;
    s.state.gap_selection = Some(GapSelection { track, range: filmcraft_time::TimeRange::from_bounds(f(10), f(20)) });
    assert!(s.execute("edit.clear", json!({})).is_err());
    assert!(s.state.gap_selection.is_none());
    assert_eq!(range(&s, picture2).0, f(96));
}

#[test]
fn a_gap_on_a_locked_track_cannot_be_selected_or_closed() {
    let mut s = Session::default();
    let [_, (picture2, _)] = two_clips(&mut s, [0, 96]);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.setTrack", json!({"track": "V1", "locked": true})).unwrap();
    assert!(s.execute("timeline.selectGap", json!({"track": "V1", "frame": 60})).is_err());
    // Sequence ▸ Close Gap moved the other tracks while the locked one stayed put
    assert!(s.execute("sequence.closeGap", json!({"track": "V1", "frame": 60})).is_err());
    assert_eq!(range(&s, picture2).0, f(96));
    let sound2 = s.active_sequence().unwrap().audio_tracks[0].items[1].id;
    assert_eq!(range(&s, sound2).0, f(96), "the unlocked sound stays in sync");
}

#[test]
fn hostile_select_gap_parameters_are_errors() {
    let mut s = Session::default();
    assert!(s.execute("timeline.selectGap", json!({"track": "V1", "frame": 1})).is_err(), "no sequence");
    two_clips(&mut s, [0, 96]);
    for p in [
        json!({}),
        json!({"track": "V1"}),
        json!({"frame": 60}),
        json!({"track": "V99", "frame": 60}),
        json!({"track": 999_999, "frame": 60}),
        json!({"track": "V1", "time": i64::MAX}),
        json!({"track": "V1", "time": i64::MIN}),
        json!({"track": "V1", "frame": i64::MAX}),
        json!({"track": "V1", "seconds": f64::MAX}),
        json!({"track": [], "frame": "x"}),
    ] {
        let _ = s.execute("timeline.selectGap", p);
    }
    assert!(s.state.gap_selection.is_none());
}

#[test]
fn the_gap_selection_is_part_of_the_ui_state() {
    let mut s = Session::default();
    two_clips(&mut s, [0, 96]);
    s.execute("timeline.selectGap", json!({"track": "V1", "frame": 60})).unwrap();
    let v = serde_json::to_value(&s.state).unwrap();
    let back: crate::EditorState = serde_json::from_value(v).unwrap();
    assert_eq!(back.gap_selection, s.state.gap_selection);
}

#[test]
fn close_gap_is_available_for_custom_shortcuts() {
    let mut s = Session::default();
    let rows = s.execute("shortcuts.list", json!({"query":"Close Gap"})).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "sequence.closeGap");
    assert!(rows[0]["shortcuts"].as_array().unwrap().is_empty(), "no new default binding");
    s.execute("shortcuts.set", json!({"command":"sequence.closeGap","keys":"Alt+Shift+F8"})).unwrap();
    assert_eq!(s.shortcuts.primary("sequence.closeGap").as_deref(), Some("Alt+Shift+F8"));
}

#[test]
fn close_gap_without_params_closes_selected_gap_in_one_undo_step() {
    let mut s = Session::default();
    let [(first, _), (second, sound)] = two_clips(&mut s, [0, 96]);
    let frame = s.sequence_rate().tick_of(48);
    s.execute("timeline.selectGap", json!({"track":"V1","frame":60})).unwrap();
    let before = serde_json::to_value(&s.project).unwrap();
    let undo = s.history.undo.len();
    assert_eq!(s.execute("sequence.closeGap", json!({})).unwrap(), serde_json::Value::Null);
    assert_eq!(range(&s, first).0, Tick::ZERO);
    assert_eq!(range(&s, second).0, frame);
    assert_eq!(range(&s, sound).0, frame);
    assert!(s.state.gap_selection.is_none());
    assert_eq!(s.history.undo.len(), undo + 1);
    let after = serde_json::to_value(&s.project).unwrap();
    s.undo();
    assert_eq!(serde_json::to_value(&s.project).unwrap(), before);
    s.redo();
    assert_eq!(serde_json::to_value(&s.project).unwrap(), after);
}

#[test]
fn close_gap_rejects_invalid_context_and_explicit_parameters_without_editing() {
    let mut s = Session::default();
    two_clips(&mut s, [0, 96]);
    let before = s.project.clone();
    let undo = s.history.undo.len();
    assert!(s.execute("sequence.closeGap", json!({})).is_err(), "a shortcut needs a selected gap");
    s.execute("timeline.selectGap", json!({"track":"V1","frame":60})).unwrap();
    for params in [json!({"track":null}), json!({"track":false}), json!({"track":"V99","frame":60}), json!({"time":1})] {
        assert!(s.execute("sequence.closeGap", params).is_err(), "invalid explicit requests must not fall back to the selected gap");
        assert!(std::sync::Arc::ptr_eq(&before, &s.project));
        assert_eq!(s.history.undo.len(), undo);
    }
    let selected = s.state.gap_selection.unwrap();
    s.state.gap_selection.as_mut().unwrap().range.duration += Tick(1);
    assert!(s.execute("sequence.closeGap", json!({})).is_err(), "a stale span cannot close a different gap");
    assert!(s.state.gap_selection.is_none());
    assert!(std::sync::Arc::ptr_eq(&before, &s.project));
    assert_eq!(s.history.undo.len(), undo);
    s.execute("timeline.setTrack", json!({"track":"V1","locked":true})).unwrap();
    s.state.gap_selection = Some(selected);
    let before = s.project.clone();
    let undo = s.history.undo.len();
    assert!(s.execute("sequence.closeGap", json!({})).is_err(), "locking a selected gap must protect both picture and sound");
    assert!(std::sync::Arc::ptr_eq(&before, &s.project));
    assert_eq!(s.history.undo.len(), undo);
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

/// Clear and Ripple Delete on a selected gap: clearing a gap has nothing to lift, so it ripples,
/// as in Premiere; the sound on the sync-locked A1 follows, and one undo step restores it.
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
        assert!(s.state.gap_selection.is_some());
        s.execute(cmd, json!({})).unwrap();
        assert_eq!(range(&s, picture2), (f48, f96), "{cmd}: the later clip moves up to close the gap");
        assert_eq!(range(&s, sound2), (f48, f96), "{cmd}: and its sound moves with it (A1 is sync-locked)");
        assert_eq!((range(&s, picture1), range(&s, sound1)), ((Tick::ZERO, f48), (Tick::ZERO, f48)));
        assert_eq!(s.history.undo.last().map(|u| u.0.as_str()), Some("Ripple Delete"));
        assert!(s.state.gap_selection.is_none(), "the closed gap is no longer selected");
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

/// Over a clip, after the last clip, on an empty track or in a sound: no gap, and nothing for
/// Clear to act on. Hostile parameters are errors, never a panic.
#[test]
fn only_empty_space_with_a_clip_after_it_is_a_gap() {
    let mut s = Session::default();
    two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("edit.deselectAll", json!({})).unwrap();
    let before = s.project.clone();
    for (track, frame) in [("V1", 10), ("V1", 200), ("V2", 60), ("A1", 20)] {
        assert!(s.execute("timeline.selectGap", json!({"track": track, "time": f(frame).0})).is_err(), "{track} at {frame}: no gap");
        assert!(s.state.gap_selection.is_none());
        assert!(s.execute("edit.clear", json!({})).is_err(), "{track} at {frame}: Clear has nothing to act on");
    }
    assert_eq!(*s.project, *before);
    assert!(s.execute("timeline.selectGap", json!({"track": "V9", "time": 0})).is_err());
    assert!(s.execute("timeline.selectGap", json!({"track": "V1"})).is_err());
    assert!(s.execute("timeline.selectGap", json!({"track": "V1", "time": i64::MIN})).is_err());
}

#[test]
fn a_clip_selection_wins_over_the_gap() {
    let mut s = Session::default();
    let [(picture1, sound1), (picture2, _)] = two_clips_with_a_gap(&mut s);
    let rate = s.sequence_rate();
    let f = |n: i64| rate.tick_of(n);
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    s.execute("timeline.select", json!({"clips": [picture1.0]})).unwrap();
    assert!(s.state.gap_selection.is_none(), "selecting a clip leaves the gap");
    s.execute("edit.clear", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(q.find_item(picture1).is_none() && q.find_item(sound1).is_none(), "Clear lifted the selected clip");
    assert_eq!(range(&s, picture2).0, f(96), "and left the gap");
    // a gap in another sequence is not this sequence's selection
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    assert!(s.state.gap_selection.is_some());
    s.execute("file.newSequence", json!({"name": "Other", "video": 1, "audio": 1})).unwrap();
    assert!(s.state.gap_selection.is_none());
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
    assert!(s.state.gap_selection.is_some(), "the gap is still selected");

    s.execute("timeline.setTrack", json!({"track": "V2", "syncLock": false})).unwrap();
    s.execute("timeline.selectGap", json!({"track": "V1", "time": f(60).0})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!((range(&s, picture2).0, range(&s, sound2).0), (f(48), f(48)));
    assert_eq!(range(&s, title), over, "the clip on the unlocked track stays");
}
