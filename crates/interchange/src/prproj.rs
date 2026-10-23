//! Premiere Pro project (`.prproj`) import.
//!
//! There is no published specification for this format. Everything here is derived from
//! observing project files saved by Premiere Pro (gzip-compressed XML, root `<PremiereData
//! Version="3">`); no application internals were inspected. The observations are recorded in this
//! crate's README (`prproj` section). Writing back into the same project is [`crate::prproj_write`].
//!
//! The document is a flat list of objects under the root, each with an `ObjectID` (integer) or
//! `ObjectUID` (UUID). Objects point at each other with `ObjectRef` / `ObjectURef` attributes; only
//! the root's direct children are addressable (nested elements may reuse ids locally). Times are
//! Premiere ticks, 254 016 000 000 per second, which is exactly [`Tick`]'s resolution.
//!
//! Mapped: the bin tree, media (path, picture size, frame rate, sample rate, channels), sequences
//! (frame size, rate, sample rate, track names), clips (position, source in point, speed,
//! reverse, frame holds), nested sequences, video/audio links, transitions, Motion, Opacity and
//! clip volume with keyframes and easing, effects FilmCraft has under the same name (their numeric,
//! boolean and point parameters), and markers on media and sequences. Everything else is listed
//! in the [`Report`].
//!
//! For round trips, every clip remembers the Premiere track item it came from
//! ([`filmcraft_project::TrackItem::origin`]), media items and sequences their Premiere UIDs in
//! their metadata ([`META_MEDIA`], [`META_SEQUENCE`]), and sequences the source document
//! ([`META_SOURCE`], [`META_FINGERPRINT`]).

use std::collections::{BTreeMap, HashMap, HashSet};

use filmcraft_media::MediaKind;
use filmcraft_media::generators::Generator;
use filmcraft_project::{
    BinId, ClipId, ItemId, ItemKind, Label, Marker, MarkerId, MarkerKind, Param, ParamValue, Sequence, TrackKind, Transition, TransitionAlign, TransitionId,
};
use filmcraft_time::{FrameRate, Tick};
use roxmltree::Node;

use crate::common::{Builder, MediaSpec, empty_sequence, file_name, is_absolute, kind_from_ext, resolve_path, set_param, settings_for, transition_effect};
use crate::prproj_fx::{self, Ctx, MAX_TIME, Target};
use crate::prproj_graph::{self, Index, TICKS_PER_SECOND};
use crate::prproj_mask;
use crate::xml::{child, child_text, path_text};
use crate::{Error, Format, ImportOptions, Imported, Report, Result};

/// Item metadata: the Premiere `Media` UID(s) behind a media item (comma-separated when Premiere
/// holds several media objects for one file).
pub const META_MEDIA: &str = "premiere.media";
/// Item metadata: the Premiere `Sequence` UID behind a sequence item.
pub const META_SEQUENCE: &str = "premiere.sequence";
/// The label an item came in with (Premiere's, or the importer's default for one without):
/// the writer saves a label only where FilmCraft's differs from it.
pub const META_LABEL: &str = "premiere.label";
/// Sequence item metadata: the `.prproj` the sequence was imported from.
pub const META_SOURCE: &str = "premiere.source";
/// Sequence item metadata: the fingerprint of that document when it was imported.
pub const META_FINGERPRINT: &str = "premiere.fingerprint";

/// The importer Premiere records for graphics (titles) made in the timeline: their "media" has no
/// file (`FilePath` is a type code) and a still video stream the size of the canvas.
const GRAPHIC_IMPORTER: &str = "42008e7a-de6f-4270-96de-7e287abb9b4b";
/// Premiere's text layer component.
pub(crate) const TEXT_COMPONENT: &str = "AE.ADBE Text";

/// Nested bins deeper than this are not followed.
const MAX_DEPTH: usize = 64;
/// Speeds are clamped to 0.1 %–10 000 % so no source range can overflow [`Tick`] arithmetic.
pub(crate) const SPEED_RANGE: (f64, f64) = (0.001, 100.0);
/// Premiere's `FrameHold` value for "hold on the frame at `FrameHoldStart`".
pub(crate) const FRAME_HOLD_AT: &str = "4";

/// A time read from the document, clamped to `0..=MAX_TIME`.
pub(crate) fn time(s: Option<&str>) -> Option<i64> {
    s.and_then(parse_i64).map(|t| t.clamp(0, MAX_TIME))
}

/// Whether `bytes` look like a Premiere project: gzip (checked by the caller's extension) or
/// uncompressed XML whose root is `<PremiereData>`.
pub(crate) fn sniff(bytes: &[u8], extension: Option<&str>) -> bool {
    let is_prproj = extension.is_some_and(|e| e.trim_start_matches('.').eq_ignore_ascii_case("prproj"));
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return is_prproj;
    }
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(512)).unwrap_or_default());
    head.contains("<PremiereData")
}

/// Import a Premiere project.
pub(crate) fn import(bytes: &[u8], opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    let (raw, _) = prproj_graph::decompress(bytes)?;
    let text = String::from_utf8_lossy(&raw);
    let doc = crate::xml::parse(&text).map_err(|e| Error::parse(Format::Prproj, e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PremiereData" {
        return Err(Error::parse(Format::Prproj, "root element is not <PremiereData>"));
    }
    let name = opts.name.clone().unwrap_or_else(|| "Premiere Project".to_string());
    let mut imp = Imp {
        b: Builder::new(&name),
        root,
        payloads: None,
        ix: Index::new(root),
        opts,
        report,
        fingerprint: prproj_graph::fingerprint(bytes),
        seq_items: HashMap::new(),
        seq_nodes: Vec::new(),
        seq_markers: HashMap::new(),
        links: HashMap::new(),
        nested: HashSet::new(),
        unmapped: BTreeMap::new(),
        mask_skips: BTreeMap::new(),
        audio_off: BTreeMap::new(),
    };
    imp.collect_links(root);
    match root.children().find(|n| n.has_tag_name("RootProjectItem")) {
        Some(rp) => imp.bin_contents(rp, None, 0),
        None => imp.report.warn("the project has no bin tree; sequences were imported at the top level"),
    }
    // Sequences that no bin lists still exist in the document.
    for n in root.children().filter(|n| n.has_tag_name("Sequence")) {
        if let Some(uid) = n.attribute("ObjectUID")
            && !imp.seq_items.contains_key(uid)
        {
            imp.reserve_sequence(n, crate::xml::name_text(n, "Name").unwrap_or("Sequence"), None);
        }
    }
    let seqs = std::mem::take(&mut imp.seq_nodes);
    for (node, id) in &seqs {
        imp.fill_sequence(*node, *id);
    }
    for (effect, params) in std::mem::take(&mut imp.unmapped) {
        let list: Vec<&str> = params.iter().map(String::as_str).collect();
        imp.report.warn(format!("effect \"{effect}\": parameter(s) {} not carried over (kept as they are when exporting back to Premiere)", list.join(", ")));
    }
    if !imp.audio_off.is_empty() {
        let list: Vec<String> = imp.audio_off.iter().map(|(n, k)| if *k > 1 { format!("{n} (×{k})") } else { n.clone() }).collect();
        imp.report.warn(format!(
            "audio effects {} are shown switched off: Premiere saves their settings in a form FilmCraft can't read yet (Premiere keeps them as they are)",
            list.join(", ")
        ));
    }
    for ((effect, why), n) in std::mem::take(&mut imp.mask_skips) {
        let masks = if n == 1 { "mask" } else { "masks" };
        match why {
            prproj_mask::Skipped::Object => imp.report.warn(format!(
                "effect \"{effect}\": {n} Object {masks} (Premiere's automatic subject masks) can't be carried over; the effect applies to the whole clip (kept as they are when exporting back to Premiere)"
            )),
            prproj_mask::Skipped::Empty => imp.report.info(format!("effect \"{effect}\": {n} {masks} without a shape skipped")),
            prproj_mask::Skipped::Unreadable => {
                imp.report.warn(format!("effect \"{effect}\": {n} {masks} could not be read; the effect applies to the whole clip"))
            }
        }
    }
    let mut top: Vec<ItemId> = seqs.iter().map(|(_, id)| *id).filter(|id| !imp.nested.contains(id)).collect();
    if top.is_empty() {
        top = seqs.iter().map(|(_, id)| *id).collect();
    }
    // The sequences Premiere had open (`MZ.PrefixKey.OpenSequenceGuidList.N` view properties)
    // come first, so the importer opens the same timeline.
    let open: Vec<ItemId> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name().starts_with("MZ.PrefixKey.OpenSequenceGuidList"))
        .filter_map(|n| n.text())
        .flat_map(|t| t.split([';', ',', ' ']).map(str::trim).filter(|u| !u.is_empty()).collect::<Vec<_>>())
        .filter_map(|u| imp.seq_items.get(u).copied())
        .collect();
    top.sort_by_key(|id| open.iter().position(|o| o == id).unwrap_or(usize::MAX));
    if top.is_empty() && imp.b.p.items.is_empty() {
        return Err(Error::Empty);
    }
    imp.b.top = top;
    for it in imp.b.p.items.values_mut() {
        it.metadata.insert(META_LABEL.to_string(), it.label.name().to_string());
    }
    Ok(imp.b.finish())
}

struct Imp<'a, 'i, 'r> {
    b: Builder,
    root: Node<'a, 'i>,
    /// Binary payloads by hash (read on first use).
    payloads: Option<HashMap<&'a str, &'a str>>,
    ix: Index<'a, 'i>,
    opts: &'r ImportOptions,
    report: &'r mut Report,
    fingerprint: String,
    /// Sequence `ObjectUID` → its item.
    seq_items: HashMap<&'a str, ItemId>,
    /// Sequences to fill, in document (bin) order.
    seq_nodes: Vec<(Node<'a, 'i>, ItemId)>,
    /// Sequence `ObjectUID` → markers found on its master clip.
    seq_markers: HashMap<&'a str, Vec<Marker>>,
    /// Track item `ObjectID` → link group.
    links: HashMap<&'a str, u64>,
    /// Sequences used as clips inside other sequences.
    nested: HashSet<ItemId>,
    /// Effect → parameters that could not be mapped (reported once per effect).
    unmapped: BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Effect masks that could not be carried over, by effect name and reason.
    mask_skips: BTreeMap<(String, prproj_mask::Skipped), usize>,
    /// Premiere audio effects brought in switched off, by name.
    audio_off: BTreeMap<String, usize>,
}

impl<'a, 'i, 'r> Imp<'a, 'i, 'r> {
    fn collect_links(&mut self, root: Node<'a, 'i>) {
        for l in root.children().filter(|n| n.has_tag_name("Link")) {
            let Some(items) = self.ix.at(l, &["TrackItemGroup", "TrackItems"]) else { continue };
            let members: Vec<&'a str> = items.children().filter(|n| n.has_tag_name("TrackItem")).filter_map(|n| n.attribute("ObjectRef")).collect();
            if members.len() < 2 {
                continue;
            }
            let g = self.b.link_id();
            for m in members {
                self.links.insert(m, g);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Bins and project items
    // -----------------------------------------------------------------------------------------

    fn bin_contents(&mut self, container: Node<'a, 'i>, bin: Option<BinId>, depth: usize) {
        if depth > MAX_DEPTH {
            self.report.warn("bins nested too deeply; the rest were not imported");
            return;
        }
        let Some(items) = self.ix.at(container, &["ProjectItemContainer", "Items"]) else { return };
        for r in items.children().filter(|n| n.has_tag_name("Item")) {
            let Some(item) = self.ix.resolve(Some(r)) else { continue };
            let name = crate::xml::path_name(item, &["ProjectItem", "Name"]).unwrap_or("Untitled");
            match item.tag_name().name() {
                "BinProjectItem" => {
                    let b = self.b.bin(name, bin);
                    self.bin_contents(item, Some(b), depth + 1);
                }
                "ClipProjectItem" => self.project_clip(item, name, bin),
                other => self.report.info(format!("project item type {other} is not supported")),
            }
        }
    }

    /// A Project panel clip: media or a sequence.
    fn project_clip(&mut self, item: Node<'a, 'i>, name: &str, bin: Option<BinId>) {
        let Some(master) = self.ix.follow(item, &["MasterClip"]) else { return };
        let markers = self.master_markers(master);
        let clips = self.ix.list(master, &["Clips"], "Clip");
        // the item's label: the Project panel's, else its master clip's
        let label = project_item_label(item).or_else(|| clips.iter().find_map(|c| clip_label(*c)));
        for clip in clips {
            if let Some(seq) = prproj_graph::clip_sequence(&self.ix, clip) {
                if let Some(uid) = seq.attribute("ObjectUID") {
                    if !self.seq_items.contains_key(uid) {
                        self.reserve_sequence(seq, name, bin);
                    }
                    self.seq_markers.entry(uid).or_insert(markers);
                    if let (Some(l), Some(id)) = (label, self.seq_items.get(uid).copied())
                        && let Some(it) = self.b.p.item_mut(id)
                    {
                        it.label = l;
                    }
                }
                return;
            }
            if let Some(media) = prproj_graph::clip_media(&self.ix, clip) {
                if let Some(id) = self.media(media, Some(name), bin, Some(master))
                    && let Some(it) = self.b.p.item_mut(id)
                {
                    if let Some(l) = label {
                        it.label = l;
                    }
                    if let Some(m) = it.as_media_mut()
                        && m.markers.is_empty()
                    {
                        m.markers = markers;
                    }
                }
                return;
            }
        }
    }

    /// Markers on a master clip (shared by its video and audio clips).
    fn master_markers(&mut self, master: Node<'a, 'i>) -> Vec<Marker> {
        for clip in self.ix.list(master, &["Clips"], "Clip") {
            if let Some(markers) = self.ix.follow(clip, &["Clip", "MarkerOwner", "Markers"]) {
                let out = self.markers(markers);
                if !out.is_empty() {
                    return out;
                }
            }
        }
        Vec::new()
    }

    fn markers(&mut self, owner: Node<'a, 'i>) -> Vec<Marker> {
        let mut out = Vec::new();
        let Some(list) = self.ix.at(owner, &["Markers"]) else { return out };
        for m in list.children().filter(|n| n.has_tag_name("Marker")) {
            let Some(obj) = self.ix.follow(m, &["Second"]) else { continue };
            let Some(json) = child_text(obj, "DVAMarker") else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
                self.report.warn("a marker could not be read and was skipped");
                continue;
            };
            let d = &v["DVAMarker"];
            let ticks = |k: &str| d[k]["ticks"].as_i64().or_else(|| d[k]["ticks"].as_str().and_then(|s| s.parse().ok()));
            let start = Tick(ticks("mStartTime").unwrap_or(0).clamp(0, MAX_TIME));
            let duration = Tick(ticks("mDuration").unwrap_or(0).clamp(0, MAX_TIME));
            let kind = match d["mType"].as_str() {
                Some("Chapter") => MarkerKind::Chapter,
                Some("Segmentation") => MarkerKind::Segmentation,
                Some("WebLink") => MarkerKind::WebLink,
                Some("FlashCuePoint") => MarkerKind::FlashCue,
                _ => MarkerKind::Comment,
            };
            out.push(Marker {
                id: MarkerId(self.b.alloc()),
                start,
                duration,
                name: d["mName"].as_str().unwrap_or("").to_string(),
                comment: d["mComment"].as_str().unwrap_or("").to_string(),
                kind,
                color: Label::Green,
            });
        }
        out.sort_by_key(|m| m.start);
        out
    }

    /// The media item for a `<Media>` object (deduplicated by its UID and by its file).
    /// The item for a Premiere media object. `master` is the master clip it was reached through
    /// (it says whether the clip is an adjustment layer).
    fn media(&mut self, media: Node<'a, 'i>, name: Option<&str>, bin: Option<BinId>, master: Option<Node<'a, 'i>>) -> Option<ItemId> {
        let key = media.attribute("ObjectUID").or_else(|| media.attribute("ObjectID"))?;
        let adjustment = master.and_then(|m| child_text(m, "IsAdjustmentLayer")).is_some_and(|v| v.trim() == "true");
        let adj_key = format!("adjustment:{key}");
        if let Some(id) = self.b.find_media(if adjustment { adj_key.as_str() } else { key }) {
            return Some(id);
        }
        if child_text(media, "ImplementationID").is_some_and(|i| i.trim().eq_ignore_ascii_case(GRAPHIC_IMPORTER)) {
            return Some(match synthetic_kind(child_text(media, "FilePath")) {
                _ if adjustment => self.adjustment_layer(media, name, bin, &adj_key),
                Some(g) => self.generator(media, name, bin, key, g),
                None => self.graphic(media, key),
            });
        }
        let path = self.media_path(media);
        if path.is_empty() {
            self.report.warn(format!("media \"{}\" has no file path and was not imported", name.unwrap_or("?")));
            return None;
        }
        // Premiere can hold several Media objects for one file (e.g. after relinking): one item.
        let path_key = format!("path:{path}");
        if let Some(id) = self.b.find_media(&path_key) {
            self.b.register_media(key, id);
            // remember every Premiere media object behind the item, for writing back
            if let Some(uid) = media.attribute("ObjectUID")
                && let Some(it) = self.b.p.item_mut(id)
            {
                let list = it.metadata.entry(META_MEDIA.to_string()).or_default();
                if !list.split(',').any(|u| u == uid) {
                    if !list.is_empty() {
                        list.push(',');
                    }
                    list.push_str(uid);
                }
            }
            return Some(id);
        }
        let title = name.or_else(|| child_text(media, "Title")).unwrap_or_else(|| file_name(&path)).to_string();
        let mut spec = MediaSpec::default();
        if let Some(v) = self.ix.follow(media, &["VideoStream"]) {
            let (w, h) = frame_rect(child_text(v, "FrameRect")).unwrap_or((1920, 1080));
            let rate = child_text(v, "FrameRate").and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_24);
            spec.video = Some((w, h, rate));
            spec.duration = time(child_text(v, "Duration")).filter(|d| *d > 0).map(Tick);
        }
        if let Some(a) = self.ix.follow(media, &["AudioStream"]) {
            let sr = child_text(a, "FrameRate").and_then(parse_i64).filter(|t| *t > 0).map(|t| (TICKS_PER_SECOND / t) as u32).unwrap_or(48_000);
            let ch =
                child_text(a, "AudioChannelLayout").and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()).and_then(|v| v.as_array().map(Vec::len));
            spec.audio = Some((sr, ch.unwrap_or(2).clamp(1, 32) as u32));
            if spec.duration.is_none() {
                spec.duration = time(child_text(a, "Duration")).filter(|d| *d > 0).map(Tick);
            }
        }
        let kind = kind_from_ext(&path);
        if matches!(kind, MediaKind::Still) {
            spec.kind = Some(kind);
            // Premiere gives stills a 12-hour placeholder duration; use FilmCraft's own default.
            spec.duration = None;
        }
        let id = self.b.file_media(key, &title, &path, &spec, bin);
        self.b.register_media(&path_key, id);
        if let Some(uid) = media.attribute("ObjectUID")
            && let Some(it) = self.b.p.item_mut(id)
        {
            it.metadata.insert(META_MEDIA.to_string(), uid.to_string());
        }
        Some(id)
    }

    /// A graphic made in Premiere's timeline (its titles): a FilmCraft graphic item the size of its
    /// canvas, whose clips get the text layers of their `AE.ADBE Text` components.
    fn graphic(&mut self, media: Node<'a, 'i>, key: &str) -> ItemId {
        let v = self.ix.follow(media, &["VideoStream"]);
        let (width, height) = v.and_then(|v| frame_rect(child_text(v, "FrameRect"))).unwrap_or((1920, 1080));
        let rate = v.and_then(|v| child_text(v, "FrameRate")).and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_30);
        let name = child_text(media, "Title").unwrap_or("Graphic");
        let id = self.b.p.add_item(name, Label::Rose, ItemKind::Graphic { width, height, rate }, None);
        self.b.register_media(key, id);
        if let Some(uid) = media.attribute("ObjectUID")
            && let Some(it) = self.b.p.item_mut(id)
        {
            it.metadata.insert(META_MEDIA.to_string(), uid.to_string());
        }
        id
    }

    /// A Premiere adjustment layer (a master clip marked `IsAdjustmentLayer` over a synthetic
    /// "Black Video"): FilmCraft's adjustment layer, its effects applying to the tracks below.
    fn adjustment_layer(&mut self, media: Node<'a, 'i>, name: Option<&str>, bin: Option<BinId>, key: &str) -> ItemId {
        let v = self.ix.follow(media, &["VideoStream"]);
        let (width, height) = v.and_then(|v| frame_rect(child_text(v, "FrameRect"))).unwrap_or((1920, 1080));
        let rate = v.and_then(|v| child_text(v, "FrameRate")).and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_30);
        let duration = v.and_then(|v| time(child_text(v, "Duration"))).filter(|d| *d > 0).map_or(Tick(MAX_TIME), Tick);
        let id = self.b.p.add_item(name.unwrap_or("Adjustment Layer"), Label::Lavender, ItemKind::AdjustmentLayer { width, height, rate, duration }, bin);
        self.b.register_media(key, id);
        if let Some(uid) = media.attribute("ObjectUID")
            && let Some(it) = self.b.p.item_mut(id)
        {
            it.metadata.insert(META_MEDIA.to_string(), uid.to_string());
        }
        id
    }

    /// Premiere's synthetic media (Color Matte, Black Video, Bars and Tone, Transparent Video) as
    /// FilmCraft's generators of the same name. A Color Matte's colour is the first three bytes
    /// (R, G, B) of the media's `ImporterPrefs`.
    fn generator(&mut self, media: Node<'a, 'i>, name: Option<&str>, bin: Option<BinId>, key: &str, kind: Synthetic) -> ItemId {
        let g = match kind {
            Synthetic::ColorMatte => {
                let prefs = child(media, "ImporterPrefs").and_then(|n| self.binary(n));
                let color = prefs
                    .and_then(|b| b.get(..3).map(|c| [c[0], c[1], c[2]]))
                    .map_or([0.0, 0.0, 0.0, 1.0], |[r, g, b]| [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0]);
                Generator::ColorMatte { color }
            }
            Synthetic::Black => Generator::BlackVideo,
            Synthetic::Bars => Generator::BarsAndTone,
            Synthetic::Transparent => Generator::TransparentVideo,
        };
        let mut spec = MediaSpec::default();
        if let Some(v) = self.ix.follow(media, &["VideoStream"]) {
            let (w, h) = frame_rect(child_text(v, "FrameRect")).unwrap_or((1920, 1080));
            let rate = child_text(v, "FrameRate").and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_30);
            spec.video = Some((w, h, rate));
            spec.duration = time(child_text(v, "Duration")).filter(|d| *d > 0).map(Tick);
        }
        if let Some(a) = self.ix.follow(media, &["AudioStream"]) {
            let sr = child_text(a, "FrameRate").and_then(parse_i64).filter(|t| *t > 0).map(|t| (TICKS_PER_SECOND / t) as u32).unwrap_or(48_000);
            spec.audio = Some((sr, 2));
        }
        let title = name.or_else(|| child_text(media, "Title")).unwrap_or("Generated").to_string();
        let id = self.b.generator_media(key, &title, g, &spec, bin);
        if let Some(uid) = media.attribute("ObjectUID")
            && let Some(it) = self.b.p.item_mut(id)
        {
            it.metadata.insert(META_MEDIA.to_string(), uid.to_string());
        }
        id
    }

    /// The bytes of a binary value (`Encoding="base64"`), stored in the element or, when another
    /// element with the same `BinaryHash` holds them, there.
    fn binary(&mut self, el: Node<'a, 'i>) -> Option<Vec<u8>> {
        let root = self.root;
        let map = self.payloads.get_or_insert_with(|| payloads(root));
        binary_in(map, el)
    }

    /// A Premiere text layer as a FilmCraft text layer: the decoded `Source Text` (text, font,
    /// size, fill, stroke, paragraph box and alignment) and the layer's transform.
    fn title(&mut self, ti: &mut filmcraft_project::TrackItem, c: &prproj_fx::Component<'a, 'i>, ctx: &Ctx) {
        let Some(def) = filmcraft_project::effect::find_effect(filmcraft_project::graphic::TEXT_LAYER) else { return };
        let source = c.params.iter().copied().find(|p| child_text(*p, "Name").is_some_and(|n| n.trim() == "Source Text"));
        let decoded = source.and_then(|p| child(p, "StartKeyframeValue")).and_then(|v| self.binary(v)).and_then(|b| crate::prproj_title::Tree::read(&b));
        let Some(tree) = decoded else {
            self.report.warn("a Premiere title's text could not be read; it was imported empty");
            ti.effects.push(def.instance());
            return;
        };
        if source.is_some_and(|p| child(p, "Keyframes").is_some()) {
            self.report.info("animated title text was imported as its first value");
        }
        let mut e = def.instance();
        let unread = crate::prproj_title::to_layer(&tree, &mut e);
        for (what, on) in [("gradient fills", unread.gradient), ("kerning between characters", unread.kerning), ("more than two strokes", unread.more_strokes)]
        {
            if on {
                self.unmapped.entry("Text".to_string()).or_default().insert(what.to_string());
            }
        }
        // the transform, by name as for any effect
        let (maps, unmapped) = prproj_fx::mappings(c, prproj_fx::Target::Effect(def));
        for m in maps {
            if let Some(p) = prproj_fx::import_param(&m, ctx) {
                e.params.insert(m.param.to_string(), p);
            }
        }
        // Horizontal Scale is FilmCraft's Scale Width
        if let Some(hs) = c.params.iter().copied().find(|p| child_text(*p, "Name").is_some_and(|n| n.trim() == "Horizontal Scale"))
            && let Some(v) = prproj_fx::start_value(hs).and_then(|v| v.trim_end_matches('.').parse::<f64>().ok())
            && (v - 100.0).abs() > 1e-6
        {
            e.params.insert("scale_width".into(), Param::new(ParamValue::Float(v)));
            e.params.insert("uniform_scale".into(), Param::new(ParamValue::Bool(false)));
        }
        const READ: [&str; 8] = ["Source Text", "Transform", "Horizontal Scale", "Parent Width", "Parent Height", "Parent Rotation", "start", "end"];
        let rest: Vec<String> = unmapped.into_iter().filter(|n| !READ.contains(&n.as_str())).collect();
        if !rest.is_empty() {
            self.unmapped.entry("Text".to_string()).or_default().extend(rest);
        }
        ti.effects.push(e);
    }

    /// Where the media file is: Premiere's project-relative path when it is a plain relative path
    /// (that is what Premiere itself tries first after a project moves), else the absolute path.
    fn media_path(&self, media: Node<'a, 'i>) -> String {
        let absolute = child_text(media, "ActualMediaFilePath").or_else(|| child_text(media, "FilePath")).unwrap_or("").trim();
        // Premiere escapes `RelativePath` twice (`&amp;amp;` for `&`); earlier FilmCraft versions
        // wrote that into every path of projects they created
        let absolute = crate::common::unescape_twice(absolute);
        let relative = child_text(media, "RelativePath").map(str::trim).filter(|r| !r.is_empty() && !r.contains(':') && !is_absolute(r));
        if let (Some(rel), Some(base)) = (relative, self.opts.base_dir.as_deref()) {
            let resolved = resolve_path(&crate::common::unescape_twice(rel).replace('\\', "/"), Some(base));
            // the relative path where it finds the file (or nothing finds it), else the absolute one
            let found = |p: &str| std::path::Path::new(p).exists();
            if found(&resolved) || !found(&absolute) {
                return resolved;
            }
        }
        absolute
    }

    // -----------------------------------------------------------------------------------------
    // Sequences
    // -----------------------------------------------------------------------------------------

    fn reserve_sequence(&mut self, seq: Node<'a, 'i>, name: &str, bin: Option<BinId>) {
        let Some(uid) = seq.attribute("ObjectUID") else { return };
        let vg = prproj_graph::track_group(&self.ix, seq, "VideoTrackGroup");
        let (w, h) = vg.and_then(|g| frame_rect(child_text(g, "FrameRect"))).unwrap_or((1920, 1080));
        let rate = vg.and_then(|g| path_text(g, &["TrackGroup", "FrameRate"])).and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_24);
        let mut settings = settings_for(rate, w, h, false);
        if let Some(t) = prproj_graph::track_group(&self.ix, seq, "AudioTrackGroup")
            .and_then(|g| path_text(g, &["TrackGroup", "FrameRate"]))
            .and_then(parse_i64)
            .filter(|t| *t > 0)
        {
            settings.sample_rate = (TICKS_PER_SECOND / t) as u32;
        }
        let name = crate::xml::name_text(seq, "Name").unwrap_or(name);
        let id = self.b.reserve_sequence(name, settings, bin);
        if let Some(it) = self.b.p.item_mut(id) {
            it.metadata.insert(META_SEQUENCE.to_string(), uid.to_string());
            it.metadata.insert(META_FINGERPRINT.to_string(), self.fingerprint.clone());
            if let Some(src) = &self.opts.source_path {
                it.metadata.insert(META_SOURCE.to_string(), src.clone());
            }
        }
        self.seq_items.insert(uid, id);
        self.seq_nodes.push((seq, id));
    }

    fn fill_sequence(&mut self, node: Node<'a, 'i>, id: ItemId) {
        let Some(settings) = self.b.p.sequence_mut(id).map(|s| s.settings.clone()) else { return };
        let mut seq: Sequence = empty_sequence(settings.clone());
        let frame = (settings.width as f64, settings.height as f64);
        for (kind, tag) in [(TrackKind::Video, "VideoTrackGroup"), (TrackKind::Audio, "AudioTrackGroup")] {
            let Some(group) = prproj_graph::track_group(&self.ix, node, tag) else { continue };
            for (ti, track) in prproj_graph::tracks(&self.ix, group).into_iter().enumerate() {
                self.b.ensure_tracks(&mut seq, kind, ti + 1);
                if let Some(name) = path_text(track, &["ClipTrack", "Track", "Name"]).filter(|n| !n.trim().is_empty())
                    && let Some(t) = seq.tracks_mut(kind).get_mut(ti)
                {
                    t.name = name.to_string();
                }
                self.track(&mut seq, kind, ti, track, frame);
            }
        }
        if seq.video_tracks.is_empty() {
            self.b.ensure_tracks(&mut seq, TrackKind::Video, 1);
        }
        if seq.audio_tracks.is_empty() {
            self.b.ensure_tracks(&mut seq, TrackKind::Audio, 1);
        }
        // sequence markers: the Sequence's own (where Premiere lists them) and those on its master
        // clip (notes some tools put there; Premiere does not list them, FilmCraft shows them too)
        let mut markers = node.attribute("ObjectUID").and_then(|uid| self.seq_markers.remove(uid)).unwrap_or_default();
        if let Some(own) = self.ix.follow(node, &["MarkerOwner", "Markers"]) {
            markers.extend(self.markers(own));
        }
        markers.sort_by_key(|m| m.start);
        seq.markers = markers;
        self.b.put_sequence(id, seq);
    }

    fn track(&mut self, seq: &mut Sequence, kind: TrackKind, ti: usize, track: Node<'a, 'i>, frame: (f64, f64)) {
        let items = self.ix.list(track, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem");
        let transitions = self.ix.list(track, &["ClipTrack", "TransitionItems", "TrackItems"], "TrackItem");
        for item in items {
            if let Some(clip) = self.clip(item, kind, frame) {
                match self.b.place(seq, kind, ti, clip) {
                    Some(placed) if placed != ti => self.report.warn("overlapping clips on one track; the later one was moved to a new track"),
                    Some(_) => {}
                    None => {
                        self.report.warn(format!("{kind:?} track exceeds the {}-track import limit; a clip was skipped", crate::common::MAX_IMPORTED_TRACKS))
                    }
                }
            }
        }
        for t in transitions {
            self.transition(seq, kind, ti, t);
        }
    }

    fn clip(&mut self, item: Node<'a, 'i>, kind: TrackKind, frame: (f64, f64)) -> Option<filmcraft_project::TrackItem> {
        let tag = item.tag_name().name();
        if !matches!(tag, "VideoClipTrackItem" | "AudioClipTrackItem") {
            self.report.info(format!("track item type {tag} is not supported"));
            return None;
        }
        let ti_node = self.ix.at(item, &["ClipTrackItem", "TrackItem"])?;
        let start = time(child_text(ti_node, "Start")).unwrap_or(0);
        let end = time(child_text(ti_node, "End"))?;
        if end <= start {
            return None;
        }
        let sub = self.ix.follow(item, &["ClipTrackItem", "SubClip"])?;
        let clip = self.ix.follow(sub, &["Clip"])?;
        let name = crate::xml::name_text(sub, "Name").unwrap_or("Clip").to_string();
        let target = if let Some(seq) = prproj_graph::clip_sequence(&self.ix, clip) {
            let id = seq.attribute("ObjectUID").and_then(|u| self.seq_items.get(u).copied());
            if let Some(id) = id {
                self.nested.insert(id);
            }
            id
        } else if let Some(media) = prproj_graph::clip_media(&self.ix, clip) {
            let master = self.ix.follow(sub, &["MasterClip"]);
            self.media(media, None, None, master)
        } else {
            None
        };
        let Some(target) = target else {
            self.report.warn(format!("clip \"{name}\" has no media or sequence the importer understands; skipped"));
            return None;
        };
        let source_in = time(path_text(clip, &["Clip", "InPoint"])).unwrap_or(0);
        let raw_speed =
            path_text(clip, &["Clip", "PlaybackSpeed"]).and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s != 0.0).unwrap_or(1.0);
        if !(SPEED_RANGE.0..=SPEED_RANGE.1).contains(&raw_speed.abs()) {
            self.report.warn(format!("clip \"{name}\" has a speed outside 0.1 %–10 000 %; it was clamped"));
        }
        let duration = end.saturating_sub(start);
        let mut ti = self.b.clip(target, kind, &name, Tick(start), Tick(duration), Tick(source_in));
        ti.speed = raw_speed.abs().clamp(SPEED_RANGE.0, SPEED_RANGE.1);
        ti.reverse = raw_speed < 0.0;
        ti.origin = item.attribute("ObjectID").map(str::to_string);
        // the clip's own label (else its item's, as `Builder::clip` gave it)
        if let Some(l) = clip_label(clip) {
            ti.label = l;
        }
        // Clip ▸ Enable off: Premiere saves `IsMuted` on the track item (video and audio alike)
        ti.enabled = path_text(item, &["ClipTrackItem", "IsMuted"]).map(str::trim) != Some("true");
        if kind == TrackKind::Audio {
            ti.gain_db = clip_gain_db(clip);
        }
        if child_text(clip, "FrameHold") == Some(FRAME_HOLD_AT)
            && let Some(at) = time(child_text(clip, "FrameHoldStart"))
        {
            ti.frame_hold = Some(Tick(at));
        }
        if let Some(id) = item.attribute("ObjectID")
            && let Some(g) = self.links.get(id)
        {
            ti.link = Some(*g);
        }
        let ctx = Ctx { speed: ti.speed, frame, src: self.source_frame(target).unwrap_or(frame) };
        if let Some(chain) = self.ix.follow(item, &["ClipTrackItem", "ComponentOwner", "Components"]) {
            self.components(&mut ti, chain, &ctx);
        }
        Some(ti)
    }

    fn source_frame(&self, item: ItemId) -> Option<(f64, f64)> {
        source_frame(&self.b.p, item)
    }

    // -----------------------------------------------------------------------------------------
    // Effects
    // -----------------------------------------------------------------------------------------

    fn components(&mut self, ti: &mut filmcraft_project::TrackItem, chain: Node<'a, 'i>, ctx: &Ctx) {
        let comps = prproj_fx::chain(&self.ix, chain);
        let targets = prproj_fx::targets(&comps);
        let titled = self.b.p.item(ti.item).is_some_and(|i| matches!(i.kind, ItemKind::Graphic { .. }));
        for (c, target) in comps.iter().zip(targets) {
            if titled && c.match_name == TEXT_COMPONENT {
                self.title(ti, c, ctx);
                continue;
            }
            match target {
                Target::Ignore => continue,
                Target::Unsupported => {
                    self.report.warn(format!("effect \"{}\" is not supported and was skipped (kept as it is when exporting back to Premiere)", c.label()));
                    continue;
                }
                Target::Effect(def) => {
                    // an effect FilmCraft has: one instance per Premiere component, in order
                    let mut e = def.instance();
                    // Premiere's audio effect settings are saved on scales of their own: those
                    // FilmCraft reads come in as they are (switched off where Premiere bypasses
                    // them); the others are shown switched off rather than played with other
                    // settings (Premiere keeps its own: on/off is not written back)
                    if def.kind == filmcraft_project::EffectKind::Audio {
                        let bypassed = c
                            .params
                            .iter()
                            .find(|p| child_text(**p, "Name").is_some_and(|n| n.trim().eq_ignore_ascii_case("Bypass")))
                            .and_then(|p| prproj_fx::start_value(*p))
                            .is_some_and(|v| v == "true" || v == "1");
                        if bypassed {
                            e.enabled = false;
                        } else if !prproj_fx::audio_effect_read(def.id) {
                            e.enabled = false;
                            *self.audio_off.entry(def.name.to_string()).or_insert(0) += 1;
                        }
                    }
                    ti.effects.push(e);
                }
                Target::Motion | Target::Opacity | Target::Volume => {}
            }
            let (maps, unmapped) = prproj_fx::mappings(c, target);
            let audio = matches!(target, Target::Effect(d) if d.kind == filmcraft_project::EffectKind::Audio);
            if !unmapped.is_empty() && !audio {
                let label = match target {
                    Target::Effect(d) => d.name.to_string(),
                    _ => c.label().to_string(),
                };
                self.unmapped.entry(label).or_default().extend(unmapped);
            }
            // a FilmCraft parameter several Premiere ones map onto (Amplify's Left and Right) takes
            // the first
            let mut taken = std::collections::HashSet::new();
            for m in maps {
                if !taken.insert((m.effect, m.param)) {
                    continue;
                }
                let Some(p) = prproj_fx::import_param(&m, ctx) else { continue };
                if let Target::Effect(_) = target {
                    // the instance just pushed is the last one with this id
                    if let Some(e) = ti.effects.iter_mut().rev().find(|e| e.effect == m.effect) {
                        e.params.insert(m.param.to_string(), p);
                    }
                } else {
                    set_param(ti, m.effect, m.param, p);
                }
            }
            self.masks(ti, c, target, ctx);
        }
    }

    /// The masks of an effect (and of Opacity, where they cut out the clip). Without them a
    /// masked effect would apply to the whole frame.
    fn masks(&mut self, ti: &mut filmcraft_project::TrackItem, c: &prproj_fx::Component<'a, 'i>, target: Target, ctx: &Ctx) {
        let id = match target {
            Target::Effect(def) => def.id,
            Target::Opacity => "opacity",
            _ => return,
        };
        if prproj_mask::components(&self.ix, c.node).is_empty() {
            return;
        }
        let root = self.root;
        let map = self.payloads.get_or_insert_with(|| payloads(root));
        let read = prproj_mask::read(&self.ix, c.node, ctx, &mut |n| binary_in(map, n));
        for why in read.skipped {
            *self.mask_skips.entry((c.label().to_string(), why)).or_insert(0) += 1;
        }
        if read.masks.is_empty() {
            return;
        }
        // the instance pushed for this component is the last one with this id
        if let Some(e) = ti.effects.iter_mut().rev().find(|e| e.effect == id) {
            e.masks = read.masks;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Transitions
    // -----------------------------------------------------------------------------------------

    fn transition(&mut self, seq: &mut Sequence, kind: TrackKind, ti: usize, t: Node<'a, 'i>) {
        let Some(tt) = child(t, "TransitionTrackItem") else { return };
        let Some(range) = child(tt, "TrackItem") else { return };
        let s = time(child_text(range, "Start")).unwrap_or(0);
        let Some(e) = time(child_text(range, "End")) else { return };
        if e <= s {
            return;
        }
        let name = child_text(tt, "DisplayName").unwrap_or("Cross Dissolve");
        let outgoing = child_text(tt, "HasOutgoingClip").is_none_or(|v| v == "true");
        let incoming = child_text(tt, "HasIncomingClip").is_none_or(|v| v == "true");
        let mut effect = transition_effect(name, kind == TrackKind::Audio, self.report);
        if effect.effect == filmcraft_project::effect::CUSTOM_FADE
            && let Some(shape) = child_text(t, "FadeShapeValue").and_then(|v| v.trim().parse::<i32>().ok())
        {
            effect.params.insert("shape".into(), filmcraft_project::Param::new(filmcraft_project::ParamValue::Float(f64::from(shape.clamp(-100, 100)))));
        }
        // where Premiere puts the cut (`Alignment` from the start): a clip's fade can run a frame
        // past the clip it belongs to
        let at = time(child_text(tt, "Alignment")).filter(|a| *a == 0 || *a == e - s);
        let (s, e) = (Tick(s), Tick(e));
        let Some(track) = seq.tracks_mut(kind).get_mut(ti) else { return };
        let from = outgoing.then(|| track.items.iter().filter(|i| i.end() >= s && i.end() <= e).max_by_key(|i| i.end()).map(|i| (i.id, i.end()))).flatten();
        let to = incoming.then(|| track.items.iter().filter(|i| i.start >= s && i.start <= e).min_by_key(|i| i.start).map(|i| (i.id, i.start))).flatten();
        let cut = from.map(|f| f.1).or(to.map(|t| t.1));
        let Some(cut) = cut else {
            self.report.warn(format!("transition \"{name}\" has no adjacent clip; skipped"));
            return;
        };
        let align = match at.map(|a| s + Tick(a)) {
            Some(c) if c == s && from.is_none() => TransitionAlign::StartAtCut,
            Some(c) if c == e && to.is_none() => TransitionAlign::EndAtCut,
            _ if cut == s => TransitionAlign::StartAtCut,
            _ if cut == e => TransitionAlign::EndAtCut,
            _ => TransitionAlign::CenterAtCut,
        };
        let id = TransitionId(self.b.alloc());
        let (from, to): (Option<ClipId>, Option<ClipId>) = (from.map(|f| f.0), to.map(|t| t.0));
        track.transitions.push(Transition { id, effect, start: s, duration: e - s, from, to, align, reverse: false });
        track.sort();
    }
}

/// Premiere's synthetic media FilmCraft has a generator for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Synthetic {
    ColorMatte,
    Black,
    Bars,
    Transparent,
}

/// The synthetic media a `FilePath` type code names (a four-character code as a number:
/// `COLR` Color Matte, `BLAK` Black Video, `BARS` Bars and Tone, `TRNV` Transparent Video).
/// Graphics (`GRFV`) and anything else are `None`.
fn synthetic_kind(path: Option<&str>) -> Option<Synthetic> {
    let code: u32 = path?.trim().parse().ok()?;
    match &code.to_be_bytes() {
        b"COLR" => Some(Synthetic::ColorMatte),
        b"BLAK" => Some(Synthetic::Black),
        b"BARS" => Some(Synthetic::Bars),
        b"TRNV" => Some(Synthetic::Transparent),
        _ => None,
    }
}

/// Every inline binary payload of the document by its `BinaryHash` (the first one wins).
pub(crate) fn payloads<'a>(root: Node<'a, '_>) -> HashMap<&'a str, &'a str> {
    let mut m = HashMap::new();
    for n in root.descendants().filter(Node::is_element) {
        if let (Some(h), Some(t)) = (n.attribute("BinaryHash"), n.text().map(str::trim).filter(|t| !t.is_empty())) {
            m.entry(h).or_insert(t);
        }
    }
    m
}

/// The bytes of a base64 element: its own text, else the payload stored under its `BinaryHash`.
pub(crate) fn binary_in(payloads: &HashMap<&str, &str>, el: Node<'_, '_>) -> Option<Vec<u8>> {
    let own = el.text().map(str::trim).filter(|t| !t.is_empty());
    let text = match own {
        Some(t) => t,
        None => payloads.get(el.attribute("BinaryHash")?).copied()?,
    };
    crate::prproj_text::base64(text)
}

/// Whether the importer brings a Premiere track item into FilmCraft. The writer keeps every item
/// that it does not (FilmCraft never showed it, so it cannot have been edited away).
pub(crate) fn importable(ix: &Index<'_, '_>, item: Node<'_, '_>) -> bool {
    if !matches!(item.tag_name().name(), "VideoClipTrackItem" | "AudioClipTrackItem") {
        return false;
    }
    let Some(ti) = ix.at(item, &["ClipTrackItem", "TrackItem"]) else { return false };
    let (start, Some(end)) = (time(child_text(ti, "Start")).unwrap_or(0), time(child_text(ti, "End"))) else { return false };
    if end <= start {
        return false;
    }
    let Some(clip) = ix.follow(item, &["ClipTrackItem", "SubClip"]).and_then(|sub| ix.follow(sub, &["Clip"])) else { return false };
    if let Some(seq) = prproj_graph::clip_sequence(ix, clip) {
        return seq.attribute("ObjectUID").is_some();
    }
    prproj_graph::clip_media(ix, clip).is_some_and(|m| {
        (m.attribute("ObjectUID").is_some() || m.attribute("ObjectID").is_some())
            && ["ActualMediaFilePath", "FilePath", "RelativePath"].iter().any(|t| child_text(m, t).is_some_and(|v| !v.trim().is_empty()))
    })
}

/// The picture size of a clip's source (media or nested sequence).
pub(crate) fn source_frame(p: &filmcraft_project::Project, item: ItemId) -> Option<(f64, f64)> {
    match &p.item(item)?.kind {
        filmcraft_project::ItemKind::Media(m) => m.info.video.as_ref().map(|v| (v.width as f64, v.height as f64)),
        filmcraft_project::ItemKind::Sequence(s) => Some((s.settings.width as f64, s.settings.height as f64)),
        filmcraft_project::ItemKind::Graphic { width, height, .. } | filmcraft_project::ItemKind::AdjustmentLayer { width, height, .. } => {
            Some((*width as f64, *height as f64))
        }
        _ => None,
    }
}

pub(crate) fn parse_i64(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// `"l,t,r,b"` → (width, height).
pub(crate) fn frame_rect(s: Option<&str>) -> Option<(u32, u32)> {
    let v: Vec<i64> = s?.split(',').filter_map(parse_i64).collect();
    let (l, t, r, b) = (*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?);
    let (w, h) = (r.checked_sub(l)?, b.checked_sub(t)?);
    (w > 0 && h > 0 && w <= 65_536 && h <= 65_536).then_some((w as u32, h as u32))
}

/// Ticks per frame → frame rate (exact: 8 467 200 000 → 30, 10 594 584 000 → 24000/1001).
pub(crate) fn rate_of(ticks_per_frame: i64) -> Option<FrameRate> {
    (ticks_per_frame > 0 && TICKS_PER_SECOND / ticks_per_frame <= 1000).then(|| FrameRate::new(TICKS_PER_SECOND, ticks_per_frame))
}

/// Clip ▸ Audio Gain of a clip (`Gain`, linear) in dB; 0 without one.
/// Premiere's label of a clip object (`Clip ▸ Node ▸ Properties ▸ asl.clip.label.name`: a
/// `BE.Prefs.LabelColors.N` index into its sixteen labels, in the order of [`Label::ALL`]).
pub(crate) fn clip_label(clip: Node<'_, '_>) -> Option<Label> {
    label_named(path_text(clip, &["Clip", "Node", "Properties", "asl.clip.label.name"])?)
}

/// The label Premiere's Project panel shows for a `ClipProjectItem` (its Label column,
/// `ProjectItem ▸ Node ▸ Properties ▸ Column.PropertyText.Label`), which can differ from its
/// master clip's.
pub(crate) fn project_item_label(item: Node<'_, '_>) -> Option<Label> {
    label_named(path_text(item, &["ProjectItem", "Node", "Properties", PROJECT_LABEL_COLUMN])?)
}

/// The Project panel's Label column in a `ClipProjectItem`'s properties.
pub(crate) const PROJECT_LABEL_COLUMN: &str = "Column.PropertyText.Label";

fn label_named(name: &str) -> Option<Label> {
    let n: usize = name.trim().strip_prefix("BE.Prefs.LabelColors.")?.parse().ok()?;
    Label::ALL.get(n).copied()
}

/// What Premiere saves for a label: its name (`BE.Prefs.LabelColors.N`) and its colour, packed
/// red + green·256 + blue·65536 (the default colours seen in projects; FilmCraft's own for the rest).
pub(crate) fn label_props(label: Label) -> (String, u32) {
    let n = Label::ALL.iter().position(|l| *l == label).unwrap_or(1);
    let seen = match n {
        0 => Some(11_405_886),
        1 => Some(6_769_408),
        2 => Some(480_554),
        3 => Some(8_851_829),
        5 => Some(19_005),
        _ => None,
    };
    let [r, g, b] = label.rgb();
    (format!("BE.Prefs.LabelColors.{n}"), seen.unwrap_or(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16))
}

pub(crate) fn clip_gain_db(clip: Node<'_, '_>) -> f64 {
    child_text(clip, "Gain")
        .and_then(|g| g.trim().parse::<f64>().ok())
        .filter(|g| g.is_finite() && *g > 0.0)
        .map(|g| (20.0 * g.log10()).clamp(-96.0, 96.0))
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "prproj_tests.rs"]
pub(crate) mod tests;

/// The interchange tests' document (see [`crate::prproj_test_document`]), with `\n` line ends
/// whatever the checkout turned them into (Git's `core.autocrlf` writes CRLF on Windows).
pub(crate) fn test_document() -> String {
    include_str!("prproj_test_document.xml").replace("\r\n", "\n")
}
