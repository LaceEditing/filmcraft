//! `perf.stats` in the app: the engine's counters (decoder / GOP cache, jobs) plus playback
//! (shown / dropped frames), the frame workers (decode and render ms per job, request hit rate,
//! cache use), UI frame rate and process CPU time. Readable over the control channel
//! (`perf.stats`, or `engine.execute {"command": "perf.stats"}`) and MCP `command_run`.

use serde_json::{Value, json};

use crate::FilmcraftApp;

pub fn stats(app: &FilmcraftApp) -> Value {
    let mut v = filmcraft_engine::perf::stats(&app.session);
    let (shown, dropped) = app.playback.meter.counts();
    let (images, image_bytes, plans, plan_bytes) = app.frames.cache_entries();
    v["playback"] = json!({
        "playing": app.playback.playing,
        "speed": app.playback.speed,
        "shown": shown,
        "dropped": dropped,
        "dropRate": if shown + dropped == 0 { 0.0 } else { dropped as f64 / (shown + dropped) as f64 },
        "resolution": app.ui.program.res.label(),
        "draftDecode": app.session.prefs.playback.draft_decode,
        "misses": app.playback.misses,
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let sr = app.audio.as_ref().map_or(0, |a| a.sample_rate());
        v["playback"]["audio"] = app.playback.audio_stats.to_json(sr);
    }
    let mut frames = app.frames.stats().to_json();
    frames["workers"] = json!(app.frames.workers());
    frames["queued"] = json!(app.frames.queue_len());
    frames["renderCostMs"] = json!(app.frames.render_cost() * 1e3);
    frames["cache"] = json!({"images": images, "imageMB": image_bytes as f64 / 1e6, "plans": plans, "planMB": plan_bytes as f64 / 1e6});
    v["frames"] = frames;
    v["ui"] = json!({"fps": app.fps, "frameMs": if app.fps > 0.0 { 1000.0 / app.fps as f64 } else { 0.0 }, "sections": app.sections.to_json(), "intervalsPlaying": app.intervals_playing});
    v["process"] = json!({"cpuS": crate::frames::process_cpu_time().map(|d| d.as_secs_f64())});
    v
}

/// Time spent per part of a UI frame on the UI thread (`ui.sections` of `perf.stats`): the mean
/// of recent frames and a slowly decaying peak, in ms. Parts: `logic`, `ui` (the whole UI
/// pass), `pump` and every drawn panel (`panel.<id>`).
#[derive(Clone, Debug, Default)]
pub struct Sections(std::collections::BTreeMap<String, (f64, f64, u64, u64)>);

impl Sections {
    pub fn record(&mut self, name: &str, ms: f64) {
        let e = self.0.entry(name.to_string()).or_insert((ms, ms, 0, 0));
        e.0 = e.0 * 0.95 + ms * 0.05;
        e.1 = (e.1 * 0.995).max(ms);
        e.2 += 1;
        // longer than a 60 Hz refresh
        if ms > 1000.0 / 60.0 {
            e.3 += 1;
        }
    }
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0.iter().map(|(k, (mean, peak, n, slow))| (k.clone(), json!({"meanMs": mean, "peakMs": peak, "count": n, "over16ms": slow}))).collect(),
        )
    }
}
