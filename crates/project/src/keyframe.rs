//! Keyframed parameters.
//!
//! A parameter is a static value or a list of keyframes (at the clip's media time: where its
//! `source_in` is, so trims leave them in place). A keyframe's interpolation shapes both sides of
//! it, as in Premiere (Hold: the segment leaving it holds): Linear, Hold, Bezier (with
//! influence/speed handles), Auto Bezier, Continuous Bezier, Ease In/Out (Bezier presets) —
//! Premiere's temporal set.

use filmcraft_geom::Vec2;
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Float(f64),
    /// A point. A coordinate may be NaN: point parameters use NaN for "auto" (the frame or source
    /// centre) until [`crate::resolve_auto_points`] fills it in. JSON has no NaN and writes it as
    /// `null`, so `null` reads back as NaN here, and only here: a `null` coordinate anywhere else
    /// in a project (a mask vertex, a layer rectangle) is still a damaged file.
    Vec2(#[serde(deserialize_with = "point_or_auto")] Vec2),
    Color([f32; 4]),
    Bool(bool),
    Choice(u32),
    Text(String),
    /// Curve control points (x, y) in 0..1, sorted by x (Color Grading curves).
    Curve(Vec<[f32; 2]>),
    /// A mask path (Bézier vertices in clip pixels); interpolates vertex-wise.
    Path(crate::mask::MaskPath),
}

/// Read a point parameter's value, taking a `null` coordinate as NaN ("auto").
fn point_or_auto<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec2, D::Error> {
    fn nan_if_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
    }
    #[derive(Deserialize)]
    struct Point {
        #[serde(deserialize_with = "nan_if_null")]
        x: f64,
        #[serde(deserialize_with = "nan_if_null")]
        y: f64,
    }
    let p = Point::deserialize(d)?;
    Ok(Vec2::new(p.x, p.y))
}

impl ParamValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Float(v) => Some(*v),
            ParamValue::Choice(c) => Some(*c as f64),
            ParamValue::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            ParamValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<[f32; 4]> {
        match self {
            ParamValue::Color(c) => Some(*c),
            _ => None,
        }
    }
    pub fn as_curve(&self) -> Option<&[[f32; 2]]> {
        match self {
            ParamValue::Curve(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Whether values of this kind can be interpolated (others hold).
    pub fn interpolates(&self) -> bool {
        matches!(self, ParamValue::Float(_) | ParamValue::Vec2(_) | ParamValue::Color(_) | ParamValue::Path(_))
    }
    pub fn as_path(&self) -> Option<&crate::mask::MaskPath> {
        match self {
            ParamValue::Path(p) => Some(p),
            _ => None,
        }
    }
    fn components(&self) -> Vec<f64> {
        match self {
            ParamValue::Float(v) => vec![*v],
            ParamValue::Vec2(v) => vec![v.x, v.y],
            ParamValue::Color(c) => c.iter().map(|&x| x as f64).collect(),
            ParamValue::Path(p) => p.components(),
            _ => vec![],
        }
    }
    fn from_components(template: &ParamValue, c: &[f64]) -> ParamValue {
        match template {
            ParamValue::Float(_) => ParamValue::Float(c[0]),
            ParamValue::Vec2(_) => ParamValue::Vec2(Vec2::new(c[0], c[1])),
            ParamValue::Color(_) => ParamValue::Color([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]),
            ParamValue::Path(p) => ParamValue::Path(p.with_components(c)),
            other => other.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    #[default]
    Linear,
    Hold,
    Bezier,
    AutoBezier,
    ContinuousBezier,
    EaseIn,
    EaseOut,
}

impl Interpolation {
    pub const ALL: [Interpolation; 7] = [
        Interpolation::Linear,
        Interpolation::Bezier,
        Interpolation::AutoBezier,
        Interpolation::ContinuousBezier,
        Interpolation::Hold,
        Interpolation::EaseIn,
        Interpolation::EaseOut,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Interpolation::Linear => "Linear",
            Interpolation::Hold => "Hold",
            Interpolation::Bezier => "Bezier",
            Interpolation::AutoBezier => "Auto Bezier",
            Interpolation::ContinuousBezier => "Continuous Bezier",
            Interpolation::EaseIn => "Ease In",
            Interpolation::EaseOut => "Ease Out",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Media time (the clip's source time; [`crate::TrackItem::effect_time_at`] gives it for a
    /// timeline time).
    pub time: Tick,
    pub value: ParamValue,
    /// Interpolation on both sides of this keyframe (Hold: of the segment leaving it).
    pub interp: Interpolation,
    /// Temporal ease: influence (0..1) of the outgoing/incoming handles.
    #[serde(default = "default_influence")]
    pub out_influence: f64,
    #[serde(default = "default_influence")]
    pub in_influence: f64,
    /// Speed of the value leaving / reaching this keyframe, relative to the straight line between
    /// it and the next / previous one (1 = the line's own speed, 0 = an ease: a flat handle). From
    /// a Premiere project's Bézier keyframes, whose handles carry a speed; None is an ease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_slope: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_slope: Option<f64>,
}

fn default_influence() -> f64 {
    1.0 / 3.0
}

impl Keyframe {
    pub fn new(time: Tick, value: ParamValue) -> Self {
        Self {
            time,
            value,
            interp: Interpolation::Linear,
            out_influence: default_influence(),
            in_influence: default_influence(),
            out_slope: None,
            in_slope: None,
        }
    }
}

/// A (possibly animated) parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub value: ParamValue,
    /// Sorted by time. Empty = static.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
}

impl Param {
    pub fn new(value: ParamValue) -> Self {
        Self { value, keyframes: Vec::new() }
    }
    pub fn is_animated(&self) -> bool {
        !self.keyframes.is_empty()
    }
    /// Apply `f` to the static value and every keyframe value.
    pub fn map_values(&mut self, f: impl Fn(ParamValue) -> ParamValue) {
        self.value = f(std::mem::replace(&mut self.value, ParamValue::Float(0.0)));
        for k in &mut self.keyframes {
            k.value = f(std::mem::replace(&mut k.value, ParamValue::Float(0.0)));
        }
    }

    /// Set a value at `t`: updates the static value, or adds/replaces a keyframe when animated.
    pub fn set_at(&mut self, t: Tick, value: ParamValue) {
        if self.keyframes.is_empty() {
            self.value = value;
            return;
        }
        match self.keyframes.binary_search_by_key(&t, |k| k.time) {
            Ok(i) => self.keyframes[i].value = value,
            Err(i) => {
                let interp = i.checked_sub(1).and_then(|j| self.keyframes.get(j)).map(|k| k.interp).unwrap_or_default();
                let mut k = Keyframe::new(t, value);
                k.interp = interp;
                self.keyframes.insert(i, k);
            }
        }
    }

    /// Toggle animation (the stopwatch): on → one keyframe at `t` with the current value; off → keep value at `t`.
    pub fn toggle_animation(&mut self, t: Tick) {
        if self.keyframes.is_empty() {
            self.keyframes.push(Keyframe::new(t, self.value.clone()));
        } else {
            self.value = self.value_at(t);
            self.keyframes.clear();
        }
    }

    pub fn add_keyframe(&mut self, t: Tick) {
        let v = self.value_at(t);
        if self.keyframes.is_empty() {
            self.value = v.clone();
        }
        self.set_at_force(t, v);
    }

    /// Add or replace a keyframe at `t` (the parameter becomes animated).
    pub fn put_keyframe(&mut self, t: Tick, value: ParamValue) {
        self.set_at_force(t, value);
    }

    fn set_at_force(&mut self, t: Tick, value: ParamValue) {
        match self.keyframes.binary_search_by_key(&t, |k| k.time) {
            Ok(i) => self.keyframes[i].value = value,
            Err(i) => self.keyframes.insert(i, Keyframe::new(t, value)),
        }
    }

    pub fn remove_keyframe_at(&mut self, t: Tick) -> bool {
        if let Ok(i) = self.keyframes.binary_search_by_key(&t, |k| k.time) {
            let removed = self.keyframes.remove(i);
            if self.keyframes.is_empty() {
                self.value = removed.value;
            }
            true
        } else {
            false
        }
    }

    /// Evaluate at media time `t`.
    pub fn value_at(&self, t: Tick) -> ParamValue {
        let k = &self.keyframes;
        if k.is_empty() {
            return self.value.clone();
        }
        if t <= k[0].time {
            return k[0].value.clone();
        }
        let last = &k[k.len() - 1];
        if t >= last.time {
            return last.value.clone();
        }
        let i = match k.binary_search_by_key(&t, |kf| kf.time) {
            Ok(i) => return k[i].value.clone(),
            Err(i) => i - 1,
        };
        let (a, b) = (&k[i], &k[i + 1]);
        if a.interp == Interpolation::Hold || !a.value.interpolates() {
            return a.value.clone();
        }
        let span = (b.time - a.time).0 as f64;
        let u = (t - a.time).0 as f64 / span;
        let comps = |j: usize| k[j].value.components();
        let e = ease(a, b, side_slope(k, i, i + 1, comps), side_slope(k, i + 1, i, comps), u);
        let ca = a.value.components();
        let cb = b.value.components();
        if ca.len() != cb.len() {
            // e.g. mask paths with different vertex counts: hold
            return a.value.clone();
        }
        let c: Vec<f64> = ca.iter().zip(&cb).map(|(x, y)| x + (y - x) * e).collect();
        ParamValue::from_components(&a.value, &c)
    }

    pub fn f64_at(&self, t: Tick) -> f64 {
        self.value_at(t).as_f64().unwrap_or(0.0)
    }

    /// Borrowing text evaluation for read-only observers (the Program monitor's IME producer).
    ///
    /// Text parameters never interpolate, so the result is the applicable static or keyframed
    /// `ParamValue::Text`; an interpolating non-text value yields `None`. Unlike
    /// [`Self::value_at`] this does not clone the document-sized string.
    pub fn text_ref_at(&self, t: Tick) -> Option<&str> {
        fn text(v: &ParamValue) -> Option<&str> {
            match v {
                ParamValue::Text(s) => Some(s.as_str()),
                _ => None,
            }
        }
        let k = &self.keyframes;
        if k.is_empty() {
            return text(&self.value);
        }
        if t <= k[0].time {
            return text(&k[0].value);
        }
        let last = &k[k.len() - 1];
        if t >= last.time {
            return text(&last.value);
        }
        let i = match k.binary_search_by_key(&t, |kf| kf.time) {
            Ok(i) => return text(&k[i].value),
            Err(i) => i - 1,
        };
        // Hold semantics match `value_at`: non-interpolating values (including text) stay until
        // the next keyframe; an interpolating segment never evaluates to text.
        if k[i].interp == Interpolation::Hold || !k[i].value.interpolates() { text(&k[i].value) } else { None }
    }

    /// Allocation-free [`Param::f64_at`] for scalar parameters (Float / Choice / Bool), for
    /// per-sample evaluation in the audio mixer. Non-scalar values evaluate to 0.
    pub fn scalar_at(&self, t: Tick) -> f64 {
        self.scalar_at_mapped(t, |v| v)
    }

    /// [`Param::scalar_at`] with every value mapped through `f` before it is interpolated. A
    /// Volume level is kept in dB but Premiere interpolates it in amplitude: half way down a
    /// 0 → −20 dB ramp it is −5.2 dB, not −10 dB, and a fade to −∞ is a straight line to silence.
    pub fn scalar_at_mapped(&self, t: Tick, f: impl Fn(f64) -> f64) -> f64 {
        let k = &self.keyframes;
        let s = |v: &ParamValue| f(v.as_f64().unwrap_or(0.0));
        if k.is_empty() {
            return s(&self.value);
        }
        if t <= k[0].time {
            return s(&k[0].value);
        }
        let last = &k[k.len() - 1];
        if t >= last.time {
            return s(&last.value);
        }
        let i = match k.binary_search_by_key(&t, |kf| kf.time) {
            Ok(i) => return s(&k[i].value),
            Err(i) => i - 1,
        };
        let (a, b) = (&k[i], &k[i + 1]);
        let (va, vb) = (s(&a.value), s(&b.value));
        if a.interp == Interpolation::Hold || !matches!(a.value, ParamValue::Float(_)) {
            return va;
        }
        let u = (t - a.time).0 as f64 / (b.time - a.time).0 as f64;
        let scalar = |j: usize| [s(&k[j].value)];
        va + (vb - va) * ease(a, b, side_slope(k, i, i + 1, scalar), side_slope(k, i + 1, i, scalar), u)
    }

    /// The speeds key `j` plays with (incoming, outgoing; as [`Keyframe::in_slope`] and
    /// [`Keyframe::out_slope`]): its own, or Auto Bézier's worked out from its neighbours.
    pub fn key_slopes(&self, j: usize) -> (Option<f64>, Option<f64>) {
        let k = &self.keyframes;
        if j >= k.len() {
            return (None, None);
        }
        let comps = |i: usize| k[i].value.components();
        let side = |other: Option<usize>| other.filter(|&o| o < k.len()).and_then(|o| side_slope(k, j, o, comps));
        (side(j.checked_sub(1)), side(Some(j + 1)))
    }

    pub fn vec2_at(&self, t: Tick) -> Vec2 {
        self.value_at(t).as_vec2().unwrap_or_default()
    }

    /// Previous/next keyframe times for navigation.
    pub fn prev_keyframe(&self, t: Tick) -> Option<Tick> {
        self.keyframes.iter().rev().map(|k| k.time).find(|&k| k < t)
    }
    pub fn next_keyframe(&self, t: Tick) -> Option<Tick> {
        self.keyframes.iter().map(|k| k.time).find(|&k| k > t)
    }
}

/// The speed of key `j` on its side towards its neighbour `other`: its own, or for Auto Bézier
/// [`auto_slope`].
fn side_slope<C: AsRef<[f64]>>(k: &[Keyframe], j: usize, other: usize, val: impl Fn(usize) -> C) -> Option<f64> {
    let key = &k[j];
    match key.interp {
        Interpolation::AutoBezier => auto_slope(k, j, other, val),
        _ if other > j => key.out_slope,
        _ => key.in_slope,
    }
}

/// Auto Bézier's speed at key `j` towards its neighbour `other`, relative to that segment's
/// straight line. A key between two others moves at the slope between them (After Effects'
/// rule); an end key at half its segment's slope, or a position along it (Round 5 B6: Premiere's
/// two-key Auto Bézier stores and plays 0.5 for Scale, Rotation and Opacity, 1 for Position).
/// `None` (flat) across a segment that does not move.
fn auto_slope<C: AsRef<[f64]>>(k: &[Keyframe], j: usize, other: usize, val: impl Fn(usize) -> C) -> Option<f64> {
    let t = |i: usize| k[i].time.0 as f64;
    let (vj, vo) = (val(j), val(other));
    let dt = t(other) - t(j);
    let seg = vj.as_ref().iter().zip(vo.as_ref()).map(|(a, b)| (b - a) / dt);
    let dd: f64 = seg.clone().map(|d| d * d).sum();
    if dd.is_nan() || dd <= 1e-24 {
        return None;
    }
    if j == 0 || j + 1 >= k.len() {
        return Some(if matches!(k[j].value, ParamValue::Vec2(_)) { 1.0 } else { 0.5 });
    }
    let (prev, next) = (val(j - 1), val(j + 1));
    let span = t(j + 1) - t(j - 1);
    let dot: f64 = prev.as_ref().iter().zip(next.as_ref()).map(|(a, b)| (b - a) / span).zip(seg).map(|(m, d)| m * d).sum();
    Some(dot / dd)
}

/// Temporal easing for u∈[0,1] between two keyframes (1D cubic Bezier in time/value space), with
/// the speeds leaving `a` and reaching `b` ([`side_slope`]). As in Premiere, a keyframe's
/// interpolation shapes both of its sides: the segment leaves `a` as `a` says and reaches `b` as
/// `b` says (a Bézier key followed by a Linear one arrives in a line).
fn ease(a: &Keyframe, b: &Keyframe, out_slope: Option<f64>, in_slope: Option<f64>, u: f64) -> f64 {
    let out_ease = matches!(a.interp, Interpolation::Bezier | Interpolation::AutoBezier | Interpolation::ContinuousBezier | Interpolation::EaseOut);
    let in_ease = matches!(b.interp, Interpolation::Bezier | Interpolation::AutoBezier | Interpolation::ContinuousBezier | Interpolation::EaseIn);
    if !out_ease && !in_ease {
        return u;
    }
    // Control points in normalized (time, value) space: the influence along time, the speed
    // relative to the straight line along the value (Premiere's and After Effects' handles); no
    // speed is a flat handle, an ease.
    let slope = |m: Option<f64>| m.filter(|m| m.is_finite()).map_or(0.0, |m| m.clamp(-10.0, 10.0));
    let (x1, y1) = if out_ease {
        let i = a.out_influence.clamp(0.01, 1.0);
        (i, i * slope(out_slope))
    } else {
        (1.0 / 3.0, 1.0 / 3.0)
    };
    let (x2, y2) = if in_ease {
        let i = b.in_influence.clamp(0.01, 1.0);
        (1.0 - i, 1.0 - i * slope(in_slope))
    } else {
        (2.0 / 3.0, 2.0 / 3.0)
    };
    cubic_bezier_y_for_x(x1, y1, x2, y2, u)
}

/// Solve a CSS-style cubic-bezier(x1,y1,x2,y2) for y at x (Newton + bisection).
pub fn cubic_bezier_y_for_x(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let bx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * x1 + 3.0 * (1.0 - s) * s * s * x2 + s * s * s;
    let by = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * y1 + 3.0 * (1.0 - s) * s * s * y2 + s * s * s;
    let dbx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * x1 + 6.0 * (1.0 - s) * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
    let mut s = x;
    for _ in 0..8 {
        let d = dbx(s);
        if d.abs() < 1e-9 {
            break;
        }
        let ns = s - (bx(s) - x) / d;
        if !(0.0..=1.0).contains(&ns) {
            break;
        }
        s = ns;
    }
    if (bx(s) - x).abs() > 1e-7 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            s = (lo + hi) / 2.0;
            if bx(s) < x { lo = s } else { hi = s }
        }
    }
    by(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_hold() {
        let mut p = Param::new(ParamValue::Float(0.0));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(100), ParamValue::Float(10.0));
        assert_eq!(p.f64_at(Tick(50)), 5.0);
        assert_eq!(p.f64_at(Tick(-5)), 0.0);
        assert_eq!(p.f64_at(Tick(500)), 10.0);
        p.keyframes[0].interp = Interpolation::Hold;
        assert_eq!(p.f64_at(Tick(99)), 0.0);
    }

    #[test]
    fn set_at_before_first_keyframe_uses_default_interp() {
        let mut p = Param::new(ParamValue::Float(0.0));
        p.toggle_animation(Tick(100));
        p.keyframes[0].interp = Interpolation::Hold;
        p.set_at(Tick(0), ParamValue::Float(5.0));
        assert_eq!(p.keyframes[0].interp, Interpolation::Linear);
        assert_eq!(p.keyframes[1].interp, Interpolation::Hold);
        p.set_at(Tick(200), ParamValue::Float(7.0));
        assert_eq!(p.keyframes[2].interp, Interpolation::Hold);
    }

    #[test]
    fn bezier_eases() {
        let mut p = Param::new(ParamValue::Vec2(Vec2::new(0.0, 0.0)));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(1000), ParamValue::Vec2(Vec2::new(100.0, 50.0)));
        p.keyframes[0].interp = Interpolation::Bezier;
        p.keyframes[1].interp = Interpolation::Bezier;
        let early = p.vec2_at(Tick(100)).x;
        let mid = p.vec2_at(Tick(500)).x;
        assert!(early < 10.0, "ease-in start should be slow: {early}");
        assert!((mid - 50.0).abs() < 1.0, "symmetric ease midpoint: {mid}");
        // monotonic
        let mut last = -1.0;
        for i in 0..=100 {
            let v = p.vec2_at(Tick(i * 10)).x;
            assert!(v >= last - 1e-9);
            last = v;
        }
    }

    /// Premiere's Bézier handles carry a speed: at the straight line's own speed a Bézier key plays
    /// as a line, with no speed on the arriving side it eases in. The numbers are Premiere's
    /// (a 2 s opacity ramp in the user's Round 5 tests: 0 → 100 %, 63.5 % half way with Ease In).
    #[test]
    fn bezier_handles_follow_premiere_speeds() {
        let ramp = |out: (f64, Option<f64>), into: (f64, Option<f64>), interp_b: Interpolation| {
            let mut p = Param::new(ParamValue::Float(0.0));
            p.toggle_animation(Tick(0));
            p.set_at(Tick(1000), ParamValue::Float(100.0));
            let (a, b) = p.keyframes.split_at_mut(1);
            (a[0].interp, a[0].out_influence, a[0].out_slope) = (Interpolation::Bezier, out.0, out.1);
            (b[0].interp, b[0].in_influence, b[0].in_slope) = (interp_b, into.0, into.1);
            p
        };
        let line = ramp((1.0 / 6.0, Some(1.0)), (1.0 / 6.0, Some(1.0)), Interpolation::Bezier);
        for t in [100, 250, 500, 750, 900] {
            assert!((line.f64_at(Tick(t)) - t as f64 / 10.0).abs() < 1e-6, "{t}: {}", line.f64_at(Tick(t)));
        }
        let ease_in = ramp((1.0 / 6.0, Some(1.0)), (1.0 / 3.0, Some(0.0)), Interpolation::Bezier);
        let half = ease_in.f64_at(Tick(500));
        assert!((half - 63.5).abs() < 0.5, "{half}");
        // a Bézier key followed by a Linear one: eased out of the first, in a line into the second
        let to_linear = ramp((1.0 / 3.0, Some(0.0)), (1.0 / 3.0, None), Interpolation::Linear);
        let (early, late) = (to_linear.f64_at(Tick(100)), to_linear.f64_at(Tick(990)) - to_linear.f64_at(Tick(980)));
        assert!(early < 10.0 && late > 0.5, "eased start, no ease at the end: {early} {late}");
    }

    /// Auto Bézier works its speeds out from the neighbours: half the line's speed at the ends of a
    /// two-key ramp (Premiere's 2 s opacity ramp is at 20 % a quarter of the way: 52 of 255), the
    /// position along the line, and an inner key the slope between its neighbours.
    #[test]
    fn auto_bezier_works_out_its_speeds() {
        let auto = |values: &[ParamValue]| {
            let mut p = Param::new(values[0].clone());
            for (i, v) in values.iter().enumerate() {
                p.put_keyframe(Tick(i as i64 * 1000), v.clone());
            }
            p.keyframes.iter_mut().for_each(|k| k.interp = Interpolation::AutoBezier);
            p
        };
        let ramp = auto(&[ParamValue::Float(0.0), ParamValue::Float(100.0)]);
        assert!((ramp.f64_at(Tick(250)) - 20.3).abs() < 0.1, "{}", ramp.f64_at(Tick(250)));
        assert!((ramp.scalar_at(Tick(250)) - ramp.f64_at(Tick(250))).abs() < 1e-9);
        assert_eq!(ramp.key_slopes(0), (None, Some(0.5)));
        let path = auto(&[ParamValue::Vec2(Vec2::new(0.0, 0.0)), ParamValue::Vec2(Vec2::new(100.0, 50.0))]);
        assert!((path.vec2_at(Tick(250)).x - 25.0).abs() < 1e-6, "{:?}", path.vec2_at(Tick(250)));
        // up to 100, then held there: the middle key arrives at half the first segment's speed
        let rise = auto(&[ParamValue::Float(0.0), ParamValue::Float(100.0), ParamValue::Float(100.0)]);
        assert_eq!(rise.key_slopes(1), (Some(0.5), None));
        assert!((rise.f64_at(Tick(1500)) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn text_ref_at_borrows_without_cloning_and_holds_non_interpolating_values() {
        let p = Param::new(ParamValue::Text("abc".into()));
        assert_eq!(p.text_ref_at(Tick(0)), Some("abc"));
        let f = Param::new(ParamValue::Float(1.0));
        assert_eq!(f.text_ref_at(Tick(0)), None);
        let mut p = Param::new(ParamValue::Text("a".into()));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(100), ParamValue::Text("b".into()));
        assert_eq!(p.text_ref_at(Tick(50)), Some("a"), "text does not interpolate");
        assert_eq!(p.text_ref_at(Tick(100)), Some("b"));
    }

    #[test]
    fn toggle_off_keeps_value() {
        let mut p = Param::new(ParamValue::Float(1.0));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(10), ParamValue::Float(3.0));
        p.toggle_animation(Tick(5));
        assert!(!p.is_animated());
        assert_eq!(p.value, ParamValue::Float(2.0));
    }
}
