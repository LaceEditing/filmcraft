//! Interchange documents (CMX 3600 EDL, FCP7 XML, FCPXML, OTIO, AAF, OMF) ↔ the session's project.

use serde_json::{Value, json};

use filmcraft_interchange::{ExportOptions, Format, ImportOptions};
use filmcraft_project::{ItemId, ItemKind, MediaRef};

use crate::{EngineError, Result, Session};

/// Item metadata keys recorded when a Premiere Pro project is opened.
pub use filmcraft_interchange::{META_SEQUENCE as PREMIERE_SEQUENCE, META_SOURCE as PREMIERE_SOURCE};

/// The interchange format of a file, if it is one (by content, with the extension as a hint).
pub fn detect(path: &str, bytes: &[u8]) -> Option<Format> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("edl" | "xml" | "fcpxml" | "otio" | "aaf" | "omf" | "omfi" | "prproj")) {
        return None;
    }
    filmcraft_interchange::detect(bytes, ext.as_deref())
}

fn report_json(r: &filmcraft_interchange::Report) -> Vec<String> {
    r.entries.iter().map(|e| if e.count > 1 { format!("{} (×{})", e.message, e.count) } else { e.message.clone() }).collect()
}

fn is_file_media(item: &filmcraft_project::ProjectItem) -> bool {
    matches!(&item.kind, ItemKind::Media(m) if matches!(m.media, MediaRef::File { .. }))
}

/// Import a document: merge its bins, media and sequences into the project (one undo step), then
/// link each media file that exists on disk. Returns the new sequences.
pub fn import(s: &mut Session, path: &str, bytes: &[u8], format: Format) -> Result<Value> {
    let p = std::path::Path::new(path);
    let opts = ImportOptions {
        base_dir: p.parent().map(|d| d.to_string_lossy().to_string()),
        name: p.file_stem().map(|n| n.to_string_lossy().to_string()),
        source_path: Some(path.to_string()),
        ..Default::default()
    };
    let (fragment, report) = match format {
        // AAF / OMF may embed audio: write it next to the document (the media items point there)
        Format::Aaf | Format::Omf => {
            let r = if format == Format::Aaf { filmcraft_interchange::aaf::import(bytes, &opts) } else { filmcraft_interchange::omf::import(bytes, &opts) };
            let (fragment, extracted, report) = r.map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            for m in extracted {
                if let Some(dir) = std::path::Path::new(&m.path).parent().filter(|_| !s.services.export_in_memory()) {
                    let _ = std::fs::create_dir_all(dir);
                }
                s.services.write_file(&m.path, &m.wav).map_err(|e| EngineError::Other(format!("{}: {e}", m.path)))?;
            }
            (fragment, report)
        }
        _ => filmcraft_interchange::import_with(bytes, format, &opts).map_err(|e| EngineError::Other(format!("{path}: {e}")))?,
    };
    if format == Format::Prproj {
        stash_premiere_original(s, bytes);
    }
    let before: std::collections::HashSet<ItemId> = s.project.items.keys().copied().collect();
    let name = opts.name.clone().unwrap_or_else(|| format.name().to_string());
    let seqs = s.edit(&format!("Import {name}"), |proj, _| {
        let seqs = filmcraft_interchange::merge_into(proj, fragment, None);
        // A document that sets only some Motion parameters leaves the others at their "auto"
        // (NaN) defaults. Resolve them as placing a clip does, for every format. A media file's
        // picture size is only what the document says so far (an EDL says nothing: the importer
        // assumes the sequence size), so its clips' anchors wait for Link Media below.
        proj.resolve_placed_auto_points(|seq| !before.contains(&seq), |source| !is_file_media(source));
        Ok(seqs)
    })?;
    // Link media: probe each new file-backed item that exists.
    let new_media: Vec<(ItemId, String)> = s
        .project
        .items
        .values()
        .filter(|i| !before.contains(&i.id))
        .filter_map(|i| match &i.kind {
            ItemKind::Media(m) => match &m.media {
                MediaRef::File { path } => Some((i.id, path.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let mut linked = 0;
    let mut offline = Vec::new();
    for (id, mpath) in new_media {
        // through the host's random-access reader: a container reads its index now and samples on
        // demand, so a project linking a 30 GB recording does not load 30 GB into memory
        let opened = s.media.open_file(&mpath, &*s.services).ok();
        match opened {
            Some(src) => {
                let info = src.info().clone();
                let identity = crate::relink::identity_of(&*s.services, &mpath).ok();
                let rebase = (format == Format::Edl).then_some(info.start_timecode).flatten();
                s.edit("Link Media", |proj, _| {
                    if let Some(item) = proj.items.get_mut(&id)
                        && let ItemKind::Media(m) = &mut item.kind
                    {
                        let rate = info.video.as_ref().map(|v| v.frame_rate);
                        m.info = info.clone();
                        m.offline = false;
                        m.identity = identity;
                        if let (Some(tc), Some(rate)) = (rebase, rate) {
                            filmcraft_interchange::rebase_source_timecode(proj, id, tc, rate);
                        }
                    }
                    // the picture size is now the file's own: centre the anchors of its clips in it
                    proj.resolve_placed_auto_points(|seq| !before.contains(&seq), |source| source.id == id);
                    Ok(())
                })?;
                s.media.insert_file(id, &mpath, src);
                linked += 1;
            }
            None => offline.push(mpath),
        }
    }
    if let Some(&first) = seqs.first() {
        s.state.active_sequence = Some(first);
        if !s.state.open_sequences.contains(&first) {
            s.state.open_sequences.push(first);
        }
    }
    Ok(json!({
        "format": format.name(),
        "sequences": seqs.iter().map(|i| i.0).collect::<Vec<_>>(),
        "linkedMedia": linked,
        "offlineMedia": offline,
        "report": report_json(&report),
    }))
}

/// `file.exportInterchange {format: "edl"|"xml"|"fcpxml"|"otio"|"aaf"|"omf" = "xml", path, sequence?}`
pub fn export(s: &mut Session, p: &Value) -> Result<Value> {
    // FCP7 XML only when no format is asked for: a format we don't know is an error, not XML.
    // The names are the documented ones (each format's extension, in either case) and nothing
    // else, so the error can list exactly what is accepted.
    let format = match p.get("format") {
        None | Some(Value::Null) => Format::Fcp7Xml,
        Some(v) => v.as_str().and_then(|name| Format::ALL.into_iter().find(|f| f.extension().eq_ignore_ascii_case(name))).ok_or_else(|| {
            let names: Vec<&str> = Format::ALL.iter().map(|f| f.extension()).collect();
            crate::commands::bad("file.exportInterchange", format!("unknown format {v}: use one of {}", names.join(", ")))
        })?,
    };
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| EngineError::Other("need `path`".into()))?.to_string();
    let seq = p.get("sequence").and_then(Value::as_u64).map(ItemId).or(s.state.active_sequence).ok_or(EngineError::NoSequence)?;
    let opts = ExportOptions {
        relative_to: std::path::Path::new(&path).parent().map(|d| d.to_string_lossy().to_string()),
        name: s.project.item(seq).map(|i| i.name.clone()),
        ..Default::default()
    };
    let (bytes, report) = filmcraft_interchange::export(&s.project, seq, format, &opts).map_err(|e| EngineError::Other(e.to_string()))?;
    s.services.write_file(&path, &bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({"path": path, "bytes": bytes.len(), "report": report_json(&report)}))
}

// ---------------------------------------------------------------------------------------------
// Premiere Pro projects: writing back
// ---------------------------------------------------------------------------------------------

/// Where the exact bytes of an opened Premiere project are kept (by fingerprint), so that
/// exporting back patches the version that was opened even after the file changed on disk.
fn premiere_originals_dir(s: &Session) -> Option<std::path::PathBuf> {
    s.persistence
        .as_ref()
        .map(|p| p.data_dir.clone())
        .or_else(|| s.prefs_path.as_ref().and_then(|p| p.parent()).map(std::path::Path::to_path_buf))
        .or_else(crate::autosave::default_data_dir)
        .map(|d| d.join("Premiere Originals"))
}

fn stash_premiere_original(s: &Session, bytes: &[u8]) {
    let Some(dir) = premiere_originals_dir(s) else { return };
    let path = dir.join(format!("{}.prproj", filmcraft_interchange::prproj_fingerprint(bytes)));
    if path.exists() || s.services.export_in_memory() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = s.services.write_file(&path.to_string_lossy(), bytes);
}

/// How Save treats a project opened from Premiere Pro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PremiereSave {
    /// A new .prproj beside the original (asks where the first time).
    Copy,
    /// Over the original (keeping `<file>.before-filmcraft`).
    Original,
}

/// The Premiere Pro project the open project came from (None: not opened from one).
pub fn premiere_source(s: &Session) -> Option<String> {
    s.project.items.values().find(|i| i.metadata.contains_key(filmcraft_interchange::META_SEQUENCE)).and_then(|i| i.metadata.get(PREMIERE_SOURCE).cloned())
}

/// Whether Save writes a Premiere Pro project here: the project came from one, was not saved as
/// a FilmCraft project, and Settings ▸ General says so.
pub fn premiere_save_mode(s: &Session) -> Option<PremiereSave> {
    if s.path.is_some() || premiere_source(s).is_none() {
        return None;
    }
    match s.prefs.general.save_premiere_projects.as_str() {
        "filmcraft" => None,
        "premiereOriginal" => Some(PremiereSave::Original),
        _ => Some(PremiereSave::Copy),
    }
}

/// The .prproj Save would write now without asking (None: it has to ask where).
pub fn premiere_save_target(s: &Session) -> Option<String> {
    match premiere_save_mode(s)? {
        PremiereSave::Original => premiere_source(s),
        PremiereSave::Copy => s.premiere_save_path.clone(),
    }
}

/// Save as a Premiere Pro project (see [`premiere_save_mode`]); None when Save writes a
/// FilmCraft project instead. `path` overrides where it goes.
pub fn save_as_premiere(s: &mut Session, path: Option<&str>) -> Option<Result<Value>> {
    let mode = premiere_save_mode(s)?;
    let target = path.map(str::to_string).or_else(|| premiere_save_target(s));
    let Some(target) = target else {
        return Some(Err(EngineError::Other(
            "file.save: where should the Premiere Pro project go? (pass `path`, or use File ▸ Export ▸ Premiere Pro Project once)".into(),
        )));
    };
    let source = premiere_source(s).unwrap_or_default();
    let overwrite = mode == PremiereSave::Original && target == source;
    Some(export_prproj(s, &json!({"path": target, "overwrite": overwrite})))
}

/// `file.exportPrproj {path, sequences?, force?, overwrite?}`: write sequences opened from a
/// Premiere Pro project back into that project, as a new `.prproj` at `path`.
pub fn export_prproj(s: &mut Session, p: &Value) -> Result<Value> {
    use filmcraft_interchange::{META_FINGERPRINT, META_SEQUENCE, META_SOURCE};
    let bad = |m: String| EngineError::Other(m);
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad("file.exportPrproj: need `path`".into()))?.to_string();
    let force = p.get("force").and_then(Value::as_bool).unwrap_or(false);
    let overwrite = p.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    let wanted: Vec<ItemId> = match p.get("sequences").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).map(ItemId).collect(),
        None => s.project.items.values().filter(|i| i.as_sequence().is_some() && i.metadata.contains_key(META_SEQUENCE)).map(|i| i.id).collect(),
    };
    if wanted.is_empty() {
        return Err(bad("No sequence in this project came from a Premiere Pro project. Open the .prproj in FilmCraft first, or use File ▸ Export ▸ Final Cut Pro XML, which Premiere imports.".into()));
    }
    let meta = |id: &ItemId, k: &str| s.project.item(*id).and_then(|i| i.metadata.get(k).cloned());
    let fingerprints: std::collections::BTreeSet<String> = wanted.iter().filter_map(|i| meta(i, META_FINGERPRINT)).collect();
    if fingerprints.len() != 1 {
        return Err(bad("These sequences come from different Premiere projects; export each project's sequences separately.".into()));
    }
    let fp = fingerprints.into_iter().next().unwrap_or_default();
    let source = wanted.iter().find_map(|i| meta(i, META_SOURCE)).unwrap_or_default();
    // the version that was opened: kept copy, else the source file if it is unchanged
    let stash = premiere_originals_dir(s).map(|d| d.join(format!("{fp}.prproj")));
    let mut warnings = Vec::new();
    let base = match stash.as_ref().and_then(|p| std::fs::read(p).ok()) {
        Some(b) if filmcraft_interchange::prproj_fingerprint(&b) == fp => b,
        _ => {
            let b = s
                .services
                .read_file(&source)
                .map_err(|e| bad(format!("The Premiere project this came from ({source}) is needed to write back into it: {e}")))?;
            if filmcraft_interchange::prproj_fingerprint(&b) != fp {
                if !force {
                    return Err(bad(format!(
                        "{source} changed after it was opened in FilmCraft (saved again in Premiere?). Writing into the newer file could put these edits on the wrong clips. Re-open it in FilmCraft, or export with force."
                    )));
                }
                warnings.push(format!("{source} changed after it was opened; the edits were written into the newer file (force)"));
            }
            b
        }
    };
    let same_file = |a: &str, b: &str| {
        let (a, b) = (std::path::Path::new(a), std::path::Path::new(b));
        a == b || (a.canonicalize().ok().is_some() && a.canonicalize().ok() == b.canonicalize().ok())
    };
    if !source.is_empty() && same_file(&path, &source) {
        if !overwrite {
            return Err(bad(format!(
                "{path} is the Premiere project the sequences came from. Save to a new file, or export with overwrite (a backup of the original is kept)."
            )));
        }
        let backup = format!("{source}.before-filmcraft");
        if !std::path::Path::new(&backup).exists() {
            let original = s.services.read_file(&source).map_err(|e| bad(format!("{source}: {e}")))?;
            s.services.write_file(&backup, &original).map_err(|e| bad(format!("{backup}: {e}")))?;
            warnings.push(format!("the original was kept as {backup}"));
        }
    }
    let (bytes, report) = filmcraft_interchange::export_prproj(&s.project, &base, &wanted, std::path::Path::new(&path).parent().and_then(|d| d.to_str()))
        .map_err(|e| bad(e.to_string()))?;
    s.services.write_file(&path, &bytes).map_err(|e| bad(format!("{path}: {e}")))?;
    // Saving to Premiere Pro (Settings ▸ General): this is where Save writes from now on, and
    // the project counts as saved
    let saving = premiere_save_mode(s).is_some();
    if saving {
        if !same_file(&path, &source) {
            s.premiere_save_path = Some(path.clone());
        }
        s.saved_revision = s.revision;
    }
    let mut lines = report_json(&report);
    lines.extend(warnings);
    let n = lines.len();
    let verb = if saving { "Saved" } else { "Exported" };
    s.toast(if n == 0 {
        format!("{verb} to Premiere Pro project {path}")
    } else {
        format!("{verb} to Premiere Pro project {path} ({n} note(s) in the export report)")
    });
    Ok(json!({"path": path, "sequences": wanted.iter().map(|i| i.0).collect::<Vec<_>>(), "report": lines}))
}
