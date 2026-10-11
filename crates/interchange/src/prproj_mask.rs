//! Premiere Pro effect masks ↔ FilmCraft [`Mask`]s.
//!
//! Observed in projects saved by Premiere Pro 25–26 (no specification exists; see the crate
//! README): every mask is an `AE.ADBE AEMask2` component in its effect's `SubComponents` (any
//! effect: Opacity, Gaussian Blur, Tint, …), in the order Premiere lists them. Its parameters are
//! found by `ParameterID`:
//!
//! | id | name | value |
//! |---|---|---|
//! | 7 | Path | binary (below); keyframes `time,base64;…` in source-media ticks |
//! | 9 | Position | `x:y`, normalised to the clip's source frame |
//! | 10 | Anchor Point | `x:y`, normalised like Position |
//! | 26 / 27 | Scale Height / Scale Width | % |
//! | 12 | Rotation | degrees, clockwise |
//! | 14 | Feather | source pixels |
//! | 15 | Opacity | % |
//! | 16 | Expansion | source pixels |
//! | 17 | (unnamed) Inverted | `true` / `false` |
//! | 21 | Blend Mode | always 0 (= Add) in every project seen |
//! | 22 | Type | 0 / 3 free-form (pen), 1 ellipse, 2 rectangle, 4 Object Mask (no path) |
//!
//! The path is a little-endian blob: `u32` 2, `u32` 1, `u32` an id, `u8` 0, `u32` 2, `u32` vertex
//! count *n*, then *n* × (`u32` 1 smooth / 0 corner, `f32` x, y, in-handle x, y, out-handle x, y,
//! all normalised to the source frame), then `u8` 1. An empty mask is the 8 bytes `2, 0`.
//!
//! A vertex `v` (in source pixels) is drawn at `Position + Rotation · Scale · (v − Anchor)`. A
//! tracked mask keys Position and Anchor together (the transform cancels) and the path on every
//! frame. FilmCraft's mask has no transform of its own, so the transform is applied to the path
//! here, at every time any of them has a keyframe.
//!
//! An Object Mask (type 4, Premiere's subject segmentation) has no path: Premiere computes it,
//! so it cannot be carried over and is reported instead.

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{Keyframe, Mask, MaskMode, MaskPath, MaskVertex, Param, ParamValue};
use filmcraft_time::Tick;
use roxmltree::Node;

use crate::prproj_fx::{self, Conv, Ctx, MAX_TIME, STATIC_TIME};
use crate::prproj_graph::Index;
use crate::xml::child_text;

/// The match name of a mask component.
pub(crate) const MASK_COMPONENT: &str = "AE.ADBE AEMask2";

/// Paths with more vertices than this are not read (Premiere's free-form masks have a few dozen).
const MAX_VERTICES: usize = 10_000;
/// Keyframes of one mask path beyond this are not read.
const MAX_KEYS: usize = 100_000;

pub(crate) const PATH: &str = "7";
const POSITION: &str = "9";
const ANCHOR: &str = "10";
const ROTATION: &str = "12";
pub(crate) const FEATHER: &str = "14";
pub(crate) const OPACITY: &str = "15";
pub(crate) const EXPANSION: &str = "16";
pub(crate) const INVERTED: &str = "17";
const BLEND_MODE: &str = "21";
const TYPE: &str = "22";
const SCALE_HEIGHT: &str = "26";
const SCALE_WIDTH: &str = "27";
/// Type value of an Object Mask.
const OBJECT_MASK: &str = "4";

/// Why a mask of a Premiere effect was not carried over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Skipped {
    /// Premiere's Object Mask: a segmentation Premiere computes, with no path in the project.
    Object,
    /// A mask without a shape (drawn but never given points).
    Empty,
    /// The path could not be read.
    Unreadable,
}

/// The masks of one effect component.
#[derive(Default)]
pub(crate) struct Read {
    pub masks: Vec<Mask>,
    pub skipped: Vec<Skipped>,
}

/// The mask components of an effect component, in order.
pub(crate) fn components<'a, 'i>(ix: &Index<'a, 'i>, effect: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    ix.list(effect, &["SubComponents"], "SubComponent").into_iter().filter(|n| child_text(*n, "MatchName").map(str::trim) == Some(MASK_COMPONENT)).collect()
}

/// A mask component's parameter by `ParameterID`.
pub(crate) fn param<'a, 'i>(ix: &Index<'a, 'i>, mask: Node<'a, 'i>, id: &str) -> Option<Node<'a, 'i>> {
    let body = crate::xml::child(mask, "Component")?;
    ix.list(body, &["Params"], "Param").into_iter().find(|p| child_text(*p, "ParameterID").map(str::trim) == Some(id))
}

/// Read the masks of an effect component. `binary` returns the bytes of a base64 element
/// (resolving `BinaryHash` references).
pub(crate) fn read<'a, 'i>(ix: &Index<'a, 'i>, effect: Node<'a, 'i>, ctx: &Ctx, binary: &mut dyn FnMut(Node<'a, 'i>) -> Option<Vec<u8>>) -> Read {
    let mut out = Read::default();
    for (_, m) in read_each(ix, effect, ctx, binary) {
        match m {
            Ok(mask) => out.masks.push(mask),
            Err(why) => out.skipped.push(why),
        }
    }
    out
}

/// Each mask component of an effect with what it reads as.
pub(crate) fn read_each<'a, 'i>(
    ix: &Index<'a, 'i>,
    effect: Node<'a, 'i>,
    ctx: &Ctx,
    binary: &mut dyn FnMut(Node<'a, 'i>) -> Option<Vec<u8>>,
) -> Vec<(Node<'a, 'i>, Result<Mask, Skipped>)> {
    components(ix, effect)
        .into_iter()
        .enumerate()
        .map(|(n, node)| {
            let m = mask(ix, node, ctx, binary).map(|mut m| {
                m.name = format!("Mask ({})", n + 1);
                m
            });
            (node, m)
        })
        .collect()
}

/// Whether two masks draw the same (names and tracking settings aside).
pub(crate) fn same_shape(a: &Mask, b: &Mask) -> bool {
    a.path == b.path && a.mode == b.mode
}

/// Whether two masks are the same (names and tracking settings aside).
pub(crate) fn same(a: &Mask, b: &Mask) -> bool {
    same_shape(a, b) && a.feather == b.feather && a.opacity == b.opacity && a.expansion == b.expansion && a.inverted == b.inverted
}

fn mask<'a, 'i>(ix: &Index<'a, 'i>, m: Node<'a, 'i>, ctx: &Ctx, binary: &mut dyn FnMut(Node<'a, 'i>) -> Option<Vec<u8>>) -> Result<Mask, Skipped> {
    let p = |id: &str| param(ix, m, id);
    if p(TYPE).and_then(prproj_fx::start_value).map(|v| v.trim_end_matches('.')) == Some(OBJECT_MASK) {
        return Err(Skipped::Object);
    }
    let shape = p(PATH).map(|n| path_param(n, ctx, binary)).unwrap_or(Ok(None))?;
    let Some(shape) = shape else { return Err(Skipped::Empty) };
    let float = |id: &str, default: f64| {
        p(id)
            .and_then(|n| prproj_fx::decode(n, Conv::Float))
            .map(|c| prproj_fx::to_param(&c, Conv::Float, ctx))
            .unwrap_or_else(|| Param::new(ParamValue::Float(default)))
    };
    let point = |id: &str| p(id).and_then(|n| prproj_fx::decode(n, Conv::SourcePoint)).map(|c| prproj_fx::to_param(&c, Conv::SourcePoint, ctx));
    let xf = Transform {
        position: point(POSITION),
        anchor: point(ANCHOR),
        scale_w: float(SCALE_WIDTH, 100.0),
        scale_h: float(SCALE_HEIGHT, 100.0),
        rotation: float(ROTATION, 0.0),
    };
    let mut mask = Mask::new("Mask", MaskPath::default());
    mask.path = xf.apply(&shape);
    mask.feather = clamp_param(float(FEATHER, 0.0), 0.0, 10_000.0);
    mask.opacity = clamp_param(float(OPACITY, 100.0), 0.0, 100.0);
    mask.expansion = clamp_param(float(EXPANSION, 0.0), -10_000.0, 10_000.0);
    mask.inverted = p(INVERTED).and_then(prproj_fx::start_value).is_some_and(|v| v == "true" || v == "1");
    // Only Blend Mode 0 has ever been seen; it combines like FilmCraft's Add.
    mask.mode = MaskMode::Add;
    let _ = p(BLEND_MODE);
    Ok(mask)
}

/// Keep a float parameter's values within a range (the document is not trusted).
fn clamp_param(mut p: Param, lo: f64, hi: f64) -> Param {
    let fix = |v: &mut ParamValue| {
        if let ParamValue::Float(x) = v {
            *x = if x.is_finite() { x.clamp(lo, hi) } else { lo.max(0.0).min(hi) };
        }
    };
    fix(&mut p.value);
    for k in &mut p.keyframes {
        fix(&mut k.value);
    }
    p
}

/// The mask path parameter as a FilmCraft path parameter in source pixels, before the mask's
/// transform. `Ok(None)` for an empty path.
fn path_param<'a, 'i>(node: Node<'a, 'i>, ctx: &Ctx, binary: &mut dyn FnMut(Node<'a, 'i>) -> Option<Vec<u8>>) -> Result<Option<Param>, Skipped> {
    let to_px = |path: MaskPath| path.transformed(&Affine::scale(ctx.src.0, ctx.src.1));
    let mut keys: Vec<(Tick, MaskPath)> = Vec::new();
    if let Some(text) = child_text(node, "Keyframes") {
        for raw in text.split(';').take(MAX_KEYS) {
            let Some((t, b64)) = raw.split_once(',') else { continue };
            let Ok(t) = t.trim().parse::<i64>() else { continue };
            if t == STATIC_TIME {
                continue;
            }
            let Some(path) = crate::prproj_text::base64(b64.trim()).and_then(|b| decode(&b)) else { continue };
            keys.push((Tick(ctx.clip_time(t.clamp(-MAX_TIME, MAX_TIME))), to_px(path)));
        }
    }
    keys.sort_by_key(|k| k.0);
    keys.dedup_by_key(|k| k.0);
    let start = match crate::xml::child(node, "StartKeyframeValue") {
        Some(v) => match binary(v) {
            Some(b) => Some(decode(&b).ok_or(Skipped::Unreadable)?),
            None => None,
        },
        None => None,
    };
    if keys.iter().all(|(_, p)| p.is_empty()) && start.as_ref().is_none_or(MaskPath::is_empty) {
        return Ok(None);
    }
    let mut param = Param::new(ParamValue::Path(start.map(to_px).unwrap_or_default()));
    for (t, path) in keys {
        param.keyframes.push(Keyframe::new(t, ParamValue::Path(path)));
    }
    if let Some(k) = param.keyframes.first() {
        param.value = k.value.clone();
    }
    Ok(Some(param))
}

/// Decode a path blob into a path normalised to the source frame. `None` when it is malformed.
pub(crate) fn decode(b: &[u8]) -> Option<MaskPath> {
    let u32_at = |o: usize| b.get(o..o.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let f32_at = |o: usize| b.get(o..o.checked_add(4)?).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64).filter(|v| v.is_finite());
    if u32_at(4)? == 0 || b.len() < 21 {
        // `2, 0`: a mask without points
        return Some(MaskPath::default());
    }
    let n = u32_at(17)? as usize;
    if n > MAX_VERTICES {
        return None;
    }
    let mut vertices = Vec::with_capacity(n);
    let mut o = 21usize;
    for _ in 0..n {
        let _smooth = u32_at(o)?;
        let f = |k: usize| f32_at(o + 4 + 4 * k);
        let p = Vec2::new(f(0)?, f(1)?);
        let i = Vec2::new(f(2)?, f(3)?);
        let q = Vec2::new(f(4)?, f(5)?);
        vertices.push(MaskVertex { p, t_in: i - p, t_out: q - p });
        o += 28;
    }
    Some(MaskPath { vertices, closed: true })
}

/// Encode a path normalised to the source frame (the inverse of [`decode`]; tests check the format).
#[cfg(test)]
pub(crate) fn encode(path: &MaskPath, id: u32) -> Vec<u8> {
    if path.is_empty() {
        return [2u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
    }
    let mut b = Vec::with_capacity(22 + 28 * path.len());
    for v in [2u32, 1, id] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.push(0);
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&(path.len() as u32).to_le_bytes());
    for v in &path.vertices {
        b.extend_from_slice(&u32::from(!v.is_corner()).to_le_bytes());
        let (i, o) = (v.p + v.t_in, v.p + v.t_out);
        for x in [v.p.x, v.p.y, i.x, i.y, o.x, o.y] {
            b.extend_from_slice(&(x as f32).to_le_bytes());
        }
    }
    b.push(1);
    b
}

/// A mask's Transform group (all in source pixels / % / degrees, in clip time).
struct Transform {
    position: Option<Param>,
    anchor: Option<Param>,
    scale_w: Param,
    scale_h: Param,
    rotation: Param,
}

impl Transform {
    fn at(&self, t: Tick) -> Affine {
        let anchor = self.anchor.as_ref().map(|p| p.vec2_at(t));
        let position = self.position.as_ref().map(|p| p.vec2_at(t));
        // a missing Position or Anchor Point leaves the path where it is
        let (pos, anchor) = match (position, anchor) {
            (Some(p), Some(a)) => (p, a),
            (Some(p), None) => (p, p),
            (None, Some(a)) => (a, a),
            (None, None) => (Vec2::ZERO, Vec2::ZERO),
        };
        let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let (sw, sh) = (finite(self.scale_w.f64_at(t), 100.0) / 100.0, finite(self.scale_h.f64_at(t), 100.0) / 100.0);
        let rot = finite(self.rotation.f64_at(t), 0.0);
        Affine::translate(pos.x, pos.y)
            .then_apply(&Affine::rotate_deg(rot))
            .then_apply(&Affine::scale(sw, sh))
            .then_apply(&Affine::translate(-anchor.x, -anchor.y))
    }

    fn is_identity_at(&self, t: Tick) -> bool {
        let m = self.at(t);
        let eps = 1e-9;
        (m.a - 1.0).abs() < eps && m.b.abs() < eps && m.c.abs() < eps && (m.d - 1.0).abs() < eps && m.e.abs() < 1e-6 && m.f.abs() < 1e-6
    }

    /// The path with the transform applied at every keyframe time of the path or the transform.
    fn apply(&self, path: &Param) -> Param {
        let mut times: Vec<Tick> = path.keyframes.iter().map(|k| k.time).collect();
        for p in [self.position.as_ref(), self.anchor.as_ref(), Some(&self.scale_w), Some(&self.scale_h), Some(&self.rotation)].into_iter().flatten() {
            times.extend(p.keyframes.iter().map(|k| k.time));
        }
        times.sort();
        times.dedup();
        times.truncate(MAX_KEYS);
        let shape_at = |t: Tick| match path.value_at(t) {
            ParamValue::Path(p) => p,
            _ => MaskPath::default(),
        };
        if times.is_empty() {
            let t = Tick(0);
            let shape = shape_at(t);
            return Param::new(ParamValue::Path(if self.is_identity_at(t) { shape } else { shape.transformed(&self.at(t)) }));
        }
        let mut out = Param::new(ParamValue::Path(MaskPath::default()));
        for t in times {
            out.keyframes.push(Keyframe::new(t, ParamValue::Path(shape_at(t).transformed(&self.at(t)))));
        }
        if let Some(k) = out.keyframes.first() {
            out.value = k.value.clone();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ellipse of a Gaussian Blur mask from a project saved by Premiere Pro 26.
    const ELLIPSE: &str = "AgAAAAEAAABWeQAAAAIAAAAEAAAAAQAAAELE7T5glO4+VsyYPmCU7j4XXiE/YJTuPgEAAADNzkM/gNkZP83OQz9EwwY/zc5DP73vLD8BAAAAQsTtPtFoPD8XXiE/0Wg8P1bMmD7RaDw/AQAAANjVJz6A2Rk/2NUnPr3vLD/Y1Sc+RMMGPwE=";

    #[test]
    fn decodes_premieres_ellipse() {
        let b = crate::prproj_text::base64(ELLIPSE).unwrap();
        let p = decode(&b).unwrap();
        assert_eq!(p.len(), 4);
        // top, right, bottom, left of an ellipse centred on (0.4644, 0.6010), radii 0.3005 × 0.1350
        let c = Vec2::new(0.464_390, 0.600_975);
        assert!((p.vertices[0].p - Vec2::new(c.x, 0.466_000)).length() < 1e-4, "{:?}", p.vertices[0]);
        assert!((p.vertices[1].p - Vec2::new(0.764_870, c.y)).length() < 1e-4, "{:?}", p.vertices[1]);
        // circular tangents: the top vertex leaves to the right (clockwise on screen)
        let k = p.vertices[0].t_out.x / (p.vertices[1].p.x - c.x);
        assert!((k - filmcraft_project::mask::KAPPA).abs() < 1e-3, "{k}");
        assert!(p.vertices[0].t_in.x < 0.0 && p.vertices[1].t_in.y < 0.0);
        // and back
        let again = decode(&encode(&p, 31062)).unwrap();
        for (a, b) in p.vertices.iter().zip(&again.vertices) {
            assert!((a.p - b.p).length() < 1e-6 && (a.t_in - b.t_in).length() < 1e-6 && (a.t_out - b.t_out).length() < 1e-6);
        }
        assert_eq!(encode(&p, 31062), b, "re-encodes byte for byte");
    }

    #[test]
    fn empty_and_malformed_paths() {
        assert_eq!(decode(&[2, 0, 0, 0, 0, 0, 0, 0]), Some(MaskPath::default()));
        assert_eq!(decode(&[]), None);
        let b = crate::prproj_text::base64(ELLIPSE).unwrap();
        // every truncation is refused, never a panic
        for n in 0..b.len() - 1 {
            if let Some(p) = decode(&b[..n]) {
                assert!(p.is_empty(), "{n}");
            }
        }
        // an absurd vertex count
        let mut huge = b.clone();
        huge[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&huge), None);
        // non-finite coordinates
        let mut nan = b.clone();
        nan[25..29].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(decode(&nan), None);
    }

    #[test]
    fn transform_moves_scales_and_rotates_about_the_anchor() {
        let square = MaskPath::rect(90.0, 90.0, 110.0, 110.0);
        let xf = Transform {
            position: Some(Param::new(ParamValue::Vec2(Vec2::new(300.0, 200.0)))),
            anchor: Some(Param::new(ParamValue::Vec2(Vec2::new(100.0, 100.0)))),
            scale_w: Param::new(ParamValue::Float(200.0)),
            scale_h: Param::new(ParamValue::Float(100.0)),
            rotation: Param::new(ParamValue::Float(90.0)),
        };
        let out = xf.apply(&Param::new(ParamValue::Path(square)));
        let ParamValue::Path(p) = out.value else { panic!() };
        // (90, 90) → (−10, −10) from the anchor → scaled (−20, −10) → rotated 90° clockwise
        // (10, −20) → (310, 180)
        assert!((p.vertices[0].p - Vec2::new(310.0, 180.0)).length() < 1e-9, "{:?}", p.vertices[0].p);
        assert!(out.keyframes.is_empty());
        // an animated position makes keys at its times
        let mut moving = Param::new(ParamValue::Vec2(Vec2::new(100.0, 100.0)));
        moving.put_keyframe(Tick(0), ParamValue::Vec2(Vec2::new(100.0, 100.0)));
        moving.put_keyframe(Tick(1000), ParamValue::Vec2(Vec2::new(150.0, 100.0)));
        let xf = Transform {
            position: Some(moving),
            anchor: Some(Param::new(ParamValue::Vec2(Vec2::new(100.0, 100.0)))),
            scale_w: Param::new(ParamValue::Float(100.0)),
            scale_h: Param::new(ParamValue::Float(100.0)),
            rotation: Param::new(ParamValue::Float(0.0)),
        };
        let out = xf.apply(&Param::new(ParamValue::Path(MaskPath::rect(0.0, 0.0, 10.0, 10.0))));
        assert_eq!(out.keyframes.len(), 2);
        let ParamValue::Path(end) = &out.keyframes[1].value else { panic!() };
        assert_eq!(end.vertices[0].p, Vec2::new(50.0, 0.0));
    }
}
