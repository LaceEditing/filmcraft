//! Corpus check for Premiere Pro project import/export (development tool, not shipped).
//!
//! `cargo run --release -p filmcraft-interchange --example prproj_corpus -- OUT_DIR FILE.prproj…`
//!
//! For each project: import; export unchanged and require the identical document; then make
//! deterministic edits to every sequence (delete every 5th clip, split every 7th, nudge every 3rd
//! into free space, set every 4th video clip's Scale and Opacity and audio clip's Volume, disable
//! every 6th) and export again (the exporter verifies its own clips), then import the result and
//! require the values back. Edited projects are written to OUT_DIR for independent checks. One
//! line per file.

use std::time::Instant;

use filmcraft_interchange::{Format, ImportOptions, export_prproj, import_with};
use filmcraft_project::{ClipId, Param, ParamValue, TrackKind};
use filmcraft_time::Tick;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next() else {
        eprintln!("usage: prproj_corpus OUT_DIR FILE.prproj…");
        std::process::exit(2);
    };
    let _ = std::fs::create_dir_all(&out_dir);
    let (mut ok, mut failed) = (0, 0);
    for (n, path) in args.enumerate() {
        let t0 = Instant::now();
        match check(&path, &out_dir, n) {
            Ok(summary) => {
                ok += 1;
                println!("OK   {:>6.2}s {summary} | {path}", t0.elapsed().as_secs_f64());
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {:>6.2}s {e} | {path}", t0.elapsed().as_secs_f64());
            }
        }
    }
    println!("== {ok} ok, {failed} failed");
}

fn check(path: &str, out_dir: &str, n: usize) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    let dir = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string());
    let opts = ImportOptions { base_dir: dir, source_path: Some(path.to_string()), ..Default::default() };
    let (imp, report) = import_with(&bytes, Format::Prproj, &opts).map_err(|e| format!("import: {e}"))?;
    let mut project = imp.project;
    let clips: usize = project.sequences().filter_map(|i| i.as_sequence()).map(|s| s.all_tracks().map(|t| t.items.len()).sum::<usize>()).sum();

    // 1. unchanged → identical
    let (same, _) = export_prproj(&project, &bytes, &[], None).map_err(|e| format!("no-op export: {e}"))?;
    if inflate(&same) != inflate(&bytes) {
        let _ = std::fs::write(format!("{out_dir}/{n:03} NOOP-DIFF.prproj"), &same);
        let (a, b) = (inflate(&same), inflate(&bytes));
        let at = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let ctx = |v: &[u8]| String::from_utf8_lossy(&v[at.saturating_sub(80)..(at + 80).min(v.len())]).to_string();
        return Err(format!("no-op export differs at byte {at}:\n  wrote …{}…\n  orig  …{}…", ctx(&a), ctx(&b)));
    }

    // 2. edits
    let mut next = project.next_id + 1_000_000;
    let mut edits = 0;
    let ids: Vec<_> = project.sequences().map(|i| i.id).collect();
    for sid in ids {
        let Some(seq) = project.sequence_mut(sid) else { continue };
        for kind in [TrackKind::Video, TrackKind::Audio] {
            for t in seq.tracks_mut(kind).iter_mut() {
                t.items.sort_by_key(|c| c.start);
                let mut out = Vec::with_capacity(t.items.len());
                for (i, c) in t.items.drain(..).enumerate() {
                    if i % 5 == 4 {
                        edits += 1;
                        continue; // delete
                    }
                    if i % 7 == 6 && !c.reverse && c.frame_hold.is_none() && c.duration.0 >= 2 * 8_467_200_000 {
                        let half = Tick(c.duration.0 / 2);
                        let mut left = c.clone();
                        left.duration = half;
                        let mut right = c.clone();
                        next += 1;
                        right.id = ClipId(next);
                        right.start = Tick(c.start.0 + half.0);
                        right.duration = Tick(c.duration.0 - half.0);
                        right.source_in = Tick(c.source_in.0 + (half.0 as f64 * c.speed).round() as i64);
                        out.push(left);
                        out.push(right);
                        edits += 1;
                        continue;
                    }
                    let mut c = c;
                    let mut set = |c: &mut filmcraft_project::TrackItem, effect: &str, param: &str, v: f64| {
                        if let Some(e) = c.effects.iter_mut().find(|e| e.effect == effect)
                            && e.params.get(param).is_none_or(|p| p.keyframes.is_empty())
                        {
                            e.params.insert(param.into(), Param::new(ParamValue::Float(v)));
                            edits += 1;
                        }
                    };
                    if kind == TrackKind::Video && i % 4 == 1 {
                        set(&mut c, "motion", "scale", 111.0);
                        set(&mut c, "opacity", "opacity", 61.0);
                    }
                    if kind == TrackKind::Audio && i % 4 == 1 {
                        set(&mut c, "volume", "level", -3.0);
                    }
                    if i % 6 == 5 {
                        c.enabled = !c.enabled;
                        edits += 1;
                    }
                    out.push(c);
                }
                // nudge every 3rd clip one frame later where that leaves no overlap
                for i in (0..out.len()).step_by(3) {
                    let end = out[i].end().0 + 8_467_200_000;
                    if out.get(i + 1).is_none_or(|n| n.start.0 >= end) {
                        out[i].start = Tick(out[i].start.0 + 8_467_200_000);
                        edits += 1;
                    }
                }
                t.items = out;
            }
            // transitions whose clips were deleted go with them
            for t in seq.tracks_mut(kind).iter_mut() {
                let have: std::collections::HashSet<_> = t.items.iter().map(|c| c.id).collect();
                t.transitions.retain(|tr| tr.from.is_none_or(|f| have.contains(&f)) && tr.to.is_none_or(|x| have.contains(&x)));
            }
        }
    }
    // 3. new media: a copy of a file the first sequence uses, under a new name, after its end
    let mut added = 0;
    let first_seq = project.sequences().next().map(|i| i.id);
    if let Some(sid) = first_seq {
        let used = project.sequence(sid).and_then(|s| {
            s.audio_tracks
                .iter()
                .chain(&s.video_tracks)
                .flat_map(|t| t.items.iter())
                .find(|c| {
                    project.item(c.item).is_some_and(|i| matches!(&i.kind, filmcraft_project::ItemKind::Media(m) if m.info.duration.0 > 4 * 254_016_000_000))
                })
                .map(|c| c.item)
        });
        if let Some(src) = used
            && let Some(orig) = project.item(src).cloned()
            && let filmcraft_project::ItemKind::Media(mut m) = orig.kind
            && let filmcraft_project::MediaRef::File { path } = m.media.clone()
        {
            let (dir, name) = path.rsplit_once(['/', '\\']).unwrap_or(("", &path));
            let sep = if path.contains('\\') { "\\" } else { "/" };
            m.media = filmcraft_project::MediaRef::File { path: format!("{dir}{sep}fc-new {name}") };
            m.markers.clear();
            let has_video = m.info.video.is_some();
            let has_audio = m.info.audio.is_some();
            let id = project.add_item(&format!("fc-new {name}"), orig.label, filmcraft_project::ItemKind::Media(m), None);
            let seq = project.sequence_mut(sid);
            if let Some(seq) = seq {
                let end = seq.all_tracks().flat_map(|t| t.items.iter().map(|c| c.end().0)).max().unwrap_or(0) + 254_016_000_000;
                let link = Some(next + 7);
                for (kind, has) in [(TrackKind::Video, has_video), (TrackKind::Audio, has_audio)] {
                    let Some(t) = seq.tracks_mut(kind).first_mut() else { continue };
                    if !has {
                        continue;
                    }
                    let template = t.items.first().cloned();
                    let Some(mut c) = template else { continue };
                    next += 1;
                    c.id = ClipId(next);
                    c.item = id;
                    c.origin = None;
                    c.link = link;
                    c.start = Tick(end);
                    c.duration = Tick(2 * 254_016_000_000);
                    c.source_in = Tick(254_016_000_000);
                    c.speed = 1.0;
                    c.reverse = false;
                    c.frame_hold = None;
                    c.effects =
                        if kind == TrackKind::Video { filmcraft_project::effect::intrinsic_video() } else { filmcraft_project::effect::intrinsic_audio() };
                    t.items.push(c);
                    added += 1;
                }
            }
        }
    }
    let (edited, rep) = export_prproj(&project, &bytes, &[], None).map_err(|e| format!("edited export ({edits} edits): {e}"))?;
    let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let out = format!("{out_dir}/{n:03} {name}.prproj");
    std::fs::write(&out, &edited).map_err(|e| format!("write: {e}"))?;
    let added_ok = rep.mentions("added \"fc-new");
    // 4. the values come back
    let (back, _) = import_with(&edited, Format::Prproj, &opts).map_err(|e| format!("re-import: {e}"))?;
    let tally = |p: &filmcraft_project::Project| {
        let mut n = [0usize; 4];
        let value = |c: &filmcraft_project::TrackItem, e: &str, k: &str| {
            c.effect(e).and_then(|e| e.params.get(k)).filter(|p| p.keyframes.is_empty()).and_then(|p| p.value.as_f64())
        };
        for c in p.sequences().filter_map(|i| i.as_sequence()).flat_map(|s| s.all_tracks()).flat_map(|t| t.items.iter()) {
            n[0] += usize::from(value(c, "motion", "scale").is_some_and(|v| (v - 111.0).abs() < 1e-6));
            n[1] += usize::from(value(c, "opacity", "opacity").is_some_and(|v| (v - 61.0).abs() < 1e-6));
            n[2] += usize::from(value(c, "volume", "level").is_some_and(|v| (v + 3.0).abs() < 0.01));
            n[3] += usize::from(!c.enabled);
        }
        n
    };
    let (want, got) = (tally(&project), tally(&back.project));
    if want != got {
        return Err(format!("values read back differently (scale 111, opacity 61, volume -3 dB, disabled): wrote {want:?}, read {got:?}"));
    }
    Ok(format!(
        "{clips} clips, {edits} edits, {added} new-media clips ({}), import report {} lines, export report {} lines",
        if added == 0 {
            "-"
        } else if added_ok {
            "added"
        } else {
            "NOT ADDED"
        },
        report.entries.len(),
        rep.entries.len()
    ))
}

fn inflate(b: &[u8]) -> Vec<u8> {
    if b.starts_with(&[0x1f, 0x8b]) {
        // skip the 10-byte header (and a file name, as Premiere writes none) and inflate
        let mut pos = 10;
        if b.get(3).is_some_and(|f| f & 0x08 != 0) {
            pos += b[pos..].iter().position(|&c| c == 0).map_or(0, |p| p + 1);
        }
        miniz_oxide::inflate::decompress_to_vec(&b[pos..]).unwrap_or_default()
    } else {
        b.to_vec()
    }
}
