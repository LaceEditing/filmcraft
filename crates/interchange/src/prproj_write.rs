//! Write FilmCraft sequences back into the Premiere Pro project they were imported from.
//!
//! A project that came from Premiere is not regenerated: FilmCraft *patches* the original document
//! (new projects are seeded by [`crate::prproj_new`] and then written by this module). Everything
//! FilmCraft did not change keeps its exact bytes (the document is edited by byte range, not
//! re-serialised); what changed is rewritten in place:
//!
//! - each track's clip list, in timeline order. A clip that came from Premiere
//!   ([`TrackItem::origin`]) reuses its track item, so every effect, keyframe and setting
//!   FilmCraft does not model survives. A second clip from the same origin (a split half, a copy)
//!   becomes a copy of that track item and the objects only it owns (sub-clip, clip, component
//!   chain, components, parameters), with new ids. A clip without an origin copies a track item
//!   that uses the same media, then gets FilmCraft's settings;
//! - clip timing (`Start`, `End`, `InPoint`, `OutPoint`, `PlaybackSpeed`), frame holds and names;
//! - Motion, Opacity, clip volume and mapped effect parameters, only where FilmCraft's value
//!   differs from what was imported (compared in Premiere's own terms, so a trim does not count as
//!   a change of source-timed keyframes); keys FilmCraft did not change keep their original text;
//! - effects removed in FilmCraft are taken off the clip; added tracks copy an existing track;
//! - transitions follow their clips; links (`LinkContainer`) are rebuilt; cached preview renders
//!   of the edited sequences are dropped (they describe the old arrangement).
//!
//! Before returning, the result is checked: it must parse, every `ObjectRef` / `ObjectURef` and
//! every `BinaryHash` must resolve, ids must be unique, and importing it again must give back
//! exactly the clips FilmCraft has. A result that fails any check is an error, never a file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use filmcraft_project::{ItemId, ItemKind, Project, Sequence, TrackItem, TrackKind};
use roxmltree::Node;

use crate::common::scaled;
use crate::prproj::{FRAME_HOLD_AT, META_MEDIA, META_SEQUENCE, parse_i64, source_frame, time};
use crate::prproj_fx::{self, Ctx, Target};
use crate::prproj_graph::{self, Index};
use crate::xml::{child, child_text, escape, path_text};
use crate::{Error, Format, ImportOptions, Report, Result};

fn err(m: impl Into<String>) -> Error {
    Error::Other(format!("Premiere project export: {}", m.into()))
}

/// Write `sequences` of `project` (all sequences that came from this document when empty) into
/// `original`, the `.prproj` they were imported from. Returns the new document (gzip-compressed
/// when the original was) and a report of what could not be written.
pub fn export(project: &Project, original: &[u8], sequences: &[ItemId], dest_dir: Option<&str>) -> Result<(Vec<u8>, Report)> {
    let (xml, compressed) = prproj_graph::decompress(original)?;
    let text = std::str::from_utf8(&xml).map_err(|_| err("the original project is not UTF-8"))?;
    let doc = crate::xml::parse(text).map_err(|e| Error::parse(Format::Prproj, e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PremiereData" {
        return Err(Error::parse(Format::Prproj, "root element is not <PremiereData>"));
    }
    let mut w = Writer::new(text, root);
    w.displaced = displaced(text);
    w.dest_dir = dest_dir.map(str::to_string);
    w.prefixes = path_prefixes(project, &w.ix);
    let wanted: Vec<ItemId> = if sequences.is_empty() {
        project.items.values().filter(|i| i.metadata.contains_key(META_SEQUENCE) && i.as_sequence().is_some()).map(|i| i.id).collect()
    } else {
        sequences.to_vec()
    };
    if wanted.is_empty() {
        return Err(err("no sequence in this project came from a Premiere project"));
    }
    let mut written = Vec::new();
    for id in &wanted {
        let Some(item) = project.item(*id) else { continue };
        let Some(seq) = item.as_sequence() else { continue };
        let Some(uid) = item.metadata.get(META_SEQUENCE) else {
            w.report.warn(format!("sequence \"{}\" was not imported from this Premiere project and was not written", item.name));
            continue;
        };
        let Some(pseq) = w.ix.uids.get(uid.as_str()).copied().filter(|n| n.has_tag_name("Sequence")) else {
            w.report.warn(format!("sequence \"{}\" is no longer in the Premiere project and was not written", item.name));
            continue;
        };
        let exported = w.sequence(project, seq, pseq)?;
        w.owner_markers(pseq.attribute("ObjectUID").unwrap_or(""), true, &seq.markers, &format!("sequence \"{}\"", item.name));
        written.push((*id, uid.clone(), exported));
    }
    // source markers of media Premiere knows
    for it in project.items.values() {
        let (ItemKind::Media(m), Some(uids)) = (&it.kind, it.metadata.get(META_MEDIA)) else { continue };
        let Some(first) = uids.split(',').next() else { continue };
        w.owner_markers(first, false, &m.markers, &format!("\"{}\"", it.name));
    }
    let out = w.finish()?;
    let known = problems(text);
    if !known.is_empty() {
        w.report.warn(format!("the original project already has {} unresolved reference(s); they were left as they are", known.len()));
    }
    let mut out = out;
    out.report.entries.extend(std::mem::take(&mut w.report.entries));
    verify(&out, &written, &known)?;
    let bytes = if compressed { prproj_graph::compress(out.text.as_bytes()) } else { out.text.into_bytes() };
    Ok((bytes, out.report))
}

/// The clips written to one Premiere track, as they must read back.
type WrittenTrack = (TrackKind, usize, Vec<ExportedClip>);

// ---------------------------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------------------------

/// One top-level object as it will be written: the original in place, or a copy with a new id.
struct Instance<'a, 'i> {
    src: Node<'a, 'i>,
    /// New `ObjectID` / `ObjectUID` for a copy; `None` edits the original in place.
    new_id: Option<String>,
    /// Edits by byte range of the original document (inside `src`).
    edits: Vec<(Range<usize>, String)>,
}

struct Writer<'a, 'i> {
    text: &'a str,
    root: Node<'a, 'i>,
    ix: Index<'a, 'i>,
    report: Report,
    instances: Vec<Instance<'a, 'i>>,
    /// Original object id → its in-place instance.
    in_place: HashMap<&'a str, usize>,
    /// New objects written from scratch (links).
    created: Vec<String>,
    next_id: u64,
    uuid_seed: u64,
    /// Every object an `ObjectRef` points at → the objects referring to it.
    referrers: HashMap<&'a str, Vec<&'a str>>,
    /// Clips the importer put on another track than their Premiere one (they overlap a clip
    /// there): (sequence UID, is video, track item id) → the FilmCraft track index it used.
    displaced: HashMap<(String, bool, String), usize>,
    /// Folder the new project is written to (relative media paths start there).
    dest_dir: Option<String>,
    /// How FilmCraft's media paths map onto the paths the Premiere project uses
    /// (`/mnt/LocalDisk` → `L:`), learned from media both know.
    prefixes: Vec<(String, String)>,
    /// Media FilmCraft added: item → the copied asset (None: could not be added).
    assets: HashMap<ItemId, Option<Asset>>,
    /// Project items added to the root bin.
    added_items: Vec<String>,
    /// Written track item id → its instance and the object it was written from.
    clip_objects: HashMap<String, (usize, Node<'a, 'i>)>,
    /// Binary payloads by hash (read on first use).
    payloads: Option<HashMap<&'a str, &'a str>>,
}

/// A media asset copied into the project for media FilmCraft added.
#[derive(Clone)]
struct Asset {
    master_uid: String,
    name: String,
    /// Per kind (video, audio): the master clip's source object id, its markers object id and
    /// its channel sources.
    video: Option<AssetClip>,
    audio: Option<AssetClip>,
}

#[derive(Clone)]
struct AssetClip {
    source: String,
    markers: Option<String>,
    secondary: Option<String>,
}

impl<'a, 'i> Writer<'a, 'i> {
    fn new(text: &'a str, root: Node<'a, 'i>) -> Self {
        let ix = Index::new(root);
        let next_id = ix.ids.keys().filter_map(|k| k.parse::<u64>().ok()).max().unwrap_or(0) + 1;
        let mut referrers: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
        for obj in root.children().filter(Node::is_element) {
            let Some(me) = obj.attribute("ObjectID").or_else(|| obj.attribute("ObjectUID")) else { continue };
            for r in public_refs(obj) {
                if let Some(target) = r.attribute("ObjectRef") {
                    referrers.entry(target).or_default().push(me);
                }
            }
        }
        let uuid_seed = prproj_graph::crc32(text.as_bytes()) as u64 ^ (text.len() as u64).rotate_left(32);
        Self {
            text,
            root,
            ix,
            report: Report::default(),
            instances: Vec::new(),
            in_place: HashMap::new(),
            created: Vec::new(),
            next_id,
            uuid_seed,
            referrers,
            displaced: HashMap::new(),
            dest_dir: None,
            prefixes: Vec::new(),
            assets: HashMap::new(),
            added_items: Vec::new(),
            clip_objects: HashMap::new(),
            payloads: None,
        }
    }

    /// The bytes of a binary value, stored in the element or in another one with its hash.
    fn binary(&mut self, el: Node<'a, 'i>) -> Option<Vec<u8>> {
        let text = match el.text().map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => {
                let hash = el.attribute("BinaryHash")?;
                let root = self.root;
                let map = self.payloads.get_or_insert_with(|| {
                    let mut m = HashMap::new();
                    for n in root.descendants().filter(Node::is_element) {
                        if let (Some(h), Some(t)) = (n.attribute("BinaryHash"), n.text().map(str::trim).filter(|t| !t.is_empty())) {
                            m.entry(h).or_insert(t);
                        }
                    }
                    m
                });
                map.get(hash).copied()?
            }
        };
        crate::prproj_text::base64(text)
    }

    /// Whether FilmCraft's text layer differs from the Premiere title it was imported from in what
    /// is not written back (the text and its formatting live in Premiere's `Source Text` value).
    fn title_edited(&mut self, comp: &prproj_fx::Component<'a, 'i>, e: &filmcraft_project::EffectInstance) -> bool {
        let source = comp.params.iter().copied().find(|p| child_text(*p, "Name").is_some_and(|n| n.trim() == "Source Text"));
        let Some(t) = source.and_then(|p| child(p, "StartKeyframeValue")).and_then(|v| self.binary(v)).and_then(|b| crate::prproj_text::decode(&b)) else {
            return false;
        };
        let text = |id: &str| match e.params.get(id).map(|p| &p.value) {
            Some(filmcraft_project::ParamValue::Text(s)) => Some(s.clone()),
            _ => None,
        };
        let num = |id: &str| e.params.get(id).and_then(|p| p.value.as_f64());
        let animated = ["text", "size", "fill_color"].iter().any(|id| e.params.get(*id).is_some_and(|p| !p.keyframes.is_empty()));
        let font = t.font.as_deref().map(crate::prproj_text::font_family_style);
        animated
            || text("text").as_deref() != Some(t.text.as_str())
            || num("size").is_some_and(|v| (v - f64::from(t.size)).abs() > 1e-3)
            || font.is_some_and(|(f, st)| text("font").as_deref() != Some(f.as_str()) || text("font_style").as_deref() != Some(st.as_str()))
    }

    /// Edits so far, counting a copied object as one.
    fn edit_count(&self) -> usize {
        self.instances.iter().map(|i| i.edits.len() + usize::from(i.new_id.is_some())).sum::<usize>() + self.created.len()
    }

    fn alloc_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        id.to_string()
    }

    /// A fresh UUID (deterministic per document, version-4 layout).
    fn uuid(&mut self) -> String {
        // splitmix64 over a per-document seed that advances on every call
        self.uuid_seed = self.uuid_seed.wrapping_add(0xA076_1D64_78BD_642F);
        let mut z = self.uuid_seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let (a, b) = (next(), next());
        let a = (a & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
        let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xFFFF, a & 0xFFFF, b >> 48, b & 0xFFFF_FFFF_FFFF)
    }

    /// The in-place instance of an object (created on first use).
    fn here(&mut self, obj: Node<'a, 'i>) -> usize {
        let key = obj.attribute("ObjectID").or_else(|| obj.attribute("ObjectUID")).unwrap_or("");
        if let Some(i) = self.in_place.get(key) {
            return *i;
        }
        self.instances.push(Instance { src: obj, new_id: None, edits: Vec::new() });
        let i = self.instances.len() - 1;
        if !key.is_empty() {
            self.in_place.insert(key, i);
        }
        i
    }

    fn edit(&mut self, inst: usize, range: Range<usize>, text: String) {
        if let Some(i) = self.instances.get_mut(inst) {
            i.edits.push((range, text));
        }
    }

    /// Set an element's text.
    fn set_text(&mut self, inst: usize, el: Node<'a, 'i>, value: &str) {
        let value = escape(value);
        match el.first_child().filter(|c| c.is_text()) {
            Some(t) if el.children().count() == 1 => {
                if self.text[t.range()] != value {
                    self.edit(inst, t.range(), value);
                }
            }
            _ => {
                let r = el.range();
                let src = &self.text[r.clone()];
                let name = el.tag_name().name();
                let open = match src.find('>') {
                    Some(p) if src[..p].ends_with('/') => format!("{}>", &src[..p - 1]),
                    Some(p) => src[..=p].to_string(),
                    None => return,
                };
                self.edit(inst, r, format!("{open}{value}</{name}>"));
            }
        }
    }

    /// Set the text of `parent`'s child `name`, adding the child when it is missing.
    fn set_child(&mut self, inst: usize, parent: Node<'a, 'i>, name: &str, value: &str) {
        match child(parent, name) {
            Some(c) => self.set_text(inst, c, value),
            None => self.insert(inst, parent, &format!("<{name}>{}</{name}>", escape(value))),
        }
    }

    /// Insert XML just before `parent`'s closing tag.
    fn insert(&mut self, inst: usize, parent: Node<'a, 'i>, xml: &str) {
        let r = parent.range();
        let src = &self.text[r.clone()];
        let name = parent.tag_name().name();
        if let Some(open) = src.strip_suffix("/>") {
            self.edit(inst, r, format!("{open}>{xml}</{name}>"));
        } else {
            let close = format!("</{name}>");
            let at = r.end.saturating_sub(close.len());
            self.edit(inst, at..at, xml.to_string());
        }
    }

    fn remove(&mut self, inst: usize, el: Node<'a, 'i>) {
        self.edit(inst, el.range(), String::new());
    }

    /// Replace a list element's children (keeping its own tag and attributes).
    fn set_list(&mut self, inst: usize, list: Node<'a, 'i>, children: &str) {
        let r = list.range();
        let src = &self.text[r.clone()];
        let name = list.tag_name().name();
        let Some(p) = src.find('>') else { return };
        let open = if src[..p].ends_with('/') { format!("{}>", &src[..p - 1]) } else { src[..=p].to_string() };
        self.edit(inst, r, format!("{open}{children}</{name}>"));
    }

    // -----------------------------------------------------------------------------------------
    // Copies of a track item and the objects only it owns
    // -----------------------------------------------------------------------------------------

    /// Copy `item` with every object that only it (transitively) refers to. Returns the instance
    /// of each copied object by original id.
    fn clone_owned(&mut self, root_obj: Node<'a, 'i>, skip: &[&str]) -> HashMap<&'a str, usize> {
        let Some(root_key) = root_obj.attribute("ObjectID").or_else(|| root_obj.attribute("ObjectUID")) else { return HashMap::new() };
        let lookup = |ix: &Index<'a, 'i>, k: &str| ix.ids.get(k).copied().or_else(|| ix.uids.get(k).copied());
        let mut owned: Vec<&'a str> = vec![root_key];
        let mut set: HashSet<&'a str> = owned.iter().copied().collect();
        let mut i = 0;
        while let Some(id) = owned.get(i).copied() {
            i += 1;
            let Some(obj) = lookup(&self.ix, id) else { continue };
            for r in public_refs_skipping(obj, skip) {
                let Some(t) = r.attribute("ObjectRef") else { continue };
                if set.contains(t) {
                    continue;
                }
                let Some(target) = self.ix.ids.get(t).copied() else { continue };
                // a clip's own objects (as writers Premiere accepts copy them), or anything only
                // this graph refers to
                let by_kind = OWNED_KINDS.contains(&target.tag_name().name()) || target.tag_name().name().ends_with("ComponentParam");
                let exclusive = self.referrers.get(t).is_some_and(|rs| rs.iter().all(|x| set.contains(x)));
                if by_kind || exclusive {
                    set.insert(t);
                    owned.push(t);
                }
            }
        }
        self.materialize(owned)
    }

    /// Every object reachable from `root` through `ObjectRef` and `ObjectURef` (a media asset's
    /// whole graph: project item, master clip, clips, sources, media, streams, channels).
    fn clone_graph(&mut self, root: Node<'a, 'i>) -> HashMap<&'a str, usize> {
        let Some(root_key) = root.attribute("ObjectID").or_else(|| root.attribute("ObjectUID")) else { return HashMap::new() };
        let mut all: Vec<&'a str> = vec![root_key];
        let mut set: HashSet<&'a str> = all.iter().copied().collect();
        let mut i = 0;
        while let Some(key) = all.get(i).copied() {
            i += 1;
            let Some(obj) = self.ix.ids.get(key).copied().or_else(|| self.ix.uids.get(key).copied()) else { continue };
            for r in public_refs(obj) {
                let target = r
                    .attribute("ObjectRef")
                    .filter(|t| self.ix.ids.contains_key(t))
                    .or_else(|| r.attribute("ObjectURef").filter(|t| self.ix.uids.contains_key(t)));
                if let Some(t) = target
                    && set.insert(t)
                {
                    all.push(t);
                }
            }
            if all.len() > 10_000 {
                break; // not a media asset
            }
        }
        self.materialize(all)
    }

    /// Copy objects with new ids (`ObjectID` → next free id, `ObjectUID` → new UUID), pointing
    /// their references at each other's copies. Returns the instance of each copy by original key.
    fn materialize(&mut self, keys: Vec<&'a str>) -> HashMap<&'a str, usize> {
        let mut map: HashMap<&'a str, String> = HashMap::new();
        for k in &keys {
            let n = if self.ix.ids.contains_key(k) { self.alloc_id() } else { self.uuid() };
            map.insert(k, n);
        }
        let mut out = HashMap::new();
        for k in keys {
            let Some(obj) = self.ix.ids.get(k).copied().or_else(|| self.ix.uids.get(k).copied()) else { continue };
            let mut edits = Vec::new();
            for r in public_refs(obj) {
                if let Some(a) = r.attributes().find(|a| a.name() == "ObjectRef" || a.name() == "ObjectURef")
                    && let Some(n) = map.get(a.value())
                {
                    edits.push((a.range_value(), n.clone()));
                }
            }
            // identities a copy must not share
            for el in obj.descendants().filter(Node::is_element) {
                let fresh = match el.tag_name().name() {
                    "ClipID" | "FileKey" => true,
                    "ID" => el.text().is_some_and(|t| t.trim().len() == 36 && t.contains('-')),
                    _ => false,
                };
                if fresh && let Some(t) = el.first_child().filter(|c| c.is_text()) {
                    let u = self.uuid();
                    edits.push((t.range(), u));
                }
            }
            self.instances.push(Instance { src: obj, new_id: map.get(k).cloned(), edits });
            out.insert(k, self.instances.len() - 1);
        }
        out
    }

    /// The instance of an object belonging to a clip: its copy when the clip is a copy, else the
    /// original in place.
    fn inst_of(&mut self, obj: Node<'a, 'i>, copies: &HashMap<&'a str, usize>) -> usize {
        match obj.attribute("ObjectID").or_else(|| obj.attribute("ObjectUID")).and_then(|id| copies.get(id)) {
            Some(i) => *i,
            None => self.here(obj),
        }
    }

    fn id_of(&self, inst: usize) -> String {
        self.instances
            .get(inst)
            .map(|i| i.new_id.clone().unwrap_or_else(|| i.src.attribute("ObjectID").or_else(|| i.src.attribute("ObjectUID")).unwrap_or("").to_string()))
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------------------------
    // Sequences
    // -----------------------------------------------------------------------------------------

    fn sequence(&mut self, project: &Project, seq: &Sequence, pseq: Node<'a, 'i>) -> Result<Vec<WrittenTrack>> {
        let edits_before = self.edit_count();
        let frame = (seq.settings.width as f64, seq.settings.height as f64);
        let mut used: HashSet<&'a str> = HashSet::new();
        // FilmCraft clip → written Premiere track item id (for links and transitions)
        let mut written_ids: HashMap<filmcraft_project::ClipId, String> = HashMap::new();
        let mut exported = Vec::new();
        let mut groups = Vec::new();
        for (kind, tag) in [(TrackKind::Video, "VideoTrackGroup"), (TrackKind::Audio, "AudioTrackGroup")] {
            let ftracks = seq.tracks(kind);
            let Some(group) = prproj_graph::track_group(&self.ix, pseq, tag) else {
                if ftracks.iter().any(|t| !t.items.is_empty()) {
                    self.report.warn(format!("the Premiere sequence has no {} tracks; those clips were not written", kind_name(kind)));
                }
                continue;
            };
            groups.push(group);
            let originals = prproj_graph::tracks(&self.ix, group);
            let mut ptracks: Vec<(usize, Node<'a, 'i>)> = originals.iter().map(|t| (self.here(*t), *t)).collect();
            // clips the importer only moved to an extra track (they overlap another clip on their
            // Premiere track) go back to that track while their timing is unchanged
            let mut home: HashMap<&str, (usize, i64, i64)> = HashMap::new();
            for (ti, t) in originals.iter().enumerate() {
                for c in self.ix.list(*t, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem") {
                    let span = self
                        .ix
                        .at(c, &["ClipTrackItem", "TrackItem"])
                        .map(|x| (time(child_text(x, "Start")).unwrap_or(0), time(child_text(x, "End")).unwrap_or(0)));
                    if let (Some(id), Some((s0, e0))) = (c.attribute("ObjectID"), span) {
                        home.insert(id, (ti, s0, e0));
                    }
                }
            }
            let seq_uid = pseq.attribute("ObjectUID").unwrap_or("").to_string();
            let mut effective: Vec<filmcraft_project::Track> = ftracks.clone();
            for ti in 0..effective.len() {
                let items = effective.get_mut(ti).map(|t| std::mem::take(&mut t.items)).unwrap_or_default();
                let mut stay = Vec::with_capacity(items.len());
                for c in items {
                    let home_track = c.origin.as_deref().and_then(|o| {
                        let placed = self.displaced.get(&(seq_uid.clone(), kind == TrackKind::Video, o.to_string())).copied()?;
                        let (h, s0, e0) = home.get(o).copied()?;
                        (placed == ti && h != ti && c.start.0 == s0 && c.end().0 == e0).then_some(h)
                    });
                    match home_track {
                        Some(h) => {
                            if let Some(t) = effective.get_mut(h) {
                                t.items.push(c);
                            }
                        }
                        None => stay.push(c),
                    }
                }
                if let Some(t) = effective.get_mut(ti) {
                    t.items.extend(stay);
                }
            }
            while effective.len() > originals.len() && effective.last().is_some_and(|t| t.items.is_empty() && t.transitions.is_empty()) {
                effective.pop();
            }
            let ftracks = &effective;
            if ftracks.len() > ptracks.len() {
                self.add_tracks(group, &mut ptracks, ftracks.len())?;
            }
            let empty = filmcraft_project::Track::new(filmcraft_project::TrackId(0), kind, String::new());
            for (ti, (tinst, pt)) in ptracks.iter().copied().enumerate() {
                let in_filmcraft = ti < ftracks.len();
                let ft = ftracks.get(ti).unwrap_or(&empty);
                if in_filmcraft
                    && !ft.name.is_empty()
                    && let Some(t) = self.ix.at(pt, &["ClipTrack", "Track"])
                    && child_text(t, "Name").unwrap_or("") != ft.name
                    && !(child_text(t, "Name").is_none() && ft.name == default_track_name(kind, ti))
                {
                    self.set_child(tinst, t, "Name", &ft.name);
                }
                let mut items: Vec<&TrackItem> = ft.items.iter().collect();
                items.sort_by_key(|c| c.start);
                // (start, position in Premiere's list) → id: FilmCraft's clips, and Premiere's clips
                // FilmCraft never had; unchanged clips keep Premiere's order
                let original_order: HashMap<&str, usize> = self
                    .ix
                    .at(pt, &["ClipTrack", "ClipItems", "TrackItems"])
                    .into_iter()
                    .flat_map(|n| n.children().filter(|c| c.has_tag_name("TrackItem")))
                    .filter_map(|r| r.attribute("ObjectRef"))
                    .enumerate()
                    .map(|(i, id)| (id, i))
                    .collect();
                let mut placed: Vec<((i64, usize), String)> = Vec::new();
                let mut back = Vec::new();
                for (n, c) in items.into_iter().enumerate() {
                    let Some(id) = self.clip(project, c, kind, frame, &mut used)? else { continue };
                    written_ids.insert(c.id, id.clone());
                    let order = original_order.get(id.as_str()).copied().unwrap_or(original_order.len() + n);
                    placed.push(((c.start.0, order), id));
                    back.push(ExportedClip::of(c));
                }
                if !inst_is_copy(&self.instances, tinst) {
                    let mut kept_foreign = 0;
                    for (n, item) in self.ix.list(pt, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem").into_iter().enumerate() {
                        if crate::prproj::importable(&self.ix, item) {
                            continue;
                        }
                        let Some(id) = item.attribute("ObjectID") else { continue };
                        let start = self.ix.at(item, &["ClipTrackItem", "TrackItem"]).and_then(|t| time(child_text(t, "Start"))).unwrap_or(0);
                        placed.push(((start, n), id.to_string()));
                        kept_foreign += 1;
                    }
                    if kept_foreign > 0 {
                        self.report.info(format!("{kept_foreign} Premiere clip(s) FilmCraft does not show were kept on {} track {}", kind_name(kind), ti + 1));
                    }
                }
                placed.sort_by_key(|(k, _)| *k);
                let ids: Vec<String> = placed.into_iter().map(|(_, id)| id).collect();
                let before: Vec<&str> = self
                    .ix
                    .at(pt, &["ClipTrack", "ClipItems", "TrackItems"])
                    .into_iter()
                    .flat_map(|n| n.children().filter(|c| c.has_tag_name("TrackItem")))
                    .filter_map(|r| r.attribute("ObjectRef"))
                    .collect();
                if ids.iter().map(String::as_str).ne(before.iter().copied()) {
                    let refs: String = ids.iter().enumerate().map(|(i, id)| format!("<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />")).collect();
                    if let Some(list) = self.ix.at(pt, &["ClipTrack", "ClipItems", "TrackItems"]) {
                        self.set_list(tinst, list, &refs);
                    } else if let Some(ci) = self.ix.at(pt, &["ClipTrack", "ClipItems"]) {
                        self.insert(tinst, ci, &format!("<TrackItems Version=\"1\">{refs}</TrackItems>"));
                    } else if !ids.is_empty() {
                        return Err(err("a Premiere track has no clip list"));
                    }
                }
                if !inst_is_copy(&self.instances, tinst) {
                    self.transitions(ft, pt, tinst, &written_ids, in_filmcraft);
                } else if !ft.transitions.is_empty() {
                    self.report.warn(format!("transitions on the new {} track {} are not written to Premiere projects yet", kind_name(kind), ti + 1));
                }
                exported.push((kind, ti, back));
            }
        }
        self.links(seq, pseq, &written_ids);
        self.sequence_duration(seq, pseq);
        // anything changed: cached renders describe the old arrangement
        let edits_after = self.edit_count();
        if edits_after != edits_before {
            for group in groups {
                if let Some(previews) = child(group, "PreviewItems").filter(|p| p.children().any(|c| c.is_element())) {
                    let gi = self.here(group);
                    self.set_list(gi, previews, "");
                }
            }
        }
        Ok(exported)
    }

    /// Copy the last Premiere track (with the mixer objects it owns, not its clips) until there
    /// are `want` tracks.
    fn add_tracks(&mut self, group: Node<'a, 'i>, ptracks: &mut Vec<(usize, Node<'a, 'i>)>, want: usize) -> Result<()> {
        let Some((_, template)) = ptracks.last().copied() else {
            self.report.warn("the Premiere sequence has no track to copy for FilmCraft's extra tracks");
            return Ok(());
        };
        let gi = self.here(group);
        let mut next_track = path_text(group, &["TrackGroup", "NextTrackID"]).and_then(parse_i64).unwrap_or(ptracks.len() as i64 + 1);
        let mut refs = String::new();
        if let Some(list) = self.ix.at(group, &["TrackGroup", "Tracks"]) {
            for r in list.children().filter(|n| n.has_tag_name("Track")) {
                refs.push_str(&self.text[r.range()]);
            }
        }
        for i in ptracks.len()..want {
            let copies = self.clone_owned(template, &["ClipItems", "TransitionItems"]);
            let Some(inst) = template.attribute("ObjectUID").and_then(|u| copies.get(u)).copied() else {
                return Err(err("a Premiere track has no ObjectUID"));
            };
            let uid = self.id_of(inst);
            if let Some(t) = self.ix.at(template, &["ClipTrack", "Track"]) {
                if let Some(id) = child(t, "ID") {
                    self.set_text(inst, id, &next_track.to_string());
                }
                if let Some(ix) = child(t, "Index") {
                    self.set_text(inst, ix, &i.to_string());
                }
                if let Some(n) = child(t, "Name") {
                    self.remove(inst, n);
                }
            }
            for p in [&["ClipTrack", "ClipItems", "Index"][..], &["ClipTrack", "TransitionItems", "Index"][..]] {
                if let Some(n) = self.ix.at(template, p) {
                    self.set_text(inst, n, &i.to_string());
                }
            }
            // its clips are written by the caller; transitions belong to the template's clips
            if let Some(list) = self.ix.at(template, &["ClipTrack", "TransitionItems", "TrackItems"]) {
                self.set_list(inst, list, "");
            }
            refs.push_str(&format!("<Track Index=\"{i}\" ObjectURef=\"{uid}\" />"));
            next_track += 1;
            ptracks.push((inst, template));
        }
        if let Some(list) = self.ix.at(group, &["TrackGroup", "Tracks"]) {
            self.set_list(gi, list, &refs);
        }
        if let Some(n) = self.ix.at(group, &["TrackGroup", "NextTrackID"]) {
            self.set_text(gi, n, &next_track.to_string());
        }
        Ok(())
    }

    /// Write one clip; returns its instance and Premiere id, or `None` when it cannot be written.
    fn clip(&mut self, project: &Project, c: &TrackItem, kind: TrackKind, frame: (f64, f64), used: &mut HashSet<&'a str>) -> Result<Option<String>> {
        let want_tag = if kind == TrackKind::Video { "VideoClipTrackItem" } else { "AudioClipTrackItem" };
        let source_uid = self.source_uid(project, c.item);
        // the Premiere object to start from, and whether it is a copy
        let plays = |w: &Self, n: Node<'a, 'i>| match (&source_uid, w.item_source_uid(n)) {
            (Some(list), Some(uid)) => list.split(',').any(|u| u == uid),
            (None, None) => true,
            _ => false,
        };
        let origin = c.origin.as_deref().and_then(|o| self.ix.ids.get(o).copied()).filter(|n| n.has_tag_name(want_tag)).filter(|n| plays(self, *n));
        let (obj, fresh) = match origin {
            Some(o) => {
                let id = o.attribute("ObjectID").unwrap_or("");
                (o, !used.insert(id))
            }
            None => match self.template(want_tag, source_uid.as_deref()) {
                Some(t) => (t, true),
                None => return self.clip_of_new_media(project, c, kind, frame),
            },
        };
        let copies = if fresh { self.clone_owned(obj, &[]) } else { HashMap::new() };
        let inst = self.inst_of(obj, &copies);
        let id = self.id_of(inst);
        self.patch_clip(project, c, kind, frame, obj, inst, &copies, origin.is_none());
        self.clip_objects.insert(id.clone(), (inst, obj));
        Ok(Some(id))
    }

    /// The Premiere UID of what a FilmCraft item is (media or sequence).
    fn source_uid(&self, project: &Project, item: ItemId) -> Option<String> {
        let it = project.item(item)?;
        let key = match it.kind {
            ItemKind::Sequence(_) => META_SEQUENCE,
            _ => META_MEDIA,
        };
        it.metadata.get(key).cloned()
    }

    /// The UID of the media or sequence a Premiere track item plays.
    fn item_source_uid(&self, item: Node<'a, 'i>) -> Option<String> {
        let sub = self.ix.follow(item, &["ClipTrackItem", "SubClip"])?;
        let clip = self.ix.follow(sub, &["Clip"])?;
        prproj_graph::clip_media(&self.ix, clip)
            .or_else(|| prproj_graph::clip_sequence(&self.ix, clip))
            .and_then(|n| n.attribute("ObjectUID"))
            .map(str::to_string)
    }

    /// A track item of this kind that plays `source` (to copy for a clip FilmCraft added).
    fn template(&self, tag: &str, source: Option<&str>) -> Option<Node<'a, 'i>> {
        let source = source?;
        self.root.children().filter(|n| n.has_tag_name(tag)).find(|n| self.item_source_uid(*n).is_some_and(|u| source.split(',').any(|s| s == u)))
    }

    #[allow(clippy::too_many_arguments)]
    fn patch_clip(
        &mut self,
        project: &Project,
        c: &TrackItem,
        kind: TrackKind,
        frame: (f64, f64),
        obj: Node<'a, 'i>,
        inst: usize,
        copies: &HashMap<&'a str, usize>,
        from_template: bool,
    ) {
        // timeline position
        if let Some(ti) = self.ix.at(obj, &["ClipTrackItem", "TrackItem"]) {
            let start = c.start.0.to_string();
            let end = c.end().0.to_string();
            if child_text(ti, "Start").unwrap_or("0").trim() != start {
                self.set_child(inst, ti, "Start", &start);
            }
            if child_text(ti, "End").map(str::trim) != Some(end.as_str()) {
                self.set_child(inst, ti, "End", &end);
            }
        }
        // transitions belong to the original clip only; `transitions` re-adds the kept ones
        if inst_is_copy(&self.instances, inst)
            && let Some(cti) = child(obj, "ClipTrackItem")
        {
            for t in cti.children().filter(|n| n.has_tag_name("HeadTransition") || n.has_tag_name("TailTransition")) {
                self.remove(inst, t);
            }
        }
        let Some(sub) = self.ix.follow(obj, &["ClipTrackItem", "SubClip"]) else { return };
        let sub_inst = self.inst_of(sub, copies);
        if child_text(sub, "Name") != Some(c.name.as_str()) && !c.name.is_empty() {
            self.set_child(sub_inst, sub, "Name", &c.name);
        }
        let Some(clip) = self.ix.follow(sub, &["Clip"]) else { return };
        let clip_inst = self.inst_of(clip, copies);
        if let Some(body) = child(clip, "Clip") {
            let old_in = time(child_text(body, "InPoint")).unwrap_or(0);
            let old_speed = child_text(body, "PlaybackSpeed").and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s != 0.0).unwrap_or(1.0);
            let new_speed = if c.reverse { -c.speed } else { c.speed };
            let old_dur = self
                .ix
                .at(obj, &["ClipTrackItem", "TrackItem"])
                .map(|t| time(child_text(t, "End")).unwrap_or(0) - time(child_text(t, "Start")).unwrap_or(0))
                .unwrap_or(0);
            let timing_changed = old_in != c.source_in.0 || (old_speed - new_speed).abs() > 1e-12 || old_dur != c.duration.0;
            if old_in != c.source_in.0 {
                self.set_child(clip_inst, body, "InPoint", &c.source_in.0.to_string());
            }
            if timing_changed {
                let out = c.source_in.0.saturating_add(scaled(c.duration, c.speed).0);
                self.set_child(clip_inst, body, "OutPoint", &out.to_string());
            }
            if (old_speed - new_speed).abs() > 1e-12 {
                self.set_child(clip_inst, body, "PlaybackSpeed", &crate::xml::num(new_speed));
            }
        }
        if kind == TrackKind::Video {
            match c.frame_hold {
                Some(at) => {
                    if child_text(clip, "FrameHold") != Some(FRAME_HOLD_AT) {
                        self.set_child(clip_inst, clip, "FrameHold", FRAME_HOLD_AT);
                    }
                    if time(child_text(clip, "FrameHoldStart")) != Some(at.0) {
                        self.set_child(clip_inst, clip, "FrameHoldStart", &at.0.to_string());
                    }
                }
                None => {
                    for t in ["FrameHold", "FrameHoldStart"] {
                        if let Some(n) = child(clip, t) {
                            self.remove(clip_inst, n);
                        }
                    }
                }
            }
        }
        // effects
        let Some(chain) = self.ix.follow(obj, &["ClipTrackItem", "ComponentOwner", "Components"]) else { return };
        let chain_inst = self.inst_of(chain, copies);
        let ctx = Ctx { source_in: c.source_in.0, speed: c.speed, frame, src: source_frame(project, c.item).unwrap_or(frame) };
        let comps = prproj_fx::chain(&self.ix, chain);
        let targets = prproj_fx::targets(&comps);
        let mut paired: HashMap<&'static str, usize> = HashMap::new();
        let mut keep_refs: Vec<bool> = Vec::with_capacity(comps.len());
        for (comp, target) in comps.iter().zip(targets.iter().copied()) {
            let instance = match target {
                Target::Motion => c.effect("motion"),
                Target::Opacity => c.effect("opacity"),
                Target::Volume => c.effect("volume"),
                Target::Effect(def) => {
                    let n = paired.entry(def.id).or_insert(0);
                    let found = c.effects.iter().filter(|e| e.effect == def.id).nth(*n);
                    *n += 1;
                    if found.is_none() {
                        // removed in FilmCraft
                        keep_refs.push(false);
                        continue;
                    }
                    if def.id == filmcraft_project::graphic::TEXT_LAYER
                        && let Some(e) = found
                        && self.title_edited(comp, e)
                    {
                        self.report.warn(format!(
                            "title \"{}\": changes to its text or formatting are not written to Premiere projects yet; Premiere keeps the original (position, size and opacity are written)",
                            c.name
                        ));
                    }
                    found
                }
                // FilmCraft never saw it: keep it on clips from Premiere, drop it from copies of
                // another clip's settings
                Target::Ignore => {
                    keep_refs.push(true);
                    continue;
                }
                Target::Unsupported => {
                    keep_refs.push(!from_template);
                    continue;
                }
            };
            keep_refs.push(true);
            let def = match target {
                Target::Effect(d) => Some(d),
                _ => None,
            };
            let (maps, _) = prproj_fx::mappings(comp, target);
            for m in maps {
                let fc_param = instance.and_then(|e| e.params.get(m.param).cloned()).or_else(|| {
                    def.or_else(|| filmcraft_project::find_effect(m.effect))
                        .and_then(|d| d.params.iter().find(|p| p.id == m.param))
                        .map(|p| filmcraft_project::Param::new(p.default.clone()))
                });
                let Some(fc_param) = fc_param else { continue };
                let Some(fc) = prproj_fx::from_param(&fc_param, m.conv, &ctx) else { continue };
                let pr = prproj_fx::decode(m.node, m.conv);
                let unchanged = match &pr {
                    Some(pr) => prproj_fx::same(pr, &fc, &ctx),
                    // Premiere stored no value: leave it unless FilmCraft set something
                    None => {
                        let default = def
                            .or_else(|| filmcraft_project::find_effect(m.effect))
                            .and_then(|d| d.params.iter().find(|p| p.id == m.param))
                            .map(|p| filmcraft_project::Param::new(p.default.clone()));
                        default.and_then(|d| prproj_fx::from_param(&d, m.conv, &ctx)).is_some_and(|d| prproj_fx::same(&d, &fc, &ctx))
                    }
                };
                if unchanged {
                    continue;
                }
                let t = prproj_fx::encode(m.node, pr.as_ref(), &fc, &ctx);
                let pi = self.inst_of(m.node, copies);
                self.write_param(pi, m.node, &t);
            }
        }
        // effects added in FilmCraft: a copy of a component of the same effect elsewhere in the
        // project (Premiere's own component layout), with FilmCraft's values
        let mut have: HashMap<&str, usize> = HashMap::new();
        for comp in &comps {
            if let Target::Effect(d) = comp.target() {
                *have.entry(d.id).or_insert(0) += 1;
            }
        }
        let mut next_component_id = comps
            .iter()
            .filter_map(|c| path_text(c.node, &["Component", "ID"]).or_else(|| path_text(c.node, &["AudioComponent", "Component", "ID"])))
            .filter_map(parse_i64)
            .max()
            .unwrap_or(0)
            + 1;
        let mut added_refs: Vec<String> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for e in &c.effects {
            let Some(def) = filmcraft_project::find_effect(&e.effect).filter(|d| !d.intrinsic) else { continue };
            if def.id == filmcraft_project::graphic::TEXT_LAYER || def.id == filmcraft_project::graphic::SHAPE_LAYER {
                let n = seen.entry(e.effect.as_str()).or_insert(0);
                *n += 1;
                if *n > have.get(e.effect.as_str()).copied().unwrap_or(0) {
                    self.report.warn(format!("a text or shape layer added to \"{}\" in FilmCraft is not written to Premiere projects yet", c.name));
                }
                continue;
            }
            let n = seen.entry(e.effect.as_str()).or_insert(0);
            *n += 1;
            if *n <= have.get(e.effect.as_str()).copied().unwrap_or(0) {
                continue;
            }
            let Some(model) = self.model_component(def, kind) else {
                self.report.warn(format!(
                    "effect \"{}\" added in FilmCraft was not written: the Premiere project has no {} to model it on (apply it once in Premiere)",
                    def.name, def.name
                ));
                continue;
            };
            let model_copies = self.clone_owned(model.node, &[]);
            let ci = self.inst_of(model.node, &model_copies);
            if let Some(idn) = self.ix.at(model.node, &["Component", "ID"]).or_else(|| self.ix.at(model.node, &["AudioComponent", "Component", "ID"])) {
                self.set_text(ci, idn, &next_component_id.to_string());
            }
            next_component_id += 1;
            let (maps, _) = prproj_fx::mappings(&model, Target::Effect(def));
            for m in maps {
                let fc_param = e
                    .params
                    .get(m.param)
                    .cloned()
                    .or_else(|| def.params.iter().find(|p| p.id == m.param).map(|p| filmcraft_project::Param::new(p.default.clone())));
                let Some(fc_param) = fc_param else { continue };
                let Some(fc) = prproj_fx::from_param(&fc_param, m.conv, &ctx) else { continue };
                let pr = prproj_fx::decode(m.node, m.conv);
                if pr.as_ref().is_some_and(|pr| prproj_fx::same(pr, &fc, &ctx)) {
                    continue;
                }
                let t = prproj_fx::encode(m.node, pr.as_ref(), &fc, &ctx);
                let pi = self.inst_of(m.node, &model_copies);
                self.write_param(pi, m.node, &t);
            }
            added_refs.push(self.id_of(ci));
            self.report.info(format!("added \"{}\" to a clip (modelled on another one in the project)", def.name));
        }
        if (keep_refs.iter().any(|k| !k) || !added_refs.is_empty())
            && let Some(list) = self.ix.at(chain, &["ComponentChain", "Components"])
        {
            let mut refs = String::new();
            let mut i = 0;
            for (r, keep) in list.children().filter(|n| n.has_tag_name("Component")).zip(keep_refs.iter().copied().chain(std::iter::repeat(true))) {
                if !keep {
                    continue;
                }
                let target = r
                    .attribute("ObjectRef")
                    .and_then(|o| copies.get(o))
                    .map(|ci| self.id_of(*ci))
                    .or_else(|| r.attribute("ObjectRef").map(str::to_string))
                    .unwrap_or_default();
                refs.push_str(&format!("<Component Index=\"{i}\" ObjectRef=\"{target}\" />"));
                i += 1;
            }
            for id in &added_refs {
                refs.push_str(&format!("<Component Index=\"{i}\" ObjectRef=\"{id}\" />"));
                i += 1;
            }
            self.set_list(chain_inst, list, &refs);
        }
    }

    /// Write a parameter's new text (`StartKeyframe`, `Keyframes`, `IsTimeVarying`, `CurrentValue`).
    fn write_param(&mut self, pi: usize, node: Node<'a, 'i>, t: &prproj_fx::ParamText) {
        if let Some(sk) = child(node, "StartKeyframe") {
            self.set_text(pi, sk, &t.start_keyframe);
        } else {
            self.insert(pi, node, &format!("<StartKeyframe>{}</StartKeyframe>", escape(&t.start_keyframe)));
        }
        match (&t.keyframes, child(node, "Keyframes")) {
            (Some(k), Some(n)) => self.set_text(pi, n, k),
            (Some(k), None) => self.insert(pi, node, &format!("<Keyframes>{}</Keyframes>", escape(k))),
            (None, Some(n)) => self.remove(pi, n),
            (None, None) => {}
        }
        let varying = if t.keyframes.is_some() { "true" } else { "false" };
        match child(node, "IsTimeVarying") {
            Some(n) => self.set_text(pi, n, varying),
            None if t.keyframes.is_some() => self.insert(pi, node, "<IsTimeVarying>true</IsTimeVarying>"),
            None => {}
        }
        if let Some(n) = child(node, "CurrentValue") {
            self.set_text(pi, n, &t.current);
        }
    }

    /// A component of effect `def` on any clip of this kind in the project, to copy for a clip
    /// that gets the effect in FilmCraft.
    fn model_component(&self, def: &'static filmcraft_project::effect::EffectDef, kind: TrackKind) -> Option<prproj_fx::Component<'a, 'i>> {
        let tag = if kind == TrackKind::Video { "VideoFilterComponent" } else { "AudioFilterComponent" };
        self.root
            .children()
            .filter(|n| n.has_tag_name(tag))
            .filter_map(|n| prproj_fx::component(&self.ix, n))
            .find(|c| matches!(c.target(), Target::Effect(d) if d.id == def.id))
    }

    // -----------------------------------------------------------------------------------------
    // Markers
    // -----------------------------------------------------------------------------------------

    /// The clips of the master clip of a sequence (`sequence`) or media, and their `Markers`.
    fn master_clips_of(&self, uid: &str, sequence: bool) -> (Vec<Node<'a, 'i>>, Option<Node<'a, 'i>>) {
        for pi in self.root.children().filter(|n| n.has_tag_name("ClipProjectItem")) {
            let Some(master) = self.ix.follow(pi, &["MasterClip"]) else { continue };
            let clips = self.ix.list(master, &["Clips"], "Clip");
            let ours = clips.iter().any(|clip| {
                let target = if sequence { prproj_graph::clip_sequence(&self.ix, *clip) } else { prproj_graph::clip_media(&self.ix, *clip) };
                target.and_then(|t| t.attribute("ObjectUID")) == Some(uid)
            });
            if ours {
                let markers = clips.iter().find_map(|c| self.ix.follow(*c, &["Clip", "MarkerOwner", "Markers"]));
                return (clips, markers);
            }
        }
        (Vec::new(), None)
    }

    /// Write the markers of a sequence or media: into its `Markers` object, or a new one when it
    /// had none.
    fn owner_markers(&mut self, uid: &str, sequence: bool, want: &[filmcraft_project::Marker], what: &str) {
        let (clips, markers) = self.master_clips_of(uid, sequence);
        if let Some(m) = markers {
            self.markers(m, want, what);
            return;
        }
        if want.is_empty() {
            return;
        }
        let bodies: Vec<(Node<'a, 'i>, Node<'a, 'i>)> =
            clips.iter().filter_map(|c| child(*c, "Clip").and_then(|b| child(b, "Source")).map(|src| (*c, src))).collect();
        if bodies.is_empty() {
            self.report.warn(format!("{what} has no master clip in the Premiere project to hold markers; they were not written"));
            return;
        }
        let id = self.alloc_id();
        let entries = self.marker_entries(want, &[]);
        self.created.push(format!(
            "<Markers ObjectID=\"{id}\" ClassID=\"bee50706-b524-416c-9f03-b596ce5f6866\" Version=\"4\"><Markers Version=\"1\">{entries}</Markers><ByGUID>byGUID</ByGUID></Markers>\n\t"
        ));
        for (clip, src) in bodies {
            let inst = self.here(clip);
            let at = src.range().start;
            self.edit(inst, at..at, format!("<MarkerOwner Version=\"1\"><Markers ObjectRef=\"{id}\" /></MarkerOwner>"));
        }
        self.report.info(format!("markers of {what} added"));
    }

    /// Rewrite a `Markers` object when FilmCraft's markers differ from Premiere's. Markers that
    /// match keep their Premiere identity; the others are written new.
    fn markers(&mut self, owner: Node<'a, 'i>, want: &[filmcraft_project::Marker], what: &str) {
        // Premiere's markers as the importer read them
        let mut have: Vec<(String, Option<&'a str>, MarkerKey)> = Vec::new();
        if let Some(list) = child(owner, "Markers") {
            for m in list.children().filter(|n| n.has_tag_name("Marker")) {
                let guid = child_text(m, "First").unwrap_or("").to_string();
                let Some(obj) = self.ix.follow(m, &["Second"]) else { continue };
                let Some(key) = child_text(obj, "DVAMarker").and_then(marker_key) else { continue };
                have.push((guid, obj.attribute("ObjectID"), key));
            }
        }
        let mut want_keys: Vec<MarkerKey> = want.iter().map(MarkerKey::of).collect();
        want_keys.sort_by_key(|a| a.start);
        let mut have_sorted: Vec<&MarkerKey> = have.iter().map(|h| &h.2).collect();
        have_sorted.sort_by_key(|a| a.start);
        if want_keys.len() == have_sorted.len() && want_keys.iter().zip(&have_sorted).all(|(a, b)| a == *b) {
            return;
        }
        let entries = self.marker_entries(want, &have);
        let inst = self.here(owner);
        match child(owner, "Markers") {
            Some(list) => self.set_list(inst, list, &entries),
            None if !entries.is_empty() => self.insert(inst, owner, &format!("<Markers Version=\"1\">{entries}</Markers>")),
            None => {}
        }
        self.report.info(format!("markers of {what} updated"));
    }

    /// `<Marker>` list entries for FilmCraft's markers, reusing Premiere's marker objects that
    /// match (`have`: GUID, object id, value) and creating the others.
    fn marker_entries(&mut self, want: &[filmcraft_project::Marker], have: &[(String, Option<&'a str>, MarkerKey)]) -> String {
        let mut want_keys: Vec<MarkerKey> = want.iter().map(MarkerKey::of).collect();
        want_keys.sort_by_key(|a| a.start);
        let mut used = vec![false; have.len()];
        let mut entries = String::new();
        for (i, k) in want_keys.iter().enumerate() {
            let reuse = have.iter().enumerate().find(|(j, h)| !used[*j] && h.2 == *k).map(|(j, h)| (j, h.0.clone(), h.1));
            let (guid, id) = match reuse {
                Some((j, guid, Some(id))) => {
                    used[j] = true;
                    (guid, id.to_string())
                }
                _ => {
                    let guid = self.uuid();
                    let id = self.alloc_id();
                    let json = k.json(&guid);
                    self.created.push(format!(
                        "<Marker ObjectID=\"{id}\" ClassID=\"a45508e0-3ff7-4d04-90a7-2e0dfff4c910\" Version=\"3\"><DVAMarker>{}</DVAMarker></Marker>\n\t",
                        escape(&json)
                    ));
                    (guid, id)
                }
            };
            entries.push_str(&format!("<Marker Version=\"1\" Index=\"{i}\"><First>{}</First><Second ObjectRef=\"{id}\" /></Marker>", escape(&guid)));
        }
        entries
    }

    // -----------------------------------------------------------------------------------------
    // Media FilmCraft added
    // -----------------------------------------------------------------------------------------

    /// A clip of media the Premiere project does not have: add the media (a copy of an asset of
    /// the same file type), then a copy of a track item of this kind retargeted to it.
    fn clip_of_new_media(&mut self, project: &Project, c: &TrackItem, kind: TrackKind, frame: (f64, f64)) -> Result<Option<String>> {
        let what = project.item(c.item).map(|i| i.name.clone()).unwrap_or_else(|| "?".into());
        let asset = match self.assets.get(&c.item) {
            Some(a) => a.clone(),
            None => {
                let a = self.add_asset(project, c.item);
                self.assets.insert(c.item, a.clone());
                a
            }
        };
        let want_tag = if kind == TrackKind::Video { "VideoClipTrackItem" } else { "AudioClipTrackItem" };
        let side = asset.as_ref().and_then(|a| if kind == TrackKind::Video { a.video.clone() } else { a.audio.clone() });
        let template = self.root.children().find(|n| n.has_tag_name(want_tag) && crate::prproj::importable(&self.ix, *n));
        let (Some(asset), Some(side), Some(template)) = (asset, side, template) else {
            self.report.warn(format!(
                "clip \"{}\" uses \"{what}\", which the Premiere project does not contain, and no {} file in it can serve as a model; it was not written (import that file in Premiere first)",
                c.name,
                ext_of(project, c.item)
            ));
            return Ok(None);
        };
        let copies = self.clone_owned(template, &[]);
        let inst = self.inst_of(template, &copies);
        let id = self.id_of(inst);
        // retarget the copy (as Premiere-accepted writers do): sub-clip → master clip and name;
        // clip → the master's source, markers and channel sources
        if let Some(sub) = self.ix.follow(template, &["ClipTrackItem", "SubClip"]) {
            let si = self.inst_of(sub, &copies);
            if let Some(m) = child(sub, "MasterClip") {
                self.set_attr(si, m, "ObjectURef", &asset.master_uid);
            }
            self.set_child(si, sub, "Name", &asset.name);
            if let Some(clip) = self.ix.follow(sub, &["Clip"]) {
                let ci = self.inst_of(clip, &copies);
                if let Some(src) = self.ix.at(clip, &["Clip", "Source"]) {
                    self.set_attr(ci, src, "ObjectRef", &side.source);
                }
                if let (Some(m), Some(new)) = (self.ix.at(clip, &["Clip", "MarkerOwner", "Markers"]), side.markers.as_deref()) {
                    self.set_attr(ci, m, "ObjectRef", new);
                }
                if let Some(sc) = child(clip, "SecondaryContents") {
                    match side.secondary.as_deref() {
                        Some(xml) => self.set_list(ci, sc, xml),
                        None => self.remove(ci, sc),
                    }
                }
            }
        }
        // a copy of another clip's settings: FilmCraft's own settings replace them
        let mut fresh = c.clone();
        fresh.origin = None;
        self.patch_clip(project, &fresh, kind, frame, template, inst, &copies, true);
        self.clip_objects.insert(id.clone(), (inst, template));
        Ok(Some(id))
    }

    /// Copy an asset of the same file type for a FilmCraft media item and point it at its file.
    fn add_asset(&mut self, project: &Project, item: ItemId) -> Option<Asset> {
        let it = project.item(item)?;
        let ItemKind::Media(m) = &it.kind else { return None };
        let filmcraft_project::MediaRef::File { path } = &m.media else { return None };
        let info = &m.info;
        let ext = ext_of(project, item);
        let channels = info.audio.as_ref().map(|a| a.channels);
        let still = info.kind == filmcraft_media::MediaKind::Still;
        if !still && info.duration.0 <= 0 {
            return None;
        }
        // a project item whose media has the same extension and streams (and channel count)
        let mut best: Option<(u8, Node<'a, 'i>)> = None;
        for pi in self.root.children().filter(|n| n.has_tag_name("ClipProjectItem")) {
            let Some(master) = self.ix.follow(pi, &["MasterClip"]) else { continue };
            let Some(media) = self.ix.list(master, &["Clips"], "Clip").into_iter().find_map(|c| prproj_graph::clip_media(&self.ix, c)) else { continue };
            let fp = child_text(media, "ActualMediaFilePath").or_else(|| child_text(media, "FilePath")).unwrap_or("");
            if !fp.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
                continue;
            }
            let has_v = child(media, "VideoStream").is_some();
            let has_a = child(media, "AudioStream").is_some();
            if has_v != info.video.is_some() || (has_a != info.audio.is_some() && !still) {
                continue;
            }
            let ch = self
                .ix
                .follow(media, &["AudioStream"])
                .and_then(|a| child_text(a, "AudioChannelLayout"))
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .and_then(|v| v.as_array().map(|a| a.len() as u32));
            let score = u8::from(ch == channels) + 1;
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, pi));
            }
        }
        let (_, template) = best?;
        let copies = self.clone_graph(template);
        let name = crate::common::file_name(path).to_string();
        let win_path = self.premiere_path(path);
        let rel = self.dest_dir.as_deref().and_then(|d| crate::common::relative_path(path, d)).map(|r| {
            let r = r.replace('/', "\\");
            if r.starts_with("..") { r } else { format!(".\\{r}") }
        });
        let length = info.duration.0.max(0);
        let video_rate = info
            .video
            .as_ref()
            .map(|v| v.frame_rate)
            .filter(|r| r.num > 0 && r.den > 0)
            .map(|r| (prproj_graph::TICKS_PER_SECOND as i128 * r.den as i128 / r.num as i128) as i64);
        let audio_rate = info.audio.as_ref().map(|a| a.sample_rate).filter(|r| *r > 0).map(|r| prproj_graph::TICKS_PER_SECOND / r as i64);
        let mut asset = Asset { master_uid: String::new(), name: name.clone(), video: None, audio: None };
        let objects: Vec<(Node<'a, 'i>, usize)> =
            copies.iter().filter_map(|(k, i)| self.ix.ids.get(k).or_else(|| self.ix.uids.get(k)).map(|n| (*n, *i))).collect();
        let new_key = |w: &Self, n: Node<'a, 'i>| -> Option<String> {
            n.attribute("ObjectID").or_else(|| n.attribute("ObjectUID")).and_then(|k| copies.get(k)).map(|i| w.id_of(*i))
        };
        for (n, i) in objects {
            match n.tag_name().name() {
                "ClipProjectItem" => {
                    if let Some(pn) = self.ix.at(n, &["ProjectItem", "Name"]) {
                        self.set_text(i, pn, &name);
                    }
                    self.added_items.push(self.id_of(i));
                }
                "MasterClip" => {
                    self.set_child(i, n, "Name", &name);
                    asset.master_uid = self.id_of(i);
                    for clip in self.ix.list(n, &["Clips"], "Clip") {
                        let src =
                            self.ix.at(clip, &["Clip", "Source"]).and_then(|r| r.attribute("ObjectRef")).and_then(|k| copies.get(k)).map(|ci| self.id_of(*ci));
                        let markers = self
                            .ix
                            .at(clip, &["Clip", "MarkerOwner", "Markers"])
                            .and_then(|r| r.attribute("ObjectRef"))
                            .and_then(|k| copies.get(k))
                            .map(|ci| self.id_of(*ci));
                        let secondary = child(clip, "SecondaryContents").map(|sc| {
                            sc.children()
                                .filter(Node::is_element)
                                .map(|r| {
                                    let mut x = self.text[r.range()].to_string();
                                    if let Some(old) = r.attribute("ObjectRef")
                                        && let Some(ci) = copies.get(old)
                                    {
                                        x = x.replace(&format!("ObjectRef=\"{old}\""), &format!("ObjectRef=\"{}\"", self.id_of(*ci)));
                                    }
                                    x
                                })
                                .collect::<String>()
                        });
                        if let Some(source) = src {
                            let side = AssetClip { source, markers, secondary };
                            if clip.has_tag_name("VideoClip") {
                                asset.video = Some(side);
                            } else if clip.has_tag_name("AudioClip") {
                                asset.audio = Some(side);
                            }
                        }
                        // the master clip's own in/out: the whole file
                        if !still && let Some(body) = child(clip, "Clip") {
                            let ci = copies.get(clip.attribute("ObjectID").unwrap_or("")).copied();
                            if let Some(ci) = ci {
                                self.set_child(ci, body, "InPoint", "0");
                                self.set_child(ci, body, "OutPoint", &length.to_string());
                            }
                        }
                    }
                }
                "ClipLoggingInfo" => {
                    if let Some(x) = child(n, "ClipName") {
                        self.set_text(i, x, &name);
                    }
                    if !still {
                        if let Some(x) = child(n, "MediaInPoint") {
                            self.set_text(i, x, "0");
                        }
                        if let Some(x) = child(n, "MediaOutPoint") {
                            self.set_text(i, x, &length.to_string());
                        }
                        if let (Some(x), Some(r)) = (child(n, "MediaFrameRate"), video_rate.or(audio_rate)) {
                            self.set_text(i, x, &r.to_string());
                        }
                    }
                }
                "Media" => {
                    for c in n.children().filter(Node::is_element) {
                        match c.tag_name().name() {
                            "FilePath" | "ActualMediaFilePath" => self.set_text(i, c, &win_path),
                            "RelativePath" => self.set_text(i, c, rel.as_deref().unwrap_or(&format!(".\\{name}"))),
                            "Title" => self.set_text(i, c, &name),
                            "ConformedAudioRate" => {
                                if let Some(r) = audio_rate {
                                    self.set_text(i, c, &r.to_string());
                                }
                            }
                            t if t == "ModificationState" || t == "ContentAndMetadataState" || t.starts_with("MediaFileHistory") => self.remove(i, c),
                            _ => {}
                        }
                    }
                }
                "VideoStream" | "AudioStream" => {
                    for c in n.children().filter(Node::is_element) {
                        if matches!(c.tag_name().name(), "PeakFilePath" | "ConformedAudioPath") {
                            self.remove(i, c);
                        }
                    }
                    if !still && let Some(d) = child(n, "Duration") {
                        self.set_text(i, d, &length.to_string());
                    }
                    if n.has_tag_name("VideoStream") {
                        if let (Some(fr), Some(v)) = (child(n, "FrameRect"), info.video.as_ref()) {
                            self.set_text(i, fr, &format!("0,0,{},{}", v.width, v.height));
                        }
                        if let (Some(r), Some(t)) = (child(n, "FrameRate"), video_rate) {
                            self.set_text(i, r, &t.to_string());
                        }
                    } else if let (Some(r), Some(t)) = (child(n, "FrameRate"), audio_rate) {
                        self.set_text(i, r, &t.to_string());
                    }
                }
                "VideoMediaSource" | "AudioMediaSource" if !still => {
                    if let Some(d) = child(n, "OriginalDuration") {
                        self.set_text(i, d, &length.to_string());
                    }
                }
                _ => {}
            }
            let _ = new_key(self, n);
        }
        if asset.master_uid.is_empty() {
            return None;
        }
        self.report.info(format!("added \"{name}\" to the Premiere project (modelled on another .{ext} file)"));
        Some(asset)
    }

    /// FilmCraft's path in the Premiere project's terms (`/mnt/LocalDisk/x.wav` → `L:\x.wav`).
    fn premiere_path(&self, path: &str) -> String {
        for (from, to) in &self.prefixes {
            if let Some(rest) = path.strip_prefix(from.as_str()) {
                let rest = rest.trim_start_matches('/').replace('/', "\\");
                return format!("{}\\{rest}", to.trim_end_matches('\\'));
            }
        }
        path.to_string()
    }

    fn set_attr(&mut self, inst: usize, el: Node<'a, 'i>, name: &str, value: &str) {
        if let Some(a) = el.attributes().find(|a| a.name() == name) {
            self.edit(inst, a.range_value(), escape(value));
        }
    }

    // -----------------------------------------------------------------------------------------
    // Transitions, links, duration
    // -----------------------------------------------------------------------------------------

    fn transitions(
        &mut self,
        ft: &filmcraft_project::Track,
        pt: Node<'a, 'i>,
        tinst: usize,
        written: &HashMap<filmcraft_project::ClipId, String>,
        exists_in_filmcraft: bool,
    ) {
        let originals = self.ix.list(pt, &["ClipTrack", "TransitionItems", "TrackItems"], "TrackItem");
        if originals.is_empty() && ft.transitions.is_empty() {
            return;
        }
        // which clips each original transition joins, by Premiere id (as the importer found them)
        let clip_items = self.ix.list(pt, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem");
        let span = |n: Node| -> Option<(i64, i64)> {
            let t = self.ix.at(n, &["ClipTrackItem", "TrackItem"])?;
            Some((time(child_text(t, "Start")).unwrap_or(0), time(child_text(t, "End"))?))
        };
        let mut joins: Vec<(Node<'a, 'i>, Option<&'a str>, Option<&'a str>)> = Vec::new();
        for t in &originals {
            let Some(r) = self.ix.at(*t, &["TransitionTrackItem", "TrackItem"]) else { continue };
            let (s, e) = (time(child_text(r, "Start")).unwrap_or(0), time(child_text(r, "End")).unwrap_or(0));
            let flag = |tag: &str| self.ix.at(*t, &["TransitionTrackItem", tag]).and_then(|n| n.text()).is_none_or(|v| v.trim() == "true");
            let (outgoing, incoming) = (flag("HasOutgoingClip"), flag("HasIncomingClip"));
            let from = clip_items
                .iter()
                .filter_map(|c| span(*c).map(|sp| (c, sp)))
                .filter(|(_, (_, ce))| *ce >= s && *ce <= e)
                .max_by_key(|(_, (_, ce))| *ce)
                .and_then(|(c, _)| c.attribute("ObjectID"));
            let to = clip_items
                .iter()
                .filter_map(|c| span(*c).map(|sp| (c, sp)))
                .filter(|(_, (cs, _))| *cs >= s && *cs <= e)
                .min_by_key(|(_, (cs, _))| *cs)
                .and_then(|(c, _)| c.attribute("ObjectID"));
            joins.push((*t, from.filter(|_| outgoing), to.filter(|_| incoming)));
        }
        let origin_of = |id: filmcraft_project::ClipId| ft.items.iter().find(|i| i.id == id).and_then(|i| i.origin.clone());
        let mut kept = Vec::new();
        let mut used = HashSet::new();
        // new transitions: (start, id) and the clip back-references they need
        let mut added: Vec<(i64, String)> = Vec::new();
        let mut backrefs: Vec<(String, &'static str, String)> = Vec::new();
        for tr in if exists_in_filmcraft { &ft.transitions[..] } else { &[] } {
            let (fo, to) = (tr.from.and_then(origin_of), tr.to.and_then(origin_of));
            let m = joins
                .iter()
                .position(|(n, f, t)| !used.contains(&n.id()) && (fo.is_some() || to.is_some()) && f.map(str::to_string) == fo && t.map(str::to_string) == to);
            let Some(m) = m else {
                if let Some(id) = self.new_transition(tr, pt, written) {
                    if let Some(f) = tr.from.and_then(|f| written.get(&f)) {
                        backrefs.push((f.clone(), "TailTransition", id.clone()));
                    }
                    if let Some(t) = tr.to.and_then(|t| written.get(&t)) {
                        backrefs.push((t.clone(), "HeadTransition", id.clone()));
                    }
                    added.push((tr.start.0, id));
                }
                continue;
            };
            let Some((node, _, _)) = joins.get(m).copied() else { continue };
            used.insert(node.id());
            let inst = self.here(node);
            if let Some(r) = self.ix.at(node, &["TransitionTrackItem", "TrackItem"]) {
                let (s, e) = (tr.start.0.to_string(), tr.end().0.to_string());
                if child_text(r, "Start").unwrap_or("0").trim() != s {
                    self.set_child(inst, r, "Start", &s);
                }
                if child_text(r, "End").map(str::trim) != Some(e.as_str()) {
                    self.set_child(inst, r, "End", &e);
                }
            }
            if let Some(tt) = child(node, "TransitionTrackItem")
                && let Some((_, jf, jt)) = joins.get(m).copied()
            {
                // a side changes only when FilmCraft's clip there differs from what was imported
                let sides = [
                    ("HasOutgoingClip", tr.from.is_some_and(|f| written.contains_key(&f)), jf.is_some()),
                    ("HasIncomingClip", tr.to.is_some_and(|t| written.contains_key(&t)), jt.is_some()),
                ];
                for (tag, has, had) in sides {
                    if has != had {
                        self.set_child(inst, tt, tag, if has { "true" } else { "false" });
                    }
                }
            }
            kept.push(node);
        }
        // transitions FilmCraft never had (joined to no clip when imported) stay as they are
        for (n, f, t) in &joins {
            if f.is_none() && t.is_none() && !kept.iter().any(|k| k.id() == n.id()) {
                kept.push(*n);
            }
        }
        let order: HashMap<roxmltree::NodeId, usize> = originals.iter().enumerate().map(|(i, n)| (n.id(), i)).collect();
        kept.sort_by_key(|n| order.get(&n.id()).copied().unwrap_or(usize::MAX));
        let same = added.is_empty() && kept.len() == originals.len() && kept.iter().zip(&originals).all(|(a, b)| a.id() == b.id());
        if !same {
            // originals in their order, new ones by time among them
            let mut all: Vec<(i64, usize, String)> = kept
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let start = self.ix.at(*n, &["TransitionTrackItem", "TrackItem"]).and_then(|r| time(child_text(r, "Start"))).unwrap_or(0);
                    (start, i, n.attribute("ObjectID").unwrap_or("").to_string())
                })
                .collect();
            all.extend(added.iter().map(|(s0, id)| (*s0, usize::MAX, id.clone())));
            all.sort_by_key(|(s0, i, _)| (*s0, *i));
            let refs: String = all.iter().enumerate().map(|(i, (_, _, id))| format!("<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />")).collect();
            if let Some(list) = self.ix.at(pt, &["ClipTrack", "TransitionItems", "TrackItems"]) {
                self.set_list(tinst, list, &refs);
            } else if let Some(ti) = self.ix.at(pt, &["ClipTrack", "TransitionItems"]) {
                self.insert(tinst, ti, &format!("<TrackItems Version=\"1\">{refs}</TrackItems>"));
            }
        }
        // clips that pointed at a dropped transition, and clips joined by a new one
        let kept_ids: HashSet<&str> = kept.iter().filter_map(|n| n.attribute("ObjectID")).collect();
        let mut retarget: HashMap<(String, &'static str), String> = backrefs.iter().map(|(c, tag, t)| ((c.clone(), *tag), t.clone())).collect();
        for c in clip_items {
            let Some(cti) = child(c, "ClipTrackItem") else { continue };
            let cid = c.attribute("ObjectID").unwrap_or("").to_string();
            for t in cti.children().filter(|n| n.has_tag_name("HeadTransition") || n.has_tag_name("TailTransition")) {
                let tag: &'static str = if t.has_tag_name("HeadTransition") { "HeadTransition" } else { "TailTransition" };
                let Some((ci, _)) = self.clip_objects.get(&cid).copied().filter(|(i, _)| !inst_is_copy(&self.instances, *i)) else { continue };
                match retarget.remove(&(cid.clone(), tag)) {
                    Some(new) => self.set_attr(ci, t, "ObjectRef", &new),
                    None if t.attribute("ObjectRef").is_some_and(|r| !kept_ids.contains(r)) => self.remove(ci, t),
                    None => {}
                }
            }
        }
        for ((clip, tag), t) in retarget {
            let Some((ci, obj)) = self.clip_objects.get(&clip).copied() else { continue };
            if let Some(cti) = child(obj, "ClipTrackItem") {
                self.insert(ci, cti, &format!("<{tag} ObjectRef=\"{t}\" />"));
            }
        }
    }

    /// A transition FilmCraft added (or moved onto other clips): a copy of a transition of the
    /// same type in the project, retimed. `None` (reported) when there is no model.
    fn new_transition(&mut self, tr: &filmcraft_project::Transition, pt: Node<'a, 'i>, written: &HashMap<filmcraft_project::ClipId, String>) -> Option<String> {
        let name = crate::common::transition_name(&tr.effect);
        let tag = if pt.tag_name().name().starts_with("Audio") { "AudioTransitionTrackItem" } else { "VideoTransitionTrackItem" };
        let model = self
            .root
            .children()
            .filter(|n| n.has_tag_name(tag))
            .find(|n| path_text(*n, &["TransitionTrackItem", "DisplayName"]).is_some_and(|d| d.eq_ignore_ascii_case(&name)));
        let Some(model) = model else {
            self.report.warn(format!(
                "a {name} transition FilmCraft added was not written: the Premiere project has no {name} to model it on (add one in Premiere once)"
            ));
            return None;
        };
        let copies = self.clone_owned(model, &[]);
        let inst = self.inst_of(model, &copies);
        if let Some(r) = self.ix.at(model, &["TransitionTrackItem", "TrackItem"]) {
            self.set_child(inst, r, "Start", &tr.start.0.to_string());
            self.set_child(inst, r, "End", &tr.end().0.to_string());
        }
        if let Some(tt) = child(model, "TransitionTrackItem") {
            let out = tr.from.is_some_and(|f| written.contains_key(&f));
            let inc = tr.to.is_some_and(|t| written.contains_key(&t));
            self.set_child(inst, tt, "HasOutgoingClip", if out { "true" } else { "false" });
            self.set_child(inst, tt, "HasIncomingClip", if inc { "true" } else { "false" });
        }
        self.report.info(format!("added a {name} transition (modelled on another one in the project)"));
        Some(self.id_of(inst))
    }

    fn links(&mut self, seq: &Sequence, pseq: Node<'a, 'i>, written: &HashMap<filmcraft_project::ClipId, String>) {
        let mut groups: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for t in seq.all_tracks() {
            for c in &t.items {
                if let (Some(l), Some(id)) = (c.link, written.get(&c.id)) {
                    groups.entry(l).or_default().push(id.clone());
                }
            }
        }
        let Some(container) = self.ix.at(pseq, &["PersistentGroupContainer", "LinkContainer"]) else {
            if groups.values().any(|g| g.len() > 1) {
                self.report.warn("the Premiere sequence keeps no links; linked clips were written unlinked");
            }
            return;
        };
        let originals = self.ix.list(container, &["Links"], "Link");
        let members = |l: Node<'a, 'i>| -> Vec<String> {
            let mut v: Vec<String> = self
                .ix
                .at(l, &["TrackItemGroup", "TrackItems"])
                .into_iter()
                .flat_map(|n| n.children().filter(|c| c.has_tag_name("TrackItem")))
                .filter_map(|r| r.attribute("ObjectRef").map(str::to_string))
                .collect();
            v.sort();
            v
        };
        let mut by_members: HashMap<Vec<String>, &'a str> = HashMap::new();
        for l in &originals {
            if let Some(id) = l.attribute("ObjectID") {
                by_members.insert(members(*l), id);
            }
        }
        let written_set: HashSet<&str> = written.values().map(String::as_str).collect();
        let mut in_sequence: HashSet<&str> = HashSet::new();
        for tag in ["VideoTrackGroup", "AudioTrackGroup"] {
            let Some(g) = prproj_graph::track_group(&self.ix, pseq, tag) else { continue };
            for t in prproj_graph::tracks(&self.ix, g) {
                for c in self.ix.list(t, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem") {
                    if let Some(id) = c.attribute("ObjectID") {
                        in_sequence.insert(id);
                    }
                }
            }
        }
        let mut wanted: Vec<Vec<String>> = groups
            .into_values()
            .filter(|g| g.len() > 1)
            .map(|mut g| {
                g.sort();
                g
            })
            .collect();
        let mut ids = Vec::new();
        for l in &originals {
            let Some(id) = l.attribute("ObjectID") else { continue };
            let m = members(*l);
            let ours = m.iter().filter(|x| written_set.contains(x.as_str())).count();
            if let Some(i) = wanted.iter().position(|g| *g == m) {
                wanted.remove(i);
                ids.push(id.to_string());
            } else if m.iter().all(|x| !in_sequence.contains(x.as_str())) || (m.len() == 1 && ours == 1) {
                // not about this sequence's clips, or a one-clip link FilmCraft cannot express
                ids.push(id.to_string());
            }
        }
        for g in wanted {
            match by_members.get(&g) {
                Some(id) => ids.push(id.to_string()),
                None => {
                    let id = self.alloc_id();
                    let refs: String = g.iter().enumerate().map(|(i, m)| format!("<TrackItem Index=\"{i}\" ObjectRef=\"{m}\" />")).collect();
                    self.created.push(format!(
                        "<Link ObjectID=\"{id}\" ClassID=\"149d4ea5-a7d4-4b34-9bb7-16d783904bf2\" Version=\"1\"><TrackItemGroup Version=\"1\"><TrackItems Version=\"1\">{refs}</TrackItems></TrackItemGroup></Link>\n\t"
                    ));
                    ids.push(id);
                }
            }
        }
        let original_ids: HashSet<&str> = originals.iter().filter_map(|l| l.attribute("ObjectID")).collect();
        if ids.len() == original_ids.len() && ids.iter().all(|i| original_ids.contains(i.as_str())) {
            return;
        }
        let refs: String = ids.iter().enumerate().map(|(i, id)| format!("<Link Index=\"{i}\" ObjectRef=\"{id}\" />")).collect();
        let ci = self.here(pseq);
        match child(container, "Links") {
            Some(list) => self.set_list(ci, list, &refs),
            None if !ids.is_empty() => self.insert(ci, container, &format!("<Links Version=\"1\">{refs}</Links>")),
            None => {}
        }
    }

    /// The sequence's own source length (used where it is nested) follows its new end.
    fn sequence_duration(&mut self, seq: &Sequence, pseq: Node<'a, 'i>) {
        let Some(uid) = pseq.attribute("ObjectUID") else { return };
        let end = seq.all_tracks().flat_map(|t| t.items.iter().map(|c| c.end().0)).max().unwrap_or(0);
        let mut old_end = 0;
        for tag in ["VideoTrackGroup", "AudioTrackGroup"] {
            let Some(g) = prproj_graph::track_group(&self.ix, pseq, tag) else { continue };
            for t in prproj_graph::tracks(&self.ix, g) {
                for c in self.ix.list(t, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem") {
                    if let Some(e) = self.ix.at(c, &["ClipTrackItem", "TrackItem"]).and_then(|ti| time(child_text(ti, "End"))) {
                        old_end = old_end.max(e);
                    }
                }
            }
        }
        if end == old_end || end <= 0 {
            return;
        }
        let sources: Vec<Node> = self
            .root
            .children()
            .filter(|n| n.has_tag_name("VideoSequenceSource") || n.has_tag_name("AudioSequenceSource"))
            .filter(|n| self.ix.at(*n, &["SequenceSource", "Sequence"]).and_then(|s| s.attribute("ObjectURef")) == Some(uid))
            .collect();
        for s in sources {
            if let Some(d) = child(s, "OriginalDuration") {
                let i = self.here(s);
                self.set_text(i, d, &end.to_string());
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Assembly
    // -----------------------------------------------------------------------------------------

    fn finish(&mut self) -> Result<Output> {
        if !self.added_items.is_empty()
            && let Some(rp) = self.root.children().find(|n| n.has_tag_name("RootProjectItem"))
            && let Some(items) = self.ix.at(rp, &["ProjectItemContainer", "Items"])
        {
            let n = items.children().filter(|c| c.has_tag_name("Item")).count();
            let xml: String = self.added_items.iter().enumerate().map(|(i, uid)| format!("<Item Index=\"{}\" ObjectURef=\"{uid}\" />", n + i)).collect();
            let ri = self.here(rp);
            self.insert(ri, items, &xml);
        }
        let mut splices: Vec<(Range<usize>, String)> = Vec::new();
        let mut appended = String::new();
        for inst in &self.instances {
            match &inst.new_id {
                None => splices.extend(inst.edits.iter().cloned()),
                Some(new_id) => {
                    let r = inst.src.range();
                    let mut edits: Vec<(Range<usize>, String)> = inst.edits.iter().filter(|(e, _)| e.start >= r.start && e.end <= r.end).cloned().collect();
                    let attr =
                        inst.src.attributes().find(|a| a.name() == "ObjectID" || a.name() == "ObjectUID").ok_or_else(|| err("copied object has no id"))?;
                    edits.push((attr.range_value(), new_id.clone()));
                    let body = apply(&self.text[r.clone()], r.start, edits)?;
                    appended.push_str(&body);
                    appended.push_str("\n\t");
                }
            }
        }
        for c in &self.created {
            appended.push_str(c);
        }
        if !appended.is_empty() {
            let r = self.root.range();
            let close = "</PremiereData>";
            let src = &self.text[r.clone()];
            if !src.trim_end().ends_with(close) {
                return Err(err("the project does not end with </PremiereData>"));
            }
            let at = r.start + src.trim_end().len() - close.len();
            splices.push((at..at, appended));
        }
        let text = apply(self.text, 0, splices)?;
        Ok(Output { text, report: std::mem::take(&mut self.report) })
    }
}

struct Output {
    text: String,
    report: Report,
}

fn inst_is_copy(instances: &[Instance], i: usize) -> bool {
    instances.get(i).is_some_and(|x| x.new_id.is_some())
}

/// Objects a copied clip always gets its own copy of (as writers Premiere accepts do), besides
/// any object only the clip refers to.
const OWNED_KINDS: &[&str] = &[
    "VideoClipTrackItem",
    "AudioClipTrackItem",
    "SubClip",
    "VideoClip",
    "AudioClip",
    "VideoComponentChain",
    "AudioComponentChain",
    "VideoFilterComponent",
    "AudioFilterComponent",
    "SecondaryContent",
];

/// Reference elements of an object, skipping embedded objects that number their own children.
pub(crate) fn public_refs<'a, 'i>(obj: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    public_refs_skipping(obj, &[])
}

/// [`public_refs`], not descending into elements named in `skip`.
fn public_refs_skipping<'a, 'i>(obj: Node<'a, 'i>, skip: &[&str]) -> Vec<Node<'a, 'i>> {
    let mut out = Vec::new();
    let mut stack: Vec<Node> = obj.children().filter(Node::is_element).collect();
    while let Some(n) = stack.pop() {
        if n.attribute("ObjectID").is_some() || n.attribute("ObjectUID").is_some() || skip.contains(&n.tag_name().name()) {
            continue; // a private namespace (view state), or a part the caller excludes
        }
        if n.attribute("ObjectRef").is_some() || n.attribute("ObjectURef").is_some() {
            out.push(n);
        }
        stack.extend(n.children().filter(Node::is_element));
    }
    out
}

/// Apply byte-range edits to `src` (whose first byte is at `base` in the original document). An
/// edit inside a larger edit is dropped (the larger replacement wins); partial overlaps are a bug
/// and an error.
fn apply(src: &str, base: usize, mut edits: Vec<(Range<usize>, String)>) -> Result<String> {
    // by start; at one position insertions first (in the order they were made), then the
    // longest replacement
    edits.sort_by(|a, b| a.0.start.cmp(&b.0.start).then((!a.0.is_empty()).cmp(&!b.0.is_empty())).then(b.0.end.cmp(&a.0.end)));
    let mut out = String::with_capacity(src.len() + edits.iter().map(|e| e.1.len()).sum::<usize>());
    let mut pos = base;
    let end = base + src.len();
    for (r, text) in edits {
        if r.start < pos {
            if r.end <= pos {
                continue; // inside an earlier, larger replacement
            }
            return Err(err("internal error: overlapping edits"));
        }
        if r.end > end || r.start > r.end {
            return Err(err("internal error: edit outside its object"));
        }
        out.push_str(src.get(pos - base..r.start - base).ok_or_else(|| err("internal error: edit not on a character boundary"))?);
        out.push_str(&text);
        pos = r.end;
    }
    out.push_str(src.get(pos - base..).ok_or_else(|| err("internal error: edit not on a character boundary"))?);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------------------------

/// What a written clip must read back as.
#[derive(Clone, Debug, PartialEq)]
struct ExportedClip {
    start: i64,
    duration: i64,
    source_in: i64,
    speed: f64,
    reverse: bool,
    frame_hold: Option<i64>,
}

impl ExportedClip {
    fn of(c: &TrackItem) -> Self {
        Self { start: c.start.0, duration: c.duration.0, source_in: c.source_in.0, speed: c.speed, reverse: c.reverse, frame_hold: c.frame_hold.map(|t| t.0) }
    }
}

/// A marker as both sides can compare it.
#[derive(Clone, Debug, PartialEq)]
struct MarkerKey {
    start: i64,
    duration: i64,
    name: String,
    comment: String,
    kind: String,
}

/// A marker's `DVAMarker` JSON, as Premiere writes it (`guid` is its `mMarkerID`).
pub(crate) fn marker_json(m: &filmcraft_project::Marker, guid: &str) -> String {
    MarkerKey::of(m).json(guid)
}

impl MarkerKey {
    fn json(&self, guid: &str) -> String {
        let mut d = serde_json::Map::new();
        if !self.comment.is_empty() {
            d.insert("mComment".into(), self.comment.clone().into());
        }
        if self.duration > 0 {
            d.insert("mDuration".into(), serde_json::json!({"ticks": self.duration}));
        }
        d.insert("mMarkerID".into(), guid.into());
        if !self.name.is_empty() {
            d.insert("mName".into(), self.name.clone().into());
        }
        d.insert("mStartTime".into(), serde_json::json!({"ticks": self.start}));
        d.insert("mType".into(), self.kind.clone().into());
        serde_json::json!({ "DVAMarker": d }).to_string()
    }

    fn of(m: &filmcraft_project::Marker) -> Self {
        let kind = match m.kind {
            filmcraft_project::MarkerKind::Chapter => "Chapter",
            filmcraft_project::MarkerKind::Segmentation => "Segmentation",
            filmcraft_project::MarkerKind::WebLink => "WebLink",
            filmcraft_project::MarkerKind::FlashCue => "FlashCuePoint",
            _ => "Comment",
        };
        Self { start: m.start.0, duration: m.duration.0, name: m.name.clone(), comment: m.comment.clone(), kind: kind.into() }
    }
}

/// A Premiere marker (`DVAMarker` JSON) the way the importer reads it.
fn marker_key(json: &str) -> Option<MarkerKey> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let d = &v["DVAMarker"];
    let ticks = |k: &str| d[k]["ticks"].as_i64().or_else(|| d[k]["ticks"].as_str().and_then(|s| s.parse().ok()));
    let kind = d["mType"].as_str().filter(|k| matches!(*k, "Chapter" | "Segmentation" | "WebLink" | "FlashCuePoint")).unwrap_or("Comment");
    Some(MarkerKey {
        start: ticks("mStartTime").unwrap_or(0).clamp(0, crate::prproj_fx::MAX_TIME),
        duration: ticks("mDuration").unwrap_or(0).clamp(0, crate::prproj_fx::MAX_TIME),
        name: d["mName"].as_str().unwrap_or("").to_string(),
        comment: d["mComment"].as_str().unwrap_or("").to_string(),
        kind: kind.to_string(),
    })
}

fn ext_of(project: &Project, item: ItemId) -> String {
    match project.item(item).map(|i| &i.kind) {
        Some(ItemKind::Media(m)) => match &m.media {
            filmcraft_project::MediaRef::File { path } => crate::common::extension(path),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Path prefixes (FilmCraft's, Premiere's) learned from media both know: the most common
/// pair of what differs once the shared tail of the two paths is removed.
fn path_prefixes(project: &Project, ix: &Index) -> Vec<(String, String)> {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for it in project.items.values() {
        let (ItemKind::Media(m), Some(uids)) = (&it.kind, it.metadata.get(META_MEDIA)) else { continue };
        let filmcraft_project::MediaRef::File { path } = &m.media else { continue };
        for uid in uids.split(',') {
            let Some(media) = ix.uids.get(uid) else { continue };
            let Some(win) = child_text(*media, "ActualMediaFilePath").or_else(|| child_text(*media, "FilePath")) else { continue };
            let a: Vec<&str> = path.split('/').collect();
            let b: Vec<&str> = win.split('\\').collect();
            let shared = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x.eq_ignore_ascii_case(y)).count();
            if shared == 0 || shared >= a.len() || shared >= b.len() {
                continue;
            }
            let from = a[..a.len() - shared].join("/");
            let to = b[..b.len() - shared].join("\\");
            *counts.entry((from, to)).or_insert(0) += 1;
        }
    }
    let mut v: Vec<((String, String), usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.0.len().cmp(&a.0.0.len())));
    v.into_iter().map(|(k, _)| k).collect()
}

/// Which clips the importer moved off their Premiere track (Premiere lets clips overlap on a
/// track; FilmCraft cannot), and to which FilmCraft track.
fn displaced(text: &str) -> HashMap<(String, bool, String), usize> {
    let mut out = HashMap::new();
    let Ok(doc) = crate::xml::parse(text) else { return out };
    let root = doc.root_element();
    let ix = Index::new(root);
    let mut report = Report::default();
    let Ok(imported) = crate::prproj::import(text.as_bytes(), &ImportOptions::default(), &mut report) else { return out };
    for item in imported.project.items.values() {
        let (Some(seq), Some(uid)) = (item.as_sequence(), item.metadata.get(META_SEQUENCE)) else { continue };
        let Some(pseq) = ix.uids.get(uid.as_str()).copied() else { continue };
        for (kind, tag) in [(TrackKind::Video, "VideoTrackGroup"), (TrackKind::Audio, "AudioTrackGroup")] {
            let Some(group) = prproj_graph::track_group(&ix, pseq, tag) else { continue };
            let mut home: HashMap<&str, usize> = HashMap::new();
            for (ti, t) in prproj_graph::tracks(&ix, group).into_iter().enumerate() {
                for c in ix.list(t, &["ClipTrack", "ClipItems", "TrackItems"], "TrackItem") {
                    if let Some(id) = c.attribute("ObjectID") {
                        home.insert(id, ti);
                    }
                }
            }
            for (ti, t) in seq.tracks(kind).iter().enumerate() {
                for c in &t.items {
                    if let Some(o) = c.origin.as_deref()
                        && home.get(o).is_some_and(|h| *h != ti)
                    {
                        out.insert((uid.clone(), kind == TrackKind::Video, o.to_string()), ti);
                    }
                }
            }
        }
    }
    out
}

/// Unresolved object references and binary payloads of a document (empty when it is sound).
pub(crate) fn problems(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(doc) = crate::xml::parse(text) else {
        out.insert("unparsable".to_string());
        return out;
    };
    let root = doc.root_element();
    let ix = Index::new(root);
    for obj in root.children().filter(Node::is_element) {
        for r in public_refs(obj) {
            match (r.attribute("ObjectRef"), r.attribute("ObjectURef")) {
                (Some(id), _) if !ix.ids.contains_key(id) => {
                    out.insert(format!("ObjectRef {id}"));
                }
                (None, Some(uid)) if !ix.uids.contains_key(uid) => {
                    out.insert(format!("ObjectURef {uid}"));
                }
                _ => {}
            }
        }
    }
    let mut payloads = HashSet::new();
    let mut wanted = HashSet::new();
    for n in root.descendants().filter(Node::is_element) {
        if let Some(h) = n.attribute("BinaryHash") {
            if n.text().is_some_and(|t| !t.trim().is_empty()) {
                payloads.insert(h);
            } else {
                wanted.insert(h);
            }
        }
    }
    for h in wanted.difference(&payloads) {
        out.insert(format!("BinaryHash {h}"));
    }
    out
}

fn verify(out: &Output, written: &[(ItemId, String, Vec<WrittenTrack>)], known: &HashSet<String>) -> Result<()> {
    let doc = crate::xml::parse(&out.text).map_err(|e| err(format!("the written project does not parse: {e}")))?;
    let root = doc.root_element();
    // unique ids
    let mut seen = HashSet::new();
    for n in root.children().filter(Node::is_element) {
        for a in ["ObjectID", "ObjectUID"] {
            if let Some(v) = n.attribute(a)
                && !seen.insert((a, v))
            {
                return Err(err(format!("duplicate {a} {v} in the written project")));
            }
        }
    }
    // references and binary payloads: nothing the original did not already lack
    if let Some(p) = problems(&out.text).difference(known).next() {
        return Err(err(format!("the written project would have an unresolved {p}")));
    }
    // read it back: every written track must come back exactly
    let mut report = Report::default();
    let back = crate::prproj::import(out.text.as_bytes(), &ImportOptions::default(), &mut report)?;
    for (_, uid, tracks) in written {
        let Some(seq) = back.project.items.values().find(|i| i.metadata.get(META_SEQUENCE) == Some(uid)).and_then(|i| i.as_sequence()) else {
            return Err(err("a written sequence does not read back"));
        };
        // per kind: Premiere allows overlapping clips on a track, which read back on extra tracks
        for kind in [TrackKind::Video, TrackKind::Audio] {
            if !tracks.iter().any(|(k, _, _)| *k == kind) {
                continue;
            }
            let key = |c: &ExportedClip| (c.start, c.duration, c.source_in);
            let mut want: Vec<ExportedClip> = tracks.iter().filter(|(k, _, _)| *k == kind).flat_map(|(_, _, v)| v.iter().cloned()).collect();
            let mut got: Vec<ExportedClip> = seq.tracks(kind).iter().flat_map(|t| t.items.iter().map(ExportedClip::of)).collect();
            want.sort_by_key(key);
            got.sort_by_key(key);
            if got.len() != want.len() || got.iter().zip(&want).any(|(g, w)| !same_clip(g, w)) {
                let i = got.iter().zip(&want).position(|(g, w)| !same_clip(g, w)).unwrap_or(got.len().min(want.len()));
                return Err(err(format!("{} clips read back differently (clip {}: wrote {:?}, read {:?})", kind_name(kind), i + 1, want.get(i), got.get(i))));
            }
        }
    }
    Ok(())
}

fn kind_name(k: TrackKind) -> &'static str {
    if k == TrackKind::Video { "video" } else { "audio" }
}

fn default_track_name(kind: TrackKind, i: usize) -> String {
    format!("{} {}", if kind == TrackKind::Video { "Video" } else { "Audio" }, i + 1)
}

fn same_clip(a: &ExportedClip, b: &ExportedClip) -> bool {
    a.start == b.start
        && a.duration == b.duration
        && a.source_in == b.source_in
        && (a.speed - b.speed).abs() < 1e-9
        && a.reverse == b.reverse
        && a.frame_hold == b.frame_hold
}

#[cfg(test)]
#[path = "prproj_write_tests.rs"]
mod tests;
