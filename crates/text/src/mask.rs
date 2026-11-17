//! Coverage-mask operations used for text and shape appearance: strokes (outer / centre / inner)
//! from the outline with mitred, round or bevelled corners, or from an exact Euclidean distance
//! transform (round corners) where there is only a mask, Gaussian-like blur for shadows, offsets.

use crate::raster::{Join, Mask, Path, fill_into, stroke_band};

/// Where a stroke sits relative to the shape's edge (Premiere's stroke types).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StrokeKind {
    #[default]
    Outer,
    Center,
    Inner,
}

const INF: f32 = 1e20;

/// 1-D squared distance transform (Felzenszwalb & Huttenlocher) of `f` into `d`.
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let fq = f[q] + (q * q) as f32;
        let mut s;
        loop {
            let p = v[k];
            s = (fq - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        if s <= z[k] {
            // k == 0: q dominates everything so far
            v[0] = q;
            z[0] = -INF;
            z[1] = INF;
            continue;
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = INF;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        d[q] = (dq * dq + f[p]).min(INF);
    }
}

/// Euclidean distance (in pixels, between pixel centres) from every pixel to the nearest pixel
/// where `seed` is true.
fn edt(w: usize, h: usize, seed: impl Fn(usize) -> bool) -> Vec<f32> {
    let mut g = vec![INF; w * h];
    for (i, v) in g.iter_mut().enumerate() {
        if seed(i) {
            *v = 0.0;
        }
    }
    let n = w.max(h);
    let mut f = vec![0.0f32; n];
    let mut d = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v[..h], &mut z[..h + 1]);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..y * w + w]);
        edt_1d(&f[..w], &mut d[..w], &mut v[..w], &mut z[..w + 1]);
        for x in 0..w {
            g[y * w + x] = d[x].sqrt();
        }
    }
    g
}

/// Approximate signed distance to the shape edge: negative inside, positive outside, with
/// sub-pixel refinement from the coverage values.
pub fn signed_distance(m: &Mask) -> Vec<f32> {
    let (w, h) = (m.w, m.h);
    let outside = edt(w, h, |i| m.a[i] >= 0.5);
    let inside = edt(w, h, |i| m.a[i] < 0.5);
    (0..w * h)
        .map(|i| {
            let c = m.a[i];
            if c > 0.0 && c < 1.0 {
                0.5 - c
            } else if c >= 0.5 {
                -(inside[i] - 0.5).max(0.0)
            } else {
                (outside[i] - 0.5).max(0.0)
            }
        })
        .collect()
}

/// [`stroke`] drawn from the outline itself, so its corners turn as `join` says (a distance
/// transform can only round them). `path` is the shape in the mask's pixels once offset by
/// `(dx, dy)` (as [`crate::raster::fill_into`] takes it) and `cover` its filled coverage.
pub fn stroke_outline(path: &Path, cover: &Mask, (dx, dy): (f32, f32), width: f32, kind: StrokeKind, join: Join) -> Mask {
    let r = if kind == StrokeKind::Center { width / 2.0 } else { width };
    let mut m = Mask::new(cover.w, cover.h);
    fill_into(&stroke_band(path, r, join), &mut m, dx, dy);
    match kind {
        StrokeKind::Outer => m.max_with(cover),
        StrokeKind::Inner => m.a.iter_mut().zip(&cover.a).for_each(|(v, c)| *v = v.min(*c)),
        StrokeKind::Center => {}
    }
    m
}

/// Stroke coverage of width `width` pixels around the shape in `m`.
pub fn stroke(m: &Mask, width: f32, kind: StrokeKind) -> Mask {
    let sd = signed_distance(m);
    let a = sd
        .iter()
        .zip(&m.a)
        .map(|(&d, &c)| {
            let v = match kind {
                StrokeKind::Outer => (width + 0.5 - d).clamp(0.0, 1.0).max(c),
                StrokeKind::Center => (width / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0),
                StrokeKind::Inner => (width + 0.5 + d).clamp(0.0, 1.0).min(c),
            };
            if kind == StrokeKind::Outer { v } else { v.min(1.0) }
        })
        .collect();
    Mask { w: m.w, h: m.h, a }
}

/// How a box-filter pass walks a mask: `lanes` independent lines of `count` samples, `stride`
/// apart within a line and `lane_stride` apart between lines.
#[derive(Clone, Copy)]
struct Lines {
    stride: usize,
    count: usize,
    lanes: usize,
    lane_stride: usize,
}

/// One running-sum box filter of radius `r` along every line of `src` into `dst`.
fn box_pass(src: &[f32], dst: &mut [f32], lines: Lines, r: usize) {
    let Lines { stride, count, lanes, lane_stride } = lines;
    let norm = 1.0 / (2 * r + 1) as f32;
    for l in 0..lanes {
        let base = l * lane_stride;
        let at = |i: isize| -> f32 { if i < 0 || i as usize >= count { 0.0 } else { src[base + i as usize * stride] } };
        let mut acc = 0.0f32;
        for i in -(r as isize)..=(r as isize) {
            acc += at(i);
        }
        for i in 0..count {
            dst[base + i * stride] = acc * norm;
            acc += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
        }
    }
}

/// Blur a mask with three box passes per axis (≈ Gaussian of standard deviation `sigma`).
pub fn blur(m: &Mask, sigma: f32) -> Mask {
    if sigma < 0.3 || m.w == 0 || m.h == 0 {
        return m.clone();
    }
    // box radius for 3 passes ≈ sigma * sqrt(3)
    let r = ((sigma * sigma + 1.0).sqrt().round() as usize).max(1);
    let mut a = m.a.clone();
    let mut b = vec![0.0f32; a.len()];
    for _ in 0..3 {
        box_pass(&a, &mut b, Lines { stride: 1, count: m.w, lanes: m.h, lane_stride: m.w }, r);
        std::mem::swap(&mut a, &mut b);
        box_pass(&a, &mut b, Lines { stride: m.w, count: m.h, lanes: m.w, lane_stride: 1 }, r);
        std::mem::swap(&mut a, &mut b);
    }
    Mask { w: m.w, h: m.h, a }
}

/// Shift a mask by whole pixels (content moving out is dropped).
pub fn offset(m: &Mask, dx: i32, dy: i32) -> Mask {
    let mut o = Mask::new(m.w, m.h);
    o.add(m, dx, dy);
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Path, fill};

    #[test]
    fn distance_transform_is_euclidean() {
        let d = edt(9, 9, |i| i == 4 * 9 + 4);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert!((d[4 * 9 + 7] - 3.0).abs() < 1e-5);
        assert!((d[0] - (32.0f32).sqrt()).abs() < 1e-4);
        let none = edt(3, 3, |_| false);
        assert!(none.iter().all(|&v| v > 1e5));
    }

    #[test]
    fn strokes_sit_outside_centre_and_inside() {
        let m = fill(&Path::rect(10.0, 10.0, 30.0, 30.0), 40, 40, 0.0, 0.0);
        let outer = stroke(&m, 4.0, StrokeKind::Outer);
        assert!(outer.get(7, 20) > 0.99, "outer covers 3px outside");
        assert!(outer.get(4, 20) < 0.01);
        assert!(outer.get(20, 20) > 0.99, "outer includes the fill area");
        let inner = stroke(&m, 4.0, StrokeKind::Inner);
        assert!(inner.get(12, 20) > 0.99 && inner.get(7, 20) < 0.01 && inner.get(20, 20) < 0.01);
        let centre = stroke(&m, 4.0, StrokeKind::Center);
        assert!(centre.get(9, 20) > 0.99 && centre.get(11, 20) > 0.99 && centre.get(20, 20) < 0.01 && centre.get(5, 20) < 0.01);
    }

    /// Drawn from the outline, an outer stroke has square corners with a mitre join and round ones
    /// with a round join (the distance transform's); a concave corner has no hole, and a corner
    /// too sharp for the miter limit is bevelled.
    #[test]
    fn outline_strokes_turn_corners_by_their_join() {
        let sq = Path::rect(20.0, 20.0, 60.0, 60.0);
        let cover = fill(&sq, 80, 80, 0.0, 0.0);
        let miter = stroke_outline(&sq, &cover, (0.0, 0.0), 8.0, StrokeKind::Outer, Join::Miter(2.5));
        let round = stroke_outline(&sq, &cover, (0.0, 0.0), 8.0, StrokeKind::Outer, Join::Round);
        // 6.5 px out on both axes: inside a square corner, 9.2 px from a round one's centre
        assert!(miter.get(13, 13) > 0.99 && round.get(13, 13) < 0.01, "{} {}", miter.get(13, 13), round.get(13, 13));
        assert!(miter.get(40, 14) > 0.99 && miter.get(40, 10) < 0.01 && miter.get(40, 40) > 0.99);
        let dt = stroke(&cover, 8.0, StrokeKind::Outer);
        let worst = dt.a.iter().zip(&round.a).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.6, "round join vs distance transform: {worst}");
        // an L (concave corner at 40, 40): the stroke inside the bend is whole
        let mut l = Path::new();
        for (i, p) in [(20.0, 20.0), (40.0, 20.0), (40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (20.0, 60.0)].iter().enumerate() {
            if i == 0 { l.move_to(p.0, p.1) } else { l.line_to(p.0, p.1) }
        }
        l.close();
        let lc = fill(&l, 80, 80, 0.0, 0.0);
        let ls = stroke_outline(&l, &lc, (0.0, 0.0), 8.0, StrokeKind::Outer, Join::Miter(2.5));
        assert!(ls.get(44, 36) > 0.99 && ls.get(50, 30) < 0.01, "{} {}", ls.get(44, 36), ls.get(50, 30));
        // a 20° spike: a mitre would reach 5.8 widths out, past the limit, so it is cut off
        let mut spike = Path::new();
        spike.move_to(10.0, 40.0);
        spike.line_to(70.0, 40.0 - 60.0 * (10f32.to_radians()).tan());
        spike.line_to(70.0, 40.0 + 60.0 * (10f32.to_radians()).tan());
        spike.close();
        let sc = fill(&spike, 80, 80, 0.0, 0.0);
        let at_tip = |join| stroke_outline(&spike, &sc, (0.0, 0.0), 4.0, StrokeKind::Outer, join);
        // 3 px before the tip: past a bevel, inside a round join; a mitre with a high limit
        // reaches 5 px out too
        assert!(at_tip(Join::Miter(2.5)).get(6, 39) < 0.01, "bevelled, not pointed");
        assert!(at_tip(Join::Round).get(6, 39) > 0.5);
        assert!(at_tip(Join::Miter(10.0)).get(4, 39) > 0.5);
    }

    #[test]
    fn blur_keeps_mass() {
        let m = fill(&Path::rect(20.0, 20.0, 30.0, 30.0), 50, 50, 0.0, 0.0);
        let b = blur(&m, 3.0);
        let (s0, s1): (f32, f32) = (m.a.iter().sum(), b.a.iter().sum());
        assert!((s0 - s1).abs() < 0.5, "{s0} {s1}");
        assert!(b.get(18, 25) > 0.05 && b.get(25, 25) < 1.0);
        let o = offset(&m, 5, -5);
        assert!(o.get(33, 18) > 0.9);
    }
}
