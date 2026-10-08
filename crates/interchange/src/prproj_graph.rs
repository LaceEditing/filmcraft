//! The object graph of a Premiere Pro project, shared by the reader ([`crate::prproj`]) and the
//! writer ([`crate::prproj_write`]): gzip framing, the object index and reference resolution.
//! See `prproj.rs` and the crate README for what is known about the format.

use std::collections::HashMap;

use roxmltree::Node;

use crate::xml::child;
use crate::{Error, Format, Result};

/// Premiere ticks per second (identical to FilmCraft's `Tick` resolution).
pub(crate) const TICKS_PER_SECOND: i64 = 254_016_000_000;
/// Inflated documents larger than this are refused (a 2-hour, 5 000-clip project is ~100 MB).
const MAX_XML_BYTES: usize = 1 << 30;

fn bad(m: &str) -> Error {
    Error::parse(Format::Prproj, m.to_string())
}

/// Undo the gzip wrapper (RFC 1952) if there is one. Returns the XML and whether it was compressed.
pub(crate) fn decompress(bytes: &[u8]) -> Result<(Vec<u8>, bool)> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok((bytes.to_vec(), false));
    }
    let header = bytes.get(..10).ok_or_else(|| bad("truncated gzip header"))?;
    if header.get(2) != Some(&8) {
        return Err(bad("unsupported gzip compression method"));
    }
    let flags = header.get(3).copied().unwrap_or(0);
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        // FEXTRA: two-byte little-endian length, then the field
        let len = bytes.get(pos..pos.saturating_add(2)).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| bad("truncated gzip extra field"))?;
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
    let xml = miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_XML_BYTES)
        .map_err(|e| bad(&format!("gzip data is damaged or too large ({:?})", e.status)))?;
    Ok((xml, true))
}

/// Wrap `data` in a gzip member (RFC 1952: no file name, no mtime, CRC-32 and size trailer).
pub(crate) fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 8 + 32);
    out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // ISIZE is the length modulo 2^32
    out
}

/// CRC-32 (ISO 3309 / ITU-T V.42, reflected, polynomial 0xEDB88320), as gzip uses it.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// A stable fingerprint of a document's bytes (FNV-1a 64), to notice that the source project was
/// changed after it was imported.
pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}-{}", bytes.len())
}

/// The root's direct children by `ObjectID` / `ObjectUID`. Only these are addressable: nested
/// elements (view state) reuse ids in their own namespaces.
pub(crate) struct Index<'a, 'i> {
    pub ids: HashMap<&'a str, Node<'a, 'i>>,
    pub uids: HashMap<&'a str, Node<'a, 'i>>,
}

impl<'a, 'i> Index<'a, 'i> {
    pub fn new(root: Node<'a, 'i>) -> Self {
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
        Self { ids, uids }
    }

    /// The object an `ObjectRef` / `ObjectURef` element points at.
    pub fn resolve(&self, r: Option<Node<'a, 'i>>) -> Option<Node<'a, 'i>> {
        let r = r?;
        if let Some(id) = r.attribute("ObjectRef") {
            return self.ids.get(id).copied();
        }
        r.attribute("ObjectURef").and_then(|u| self.uids.get(u).copied())
    }

    /// Walk child elements by name.
    pub fn at(&self, n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
        let mut cur = n;
        for p in path {
            cur = child(cur, p)?;
        }
        Some(cur)
    }

    /// Walk to a reference element and resolve it.
    pub fn follow(&self, n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
        self.resolve(self.at(n, path))
    }

    /// The referenced objects of the `tag` children of the element at `path`.
    pub fn list(&self, n: Node<'a, 'i>, path: &[&str], tag: &str) -> Vec<Node<'a, 'i>> {
        self.at(n, path).into_iter().flat_map(|p| p.children().filter(move |c| c.has_tag_name(tag))).filter_map(|r| self.resolve(Some(r))).collect()
    }
}

/// The video or audio track group of a sequence.
pub(crate) fn track_group<'a, 'i>(ix: &Index<'a, 'i>, seq: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    let groups = ix.at(seq, &["TrackGroups"])?;
    groups.children().filter(|n| n.has_tag_name("TrackGroup")).filter_map(|g| ix.follow(g, &["Second"])).find(|g| g.has_tag_name(tag))
}

/// The tracks of a track group in `Index` order.
pub(crate) fn tracks<'a, 'i>(ix: &Index<'a, 'i>, group: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    let Some(list) = ix.at(group, &["TrackGroup", "Tracks"]) else { return Vec::new() };
    let mut refs: Vec<(i64, Node)> = list
        .children()
        .filter(|n| n.has_tag_name("Track"))
        .map(|n| (n.attribute("Index").and_then(|i| i.trim().parse().ok()).unwrap_or(i64::MAX), n))
        .collect();
    refs.sort_by_key(|(i, _)| *i);
    refs.into_iter().filter_map(|(_, r)| ix.resolve(Some(r))).collect()
}

/// The media file behind a clip object (`VideoClip` / `AudioClip`), if its source is media.
pub(crate) fn clip_media<'a, 'i>(ix: &Index<'a, 'i>, clip: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    let src = ix.follow(clip, &["Clip", "Source"])?;
    ix.follow(src, &["MediaSource", "Media"])
}

/// The sequence behind a clip object, if its source is a (nested) sequence.
pub(crate) fn clip_sequence<'a, 'i>(ix: &Index<'a, 'i>, clip: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    let src = ix.follow(clip, &["Clip", "Source"])?;
    ix.follow(src, &["SequenceSource", "Sequence"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_gzip_round_trip() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let data = b"<PremiereData Version=\"3\"/>".repeat(100);
        let gz = compress(&data);
        assert_eq!(decompress(&gz).unwrap(), (data.clone(), true));
        assert_eq!(&gz[gz.len() - 8..gz.len() - 4], &crc32(&data).to_le_bytes());
    }
}
