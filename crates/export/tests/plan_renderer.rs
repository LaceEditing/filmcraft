//! Exports composite through a registered plan renderer (the GPU compositor in the apps) when a
//! frame can be rendered exactly that way, and on the CPU otherwise. A renderer that counts its
//! frames and executes plans on the CPU stands in for the GPU. Its own test binary: the
//! registry is process-wide.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use filmcraft_export::{ExportSettings, Format, Progress, composited_frames, export};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_render::SourceMap;
use filmcraft_render::plan::{FramePlan, PlanRenderer, execute_cpu, register_plan_renderer};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

static RENDERED: AtomicU64 = AtomicU64::new(0);

struct Counting;

impl PlanRenderer for Counting {
    fn render_rgba8(&mut self, plan: &FramePlan) -> Option<Vec<u8>> {
        RENDERED.fetch_add(1, Ordering::Relaxed);
        Some(execute_cpu(plan).over_black_rgba8())
    }
    fn name(&self) -> String {
        "counting".into()
    }
}

fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("x");
    let g = GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.5, 0.9, 1.0] }, 320, 180, FrameRate::FPS_24, Tick(2 * TICKS_PER_SECOND));
    let info = g.info().clone();
    let item = p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(g.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let seq = p.new_sequence("s", SequenceSettings { width: 320, height: 180, frame_rate: FrameRate::FPS_24, ..Default::default() }, 1, 1, None);
    let r = FrameRate::FPS_24;
    let v = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(24)), r).unwrap();
    p.sequence_mut(seq).unwrap().video_tracks[0].items.push(v);
    let mut m = SourceMap::default();
    m.0.insert(item, Arc::new(g));
    (Arc::new(p), seq, m)
}

fn run(name: &str, settings: ExportSettings) -> (u64, u64, u64) {
    let (p, seq, m) = project();
    let dir = std::env::temp_dir().join(format!("fc-plan-renderer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name).to_string_lossy().to_string();
    let (g0, c0, r0) = (composited_frames().0, composited_frames().1, RENDERED.load(Ordering::Relaxed));
    let rep = export(&p, seq, &ExportSettings { path, include_audio: false, ..settings }, &m, &Progress::default()).unwrap();
    assert_eq!(rep.frames, 24);
    let (g1, c1) = composited_frames();
    (g1 - g0, c1 - c0, RENDERED.load(Ordering::Relaxed) - r0)
}

#[test]
fn eligible_frames_go_through_the_renderer_and_the_rest_through_the_cpu() {
    // without a renderer: all on the CPU
    let (gpu, cpu, _) = run("none.mov", ExportSettings { format: Format::Mjpeg, ..Default::default() });
    assert_eq!((gpu, cpu), (0, 24));
    register_plan_renderer(|| Some(Box::new(Counting)));
    let (gpu, cpu, rendered) = run("gpu.mov", ExportSettings { format: Format::Mjpeg, ..Default::default() });
    assert_eq!((gpu, cpu, rendered), (24, 0, 24), "every frame composited by the renderer");
    // Export overlays are drawn on the CPU picture: the whole export stays on the CPU
    let mut s = ExportSettings { format: Format::Mjpeg, ..Default::default() };
    s.effects.timecode_overlay.enabled = true;
    let (gpu, cpu, rendered) = run("overlay.mov", s);
    assert_eq!((gpu, cpu, rendered), (0, 24, 0));
    // a frame size other than the sequence's is fitted on the CPU
    let (gpu, cpu, _) = run("fit.mov", ExportSettings { format: Format::Mjpeg, frame_size: Some((200, 200)), ..Default::default() });
    assert_eq!((gpu, cpu), (0, 24));
}
