//! Colour, keying and matte effects.

use filmcraft_color::{hsl_to_rgb, rgb_to_hsl};
use filmcraft_project::EffectInstance;
use filmcraft_project::effect::LIGHT_IDS;
use rayon::prelude::*;

use super::*;

/// Lighting Effects: up to five directional / omni / spot lights, ambience, gloss and a bump map
/// from the layer's own luminance.
pub fn lighting(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    struct L {
        kind: u32,
        color: [f32; 3],
        c: Vec2,
        major: f64,
        minor: f64,
        rot: (f64, f64),
        intensity: f32,
        focus: f32,
    }
    let diag = (img.w as f64).hypot(img.h as f64);
    let lights: Vec<L> = LIGHT_IDS
        .iter()
        .map(|ids| L {
            kind: chv(e, ids[0]),
            color: lin(cv(e, ids[1], cx)),
            c: pv(e, ids[2], cx, img),
            major: (fv(e, ids[3], cx) as f64 / 100.0 * diag).max(1.0),
            minor: (fv(e, ids[4], cx) as f64 / 100.0 * diag).max(1.0),
            rot: (fv(e, ids[5], cx) as f64).to_radians().sin_cos(),
            intensity: fv(e, ids[6], cx) / 20.0,
            focus: fv(e, ids[7], cx) / 100.0,
        })
        .filter(|l| l.kind != 0)
        .collect();
    let amb = lin(cv(e, "ambient_color", cx)).map(|v| v * fv(e, "ambient", cx) / 100.0);
    let gloss = fv(e, "gloss", cx) / 100.0;
    let material = (fv(e, "material", cx) / 100.0 + 1.0) / 2.0;
    let exposure = 2f32.powf(fv(e, "exposure", cx) / 100.0 * 2.0);
    let bump = fv(e, "bump_height", cx) / 100.0 * 4.0;
    let white_high = bv(e, "white_high");
    let src = img.clone();
    let lum = |x: isize, y: isize| {
        let p = src.get_clamped(x, y);
        let l = luma(Image::unpremul(p));
        if white_high { l } else { 1.0 - l }
    };
    let w = img.w;
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let p = &mut row[x * 4..x * 4 + 4];
            if p[3] <= 1e-6 {
                continue;
            }
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let (gx, gy) = if bump != 0.0 {
                (
                    (lum(x as isize + 1, y as isize) - lum(x as isize - 1, y as isize)) * bump,
                    (lum(x as isize, y as isize + 1) - lum(x as isize, y as isize - 1)) * bump,
                )
            } else {
                (0.0, 0.0)
            };
            let mut light = amb;
            let mut spec = [0.0f32; 3];
            for l in &lights {
                let (dx, dy) = (px - l.c.x, py - l.c.y);
                let (s, c) = l.rot;
                let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
                let fall = match l.kind {
                    1 => 1.0,
                    2 => {
                        let d = ((dx * dx + dy * dy).sqrt() / l.major) as f32;
                        (1.0 - d).max(0.0).powi(2)
                    }
                    _ => {
                        let d = ((u / l.major).powi(2) + (v / l.minor).powi(2)).sqrt() as f32;
                        1.0 - smoothstep(l.focus * 0.95, 1.0, d)
                    }
                };
                // bump shading: light direction in the image plane (towards the light), tilted 45°
                let (lx, ly) = if l.kind == 1 { (l.rot.1 as f32, l.rot.0 as f32) } else { (-dx as f32, -dy as f32) };
                let ln = (lx * lx + ly * ly).sqrt().max(1e-6);
                let shade = (1.0 - (gx * lx + gy * ly) / ln).max(0.0);
                for k in 0..3 {
                    light[k] += l.color[k] * l.intensity * fall * shade;
                    spec[k] += l.color[k] * gloss.max(0.0) * fall.powi(8) * l.intensity.max(0.0);
                }
            }
            let c = Image::unpremul([p[0], p[1], p[2], p[3]]);
            for k in 0..3 {
                let s = spec[k] * (1.0 - material) + spec[k] * c[k] * material;
                let v = ((c[k] * light[k] + s) * exposure).max(0.0);
                p[k] = v * p[3];
            }
        }
    });
}

/// ASC CDL (v1.2): clamp(in × slope + offset)^power per channel, then saturation (Rec. 709 luma),
/// on display-encoded values (`gpufx::FxOp::AscCdl`).
pub(crate) fn cdl(c: [f32; 3], s: [f32; 3], o: [f32; 3], p: [f32; 3], sat: f32) -> [f32; 3] {
    let mut v = [0.0; 3];
    for k in 0..3 {
        v[k] = (c[k] * s[k] + o[k]).clamp(0.0, 1.0).powf(p[k].max(0.0));
    }
    let l = luma(v);
    v.map(|q| (l + sat * (q - l)).clamp(0.0, 1.0))
}

#[allow(unused_imports)]
pub(crate) use crate::gpufx::limit;

/// Logo Cutout: removes a flat white/black/custom background by un-multiplying it — the alpha of
/// a pixel is how far it is from the background towards the gamut edge, and the colour is
/// recovered as if it had been composited over that background.
pub fn logo_cutout(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let bg = match chv(e, "background") {
        0 => [1.0; 3],
        1 => [0.0; 3],
        _ => {
            let c = cv(e, "color", cx);
            [c[0], c[1], c[2]]
        }
    };
    let th = fv(e, "threshold", cx) / 100.0;
    let soft = (fv(e, "softness", cx) / 100.0).max(1e-3);
    let unmult = bv(e, "unmultiply");
    let invert = bv(e, "invert");
    img.px.par_chunks_mut(4).for_each(|p| {
        if p[3] <= 1e-6 {
            return;
        }
        let v = enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
        let mut a0 = 0.0f32;
        for k in 0..3 {
            let d = v[k] - bg[k];
            let room = if d >= 0.0 { 1.0 - bg[k] } else { bg[k] };
            if room > 1e-6 {
                a0 = a0.max(d.abs() / room);
            }
        }
        let a0 = a0.clamp(0.0, 1.0);
        let mut a = ((a0 - th) / ((1.0 - th).max(1e-3) * soft)).clamp(0.0, 1.0);
        if invert {
            a = 1.0 - a;
        }
        let col = if unmult && a0 > 1e-4 && !invert { [0, 1, 2].map(|k| (bg[k] + (v[k] - bg[k]) / a0).clamp(0.0, 1.0)) } else { v };
        let l = dec(col);
        let na = a * p[3];
        p.copy_from_slice(&[l[0] * na, l[1] * na, l[2] * na, na]);
    });
}

/// Ultra Key, calibrated against renders Premiere Pro made of the same frames (preview files of
/// a keyed clip, matched to its source frames; see `crates/render/README.md`). Observed: the matte
/// is a per-pixel function of the colour (no spatial filtering unless Choke / Soften are set) of
/// the green excess over the other two channels, `key channel − max(others)` (in display-encoded
/// R′G′B′, normalised by the key colour's own excess), opaque up to a point set by Tolerance and
/// then falling linearly with a slope set by Transparency and weighted by brightness relative to
/// the key colour (Shadow keeps darker pixels opaque, Highlight brighter ones). Pedestal cuts the
/// faint end, Contrast steepens around Mid Point. Spill suppression takes the excess (plus a margin
/// set by Range) off the key channel, scaled by Spill, and adds the lost brightness back to all
/// three channels (Luma), so a despilled edge keeps its brightness instead of going dark;
/// Desaturate greys nearly transparent pixels. The constants reproduce Premiere's matte to about
/// 2/255 on average (13/255 on edge pixels) and its foreground colour to 2/255 at the settings
/// observed; how other settings scale follows Adobe's descriptions of the controls.
pub fn ultra_key(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let kc = cv(e, "key_color", cx);
    let key = [kc[0].clamp(0.0, 1.0), kc[1].clamp(0.0, 1.0), kc[2].clamp(0.0, 1.0)];
    // the key's dominant channel is the one keyed (green for a green screen)
    let dom = if key[1] >= key[0] && key[1] >= key[2] {
        1
    } else if key[2] >= key[0] {
        2
    } else {
        0
    };
    let (o1, o2) = ((dom + 1) % 3, (dom + 2) % 3);
    let excess = |v: [f32; 3]| v[dom] - v[o1].max(v[o2]);
    // a key without a dominant channel (grey) keys nothing meaningful: keep the division sane
    let kd = excess(key).max(0.02);
    let yk = luma(key).clamp(0.02, 1.0);
    // Default, Relaxed and Aggressive use their preset values (see `ultra_key_setting`); Custom
    // uses the values as set
    let preset = filmcraft_project::effect::ultra_key_setting(chv(e, "setting"));
    let pf = |id: &str, fallback: f32| preset.and_then(|rows| rows.iter().find(|(k, _)| *k == id).map(|(_, v)| *v as f32)).unwrap_or(fallback);
    let pct = |id: &str| (pf(id, fv(e, id, cx)) / 100.0).clamp(0.0, 1.0);
    let transparency = pct("transparency");
    let highlight = pct("highlight");
    let shadow = pct("shadow");
    let tolerance = pct("tolerance");
    let pedestal = (pct("pedestal") * 0.25).min(0.9);
    let slope = 0.5876 / (1.0 - 0.99 * transparency);
    let x0 = -0.093 * tolerance;
    let dark_pow = 5.24 * shadow;
    let bright_pow = 0.2975 + 3.0 * highlight;
    let output = chv(e, "output");
    // matte
    let n = img.w * img.h;
    let mut alpha = vec![1.0f32; n];
    alpha.par_iter_mut().zip(img.px.par_chunks(4)).for_each(|(a, p)| {
        if p[3] <= 1e-6 {
            *a = 0.0;
            return;
        }
        let v = enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
        let xn = excess(v) / kd;
        let y = luma(v);
        let w = if y < yk { (y.max(0.0) / yk).powf(dark_pow) } else { (yk / y).powf(bright_pow) };
        let raw = (1.0 - slope * w * (xn - x0 * w.powf(0.2427))).clamp(0.0, 1.0);
        *a = ((raw - pedestal) / (1.0 - pedestal)).clamp(0.0, 1.0);
    });
    // cleanup (Premiere: Choke is an erode and Soften a box blur, both up to 9 × 9 at 100)
    let choke = pct("choke");
    let soften = pct("soften");
    let contrast = pct("contrast");
    let mid = pct("mid_point");
    if choke > 0.0 {
        alpha = erode_plane(&alpha, img.w, img.h, choke * 4.0 * cx.px_scale);
    }
    if soften > 0.0 {
        alpha = box_plane(&alpha, img.w, img.h, soften * 4.0 * cx.px_scale);
    }
    if contrast > 0.0 {
        let g = 1.0 + 3.0 * contrast;
        alpha.par_iter_mut().for_each(|a| *a = ((*a - mid) * g + mid).clamp(0.0, 1.0));
    }
    // spill suppression + colour correction on the foreground
    let spill = pf("spill", fv(e, "spill", cx)).max(0.0) / 50.0;
    let range = pct("range");
    let luma_back = pf("spill_luma", fv(e, "spill_luma", cx)).max(0.0) / 50.0;
    let desat = pct("desaturate");
    let weights = [0.2126f32, 0.7152, 0.0722];
    let dom_weight = weights[dom];
    let (cc_s, cc_h, cc_l) = (fv(e, "cc_saturation", cx) / 100.0, fv(e, "cc_hue", cx) / 360.0, fv(e, "cc_luminance", cx) / 100.0);
    let cc = (cc_s - 1.0).abs() > 1e-4 || cc_h.abs() > 1e-6 || (cc_l - 1.0).abs() > 1e-4;
    img.px.par_chunks_mut(4).zip(alpha.par_iter()).for_each(|(p, &a)| {
        let src_a = p[3];
        let mut v = enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
        let r = (excess(v) + 0.034 * range).max(0.0) * 0.97 * spill;
        if r > 0.0 {
            v[dom] -= r;
            let back = r * dom_weight * 1.086 * luma_back;
            v = v.map(|q| q + back);
        }
        if desat > 0.0 && a < 1.0 {
            let t = (desat * 0.5 * (1.0 - a)).clamp(0.0, 1.0);
            let l = luma(v);
            v = v.map(|q| q + (l - q) * t);
        }
        if cc {
            let mut h = rgb_to_hsl(v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0), v[2].clamp(0.0, 1.0));
            h[0] = (h[0] + cc_h).rem_euclid(1.0);
            h[1] = (h[1] * cc_s).clamp(0.0, 1.0);
            h[2] = (h[2] * cc_l).clamp(0.0, 1.0);
            v = hsl_to_rgb(h[0], h[1], h[2]);
        }
        let l = dec(v);
        match output {
            1 => {
                let g = filmcraft_color::srgb_to_linear(a);
                p.copy_from_slice(&[g * src_a, g * src_a, g * src_a, src_a]);
            }
            2 => p.copy_from_slice(&[l[0] * src_a, l[1] * src_a, l[2] * src_a, src_a]),
            _ => {
                let na = a * src_a;
                p.copy_from_slice(&[l[0] * na, l[1] * na, l[2] * na, na]);
            }
        }
    });
}

/// Separable min filter over `plane` with a fractional radius (`r` = 2.5 blends the 2- and
/// 3-pixel erosions halfway).
pub(crate) fn erode_plane(plane: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let r = r.clamp(0.0, 64.0);
    let lo = r.floor() as usize;
    let frac = r - lo as f32;
    let a = min_plane(plane, w, h, lo);
    if frac <= 1e-4 {
        return a;
    }
    let b = min_plane(plane, w, h, lo + 1);
    a.iter().zip(&b).map(|(x, y)| x + (y - x) * frac).collect()
}

fn min_plane(plane: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return plane.to_vec();
    }
    let mut tmp = vec![0.0f32; plane.len()];
    tmp.par_chunks_mut(w).zip(plane.par_chunks(w)).for_each(|(out, row)| {
        for (x, o) in out.iter_mut().enumerate() {
            let (s, e) = (x.saturating_sub(r), (x + r).min(w - 1));
            *o = row.get(s..=e).unwrap_or_default().iter().copied().fold(f32::INFINITY, f32::min);
        }
    });
    let mut out = vec![0.0f32; plane.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (s, e) = (y.saturating_sub(r), (y + r).min(h - 1));
        for (x, o) in row.iter_mut().enumerate() {
            *o = (s..=e).filter_map(|yy| tmp.get(yy * w + x).copied()).fold(f32::INFINITY, f32::min);
        }
    });
    out
}

/// Separable box blur with a fractional radius (the outermost taps weigh the fraction).
pub(crate) fn box_plane(plane: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let r = r.clamp(0.0, 64.0);
    if r <= 1e-4 || w == 0 || h == 0 {
        return plane.to_vec();
    }
    let n = r.ceil() as isize;
    let frac = r - (n - 1) as f32;
    let weight = |d: isize| if d.abs() == n { frac } else { 1.0 };
    let total: f32 = (-n..=n).map(weight).sum();
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let mut s = 0.0;
                for d in -n..=n {
                    let (xx, yy) = if horizontal {
                        ((x as isize + d).clamp(0, w as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + d).clamp(0, h as isize - 1) as usize)
                    };
                    s += src.get(yy * w + xx).copied().unwrap_or(0.0) * weight(d);
                }
                *o = s / total;
            }
        });
        out
    };
    let a = pass(plane, true);
    pass(&a, false)
}

/// Track Matte Key: multiplies the layer by another video track's alpha or luma (sampled where
/// the layer lands in the frame).
pub fn track_matte(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let idx = chv(e, "matte") as usize;
    if idx == 0 {
        return;
    }
    let Some(env) = cx.env else { return };
    let Some(matte) = env.track(idx - 1) else { return };
    apply_matte(img, &matte, &env.layer_to_output(), chv(e, "composite") == 1, bv(e, "reverse"));
}

/// Multiply `img` (layer pixels) by `matte` (output pixels) mapped through `m`.
pub(crate) fn apply_matte(img: &mut Image, matte: &Image, m: &Affine, use_luma: bool, reverse: bool) {
    let w = img.w;
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let o = m.apply(Vec2::new(x as f64 + 0.5, y as f64 + 0.5));
            let s = matte.sample_bilinear(o.x as f32, o.y as f32);
            let mut v = if use_luma { filmcraft_color::linear_to_srgb(luma([s[0], s[1], s[2]]).clamp(0.0, 1.0)) } else { s[3] };
            if reverse {
                v = 1.0 - v;
            }
            for c in &mut row[x * 4..x * 4 + 4] {
                *c *= v.clamp(0.0, 1.0);
            }
        }
    });
}

/// Cineon Converter (Kodak Cineon printing-density model, film gamma 0.6, 0.002 density / code).
pub fn cineon(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let mode = chv(e, "conversion");
    let black = fv(e, "black10", cx);
    let white = fv(e, "white10", cx).max(black + 1.0);
    let ib = fv(e, "black_internal", cx) / 255.0;
    let iw = fv(e, "white_internal", cx) / 255.0;
    let gamma = fv(e, "gamma", cx).max(0.1);
    let roll = fv(e, "rolloff", cx) / 255.0;
    img.map_rgb(|c, _, _| dec(enc(c).map(|v| cineon_value(v, mode, black, white, ib, iw, gamma, roll))));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cineon_value(v: f32, mode: u32, black: f32, white: f32, ib: f32, iw: f32, gamma: f32, roll: f32) -> f32 {
    let k = 0.002 / 0.6;
    let offset = 10f32.powf((black - white) * k);
    match mode {
        0 => {
            let code = v * 1023.0;
            let linv = ((10f32.powf((code - white) * k) - offset) / (1.0 - offset)).max(0.0);
            let mut o = linv.powf(1.0 / gamma);
            o = ib + o * (iw - ib);
            let r0 = 1.0 - roll;
            if roll > 1e-4 && o > r0 {
                o = r0 + roll * (1.0 - (-(o - r0) / roll).exp());
            }
            o.clamp(0.0, 1.0)
        }
        1 => {
            let d = ((v - ib) / (iw - ib).max(1e-4)).clamp(0.0, 1.0);
            let linv = d.powf(gamma);
            let code = white + (linv * (1.0 - offset) + offset).max(1e-6).log10() / k;
            (code / 1023.0).clamp(0.0, 1.0)
        }
        _ => {
            let code = v * 1023.0;
            (((code - black) / (white - black)) * (685.0 - 95.0) + 95.0) / 1023.0
        }
    }
}

/// Noise (and Noise (Legacy)): per-frame random noise added in display space, with grain size.
pub fn noise(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let amt = fv(e, "amount", cx) / 100.0;
    if amt <= 0.0 {
        return;
    }
    let colored = bv(e, "color");
    let clip = bv(e, "clip");
    let grain = (fv(e, "grain_size", cx).max(1.0) * cx.px_scale).max(1.0);
    let frame = crate::effects::noise_frame_seed(cx);
    let w = img.w;
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let p = &mut row[x * 4..x * 4 + 4];
            let a = p[3];
            if a <= 0.0 {
                continue;
            }
            let (gx, gy) = ((x as f32 / grain) as usize, (y as f32 / grain) as usize);
            let h = |z: u64| crate::effects::hash3(gx, gy, z) - 0.5;
            let n0 = h(frame);
            let ns = if colored { [n0, h(frame + 7777), h(frame + 99_991)] } else { [n0; 3] };
            let c = enc([p[0] / a, p[1] / a, p[2] / a]);
            let mut o = [c[0] + ns[0] * amt, c[1] + ns[1] * amt, c[2] + ns[2] * amt];
            if clip {
                o = o.map(|v| v.clamp(0.0, 1.0));
            }
            let o = dec(o);
            p[0] = o[0] * a;
            p[1] = o[1] * a;
            p[2] = o[2] * a;
        }
    });
}
