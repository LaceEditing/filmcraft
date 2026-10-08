//! Decode-order warming for parallel frame rendering.
//!
//! An export renders a batch of frames on all cores at once. Left alone, the workers ask the same
//! media source for neighbouring frames in whatever order they run, and a long-GOP decoder serves
//! each request that lands behind its position by seeking back to the key frame and decoding the
//! group again: with sixteen workers an export got slower than with four. [`warm`] walks the
//! batch's clips in timeline order on one thread first, asking every source for its frames in
//! decoding order; the decoders' frame cache then answers the parallel renders.

use std::collections::HashSet;

use filmcraft_media::FrameRequest;
use filmcraft_project::{ItemId, ItemKind, Project, Sequence, TrackKind};
use filmcraft_render::{MAX_NEST_DEPTH, SourceProvider};
use filmcraft_time::{FrameRate, Tick};

/// Request, in order, every source frame that rendering frames `f0..f1` of `seq` will need.
pub fn warm(project: &Project, seq: ItemId, rate: FrameRate, f0: i64, f1: i64, sources: &dyn SourceProvider) {
    if f1 - f0 < 2 {
        return;
    }
    let Some(s) = project.sequence(seq) else { return };
    let times: Vec<Tick> = (f0..f1).map(|f| rate.tick_of(f)).collect();
    let mut seen = HashSet::new();
    warm_sequence(project, s, &times, sources, 0, &mut seen);
}

fn warm_sequence(project: &Project, seq: &Sequence, times: &[Tick], sources: &dyn SourceProvider, depth: u32, seen: &mut HashSet<(ItemId, i64)>) {
    if depth >= MAX_NEST_DEPTH || times.is_empty() {
        return;
    }
    let (Some(&first), Some(&last)) = (times.first(), times.last()) else { return };
    for track in seq.tracks(TrackKind::Video).iter().filter(|t| t.enabled) {
        for item in track.items.iter().filter(|i| i.enabled && i.end() > first && i.start <= last) {
            let Some(pi) = project.item(item.item) else { continue };
            let shown: Vec<Tick> = times.iter().copied().filter(|&t| t >= item.start && t < item.end()).collect();
            if shown.is_empty() {
                continue;
            }
            match &pi.kind {
                ItemKind::Media(_) | ItemKind::Subclip { .. } => {
                    let Some(src) = sources.source(item.item) else { continue };
                    if src.info().video.is_none() {
                        continue;
                    }
                    for t in shown {
                        let mt = item.source_time_at(t);
                        if seen.insert((item.item, mt.0)) {
                            let _ = src.video_frame(FrameRequest::full(mt));
                        }
                    }
                }
                ItemKind::Sequence(nested) => {
                    let mapped: Vec<Tick> = shown.iter().map(|&t| item.source_time_at(t)).collect();
                    warm_sequence(project, nested, &mapped, sources, depth + 1, seen);
                }
                _ => {}
            }
        }
    }
}
