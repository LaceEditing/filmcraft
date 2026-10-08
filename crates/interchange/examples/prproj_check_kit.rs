//! Premiere check kit (development tool, not shipped): for each project, write an unchanged
//! export and an export with a few visible edits, plus a log of exactly what changed, for opening
//! in Premiere Pro.
//!
//! `cargo run --release -p filmcraft-interchange --example prproj_check_kit -- OUT_DIR FILE.prproj…`

use std::fmt::Write as _;

use filmcraft_interchange::{Format, ImportOptions, export_prproj, import_with};
use filmcraft_project::{ClipId, MarkerId, MarkerKind, Param, ParamValue, TrackKind};
use filmcraft_time::Tick;

const S: i64 = 254_016_000_000;

fn tc(t: i64, fps: f64) -> String {
    let frames = ((t as f64 / S as f64) * fps).round() as i64;
    let f = fps.round() as i64;
    format!("{:02}:{:02}:{:02}:{:02}", frames / (3600 * f), frames / (60 * f) % 60, frames / f % 60, frames % f)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next() else {
        eprintln!("usage: prproj_check_kit OUT_DIR FILE.prproj…");
        std::process::exit(2);
    };
    let _ = std::fs::create_dir_all(&out_dir);
    for path in args {
        match kit(&path, &out_dir) {
            Ok(name) => println!("OK   {name}"),
            Err(e) => println!("FAIL {e} | {path}"),
        }
    }
}

fn kit(path: &str, out_dir: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string());
    let opts = ImportOptions { base_dir: dir, source_path: Some(path.to_string()), ..Default::default() };
    let (imp, _) = import_with(&bytes, Format::Prproj, &opts).map_err(|e| e.to_string())?;
    let mut p = imp.project;
    // 1. unchanged
    let (same, _) = export_prproj(&p, &bytes, &[], Some(out_dir)).map_err(|e| e.to_string())?;
    std::fs::write(format!("{out_dir}/{stem} - 1 unchanged.prproj"), same).map_err(|e| e.to_string())?;

    // 2. visible edits on the sequence Premiere had open
    let sid = *imp.sequences.first().ok_or("no sequence")?;
    let seq_name = p.item(sid).map(|i| i.name.clone()).unwrap_or_default();
    let mut log = String::new();
    let _ = writeln!(log, "Project: {path}\nSequence: \"{seq_name}\"\nEdits made in FilmCraft (timecodes are the sequence's):\n");
    let next = p.next_id + 10_000;
    let seq = p.sequence_mut(sid).ok_or("no sequence")?;
    let fps = seq.settings.frame_rate.as_f64();
    // the video track with the most clips of 2 s or more
    let ok = |c: &filmcraft_project::TrackItem| c.duration.0 >= 2 * S && !c.reverse && c.frame_hold.is_none();
    let vt = (0..seq.video_tracks.len()).max_by_key(|i| seq.video_tracks[*i].items.iter().filter(|c| ok(c)).count()).ok_or("no video track")?;
    let vname = format!("V{}", vt + 1);
    let v1 = &mut seq.video_tracks[vt];
    v1.items.sort_by_key(|c| c.start);
    let long: Vec<usize> = v1.items.iter().enumerate().filter(|(_, c)| ok(c)).map(|(i, _)| i).collect();
    if long.len() < 4 {
        return Err("no video track with four clips of 2 s or more".into());
    }
    let (a, b, c, d) = (long[0], long[1], long[2], long[3]);
    // a: zoom
    let ca = &mut v1.items[a];
    if let Some(m) = ca.effects.iter_mut().find(|e| e.effect == "motion") {
        m.params.insert("scale".into(), Param::new(ParamValue::Float(150.0)));
    }
    let _ = writeln!(log, "- {vname} clip \"{}\" at {}: Scale set to 150 (any Scale keyframes on it removed)", ca.name, tc(ca.start.0, fps));
    // b: split in the middle, with a cross dissolve over the cut
    let cb = v1.items[b].clone();
    let half = Tick((cb.duration.0 / 2 / 8_467_200_000) * 8_467_200_000);
    let mut right = cb.clone();
    right.id = ClipId(next + 1);
    right.start = Tick(cb.start.0 + half.0);
    right.duration = Tick(cb.duration.0 - half.0);
    right.source_in = Tick(cb.source_in.0 + (half.0 as f64 * cb.speed).round() as i64);
    v1.items[b].duration = half;
    let _ = writeln!(log, "- {vname} clip \"{}\" at {}: split at {}", cb.name, tc(cb.start.0, fps), tc(right.start.0, fps));
    if let Some(model) = v1.transitions.first().cloned() {
        let mut t = model;
        t.id = filmcraft_project::TransitionId(next + 2);
        t.start = Tick(right.start.0 - S / 2);
        t.duration = Tick(S);
        t.from = Some(v1.items[b].id);
        t.to = Some(right.id);
        t.align = filmcraft_project::TransitionAlign::CenterAtCut;
        let _ = writeln!(log, "- a 1 s {} over that cut", filmcraft_project::effect::find_effect(&t.effect.effect).map(|e| e.name).unwrap_or("transition"));
        v1.transitions.push(t);
    }
    v1.items.push(right);
    // c: trim the end by one second
    let cc = &mut v1.items[c];
    cc.duration = Tick(cc.duration.0 - S);
    let _ = writeln!(log, "- {vname} clip \"{}\" at {}: end trimmed by 1 second (now ends at {})", cc.name, tc(cc.start.0, fps), tc(cc.end().0, fps));
    // d: delete (its linked audio stays, unlinked)
    let cd = v1.items[d].clone();
    v1.items.retain(|x| x.id != cd.id);
    v1.transitions.retain(|t| t.from != Some(cd.id) && t.to != Some(cd.id));
    let _ = writeln!(log, "- {vname} clip \"{}\" at {}–{}: deleted (gap left; its audio stays)", cd.name, tc(cd.start.0, fps), tc(cd.end().0, fps));
    // audio: +3 dB on the first A1 clip
    if let Some(a1) = seq.audio_tracks.first_mut().and_then(|t| t.items.iter_mut().min_by_key(|x| x.start))
        && let Some(v) = a1.effects.iter_mut().find(|e| e.effect == "volume")
    {
        let old = v.params.get("level").and_then(|p| p.value.as_f64()).unwrap_or(0.0);
        v.params.insert("level".into(), Param::new(ParamValue::Float(old + 3.0)));
        let _ = writeln!(log, "- A1 clip \"{}\" at {}: volume {old:+.1} dB → {:+.1} dB", a1.name, tc(a1.start.0, fps), old + 3.0);
    }
    // a marker at 5 s
    let mut m = seq.markers.first().cloned().unwrap_or(filmcraft_project::Marker {
        id: MarkerId(0),
        start: Tick(0),
        duration: Tick(0),
        name: String::new(),
        comment: String::new(),
        kind: MarkerKind::Comment,
        color: filmcraft_project::Label::Green,
    });
    m.id = MarkerId(next + 3);
    m.start = Tick(5 * S);
    m.duration = Tick(0);
    m.name = "FilmCraft check".into();
    m.comment = "added in FilmCraft on Linux".into();
    m.kind = MarkerKind::Comment;
    seq.markers.push(m);
    let _ = writeln!(log, "- sequence marker \"FilmCraft check\" at {}", tc(5 * S, fps));
    let _ = writeln!(log, "\nEverything else should be exactly as in the original project.");

    let (edited, report) = export_prproj(&p, &bytes, &[sid], Some(out_dir)).map_err(|e| e.to_string())?;
    std::fs::write(format!("{out_dir}/{stem} - 2 edited in FilmCraft.prproj"), edited).map_err(|e| e.to_string())?;
    if !report.entries.is_empty() {
        let _ = writeln!(log, "\nExport notes:\n{report}");
    }
    std::fs::write(format!("{out_dir}/{stem} - 2 edited in FilmCraft - what changed.txt"), log).map_err(|e| e.to_string())?;
    let _ = TrackKind::Video;
    Ok(stem)
}
