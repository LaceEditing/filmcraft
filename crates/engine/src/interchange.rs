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

/// The report's warnings (what was left out or approximated), for the user to see.
fn report_warnings(r: &filmcraft_interchange::Report) -> Vec<String> {
    r.entries
        .iter()
        .filter(|e| e.level == filmcraft_interchange::Level::Warning)
        .map(|e| if e.count > 1 { format!("{} (×{})", e.message, e.count) } else { e.message.clone() })
        .collect()
}

/// Tell the user what a Premiere Pro project they saved could not hold (the Events panel, and a
/// notice the app shows).
fn note_unsaved(s: &mut Session, path: &str, notes: Vec<String>) {
    if notes.is_empty() {
        return;
    }
    for n in &notes {
        s.log.push(crate::panels::Level::Warning, "save", format!("{path}: {n}"));
    }
    s.events.push(crate::Event::SaveNotes { path: path.to_string(), notes });
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
        stash_premiere_original(s, path, bytes);
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

/// Whether a Premiere project is a layout kit made for FilmCraft (its file name says "Layout
/// Kit"): one clip per effect at Premiere's default settings.
fn is_layout_kit(path: &str) -> bool {
    std::path::Path::new(path).file_stem().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("layout kit"))
}

fn stash_premiere_original(s: &Session, source: &str, bytes: &[u8]) {
    let Some(dir) = premiere_originals_dir(s) else { return };
    let path = dir.join(format!("{}.prproj", filmcraft_interchange::prproj_fingerprint(bytes)));
    if path.exists() || s.services.export_in_memory() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = s.services.write_file(&path.to_string_lossy(), bytes);
    // a version of a project not seen before: keep its effect and transition layouts
    learn_layouts(s, bytes, is_layout_kit(source));
}

/// The effect and transition layouts FilmCraft has learned from Premiere projects (one small
/// `.prproj` per effect or transition type), for effects added to a project that never used them.
fn premiere_layouts_dir(s: &Session) -> Option<std::path::PathBuf> {
    premiere_originals_dir(s).and_then(|d| d.parent().map(|p| p.join("Premiere Layouts")))
}

/// Keep the layouts of a Premiere project (`at_defaults`: its effects are at Premiere's default
/// settings, as in a layout kit). A layout replaces a kept one when it is better: learned at
/// defaults, or not animated where the kept one is. Returns how many were added or improved.
fn learn_layouts(s: &Session, bytes: &[u8], at_defaults: bool) -> usize {
    let Some(dir) = premiere_layouts_dir(s) else { return 0 };
    if s.services.export_in_memory() {
        return 0;
    }
    let mut n = 0;
    for l in filmcraft_interchange::prproj_layouts(bytes, at_defaults) {
        let path = dir.join(format!("{}.prproj", l.key));
        let better = match std::fs::read(&path) {
            Err(_) => true,
            Ok(old) => {
                let old_defaults = old.windows(26).any(|w| w == b"FilmCraftLayoutAtDefaults=");
                let old_animated = filmcraft_interchange::prproj_layouts(&old, old_defaults).first().is_some_and(|o| o.animated);
                (l.at_defaults && !old_defaults) || (l.at_defaults == old_defaults && old_animated && !l.animated)
            }
        };
        if better {
            let _ = std::fs::create_dir_all(&dir);
            if s.services.write_file(&path.to_string_lossy(), &l.document).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// The `.prproj` files under `root` (or `root` itself), skipping symbolic links (a drive can
/// have links back up to `/`), hidden folders and Premiere's auto-save copies; bounded.
fn prproj_files_under(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    const MAX_DIRS: usize = 200_000;
    const MAX_FILES: usize = 5_000;
    if root.is_file() {
        return if is_prproj(&root.to_string_lossy()) { vec![root.to_path_buf()] } else { Vec::new() };
    }
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = 0;
    while let Some(dir) = stack.pop() {
        dirs += 1;
        if dirs > MAX_DIRS || files.len() >= MAX_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            // the entry's own type: a link is not followed
            let Ok(t) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if t.is_dir() {
                if !name.starts_with('.') && name != "Adobe Premiere Pro Auto-Save" {
                    stack.push(e.path());
                }
            } else if t.is_file() && is_prproj(&name) {
                files.push(e.path());
            }
        }
    }
    files.sort();
    files
}

/// `file.learnPremiereLayouts {path, atDefaults?}`: learn the effect and transition layouts of a
/// Premiere project, or of every `.prproj` under a folder. `atDefaults` (or a file name with
/// "Layout Kit"): the effects are at Premiere's default settings, so effects with settings
/// FilmCraft does not map are learned too.
pub fn learn_layouts_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| EngineError::Other("file.learnPremiereLayouts: need `path`".into()))?.to_string();
    let at_defaults = p.get("atDefaults").and_then(Value::as_bool).unwrap_or(false);
    let files = prproj_files_under(std::path::Path::new(&path));
    let mut learned = 0;
    for f in &files {
        if let Ok(bytes) = std::fs::read(f) {
            learned += learn_layouts(s, &bytes, at_defaults || is_layout_kit(&f.to_string_lossy()));
        }
    }
    let total = premiere_layouts_dir(s).and_then(|d| std::fs::read_dir(d).ok()).map(|e| e.filter_map(|e| e.ok()).count()).unwrap_or(0);
    s.toast(format!("Learned {learned} effect/transition layout(s) from {} Premiere project(s); {total} known", files.len()));
    Ok(json!({"projects": files.len(), "learned": learned, "known": total}))
}

/// How Save treats a project opened from Premiere Pro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PremiereSave {
    /// A new .prproj beside the original (asks where the first time).
    Copy,
    /// Over the original (keeping `<file>.before-filmcraft`).
    Original,
}

/// Whether a path names a Premiere Pro project (`.prproj`).
pub fn is_prproj(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("prproj"))
}

/// Whether any sequence of the open project came from a Premiere Pro project.
fn from_premiere(s: &Session) -> bool {
    s.project.items.values().any(|i| i.as_sequence().is_some() && i.metadata.contains_key(filmcraft_interchange::META_SEQUENCE))
}

/// The Premiere Pro project the open project came from (None: not opened from one).
pub fn premiere_source(s: &Session) -> Option<String> {
    s.project.items.values().find(|i| i.metadata.contains_key(filmcraft_interchange::META_SEQUENCE)).and_then(|i| i.metadata.get(PREMIERE_SOURCE).cloned())
}

/// Whether Save writes a Premiere Pro project here: the project came from one, was not saved as
/// a FilmCraft project, and Settings ▸ General says so.
pub fn premiere_save_mode(s: &Session) -> Option<PremiereSave> {
    if s.path.is_some() {
        return None;
    }
    if premiere_source(s).is_none() {
        // a FilmCraft project saved as a new .prproj (Save As): Save writes that file again
        return s.premiere_save_path.as_ref().map(|_| PremiereSave::Copy);
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
    if wanted.is_empty() && !from_premiere(s) {
        // a project that did not come from Premiere: a new Premiere project
        let adopt = premiere_save_mode(s).is_some();
        return create_prproj(s, &path, adopt);
    }
    if wanted.is_empty() {
        return Err(bad("None of these sequences came from the Premiere Pro project this project was opened from.".into()));
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
    // every sequence unless some were asked for: sequences made here are added to the project
    let written: &[ItemId] = if p.get("sequences").is_some() { &wanted } else { &[] };
    // an effect or transition added here that this project has never used is laid out as in
    // another Premiere project FilmCraft has opened
    let models = premiere_layouts(s);
    let (bytes, report) =
        filmcraft_interchange::export_prproj_with(&s.project, &base, written, std::path::Path::new(&path).parent().and_then(|d| d.to_str()), &models)
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
    let unsaved = report_warnings(&report);
    let verb = if saving { "Saved" } else { "Exported" };
    s.toast(if unsaved.is_empty() {
        format!("{verb} to Premiere Pro project {path}")
    } else {
        format!("{verb} to Premiere Pro project {path}; {} change(s) could not be written into it", unsaved.len())
    });
    note_unsaved(s, &path, unsaved.clone());
    Ok(json!({"path": path, "sequences": wanted.iter().map(|i| i.0).collect::<Vec<_>>(), "report": lines, "unsaved": unsaved}))
}

// ---------------------------------------------------------------------------------------------
// Premiere Pro projects: new ones
// ---------------------------------------------------------------------------------------------

/// Write the open project (which did not come from Premiere Pro) as a new `.prproj` at `path`.
/// `adopt`: Save writes this file from now on (Save As, or Save after one).
pub fn create_prproj(s: &mut Session, path: &str, adopt: bool) -> Result<Value> {
    let bad = |m: String| EngineError::Other(m);
    let dest_dir = std::path::Path::new(path).parent().and_then(|d| d.to_str()).map(str::to_string);
    let path_map = premiere_drives(s);
    let models = if needs_layouts(&s.project) { premiere_layouts(s) } else { Vec::new() };
    let opts = filmcraft_interchange::prproj_new::CreateOptions { dest_dir, path_map: path_map.clone(), models };
    let (bytes, report) = filmcraft_interchange::create_prproj(&s.project, &opts).map_err(|e| bad(e.to_string()))?;
    let mut lines = report_json(&report);
    // a .prproj FilmCraft did not write for this project is kept
    let ours = s.premiere_save_path.as_deref() == Some(path);
    if !ours && s.services.read_file(path).is_ok() {
        let backup = format!("{path}.before-filmcraft");
        if s.services.read_file(&backup).is_err() {
            let old = s.services.read_file(path).map_err(|e| bad(format!("{path}: {e}")))?;
            s.services.write_file(&backup, &old).map_err(|e| bad(format!("{backup}: {e}")))?;
            lines.push(format!("the file that was there was kept as {backup}"));
        }
    }
    s.services.write_file(path, &bytes).map_err(|e| bad(format!("{path}: {e}")))?;
    let has_files = s.project.items.values().any(is_file_media);
    if path_map.is_empty() && has_files {
        lines.push(
            "media paths are written as this computer's paths (open a .prproj saved on the Premiere machine in FilmCraft once so it learns that machine's drives); Premiere finds media stored with the project by its relative path, or asks to locate it".into(),
        );
    }
    if adopt {
        s.path = None;
        s.premiere_save_path = Some(path.to_string());
        s.saved_revision = s.revision;
    }
    let unsaved = report_warnings(&report);
    let verb = if adopt { "Saved" } else { "Exported" };
    s.toast(if unsaved.is_empty() {
        format!("{verb} as Premiere Pro project {path}")
    } else {
        format!("{verb} as Premiere Pro project {path}; {} part(s) of the project could not be written into it", unsaved.len())
    });
    note_unsaved(s, path, unsaved);
    let sequences: Vec<u64> = s.project.items.values().filter(|i| i.as_sequence().is_some()).map(|i| i.id.0).collect();
    Ok(json!({"path": path, "sequences": sequences, "created": true, "report": lines}))
}

/// Whether the project has effects or transitions whose Premiere layout must be borrowed.
fn needs_layouts(p: &filmcraft_project::Project) -> bool {
    p.sequences().filter_map(|i| i.as_sequence()).flat_map(|q| q.all_tracks()).any(|t| {
        !t.transitions.is_empty()
            || t.items.iter().any(|c| c.effects.iter().any(|e| filmcraft_project::effect::find_effect(&e.effect).is_some_and(|d| !d.intrinsic)))
    })
}

/// The kept copies of opened Premiere projects, newest first (at most 16).
fn premiere_originals(s: &Session) -> Vec<std::path::PathBuf> {
    let Some(dir) = premiere_originals_dir(s) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_prproj(&p.to_string_lossy()))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().take(16).map(|(_, p)| p).collect()
}

/// Premiere projects to borrow effect and transition layouts from: the learned layouts first
/// (small), then the kept copies of the projects opened most recently.
fn premiere_layouts(s: &Session) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    if let Some(dir) = premiere_layouts_dir(s)
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        let mut files: Vec<std::path::PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| is_prproj(&p.to_string_lossy())).collect();
        files.sort();
        out.extend(files.iter().filter_map(|p| std::fs::read(p).ok()));
    }
    out.extend(premiere_originals(s).iter().filter_map(|p| std::fs::read(p).ok()));
    out
}

/// (local folder, Premiere drive) pairs, learned from the Premiere projects FilmCraft has opened:
/// a media path such as `L:\Editing\a.mp4` in one of them, found on this computer as
/// `/mnt/LocalDisk/Editing/a.mp4`, maps `/mnt/LocalDisk` to `L:`. Kept until another Premiere
/// project is opened.
fn premiere_drives(s: &mut Session) -> Vec<(String, String)> {
    let originals = premiere_originals(s);
    let names: Vec<String> = originals.iter().map(|p| p.to_string_lossy().to_string()).collect();
    if let Some((known, map)) = &s.premiere_drives
        && *known == names
    {
        return map.clone();
    }
    let mut windows = Vec::new();
    for p in &originals {
        if let Ok(bytes) = std::fs::read(p)
            && let Ok(paths) = filmcraft_interchange::prproj_media_paths(&bytes)
        {
            windows.extend(paths);
        }
    }
    let local: Vec<String> = s
        .project
        .items
        .values()
        .filter_map(|i| match &i.kind {
            ItemKind::Media(m) => match &m.media {
                MediaRef::File { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let map = learn_drives(&windows, &local, &mount_roots(), |p| std::path::Path::new(p).exists());
    s.premiere_drives = Some((names, map.clone()));
    map
}

/// Folders drives are usually mounted at (`/mnt/*`, `/media/*`, `/media/<user>/*`,
/// `/run/media/<user>/*`, `/Volumes/*`).
fn mount_roots() -> Vec<String> {
    let mut out = Vec::new();
    let user = std::env::var("USER").unwrap_or_default();
    let mut bases = vec!["/mnt".to_string(), "/media".to_string(), "/Volumes".to_string()];
    if !user.is_empty() {
        bases.push(format!("/media/{user}"));
        bases.push(format!("/run/media/{user}"));
    }
    for b in bases {
        let Ok(entries) = std::fs::read_dir(&b) else { continue };
        for e in entries.filter_map(|e| e.ok()).take(64) {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                out.push(e.path().to_string_lossy().to_string());
            }
        }
    }
    out
}

/// Which local folder each Windows drive letter of `windows` paths is: the first candidate
/// (folders above the project's media first, then `mounts`) under which a path's remainder
/// exists. Bounded: at most 20 000 lookups.
pub(crate) fn learn_drives(windows: &[String], local_media: &[String], mounts: &[String], exists: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut roots: Vec<String> = Vec::new();
    for p in local_media {
        let mut cur = std::path::Path::new(p).parent();
        while let Some(d) = cur {
            let d_s = d.to_string_lossy().to_string();
            if d_s.len() > 1 && !roots.contains(&d_s) {
                roots.push(d_s);
            }
            cur = d.parent();
        }
    }
    for m in mounts {
        if !roots.contains(m) {
            roots.push(m.clone());
        }
    }
    let mut found: Vec<(String, String)> = Vec::new();
    let mut budget = 20_000usize;
    for w in windows {
        let b = w.as_bytes();
        let drive = match (b.first(), b.get(1), b.get(2)) {
            (Some(l), Some(b':'), Some(b'\\')) if l.is_ascii_alphabetic() => format!("{}:", (*l as char).to_ascii_uppercase()),
            _ => continue,
        };
        if found.iter().any(|(_, d)| *d == drive) {
            continue;
        }
        let rest = w.get(3..).unwrap_or("").replace('\\', "/");
        if rest.is_empty() {
            continue;
        }
        for r in &roots {
            if budget == 0 {
                return found;
            }
            budget -= 1;
            if exists(&format!("{}/{rest}", r.trim_end_matches('/'))) {
                found.push((r.clone(), drive));
                break;
            }
        }
    }
    found
}
