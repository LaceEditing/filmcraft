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

#[test]
fn export_needs_a_project_that_came_from_premiere() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    assert!(s.execute("file.exportPrproj", json!({"path": "/nonexistent/x.prproj"})).is_err());
}
