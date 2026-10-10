//! Corpus check for creating Premiere Pro projects (development tool, not shipped).
//!
//! `cargo run --release -p filmcraft-interchange --example prproj_recreate -- OUT_DIR FILE.prproj…`
//!
//! For each project: import it, take off everything that ties it to Premiere, create a new
//! Premiere project from it (borrowing effect and transition layouts from up to eight of the other
//! files given, as from the projects a user opened before), import that and require every
//! sequence's clips back (position, length, source in point, source). New projects are written to
//! OUT_DIR for opening in Premiere. One line per file, then what the report left out.

use std::time::Instant;

use filmcraft_interchange::prproj_new::CreateOptions;
use filmcraft_interchange::{Format, ImportOptions, create_prproj, import_with};
use filmcraft_project::{Project, Sequence, TrackKind};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next() else {
        eprintln!("usage: prproj_recreate OUT_DIR FILE.prproj…");
        std::process::exit(2);
    };
    let _ = std::fs::create_dir_all(&out_dir);
    let paths: Vec<String> = args.collect();
    let (mut ok, mut failed) = (0, 0);
    for path in &paths {
        let t0 = Instant::now();
        match check(path, &paths, &out_dir) {
            Ok((summary, notes)) => {
                ok += 1;
                println!("OK   {:>6.2}s {summary} | {path}", t0.elapsed().as_secs_f64());
                for n in notes {
                    println!("       {n}");
                }
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {:>6.2}s {e} | {path}", t0.elapsed().as_secs_f64());
            }
        }
    }
    println!("== {ok} ok, {failed} failed");
}

fn clips(p: &Project, s: &Sequence, kind: TrackKind) -> Vec<(i64, i64, i64, String)> {
    let mut v: Vec<_> = s
        .tracks(kind)
        .iter()
        .flat_map(|t| t.items.iter().map(|c| (c.start.0, c.duration.0, c.source_in.0, p.item(c.item).map(|i| i.name.clone()).unwrap_or_default())))
        .collect();
    v.sort();
    v
}

fn check(path: &str, all: &[String], out_dir: &str) -> Result<(String, Vec<String>), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string());
    let opts = ImportOptions { base_dir: dir.clone(), ..Default::default() };
    let (mut imp, _) = import_with(&bytes, Format::Prproj, &opts).map_err(|e| format!("import: {e}"))?;
    for it in imp.project.items.values_mut() {
        it.metadata.retain(|k, _| !k.starts_with("premiere."));
    }
    let models = all.iter().filter(|p| p.as_str() != path).take(8).filter_map(|p| std::fs::read(p).ok()).collect();
    let copts = CreateOptions { dest_dir: dir, path_map: vec![("/mnt/LocalDisk".into(), "L:".into())], models };
    let (out, report) = create_prproj(&imp.project, &copts).map_err(|e| format!("create: {e}"))?;
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let _ = std::fs::write(format!("{out_dir}/{stem} (new).prproj"), &out);
    let (back, _) = import_with(&out, Format::Prproj, &opts).map_err(|e| format!("re-import: {e}"))?;
    let mut n = 0;
    for it in imp.project.items.values() {
        let Some(seq) = it.as_sequence() else { continue };
        let other = back.project.items.values().find(|o| o.name == it.name).and_then(|o| o.as_sequence()).ok_or(format!("sequence \"{}\" missing", it.name))?;
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let (a, b) = (clips(&imp.project, seq, kind), clips(&back.project, other, kind));
            if a != b {
                return Err(format!("\"{}\" {kind:?}: {} clips written, {} read back", it.name, a.len(), b.len()));
            }
            n += a.len();
        }
    }
    let notes = report.entries.iter().map(|e| e.message.clone()).collect();
    Ok((format!("{n} clips, {} KiB", out.len() / 1024), notes))
}
