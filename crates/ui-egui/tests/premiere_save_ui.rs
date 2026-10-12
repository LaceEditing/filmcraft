//! Headless UI test of Ctrl+S on a project opened from Premiere Pro (Settings ▸ General ▸ Save a
//! project opened from Premiere Pro, default: a .prproj beside the original): the first Save shows
//! the Premiere Pro project dialog, later ones write there without asking, and the project counts
//! as saved.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

const S: i64 = 254_016_000_000;

/// The engine tests' small Premiere project: one media file, one sequence, one clip per track.
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

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
}

#[test]
fn ctrl_s_saves_a_premiere_project_as_prproj() {
    // the default: Save writes into the Premiere Pro project the sequences came from (a backup
    // of the original is kept), without asking
    let dir = std::env::temp_dir().join(format!("fc-prproj-save-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = Session::default();
    s.prefs_path = Some(dir.join("preferences.json"));
    assert_eq!(s.prefs.general.save_premiere_projects, "premiereOriginal", "the default");
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = channel();
    let mut app = FilmcraftApp::new(s).with_control(rx);
    let n = asked.clone();
    app.hooks.pick_save_as = Some(Box::new(move |_, _, _| {
        n.fetch_add(1, Ordering::Relaxed);
        None
    }));
    let n = asked.clone();
    app.hooks.pick_save = Some(Box::new(move |_| {
        n.fetch_add(1, Ordering::Relaxed);
        None
    }));
    let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    let mut d = Driver { harness, tx };
    d.frames(4);
    // an edit, then Ctrl+S
    d.ok("engine.execute", json!({"command": "playhead.set", "params": {"seconds": 4.0}}));
    d.ok("engine.execute", json!({"command": "sequence.addEditAllTracks", "params": {}}));
    assert!(d.harness.state().session.is_dirty());
    d.ok("ui.key", json!({"key": "Cmd+S"}));
    d.frames(3);
    assert_eq!(asked.load(Ordering::Relaxed), 0, "no dialog: Save knows where");
    let first = std::fs::read_to_string(&src).expect("the .prproj is there");
    assert_ne!(first, prproj(), "written into the original");
    assert_eq!(std::fs::read_to_string(dir.join("show.prproj.before-filmcraft")).unwrap(), prproj(), "the original is kept as a backup");
    assert!(!d.harness.state().session.is_dirty(), "saved");
    // another edit; Ctrl+S writes there again
    d.ok("engine.execute", json!({"command": "playhead.set", "params": {"seconds": 6.0}}));
    d.ok("engine.execute", json!({"command": "sequence.addEditAllTracks", "params": {}}));
    d.ok("ui.key", json!({"key": "Cmd+S"}));
    d.frames(3);
    assert_eq!(asked.load(Ordering::Relaxed), 0, "still not asked");
    assert_ne!(std::fs::read_to_string(&src).unwrap(), first, "written again with the new edit");
    assert!(!d.harness.state().session.is_dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ctrl_s_with_the_copy_preference_asks_once_and_keeps_the_original() {
    let dir = std::env::temp_dir().join(format!("fc-prproj-copy-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("show.prproj");
    std::fs::write(&src, prproj()).unwrap();
    let mut s = Session::default();
    s.prefs_path = Some(dir.join("preferences.json"));
    s.prefs.general.save_premiere_projects = "premiereCopy".into();
    s.execute("file.open", json!({"path": src.to_string_lossy()})).unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = channel();
    let mut app = FilmcraftApp::new(s).with_control(rx);
    let (n, out_dir) = (asked.clone(), dir.clone());
    app.hooks.pick_save_as = Some(Box::new(move |_, exts, suggested| {
        n.fetch_add(1, Ordering::Relaxed);
        assert_eq!((exts, suggested), (&["prproj"][..], "show (FilmCraft).prproj"), "the Premiere Pro project dialog");
        Some(out_dir.join(suggested).to_string_lossy().into_owned())
    }));
    let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    let mut d = Driver { harness, tx };
    d.frames(4);
    let out = dir.join("show (FilmCraft).prproj");
    d.ok("engine.execute", json!({"command": "playhead.set", "params": {"seconds": 4.0}}));
    d.ok("engine.execute", json!({"command": "sequence.addEditAllTracks", "params": {}}));
    d.ok("ui.key", json!({"key": "Cmd+S"}));
    d.frames(3);
    assert_eq!(asked.load(Ordering::Relaxed), 1, "asked where, once");
    let first = std::fs::read(&out).expect("the .prproj was written");
    assert!(!d.harness.state().session.is_dirty(), "saved");
    d.ok("engine.execute", json!({"command": "playhead.set", "params": {"seconds": 6.0}}));
    d.ok("engine.execute", json!({"command": "sequence.addEditAllTracks", "params": {}}));
    d.ok("ui.key", json!({"key": "Cmd+S"}));
    d.frames(3);
    assert_eq!(asked.load(Ordering::Relaxed), 1, "not asked again");
    assert_ne!(std::fs::read(&out).unwrap(), first, "written again with the new edit");
    assert_eq!(std::fs::read_to_string(&src).unwrap(), prproj(), "the original is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
