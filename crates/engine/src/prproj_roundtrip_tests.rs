//! Save a real Premiere Pro project with Ctrl+S, open it again, and compare (development tool,
//! ignored by default). Run with
//! `FC_ROUNDTRIP=/path/to/show.prproj cargo test -p filmcraft-engine --lib roundtrip -- --ignored --nocapture`
//! (the project is copied first; the original is never written). `FC_EDITS=scale,opacity,…` limits
//! the edits to those named; `FC_MODELS=a.prproj,b.prproj` opens other projects first (their
//! effect layouts can be borrowed); `FC_KEEP=out.prproj` keeps the saved project.

use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;
use filmcraft_project::{ItemKind, Project};

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for k in ["id", "uid"] {
                m.remove(k);
            }
            m.values_mut().for_each(strip);
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|f| f.fract() != 0.0) {
                *v = json!((f * 1000.0).round() / 1000.0);
            }
        }
        _ => {}
    }
}

/// One line per thing a user would notice, keyed by sequence name, independent of ids.
pub(crate) fn summary(p: &Project) -> Vec<String> {
    let name_of = |id: filmcraft_project::ItemId| p.item(id).map(|i| i.name.clone()).unwrap_or_else(|| "?".into());
    let mut out = Vec::new();
    fn walk(p: &Project, b: &filmcraft_project::Bin, path: &str, out: &mut Vec<String>) {
        for e in &b.children {
            match e {
                filmcraft_project::BinEntry::Item(id) => {
                    let Some(i) = p.item(*id) else { continue };
                    let kind = match &i.kind {
                        ItemKind::Media(_) => "media",
                        ItemKind::Sequence(_) => "sequence",
                        ItemKind::AdjustmentLayer { .. } => "adjustment",
                        _ => "other",
                    };
                    out.push(format!("item {path}/{:?} {kind}", i.name));
                }
                filmcraft_project::BinEntry::Bin(c) => {
                    out.push(format!("bin {path}/{:?}", c.name));
                    walk(p, c, &format!("{path}/{}", c.name), out);
                }
            }
        }
    }
    let mut items = Vec::new();
    walk(p, &p.root, "", &mut items);
    items.sort();
    out.extend(items);
    let mut seqs: Vec<_> = p.sequences().collect();
    seqs.sort_by(|a, b| a.name.cmp(&b.name));
    for it in seqs {
        let Some(q) = it.as_sequence() else { continue };
        let st = &q.settings;
        let n = &it.name;
        out.push(format!("[{n}] settings {}x{} {:?} {}Hz {:?}", st.width, st.height, st.frame_rate, st.sample_rate, st.audio_master));
        for m in &q.markers {
            out.push(format!("[{n}] marker {} {} {:?} {:?}", m.start.0, m.duration.0, m.name, m.comment));
        }
        for (kind, tracks) in [("V", &q.video_tracks), ("A", &q.audio_tracks)] {
            for (ti, t) in tracks.iter().enumerate() {
                let tn = format!("{kind}{}", ti + 1);
                out.push(format!("[{n}] track {tn} {:?} mute={} lock={} vol={:.2}", t.name, t.muted, t.locked, t.volume_db));
                for c in &t.items {
                    // Motion / Opacity render after standard effects whatever the list order
                    let mut effects = c.effects.clone();
                    effects.sort_by_key(|e| !e.def().is_some_and(|d| d.intrinsic));
                    let mut fx = serde_json::to_value(&effects).unwrap_or_default();
                    strip(&mut fx);
                    out.push(format!(
                        "[{n}] {tn} clip {:?} src={:?} start={} dur={} in={} speed={} rev={} en={} gain={:.2} hold={:?}",
                        c.name,
                        name_of(c.item),
                        c.start.0,
                        c.duration.0,
                        c.source_in.0,
                        c.speed,
                        c.reverse,
                        c.enabled,
                        c.gain_db,
                        c.frame_hold
                    ));
                    for (k, e) in fx.as_array().into_iter().flatten().enumerate() {
                        out.push(format!("[{n}] {tn} clip {:?}@{} fx{k} {e}", c.name, c.start.0));
                    }
                }
                for tr in &t.transitions {
                    out.push(format!("[{n}] {tn} transition {} {} {}", tr.effect.effect, tr.start.0, tr.duration.0));
                }
            }
        }
    }
    out
}

fn diff(label: &str, a: &[String], b: &[String]) -> usize {
    let (sa, sb): (std::collections::BTreeSet<_>, std::collections::BTreeSet<_>) = (a.iter().collect(), b.iter().collect());
    let lost: Vec<_> = sa.difference(&sb).collect();
    let gained: Vec<_> = sb.difference(&sa).collect();
    println!("=== {label}: {} lost, {} changed/new", lost.len(), gained.len());
    for l in &lost {
        println!("  - {}", l.chars().take(400).collect::<String>());
    }
    for l in &gained {
        println!("  + {}", l.chars().take(400).collect::<String>());
    }
    lost.len() + gained.len()
}

fn open(dir: &std::path::Path, path: &std::path::Path) -> Session {
    let mut s = Session { prefs_path: Some(dir.join("prefs.json")), ..Default::default() };
    s.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    s
}

fn try_run(s: &mut Session, what: &str, cmd: &str, p: Value) -> Option<Value> {
    if let Ok(only) = std::env::var("FC_EDITS")
        && !only.split(',').any(|w| w == what)
    {
        return None;
    }
    match s.execute(cmd, p.clone()) {
        Ok(v) => {
            println!("ok   {what}: {cmd}");
            Some(v)
        }
        Err(e) => {
            println!("FAIL {what}: {cmd} {p} -> {e}");
            None
        }
    }
}

#[test]
#[ignore = "needs FC_ROUNDTRIP=<a real .prproj>"]
fn roundtrip_a_real_premiere_project() {
    let Ok(src) = std::env::var("FC_ROUNDTRIP") else { return };
    let dir = tmp_dir("roundtrip");
    let copy = dir.join(std::path::Path::new(&src).file_name().unwrap());
    std::fs::copy(&src, &copy).unwrap();
    // other Premiere projects opened before (their layouts can be borrowed)
    for m in std::env::var("FC_MODELS").unwrap_or_default().split(',').filter(|m| !m.is_empty()) {
        let _ = open(&dir, std::path::Path::new(m));
    }
    let mut s = open(&dir, &copy);
    let opened = summary(&s.project);
    // open → save untouched → open: nothing should change
    s.execute("file.save", json!({})).unwrap();
    let t = open(&dir, &copy);
    diff("save without edits", &opened, &summary(&t.project));

    // edits a user makes
    let mut s = open(&dir, &copy);
    let seq = s.state.active_sequence.unwrap();
    let q = s.project.sequence(seq).unwrap().clone();
    let v: Vec<_> = q.video_tracks.iter().flat_map(|t| t.items.iter()).cloned().collect();
    let a: Vec<_> = q.audio_tracks.iter().flat_map(|t| t.items.iter()).cloned().collect();
    let end = q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| t.items.iter()).map(|c| c.start.0 + c.duration.0).max().unwrap_or(0);
    let fps = q.settings.frame_rate;
    println!("sequence {:?}: {} video clips, {} audio clips, end {end}", s.project.item(seq).unwrap().name, v.len(), a.len());
    if let Some(c) = v.first() {
        try_run(&mut s, "trim out", "timeline.trim", json!({"clip": c.id.0, "edge": "out", "mode": "regular", "deltaFrames": -5}));
        try_run(&mut s, "blur", "effects.apply", json!({"clips": [c.id.0], "effect": "gaussian_blur"}));
        try_run(&mut s, "blur amount", "effects.setParam", json!({"clip": c.id.0, "effect": "gaussian_blur", "param": "blurriness", "value": 20.0}));
        try_run(&mut s, "scale", "effects.setParam", json!({"clip": c.id.0, "effect": "motion", "param": "scale", "value": 150.0}));
        try_run(&mut s, "opacity", "effects.setParam", json!({"clip": c.id.0, "effect": "opacity", "param": "opacity", "value": 50.0}));
    }
    if let Some(c) = v.get(1) {
        try_run(&mut s, "trim in", "timeline.trim", json!({"clip": c.id.0, "edge": "in", "mode": "regular", "deltaFrames": 7}));
        try_run(&mut s, "disable", "clip.enable", json!({"clips": [c.id.0]}));
    }
    if let Some(c) = v.last() {
        let mid = c.start.0 + c.duration.0 / 2;
        try_run(&mut s, "razor", "timeline.razor", json!({"time": mid, "clip": c.id.0}));
        try_run(
            &mut s,
            "move",
            "timeline.move",
            json!({"moves": [{"clip": c.id.0, "track": "V1", "time": end + fps.frame_duration().0 * 30}], "linked": true}),
        );
    }
    if let Some(c) = a.first() {
        try_run(&mut s, "gain", "clip.audioGain", json!({"clips": [c.id.0], "gain": 3.0}));
        try_run(&mut s, "volume fx", "effects.setParam", json!({"clip": c.id.0, "effect": "volume", "param": "level", "value": -6.0}));
        try_run(&mut s, "compressor", "effects.apply", json!({"clips": [c.id.0], "effect": "dynamics"}));
    }
    try_run(&mut s, "seq marker", "markers.add", json!({"time": end / 3, "name": "Round trip"}));
    try_run(&mut s, "tracks", "sequence.addTracks", json!({"video": 1, "audio": 1}));
    if let Some(r) = try_run(&mut s, "adjustment layer", "file.newAdjustmentLayer", json!({"seconds": 3.0})) {
        let item = r.get("item").and_then(Value::as_u64).unwrap_or(0);
        let nv = s.project.sequence(seq).unwrap().video_tracks.len();
        try_run(&mut s, "place adjustment", "timeline.place", json!({"item": item, "track": format!("V{nv}"), "seconds": 1.0}));
    }
    if let Some(r) = try_run(&mut s, "color matte", "file.newColorMatte", json!({"width": 1920, "height": 1080, "color": "#3366cc"})) {
        let item = r.get("item").and_then(Value::as_u64).unwrap_or(0);
        try_run(&mut s, "place matte", "timeline.place", json!({"item": item, "track": "V1", "time": end + 10 * fps.frame_duration().0}));
    }
    try_run(&mut s, "bin", "file.newBin", json!({"name": "FilmCraft Bin"}));
    // a sequence made in FilmCraft, with a clip pasted into it
    if let Some(c) = v.first() {
        try_run(&mut s, "select", "timeline.select", json!({"clips": [c.id.0]}));
        try_run(&mut s, "copy", "edit.copy", json!({}));
        if try_run(&mut s, "new seq", "file.newSequence", json!({"name": "Made in FilmCraft"})).is_some() {
            try_run(&mut s, "paste", "edit.paste", json!({}));
        }
        try_run(&mut s, "back", "sequence.open", json!({"item": seq.0}));
    }
    let edited = summary(&s.project);
    diff("edits made (sanity)", &opened, &edited);
    let r = s.execute("file.save", json!({})).unwrap();
    println!("save report: {}", r["report"]);
    assert!(!s.is_dirty());
    if let Ok(keep) = std::env::var("FC_KEEP") {
        std::fs::copy(&copy, &keep).unwrap();
    }
    let t = open(&dir, &copy);
    let n = diff("after save + reopen (should be empty)", &edited, &summary(&t.project));
    println!("ROUNDTRIP DIFFS: {n}");

    // Sequence Settings on its own (a rename changes every line of the summary)
    let copy2 = dir.join("settings.prproj");
    std::fs::copy(&src, &copy2).unwrap();
    let mut s = open(&dir, &copy2);
    println!(
        "settings before: {:?}",
        s.project
            .sequences()
            .map(|i| (
                i.name.clone(),
                i.as_sequence().map(|q| format!(
                    "{}x{} {:?} {} {:?}",
                    q.settings.width, q.settings.height, q.settings.frame_rate, q.settings.sample_rate, q.settings.audio_master
                ))
            ))
            .collect::<Vec<_>>()
    );
    let p = json!({"name": "Renamed In FilmCraft", "width": 1080, "height": 1920, "sampleRate": 32000, "fps": 25.0, "mix": "Mono"});
    s.execute("sequence.settings", p).unwrap();
    let seq = s.state.active_sequence.unwrap();
    let show = |s: &Session| {
        let it = s.project.item(seq).map(|i| {
            (
                i.name.clone(),
                i.as_sequence().map(|q| format!("{}x{} {:?} {}", q.settings.width, q.settings.height, q.settings.frame_rate, q.settings.sample_rate)),
            )
        });
        format!("{it:?}")
    };
    println!("settings edited: {}", show(&s));
    let r = s.execute("file.save", json!({})).unwrap();
    println!("settings save report: {}", r["report"]);
    let t = open(&dir, &copy2);
    let back = t
        .project
        .sequences()
        .map(|i| {
            (
                i.name.clone(),
                i.as_sequence().map(|q| format!("{}x{} {:?} {}", q.settings.width, q.settings.height, q.settings.frame_rate, q.settings.sample_rate)),
            )
        })
        .collect::<Vec<_>>();
    println!("settings after reopen: {back:?}");
}
