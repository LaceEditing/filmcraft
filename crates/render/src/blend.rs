//! Blend modes (Premiere's 27), composited on premultiplied linear-light images.
//!
//! Normal is computed in linear light. The other modes are defined on display-encoded values (as
//! artists expect from every editing app), so their math runs on sRGB-encoded straight colour, and,
//! as in Premiere (with Composite in Linear Color on or off), the result is mixed with the
//! backdrop by the top layer's alpha on those encoded values too, then converted back to linear.
//! Formulas follow the W3C Compositing and Blending spec, with Premiere's measured differences:
//! Darker/Lighter Color compare Rec. 709 luma, the other non-separable modes exact Rec. 601 luma,
//! Soft Light is Photoshop's (√b), Hard Mix is
//! `b + s ≥ 1`, 0 ÷ 0 is 0, and Linear Dodge, Linear Light, Subtract and Divide are not clamped
//! before the mix. Where nothing is under a layer in one of those modes (every mode but Normal and
//! Dissolve), Premiere draws the layer as it is, ignoring its opacity.

use filmcraft_color::{linear_to_srgb, srgb_to_linear};
use rayon::prelude::*;

use crate::image::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    pub const ALL: [Blend; 27] = [
        Blend::Normal,
        Blend::Dissolve,
        Blend::Darken,
        Blend::Multiply,
        Blend::ColorBurn,
        Blend::LinearBurn,
        Blend::DarkerColor,
        Blend::Lighten,
        Blend::Screen,
        Blend::ColorDodge,
        Blend::LinearDodge,
        Blend::LighterColor,
        Blend::Overlay,
        Blend::SoftLight,
        Blend::HardLight,
        Blend::VividLight,
        Blend::LinearLight,
        Blend::PinLight,
        Blend::HardMix,
        Blend::Difference,
        Blend::Exclusion,
        Blend::Subtract,
        Blend::Divide,
        Blend::Hue,
        Blend::Saturation,
        Blend::Color,
        Blend::Luminosity,
    ];
    /// Position in [`Blend::ALL`] (= `filmcraft_project::effect::BLEND_MODES`).
    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|b| *b == self).unwrap_or(0) as u32
    }
    /// Whether compositing needs the destination colour (every mode but Normal and Dissolve,
    /// which are plain "over" per pixel).
    pub fn reads_destination(self) -> bool {
        !matches!(self, Blend::Normal | Blend::Dissolve)
    }
    /// Index into `filmcraft_project::effect::BLEND_MODES`.
    pub fn from_index(i: u32) -> Blend {
        Self::ALL.get(i as usize).copied().unwrap_or(Blend::Normal)
    }
}

/// Hard Mix's tolerance below `b + s = 1`.
pub const HARD_MIX_SLACK: f32 = 1e-3;
/// Divide's smallest divisor.
pub const DIVIDE_FLOOR: f32 = 1e-4;
/// How near black or white a backdrop counts as it for Color Burn and Color Dodge (an 8-bit
/// white comes back from linear a hair under 1).
pub const EDGE_SLACK: f32 = 1e-4;

/// Whether a mode's result goes into the alpha mix unclamped (as Premiere's does).
pub fn unclamped(mode: Blend) -> bool {
    matches!(mode, Blend::LinearDodge | Blend::LinearLight | Blend::Subtract | Blend::Divide)
}

#[inline]
fn sep(mode: Blend, b: f32, s: f32) -> f32 {
    match mode {
        Blend::Darken => b.min(s),
        Blend::Multiply => b * s,
        Blend::ColorBurn => {
            if b >= 1.0 - EDGE_SLACK {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        Blend::LinearBurn => (b + s - 1.0).max(0.0),
        Blend::Lighten => b.max(s),
        Blend::Screen => b + s - b * s,
        Blend::ColorDodge => {
            if b <= EDGE_SLACK {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        Blend::LinearDodge => b + s,
        Blend::Overlay => sep(Blend::HardLight, s, b),
        Blend::HardLight => {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                sep(Blend::Screen, b, 2.0 * s - 1.0)
            }
        }
        // Premiere's (and Photoshop's): √b throughout, not the W3C's cubic below ¼
        Blend::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                b + (2.0 * s - 1.0) * (b.max(0.0).sqrt() - b)
            }
        }
        Blend::VividLight => {
            if s <= 0.5 {
                sep(Blend::ColorBurn, b, 2.0 * s)
            } else {
                sep(Blend::ColorDodge, b, 2.0 * s - 1.0)
            }
        }
        Blend::LinearLight => b + 2.0 * s - 1.0,
        Blend::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        // (8-bit inputs summing to exactly 255 are white: allow for the trip through linear)
        Blend::HardMix => {
            if b + s >= 1.0 - HARD_MIX_SLACK {
                1.0
            } else {
                0.0
            }
        }
        Blend::Difference => (b - s).abs(),
        Blend::Exclusion => b + s - 2.0 * b * s,
        Blend::Subtract => b - s,
        // 0 ÷ 0 is 0, anything else ÷ 0 is far above white
        Blend::Divide => b / s.max(DIVIDE_FLOOR),
        _ => s,
    }
}

/// The non-separable modes' luminosity: Rec. 601 luma, exactly (Premiere's; the W3C rounds it to
/// 0.3, 0.59, 0.11).
#[inline]
fn lum(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}
/// Rec. 709 luma: what Premiere's Darker and Lighter Color compare.
#[inline]
fn luma709(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut o = c;
    if n < 0.0 {
        for v in &mut o {
            *v = l + (*v - l) * l / (l - n).max(1e-6);
        }
    }
    if x > 1.0 {
        for v in &mut o {
            *v = l + (*v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
    }
    o
}
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx - mn <= 1e-6 {
        return [0.0; 3];
    }
    let mut o = [0.0; 3];
    for k in 0..3 {
        o[k] = (c[k] - mn) * s / (mx - mn);
    }
    o
}

/// Blend function B(Cb, Cs) on straight, display-encoded colour.
pub fn blend_rgb(mode: Blend, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        Blend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        Blend::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        Blend::Color => set_lum(s, lum(b)),
        Blend::Luminosity => set_lum(b, lum(s)),
        Blend::DarkerColor => {
            if luma709(s) < luma709(b) {
                s
            } else {
                b
            }
        }
        Blend::LighterColor => {
            if luma709(s) > luma709(b) {
                s
            } else {
                b
            }
        }
        m => [sep(m, b[0], s[0]), sep(m, b[1], s[1]), sep(m, b[2], s[2])],
    }
}

fn hash2(x: usize, y: usize) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Composite `src` over `dst` (same size) with `opacity` and `mode`.
pub fn composite(dst: &mut Image, src: &Image, opacity: f32, mode: Blend) {
    debug_assert_eq!((dst.w, dst.h), (src.w, src.h));
    composite_at(dst, src, 0, 0, opacity, mode);
}

/// [`composite`] for a `src` that is only a rectangle of its layer, with its top-left corner at
/// (`x0`, `y0`) of `dst`. The rest of the layer is transparent, which leaves `dst` as it is, so only
/// the rectangle is visited; its pixels are mixed exactly as [`composite`] mixes them (the dissolve
/// pattern follows the pixel's place in `dst`). The part of the rectangle outside `dst` is ignored.
pub fn composite_at(dst: &mut Image, src: &Image, x0: usize, y0: usize, opacity: f32, mode: Blend) {
    let (cw, ch) = (src.w.min(dst.w.saturating_sub(x0)), src.h.min(dst.h.saturating_sub(y0)));
    if cw == 0 || ch == 0 || src.px.len() != src.w * src.h * 4 {
        return;
    }
    let dw = dst.w;
    dst.px.par_chunks_mut(dw * 4).skip(y0).take(ch).zip(src.px.par_chunks(src.w * 4)).enumerate().for_each(|(ry, (d, s))| {
        let y = y0 + ry;
        for rx in 0..cw {
            let x = x0 + rx;
            let (i, j) = (x * 4, rx * 4);
            let da = d[i + 3];
            // a blended layer with nothing under it is drawn as it is, at full opacity: Premiere
            // 26.5 draws a picture alone on V1 in Color or Multiply at 100, 50 and 0 % opaque and
            // unchanged (Round 5 A4 probe; Normal at 50 % is half transparent, at 0 % not drawn)
            if mode.reads_destination() && da <= 0.0 {
                let k = 1.0 - s[j + 3];
                for c in 0..4 {
                    d[i + c] = s[j + c] + d[i + c] * k;
                }
                continue;
            }
            let mut sp = [s[j] * opacity, s[j + 1] * opacity, s[j + 2] * opacity, s[j + 3] * opacity];
            let sa = sp[3];
            if sa <= 0.0 {
                continue;
            }
            if mode == Blend::Dissolve {
                if hash2(x, y) >= sa {
                    continue;
                }
                sp = [sp[0] / sa, sp[1] / sa, sp[2] / sa, 1.0];
            }
            if mode == Blend::Normal || mode == Blend::Dissolve {
                let k = 1.0 - sp[3];
                for c in 0..4 {
                    d[i + c] = sp[c] + d[i + c] * k;
                }
                continue;
            }
            let sa = sp[3];
            let cs = [linear_to_srgb(sp[0] / sa), linear_to_srgb(sp[1] / sa), linear_to_srgb(sp[2] / sa)];
            let cb = [linear_to_srgb(d[i] / da), linear_to_srgb(d[i + 1] / da), linear_to_srgb(d[i + 2] / da)];
            let bl = blend_rgb(mode, cb, cs);
            let ao = sa + da - sa * da;
            for c in 0..3 {
                let b = if unclamped(mode) { bl[c] } else { bl[c].clamp(0.0, 1.0) };
                // source-over of the blended colour, on the encoded straight values
                let enc = (sa * (1.0 - da) * cs[c] + (1.0 - sa) * da * cb[c] + sa * da * b) / ao;
                d[i + c] = srgb_to_linear(enc.clamp(0.0, 1.0)) * ao;
            }
            d[i + 3] = ao;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_over() {
        let mut d = Image::filled(2, 1, [0.5, 0.0, 0.0, 1.0]);
        let s = Image::filled(2, 1, [0.0, 0.25, 0.0, 0.5]);
        composite(&mut d, &s, 1.0, Blend::Normal);
        assert_eq!(d.get(0, 0), [0.25, 0.25, 0.0, 1.0]);
    }

    #[test]
    fn multiply_white_is_identity() {
        let base = [0.2, 0.4, 0.6, 1.0];
        let mut d = Image::filled(1, 1, base);
        let s = Image::filled(1, 1, [1.0, 1.0, 1.0, 1.0]);
        composite(&mut d, &s, 1.0, Blend::Multiply);
        let p = d.get(0, 0);
        for k in 0..3 {
            assert!((p[k] - base[k]).abs() < 1e-4);
        }
    }

    /// `top` (8-bit sRGB, straight alpha) over an opaque `bottom` with `mode`, back in 8-bit.
    fn over8(mode: Blend, bottom: [u8; 3], top: [u8; 3], alpha: f32) -> [i32; 3] {
        let lin = |c: [u8; 3], a: f32| [0, 1, 2].map(|k| srgb_to_linear(c[k] as f32 / 255.0) * a).into_iter().chain([a]).collect::<Vec<_>>();
        let b = lin(bottom, 1.0);
        let t = lin(top, alpha);
        let mut d = Image::filled(1, 1, [b[0], b[1], b[2], 1.0]);
        composite(&mut d, &Image::filled(1, 1, [t[0], t[1], t[2], alpha]), 1.0, mode);
        let p = d.get(0, 0);
        [0, 1, 2].map(|k| (linear_to_srgb(p[k] / p[3]) * 255.0).round() as i32)
    }

    /// Pixels Premiere 26.5 made (Round 4 blend kit), to a level.
    #[test]
    fn modes_match_premiere_pixels() {
        let near = |got: [i32; 3], want: [i32; 3], what: &str| {
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "{what}: {got:?}, Premiere {want:?}");
        };
        // the blend is mixed with the backdrop by the top's alpha on encoded values
        near(over8(Blend::Multiply, [71; 3], [0, 54, 54], 0.5), [35, 43, 43], "Multiply at 50 %");
        near(over8(Blend::LinearBurn, [71; 3], [0, 54, 54], 0.5), [35, 35, 35], "Linear Burn at 50 %");
        // Linear Dodge, Linear Light, Subtract and Divide go into that mix unclamped
        near(over8(Blend::LinearLight, [71; 3], [0, 54, 54], 0.5), [0, 0, 0], "Linear Light at 50 %");
        near(over8(Blend::LinearDodge, [195; 3], [0, 0, 70], 0.5), [195, 195, 230], "Linear Dodge at 50 %");
        near(over8(Blend::Subtract, [9; 3], [70; 3], 0.5), [0, 0, 0], "Subtract at 50 %");
        assert!((over8(Blend::Divide, [71; 3], [0, 54, 54], 0.5)[1] - 204).abs() <= 1, "Divide at 50 %");
        // Darker / Lighter Color compare Rec. 709 luma
        near(over8(Blend::DarkerColor, [0, 87, 217], [254, 0, 0], 1.0), [254, 0, 0], "Darker Color");
        near(over8(Blend::LighterColor, [0, 87, 217], [254, 0, 0], 1.0), [0, 87, 217], "Lighter Color");
        // Hue, Color and Luminosity weigh luminosity by Rec. 601
        near(over8(Blend::Hue, [0, 68, 217], [255, 0, 0], 1.0), [216, 0, 0], "Hue");
        near(over8(Blend::Color, [0, 87, 217], [255, 0, 0], 1.0), [254, 0, 0], "Color");
        near(over8(Blend::Luminosity, [217, 147, 0], [255, 255, 0], 1.0), [255, 224, 159], "Luminosity");
        // Soft Light lifts the darkest values by √b; white stays white under Color Burn
        near(over8(Blend::SoftLight, [217, 1, 4], [255; 3], 1.0), [235, 16, 32], "Soft Light");
        near(over8(Blend::ColorBurn, [255; 3], [0; 3], 0.5), [255; 3], "Color Burn on white");
        // Hard Mix is white from b + s = 1; 0 ÷ 0 is black, anything else ÷ 0 white
        near(over8(Blend::HardMix, [205; 3], [50, 49, 51], 1.0), [255, 0, 255], "Hard Mix");
        near(over8(Blend::Divide, [0, 0, 90], [0, 200, 0], 1.0), [0, 0, 255], "Divide by zero");
    }

    /// Premiere draws a layer in a blend mode with nothing under it as it is, whatever its opacity
    /// (Round 5 A4 probe: chart.png alone on V1 in Color and Multiply at 100, 50 and 0 %); in
    /// Normal its opacity applies.
    #[test]
    fn a_blended_layer_over_nothing_is_drawn_as_it_is() {
        let top = [0.2, 0.3, 0.4, 1.0];
        for mode in Blend::ALL.into_iter().filter(|m| m.reads_destination()) {
            for op in [1.0, 0.5, 0.0] {
                let mut d = Image::new(2, 1);
                d.px[3] = 1.0;
                composite(&mut d, &Image::filled(2, 1, top), op, mode);
                assert_eq!(d.get(1, 0), top, "{mode:?} at {op}");
                // over something it blends at its opacity: at 0 % the backdrop shows
                if op == 0.0 {
                    assert_eq!(d.get(0, 0), [0.0, 0.0, 0.0, 1.0], "{mode:?} at {op}");
                }
            }
        }
        let mut d = Image::new(1, 1);
        composite(&mut d, &Image::filled(1, 1, top), 0.5, Blend::Normal);
        assert_eq!(d.get(0, 0), [0.1, 0.15, 0.2, 0.5]);
    }

    #[test]
    fn all_modes_stay_in_range() {
        for m in Blend::ALL {
            let mut d = Image::filled(4, 4, [0.3, 0.1, 0.7, 1.0]);
            let s = Image::filled(4, 4, [0.4, 0.4, 0.1, 0.8]);
            composite(&mut d, &s, 0.9, m);
            for v in &d.px {
                assert!(v.is_finite() && *v >= -1e-5 && *v <= 1.0001, "{m:?} {v}");
            }
        }
    }
}
