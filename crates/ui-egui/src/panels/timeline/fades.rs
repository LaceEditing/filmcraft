//! A clip's own fades in the Timeline (Premiere's fade handles): the handles at a clip's top
//! corners, their hit test, and the fade drawn as its gain line.

use filmcraft_project::{Track, Transition};

use super::layout::Layout;
use super::{ClipId, Color32, FadeCurve, FilmcraftApp, Grab, Pos2, Rect, Sequence, Stroke, Tick, Tool, TrackId, TrackItem, grab_at, pos2, vec2};

/// How far an audio fade's curve bends per pixel the fade handle is dragged up (Premiere: about
/// 1.7, up is positive; Round 5 A7).
pub(super) const FADE_BEND_PER_PX: f32 = 1.7;

/// Size of a fade handle (px): a small square in the clip's top corner, as in Premiere. Only
/// the square itself takes a press, so the rest of the edge below it still trims (the edge is
/// grabbed from 8 px below the clip's top, above where a transition on a cut takes the press).
const FADE_HANDLE: f32 = 7.0;

/// The fade handle at `pos` (Premiere's fade handles; only the Selection tool drags them): its
/// track, clip and edge. None on a locked track.
pub(super) fn fade_at(seq: &Sequence, layout: &Layout, pos: Pos2) -> Option<(TrackId, ClipId, filmcraft_edit::Edge)> {
    if !layout.content.contains(pos) {
        return None;
    }
    let row = layout.row_at(pos.y)?;
    let tr = seq.track(row.track).filter(|t| !t.locked)?;
    tr.items
        .iter()
        .filter(|i| layout.x_of(i.start) <= pos.x + FADE_HANDLE && layout.x_of(i.end()) >= pos.x - FADE_HANDLE)
        .find_map(|it| fade_handles(tr, it, layout, row.rect, None).into_iter().find(|(_, r)| r.contains(pos)).map(|(edge, _)| (row.track, it.id, edge)))
}

/// [`grab_at`] as the Timeline shows it: with Timeline settings ▸ Show Audio / Video Keyframes
/// off the clips' Volume / Opacity lines are hidden, and a press on one is a press on its clip.
/// With Alt held a press on an Opacity line is one on its clip too: Alt-click selects one side
/// of a link and Alt-drag copies the clip, wherever on the clip they start.
pub(super) fn grab_shown(app: &FilmcraftApp, seq: &Sequence, layout: &Layout, pos: Pos2, tool: Tool, mods: egui::Modifiers, roll_ripple: bool) -> Grab {
    match grab_at(seq, layout, pos, tool, mods, roll_ripple) {
        g @ Grab::Volume { .. } if !app.ui.timeline.show_audio_keyframes => Grab::Other(g.hit()),
        g @ Grab::Opacity { .. } if !app.ui.timeline.show_video_keyframes || mods.alt => Grab::Other(g.hit()),
        g => g,
    }
}

/// A clip's own fade at an edge: the one-sided transition on it there (Premiere's fade handles).
pub(super) fn clip_fade(tr: &Track, clip: ClipId, edge: filmcraft_edit::Edge) -> Option<&Transition> {
    tr.transitions.iter().find(|t| match edge {
        filmcraft_edit::Edge::In => t.to == Some(clip) && t.from.is_none(),
        filmcraft_edit::Edge::Out => t.from == Some(clip) && t.to.is_none(),
    })
}

/// Whether an edge of a clip is joined to its neighbour by a transition (no fade handle there).
fn edge_joined(tr: &Track, clip: ClipId, edge: filmcraft_edit::Edge) -> bool {
    tr.transitions.iter().any(|t| match edge {
        filmcraft_edit::Edge::In => t.to == Some(clip) && t.from.is_some(),
        filmcraft_edit::Edge::Out => t.from == Some(clip) && t.to.is_some(),
    })
}

/// The fade handles of a clip in a track row: at its top corners, moved in by its fades
/// (`drag`: a fade handle being dragged and its length). None on a clip too narrow for them or
/// an edge joined to the next clip.
pub(super) fn fade_handles(
    tr: &Track,
    it: &TrackItem,
    layout: &Layout,
    row: Rect,
    drag: Option<(ClipId, filmcraft_edit::Edge, Tick)>,
) -> Vec<(filmcraft_edit::Edge, Rect)> {
    let (x0, x1) = (layout.x_of(it.start), layout.x_of(it.end()));
    if x1 - x0 < 4.0 * FADE_HANDLE || row.height() < 2.0 * FADE_HANDLE {
        return Vec::new();
    }
    let len = |edge| match drag {
        Some((c, e, d)) if c == it.id && e == edge => d,
        _ => clip_fade(tr, it.id, edge).map_or(Tick::ZERO, |t| t.duration),
    };
    // flush with the top of the clip (drawn 1 px below its track's top)
    let y = row.min.y + 1.0;
    let mut out = Vec::new();
    for edge in [filmcraft_edit::Edge::In, filmcraft_edit::Edge::Out] {
        if edge_joined(tr, it.id, edge) {
            continue;
        }
        let x = match edge {
            filmcraft_edit::Edge::In => (layout.x_of(it.start + len(edge)) - FADE_HANDLE / 2.0).max(x0 + 1.0),
            filmcraft_edit::Edge::Out => (layout.x_of(it.end() - len(edge)) - FADE_HANDLE / 2.0).min(x1 - FADE_HANDLE - 1.0),
        };
        out.push((edge, Rect::from_min_size(pos2(x, y), vec2(FADE_HANDLE, FADE_HANDLE))));
    }
    out
}

/// A clip fade drawn as Premiere draws one: its gain line across the clip (full at the clip side,
/// silent at the edge) in the fade's own curve, with the faded part shaded above it.
pub(super) fn draw_fade(p: &egui::Painter, body: Rect, x_full: f32, x_edge: f32, curve: FadeCurve) {
    if (x_full - x_edge).abs() < 1.0 {
        return;
    }
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let f = i as f32 / n as f32;
            // from full (f = 0) to silent (f = 1)
            let g = curve.fade_out(f);
            pos2(x_full + (x_edge - x_full) * f, body.max.y - (body.height() - 2.0) * g)
        })
        .collect();
    // the faded part shaded above the line (in strips: the region is not convex)
    for w in pts.windows(2) {
        let [a, b] = w else { continue };
        let (l, r) = if a.x <= b.x { (*a, *b) } else { (*b, *a) };
        p.add(egui::Shape::convex_polygon(vec![pos2(l.x, body.min.y), pos2(r.x, body.min.y), r, l], Color32::from_black_alpha(80), Stroke::NONE));
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::WHITE)));
}
