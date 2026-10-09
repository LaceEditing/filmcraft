use serde_json::json;

use crate::Session;
use crate::presets::{KeyframeMode, retime};
use filmcraft_project::{ClipId, ParamValue, TrackItem};
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

fn media_len(it: &TrackItem) -> Tick {
    Tick((it.duration.0 as f64 * it.speed.abs()).round() as i64)
}

/// Save a keyframed blur from clip A, apply with each keyframe mode to clip B (another length).
#[test]
fn keyframe_modes_retime_onto_the_target() {
    let (mut s, clips) = demo();
    let (a, b) = (clips[0], clips[2]);
    let ia = item(&s, a);
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "gaussian_blur"})).unwrap();
    // keyframes 0.5 s after the in point (20) and 1 s before the out point (0)
    let k1 = ia.start + secs(0.5);
    let k2 = ia.end() - secs(1.0);
    s.set_playhead(k1);
    let k1 = s.playhead();
    s.execute("effects.toggleAnimation", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "time": k1.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 20.0, "time": k1.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 0.0, "time": k2.0})).unwrap();
    let ib = item(&s, b);
    assert_ne!(media_len(&ia), media_len(&ib), "the demo clips differ in length");
    let la = media_len(&ia);
    let lb = media_len(&ib);
    for (mode, key) in [(KeyframeMode::AnchorToIn, "anchorIn"), (KeyframeMode::AnchorToOut, "anchorOut"), (KeyframeMode::Scale, "scale")] {
        let name = format!("blur-{key}");
        let bi = item(&s, a).effects.iter().position(|e| e.effect == "gaussian_blur").unwrap();
        s.execute("presets.save", json!({"clip": a.0, "name": name, "keyframes": key, "effects": [bi]})).unwrap();
        let before = item(&s, b).effects.len();
        s.execute("presets.apply", json!({"preset": name, "clips": [b.0]})).unwrap();
        let it = item(&s, b);
        assert_eq!(it.effects.len(), before + 1);
        let e = it.effects.iter().rfind(|e| e.effect == "gaussian_blur").unwrap();
        // standard effects go before the intrinsic ones
        assert!(it.effects.iter().position(|x| x.effect == "gaussian_blur").unwrap() < it.effects.iter().position(|x| x.effect == "motion").unwrap());
        let p = e.param("blurriness").unwrap();
        assert_eq!(
            p.keyframes.len(),
            2,
            "{:?} src {:?}",
            p.keyframes.iter().map(|k| (k.time, k.value.clone())).collect::<Vec<_>>(),
            item(&s, a)
                .effects
                .iter()
                .find(|e| e.effect == "gaussian_blur")
                .unwrap()
                .param("blurriness")
                .unwrap()
                .keyframes
                .iter()
                .map(|k| k.time)
                .collect::<Vec<_>>()
        );
        let rel1 = ia.source_time_at(k1) - ia.source_in;
        let rel2 = ia.source_time_at(k2) - ia.source_in;
        let (e1, e2) = match mode {
            KeyframeMode::AnchorToIn => (ib.source_in + rel1, ib.source_in + rel2),
            KeyframeMode::AnchorToOut => (ib.source_in + lb - (la - rel1), ib.source_in + lb - (la - rel2)),
            KeyframeMode::Scale => (
                ib.source_in + Tick((rel1.0 as i128 * lb.0 as i128 / la.0 as i128) as i64),
                ib.source_in + Tick((rel2.0 as i128 * lb.0 as i128 / la.0 as i128) as i64),
            ),
        };
        assert_eq!(p.keyframes[0].time, e1, "{key}");
        assert_eq!(p.keyframes[1].time, e2, "{key}");
        assert_eq!(p.keyframes[0].value, ParamValue::Float(20.0));
        assert_eq!(retime(rel1, mode, la, &ib), e1);
        s.undo();
        assert_eq!(item(&s, b).effects.len(), before, "apply is one undo step");
    }
}

#[test]
fn save_without_keyframes_keeps_the_value_at_the_playhead() {
    let (mut s, clips) = demo();
    let a = clips[0];
    let ia = item(&s, a);
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "gaussian_blur"})).unwrap();
    s.set_playhead(ia.start);
    s.execute("effects.toggleAnimation", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "time": ia.start.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 40.0, "time": ia.start.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 0.0, "time": (ia.start + secs(2.0)).0}))
        .unwrap();
    s.set_playhead(ia.start + secs(1.0));
    let bi = item(&s, a).effects.iter().position(|e| e.effect == "gaussian_blur").unwrap();
    s.execute("presets.save", json!({"name": "half blur", "keyframes": "none", "effects": [bi]})).unwrap();
    let l = s.execute("presets.list", json!({})).unwrap();
    let p = l["presets"].as_array().unwrap().iter().find(|p| p["name"] == "half blur").unwrap().clone();
    assert_eq!(p["animated"], false);
    s.execute("presets.apply", json!({"preset": "half blur", "clips": [clips[1].0]})).unwrap();
    let e = item(&s, clips[1]).effects.into_iter().find(|e| e.effect == "gaussian_blur").unwrap();
    let v = e.param("blurriness").unwrap();
    assert!(!v.is_animated());
    let ParamValue::Float(x) = v.value else { panic!() };
    assert!((x - 20.0).abs() < 1.0, "value at the playhead: {x}");
}

#[test]
fn presets_with_masks_and_intrinsics_round_trip_through_files() {
    let (mut s, clips) = demo();
    let a = clips[0];
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "black_white"})).unwrap();
    s.execute("masks.add", json!({"effect": "black_white", "shape": "ellipse", "center": [960, 540], "size": [800, 400]})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "opacity", "param": "opacity", "value": 50.0})).unwrap();
    let ia = item(&s, a);
    let bw = ia.effects.iter().position(|e| e.effect == "black_white").unwrap();
    let op = ia.effects.iter().position(|e| e.effect == "opacity").unwrap();
    s.execute("presets.save", json!({"name": "masked bw", "effects": [bw, op], "description": "test"})).unwrap();
    let dir = std::env::temp_dir().join(format!("filmcraft-presets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.json").to_string_lossy().to_string();
    let r = s.execute("presets.export", json!({"path": path, "names": ["masked bw", "Fade In"]})).unwrap();
    assert_eq!(r["count"], 2);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("filmcraft.effect-presets"));
    // a fresh session imports and applies it
    let (mut s2, clips2) = demo();
    s2.execute("presets.import", json!({"path": path})).unwrap();
    let names: Vec<String> =
        s2.execute("presets.list", json!({})).unwrap()["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"masked bw".to_string()));
    s2.execute("presets.apply", json!({"preset": "masked bw", "clips": [clips2[1].0]})).unwrap();
    let it = item(&s2, clips2[1]);
    let e = it.effects.iter().find(|e| e.effect == "black_white").unwrap();
    assert_eq!(e.masks.len(), 1);
    assert_eq!(it.effects.iter().filter(|e| e.effect == "opacity").count(), 1, "intrinsic replaced, not duplicated");
    assert_eq!(it.effect("opacity").unwrap().param("opacity").unwrap().value, ParamValue::Float(50.0));
    // garbage is rejected
    std::fs::write(dir.join("bad.json"), b"{\"format\":\"something\",\"version\":1,\"presets\":[]}").unwrap();
    assert!(s2.execute("presets.import", json!({"path": dir.join("bad.json").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builtins_apply_and_user_presets_persist() {
    let (mut s, clips) = demo();
    let l = s.execute("presets.list", json!({})).unwrap();
    let all = l["presets"].as_array().unwrap();
    assert!(all.len() >= 8);
    assert!(all.iter().all(|p| p["builtin"] == true));
    for p in all {
        let name = p["name"].as_str().unwrap();
        s.execute("presets.apply", json!({"preset": name, "clips": [clips[0].0]})).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    // Fade Out anchors to the out point
    let it = item(&s, clips[0]);
    let op = it.effect("opacity").unwrap().param("opacity").unwrap();
    assert_eq!(op.keyframes.last().unwrap().time, it.source_in + media_len(&it));
    assert!(s.execute("presets.delete", json!({"name": "Fade In"})).is_err(), "built-ins stay");
    // persistence through the data directory
    let dir = std::env::temp_dir().join(format!("filmcraft-presets-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.presets.set_dir(&dir);
    s.state.selection = vec![clips[1]];
    s.execute("effects.apply", json!({"effect": "invert"})).unwrap();
    s.execute("presets.save", json!({"name": "Mine"})).unwrap();
    s.execute("presets.rename", json!({"name": "Mine", "to": "Mine 2"})).unwrap();
    let mut lib = crate::presets::PresetLibrary::default();
    lib.set_dir(&dir);
    assert_eq!(lib.user.len(), 1);
    assert_eq!(lib.user[0].name, "Mine 2");
    s.execute("presets.delete", json!({"name": "Mine 2"})).unwrap();
    let mut lib = crate::presets::PresetLibrary::default();
    lib.set_dir(&dir);
    assert!(lib.user.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mask_geometry_scales_to_the_target_frame_size() {
    let (s, clips) = demo();
    let it = item(&s, clips[0]);
    let p = crate::presets::builtin_presets().into_iter().find(|p| p.name == "Spotlight").unwrap();
    let fx = crate::presets::instantiate(&p, &it, (960, 540));
    let m = &fx[0].masks[0];
    let path = m.path_at(it.source_in);
    let c = path.centroid();
    assert!((c.x - 480.0).abs() < 1e-6 && (c.y - 270.0).abs() < 1e-6, "{c:?}");
    assert_eq!(m.feather.value, ParamValue::Float(80.0));
}

/// The Fx Editor's Presets menu: the settings of one effect (a track insert or a clip effect)
/// saved under a name, applied to the same kind of effect anywhere, "(Default)" resets it.
#[test]
fn effect_settings_presets_from_inserts_and_clips() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "hard_limiter"})).unwrap();
    s.execute("mixer.setInsert", json!({"strip": "A1", "slot": 0, "params": {"max": -3.0, "link": false}})).unwrap();
    // automation on the insert: the preset keeps the value at the playhead only
    s.execute("mixer.setKeyframe", json!({"strip": "A1", "lane": "fx.0.boost", "time": 0, "value": 6.0})).unwrap();
    let r = s.execute("presets.saveSettings", json!({"name": "Loud limit", "strip": "A1", "slot": 0})).unwrap();
    assert_eq!(r["ids"], json!(["hard_limiter"]));
    let saved = s.presets.find("Loud limit").unwrap();
    assert!(saved.effects[0].params.values().all(|p| p.keyframes.is_empty()));
    // onto a clip's Hard Limiter
    let clip = s.active_sequence().unwrap().audio_tracks[0].items[0].id;
    s.execute("effects.apply", json!({"clips": [clip.0], "effect": "hard_limiter"})).unwrap();
    let index = s.active_sequence().unwrap().find_item(clip).unwrap().1.effects.iter().position(|e| e.effect == "hard_limiter").unwrap();
    s.execute("presets.applySettings", json!({"preset": "Loud limit", "clip": clip.0, "index": index})).unwrap();
    let e = s.active_sequence().unwrap().find_item(clip).unwrap().1.effects[index].clone();
    assert_eq!(e.param("max").map(|p| p.value.clone()), Some(ParamValue::Float(-3.0)));
    assert_eq!(e.param("link").map(|p| p.value.clone()), Some(ParamValue::Bool(false)));
    assert_eq!(s.history.undo.last().map(|u| u.0.as_str()), Some("Apply Preset Loud limit"));
    // not onto another effect
    s.execute("mixer.addInsert", json!({"strip": "A2", "effect": "bass"})).unwrap();
    assert!(s.execute("presets.applySettings", json!({"preset": "Loud limit", "strip": "A2", "slot": 0})).is_err());
    // (Default) resets the insert, automation included
    s.execute("presets.applySettings", json!({"preset": "(Default)", "strip": "A1", "slot": 0})).unwrap();
    let ins = s.active_sequence().unwrap().audio_tracks[0].effects[0].clone();
    assert_eq!(ins.param("max").map(|p| p.value.clone()), Some(ParamValue::Float(-0.1)));
    assert!(ins.param("boost").is_some_and(|p| p.keyframes.is_empty()));
    assert!(s.execute("presets.saveSettings", json!({"name": "(Default)", "strip": "A1", "slot": 0})).is_err(), "reserved name");
    assert!(s.execute("presets.saveSettings", json!({"name": "x", "strip": "A1", "slot": 4})).is_err());
}
