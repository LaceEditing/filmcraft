//! Premiere Pro clip components (effects) ↔ FilmCraft effect instances, shared by the reader and
//! the writer. A parameter is decoded into a [`Canon`]: its value and keyframes in Premiere's own
//! terms (source-media ticks, normalised points, linear gain). Import converts a `Canon` into a
//! FilmCraft [`Param`]; export converts the FilmCraft `Param` back into a `Canon` and rewrites the
//! Premiere parameter only when the two differ, so untouched parameters stay byte-identical.
//!
//! Observed encoding (no specification exists; see the crate README):
//! `StartKeyframe` = `time,value,…` with time `-91445760000000000` (static value);
//! `Keyframes` = `time,value,f2,f3,in speed,in influence,out speed,out influence;` repeated, time
//! in source-media ticks. `f2` = 0 linear, 5 Bézier; with Bézier `f3` = 0 Bézier, 2 auto Bézier,
//! 4 continuous Bézier. Points are `x:y` normalised to the frame (Position) or source (Anchor).
//! Clip volume `Level` is linear gain where 0.177828 is 0 dB.

use filmcraft_geom::Vec2;
use filmcraft_project::effect::{EffectDef, EffectKind, ParamKind, find_effect_by_name};
use filmcraft_project::{Interpolation, Keyframe, Param, ParamValue};
use roxmltree::Node;

use crate::prproj_graph::{Index, TICKS_PER_SECOND};
use crate::xml::{child_text, num, path_text};

/// Stored clip volume of a clip at 0 dB (observed on every untouched clip): linear gain with
/// +15 dB of headroom above 1.0.
pub(crate) const UNITY_LEVEL: f64 = 0.177_827_941_003_892_3;
/// `StartKeyframe` time meaning "before the start of time" (a static value).
pub(crate) const STATIC_TIME: i64 = -91_445_760_000_000_000;
/// Times from the document are clamped to ±48 hours so no conversion can overflow.
pub(crate) const MAX_TIME: i64 = 48 * 3600 * TICKS_PER_SECOND;
const MAX_KEYS: usize = 100_000;

/// One component of a clip's component chain.
pub(crate) struct Component<'a, 'i> {
    pub node: Node<'a, 'i>,
    pub match_name: &'a str,
    pub display: Option<&'a str>,
    pub audio: bool,
    pub params: Vec<Node<'a, 'i>>,
}

/// Read a component object (`VideoFilterComponent`, `AudioFilterComponent`, …).
pub(crate) fn component<'a, 'i>(ix: &Index<'a, 'i>, node: Node<'a, 'i>) -> Option<Component<'a, 'i>> {
    let (body, display, audio) = match crate::xml::child(node, "AudioComponent") {
        Some(a) => (crate::xml::child(a, "Component"), path_text(a, &["Component", "DisplayName"]), true),
        None => (crate::xml::child(node, "Component"), path_text(node, &["Component", "DisplayName"]), node.tag_name().name().starts_with("Audio")),
    };
    let body = body?;
    let match_name = child_text(node, "MatchName").or_else(|| child_text(node, "FilterMatchName")).unwrap_or("");
    let params = ix.list(body, &["Params"], "Param");
    Some(Component { node, match_name, display: display.filter(|d| !d.trim().is_empty()), audio, params })
}

/// The chain's components, in order.
pub(crate) fn chain<'a, 'i>(ix: &Index<'a, 'i>, chain: Node<'a, 'i>) -> Vec<Component<'a, 'i>> {
    ix.list(chain, &["ComponentChain", "Components"], "Component").into_iter().filter_map(|c| component(ix, c)).collect()
}

/// What a component becomes in FilmCraft.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    /// The intrinsic Motion / Opacity / clip volume.
    Motion,
    Opacity,
    Volume,
    /// Premiere-internal state FilmCraft has its own defaults for (channel volume, panner, …).
    Ignore,
    /// A FilmCraft effect with the same name.
    Effect(&'static EffectDef),
    Unsupported,
}

impl Component<'_, '_> {
    pub fn label(&self) -> &str {
        self.display.unwrap_or(self.match_name)
    }

    pub fn target(&self) -> Target {
        match self.match_name {
            "AE.ADBE Motion" => return Target::Motion,
            "AE.ADBE Opacity" => return Target::Opacity,
            m if m.starts_with("Internal Volume") => return Target::Volume,
            m if m.starts_with("Internal") => return Target::Ignore,
            "" if self.display.is_none() => return Target::Ignore,
            _ => {}
        }
        let name = self.label().trim();
        let base = name.strip_suffix("(Legacy)").map(str::trim).unwrap_or(name);
        let want = if self.audio { EffectKind::Audio } else { EffectKind::Video };
        match find_effect_by_name(base) {
            Some(d) if d.kind == want && !d.intrinsic => Target::Effect(d),
            _ => Target::Unsupported,
        }
    }
}

/// The targets of a chain's components. Only the first Motion, Opacity and clip volume of a clip
/// map onto FilmCraft's intrinsic effects; a second one (Premiere allows it) is left as it is.
pub(crate) fn targets(comps: &[Component]) -> Vec<Target> {
    let (mut motion, mut opacity, mut volume) = (false, false, false);
    comps
        .iter()
        .map(|c| {
            let t = c.target();
            let seen = match t {
                Target::Motion => std::mem::replace(&mut motion, true),
                Target::Opacity => std::mem::replace(&mut opacity, true),
                Target::Volume => std::mem::replace(&mut volume, true),
                _ => false,
            };
            if seen { Target::Ignore } else { t }
        })
        .collect()
}

/// How a Premiere value converts to a FilmCraft one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Conv {
    Float,
    /// Linear gain ↔ dB (clip volume).
    Db,
    Bool,
    /// `x:y` normalised to the sequence frame ↔ pixels.
    FramePoint,
    /// `x:y` normalised to the source frame ↔ pixels.
    SourcePoint,
    /// A popup's 0-based index (observed: Blur Dimensions 0 = Horizontal and Vertical, Ultra Key
    /// Setting 3 = Custom); the list length bounds it.
    Choice(usize),
    /// A colour as one integer: four 16-bit channels A, R, G, B, high byte first (observed:
    /// `0xff00_4800_e000_0000` is Ultra Key's default green, `0xff00_0000_0000_0000` black).
    Color,
}

/// One Premiere parameter mapped onto a FilmCraft parameter.
pub(crate) struct Mapping<'a, 'i> {
    pub node: Node<'a, 'i>,
    pub effect: &'static str,
    pub param: &'static str,
    pub conv: Conv,
}

/// The mapped parameters of a component and the names of those that could not be mapped.
pub(crate) fn mappings<'a, 'i>(c: &Component<'a, 'i>, target: Target) -> (Vec<Mapping<'a, 'i>>, Vec<String>) {
    let mut out = Vec::new();
    let mut unmapped = Vec::new();
    let named = |n: &str| c.params.iter().copied().find(|p| child_text(*p, "Name").is_some_and(|x| x.trim().eq_ignore_ascii_case(n)));
    let mut add = |node: Option<Node<'a, 'i>>, effect: &'static str, param: &'static str, conv: Conv| {
        if let Some(node) = node {
            out.push(Mapping { node, effect, param, conv });
        }
    };
    match target {
        Target::Motion => {
            add(named("Position"), "motion", "position", Conv::FramePoint);
            add(named("Scale"), "motion", "scale", Conv::Float);
            add(named("Scale Width"), "motion", "scale_width", Conv::Float);
            add(named("Rotation"), "motion", "rotation", Conv::Float);
            add(named("Anchor Point"), "motion", "anchor", Conv::SourcePoint);
            add(named("Anti-flicker Filter"), "motion", "anti_flicker", Conv::Float);
            // Uniform Scale is Motion's one boolean parameter; it is saved without a name.
            let uniform = c
                .params
                .iter()
                .copied()
                .find(|p| child_text(*p, "Name").is_none_or(|n| n.trim().is_empty()) && start_value(*p).is_some_and(|v| v == "true" || v == "false"));
            add(uniform, "motion", "uniform_scale", Conv::Bool);
        }
        Target::Opacity => add(named("Opacity"), "opacity", "opacity", Conv::Float),
        Target::Volume => add(named("Level"), "volume", "level", Conv::Db),
        Target::Effect(def) => {
            // a name Premiere uses for several parameters of one effect (Lumetri's "Intensity")
            // cannot be mapped by name; each FilmCraft parameter takes at most one value
            let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for p in &c.params {
                if let Some(n) = child_text(*p, "Name").map(str::trim).filter(|n| !n.is_empty()) {
                    *counts.entry(n.to_ascii_lowercase()).or_insert(0) += 1;
                }
            }
            let mut taken = std::collections::HashSet::new();
            for p in &c.params {
                let Some(name) = child_text(*p, "Name").map(str::trim).filter(|n| !n.is_empty()) else { continue };
                if counts.get(&name.to_ascii_lowercase()).copied().unwrap_or(0) > 1 {
                    if !unmapped.iter().any(|u| u == name) {
                        unmapped.push(name.to_string());
                    }
                    continue;
                }
                let Some(pd) = def.params.iter().find(|d| d.label.eq_ignore_ascii_case(name)).filter(|d| taken.insert(d.id)) else {
                    unmapped.push(name.to_string());
                    continue;
                };
                let conv = match pd.kind {
                    ParamKind::Float { .. } | ParamKind::Angle => Conv::Float,
                    ParamKind::Bool => Conv::Bool,
                    ParamKind::Point => Conv::SourcePoint,
                    // blend-mode popups use an enumeration of Premiere's own (0 and 18 are both
                    // common defaults in saved projects); every other popup is a 0-based index
                    ParamKind::Choice(list) if !name.eq_ignore_ascii_case("Blend Mode") => Conv::Choice(list.len()),
                    ParamKind::Color => Conv::Color,
                    _ => {
                        unmapped.push(name.to_string());
                        continue;
                    }
                };
                // only a value of the right shape maps (a slider saved as text is left alone)
                if decode(*p, conv).is_some() {
                    out.push(Mapping { node: *p, effect: def.id, param: pd.id, conv });
                } else {
                    unmapped.push(name.to_string());
                }
            }
        }
        Target::Ignore | Target::Unsupported => {}
    }
    (out, unmapped)
}

// ---------------------------------------------------------------------------------------------
// Values in Premiere's terms
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    F(f64),
    /// Normalised point.
    P(f64, f64),
    B(bool),
    /// Packed A, R, G, B, 16 bits each.
    C(u64),
}

#[derive(Clone, Debug)]
pub(crate) struct Key {
    /// Source-media ticks.
    pub t: i64,
    pub v: Val,
    /// (`f2`, `f3`).
    pub kind: (u8, u8),
    /// (in, out) influence.
    pub infl: (f64, f64),
    /// The key's original text, reused when the key is written back unchanged.
    pub raw: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Canon {
    pub value: Val,
    pub keys: Vec<Key>,
}

/// Clip timing and frame sizes needed to convert times and points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ctx {
    pub source_in: i64,
    pub speed: f64,
    pub frame: (f64, f64),
    pub src: (f64, f64),
}

impl Ctx {
    fn clip_time(&self, t: i64) -> i64 {
        let d = t.saturating_sub(self.source_in).clamp(-MAX_TIME, MAX_TIME);
        if self.speed == 1.0 { d } else { ((d as f64 / self.speed).round() as i64).clamp(-MAX_TIME, MAX_TIME) }
    }
    fn source_time(&self, clip_t: i64) -> i64 {
        let c = clip_t.clamp(-MAX_TIME, MAX_TIME);
        let d = if self.speed == 1.0 { c } else { (c as f64 * self.speed).round() as i64 };
        self.source_in.saturating_add(d).clamp(-MAX_TIME, MAX_TIME)
    }
    /// How far apart two source times may be and still be the same key after a round trip.
    fn tolerance(&self) -> i64 {
        (self.speed.ceil() as i64).max(1) + 1
    }
}

fn parse_val(s: &str, conv: Conv) -> Option<Val> {
    let s = s.trim();
    match conv {
        Conv::Float | Conv::Db => s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Val::F),
        Conv::Bool => match s {
            "true" | "1" => Some(Val::B(true)),
            "false" | "0" => Some(Val::B(false)),
            _ => None,
        },
        Conv::FramePoint | Conv::SourcePoint => {
            let (x, y) = s.split_once(':')?;
            let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
            (x.is_finite() && y.is_finite()).then_some(Val::P(x, y))
        }
        Conv::Choice(len) => {
            let n = s.trim_end_matches('.').parse::<i64>().ok()?;
            (n >= 0 && (n as usize) < len).then_some(Val::F(n as f64))
        }
        Conv::Color => s.parse::<u64>().ok().map(Val::C),
    }
}

/// Packed Premiere colour → RGBA 0..1. A channel whose low byte is clear is an 8-bit value in the
/// high byte (`0xff00` = 1.0); anything else is read as 16-bit.
fn unpack_color(c: u64) -> [f32; 4] {
    let ch = |shift: u32| {
        let field = ((c >> shift) & 0xFFFF) as u32;
        if field & 0xFF == 0 { (field >> 8) as f32 / 255.0 } else { field as f32 / 65535.0 }
    };
    [ch(32), ch(16), ch(0), ch(48)]
}

fn pack_color(rgba: [f32; 4]) -> u64 {
    let ch = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u64) << 8;
    (ch(rgba[3]) << 48) | (ch(rgba[0]) << 32) | (ch(rgba[1]) << 16) | ch(rgba[2])
}

/// The value field of a parameter's `StartKeyframe`.
pub(crate) fn start_value<'a>(p: Node<'a, '_>) -> Option<&'a str> {
    child_text(p, "StartKeyframe").and_then(|s| s.split(',').nth(1)).map(str::trim).or_else(|| child_text(p, "CurrentValue").map(str::trim))
}

/// A Premiere parameter in Premiere's terms.
pub(crate) fn decode(p: Node<'_, '_>, conv: Conv) -> Option<Canon> {
    let mut keys = Vec::new();
    if let Some(text) = child_text(p, "Keyframes") {
        for raw in text.split(';').take(MAX_KEYS) {
            let f: Vec<&str> = raw.split(',').collect();
            let (Some(t), Some(v)) = (f.first().and_then(|t| t.trim().parse::<i64>().ok()), f.get(1).and_then(|v| parse_val(v, conv))) else { continue };
            if t == STATIC_TIME {
                continue;
            }
            let num_at = |i: usize| f.get(i).and_then(|x| x.trim().parse::<f64>().ok()).filter(|x| x.is_finite());
            let code = |i: usize| f.get(i).and_then(|x| x.trim().parse::<u8>().ok()).unwrap_or(0);
            keys.push(Key {
                t: t.clamp(-MAX_TIME, MAX_TIME),
                v,
                kind: (code(2), code(3)),
                infl: (num_at(5).unwrap_or(0.0), num_at(7).unwrap_or(0.0)),
                raw: Some(raw.trim().to_string()),
            });
        }
    }
    keys.sort_by_key(|k| k.t);
    keys.dedup_by_key(|k| k.t);
    let value = start_value(p).and_then(|v| parse_val(v, conv)).or_else(|| keys.first().map(|k| k.v.clone()))?;
    Some(Canon { value, keys })
}

fn interp_of(kind: (u8, u8)) -> Interpolation {
    match kind {
        (5, 2) => Interpolation::AutoBezier,
        (5, 4) => Interpolation::ContinuousBezier,
        (5, _) => Interpolation::Bezier,
        _ => Interpolation::Linear,
    }
}

fn kind_of(i: Interpolation) -> (u8, u8) {
    match i {
        Interpolation::Linear | Interpolation::Hold => (0, 0),
        Interpolation::Bezier | Interpolation::EaseIn | Interpolation::EaseOut => (5, 0),
        Interpolation::AutoBezier => (5, 2),
        Interpolation::ContinuousBezier => (5, 4),
    }
}

fn to_value(v: &Val, conv: Conv, ctx: &Ctx) -> ParamValue {
    match (v, conv) {
        (Val::F(x), Conv::Db) => ParamValue::Float(if *x > 0.0 { (20.0 * (x / UNITY_LEVEL).log10()).max(-200.0) } else { -200.0 }),
        (Val::F(x), Conv::Choice(_)) => ParamValue::Choice(x.round().max(0.0) as u32),
        (Val::C(c), _) => ParamValue::Color(unpack_color(*c)),
        (Val::F(x), _) => ParamValue::Float(*x),
        (Val::B(b), _) => ParamValue::Bool(*b),
        (Val::P(x, y), Conv::FramePoint) => ParamValue::Vec2(Vec2::new(x * ctx.frame.0, y * ctx.frame.1)),
        (Val::P(x, y), _) => ParamValue::Vec2(Vec2::new(x * ctx.src.0, y * ctx.src.1)),
    }
}

fn from_value(v: &ParamValue, conv: Conv, ctx: &Ctx) -> Option<Val> {
    let norm = |p: f64, size: f64| if p.is_nan() || size <= 0.0 { 0.5 } else { p / size };
    Some(match (v, conv) {
        (ParamValue::Float(db), Conv::Db) => Val::F(UNITY_LEVEL * 10f64.powf(db / 20.0)),
        (ParamValue::Float(x), Conv::Float) => Val::F(*x),
        (ParamValue::Choice(n), Conv::Choice(len)) if (*n as usize) < len => Val::F(*n as f64),
        (ParamValue::Color(c), Conv::Color) => Val::C(pack_color(*c)),
        (ParamValue::Bool(b), Conv::Bool) => Val::B(*b),
        (ParamValue::Vec2(p), Conv::FramePoint) => Val::P(norm(p.x, ctx.frame.0), norm(p.y, ctx.frame.1)),
        (ParamValue::Vec2(p), Conv::SourcePoint) => Val::P(norm(p.x, ctx.src.0), norm(p.y, ctx.src.1)),
        _ => return None,
    })
}

/// Premiere → FilmCraft.
pub(crate) fn to_param(c: &Canon, conv: Conv, ctx: &Ctx) -> Param {
    let mut p = Param::new(to_value(&c.value, conv, ctx));
    for k in &c.keys {
        let mut kf = Keyframe::new(filmcraft_time::Tick(ctx.clip_time(k.t)), to_value(&k.v, conv, ctx));
        kf.interp = interp_of(k.kind);
        if kf.interp != Interpolation::Linear {
            if k.infl.0 > 0.0 {
                kf.in_influence = k.infl.0.min(1.0);
            }
            if k.infl.1 > 0.0 {
                kf.out_influence = k.infl.1.min(1.0);
            }
        }
        p.keyframes.push(kf);
    }
    p.keyframes.sort_by_key(|k| k.time);
    p.keyframes.dedup_by_key(|k| k.time);
    if let Some(k) = p.keyframes.first() {
        p.value = k.value.clone();
    }
    p
}

/// FilmCraft → Premiere. `None` when the value has another shape than the conversion expects.
pub(crate) fn from_param(p: &Param, conv: Conv, ctx: &Ctx) -> Option<Canon> {
    let value = from_value(&p.value, conv, ctx)?;
    let mut keys = Vec::with_capacity(p.keyframes.len());
    for k in &p.keyframes {
        keys.push(Key {
            t: ctx.source_time(k.time.0),
            v: from_value(&k.value, conv, ctx)?,
            kind: kind_of(k.interp),
            infl: if k.interp == Interpolation::Linear { (0.0, 0.0) } else { (k.in_influence, k.out_influence) },
            raw: None,
        });
    }
    Some(Canon { value, keys })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn same_val(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::F(x), Val::F(y)) => close(*x, *y),
        (Val::P(x1, y1), Val::P(x2, y2)) => close(*x1, *x2) && close(*y1, *y2),
        (Val::B(x), Val::B(y)) => x == y,
        (Val::C(x), Val::C(y)) => x == y,
        _ => false,
    }
}

fn same_key(a: &Key, b: &Key, tol: i64) -> bool {
    let kinds = (a.kind.0 == 5) == (b.kind.0 == 5) && (a.kind.0 != 5 || a.kind.1 == b.kind.1);
    let infl = a.kind.0 != 5 || (close(a.infl.0, b.infl.0) && close(a.infl.1, b.infl.1));
    (a.t - b.t).abs() <= tol && same_val(&a.v, &b.v) && kinds && infl
}

/// Whether FilmCraft's value (`fc`) is what was imported from Premiere (`pr`). Animated
/// parameters compare their keys only (Premiere's static value is then not used).
pub(crate) fn same(pr: &Canon, fc: &Canon, ctx: &Ctx) -> bool {
    if pr.keys.is_empty() != fc.keys.is_empty() {
        return false;
    }
    if pr.keys.is_empty() {
        return same_val(&pr.value, &fc.value);
    }
    pr.keys.len() == fc.keys.len() && pr.keys.iter().zip(&fc.keys).all(|(a, b)| same_key(a, b, ctx.tolerance()))
}

/// The value FilmCraft gets on import, or `None` where it keeps its own default (a centred
/// Position / Anchor Point, a 0 dB volume) so that untouched clips stay "auto".
pub(crate) fn import_param(m: &Mapping, ctx: &Ctx) -> Option<Param> {
    let c = decode(m.node, m.conv)?;
    if c.keys.is_empty() {
        let neutral = match (&c.value, m.conv) {
            (Val::P(x, y), Conv::FramePoint | Conv::SourcePoint) => close(*x, 0.5) && close(*y, 0.5),
            (Val::F(v), Conv::Db) => close(*v, UNITY_LEVEL),
            _ => false,
        };
        if neutral {
            return None;
        }
    }
    Some(to_param(&c, m.conv, ctx))
}

fn fmt_val(v: &Val) -> String {
    match v {
        Val::F(x) => num(*x),
        Val::P(x, y) => format!("{}:{}", num(*x), num(*y)),
        Val::B(b) => if *b { "true" } else { "false" }.to_string(),
        Val::C(c) => c.to_string(),
    }
}

/// New text for a Premiere parameter: its `StartKeyframe`, `Keyframes` (None = no keys) and
/// `CurrentValue`. Keys that match an original key keep its exact text.
pub(crate) struct ParamText {
    pub start_keyframe: String,
    pub keyframes: Option<String>,
    pub current: String,
}

pub(crate) fn encode(original: Node<'_, '_>, pr: Option<&Canon>, fc: &Canon, ctx: &Ctx) -> ParamText {
    let value = fc.keys.first().map(|k| &k.v).unwrap_or(&fc.value);
    let start = match child_text(original, "StartKeyframe") {
        Some(s) => {
            let mut f: Vec<String> = s.split(',').map(str::to_string).collect();
            if let Some(slot) = f.get_mut(1) {
                *slot = fmt_val(value);
            }
            f.join(",")
        }
        None => format!("{STATIC_TIME},{},0,0,0,0,0,0", fmt_val(value)),
    };
    let keyframes = (!fc.keys.is_empty()).then(|| {
        let mut s = String::new();
        for k in &fc.keys {
            let reuse = pr.and_then(|pr| pr.keys.iter().find(|o| same_key(o, k, ctx.tolerance()))).and_then(|o| o.raw.clone());
            match reuse {
                Some(raw) => s.push_str(&raw),
                None => s.push_str(&format!("{},{},{},{},0,{},0,{}", k.t, fmt_val(&k.v), k.kind.0, k.kind.1, num(k.infl.0), num(k.infl.1))),
            }
            s.push(';');
        }
        s
    });
    ParamText { start_keyframe: start, keyframes, current: fmt_val(value) }
}
