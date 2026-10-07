//! Premiere Pro project (`.prproj`) import.
//!
//! There is no published specification for this format. Everything here is derived from
//! observing project files saved by Premiere Pro (gzip-compressed XML, root `<PremiereData
//! Version="3">`); no application internals were inspected. The observations are recorded in this
//! crate's README (`prproj` section). Import is one-way: FilmCraft never writes `.prproj`.
//!
//! The document is a flat list of objects under the root, each with an `ObjectID` (integer) or
//! `ObjectUID` (UUID). Objects point at each other with `ObjectRef` / `ObjectURef` attributes; only
//! the root's direct children are addressable (nested elements may reuse ids locally). Times are
//! Premiere ticks, 254 016 000 000 per second, which is exactly [`Tick`]'s resolution.
//!
//! Mapped: the bin tree, media (path, picture size, frame rate, sample rate, channels), sequences
//! (frame size, rate, sample rate, track names), clips (position, source in point, speed,
//! reverse), nested sequences, video/audio links, transitions, Motion (position, scale, scale
//! width, uniform scale, rotation, anchor point, anti-flicker) and Opacity with keyframes, clip
//! volume, and markers on media and sequences. Everything else (third-party and most native
//! effects, titles made with the legacy Text effect, masks, Essential Sound) is listed in the
//! [`Report`].

use std::collections::{HashMap, HashSet};

use filmcraft_geom::Vec2;
use filmcraft_media::MediaKind;
use filmcraft_project::{
    BinId, ClipId, Interpolation, ItemId, Keyframe, Label, Marker, MarkerId, MarkerKind, Param, ParamValue, Sequence, TrackKind, Transition, TransitionAlign,
    TransitionId,
};
use filmcraft_time::{FrameRate, Tick};
use roxmltree::Node;

use crate::common::{
    Builder, MediaSpec, empty_sequence, file_name, gain_to_db, is_absolute, kind_from_ext, resolve_path, set_param, settings_for, transition_effect,
};
use crate::xml::{child, child_text, path_text};
use crate::{Error, Format, ImportOptions, Imported, Report, Result};

/// Premiere ticks per second (identical to FilmCraft's [`Tick`] resolution).
const TICKS_PER_SECOND: i64 = 254_016_000_000;
/// Inflated documents larger than this are refused (a 2-hour, 5 000-clip project is ~100 MB).
const MAX_XML_BYTES: usize = 1 << 30;
/// Stored clip volume of a clip at 0 dB (observed: every untouched clip carries this value, i.e. the
/// stored value is linear gain with +15 dB of headroom above 1.0).
const UNITY_LEVEL: f64 = 0.177_827_941_003_892_3;
/// `StartKeyframe` time meaning "before the start of time" (a static value).
const STATIC_TIME: i64 = -91_445_760_000_000_000;
/// Nested bins deeper than this are not followed.
const MAX_DEPTH: usize = 64;
/// Times from the document are clamped to 48 hours, speeds to 0.1 %–10 000 %, so that no
/// position, duration or source range can overflow [`Tick`] arithmetic.
const MAX_TIME: i64 = 48 * 3600 * TICKS_PER_SECOND;
const SPEED_RANGE: (f64, f64) = (0.001, 100.0);

/// A time read from the document, clamped to `0..=MAX_TIME`.
fn time(s: Option<&str>) -> Option<i64> {
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

/// Undo the gzip wrapper (RFC 1952) if there is one.
fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let bad = |m: &str| Error::parse(Format::Prproj, m.to_string());
    let header = bytes.get(..10).ok_or_else(|| bad("truncated gzip header"))?;
    if header.get(2) != Some(&8) {
        return Err(bad("unsupported gzip compression method"));
    }
    let flags = header.get(3).copied().unwrap_or(0);
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        // FEXTRA: two-byte little-endian length, then the field
        let len = bytes.get(pos..pos + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| bad("truncated gzip extra field"))?;
        pos = pos.saturating_add(2).saturating_add(len);
    }
    for flag in [0x08u8, 0x10] {
        // FNAME, FCOMMENT: zero-terminated strings
        if flags & flag != 0 {
            let rest = bytes.get(pos..).ok_or_else(|| bad("truncated gzip header"))?;
            let end = rest.iter().position(|&b| b == 0).ok_or_else(|| bad("unterminated gzip header string"))?;
            pos = pos.saturating_add(end + 1);
        }
    }
    if flags & 0x02 != 0 {
        pos = pos.saturating_add(2); // FHCRC
    }
    let body = bytes.get(pos..).ok_or_else(|| bad("truncated gzip stream"))?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_XML_BYTES).map_err(|e| bad(&format!("gzip data is damaged or too large ({:?})", e.status)))
}

/// Import a Premiere project.
pub(crate) fn import(bytes: &[u8], opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    let raw = decompress(bytes)?;
    let text = String::from_utf8_lossy(&raw);
    let doc = crate::xml::parse(&text).map_err(|e| Error::parse(Format::Prproj, e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PremiereData" {
        return Err(Error::parse(Format::Prproj, "root element is not <PremiereData>"));
    }
    let mut ids = HashMap::new();
    let mut uids = HashMap::new();
    for n in root.children().filter(Node::is_element) {
        if let Some(id) = n.attribute("ObjectID") {
            ids.insert(id, n);
        }
        if let Some(uid) = n.attribute("ObjectUID") {
            uids.insert(uid, n);
        }
    }
    let name = opts.name.clone().unwrap_or_else(|| "Premiere Project".to_string());
    let mut imp = Imp {
        b: Builder::new(&name),
        opts,
        report,
        ids,
        uids,
        seq_items: HashMap::new(),
        seq_nodes: Vec::new(),
        seq_markers: HashMap::new(),
        links: HashMap::new(),
        nested: HashSet::new(),
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
            imp.reserve_sequence(n, child_text(n, "Name").unwrap_or("Sequence"), None);
        }
    }
    let seqs = std::mem::take(&mut imp.seq_nodes);
    for (node, id) in &seqs {
        imp.fill_sequence(*node, *id);
    }
    let mut top: Vec<ItemId> = seqs.iter().map(|(_, id)| *id).filter(|id| !imp.nested.contains(id)).collect();
    if top.is_empty() {
        top = seqs.iter().map(|(_, id)| *id).collect();
    }
    if top.is_empty() && imp.b.p.items.is_empty() {
        return Err(Error::Empty);
    }
    imp.b.top = top;
    Ok(imp.b.finish())
}

struct Imp<'a, 'i, 'r> {
    b: Builder,
    opts: &'r ImportOptions,
    report: &'r mut Report,
    ids: HashMap<&'a str, Node<'a, 'i>>,
    uids: HashMap<&'a str, Node<'a, 'i>>,
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
}

impl<'a, 'i, 'r> Imp<'a, 'i, 'r> {
    /// The object an `ObjectRef` / `ObjectURef` element points at.
    fn resolve(&self, r: Option<Node<'a, 'i>>) -> Option<Node<'a, 'i>> {
        let r = r?;
        if let Some(id) = r.attribute("ObjectRef") {
            return self.ids.get(id).copied();
        }
        r.attribute("ObjectURef").and_then(|u| self.uids.get(u).copied())
    }

    fn at(&self, n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
        let mut cur = n;
        for p in path {
            cur = child(cur, p)?;
        }
        Some(cur)
    }

    fn follow(&self, n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
        self.resolve(self.at(n, path))
    }

    fn collect_links(&mut self, root: Node<'a, 'i>) {
        for l in root.children().filter(|n| n.has_tag_name("Link")) {
            let Some(items) = self.at(l, &["TrackItemGroup", "TrackItems"]) else { continue };
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
        let Some(items) = self.at(container, &["ProjectItemContainer", "Items"]) else { return };
        for r in items.children().filter(|n| n.has_tag_name("Item")) {
            let Some(item) = self.resolve(Some(r)) else { continue };
            let name = path_text(item, &["ProjectItem", "Name"]).unwrap_or("Untitled");
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
        let Some(master) = self.follow(item, &["MasterClip"]) else { return };
        let markers = self.master_markers(master);
        for c in self.at(master, &["Clips"]).into_iter().flat_map(|c| c.children().filter(|n| n.has_tag_name("Clip"))) {
            let Some(clip) = self.resolve(Some(c)) else { continue };
            let Some(source) = self.follow(clip, &["Clip", "Source"]) else { continue };
            if let Some(seq) = self.follow(source, &["SequenceSource", "Sequence"]) {
                if let Some(uid) = seq.attribute("ObjectUID") {
                    if !self.seq_items.contains_key(uid) {
                        self.reserve_sequence(seq, name, bin);
                    }
                    self.seq_markers.entry(uid).or_insert(markers);
                }
                return;
            }
            if let Some(media) = self.follow(source, &["MediaSource", "Media"]) {
                if let Some(id) = self.media(media, Some(name), bin)
                    && let Some(m) = self.b.p.item_mut(id).and_then(|i| i.as_media_mut())
                    && m.markers.is_empty()
                {
                    m.markers = markers;
                }
                return;
            }
        }
    }

    /// Markers on a master clip (shared by its video and audio clips).
    fn master_markers(&mut self, master: Node<'a, 'i>) -> Vec<Marker> {
        let clips: Vec<Node> = self.at(master, &["Clips"]).into_iter().flat_map(|c| c.children().filter(|n| n.has_tag_name("Clip"))).collect();
        for c in clips {
            let Some(clip) = self.resolve(Some(c)) else { continue };
            if let Some(markers) = self.follow(clip, &["Clip", "MarkerOwner", "Markers"]) {
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
        let Some(list) = self.at(owner, &["Markers"]) else { return out };
        for m in list.children().filter(|n| n.has_tag_name("Marker")) {
            let Some(obj) = self.follow(m, &["Second"]) else { continue };
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

    /// The media item for a `<Media>` object (deduplicated by its UID).
    fn media(&mut self, media: Node<'a, 'i>, name: Option<&str>, bin: Option<BinId>) -> Option<ItemId> {
        let key = media.attribute("ObjectUID").or_else(|| media.attribute("ObjectID"))?;
        if let Some(id) = self.b.find_media(key) {
            return Some(id);
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
            return Some(id);
        }
        let title = name.or_else(|| child_text(media, "Title")).unwrap_or_else(|| file_name(&path)).to_string();
        let mut spec = MediaSpec::default();
        if let Some(v) = self.follow(media, &["VideoStream"]) {
            let (w, h) = frame_rect(child_text(v, "FrameRect")).unwrap_or((1920, 1080));
            let rate = child_text(v, "FrameRate").and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_24);
            spec.video = Some((w, h, rate));
            spec.duration = time(child_text(v, "Duration")).filter(|d| *d > 0).map(Tick);
        }
        if let Some(a) = self.follow(media, &["AudioStream"]) {
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
        }
        let id = self.b.file_media(key, &title, &path, &spec, bin);
        self.b.register_media(&path_key, id);
        Some(id)
    }

    /// Where the media file is: Premiere's project-relative path when it is a plain relative path
    /// (that is what Premiere itself tries first after a project moves), else the absolute path.
    fn media_path(&self, media: Node<'a, 'i>) -> String {
        let absolute = child_text(media, "ActualMediaFilePath").or_else(|| child_text(media, "FilePath")).unwrap_or("").trim();
        let relative = child_text(media, "RelativePath").map(str::trim).filter(|r| !r.is_empty() && !r.contains(':') && !is_absolute(r));
        if let (Some(rel), Some(base)) = (relative, self.opts.base_dir.as_deref()) {
            return resolve_path(&rel.replace('\\', "/"), Some(base));
        }
        absolute.to_string()
    }

    // -----------------------------------------------------------------------------------------
    // Sequences
    // -----------------------------------------------------------------------------------------

    fn video_group(&self, seq: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
        let groups = self.at(seq, &["TrackGroups"])?;
        groups.children().filter(|n| n.has_tag_name("TrackGroup")).filter_map(|g| self.follow(g, &["Second"])).find(|g| g.has_tag_name(tag))
    }

    fn reserve_sequence(&mut self, seq: Node<'a, 'i>, name: &str, bin: Option<BinId>) {
        let Some(uid) = seq.attribute("ObjectUID") else { return };
        let vg = self.video_group(seq, "VideoTrackGroup");
        let (w, h) = vg.and_then(|g| frame_rect(child_text(g, "FrameRect"))).unwrap_or((1920, 1080));
        let rate = vg.and_then(|g| path_text(g, &["TrackGroup", "FrameRate"])).and_then(parse_i64).and_then(rate_of).unwrap_or(FrameRate::FPS_24);
        let mut settings = settings_for(rate, w, h, false);
        if let Some(t) =
            self.video_group(seq, "AudioTrackGroup").and_then(|g| path_text(g, &["TrackGroup", "FrameRate"])).and_then(parse_i64).filter(|t| *t > 0)
        {
            settings.sample_rate = (TICKS_PER_SECOND / t) as u32;
        }
        let name = child_text(seq, "Name").unwrap_or(name);
        let id = self.b.reserve_sequence(name, settings, bin);
        self.seq_items.insert(uid, id);
        self.seq_nodes.push((seq, id));
    }

    fn fill_sequence(&mut self, node: Node<'a, 'i>, id: ItemId) {
        let Some(settings) = self.b.p.sequence_mut(id).map(|s| s.settings.clone()) else { return };
        let mut seq: Sequence = empty_sequence(settings.clone());
        let frame = (settings.width as f64, settings.height as f64);
        for (kind, tag) in [(TrackKind::Video, "VideoTrackGroup"), (TrackKind::Audio, "AudioTrackGroup")] {
            let Some(group) = self.video_group(node, tag) else { continue };
            let Some(tracks) = self.at(group, &["TrackGroup", "Tracks"]) else { continue };
            let mut refs: Vec<(i64, Node)> =
                tracks.children().filter(|n| n.has_tag_name("Track")).map(|n| (n.attribute("Index").and_then(parse_i64).unwrap_or(i64::MAX), n)).collect();
            refs.sort_by_key(|(i, _)| *i);
            for (ti, (_, r)) in refs.into_iter().enumerate() {
                let Some(track) = self.resolve(Some(r)) else { continue };
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
        if let Some(uid) = node.attribute("ObjectUID")
            && let Some(m) = self.seq_markers.remove(uid)
        {
            seq.markers = m;
        }
        self.b.put_sequence(id, seq);
    }

    fn track(&mut self, seq: &mut Sequence, kind: TrackKind, ti: usize, track: Node<'a, 'i>, frame: (f64, f64)) {
        let refs = |path: &[&str]| -> Vec<Node<'a, 'i>> {
            self.at(track, path).into_iter().flat_map(|n| n.children().filter(|c| c.has_tag_name("TrackItem"))).filter_map(|r| self.resolve(Some(r))).collect()
        };
        let items = refs(&["ClipTrack", "ClipItems", "TrackItems"]);
        let transitions = refs(&["ClipTrack", "TransitionItems", "TrackItems"]);
        for item in items {
            if let Some(clip) = self.clip(item, kind, frame) {
                let placed = self.b.place(seq, kind, ti, clip);
                if placed != ti {
                    self.report.warn("overlapping clips on one track; the later one was moved to a new track");
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
        let ti_node = self.at(item, &["ClipTrackItem", "TrackItem"])?;
        let start = time(child_text(ti_node, "Start")).unwrap_or(0);
        let end = time(child_text(ti_node, "End"))?;
        if end <= start {
            return None;
        }
        let sub = self.follow(item, &["ClipTrackItem", "SubClip"])?;
        let clip = self.follow(sub, &["Clip"])?;
        let name = child_text(sub, "Name").unwrap_or("Clip").to_string();
        let source = self.follow(clip, &["Clip", "Source"]);
        let target = match source {
            Some(src) => {
                if let Some(seq) = self.follow(src, &["SequenceSource", "Sequence"]) {
                    let id = seq.attribute("ObjectUID").and_then(|u| self.seq_items.get(u).copied());
                    if let Some(id) = id {
                        self.nested.insert(id);
                    }
                    id
                } else if let Some(media) = self.follow(src, &["MediaSource", "Media"]) {
                    self.media(media, None, None)
                } else {
                    None
                }
            }
            None => None,
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
        if let Some(id) = item.attribute("ObjectID")
            && let Some(g) = self.links.get(id)
        {
            ti.link = Some(*g);
        }
        let src_frame = self.source_frame(target).unwrap_or(frame);
        let timing = KeyTiming { source_in, speed: ti.speed };
        if let Some(chain) = self.follow(item, &["ClipTrackItem", "ComponentOwner", "Components"]) {
            self.components(&mut ti, chain, timing, frame, src_frame);
        }
        Some(ti)
    }

    fn source_frame(&self, item: ItemId) -> Option<(f64, f64)> {
        match &self.b.p.item(item)?.kind {
            filmcraft_project::ItemKind::Media(m) => m.info.video.as_ref().map(|v| (v.width as f64, v.height as f64)),
            filmcraft_project::ItemKind::Sequence(s) => Some((s.settings.width as f64, s.settings.height as f64)),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Effects
    // -----------------------------------------------------------------------------------------

    fn components(&mut self, ti: &mut filmcraft_project::TrackItem, chain: Node<'a, 'i>, timing: KeyTiming, frame: (f64, f64), src: (f64, f64)) {
        let Some(list) = self.at(chain, &["ComponentChain", "Components"]) else { return };
        let comps: Vec<Node> = list.children().filter(|n| n.has_tag_name("Component")).filter_map(|r| self.resolve(Some(r))).collect();
        for comp in comps {
            let (body, display) = match child(comp, "AudioComponent") {
                Some(a) => (child(a, "Component"), child_text(comp, "FilterMatchName")),
                None => (child(comp, "Component"), path_text(comp, &["Component", "DisplayName"])),
            };
            let Some(body) = body else { continue };
            let match_name = child_text(comp, "MatchName").or_else(|| child_text(comp, "FilterMatchName")).unwrap_or("");
            let params: Vec<Node> = self
                .at(body, &["Params"])
                .into_iter()
                .flat_map(|p| p.children().filter(|n| n.has_tag_name("Param")))
                .filter_map(|r| self.resolve(Some(r)))
                .collect();
            let named = |n: &str| params.iter().copied().find(|p| child_text(*p, "Name").is_some_and(|x| x.trim().eq_ignore_ascii_case(n)));
            if match_name == "AE.ADBE Motion" {
                if let Some(p) = named("Position").and_then(|p| point_param(p, timing, |x, y| Vec2::new(x * frame.0, y * frame.1))) {
                    let centred = p.value.as_vec2().is_some_and(|v| (v.x - frame.0 / 2.0).abs() < 1e-6 && (v.y - frame.1 / 2.0).abs() < 1e-6);
                    if p.is_animated() || !centred {
                        set_param(ti, "motion", "position", p);
                    }
                }
                if let Some(p) = named("Anchor Point").and_then(|p| point_param(p, timing, |x, y| Vec2::new(x * src.0, y * src.1))) {
                    let centred = p.value.as_vec2().is_some_and(|v| (v.x - src.0 / 2.0).abs() < 1e-6 && (v.y - src.1 / 2.0).abs() < 1e-6);
                    if p.is_animated() || !centred {
                        set_param(ti, "motion", "anchor", p);
                    }
                }
                for (pname, id) in [("Scale", "scale"), ("Scale Width", "scale_width"), ("Rotation", "rotation"), ("Anti-flicker Filter", "anti_flicker")] {
                    if let Some(p) = named(pname).and_then(|p| float_param(p, timing, |v| v)) {
                        set_param(ti, "motion", id, p);
                    }
                }
                // Uniform Scale is the one boolean parameter of Motion; it is saved without a name.
                if let Some(v) = params.iter().find_map(|p| static_value(*p).filter(|v| *v == "true" || *v == "false")) {
                    set_param(ti, "motion", "uniform_scale", Param::new(ParamValue::Bool(v == "true")));
                }
            } else if match_name == "AE.ADBE Opacity" {
                if let Some(p) = named("Opacity").and_then(|p| float_param(p, timing, |v| v)) {
                    set_param(ti, "opacity", "opacity", p);
                }
            } else if match_name.starts_with("Internal Volume") {
                if let Some(p) = named("Level").and_then(|p| float_param(p, timing, |v| gain_to_db(v / UNITY_LEVEL)))
                    && (p.is_animated() || p.value.as_f64().is_some_and(|v| v.abs() > 1e-6))
                {
                    set_param(ti, "volume", "level", p);
                }
            } else if match_name.starts_with("Internal") || display.is_none() && match_name.is_empty() {
                // channel volume, panner, Essential Sound state: FilmCraft's own defaults apply
            } else {
                let label = display.filter(|d| !d.trim().is_empty()).unwrap_or(match_name);
                self.report.warn(format!("effect \"{label}\" is not supported and was skipped"));
            }
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
        let effect = transition_effect(name, kind == TrackKind::Audio, self.report);
        let (s, e) = (Tick(s), Tick(e));
        let Some(track) = seq.tracks_mut(kind).get_mut(ti) else { return };
        let from = outgoing.then(|| track.items.iter().filter(|i| i.end() >= s && i.end() <= e).max_by_key(|i| i.end()).map(|i| (i.id, i.end()))).flatten();
        let to = incoming.then(|| track.items.iter().filter(|i| i.start >= s && i.start <= e).min_by_key(|i| i.start).map(|i| (i.id, i.start))).flatten();
        let cut = from.map(|f| f.1).or(to.map(|t| t.1));
        let Some(cut) = cut else {
            self.report.warn(format!("transition \"{name}\" has no adjacent clip; skipped"));
            return;
        };
        let align = if cut == s {
            TransitionAlign::StartAtCut
        } else if cut == e {
            TransitionAlign::EndAtCut
        } else {
            TransitionAlign::CenterAtCut
        };
        let id = TransitionId(self.b.alloc());
        let (from, to): (Option<ClipId>, Option<ClipId>) = (from.map(|f| f.0), to.map(|t| t.0));
        track.transitions.push(Transition { id, effect, start: s, duration: e - s, from, to, align, reverse: false });
        track.sort();
    }
}

// ---------------------------------------------------------------------------------------------
// Parameter values
// ---------------------------------------------------------------------------------------------

/// How keyframe times (source-media ticks in Premiere) map to clip time.
#[derive(Clone, Copy)]
struct KeyTiming {
    source_in: i64,
    speed: f64,
}

impl KeyTiming {
    fn clip_time(self, source_time: i64) -> Tick {
        let d = source_time.saturating_sub(self.source_in).clamp(0, MAX_TIME);
        if self.speed == 1.0 { Tick(d) } else { Tick(((d as f64 / self.speed).round() as i64).clamp(0, MAX_TIME)) }
    }
}

fn parse_i64(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// `"l,t,r,b"` → (width, height).
fn frame_rect(s: Option<&str>) -> Option<(u32, u32)> {
    let v: Vec<i64> = s?.split(',').filter_map(parse_i64).collect();
    let (l, t, r, b) = (*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?);
    let (w, h) = (r.checked_sub(l)?, b.checked_sub(t)?);
    (w > 0 && h > 0 && w <= 65_536 && h <= 65_536).then_some((w as u32, h as u32))
}

/// Ticks per frame → frame rate (exact: 8 467 200 000 → 30, 10 594 584 000 → 24000/1001).
fn rate_of(ticks_per_frame: i64) -> Option<FrameRate> {
    (ticks_per_frame > 0 && TICKS_PER_SECOND / ticks_per_frame <= 1000).then(|| FrameRate::new(TICKS_PER_SECOND, ticks_per_frame))
}

/// The value field of a parameter's `StartKeyframe` (`time,value,…`).
fn static_value<'a>(p: Node<'a, '_>) -> Option<&'a str> {
    child_text(p, "StartKeyframe").and_then(|s| s.split(',').nth(1)).map(str::trim).or_else(|| child_text(p, "CurrentValue").map(str::trim))
}

/// `time,value,…;time,value,…;` → (time, value) pairs.
fn keyframes(p: Node<'_, '_>) -> Vec<(i64, String)> {
    let Some(k) = child_text(p, "Keyframes") else { return Vec::new() };
    k.split(';')
        .filter_map(|e| {
            let mut f = e.split(',');
            let t = parse_i64(f.next()?)?;
            let v = f.next()?.trim().to_string();
            (t != STATIC_TIME).then_some((t, v))
        })
        .take(100_000)
        .collect()
}

fn float_param(p: Node<'_, '_>, timing: KeyTiming, map: impl Fn(f64) -> f64) -> Option<Param> {
    let parse = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let keys = keyframes(p);
    let first = keys.first().and_then(|(_, v)| parse(v));
    let value = static_value(p).and_then(parse).or(first)?;
    let mut param = Param::new(ParamValue::Float(map(value)));
    for (t, v) in keys {
        if let Some(v) = parse(&v) {
            let mut kf = Keyframe::new(timing.clip_time(t), ParamValue::Float(map(v)));
            kf.interp = Interpolation::Linear;
            param.keyframes.push(kf);
        }
    }
    finish_param(param)
}

fn point_param(p: Node<'_, '_>, timing: KeyTiming, map: impl Fn(f64, f64) -> Vec2) -> Option<Param> {
    let parse = |s: &str| {
        let (x, y) = s.split_once(':')?;
        let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
        (x.is_finite() && y.is_finite()).then_some((x, y))
    };
    let keys = keyframes(p);
    let (x, y) = static_value(p).and_then(parse).or_else(|| keys.first().and_then(|(_, v)| parse(v)))?;
    let mut param = Param::new(ParamValue::Vec2(map(x, y)));
    for (t, v) in keys {
        if let Some((x, y)) = parse(&v) {
            param.keyframes.push(Keyframe::new(timing.clip_time(t), ParamValue::Vec2(map(x, y))));
        }
    }
    finish_param(param)
}

fn finish_param(mut p: Param) -> Option<Param> {
    p.keyframes.sort_by_key(|k| k.time);
    p.keyframes.dedup_by_key(|k| k.time);
    if let Some(k) = p.keyframes.first() {
        p.value = k.value.clone();
    }
    Some(p)
}

#[cfg(test)]
#[path = "prproj_tests.rs"]
mod tests;
