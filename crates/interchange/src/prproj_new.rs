//! Create a Premiere Pro project (`.prproj`) for a FilmCraft project that did not come from one.
//!
//! Like the rest of the `.prproj` support, this follows what Premiere Pro writes, as observed in
//! projects it saved (see the crate README); no application internals were inspected. FilmCraft
//! first builds a *seed* document with the objects a project saved by Premiere has before any
//! clip is placed: the project and its settings, the bin tree, every media file (project item,
//! master clip, media, streams, audio channels) and every sequence (track groups, tracks, the
//! audio mixer's tracks, faders and panners), plus, for each media file and nested sequence a
//! sequence uses, a template clip per kind of track carrying Premiere's intrinsic Motion /
//! Opacity or Volume / Channel Volume components. The sequences are then written into the seed by
//! [`crate::prproj_write::export`], the code that writes back into projects opened from Premiere:
//! it places a copy of a template for every clip, sets its timing, speed, frame hold, Motion,
//! Opacity and volume (with keyframes), links the clips and checks the result (every reference
//! resolves; importing it again gives back exactly the clips written). The templates themselves,
//! which no track uses, are dropped last.
//!
//! Effects and transitions need Premiere's own layout of their component (parameter ids,
//! classes, bounds), which only projects Premiere saved have. [`CreateOptions::models`] passes such
//! projects (the ones the user opened in FilmCraft): for each effect or transition the project uses,
//! one component or transition is copied from them into the seed (with its parameters, renumbered)
//! and the writer models clips on it as it does in projects opened from Premiere. Only effects all
//! of whose parameters FilmCraft maps are borrowed, so no setting of the borrowed clip leaks in.
//!
//! Not written (each listed in the report): effects and transitions with no model, graphics /
//! titles, adjustment layers, generated media, captions and track effects.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::ops::Range;

use filmcraft_media::MediaKind;
use filmcraft_project::{AudioChannels, Bin, BinEntry, ItemId, ItemKind, Marker, MediaRef, Project, TrackKind};
use roxmltree::Node;

use crate::prproj::{META_MEDIA, META_SEQUENCE};
use crate::prproj_graph::{self, TICKS_PER_SECOND};
use crate::xml::{escape, num};
use crate::{Error, Report, Result};

const VIDEO_MEDIA_TYPE: &str = "228cda18-3625-4d2d-951e-348879e4ed93";
const AUDIO_MEDIA_TYPE: &str = "80b8e3d5-6dca-4195-aefb-cb5f407ab009";
const DATA_MEDIA_TYPE: &str = "d8143ffe-eec4-4d2a-a909-d5f7bf094dc5";
/// The importer Premiere records for ordinary media files (movies, audio, stills).
const IMPORTER: &str = "1fa18bfa-255c-44b1-ad73-56bcd99fceaf";
/// `StartKeyframe` time of a static value.
const NO_TIME: &str = "-91445760000000000";
const STEREO: &str = r#"[{"channellabel":100},{"channellabel":101}]"#;
const MONO: &str = r#"[{"channellabel":0}]"#;
/// Premiere's length of a still's media (12 hours).
const STILL_DURATION: i64 = 12 * 3600 * TICKS_PER_SECOND;
/// Clip volume at 0 dB as Premiere stores it (linear, +15 dB of headroom above 1.0).
const UNITY_LEVEL: &str = "0.177827939391";
/// A track fader's upper bound (+15 dB, linear).
const FADER_MAX: &str = "5.6234130859375";
/// Premiere's "Custom" sequence editing mode (any frame size and rate) and its preview preset.
const CUSTOM_MODE: &str = "9678af98-a7b7-4bdb-b477-7ac9c8df4a4e";
/// The component id Premiere gives every panner.
const PANNER_ID: &str = "4294967280";
/// Length of the template clips (the writer retimes every copy).
const TEMPLATE_LENGTH: i64 = TICKS_PER_SECOND;
/// Channel Volume has a level per possible channel (Left, Right and 30 more, as Premiere saves it).
const CHANNEL_VOLUME_SLOTS: usize = 32;

/// Class ids (and object versions) of the objects the seed writes.
mod class {
    pub const PROJECT: (&str, u32) = ("62ad66dd-0dcd-42da-a660-6d8fbde94876", 45);
    pub const ROOT_ITEM: (&str, u32) = ("1c307a89-9318-47d7-a583-bf2553736543", 1);
    pub const BIN_ITEM: (&str, u32) = ("dbfd6653-24da-480e-a35e-ba45e9504e4b", 1);
    pub const CLIP_ITEM: (&str, u32) = ("cb4e0ed7-aca1-4171-8525-e3658dec06dd", 1);
    pub const PROJECT_SETTINGS: (&str, u32) = ("50c16708-a1a1-4d2f-98d5-4e283ae28353", 21);
    pub const COMPILE_SETTINGS: (&str, u32) = ("18a35d66-597e-4157-b783-938b5bec3547", 4);
    pub const VIDEO_COMPILE: (&str, u32) = ("db372db5-7de2-4d3c-98ae-f42659d77b22", 9);
    pub const AUDIO_COMPILE: (&str, u32) = ("34b10007-ab6d-49a7-bac5-7b60d919e387", 6);
    pub const VIDEO_SETTINGS: (&str, u32) = ("58474264-30c4-43a2-bba5-dc0812df8a3a", 10);
    pub const AUDIO_SETTINGS: (&str, u32) = ("6baf5521-b132-4634-840e-13cec5bc86a4", 8);
    pub const SCRATCH_DISKS: (&str, u32) = ("4c6ed82b-a81c-4df1-8bd0-750504c4b560", 4);
    pub const INGEST: (&str, u32) = ("2db8f76b-2c37-48ee-925d-9a4f7278152d", 2);
    pub const WORKSPACE: (&str, u32) = ("c4372273-e1aa-4683-98aa-a2ceadf3066c", 1);
    pub const CAPTURE: (&str, u32) = ("328c2aa2-47f9-4211-805b-b6a6dbd4ca29", 1);
    pub const DEFAULT_SEQUENCE: (&str, u32) = ("567bdf53-d6d9-4d61-b2f1-f4834bebea9b", 2);
    pub const MASTER_CLIP: (&str, u32) = ("fb11c33a-b0a9-4465-aa94-b6d5db2628cf", 12);
    pub const LOGGING: (&str, u32) = ("77ab7fdd-dcdf-465d-9906-7a330ca1e738", 10);
    pub const VIDEO_CLIP: (&str, u32) = ("9308dbef-2440-4acb-9ab2-953b9a4e82ec", 11);
    pub const AUDIO_CLIP: (&str, u32) = ("b8830d03-de02-41ee-84ec-fe566dc70cd9", 8);
    pub const MARKERS: (&str, u32) = ("bee50706-b524-416c-9f03-b596ce5f6866", 4);
    pub const MARKER: (&str, u32) = ("a45508e0-3ff7-4d04-90a7-2e0dfff4c910", 3);
    pub const VIDEO_MEDIA_SOURCE: (&str, u32) = ("e64ddf74-8fac-4682-8aa8-0e0ca2248949", 2);
    pub const AUDIO_MEDIA_SOURCE: (&str, u32) = ("f588da05-fc2a-4fbc-9383-74d653b379e3", 2);
    pub const VIDEO_SEQUENCE_SOURCE: (&str, u32) = ("4752dfa9-7a7e-4a3b-a25b-cafde1a8d036", 3);
    pub const AUDIO_SEQUENCE_SOURCE: (&str, u32) = ("e8d4cc83-38cb-491f-9d94-e5f7e3b205ee", 7);
    pub const SECONDARY_CONTENT: (&str, u32) = ("f9d004b5-cb04-4e2f-af6f-64fadc2c4be9", 1);
    pub const CHANNEL_GROUPS: (&str, u32) = ("a3127a8c-95d4-456e-a7f5-171b3f922426", 1);
    pub const CHANNEL_VECTOR: (&str, u32) = ("333d203b-3a53-4195-8894-fc7523ff3dc7", 1);
    pub const CHANNEL: (&str, u32) = ("5c89aa7a-89a6-4483-becd-f2b1def42316", 1);
    pub const MEDIA: (&str, u32) = ("7a5c103e-f3ac-4391-b6b4-7cc3d2f9a7ff", 30);
    pub const VIDEO_STREAM: (&str, u32) = ("a36e4719-3ec6-4a0c-ab11-8b4aab377aa5", 23);
    pub const AUDIO_STREAM: (&str, u32) = ("0b5cf52f-2b85-4863-890b-8844b64ecfe9", 8);
    pub const SEQUENCE: (&str, u32) = ("6a15d903-8739-11d5-af2d-9b7855ad8974", 12);
    pub const VIDEO_TRACK_GROUP: (&str, u32) = ("9e9abf7a-0918-49c2-91ae-991b5dde77bb", 13);
    pub const AUDIO_TRACK_GROUP: (&str, u32) = ("9b9238b9-53a8-4cc3-b03f-b36246d052e6", 6);
    pub const DATA_TRACK_GROUP: (&str, u32) = ("b714b71d-6838-48dd-9b77-db19088ced7e", 1);
    pub const VIDEO_TRACK: (&str, u32) = ("f68dcd81-8805-11d5-af2d-9bfa89d4ddd4", 1);
    pub const AUDIO_TRACK: (&str, u32) = ("097f6203-99ae-11d5-84f2-8cf14bde7040", 7);
    pub const MIX_TRACK: (&str, u32) = ("4b1d8400-e89e-11d5-abc4-a1a13b1e80a0", 4);
    pub const TRACK_INLET: (&str, u32) = ("be3af080-e8c6-11d5-abc4-a1c6d5dee670", 4);
    pub const VIDEO_CHAIN: (&str, u32) = ("0970e08a-f58f-4108-b29a-1a717b8e12e2", 3);
    pub const AUDIO_CHAIN: (&str, u32) = ("3cb131d1-d3c0-47ae-a19a-bdf75ea11674", 4);
    pub const FADER: (&str, u32) = ("1a38c583-ed5c-11d5-abc4-c1cbf61ec590", 3);
    pub const METER: (&str, u32) = ("72ea4700-f615-11d5-abc4-c186585e63e0", 2);
    pub const STEREO_PANNER: (&str, u32) = ("7bf86a01-efbe-11d5-abc4-c1ce2b1e9090", 1);
    pub const DEFAULT_PANNER: (&str, u32) = ("33a94282-ee2c-11d5-abc4-c1cd7f9e3c10", 2);
    pub const AUDIO_PARAM: (&str, u32) = ("a714635e-a628-4b27-9d59-77eba47dbc1a", 10);
    pub const AUDIO_BOOL_PARAM: (&str, u32) = ("32657501-3aa4-445f-a49b-d09ecb9fa1ae", 10);
    pub const AUDIO_FILTER: (&str, u32) = ("d77a90a0-6c9e-44bf-9b20-de8c21168fe1", 4);
    pub const VIDEO_FILTER: (&str, u32) = ("d10da199-beea-4dd1-b941-ed3a78766d50", 9);
    pub const POINT_PARAM: (&str, u32) = ("ca81d347-309b-44d2-acc7-1c572efb973c", 4);
    pub const FLOAT_PARAM: (&str, u32) = ("fe47129e-6c94-4fc0-95d5-c056a517aaf3", 10);
    pub const BOOL_PARAM: (&str, u32) = ("cc12343e-f113-4d3b-ae05-b287db77d461", 10);
    pub const SLIDER_PARAM: (&str, u32) = ("a4ff2d6e-7ac2-44f8-9d52-17d9ca50e542", 10);
    pub const POPUP_PARAM: (&str, u32) = ("6e02e8bb-2569-46b2-8ab1-4ab11c43e9c8", 10);
    pub const VIDEO_TRACK_ITEM: (&str, u32) = ("368b0406-29e3-4923-9fcd-094fbf9a1089", 8);
    pub const AUDIO_TRACK_ITEM: (&str, u32) = ("064ec682-9ba6-11d5-af2d-9ca32c7d6164", 11);
    pub const SUB_CLIP: (&str, u32) = ("e0c58dc9-dbdd-4166-aef7-5db7e3f22e84", 6);
}

/// Where the new project goes and how its media paths are written.
#[derive(Clone, Debug, Default)]
pub struct CreateOptions {
    /// The folder the project is saved in: media paths are also written relative to it, which is
    /// where Premiere looks first when a project moved.
    pub dest_dir: Option<String>,
    /// (local prefix, Premiere prefix) pairs, e.g. `("/mnt/LocalDisk", "L:")`: a media path under
    /// a local prefix is written the way the machine running Premiere sees it (`L:\Editing\a.mp4`).
    pub path_map: Vec<(String, String)>,
    /// Premiere projects (`.prproj` bytes) to borrow effect and transition layouts from.
    pub models: Vec<Vec<u8>>,
}

/// The `.prproj` (gzip-compressed) for every sequence of `project`, and what could not be written.
pub fn create(project: &Project, opts: &CreateOptions) -> Result<(Vec<u8>, Report)> {
    let mut report = Report::default();
    let needs = needs(project);
    let grafts = if needs.is_empty() { Vec::new() } else { find_models(&opts.models, &needs) };
    let have: HashSet<Need> = grafts.iter().map(|g| g.what.clone()).collect();
    let mut work = prepare(project, &mut report, &have);
    if !work.items.values().any(|i| i.as_sequence().is_some()) {
        return Err(Error::Other("Premiere Pro project: the project has no sequence to write".into()));
    }
    let mut seed = Seed::new(&work, opts);
    seed.build();
    for g in &grafts {
        seed.graft(g);
    }
    if !grafts.is_empty() {
        let effects = grafts.iter().filter(|g| matches!(g.what, Need::Effect(_))).count();
        let transitions = grafts.len() - effects;
        report.info(format!("{effects} effect(s) and {transitions} transition type(s) were laid out as in your Premiere projects"));
    }
    let (media_uids, sequence_uids) = (std::mem::take(&mut seed.media_uid), std::mem::take(&mut seed.sequence_uid));
    report.entries.extend(std::mem::take(&mut seed.report.entries));
    let text = seed.finish();
    // what the writer needs to find each item in the seed
    for (id, uid) in media_uids {
        if let Some(it) = work.item_mut(id) {
            it.metadata.insert(META_MEDIA.to_string(), uid);
        }
    }
    for (id, uid) in sequence_uids {
        if let Some(it) = work.item_mut(id) {
            it.metadata.insert(META_SEQUENCE.to_string(), uid);
        }
    }
    let (bytes, written) = crate::prproj_write::export(&work, text.as_bytes(), &[], opts.dest_dir.as_deref())?;
    // every effect and transition of a new project is modelled on a borrowed layout: the writer's
    // notes saying so for each clip are no news here
    let modelled = |e: &crate::ReportEntry| e.level == crate::Level::Info && (e.message.starts_with("added \"") || e.message.starts_with("added a "));
    report.entries.extend(written.entries.into_iter().filter(|e| !modelled(e)));
    let (xml, _) = prproj_graph::decompress(&bytes)?;
    let xml = String::from_utf8(xml).map_err(|_| Error::Other("Premiere Pro project: the written project is not UTF-8".into()))?;
    let pruned = prune(&xml)?;
    if let Some(p) = crate::prproj_write::problems(&pruned).into_iter().next() {
        return Err(Error::Other(format!("Premiere Pro project: the new project would have an unresolved {p}")));
    }
    Ok((prproj_graph::compress(pruned.as_bytes()), report))
}

// ---------------------------------------------------------------------------------------------
// What can be written
// ---------------------------------------------------------------------------------------------

/// The project as it will be written: Premiere ids of an earlier import dropped, subclip clips
/// playing their media, and what a new Premiere project cannot hold taken out (and reported).
fn prepare(project: &Project, report: &mut Report, have: &HashSet<Need>) -> Project {
    let mut work = project.clone();
    for it in work.items.values_mut() {
        it.metadata.retain(|k, _| !k.starts_with("premiere."));
    }
    // what each item plays in Premiere: a media file, a sequence, or nothing it can hold
    let mut target: HashMap<ItemId, std::result::Result<ItemId, &'static str>> = HashMap::new();
    for id in project.items.keys() {
        target.insert(*id, playable(project, *id));
    }
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut effects: BTreeMap<String, usize> = BTreeMap::new();
    let mut unwritten: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut transitions = 0usize;
    let mut track_settings = 0usize;
    let mut captions = 0usize;
    let mut submixes = 0usize;
    for it in work.items.values_mut() {
        let ItemKind::Sequence(seq) = &mut it.kind else { continue };
        captions += seq.caption_tracks.len();
        submixes += seq.submix_tracks.len();
        for t in seq.all_tracks_mut() {
            let audio = t.kind == TrackKind::Audio;
            let before = t.transitions.len();
            t.transitions.retain(|tr| have.contains(&Need::transition(&tr.effect, audio)));
            transitions += before - t.transitions.len();
            if !t.effects.is_empty() || t.pan.abs() > 1e-9 || t.muted || t.solo {
                track_settings += 1;
            }
            t.items.retain_mut(|c| match target.get(&c.item).copied().unwrap_or(Err("missing items")) {
                Ok(play) => {
                    c.item = play;
                    true
                }
                Err(what) => {
                    *skipped.entry(what).or_insert(0) += 1;
                    false
                }
            });
            for c in &mut t.items {
                // ids from another Premiere project mean nothing here
                c.origin = None;
                c.effects.retain(|e| match filmcraft_project::effect::find_effect(&e.effect) {
                    Some(d) if d.intrinsic || have.contains(&Need::Effect(d.id)) => true,
                    d => {
                        *effects.entry(d.map(|d| d.name.to_string()).unwrap_or_else(|| e.effect.clone())).or_insert(0) += 1;
                        false
                    }
                });
                for e in &c.effects {
                    if !e.masks.is_empty() {
                        *unwritten.entry("masks").or_insert(0) += 1;
                    }
                    let changed = |p: &str| {
                        let def =
                            filmcraft_project::effect::find_effect(&e.effect).and_then(|d| d.params.iter().find(|x| x.id == p)).map(|x| x.default.clone());
                        e.params.get(p).is_some_and(|v| !v.keyframes.is_empty() || Some(&v.value) != def.as_ref())
                    };
                    match e.effect.as_str() {
                        "time_remap" if e.params.keys().any(|p| changed(p)) => *unwritten.entry("time remapping").or_insert(0) += 1,
                        "channel_volume" | "panner" if e.params.keys().any(|p| changed(p)) => {
                            *unwritten.entry("clip panning and channel volume").or_insert(0) += 1
                        }
                        "opacity" if changed("blend") => *unwritten.entry("blend modes").or_insert(0) += 1,
                        _ => {}
                    }
                }
                if c.gain_db.abs() > 1e-9 {
                    *unwritten.entry("audio gain").or_insert(0) += 1;
                }
                if !c.enabled {
                    *unwritten.entry("disabled clips (written enabled)").or_insert(0) += 1;
                }
            }
        }
    }
    for (what, n) in &skipped {
        report.warn(format!("{n} clip(s) of {what} were not written: a new Premiere project holds media files and sequences only"));
    }
    if !effects.is_empty() {
        let list: Vec<String> = effects.iter().map(|(name, n)| format!("{name} ({n} clip{})", if *n == 1 { "" } else { "s" })).collect();
        report.warn(format!(
            "effects left out (none of your Premiere projects FilmCraft has opened has one to copy Premiere's layout from, with every setting FilmCraft knows): {}",
            list.join(", ")
        ));
    }
    for (what, n) in &unwritten {
        report.warn(format!("{what} on {n} clip(s) not written"));
    }
    if transitions > 0 {
        report.warn(format!("{transitions} transition(s) left out (none of your Premiere projects FilmCraft has opened has that transition to copy)"));
    }
    if track_settings > 0 {
        report.warn(format!("track effects, pan, mute and solo of {track_settings} track(s) were not written (track volume was)"));
    }
    if captions > 0 {
        report.warn(format!("{captions} caption track(s) were not written"));
    }
    if submixes > 0 {
        report.warn(format!("{submixes} submix track(s) were not written"));
    }
    work
}

/// What a clip of `item` plays in the new project.
fn playable(p: &Project, item: ItemId) -> std::result::Result<ItemId, &'static str> {
    let mut id = item;
    // subclips (of subclips) play their media from the same source times; bounded, a damaged
    // project could point a subclip at itself
    for _ in 0..16 {
        let Some(it) = p.item(id) else { return Err("missing items") };
        return match &it.kind {
            ItemKind::Media(m) => match m.media {
                MediaRef::File { .. } => Ok(id),
                MediaRef::Generator(_) => Err("generated media (bars, colour mattes…)"),
            },
            ItemKind::Sequence(_) => Ok(id),
            ItemKind::Subclip { parent, .. } => {
                id = *parent;
                continue;
            }
            ItemKind::AdjustmentLayer { .. } => Err("adjustment layers"),
            ItemKind::Graphic { .. } => Err("graphics and titles"),
        };
    }
    Err("subclips")
}

// ---------------------------------------------------------------------------------------------
// Layouts borrowed from Premiere projects
// ---------------------------------------------------------------------------------------------

/// An effect or transition the project uses, which needs Premiere's layout to be written.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Need {
    /// A FilmCraft effect (by id).
    Effect(&'static str),
    /// A transition (lower-case display name, audio).
    Transition(String, bool),
}

impl Need {
    fn transition(e: &filmcraft_project::EffectInstance, audio: bool) -> Need {
        Need::Transition(crate::common::transition_name(e).to_lowercase(), audio)
    }
}

/// Every non-intrinsic effect and transition in the project's sequences.
fn needs(project: &Project) -> BTreeSet<Need> {
    let mut out = BTreeSet::new();
    for seq in project.sequences().filter_map(|i| i.as_sequence()) {
        for t in seq.all_tracks() {
            for tr in &t.transitions {
                out.insert(Need::transition(&tr.effect, t.kind == TrackKind::Audio));
            }
            for c in &t.items {
                for e in &c.effects {
                    if let Some(d) = filmcraft_project::effect::find_effect(&e.effect).filter(|d| !d.intrinsic) {
                        out.insert(Need::Effect(d.id));
                    }
                }
            }
        }
    }
    out
}

/// One object of a borrowed layout: its XML and where its id, references and missing binary
/// payloads are (byte ranges within `text`).
struct GraftObj {
    text: String,
    key: String,
    id_at: Range<usize>,
    refs: Vec<(Range<usize>, String)>,
    /// Elements whose binary payload is stored elsewhere in their document: replaced by a copy
    /// that carries it.
    payloads: Vec<(Range<usize>, String)>,
}

/// A component (effect) or transition track item with the objects only it owns, first object
/// first.
struct Graft {
    what: Need,
    objects: Vec<GraftObj>,
}

/// Find a layout for each need in `models` (earlier projects first; a model without animated
/// parameters is preferred).
fn find_models(models: &[Vec<u8>], needs: &BTreeSet<Need>) -> Vec<Graft> {
    // (need → (score, graft)); score 0 is a model with no animated parameter
    let mut best: BTreeMap<Need, (u8, Graft)> = BTreeMap::new();
    for bytes in models {
        if needs.iter().all(|n| best.get(n).is_some_and(|b| b.0 == 0)) {
            break;
        }
        let Ok((raw, _)) = prproj_graph::decompress(bytes) else { continue };
        let Ok(text) = std::str::from_utf8(&raw) else { continue };
        let Ok(doc) = crate::xml::parse(text) else { continue };
        let root = doc.root_element();
        if root.tag_name().name() != "PremiereData" {
            continue;
        }
        let ix = prproj_graph::Index::new(root);
        let mut payloads: Option<HashMap<&str, &str>> = None;
        for n in root.children().filter(Node::is_element) {
            let tag = n.tag_name().name();
            let need = match tag {
                "VideoFilterComponent" | "AudioFilterComponent" => {
                    let Some(c) = crate::prproj_fx::component(&ix, n) else { continue };
                    let target = c.target();
                    let crate::prproj_fx::Target::Effect(def) = target else { continue };
                    // every setting maps onto FilmCraft's, so none of the model's leaks in
                    let (_, unmapped) = crate::prproj_fx::mappings(&c, target);
                    if !unmapped.iter().all(|name| holds_no_setting(&c, name)) {
                        continue;
                    }
                    Need::Effect(def.id)
                }
                "VideoTransitionTrackItem" | "AudioTransitionTrackItem" => {
                    let Some(name) = crate::xml::path_text(n, &["TransitionTrackItem", "DisplayName"]) else { continue };
                    Need::Transition(name.trim().to_lowercase(), tag.starts_with("Audio"))
                }
                _ => continue,
            };
            if !needs.contains(&need) || best.get(&need).is_some_and(|b| b.0 == 0) {
                continue;
            }
            let payloads = payloads.get_or_insert_with(|| binary_payloads(root));
            let Some((objects, animated)) = extract(&ix, text, n, payloads) else { continue };
            let score = u8::from(animated);
            if best.get(&need).is_none_or(|b| score < b.0) {
                best.insert(need.clone(), (score, Graft { what: need, objects }));
            }
        }
    }
    best.into_values().map(|(_, g)| g).collect()
}

/// Whether every parameter of `c` named `name` is something other than a setting: a group
/// heading (control type 11, which only stores whether it is expanded), an audio effect's
/// processing state, or a Bypass that is off.
fn holds_no_setting(c: &crate::prproj_fx::Component<'_, '_>, name: &str) -> bool {
    let named = c.params.iter().filter(|p| crate::xml::child_text(**p, "Name").is_some_and(|n| n.trim().eq_ignore_ascii_case(name)));
    named.into_iter().all(|p| {
        let heading = crate::xml::child_text(*p, "ParameterControlType").is_some_and(|t| t.trim() == "11");
        let state = matches!(name, "IsProcessing" | "Is Processing" | "Reset");
        let off = name.eq_ignore_ascii_case("Bypass") && crate::prproj_fx::start_value(*p).is_none_or(|v| v == "false" || v == "0");
        heading || state || off
    })
}

/// Binary payloads of a document by hash (the element that carries the data).
fn binary_payloads<'a>(root: Node<'a, '_>) -> HashMap<&'a str, &'a str> {
    let mut out = HashMap::new();
    for n in root.descendants().filter(Node::is_element) {
        if let (Some(h), Some(t)) = (n.attribute("BinaryHash"), n.text().filter(|t| !t.trim().is_empty())) {
            out.entry(h).or_insert(t);
        }
    }
    out
}

/// `root` and the parameters and components it refers to, as owned XML; `None` when it refers to
/// anything else (a layout that is not self-contained) or is too large. Also says whether any
/// parameter is animated.
fn extract(ix: &prproj_graph::Index<'_, '_>, text: &str, root: Node<'_, '_>, payloads: &HashMap<&str, &str>) -> Option<(Vec<GraftObj>, bool)> {
    let mut order: Vec<Node> = vec![root];
    let mut seen: HashSet<&str> = root.attribute("ObjectID").into_iter().collect();
    root.attribute("ObjectID")?;
    let mut i = 0;
    let mut animated = false;
    while let Some(obj) = order.get(i).copied() {
        i += 1;
        if order.len() > 4000 {
            return None;
        }
        for r in crate::prproj_write::public_refs(obj) {
            if r.attribute("ObjectURef").is_some() {
                return None;
            }
            let Some(id) = r.attribute("ObjectRef") else { continue };
            let target = ix.ids.get(id).copied()?;
            let tag = target.tag_name().name();
            if !(tag.ends_with("ComponentParam") || tag == "VideoFilterComponent" || tag == "AudioFilterComponent") {
                return None;
            }
            if seen.insert(id) {
                animated |=
                    crate::xml::child_text(target, "IsTimeVarying").is_some_and(|v| v.trim() == "true") || crate::xml::child(target, "Keyframes").is_some();
                order.push(target);
            }
        }
    }
    let mut out = Vec::with_capacity(order.len());
    for obj in order {
        let r = obj.range();
        let base = r.start;
        let text_of = text.get(r.clone())?.to_string();
        let attr = obj.attributes().find(|a| a.name() == "ObjectID")?;
        let rel = |x: Range<usize>| x.start.checked_sub(base).zip(x.end.checked_sub(base)).map(|(s, e)| s..e);
        let id_at = rel(attr.range_value())?;
        let mut refs = Vec::new();
        for n in crate::prproj_write::public_refs(obj) {
            if let Some(a) = n.attributes().find(|a| a.name() == "ObjectRef") {
                refs.push((rel(a.range_value())?, a.value().to_string()));
            }
        }
        let mut fills = Vec::new();
        for n in obj.descendants().filter(Node::is_element) {
            let Some(h) = n.attribute("BinaryHash") else { continue };
            if n.text().is_some_and(|t| !t.trim().is_empty()) {
                continue;
            }
            let data = payloads.get(h)?;
            let src = text.get(n.range())?;
            let name = n.tag_name().name();
            let open = match src.find('>') {
                Some(p) if src.get(..p).is_some_and(|x| x.ends_with('/')) => format!("{}>", src.get(..p - 1)?),
                Some(p) => src.get(..=p)?.to_string(),
                None => return None,
            };
            fills.push((rel(n.range())?, format!("{open}{data}</{name}>")));
        }
        out.push(GraftObj { text: text_of, key: attr.value().to_string(), id_at, refs, payloads: fills });
    }
    Some((out, animated))
}

// ---------------------------------------------------------------------------------------------
// The seed document
// ---------------------------------------------------------------------------------------------

/// A media file's or sequence's objects that clips refer to.
#[derive(Clone, Default)]
struct Sources {
    master: String,
    name: String,
    video: Option<u64>,
    audio: Option<u64>,
    markers: u64,
    /// Picture size (video).
    size: (u32, u32),
    /// The audio clips' layout: mono or stereo.
    mono: bool,
    /// Ticks per audio sample.
    sample: i64,
}

struct Seed<'p> {
    project: &'p Project,
    opts: &'p CreateOptions,
    objs: Vec<String>,
    next_id: u64,
    hash: u64,
    report: Report,
    /// Item → its ClipProjectItem UID.
    project_items: HashMap<ItemId, String>,
    sources: HashMap<ItemId, Sources>,
    media_uid: Vec<(ItemId, String)>,
    sequence_uid: Vec<(ItemId, String)>,
}

fn attrs(c: (&str, u32)) -> String {
    format!("ClassID=\"{}\" Version=\"{}\"", c.0, c.1)
}

fn el(name: &str, value: &str) -> String {
    format!("<{name}>{}</{name}>", escape(value))
}

/// A list element (`<Items Version="1"><Item Index="0" ObjectURef="…"/>…</Items>`); `uref` picks
/// `ObjectURef` over `ObjectRef`.
fn list<T: std::fmt::Display>(tag: &str, item: &str, refs: &[T], uref: bool) -> String {
    let attr = if uref { "ObjectURef" } else { "ObjectRef" };
    let items: String = refs.iter().enumerate().map(|(i, r)| format!("<{item} Index=\"{i}\" {attr}=\"{r}\"/>")).collect();
    format!("<{tag} Version=\"1\">{items}</{tag}>")
}

/// Ticks per audio sample.
fn sample_ticks(rate: u32) -> i64 {
    TICKS_PER_SECOND / i64::from(rate.max(1))
}

impl<'p> Seed<'p> {
    fn new(project: &'p Project, opts: &'p CreateOptions) -> Self {
        // UUIDs differ between exports (two new projects imported into one Premiere project must
        // not share ids); `RandomState` is random per process where the platform has entropy
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write(project.name.as_bytes());
        h.write_usize(project.items.len());
        Seed {
            project,
            opts,
            objs: Vec::new(),
            next_id: 1,
            hash: h.finish() | 1,
            report: Report::default(),
            project_items: HashMap::new(),
            sources: HashMap::new(),
            media_uid: Vec::new(),
            sequence_uid: Vec::new(),
        }
    }

    fn id(&mut self) -> u64 {
        let i = self.next_id;
        self.next_id += 1;
        i
    }

    /// A version-4 UUID (splitmix64).
    fn uuid(&mut self) -> String {
        let mut next = || {
            self.hash = self.hash.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = self.hash;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let (a, b) = (next(), next());
        let a = (a & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
        let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xFFFF, a & 0xFFFF, b >> 48, b & 0xFFFF_FFFF_FFFF)
    }

    /// An object with an `ObjectID`.
    fn obj(&mut self, tag: &str, c: (&str, u32), body: &str) -> u64 {
        let id = self.id();
        self.objs.push(format!("\t<{tag} ObjectID=\"{id}\" {}>{body}</{tag}>\n", attrs(c)));
        id
    }

    /// An object with an `ObjectUID`.
    fn uobj(&mut self, uid: &str, tag: &str, c: (&str, u32), body: &str) {
        self.objs.push(format!("\t<{tag} ObjectUID=\"{uid}\" {}>{body}</{tag}>\n", attrs(c)));
    }

    fn finish(self) -> String {
        let mut s = String::with_capacity(self.objs.iter().map(String::len).sum::<usize>() + 128);
        s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<PremiereData Version=\"3\">\n\t<Project ObjectRef=\"1\"/>\n");
        for o in &self.objs {
            s.push_str(o);
        }
        s.push_str("</PremiereData>\n");
        s
    }

    fn build(&mut self) {
        // the Project object comes first, as Premiere writes it; it is filled in last
        let project_id = self.id();
        let at = self.objs.len();
        self.objs.push(String::new());
        let parts = self.settings();
        let project = self.project;
        let first = project.sequences().filter_map(|i| i.as_sequence()).map(|s| s.settings.clone()).next();
        let rate = first.as_ref().map(|s| s.frame_rate).unwrap_or_default();
        // what the sequences use each item as (media whose streams FilmCraft does not know, being
        // offline, still get the streams their clips need)
        let mut uses: HashMap<ItemId, (bool, bool)> = HashMap::new();
        for seq in project.sequences().filter_map(|i| i.as_sequence()) {
            for t in seq.all_tracks() {
                for c in &t.items {
                    let u = uses.entry(c.item).or_insert((false, false));
                    if t.kind == TrackKind::Video {
                        u.0 = true;
                    } else {
                        u.1 = true;
                    }
                }
            }
        }
        let mut channels_noted = false;
        for it in project.items.values() {
            if let ItemKind::Media(m) = &it.kind
                && let MediaRef::File { path } = &m.media
            {
                let ch = m.info.audio.as_ref().map(|a| a.channels).unwrap_or(0);
                if ch > 2 && !channels_noted {
                    self.report.warn("audio with more than two channels was written as stereo (its first two channels)");
                    channels_noted = true;
                }
                let mut info = m.info.clone();
                let (video, audio) = uses.get(&it.id).copied().unwrap_or((false, false));
                if video && info.video.is_none() {
                    let (w, h) = first.as_ref().map(|s| (s.width, s.height)).unwrap_or((1920, 1080));
                    info.video = Some(filmcraft_media::VideoStreamInfo {
                        width: w,
                        height: h,
                        frame_rate: rate,
                        par: (1, 1),
                        codec: String::new(),
                        pixel_format: String::new(),
                        color: filmcraft_color::ColorInfo::REC709,
                        has_alpha: false,
                        bitrate: None,
                        hdr: None,
                    });
                }
                if audio && info.audio.as_ref().is_none_or(|a| a.channels == 0) && info.kind != MediaKind::Still {
                    let sample_rate = first.as_ref().map(|s| s.sample_rate).filter(|r| *r > 0).unwrap_or(48_000);
                    info.audio = Some(filmcraft_media::AudioStreamInfo { sample_rate, channels: 2, codec: String::new(), bits_per_sample: None });
                }
                self.media(it.id, &it.name, path, &info, &m.markers, rate);
            }
        }
        // sequences: UIDs first (a sequence's clips can play another one)
        let seqs: Vec<ItemId> = project.items.values().filter(|i| i.as_sequence().is_some()).map(|i| i.id).collect();
        let uids: HashMap<ItemId, String> = seqs.iter().map(|id| (*id, self.uuid())).collect();
        for id in &seqs {
            if let Some(uid) = uids.get(id) {
                self.sequence(*id, uid);
            }
        }
        if project.sequences().filter_map(|i| i.as_sequence()).flat_map(|s| s.audio_tracks.iter()).any(|t| t.channels != AudioChannels::Stereo) {
            self.report.warn("mono, 5.1 and adaptive audio tracks were written as stereo tracks");
        }
        self.templates();
        // the bin tree, in FilmCraft's order; items no bin lists go to the root
        let mut listed = HashSet::new();
        let mut root_items = self.bin_children(&project.root, &mut listed);
        let mut skipped = 0;
        for it in project.items.values() {
            if listed.contains(&it.id) {
                continue;
            }
            match self.project_items.get(&it.id) {
                Some(uid) => root_items.push(uid.clone()),
                None if !matches!(it.kind, ItemKind::Graphic { .. }) => skipped += 1,
                None => {}
            }
        }
        skipped += listed
            .iter()
            .filter(|i| !self.project_items.contains_key(i) && !matches!(project.item(**i).map(|x| &x.kind), Some(ItemKind::Graphic { .. })))
            .count();
        if skipped > 0 {
            self.report.info(format!(
                "{skipped} project item(s) that are not media files or sequences (subclips, generated media, adjustment layers) were left out of the bins"
            ));
        }
        let root_uid = self.uuid();
        let items = if root_items.is_empty() { String::new() } else { list("Items", "Item", &root_items, true) };
        self.uobj(
            &root_uid,
            "RootProjectItem",
            class::ROOT_ITEM,
            &format!(
                "<ProjectItem Version=\"1\"><Node Version=\"1\"><ID>1000000</ID></Node><Name>Root Bin</Name></ProjectItem><ProjectItemContainer Version=\"1\">{items}</ProjectItemContainer>"
            ),
        );
        let p = parts;
        if let Some(slot) = self.objs.get_mut(at) {
            *slot = format!(
                "\t<Project ObjectID=\"{project_id}\" {}><RootProjectItem ObjectURef=\"{root_uid}\"/><ProjectSettings ObjectRef=\"{}\"/><MovieCompileSettings ObjectRef=\"{}\"/><StillCompileSettings ObjectRef=\"{}\"/><AudioCompileSettings ObjectRef=\"{}\"/><CustomCompileSettings ObjectRef=\"{}\"/><VideoPreviewCompileSettings ObjectRef=\"{}\"/><ScratchDiskSettings ObjectRef=\"{}\"/><IngestSettings ObjectRef=\"{}\"/><ProjectWorkspace ObjectRef=\"{}\"/><NextID>1000001</NextID></Project>\n",
                attrs(class::PROJECT),
                p.settings,
                p.compile[0],
                p.compile[1],
                p.compile[2],
                p.compile[3],
                p.compile[4],
                p.scratch,
                p.ingest,
                p.workspace
            );
        }
    }

    // ----------------------------------------------------------------------------- settings

    fn video_compile(&mut self) -> u64 {
        let vs = self.obj("VideoSettings", class::VIDEO_SETTINGS, "");
        self.obj(
            "VideoCompileSettings",
            class::VIDEO_COMPILE,
            &format!(
                "<VideoSettings ObjectRef=\"{vs}\"/><VideoFieldType>0</VideoFieldType><Compressor>1685288558</Compressor><VideoCompilerClassIDFourCC>1061109567</VideoCompilerClassIDFourCC><VideoFileTypeFourCC>1096173910</VideoFileTypeFourCC><Depth>24</Depth><RenderDepth>0</RenderDepth><Aspect43>false</Aspect43><Quality>100</Quality><UseDataRate>false</UseDataRate><DataRate>3500</DataRate><ForceRecompress>true</ForceRecompress><ForceRecompressValue>2</ForceRecompressValue><Deinterlace>false</Deinterlace><IgnoreVideoFilters>false</IgnoreVideoFilters><OptimizeStills>false</OptimizeStills><FramesAtMarkers>false</FramesAtMarkers><RealTimePreview>true</RealTimePreview><DoKeyframeEveryNFrames>false</DoKeyframeEveryNFrames><DoKeyframeEveryNFramesValue>0</DoKeyframeEveryNFramesValue><AddKeyframesAtMarkers>false</AddKeyframesAtMarkers><AddKeyframesAtEdits>false</AddKeyframesAtEdits><RelativeFrameSize>1</RelativeFrameSize>"
            ),
        )
    }

    fn audio_compile(&mut self) -> u64 {
        let a = self.obj("AudioSettings", class::AUDIO_SETTINGS, "");
        self.obj(
            "AudioCompileSettings",
            class::AUDIO_COMPILE,
            &format!("<AudioSettings ObjectRef=\"{a}\"/><Interleave>1</Interleave><SampleType>3</SampleType><Compressor>1380013856</Compressor>"),
        )
    }

    /// The settings objects a new Premiere project has.
    fn settings(&mut self) -> ProjectParts {
        let vs = self.obj("VideoSettings", class::VIDEO_SETTINGS, "");
        let au = self.obj("AudioSettings", class::AUDIO_SETTINGS, "");
        let vc = self.video_compile();
        let ac = self.audio_compile();
        let capture = self.obj("DummyCaptureSettings", class::CAPTURE, "");
        let defseq = self.obj(
            "DefaultSequenceSettings",
            class::DEFAULT_SEQUENCE,
            "<TotalVideoTracks>3</TotalVideoTracks><DefaultAudioStandardMonoTracks>0</DefaultAudioStandardMonoTracks><DefaultAudioStandardStereoTracks>3</DefaultAudioStandardStereoTracks><DefaultAudioStandard51Tracks>0</DefaultAudioStandard51Tracks><DefaultAudioSubmixMonoTracks>0</DefaultAudioSubmixMonoTracks><DefaultAudioSubmixStereoTracks>0</DefaultAudioSubmixStereoTracks><DefaultAudioSubmix51Tracks>0</DefaultAudioSubmix51Tracks>",
        );
        let settings = self.obj(
            "ProjectSettings",
            class::PROJECT_SETTINGS,
            &format!(
                "<VideoSettings ObjectRef=\"{vs}\"/><AudioSettings ObjectRef=\"{au}\"/><VideoCompileSettings ObjectRef=\"{vc}\"/><AudioCompileSettings ObjectRef=\"{ac}\"/><CaptureSettings ObjectRef=\"{capture}\"/><DefaultSequenceSettings ObjectRef=\"{defseq}\"/>{}<AudioTimeDisplay>200</AudioTimeDisplay><VideoTimeDisplay>102</VideoTimeDisplay><ColorAwareEffectsEnabled>0</ColorAwareEffectsEnabled><VideoTimeDisplayInitial>102</VideoTimeDisplayInitial><ActionSafeWidth>10</ActionSafeWidth><ActionSafeHeight>10</ActionSafeHeight><TitleSafeWidth>20</TitleSafeWidth><TitleSafeHeight>20</TitleSafeHeight><ShouldScaleMedia>false</ShouldScaleMedia><EditingModeID>00000000-0000-0000-0000-000000000000</EditingModeID><PreviewFileFormatID>00000000-0000-0000-0000-000000000000</PreviewFileFormatID><UsePreviewCache>false</UsePreviewCache>",
                el("ColorManagementSettings", r#"{"enableLogColorManagement":2,"graphicsWhiteLuminance":203,"lutInterpolationMethod":1}"#)
            ),
        );
        let mut compile = [0u64; 5];
        for (i, c) in compile.iter_mut().enumerate() {
            let v = self.video_compile();
            let a = self.audio_compile();
            // the last one (Video Preview) names its compiler, as Premiere's does
            let (class_cc, cc) = if i == 4 { ("1061109567", "1096173910") } else { ("0", "0") };
            *c = self.obj(
                "CompileSettings",
                class::COMPILE_SETTINGS,
                &format!(
                    "<VideoCompileSettings ObjectRef=\"{v}\"/><AudioCompileSettings ObjectRef=\"{a}\"/><CompilerClassIDFourCC>{class_cc}</CompilerClassIDFourCC><CompilerFourCC>{cc}</CompilerFourCC><ExportVideo>true</ExportVideo><ExportAudio>true</ExportAudio><AddToProjectWhenFinished>true</AddToProjectWhenFinished><BeepWhenFinished>false</BeepWhenFinished><ExportWorkAreaOnly>false</ExportWorkAreaOnly><EmbedProjectLink>false</EmbedProjectLink>"
                ),
            );
        }
        let scratch = self.obj(
            "ScratchDiskSettings",
            class::SCRATCH_DISKS,
            "<AudioPreviewLocation0>SameAsProject</AudioPreviewLocation0><VideoPreviewLocation0>SameAsProject</VideoPreviewLocation0><TransferMediaLocation0>SameAsProject</TransferMediaLocation0><CapsuleMediaLocation0>SameAsProject</CapsuleMediaLocation0><CCLibrariesLocation0>SameAsProject</CCLibrariesLocation0><DVDEncodingLocation0>SameAsProject</DVDEncodingLocation0><AutoSaveLocation0>SameAsProject</AutoSaveLocation0><CapturedVideoLocation0>SameAsProject</CapturedVideoLocation0>",
        );
        let ingest = self.obj("IngestSettings", class::INGEST, "<Action>copy</Action><Enabled>false</Enabled>");
        let workspace = self.obj("WorkspaceSettings", class::WORKSPACE, "");
        ProjectParts { settings, compile, scratch, ingest, workspace }
    }

    // ----------------------------------------------------------------------------- shared parts

    /// A `Markers` object holding `markers`.
    fn markers(&mut self, markers: &[Marker]) -> u64 {
        let mut entries = String::new();
        let mut sorted: Vec<&Marker> = markers.iter().collect();
        sorted.sort_by_key(|m| m.start);
        for (i, m) in sorted.into_iter().enumerate() {
            let guid = self.uuid();
            let json = crate::prproj_write::marker_json(m, &guid);
            let id = self.obj("Marker", class::MARKER, &el("DVAMarker", &json));
            entries.push_str(&format!("<Marker Version=\"1\" Index=\"{i}\"><First>{guid}</First><Second ObjectRef=\"{id}\"/></Marker>"));
        }
        let list = if entries.is_empty() { String::new() } else { format!("<Markers Version=\"1\">{entries}</Markers>") };
        self.obj(
            "Markers",
            class::MARKERS,
            &format!(
                "{list}<ByGUID>byGUID</ByGUID><LastMetadataState>00000000-0000-0000-0000-000000000000</LastMetadataState><LastContentState>00000000-0000-0000-0000-000000000000</LastContentState>"
            ),
        )
    }

    /// `SecondaryContent` objects, one per channel of `source`.
    fn secondary(&mut self, source: u64, channels: usize) -> Vec<u64> {
        (0..channels)
            .map(|c| self.obj("SecondaryContent", class::SECONDARY_CONTENT, &format!("<Content ObjectRef=\"{source}\"/><ChannelIndex>{c}</ChannelIndex>")))
            .collect()
    }

    /// The master clip's audio channel groups: one clip of `channels` channels.
    fn channel_groups(&mut self, channels: Option<usize>) -> u64 {
        let Some(n) = channels else { return self.obj("ClipChannelGroupVectorSerializer", class::CHANNEL_GROUPS, "") };
        let chans: Vec<u64> = (0..n)
            .map(|c| {
                self.obj(
                    "ClipChannelSerializer",
                    class::CHANNEL,
                    &format!("<SourceClipIndex>0</SourceClipIndex><mSourceChannelIndex>{c}</mSourceChannelIndex>"),
                )
            })
            .collect();
        let kind = if n == 1 { 0 } else { 1 };
        let vector = self.obj(
            "ClipChannelVectorSerializer",
            class::CHANNEL_VECTOR,
            &format!("{}<ChannelType>{kind}</ChannelType>", list("ClipChannels", "ClipChannelItem", &chans, false)),
        );
        self.obj("ClipChannelGroupVectorSerializer", class::CHANNEL_GROUPS, &list("ClipChannelVectors", "ClipChannelVectorItem", &[vector], false))
    }

    /// A clip chain with Premiere's default (implicit) volume, as master clips have.
    fn default_audio_chain(&mut self, sample: i64, mono: bool) -> u64 {
        let (layout, kind, channel_volume) =
            if mono { (MONO, 0, "") } else { (STEREO, 1, "<DefaultChannelVolumeComponentID>2</DefaultChannelVolumeComponentID>") };
        self.obj(
            "AudioComponentChain",
            class::AUDIO_CHAIN,
            &format!(
                "<DefaultVol>true</DefaultVol><DefaultVolumeComponentID>1</DefaultVolumeComponentID>{channel_volume}<ComponentChain Version=\"3\"></ComponentChain><FrameRate>{sample}</FrameRate>{}<ChannelType>{kind}</ChannelType>",
                el("AudioChannelLayout", layout)
            ),
        )
    }

    fn clip_body(markers: u64, source: u64, range: Option<(i64, i64)>, clip_id: &str, master: bool) -> String {
        let range = range.map(|(i, o)| format!("<OutPoint>{o}</OutPoint><InPoint>{i}</InPoint>")).unwrap_or_default();
        let in_use = if master { "<InUse>false</InUse>" } else { "" };
        format!(
            "<Clip Version=\"18\"><MarkerOwner Version=\"1\"><Markers ObjectRef=\"{markers}\"/></MarkerOwner><Source ObjectRef=\"{source}\"/>{range}<ClipID>{clip_id}</ClipID>{in_use}</Clip>"
        )
    }

    fn video_clip(&mut self, markers: u64, source: u64, range: Option<(i64, i64)>, master: bool) -> u64 {
        let clip_id = self.uuid();
        let body = Self::clip_body(markers, source, range, &clip_id, master);
        self.obj("VideoClip", class::VIDEO_CLIP, &body)
    }

    fn audio_clip(&mut self, markers: u64, source: u64, range: Option<(i64, i64)>, mono: bool, master: bool) -> u64 {
        let channels = self.secondary(source, if mono { 1 } else { 2 });
        let clip_id = self.uuid();
        let body = format!(
            "{}{}{}",
            Self::clip_body(markers, source, range, &clip_id, master),
            list("SecondaryContents", "SecondaryContentItem", &channels, false),
            el("AudioChannelLayout", if mono { MONO } else { STEREO })
        );
        self.obj("AudioClip", class::AUDIO_CLIP, &body)
    }

    fn project_item(&mut self, item: ItemId, name: &str, master: &str) {
        let uid = self.uuid();
        self.uobj(
            &uid,
            "ClipProjectItem",
            class::CLIP_ITEM,
            &format!("<ProjectItem Version=\"1\"><Node Version=\"1\"><Properties Version=\"1\"></Properties></Node>{}</ProjectItem><MasterClip ObjectURef=\"{master}\"/>", el("Name", name)),
        );
        self.project_items.insert(item, uid);
    }

    // ----------------------------------------------------------------------------- media

    /// A media file's project item and the objects behind it.
    fn media(&mut self, item: ItemId, name: &str, path: &str, info: &filmcraft_media::MediaInfo, markers: &[Marker], seq_rate: filmcraft_time::FrameRate) {
        let still = info.kind == MediaKind::Still;
        let file = crate::common::file_name(path).to_string();
        let video_rate = info.video.as_ref().map(|v| if still { seq_rate } else { v.frame_rate }).map(|r| r.frame_duration().0);
        let audio = info.audio.as_ref().filter(|a| a.channels > 0 && !still);
        let sample = audio.map(|a| sample_ticks(a.sample_rate));
        let mono = audio.is_some_and(|a| a.channels == 1);
        let length = if still { STILL_DURATION } else { info.duration.0.max(0) };
        // a still's master clip spans FilmCraft's still duration
        let still_range = still.then_some((0, if info.duration.0 > 0 { info.duration.0 } else { 5 * TICKS_PER_SECOND }));

        let master_uid = self.uuid();
        let media_uid = self.uuid();
        let markers_id = self.markers(markers);
        let logging = match (still, video_rate.or(sample)) {
            (true, Some(r)) => format!("{}<MediaFrameRate>{r}</MediaFrameRate>", el("ClipName", name)),
            (false, Some(r)) => {
                format!("{}<MediaInPoint>0</MediaInPoint><MediaOutPoint>{length}</MediaOutPoint><MediaFrameRate>{r}</MediaFrameRate>", el("ClipName", name))
            }
            (_, None) => el("ClipName", name),
        };
        let logging = self.obj("ClipLoggingInfo", class::LOGGING, &logging);
        let chain = sample.map(|s| self.default_audio_chain(s, mono));
        let mut clips = Vec::new();
        let mut src =
            Sources { master: master_uid.clone(), name: name.to_string(), markers: markers_id, mono, sample: sample.unwrap_or(0), ..Default::default() };
        let media_source = |s: &mut Self, c: (&str, u32), tag: &str| {
            s.obj(tag, c, &format!("<MediaSource Version=\"4\"><Content Version=\"10\"></Content><Media ObjectURef=\"{media_uid}\"/></MediaSource><OriginalDuration>{length}</OriginalDuration>"))
        };
        if let Some(v) = &info.video {
            let source = media_source(self, class::VIDEO_MEDIA_SOURCE, "VideoMediaSource");
            clips.push(self.video_clip(markers_id, source, still_range, true));
            src.video = Some(source);
            src.size = (v.width, v.height);
        }
        let mut streams = String::new();
        if let (Some(a), Some(s)) = (audio, sample) {
            let source = media_source(self, class::AUDIO_MEDIA_SOURCE, "AudioMediaSource");
            clips.push(self.audio_clip(markers_id, source, None, mono, true));
            src.audio = Some(source);
            let stream = self.obj(
                "AudioStream",
                class::AUDIO_STREAM,
                &format!("<FrameRate>{s}</FrameRate>{}<Duration>{length}</Duration>", el("AudioChannelLayout", if a.channels == 1 { MONO } else { STEREO })),
            );
            streams.push_str(&format!("<AudioStream ObjectRef=\"{stream}\"/>"));
        }
        if let Some(v) = &info.video {
            let rate = video_rate.unwrap_or(TICKS_PER_SECOND / 30);
            let still_tag = if still { "<IsStill>true</IsStill>" } else { "" };
            let stream = self.obj(
                "VideoStream",
                class::VIDEO_STREAM,
                &format!("<FrameRate>{rate}</FrameRate><Duration>{length}</Duration><FrameRect>0,0,{},{}</FrameRect>{still_tag}", v.width, v.height),
            );
            streams.push_str(&format!("<VideoStream ObjectRef=\"{stream}\"/>"));
        }
        let groups = self.channel_groups(src.audio.map(|_| if mono { 1 } else { 2 }));
        let win = premiere_path(path, &self.opts.path_map);
        let rel = self
            .opts
            .dest_dir
            .as_deref()
            .and_then(|d| crate::common::relative_path(path, d))
            .map(|r| {
                let r = r.replace('/', "\\");
                if r.starts_with("..") { r } else { format!(".\\{r}") }
            })
            .map(|r| el("RelativePath", &r))
            .unwrap_or_default();
        let key = self.uuid();
        let infinite = if still { "<Infinite>true</Infinite>" } else { "" };
        let conformed = sample.map(|s| format!("<ConformedAudioRate>{s}</ConformedAudioRate>")).unwrap_or_default();
        self.uobj(
            &media_uid,
            "Media",
            class::MEDIA,
            &format!(
                "{streams}{rel}{}<ImplementationID>{IMPORTER}</ImplementationID>{}<FileKey>{key}</FileKey>{infinite}{conformed}{}",
                el("FilePath", &win),
                el("Title", &file),
                el("ActualMediaFilePath", &win)
            ),
        );
        let chains = chain.map(|c| list("AudioComponentChains", "AudioComponentChain", &[c], false)).unwrap_or_default();
        self.uobj(
            &master_uid,
            "MasterClip",
            class::MASTER_CLIP,
            &format!(
                "<LoggingInfo ObjectRef=\"{logging}\"/>{chains}{}<AudioClipChannelGroups ObjectRef=\"{groups}\"/>{}<MasterClipChangeVersion>1</MasterClipChangeVersion>",
                list("Clips", "Clip", &clips, false),
                el("Name", name)
            ),
        );
        self.project_item(item, name, &master_uid);
        self.sources.insert(item, src);
        self.media_uid.push((item, media_uid));
    }

    // ----------------------------------------------------------------------------- sequences

    /// A sequence's project item, master clip and the sequence itself with empty tracks.
    fn sequence(&mut self, item: ItemId, uid: &str) {
        let project = self.project;
        let Some(it) = project.item(item) else { return };
        let Some(seq) = it.as_sequence() else { return };
        let name = it.name.clone();
        let st = &seq.settings;
        let frame = st.frame_rate.frame_duration().0.max(1);
        let sample = sample_ticks(st.sample_rate);
        let end = seq.all_tracks().flat_map(|t| t.items.iter().map(|c| c.end().0)).max().unwrap_or(0).max(0);

        // master clip: an audio and a video clip of the sequence
        let master_uid = self.uuid();
        let markers = self.markers(&seq.markers);
        let logging = self.obj("ClipLoggingInfo", class::LOGGING, "");
        let chain = self.default_audio_chain(sample, false);
        let src_body = format!(
            "<SequenceSource Version=\"4\"><Content Version=\"10\"></Content><Sequence ObjectURef=\"{uid}\"/></SequenceSource><OriginalDuration>{end}</OriginalDuration>"
        );
        let audio_source = self.obj("AudioSequenceSource", class::AUDIO_SEQUENCE_SOURCE, &src_body);
        let video_source = self.obj("VideoSequenceSource", class::VIDEO_SEQUENCE_SOURCE, &src_body);
        let audio_clip = self.audio_clip(markers, audio_source, None, false, true);
        let video_clip = self.video_clip(markers, video_source, None, true);
        let groups = self.channel_groups(Some(2));
        self.uobj(
            &master_uid,
            "MasterClip",
            class::MASTER_CLIP,
            &format!(
                "<LoggingInfo ObjectRef=\"{logging}\"/>{}{}<AudioClipChannelGroups ObjectRef=\"{groups}\"/>{}<MasterClipChangeVersion>1</MasterClipChangeVersion>",
                list("AudioComponentChains", "AudioComponentChain", &[chain], false),
                list("Clips", "Clip", &[audio_clip, video_clip], false),
                el("Name", &name)
            ),
        );
        self.project_item(item, &name, &master_uid);
        self.sources.insert(
            item,
            Sources {
                master: master_uid,
                name: name.clone(),
                video: Some(video_source),
                audio: Some(audio_source),
                markers,
                size: (st.width, st.height),
                mono: false,
                sample,
            },
        );

        // tracks
        let video_n = seq.video_tracks.len().max(1);
        let mut video_tracks = Vec::new();
        for i in 0..video_n {
            let t = self.uuid();
            let props = "<TL.SQTrackExpanded>0</TL.SQTrackExpanded><TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight><MZ.TrackTargeted>1</MZ.TrackTargeted><TL.SQTrackShy>0</TL.SQTrackShy>";
            self.uobj(&t, "VideoClipTrack", class::VIDEO_TRACK, &Self::clip_track(props, i + 1, VIDEO_MEDIA_TYPE, i));
            video_tracks.push(t);
        }
        let audio_n = seq.audio_tracks.len().max(1);
        let mut audio_tracks = Vec::new();
        for i in 0..audio_n {
            let volume = seq.audio_tracks.get(i).map(|t| t.volume_db).unwrap_or(0.0);
            let fader = self.fader(sample, volume);
            let meter = self.meter(sample);
            let chain = self.obj(
                "AudioComponentChain",
                class::AUDIO_CHAIN,
                &format!(
                    "<ComponentChain Version=\"3\">{}</ComponentChain><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType>",
                    list("Components", "Component", &[fader, meter], false),
                    el("AudioChannelLayout", STEREO)
                ),
            );
            let balance = self.obj(
                "AudioComponentParam",
                class::AUDIO_PARAM,
                &format!(
                    "<StartKeyframe>{NO_TIME},0.5,0,0,0,0,0,0</StartKeyframe><CurrentValue>0.5</CurrentValue><Name>Balance</Name><IsInverted>true</IsInverted>"
                ),
            );
            let panner = self.obj(
                "StereoToStereoPanProcessor",
                class::STEREO_PANNER,
                &format!(
                    "<PanProcessor Version=\"3\"><AudioComponent Version=\"3\"><Component Version=\"7\">{}<ID>{PANNER_ID}</ID></Component><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType><AudioComponentType>0</AudioComponentType></AudioComponent></PanProcessor>",
                    list("Params", "Param", &[balance], false),
                    el("AudioChannelLayout", STEREO)
                ),
            );
            let t = self.uuid();
            let track_id = self.uuid();
            let props = "<TL.SQTrackExpanded>0</TL.SQTrackExpanded><TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight><MZ.TrackTargeted>1</MZ.TrackTargeted><TL.SQTrackAudioKeyframeStyle>0</TL.SQTrackAudioKeyframeStyle><TL.SQTrackShy>0</TL.SQTrackShy><CM.KeyframeMode>true</CM.KeyframeMode>";
            let body = format!(
                "{}<AudioTrack Version=\"12\"><ComponentOwner Version=\"1\"><Components ObjectRef=\"{chain}\"/></ComponentOwner><Panner ObjectRef=\"{panner}\"/><FrameRate>{sample}</FrameRate><ID>{track_id}</ID><NextPannerID>4294967279</NextPannerID></AudioTrack>",
                // the master track is track 1 of the audio group
                Self::clip_track(props, i + 2, AUDIO_MEDIA_TYPE, i)
            );
            self.uobj(&t, "AudioClipTrack", class::AUDIO_TRACK, &body);
            audio_tracks.push(t);
        }
        // the master track
        let fader = self.fader(sample, seq.master_volume_db);
        let meter = self.meter(sample);
        let master_chain = self.obj(
            "AudioComponentChain",
            class::AUDIO_CHAIN,
            &format!(
                "<ComponentChain Version=\"3\">{}</ComponentChain><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType>",
                list("Components", "Component", &[fader, meter], false),
                el("AudioChannelLayout", STEREO)
            ),
        );
        let master_panner = self.obj(
            "DefaultPanProcessor",
            class::DEFAULT_PANNER,
            &format!(
                "<PanProcessor Version=\"3\"><AudioComponent Version=\"3\"><Component Version=\"7\"><ID>{PANNER_ID}</ID></Component><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType><AudioComponentType>0</AudioComponentType></AudioComponent></PanProcessor><DefaultPannerInputChannelType>1</DefaultPannerInputChannelType><DefaultPannerOutputChannelType>1</DefaultPannerOutputChannelType>",
                el("AudioChannelLayout", STEREO)
            ),
        );
        let inlet = self.obj(
            "AudioTrackInlet",
            class::TRACK_INLET,
            &format!("{}<FrameRate>{sample}</FrameRate>{}", list("Sources", "Source", &audio_tracks, true), el("AudioChannelLayout", STEREO)),
        );
        let mix_id = self.uuid();
        let mix = self.obj(
            "AudioMixTrack",
            class::MIX_TRACK,
            &format!(
                "<AudioTrack Version=\"12\"><ComponentOwner Version=\"1\"><Components ObjectRef=\"{master_chain}\"/></ComponentOwner><Panner ObjectRef=\"{master_panner}\"/><FrameRate>{sample}</FrameRate><ID>{mix_id}</ID><SubType>3</SubType><Assign>0</Assign><NextPannerID>4294967279</NextPannerID></AudioTrack><Track Version=\"4\"><Node Version=\"1\"><Properties Version=\"1\"><TL.SQTrackExpanded>0</TL.SQTrackExpanded><TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight><TL.SQTrackAudioKeyframeStyle>2</TL.SQTrackAudioKeyframeStyle><TL.SQTrackShy>0</TL.SQTrackShy></Properties></Node><ID>1</ID><MediaType>{AUDIO_MEDIA_TYPE}</MediaType><Index>0</Index></Track><Inlet ObjectRef=\"{inlet}\"/>"
            ),
        );
        // track groups
        let group_chain = self.obj("VideoComponentChain", class::VIDEO_CHAIN, "<ComponentChain Version=\"3\"></ComponentChain>");
        let video_group = self.obj(
            "VideoTrackGroup",
            class::VIDEO_TRACK_GROUP,
            &format!(
                "<TrackGroup Version=\"1\">{}<FrameRate>{frame}</FrameRate><NextTrackID>{}</NextTrackID></TrackGroup>{}<FrameRect>0,0,{},{}</FrameRect>{}<ToneMappingDesaturation>0.5</ToneMappingDesaturation><AutoInputGamutCompressionEnabled>true</AutoInputGamutCompressionEnabled><IsGraphicsWhiteSameAsProject>false</IsGraphicsWhiteSameAsProject><IsColorAwareEffectsEnabledSameAsProject>false</IsColorAwareEffectsEnabledSameAsProject><ComponentOwner Version=\"1\"><Components ObjectRef=\"{group_chain}\"/></ComponentOwner>",
                list("Tracks", "Track", &video_tracks, true),
                video_n + 1,
                el("ColorManagementSettings", r#"{"autoToneMapEnabled":true,"enableLogColorManagement":2,"graphicsWhiteLuminance":203,"lutInterpolationMethod":1}"#),
                st.width,
                st.height,
                el(
                    "OutputColorSpace",
                    r#"{"baseColorProfile":{"colorProfileData":"AQAAAGQAAAA=","colorProfileName":"BT.709,8-bit,Display-Referred"},"baseProfileType":1,"colorSpaceMetadata":{"peakLuminance":100}}"#
                )
            ),
        );
        let audio_group_id = self.uuid();
        let audio_group = self.obj(
            "AudioTrackGroup",
            class::AUDIO_TRACK_GROUP,
            &format!(
                "<TrackGroup Version=\"1\">{}<FrameRate>{sample}</FrameRate><NextTrackID>{}</NextTrackID></TrackGroup><MasterTrack ObjectRef=\"{mix}\"/><ID>{audio_group_id}</ID><AutomationSafeFlags>0</AutomationSafeFlags><NumAdaptiveChannels>2</NumAdaptiveChannels>",
                list("Tracks", "Track", &audio_tracks, true),
                audio_n + 2
            ),
        );
        let data_group = self.obj(
            "DataTrackGroup",
            class::DATA_TRACK_GROUP,
            &format!("<TrackGroup Version=\"1\"><FrameRate>{frame}</FrameRate><NextTrackID>1</NextTrackID></TrackGroup>"),
        );
        let groups: String = [(VIDEO_MEDIA_TYPE, video_group), (AUDIO_MEDIA_TYPE, audio_group), (DATA_MEDIA_TYPE, data_group)]
            .iter()
            .enumerate()
            .map(|(i, (t, g))| format!("<TrackGroup Version=\"1\" Index=\"{i}\"><First>{t}</First><Second ObjectRef=\"{g}\"/></TrackGroup>"))
            .collect();
        let work_out = if end > 0 { end } else { TICKS_PER_SECOND };
        let props = format!(
            "<MZ.EditLine>0</MZ.EditLine><MZ.Sequence.AudioTimeDisplayFormat>200</MZ.Sequence.AudioTimeDisplayFormat><MZ.WorkInPoint>0</MZ.WorkInPoint><MZ.WorkOutPoint>{work_out}</MZ.WorkOutPoint><MZ.Sequence.EditingModeGUID>{CUSTOM_MODE}</MZ.Sequence.EditingModeGUID><MZ.Sequence.PreviewUseMaxBitDepth>false</MZ.Sequence.PreviewUseMaxBitDepth><MZ.Sequence.PreviewUseMaxRenderQuality>false</MZ.Sequence.PreviewUseMaxRenderQuality>{}<MZ.Sequence.PreviewRenderingPresetCodec>1634755443</MZ.Sequence.PreviewRenderingPresetCodec><MZ.Sequence.PreviewRenderingClassID>1061109567</MZ.Sequence.PreviewRenderingClassID><MZ.Sequence.PreviewFrameSizeWidth>{}</MZ.Sequence.PreviewFrameSizeWidth><MZ.Sequence.PreviewFrameSizeHeight>{}</MZ.Sequence.PreviewFrameSizeHeight>",
            el("MZ.Sequence.PreviewRenderingPresetPath", &format!("EncoderPresets\\SequencePreview\\{CUSTOM_MODE}\\QuickTime.epr")),
            st.width,
            st.height
        );
        self.uobj(
            uid,
            "Sequence",
            class::SEQUENCE,
            &format!(
                "<Node Version=\"1\"><Properties Version=\"1\">{props}</Properties></Node><PersistentGroupContainer Version=\"1\"><LinkContainer Version=\"1\"></LinkContainer></PersistentGroupContainer><TrackGroups Version=\"1\">{groups}</TrackGroups>{}",
                el("Name", &name)
            ),
        );
        self.sequence_uid.push((item, uid.to_string()));
    }

    /// A `ClipTrack` with no clips.
    fn clip_track(props: &str, id: usize, media_type: &str, index: usize) -> String {
        format!(
            "<ClipTrack Version=\"2\"><Track Version=\"4\"><Node Version=\"1\"><Properties Version=\"1\">{props}</Properties></Node><ID>{id}</ID><MediaType>{media_type}</MediaType><Index>{index}</Index></Track><ClipItems Version=\"3\"><MediaType>{media_type}</MediaType><Index>{index}</Index></ClipItems><TransitionItems Version=\"3\"><MediaType>{media_type}</MediaType><Index>{index}</Index></TransitionItems></ClipTrack>"
        )
    }

    /// A track fader at `db` (linear; 1.0 is 0 dB).
    fn fader(&mut self, sample: i64, db: f64) -> u64 {
        let value = if db.is_finite() && db.abs() > 1e-9 {
            let g = 10f64.powf(db.clamp(-200.0, 15.0) / 20.0);
            format!("<StartKeyframe>{NO_TIME},{},0,0,0,0,0,0</StartKeyframe><CurrentValue>{}</CurrentValue>", num(g), num(g))
        } else {
            String::new()
        };
        let volume = self.obj(
            "AudioComponentParam",
            class::AUDIO_PARAM,
            &format!("{value}<Name>Volume</Name><RangeLocked>false</RangeLocked><UnitsString>dB</UnitsString><UpperBound>{FADER_MAX}</UpperBound>"),
        );
        let mute = self.obj("AudioComponentParam", class::AUDIO_BOOL_PARAM, "<Name>Mute</Name><RangeLocked>false</RangeLocked>");
        self.obj(
            "AudioFader",
            class::FADER,
            &format!(
                "<AudioComponent Version=\"3\"><Component Version=\"7\">{}<ID>1</ID></Component><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType><AudioComponentType>1</AudioComponentType></AudioComponent>",
                list("Params", "Param", &[volume, mute], false),
                el("AudioChannelLayout", STEREO)
            ),
        )
    }

    fn meter(&mut self, sample: i64) -> u64 {
        self.obj(
            "AudioMeter",
            class::METER,
            &format!(
                "<AudioComponent Version=\"3\"><Component Version=\"7\"><ID>2</ID></Component><FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType><AudioComponentType>2</AudioComponentType></AudioComponent>",
                el("AudioChannelLayout", STEREO)
            ),
        )
    }

    // ----------------------------------------------------------------------------- templates

    /// A template clip per (source, kind of track) the sequences use: Premiere's layout of a clip
    /// with explicit intrinsic components, which the writer copies for every clip.
    fn templates(&mut self) {
        let project = self.project;
        let mut wanted: Vec<(ItemId, TrackKind)> = Vec::new();
        let mut seen = HashSet::new();
        for seq in project.sequences().filter_map(|i| i.as_sequence()) {
            for t in seq.all_tracks() {
                for c in &t.items {
                    if seen.insert((c.item, t.kind == TrackKind::Video)) {
                        wanted.push((c.item, t.kind));
                    }
                }
            }
        }
        for (item, kind) in wanted {
            let Some(src) = self.sources.get(&item).cloned() else { continue };
            match kind {
                TrackKind::Video => {
                    if let Some(source) = src.video {
                        self.video_template(&src, source);
                    }
                }
                TrackKind::Audio => {
                    if let Some(source) = src.audio {
                        self.audio_template(&src, source);
                    }
                }
            }
        }
    }

    fn sub_clip(&mut self, clip: u64, src: &Sources) -> u64 {
        self.obj(
            "SubClip",
            class::SUB_CLIP,
            &format!("<Clip ObjectRef=\"{clip}\"/><MasterClip ObjectURef=\"{}\"/>{}<OrigChGrp>0</OrigChGrp>", src.master, el("Name", &src.name)),
        )
    }

    fn video_template(&mut self, src: &Sources, source: u64) {
        let float = |s: &mut Self, name: &str, id: u32, v: &str, bounds: &str, extra: &str| {
            s.obj(
                "VideoComponentParam",
                class::FLOAT_PARAM,
                &format!("{}{extra}<ParameterID>{id}</ParameterID><StartKeyframe>{NO_TIME},{v},0,0,0,0,0,0</StartKeyframe>{bounds}", el("Name", name)),
            )
        };
        let point = |s: &mut Self, name: &str, id: u32| {
            s.obj(
                "PointComponentParam",
                class::POINT_PARAM,
                &format!("{}<ParameterID>{id}</ParameterID><StartKeyframe>{NO_TIME},0.5:0.5,0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe>", el("Name", name)),
            )
        };
        let scale_bounds = "<LowerBound>0</LowerBound><UpperBound>10000</UpperBound>";
        let pct = "<LowerBound>0</LowerBound><UpperBound>100</UpperBound>";
        let motion_params = vec![
            point(self, "Position", 1),
            float(self, "Scale", 2, "100.", scale_bounds, "<UpperUIBound>200</UpperUIBound>"),
            float(self, "Scale Width", 3, "100.", scale_bounds, "<UpperUIBound>200</UpperUIBound>"),
            // Uniform Scale: Premiere saves it without a name
            self.obj("VideoComponentParam", class::BOOL_PARAM, &format!("<Name> </Name><ParameterID>4</ParameterID><StartKeyframe>{NO_TIME},true,0,0,0,0,0,0</StartKeyframe>")),
            float(self, "Rotation", 5, "0.", "<LowerBound>-32768</LowerBound><UpperBound>32767</UpperBound>", "<ParameterControlType>3</ParameterControlType>"),
            point(self, "Anchor Point", 6),
            self.obj(
                "VideoComponentParam",
                class::SLIDER_PARAM,
                &format!("<Name>Anti-flicker Filter</Name><ParameterID>7</ParameterID><StartKeyframe>{NO_TIME},0.,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>1</UpperBound>"),
            ),
            float(self, "Crop Left", 8, "0.", pct, ""),
            float(self, "Crop Top", 9, "0.", pct, ""),
            float(self, "Crop Right", 10, "0.", pct, ""),
            float(self, "Crop Bottom", 11, "0.", pct, ""),
        ];
        let motion = self.obj(
            "VideoFilterComponent",
            class::VIDEO_FILTER,
            &format!(
                "<Component Version=\"7\">{}<ID>1</ID><DisplayName>Motion</DisplayName><Intrinsic>true</Intrinsic></Component><PremiereFilterPrivateData Encoding=\"base64\" BinaryHash=\"3805854c-243a-c4be-bd40-a8a40000000e\">AQA=</PremiereFilterPrivateData><MatchName>AE.ADBE Motion</MatchName><VideoFilterType>2</VideoFilterType>",
                list("Params", "Param", &motion_params, false)
            ),
        );
        let opacity_params = vec![
            float(self, "Opacity", 1, "100.", pct, ""),
            self.obj(
                "VideoComponentParam",
                class::POPUP_PARAM,
                &format!("<Name>Blend Mode</Name><ParameterControlType>10</ParameterControlType><ParameterID>2</ParameterID><DiscontinuousInterpolate>true</DiscontinuousInterpolate><StartKeyframe>{NO_TIME},18,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>27</UpperBound>"),
            ),
            self.obj(
                "VideoComponentParam",
                class::POPUP_PARAM,
                &format!("<Name>Blend Mode</Name><ParameterID>3</ParameterID><DiscontinuousInterpolate>true</DiscontinuousInterpolate><StartKeyframe>{NO_TIME},0,0,0,0,0,0,0</StartKeyframe><LowerBound>0</LowerBound><UpperBound>31</UpperBound>"),
            ),
        ];
        let opacity = self.obj(
            "VideoFilterComponent",
            class::VIDEO_FILTER,
            &format!(
                "<Component Version=\"7\">{}<ID>2</ID><DisplayName>Opacity</DisplayName><Intrinsic>true</Intrinsic></Component><MatchName>AE.ADBE Opacity</MatchName><VideoFilterType>2</VideoFilterType>",
                list("Params", "Param", &opacity_params, false)
            ),
        );
        let chain = self.obj(
            "VideoComponentChain",
            class::VIDEO_CHAIN,
            &format!("<ComponentChain Version=\"3\">{}</ComponentChain>", list("Components", "Component", &[motion, opacity], false)),
        );
        let clip = self.video_clip(src.markers, source, Some((0, TEMPLATE_LENGTH)), false);
        let sub = self.sub_clip(clip, src);
        let (w, h) = src.size;
        self.obj(
            "VideoClipTrackItem",
            class::VIDEO_TRACK_ITEM,
            &format!(
                "<ClipTrackItem Version=\"8\"><ComponentOwner Version=\"1\"><Components ObjectRef=\"{chain}\"/></ComponentOwner><TrackItem Version=\"4\"><Start>0</Start><End>{TEMPLATE_LENGTH}</End></TrackItem><SubClip ObjectRef=\"{sub}\"/></ClipTrackItem><PixelAspectRatio>1,1</PixelAspectRatio><FrameRect>0,0,{w},{h}</FrameRect>"
            ),
        );
    }

    fn audio_template(&mut self, src: &Sources, source: u64) {
        let sample = if src.sample > 0 { src.sample } else { sample_ticks(48_000) };
        let level = |s: &mut Self, name: Option<&str>| {
            let name = name.map(|n| el("Name", n)).unwrap_or_default();
            s.obj(
                "AudioComponentParam",
                class::AUDIO_PARAM,
                &format!("<StartKeyframe>{NO_TIME},{UNITY_LEVEL},0,0,0,0,0,0</StartKeyframe><CurrentValue>{UNITY_LEVEL}</CurrentValue><IsTimeVarying>false</IsTimeVarying><UnitsString>dB</UnitsString>{name}"),
            )
        };
        let toggle = |s: &mut Self, name: &str| {
            s.obj(
                "AudioComponentParam",
                class::AUDIO_BOOL_PARAM,
                &format!("<IsTimeVarying>false</IsTimeVarying><RangeLocked>false</RangeLocked>{}", el("Name", name)),
            )
        };
        let (layout, kind, config, suffix) = if src.mono {
            (MONO, 0, r#"{"in":[{"layout":[0],"name":"Mono In","type":0}],"out":[{"layout":[0],"name":"Mono Out","type":0}]}"#, "Mono")
        } else {
            (STEREO, 1, r#"{"in":[{"layout":[100,101],"name":"Stereo In","type":0}],"out":[{"layout":[100,101],"name":"Stereo Out","type":0}]}"#, "Stereo")
        };
        let filter = |s: &mut Self, params: &[u64], id: u32, name: &str| {
            s.obj(
                "AudioFilterComponent",
                class::AUDIO_FILTER,
                &format!(
                    "<AudioComponent Version=\"3\"><Component Version=\"7\">{}<Intrinsic>true</Intrinsic><ID>{id}</ID></Component>{}<ChannelType>{kind}</ChannelType><FrameRate>{sample}</FrameRate><AudioComponentType>0</AudioComponentType></AudioComponent><FilterPreset>0</FilterPreset>{}<FilterIndex>-1</FilterIndex>{}",
                    list("Params", "Param", params, false),
                    el("AudioChannelLayout", layout),
                    el("FilterMatchName", &format!("{name} {suffix}")),
                    el("ChannelConfigData", config)
                ),
            )
        };
        let mute = toggle(self, "Mute");
        let lvl = level(self, Some("Level"));
        let volume = filter(self, &[mute, lvl], 1, "Internal Volume");
        let mut components = vec![volume];
        if !src.mono {
            let mut params = vec![toggle(self, "Bypass"), level(self, Some("Left")), level(self, Some("Right"))];
            for _ in 2..CHANNEL_VOLUME_SLOTS {
                params.push(level(self, None));
            }
            components.push(filter(self, &params, 2, "Internal Channel Volume"));
        }
        let chain_tail =
            if src.mono { String::new() } else { format!("<FrameRate>{sample}</FrameRate>{}<ChannelType>1</ChannelType>", el("AudioChannelLayout", STEREO)) };
        let chain = self.obj(
            "AudioComponentChain",
            class::AUDIO_CHAIN,
            &format!("<ComponentChain Version=\"3\">{}</ComponentChain>{chain_tail}", list("Components", "Component", &components, false)),
        );
        let clip = self.audio_clip(src.markers, source, Some((0, TEMPLATE_LENGTH)), src.mono, false);
        let sub = self.sub_clip(clip, src);
        let id = self.uuid();
        self.obj(
            "AudioClipTrackItem",
            class::AUDIO_TRACK_ITEM,
            &format!(
                "<ClipTrackItem Version=\"8\"><ComponentOwner Version=\"1\"><Components ObjectRef=\"{chain}\"/></ComponentOwner><TrackItem Version=\"4\"><Start>0</Start><End>{TEMPLATE_LENGTH}</End></TrackItem><SubClip ObjectRef=\"{sub}\"/></ClipTrackItem><ID>{id}</ID><PreRenderComponentChainHashVersion>1</PreRenderComponentChainHashVersion>"
            ),
        );
    }

    // ----------------------------------------------------------------------------- borrowed

    /// Add a borrowed layout with ids of this document (it is copied by the writer for the clips
    /// that use it, and dropped with the templates).
    fn graft(&mut self, g: &Graft) {
        let ids: HashMap<&str, u64> = g.objects.iter().map(|o| (o.key.as_str(), self.id())).collect();
        let mut objs = Vec::with_capacity(g.objects.len());
        for o in &g.objects {
            let Some(new) = ids.get(o.key.as_str()) else { return };
            let mut edits: Vec<(Range<usize>, String)> = vec![(o.id_at.clone(), new.to_string())];
            for (r, old) in &o.refs {
                match ids.get(old.as_str()) {
                    Some(n) => edits.push((r.clone(), n.to_string())),
                    None => return,
                }
            }
            edits.extend(o.payloads.iter().cloned());
            edits.sort_by_key(|(r, _)| r.start);
            let mut out = String::with_capacity(o.text.len() + 64);
            let mut pos = 0;
            for (r, t) in edits {
                if r.start < pos {
                    return;
                }
                out.push_str(o.text.get(pos..r.start).unwrap_or(""));
                out.push_str(&t);
                pos = r.end;
            }
            out.push_str(o.text.get(pos..).unwrap_or(""));
            objs.push(format!("\t{out}\n"));
        }
        // all or nothing: a part of a layout would leave references unresolved
        self.objs.extend(objs);
    }

    // ----------------------------------------------------------------------------- bins

    fn bin_children(&mut self, bin: &Bin, listed: &mut HashSet<ItemId>) -> Vec<String> {
        self.bin_children_at(bin, listed, 0)
    }

    fn bin_children_at(&mut self, bin: &Bin, listed: &mut HashSet<ItemId>, depth: usize) -> Vec<String> {
        let mut out = Vec::new();
        for c in &bin.children {
            match c {
                BinEntry::Item(i) => {
                    listed.insert(*i);
                    if let Some(uid) = self.project_items.get(i) {
                        out.push(uid.clone());
                    }
                }
                // bounded: bins nested deeper than this are not followed (their items go to the
                // root bin)
                BinEntry::Bin(b) if depth < 64 => {
                    let kids = self.bin_children_at(b, listed, depth + 1);
                    let items = if kids.is_empty() { String::new() } else { list("Items", "Item", &kids, true) };
                    let uid = self.uuid();
                    self.uobj(
                        &uid,
                        "BinProjectItem",
                        class::BIN_ITEM,
                        &format!(
                            "<ProjectItem Version=\"1\"><Node Version=\"1\"><Properties Version=\"1\"></Properties></Node>{}</ProjectItem><ProjectItemContainer Version=\"1\">{items}</ProjectItemContainer>",
                            el("Name", &b.name)
                        ),
                    );
                    out.push(uid);
                }
                BinEntry::Bin(_) => {}
            }
        }
        out
    }
}

struct ProjectParts {
    settings: u64,
    compile: [u64; 5],
    scratch: u64,
    ingest: u64,
    workspace: u64,
}

/// FilmCraft's path in the terms of the machine running Premiere (`/mnt/LocalDisk/x.wav` →
/// `L:\x.wav`, the longest matching prefix); unchanged when no prefix maps it.
pub(crate) fn premiere_path(path: &str, map: &[(String, String)]) -> String {
    let mut best: Option<(&str, &str)> = None;
    for (from, to) in map {
        let from = from.trim_end_matches('/');
        let under = path.strip_prefix(from).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if !from.is_empty() && under && best.is_none_or(|b| from.len() > b.0.len()) {
            best = Some((from, to.as_str()));
        }
    }
    let Some((from, to)) = best else { return path.to_string() };
    let rest = path.get(from.len()..).unwrap_or("").trim_start_matches('/').replace('/', "\\");
    format!("{}\\{rest}", to.trim_end_matches('\\'))
}

/// The document without the objects nothing reachable from the project refers to (the templates).
fn prune(text: &str) -> Result<String> {
    let doc = crate::xml::parse(text).map_err(|e| Error::Other(format!("Premiere Pro project: the written project does not parse: {e}")))?;
    let root = doc.root_element();
    let objects: Vec<Node> = root.children().filter(Node::is_element).collect();
    let mut by_key: HashMap<&str, usize> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        if let Some(k) = o.attribute("ObjectID").or_else(|| o.attribute("ObjectUID")) {
            by_key.insert(k, i);
        }
    }
    // from the root's references (`<Project ObjectRef="1"/>`)
    let mut keep = vec![false; objects.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        if o.attribute("ObjectID").is_none() && o.attribute("ObjectUID").is_none() {
            keep[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let Some(o) = objects.get(i) else { continue };
        let refs = crate::prproj_write::public_refs(*o).into_iter().chain(o.attribute("ObjectRef").map(|_| *o));
        for r in refs {
            let Some(k) = r.attribute("ObjectRef").or_else(|| r.attribute("ObjectURef")) else { continue };
            if let Some(j) = by_key.get(k).copied()
                && let Some(slot) = keep.get_mut(j)
                && !*slot
            {
                *slot = true;
                stack.push(j);
            }
        }
    }
    let close = root.range().end.saturating_sub("</PremiereData>".len());
    let first = objects.first().map(|o| o.range().start).unwrap_or(close);
    let mut out = String::with_capacity(text.len());
    out.push_str(text.get(..first).unwrap_or(""));
    for (i, o) in objects.iter().enumerate() {
        if !keep.get(i).copied().unwrap_or(true) {
            continue;
        }
        let end = objects.get(i + 1).map(|n| n.range().start).unwrap_or(close);
        out.push_str(text.get(o.range().start..end).unwrap_or(""));
    }
    out.push_str(text.get(close..).unwrap_or(""));
    Ok(out)
}

#[cfg(test)]
#[path = "prproj_new_tests.rs"]
mod tests;
