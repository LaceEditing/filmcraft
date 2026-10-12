//! File ▸ Open a Premiere Pro project, edit, File ▸ Export ▸ Premiere Pro Project, end to end.

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

const S: i64 = 254_016_000_000;

/// A small Premiere project: one media file, one sequence with one video and one audio clip.
fn prproj() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
  <RootProjectItem ObjectUID="root"><ProjectItem><Name>Root Bin</Name></ProjectItem>
    <ProjectItemContainer><Items><Item Index="0" ObjectURef="mpi"/><Item Index="1" ObjectURef="seqpi"/></Items></ProjectItemContainer></RootProjectItem>
  <ClipProjectItem ObjectUID="mpi"><ProjectItem><Name>take.mov</Name></ProjectItem><MasterClip ObjectURef="mmc"/></ClipProjectItem>
  <MasterClip ObjectUID="mmc"><Clips><Clip Index="0" ObjectRef="10"/></Clips></MasterClip>
  <VideoClip ObjectID="10"><Clip><Source ObjectRef="11"/></Clip></VideoClip>
  <VideoMediaSource ObjectID="11"><MediaSource><Media ObjectURef="media1"/></MediaSource></VideoMediaSource>
  <AudioMediaSource ObjectID="12"><MediaSource><Media ObjectURef="media1"/></MediaSource></AudioMediaSource>
  <Media ObjectUID="media1"><VideoStream ObjectRef="14"/><RelativePath>.\take.mov</RelativePath><FilePath>D:\Edit\take.mov</FilePath></Media>
  <VideoStream ObjectID="14"><FrameRate>8467200000</FrameRate><Duration>{d}</Duration><FrameRect>0,0,1920,1080</FrameRect></VideoStream>
  <ClipProjectItem ObjectUID="seqpi"><ProjectItem><Name>Edit</Name></ProjectItem><MasterClip ObjectURef="smc"/></ClipProjectItem>
  <MasterClip ObjectUID="smc"><Clips><Clip Index="0" ObjectRef="20"/></Clips></MasterClip>
  <VideoClip ObjectID="20"><Clip><Source ObjectRef="21"/></Clip></VideoClip>
  <VideoSequenceSource ObjectID="21"><SequenceSource><Sequence ObjectURef="seq1"/></SequenceSource><OriginalDuration>{e}</OriginalDuration></VideoSequenceSource>
  <Sequence ObjectUID="seq1"><Name>Edit</Name><TrackGroups><TrackGroup><Second ObjectRef="30"/></TrackGroup><TrackGroup><Second ObjectRef="31"/></TrackGroup></TrackGroups></Sequence>
  <VideoTrackGroup ObjectID="30"><TrackGroup><Tracks><Track Index="0" ObjectURef="vt1"/></Tracks><FrameRate>8467200000</FrameRate><NextTrackID>2</NextTrackID></TrackGroup><FrameRect>0,0,1920,1080</FrameRect></VideoTrackGroup>
  <AudioTrackGroup ObjectID="31"><TrackGroup><Tracks><Track Index="0" ObjectURef="at1"/></Tracks><FrameRate>5292000</FrameRate></TrackGroup></AudioTrackGroup>
  <VideoClipTrack ObjectUID="vt1"><ClipTrack><Track><ID>1</ID><Index>0</Index></Track><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="40"/></TrackItems><Index>0</Index></ClipItems><TransitionItems><TrackItems></TrackItems><Index>0</Index></TransitionItems></ClipTrack></VideoClipTrack>
  <AudioClipTrack ObjectUID="at1"><ClipTrack><Track><ID>1</ID><Index>0</Index></Track><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="50"/></TrackItems></ClipItems></ClipTrack></AudioClipTrack>
  <VideoClipTrackItem ObjectID="40"><ClipTrackItem><TrackItem><Start>0</Start><End>{e}</End></TrackItem><SubClip ObjectRef="41"/></ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="41"><Clip ObjectRef="42"/><Name>take.mov</Name></SubClip>
  <VideoClip ObjectID="42"><Clip><Source ObjectRef="11"/><InPoint>0</InPoint><OutPoint>{e}</OutPoint></Clip></VideoClip>
  <AudioClipTrackItem ObjectID="50"><ClipTrackItem><TrackItem><Start>0</Start><End>{e}</End></TrackItem><SubClip ObjectRef="51"/></ClipTrackItem></AudioClipTrackItem>
  <SubClip ObjectID="51"><Clip ObjectRef="52"/><Name>take.mov</Name></SubClip>
  <AudioClip ObjectID="52"><Clip><Source ObjectRef="12"/><InPoint>0</InPoint><OutPoint>{e}</OutPoint></Clip></AudioClip>
</PremiereData>
"#,
        d = 60 * S,
        e = 10 * S
    )
}

fn session(dir: &std::path::Path) -> Session {
    // keep the copy of the opened project inside the test directory
    Session { prefs_path: Some(dir.join("prefs.json")), ..Default::default() }
}

fn trim_first_video_clip(s: &mut Session) {
    let mut p = (*s.project).clone();
    let seq_id = p.sequences().next().map(|i| i.id).unwrap();
    let seq = p.sequence_mut(seq_id).unwrap();
    let c = &mut seq.video_tracks[0].items[0];
    c.duration = filmcraft_time::Tick(4 * S);
    s.project = std::sync::Arc::new(p);
}

#[test]
fn open_edit_and_export_a_premiere_project() {
    let dir = tmp_dir("prproj-roundtrip");
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(&dir);
    let r = s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    assert_eq!(r["format"], "Premiere Pro Project");
    assert!(s.path.is_none(), "opened as a new, unsaved project");
    assert!(dir.join("Premiere Originals").read_dir().unwrap().count() == 1, "the opened version is kept");

    trim_first_video_clip(&mut s);
    let out = dir.join("show (FilmCraft).prproj");
    let r = s.execute("file.exportPrproj", json!({"path": out.to_string_lossy()})).unwrap();
    assert_eq!(r["path"], out.to_string_lossy().as_ref());
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains(&format!("<End>{}</End>", 4 * S)));
    assert_eq!(std::fs::read_to_string(&src).unwrap(), prproj(), "the source is never touched");

    // the exported project opens again, with the edit
    let mut t = session(&dir);
    t.execute("file.open", json!({"path": out.to_string_lossy()})).unwrap();
    let seq = t.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(seq.video_tracks[0].items[0].duration.0, 4 * S);
    assert_eq!(seq.audio_tracks[0].items[0].duration.0, 10 * S);
}

#[test]
fn exporting_over_the_source_needs_overwrite_and_keeps_a_backup() {
    let dir = tmp_dir("prproj-overwrite");
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(&dir);
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    trim_first_video_clip(&mut s);
    let e = s.execute("file.exportPrproj", json!({"path": src.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("overwrite"), "{e}");
    s.execute("file.exportPrproj", json!({"path": src.to_string_lossy(), "overwrite": true})).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("show.prproj.before-filmcraft")).unwrap(), prproj());
    assert!(std::fs::read_to_string(&src).unwrap().contains(&format!("<End>{}</End>", 4 * S)));
}

#[test]
fn a_source_saved_again_elsewhere_is_not_patched_silently() {
    let dir = tmp_dir("prproj-changed");
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(&dir);
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    // the kept copy is gone (another machine) and Premiere saved the source again
    std::fs::remove_dir_all(dir.join("Premiere Originals")).unwrap();
    std::fs::write(&src, prproj().replace("<Name>Edit</Name>", "<Name>Edit v2</Name>")).unwrap();
    let out = dir.join("out.prproj");
    let e = s.execute("file.exportPrproj", json!({"path": out.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("changed after it was opened"), "{e}");
    let r = s.execute("file.exportPrproj", json!({"path": out.to_string_lossy(), "force": true})).unwrap();
    assert!(r["report"].to_string().contains("changed after it was opened"));
}

/// A FilmCraft project with a media file: the Premiere project opened, then everything that ties
/// it to Premiere taken off (as if the media had been imported into FilmCraft).
fn filmcraft_project(dir: &std::path::Path) -> Session {
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(dir);
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    let mut p = (*s.project).clone();
    for it in p.items.values_mut() {
        it.metadata.retain(|k, _| !k.starts_with("premiere."));
    }
    s.project = std::sync::Arc::new(p);
    assert!(crate::interchange::premiere_source(&s).is_none());
    s
}

fn clips_of(path: &std::path::Path, dir: &std::path::Path) -> Vec<(i64, i64, i64)> {
    let mut t = session(dir);
    t.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let seq = t.project.sequences().find(|i| i.name == "Edit").and_then(|i| i.as_sequence()).unwrap().clone();
    seq.all_tracks().flat_map(|tr| tr.items.iter().map(|c| (c.start.0, c.duration.0, c.source_in.0))).collect()
}

/// File ▸ Export ▸ Premiere Pro Project and Save As … .prproj on a project that did not come from
/// Premiere: a new Premiere project; after Save As, Save writes it again.
#[test]
fn a_filmcraft_project_becomes_a_new_premiere_project() {
    let dir = tmp_dir("prproj-create");
    let mut s = filmcraft_project(&dir);
    assert!(crate::find_command("file.exportPrproj").map(|c| (c.enabled)(&s)).unwrap().is_ok());

    // Export: written, nothing about Save changes
    let out = dir.join("new.prproj");
    let r = s.execute("file.exportPrproj", json!({"path": out.to_string_lossy()})).unwrap();
    assert_eq!(r["created"], true, "{r}");
    assert!(std::fs::read(&out).unwrap().starts_with(&[0x1f, 0x8b]), "gzip, as Premiere saves");
    assert_eq!(clips_of(&out, &dir), vec![(0, 10 * S, 0), (0, 10 * S, 0)]);
    assert!(s.premiere_save_path.is_none());

    // Save As .prproj: saved there, and Save writes it again
    let saved = dir.join("saved.prproj");
    s.execute("file.saveAs", json!({"path": saved.to_string_lossy()})).unwrap();
    assert_eq!(s.premiere_save_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
    assert!(s.path.is_none() && !s.is_dirty());
    set_first_video_clip(&mut s, 4);
    assert!(s.is_dirty());
    s.execute("file.save", json!({})).unwrap();
    assert!(!s.is_dirty());
    assert!(clips_of(&saved, &dir).contains(&(0, 4 * S, 0)));
    assert!(!dir.join("saved.prproj.before-filmcraft").exists(), "its own file is simply replaced");

    // a .prproj FilmCraft did not write is kept
    let mut t = filmcraft_project(&dir);
    let foreign = dir.join("show.prproj");
    t.execute("file.saveCopy", json!({"path": foreign.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("show.prproj.before-filmcraft")).unwrap(), prproj());
    assert!(t.premiere_save_path.is_none(), "Save a Copy does not change where Save goes");
}

#[test]
fn a_project_of_generated_media_reports_what_it_left_out() {
    let dir = tmp_dir("prproj-demo");
    let mut s = Session { prefs_path: Some(dir.join("prefs.json")), ..Default::default() };
    s.execute("file.openDemoProject", json!({})).unwrap();
    let out = dir.join("demo.prproj");
    let r = s.execute("file.exportPrproj", json!({"path": out.to_string_lossy()})).unwrap();
    assert!(r["report"].to_string().contains("generated media"), "{r}");
    assert!(out.exists());
}

#[test]
fn premiere_drives_are_learned_from_files_both_machines_have() {
    let on_disk = ["/mnt/LocalDisk/Editing/Show/a.mp4", "/run/media/me/Drive/Music/b.wav"];
    let exists = |p: &str| on_disk.contains(&p);
    let windows =
        ["L:\\Editing\\Show\\a.mp4".to_string(), "M:\\Music\\b.wav".to_string(), "\\\\server\\share\\c.mp4".to_string(), "Q:\\nowhere.mp4".to_string()];
    let local = ["/mnt/LocalDisk/Editing/New/clip.mp4".to_string()];
    let map = crate::interchange::learn_drives(&windows, &local, &["/run/media/me/Drive".to_string()], exists);
    assert_eq!(map, vec![("/mnt/LocalDisk".to_string(), "L:".to_string()), ("/run/media/me/Drive".to_string(), "M:".to_string())]);
}

fn set_first_video_clip(s: &mut Session, secs: i64) {
    s.edit("Trim", |p, _| {
        let seq_id = p.sequences().next().map(|i| i.id).unwrap();
        p.sequence_mut(seq_id).unwrap().video_tracks[0].items[0].duration = filmcraft_time::Tick(secs * S);
        Ok(())
    })
    .unwrap();
}

/// Settings ▸ General ▸ Save a project opened from Premiere Pro: Save (Ctrl+S) writes over the
/// original with a backup (the default), a .prproj beside the original (the first Save asks
/// where, later ones write there), or a FilmCraft project.
#[test]
fn save_writes_a_premiere_project_for_projects_opened_from_one() {
    let dir = tmp_dir("prproj-save");
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(&dir);
    assert_eq!(s.prefs.general.save_premiere_projects, "premiereOriginal", "the default: Ctrl+S saves the project it came from");
    // beside the original: the first Save asks where
    s.prefs.general.save_premiere_projects = "premiereCopy".into();
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    set_first_video_clip(&mut s, 4);
    assert!(s.is_dirty());
    // the first Save needs to know where (the app shows the Premiere Pro project dialog)
    let e = s.execute("file.save", json!({})).unwrap_err().to_string();
    assert!(e.contains("Premiere Pro project"), "{e}");
    let out = dir.join("show (FilmCraft).prproj");
    s.execute("file.save", json!({"path": out.to_string_lossy()})).unwrap();
    assert!(std::fs::read_to_string(&out).unwrap().contains(&format!("<End>{}</End>", 4 * S)));
    assert!(!s.is_dirty(), "saved");
    assert!(s.path.is_none(), "still not a FilmCraft project");
    // later Saves write there without asking
    set_first_video_clip(&mut s, 3);
    assert!(s.is_dirty());
    s.execute("file.save", json!({})).unwrap();
    assert!(std::fs::read_to_string(&out).unwrap().contains(&format!("<End>{}</End>", 3 * S)));
    assert!(!s.is_dirty());
    assert_eq!(std::fs::read_to_string(&src).unwrap(), prproj(), "the original is never touched");

    // over the original: a backup is kept
    let mut s = session(&dir);
    s.prefs.general.save_premiere_projects = "premiereOriginal".into();
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    set_first_video_clip(&mut s, 5);
    s.execute("file.save", json!({})).unwrap();
    assert!(std::fs::read_to_string(&src).unwrap().contains(&format!("<End>{}</End>", 5 * S)));
    assert_eq!(std::fs::read_to_string(dir.join("show.prproj.before-filmcraft")).unwrap(), prproj());
    assert!(!s.is_dirty());

    // as a FilmCraft project: Save asks for a .fcproj, as before
    let mut s = session(&dir);
    s.prefs.general.save_premiere_projects = "filmcraft".into();
    s.execute("file.open", json!({"path": out.to_string_lossy()})).unwrap();
    let e = s.execute("file.save", json!({})).unwrap_err().to_string();
    assert!(e.contains("Save As"), "{e}");
    let fc = dir.join("show.fcproj");
    s.execute("file.save", json!({"path": fc.to_string_lossy()})).unwrap();
    assert!(filmcraft_format::decode(&std::fs::read(&fc).unwrap()).is_ok());
}

/// Save into a Premiere Pro project says what it could not write (the Events panel, an event
/// the app shows as a notice, and `unsaved` in the result); a complete save says nothing.
#[test]
fn save_says_what_the_premiere_project_could_not_hold() {
    let dir = tmp_dir("prproj-save-notes");
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = session(&dir);
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    set_first_video_clip(&mut s, 4);
    s.drain_events();
    let r = s.execute("file.save", json!({})).unwrap();
    assert_eq!(r["unsaved"], json!([]), "{r}");
    assert!(!s.drain_events().iter().any(|e| matches!(e, crate::Event::SaveNotes { .. })));
    // a sequence made here cannot go into the Premiere project yet
    s.execute("file.newSequence", json!({"name": "Made Here"})).unwrap();
    let r = s.execute("file.save", json!({})).unwrap();
    let unsaved = r["unsaved"].as_array().cloned().unwrap_or_default();
    assert!(unsaved.iter().any(|n| n.as_str().is_some_and(|n| n.contains("\"Made Here\""))), "{r}");
    let notes = s.drain_events().into_iter().find_map(|e| match e {
        crate::Event::SaveNotes { path, notes } => Some((path, notes)),
        _ => None,
    });
    let (path, notes) = notes.expect("a SaveNotes event");
    assert_eq!(path, src.to_string_lossy());
    assert_eq!(notes.len(), unsaved.len());
    assert!(s.log.entries.iter().any(|e| e.level == crate::panels::Level::Warning && e.message.contains("Made Here")));
}
