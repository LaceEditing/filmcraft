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
#[derive(Clone)]
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
        // Premiere's simple audio effects are "Internal …" components like the clip's own volume
        if self.audio
            && let Some(d) = AUDIO_EFFECT_GUIDS
                .iter()
                .find(|(g, _)| g.starts_with("Internal ") && g.eq_ignore_ascii_case(self.match_name))
                .and_then(|(_, id)| filmcraft_project::find_effect(id))
                .filter(|d| d.kind == EffectKind::Audio && !d.intrinsic)
        {
            return Target::Effect(d);
        }
        match self.match_name {
            "AE.ADBE Motion" => return Target::Motion,
            "AE.ADBE Opacity" => return Target::Opacity,
            m if m.starts_with("Internal Volume") => return Target::Volume,
            m if m.starts_with("Internal") => return Target::Ignore,
            "" if self.display.is_none() => return Target::Ignore,
            _ => {}
        }
        let want = if self.audio { EffectKind::Audio } else { EffectKind::Video };
        // Premiere's own audio effects are saved by a GUID, usually without a display name
        if self.audio
            && let Some(id) = AUDIO_EFFECT_GUIDS.iter().find(|(g, _)| g.eq_ignore_ascii_case(self.match_name)).map(|(_, id)| *id)
            && let Some(d) = filmcraft_project::find_effect(id).filter(|d| d.kind == want && !d.intrinsic)
        {
            return Target::Effect(d);
        }
        let name = self.label().trim();
        let base = name.strip_suffix("(Legacy)").map(str::trim).unwrap_or(name);
        match find_effect_by_name(base) {
            Some(d) if d.kind == want && !d.intrinsic => Target::Effect(d),
            _ => Target::Unsupported,
        }
    }
}

/// Premiere's audio effects by the GUID it saves as their `FilterMatchName` → FilmCraft's effect.
/// Identified from a layout kit (every audio effect applied once, at defaults) by their parameter
/// names, and where several share a parameter set (DeNoise / DeReverb, Single-band / Tube-modeled
/// Compressor) by the order Premiere lists them in; Single-band Compressor, Parametric Equalizer,
/// Studio Reverb and Automatic Click Remover also appear with their display names in projects.
pub(crate) const AUDIO_EFFECT_GUIDS: &[(&str, &str)] = &[
    ("433322b0-d25a-4cfa-a4f1-4a1ab2d7615b", "declicker"),
    ("25f78adc-442b-4221-8a59-d19d3bf14687", "dehummer"),
    ("8edcd326-33de-4a4d-985b-f1bf48ed1427", "parametric_eq"),
    ("c4ad21d8-9748-4d79-b4a3-d88ab7e4be34", "analog_delay"),
    ("c5ca9e29-df07-42a6-8800-ba80324e5cbc", "channel_mixer_a"),
    ("1ffc7dd8-df1b-475b-bff6-f5240aa6a097", "chorus_flanger"),
    ("99c2e75f-0bdb-48d8-8b51-2bac2e26b2cf", "convolution_reverb"),
    ("ffbe710f-cd69-4139-ad26-65603616d9d4", "deesser"),
    ("9b66ab13-4d1b-469a-88c6-b807fcf1df16", "distortion"),
    ("f76dad9c-77da-477a-9939-eefefc5c23bd", "dynamics"),
    ("4f327230-f04c-4c34-9ea5-a998b4459221", "fft_filter"),
    ("d30b6731-3e00-4a11-9650-e816f58a9d12", "flanger"),
    ("3792d46f-5036-4be6-ab5a-177622246a5d", "graphic_eq"),
    ("0e133ef5-2371-48db-9b72-b4c441460391", "graphic_eq_20"),
    ("c6ba8ed5-e0d5-4ed9-a209-08adec9b18a9", "graphic_eq_30"),
    ("c23a8acb-affc-4ff4-9ca6-2b560b340a56", "guitar_suite"),
    ("e0b23f05-f1a7-4ef7-9b50-7ec3e3002058", "hard_limiter"),
    ("43f746b7-b34e-402b-8fb4-479cf3466264", "loudness_radar"),
    ("094e8ed0-1fcc-4bec-ac4a-b358c4ce1f27", "mastering"),
    ("3981e750-c8ae-40a2-889f-41a01c7efa03", "multiband_compressor"),
    ("ee38cdc5-5914-4a75-b7dd-73a265e55c31", "notch"),
    ("1eee85a7-3087-439e-906d-dc6204330e25", "dynamics_rack"),
    ("d7ca44f4-7ac1-4243-a79e-59e283b71cca", "phaser"),
    ("164b4e8a-7105-406b-b23b-1ef2cc4d8957", "pitch_shifter"),
    ("4d0a6193-2769-4497-a28e-30e42d1bc376", "scientific_filter"),
    ("2da3e2bc-5005-481c-bbeb-7689d3169edf", "single_band_compressor"),
    ("f527fd12-24bf-411f-a46e-5b997cec32d4", "tube_compressor"),
    ("fe767733-0969-46cf-bf45-fbf3d58ec46a", "stereo_expander"),
    ("7ea99d31-ca8f-454f-acd7-81a258efb336", "studio_reverb"),
    ("6e496ba0-3dbc-4c41-a272-7071daff0d95", "surround_reverb"),
    ("82882adc-18cb-475b-b237-597576a5e707", "vocal_enhancer"),
    ("c6762838-e451-4d22-9c90-b0eaa734605a", "denoise"),
    ("aeb0b5ea-5024-4f02-8097-81f8fd7aba92", "dereverb"),
    // Amplify (Left / Right gain), not Channel Volume: told apart by Premiere's own UI (Round 2)
    ("f14fee7d-fe5f-4202-b790-f53209e56411", "amplify"),
    // Premiere 26.5's Loudness Radar is saved under this plug-in style name
    ("3:24e81ecf 935d4253 26f4e4c3 3a945a38", "loudness_radar"),
    // the simple audio effects, saved by an internal name
    ("Internal Channel Volume", "channel_volume_a"),
    ("Internal Bass", "bass"),
    ("Internal Treble", "treble"),
    ("Internal Delay", "delay"),
    ("Internal Highpass", "highpass"),
    ("Internal Lowpass", "lowpass"),
    ("Internal Bandpass", "bandpass"),
    ("Internal Bandreject", "simple_notch"),
    ("Internal Parameteric EQ", "simple_eq"),
    ("Internal MultitapDelay", "multitap_delay"),
    ("Internal Invert", "invert_a"),
    ("Internal Audio Balance", "balance_a"),
    ("Internal Audio Fill Left", "fill_left"),
    ("Internal Audio Fill Right", "fill_right"),
    ("Internal Audio Swap Channels", "swap_channels"),
];

/// How Premiere's audio effect settings (all saved as 0…1) read in FilmCraft's units: (FilmCraft
/// effect, Premiere parameter name, FilmCraft parameter, conversion). Each scale was measured in
/// Premiere 26.5 by setting the saved number to 0, 0.5 and 1 and reading what Premiere shows
/// (Round 2 audio scales). Settings not listed keep Premiere's value when saving back.
pub(crate) const AUDIO_PARAMS: &[(&str, &str, &str, Conv)] = &[
    ("single_band_compressor", "Threshold", "threshold", Conv::Lin(-60.0, 0.0)),
    ("single_band_compressor", "Ratio", "ratio", Conv::Lin(1.0, 30.0)),
    ("single_band_compressor", "Attack", "attack", Conv::Lin(0.0, 500.0)),
    ("single_band_compressor", "Release", "release", Conv::Lin(0.0, 5000.0)),
    ("single_band_compressor", "Gain", "output", Conv::Lin(-30.0, 30.0)),
    ("tube_compressor", "Threshold", "threshold", Conv::Lin(-60.0, 0.0)),
    ("tube_compressor", "Ratio", "ratio", Conv::Lin(1.0, 30.0)),
    ("tube_compressor", "Gain", "output", Conv::Lin(-30.0, 30.0)),
    ("hard_limiter", "Maximum Amplitude", "max", Conv::Lin(-100.0, 0.0)),
    ("hard_limiter", "Input Boost", "boost", Conv::Lin(-100.0, 50.0)),
    ("hard_limiter", "Look-Ahead Time", "lookahead", Conv::Lin(5.0, 20.0)),
    ("hard_limiter", "Release Time", "release", Conv::Lin(40.0, 200.0)),
    ("hard_limiter", "Link Channels", "link", Conv::Flag(1)),
    ("declicker", "Threshold", "threshold", Conv::Lin(1.0, 100.0)),
    ("declicker", "Complexity", "complexity", Conv::Lin(1.0, 100.0)),
    ("denoise", "Amount", "amount", Conv::Lin(0.0, 100.0)),
    ("dynamics_rack", "AutoGate Enabled", "gate_on", Conv::Flag(1)),
    ("dynamics_rack", "AutoGate Threshold", "gate_threshold", Conv::Lin(-60.0, 0.0)),
    ("dynamics_rack", "Compressor Enabled", "comp_on", Conv::Flag(1)),
    ("dynamics_rack", "Compressor Threshold", "comp_threshold", Conv::Lin(-60.0, 0.0)),
    ("dynamics_rack", "Compressor Ratio", "comp_ratio", Conv::Lin(1.0, 8.0)),
    ("dynamics_rack", "Limiter Enabled", "lim_on", Conv::Flag(1)),
    ("dynamics_rack", "Limiter Threshold", "lim_threshold", Conv::Lin(-60.0, 0.0)),
    ("multiband_compressor", "Threshold 1", "b1_threshold", Conv::Lin(-60.0, 0.0)),
    ("multiband_compressor", "Threshold 2", "b2_threshold", Conv::Lin(-60.0, 0.0)),
    ("multiband_compressor", "Threshold 3", "b3_threshold", Conv::Lin(-60.0, 0.0)),
    ("multiband_compressor", "Threshold 4", "b4_threshold", Conv::Lin(-60.0, 0.0)),
    ("multiband_compressor", "Ratio 1", "b1_ratio", Conv::Lin(1.0, 30.0)),
    ("multiband_compressor", "Ratio 2", "b2_ratio", Conv::Lin(1.0, 30.0)),
    ("multiband_compressor", "Ratio 3", "b3_ratio", Conv::Lin(1.0, 30.0)),
    ("multiband_compressor", "Ratio 4", "b4_ratio", Conv::Lin(1.0, 30.0)),
    ("multiband_compressor", "Gain 1", "b1_gain", Conv::Lin(-18.0, 18.0)),
    ("multiband_compressor", "Gain 2", "b2_gain", Conv::Lin(-18.0, 18.0)),
    ("multiband_compressor", "Gain 3", "b3_gain", Conv::Lin(-18.0, 18.0)),
    ("multiband_compressor", "Gain 4", "b4_gain", Conv::Lin(-18.0, 18.0)),
    ("studio_reverb", "Room Size", "room", Conv::Lin(1.0, 100.0)),
    ("studio_reverb", "Dry Output Level", "dry", Conv::Lin(0.0, 100.0)),
    ("studio_reverb", "Wet Output Level", "wet", Conv::Lin(0.0, 100.0)),
    ("amplify", "Left", "gain", Conv::Lin(-96.0, 48.0)),
    ("amplify", "Right", "gain", Conv::Lin(-96.0, 48.0)),
    // linear gain on the clip volume's scale (15 dB at 1, 0 dB at 0.1778…)
    ("channel_volume_a", "Left", "left", Conv::FxDb),
    ("channel_volume_a", "Right", "right", Conv::FxDb),
    ("bass", "Boost", "boost", Conv::Lin(-24.0, 24.0)),
    ("treble", "Boost", "boost", Conv::Lin(-24.0, 24.0)),
    ("delay", "Delay", "delay", Conv::Lin(0.0, 2.0)),
    ("delay", "Feedback", "feedback", Conv::Lin(0.0, 100.0)),
    ("delay", "Mix", "mix", Conv::Lin(0.0, 100.0)),
    // 10 Hz at 0, 5 950 Hz at 0.5, 23 770 Hz at 1: the square of the saved number
    ("highpass", "Cutoff", "cutoff", Conv::Sq(10.0, 23770.0)),
    ("lowpass", "Cutoff", "cutoff", Conv::Sq(10.0, 23770.0)),
    ("bandpass", "Cutoff", "center", Conv::Sq(10.0, 23770.0)),
    ("pitch_shifter", "Transpose Ratio", "semitones", Conv::Ratio),
    ("loudness_radar", "Target Loudness", "target", Conv::Lin(-36.0, -6.0)),
    ("parametric_eq", "Gain", "master_gain", Conv::Lin(-96.0, 48.0)),
    ("parametric_eq", "Low Shelf Enable", "low_on", Conv::Flag(1)),
    ("parametric_eq", "Low Shelf Frequency", "low_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "Low Shelf Gain", "low_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "High Shelf Enable", "high_on", Conv::Flag(1)),
    ("parametric_eq", "High Shelf Frequency", "high_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "High Shelf Gain", "high_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 1 Enable", "b1_on", Conv::Flag(1)),
    ("parametric_eq", "EQ Band 1 Center Frequency", "b1_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "EQ Band 1 Gain", "b1_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 1 Q", "b1_q", Conv::Lin(0.0, 10000.0)),
    ("parametric_eq", "EQ Band 2 Enable", "b2_on", Conv::Flag(1)),
    ("parametric_eq", "EQ Band 2 Center Frequency", "b2_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "EQ Band 2 Gain", "b2_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 2 Q", "b2_q", Conv::Lin(0.0, 10000.0)),
    ("parametric_eq", "EQ Band 3 Enable", "mid_on", Conv::Flag(1)),
    ("parametric_eq", "EQ Band 3 Center Frequency", "mid_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "EQ Band 3 Gain", "mid_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 3 Q", "mid_q", Conv::Lin(0.0, 10000.0)),
    ("parametric_eq", "EQ Band 4 Enable", "b4_on", Conv::Flag(1)),
    ("parametric_eq", "EQ Band 4 Center Frequency", "b4_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "EQ Band 4 Gain", "b4_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 4 Q", "b4_q", Conv::Lin(0.0, 10000.0)),
    ("parametric_eq", "EQ Band 5 Enable", "b5_on", Conv::Flag(1)),
    ("parametric_eq", "EQ Band 5 Center Frequency", "b5_freq", Conv::Lin(20.0, 24000.0)),
    ("parametric_eq", "EQ Band 5 Gain", "b5_gain", Conv::Lin(-48.0, 48.0)),
    ("parametric_eq", "EQ Band 5 Q", "b5_q", Conv::Lin(0.0, 10000.0)),
];

/// Class of an audio effect's numeric settings; one at 1.0 is saved without a value.
const AUDIO_PARAM_CLASS: &str = "a714635e";

/// Whether FilmCraft reads this audio effect's settings (else it comes in switched off).
pub(crate) fn audio_effect_read(id: &str) -> bool {
    AUDIO_PARAMS.iter().any(|(e, ..)| *e == id) || matches!(id, "invert_a" | "fill_left" | "fill_right" | "swap_channels")
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
    /// A number FilmCraft holds multiplied by this factor (Premiere's Gaussian Blur "Amount" ↔
    /// FilmCraft's Blurriness).
    Scale(f64),
    /// A popup whose value `on` is FilmCraft's switch on, any other value off (the current
    /// Gaussian Blur's Edge Behavior: 1, its default, repeats the edge pixels; written 1 / 0).
    Flag(i64),
    /// A 0…1 number standing for `lo + (hi − lo)·n` (Premiere's audio effect settings).
    Lin(f64, f64),
    /// A 0…1 number standing for `lo + (hi − lo)·n²` (audio filter cutoffs).
    Sq(f64, f64),
    /// Pitch Shifter's Transpose Ratio (0…1 for a ratio of 0.5…2) ↔ semitones.
    Ratio,
    /// An audio effect's gain on the clip volume's scale ([`Conv::Db`]: 0 dB at 0.1778…, +15 dB
    /// at 1).
    FxDb,
}

/// Match name of Premiere's current Gaussian Blur (the one that is not "Legacy").
pub(crate) const IMPACT_BLUR: &str = "AE.Impact_Blur_FX";
/// Blurriness per unit of that blur's Amount. Measured on a Premiere render of Amount 2, 5, 20
/// and 50 (a Gaussian in linear light, as the legacy blur): σ ≈ 1.624 px per unit of Amount
/// (least squares through 0; 3.37, 8.24, 32.9 and 81.0 px). FilmCraft's Blurriness is
/// σ / 0.285, so Blurriness = Amount × 1.624 / 0.285.
pub(crate) const IMPACT_BLUR_AMOUNT: f64 = 1.624 / 0.285;

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
        Target::Effect(def) if def.id == "gaussian_blur" && c.match_name == IMPACT_BLUR => {
            // Premiere's current Gaussian Blur: Amount (and Thickness, the same with Uniform Blur
            // on, as in every project seen) is the strength; the rest has no FilmCraft equivalent
            add(named("Amount"), "gaussian_blur", "blurriness", Conv::Scale(IMPACT_BLUR_AMOUNT));
            // Edge Behavior 1 (the default) keeps the edges opaque, as Repeat Edge Pixels
            add(named("Edge Behavior"), "gaussian_blur", "repeat_edge", Conv::Flag(1));
            for (name, neutral) in [("Uniform Blur", "true"), ("Chromatic Aberration", "0")] {
                if named(name).and_then(start_value).is_some_and(|v| v.trim_end_matches('.') != neutral) {
                    unmapped.push(name.to_string());
                }
            }
        }
        // Premiere saves every setting of its audio effects normalized to 0…1 (Maximum Amplitude
        // −0.1 dB is 0.999, an Attack of a few ms 0.02), on scales not measured yet: none is read
        // or written as if it were in the effect's units
        Target::Effect(def) if def.kind == EffectKind::Audio => {
            for p in &c.params {
                let Some(name) = child_text(*p, "Name").map(str::trim).filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("Bypass")) else { continue };
                match AUDIO_PARAMS.iter().find(|(e, n, ..)| *e == def.id && n.eq_ignore_ascii_case(name)) {
                    Some((_, _, param, conv)) if decode(*p, *conv).is_some() => out.push(Mapping { node: *p, effect: def.id, param, conv: *conv }),
                    _ => {
                        if !unmapped.iter().any(|u| u == name) {
                            unmapped.push(name.to_string());
                        }
                    }
                }
            }
        }
        Target::Effect(def) => {
            // the legacy Gaussian Blur's Repeat Edge Pixels (parameter 3) is saved without a name
            // by current Premiere versions (on by default: 6 601 of 6 601 in the projects seen)
            if def.id == "gaussian_blur" {
                let repeat = c.params.iter().copied().find(|p| {
                    child_text(*p, "Name").is_none_or(|n| n.trim().is_empty())
                        && child_text(*p, "ParameterID").map(str::trim) == Some("3")
                        && start_value(*p).is_some_and(|v| v == "true" || v == "false")
                });
                if let Some(n) = repeat {
                    out.push(Mapping { node: n, effect: def.id, param: "repeat_edge", conv: Conv::Bool });
                }
            }
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
    pub(crate) fn clip_time(&self, t: i64) -> i64 {
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
        Conv::Float | Conv::Db | Conv::FxDb | Conv::Scale(_) | Conv::Lin(..) | Conv::Sq(..) | Conv::Ratio => {
            s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Val::F)
        }
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
        Conv::Flag(on) => s.trim_end_matches('.').parse::<i64>().ok().map(|n| Val::F(if n == on { on as f64 } else { 0.0 })),
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
    // an audio effect setting at 1 (the top of its 0…1 range) is saved with no value at all
    let audio_top = || {
        (matches!(conv, Conv::Lin(..) | Conv::Sq(..) | Conv::Ratio | Conv::FxDb | Conv::Flag(_))
            && p.attribute("ClassID").is_some_and(|c| c.starts_with(AUDIO_PARAM_CLASS)))
        .then_some(Val::F(1.0))
    };
    let value = start_value(p).and_then(|v| parse_val(v, conv)).or_else(|| keys.first().map(|k| k.v.clone())).or_else(audio_top)?;
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
        (Val::F(x), Conv::Db | Conv::FxDb) => ParamValue::Float(if *x > 0.0 { (20.0 * (x / UNITY_LEVEL).log10()).max(-200.0) } else { -200.0 }),
        (Val::F(x), Conv::Choice(_)) => ParamValue::Choice(x.round().max(0.0) as u32),
        (Val::F(x), Conv::Flag(on)) => ParamValue::Bool(x.round() as i64 == on),
        (Val::C(c), _) => ParamValue::Color(unpack_color(*c)),
        (Val::F(x), Conv::Scale(k)) => ParamValue::Float(x * k),
        (Val::F(n), Conv::Lin(lo, hi)) => ParamValue::Float(lo + (hi - lo) * n.clamp(0.0, 1.0)),
        (Val::F(n), Conv::Sq(lo, hi)) => ParamValue::Float(lo + (hi - lo) * n.clamp(0.0, 1.0).powi(2)),
        (Val::F(n), Conv::Ratio) => ParamValue::Float(12.0 * (0.5 + 1.5 * n.clamp(0.0, 1.0)).log2()),
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
        (ParamValue::Float(db), Conv::FxDb) => Val::F((UNITY_LEVEL * 10f64.powf(db.min(15.0) / 20.0)).clamp(0.0, 1.0)),
        (ParamValue::Float(x), Conv::Float) => Val::F(*x),
        (ParamValue::Float(x), Conv::Scale(k)) if k != 0.0 && k.is_finite() => Val::F(x / k),
        (ParamValue::Float(x), Conv::Lin(lo, hi)) if hi != lo => Val::F(((x - lo) / (hi - lo)).clamp(0.0, 1.0)),
        (ParamValue::Float(x), Conv::Sq(lo, hi)) if hi != lo => Val::F(((x - lo) / (hi - lo)).clamp(0.0, 1.0).sqrt()),
        (ParamValue::Float(semis), Conv::Ratio) => Val::F(((2f64.powf(semis.clamp(-12.0, 12.0) / 12.0) - 0.5) / 1.5).clamp(0.0, 1.0)),
        (ParamValue::Choice(n), Conv::Choice(len)) if (*n as usize) < len => Val::F(*n as f64),
        (ParamValue::Color(c), Conv::Color) => Val::C(pack_color(*c)),
        (ParamValue::Bool(b), Conv::Bool) => Val::B(*b),
        (ParamValue::Bool(b), Conv::Flag(on)) => Val::F(if *b { on as f64 } else { 0.0 }),
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

/// A keyframe value as Premiere writes it: a float parameter's whole number with a trailing point
/// (`150.`, never `150`), a menu choice as a plain integer (`18`).
fn fmt_key_val(v: &Val, float: bool) -> String {
    match v {
        Val::F(x) if float && x.is_finite() && *x == x.round() && x.abs() < 1e15 => format!("{}.", *x as i64),
        other => fmt_val(other),
    }
}

/// Whether a parameter holds a float (Premiere writes its keyframe values with a point): its
/// original value says so, else its class (the float and anti-flicker sliders).
fn is_float_param(original: Node<'_, '_>) -> bool {
    match child_text(original, "StartKeyframe").and_then(|s| s.split(',').nth(1)) {
        Some(v) => v.contains('.') || v.contains('e'),
        None => original.attribute("ClassID").is_some_and(|c| c.starts_with("fe47129e") || c.starts_with("a4ff2d6e")),
    }
}

pub(crate) fn encode(original: Node<'_, '_>, pr: Option<&Canon>, fc: &Canon, ctx: &Ctx) -> ParamText {
    let value = fc.keys.first().map(|k| &k.v).unwrap_or(&fc.value);
    let float = is_float_param(original);
    let start = match child_text(original, "StartKeyframe") {
        Some(s) => {
            let mut f: Vec<String> = s.split(',').map(str::to_string).collect();
            if let Some(slot) = f.get_mut(1) {
                *slot = fmt_key_val(value, float);
            }
            f.join(",")
        }
        None => format!("{STATIC_TIME},{},0,0,0,0,0,0", fmt_key_val(value, float)),
    };
    let keyframes = (!fc.keys.is_empty()).then(|| {
        let mut s = String::new();
        for k in &fc.keys {
            let reuse = pr.and_then(|pr| pr.keys.iter().find(|o| same_key(o, k, ctx.tolerance()))).and_then(|o| o.raw.clone());
            match reuse {
                Some(raw) => s.push_str(&raw),
                None => s.push_str(&format!("{},{},{},{},0,{},0,{}", k.t, fmt_key_val(&k.v, float), k.kind.0, k.kind.1, num(k.infl.0), num(k.infl.1))),
            }
            s.push(';');
        }
        s
    });
    ParamText { start_keyframe: start, keyframes, current: fmt_val(value) }
}
