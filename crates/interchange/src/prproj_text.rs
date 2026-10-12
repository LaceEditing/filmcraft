//! Premiere Pro titles: the value of a text layer's `Source Text` parameter (component
//! `AE.ADBE Text`), read from project files Premiere saved; no application internals were
//! inspected. The value is base64 in `StartKeyframeValue`: an 8-byte length, the marker
//! `44 33 22 11`, then a FlatBuffers buffer (root table → field 0: the text document).
//!
//! Observed fields of the text document (field numbers are vtable slots), checked against a
//! title in a project and the same frame of the video Premiere exported from it:
//! - 0: the text runs, each a table of (0) its text and (1) its style;
//! - 1: the font PostScript names used (`FatFrank-Heavy`);
//! - 2, 3: the text box width and height in pixels; 4: 2 for paragraph (area) text;
//! - 5: paragraph alignment (absent / 0 left, 1 centre, 2 right).
//!
//! Style: 1 font size in pixels (absent: 100); 2 fill colour, a table of red / green / blue bytes
//! that are 255 unless stored (absent: white); 4 stroke colour (same layout); 5 stroke on (1);
//! 6 stroke width in pixels. The layer's origin (Position, Anchor Point) is the top-left corner of
//! the text's bounds. Shadows, backgrounds, gradients and per-character styles are not read.

/// A decoded Premiere title.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Title {
    pub text: String,
    /// PostScript name of the first font (`FatFrank-Heavy`).
    pub font: Option<String>,
    pub size: f32,
    pub fill: [u8; 3],
    /// Stroke colour and width.
    pub stroke: Option<([u8; 3], f32)>,
    /// Paragraph text box (width, height); None for point text.
    pub area: Option<(f32, f32)>,
    /// 0 left, 1 centre, 2 right.
    pub align: u32,
}

/// Bounds-checked FlatBuffers reads over a buffer of untrusted bytes.
struct Fb<'a> {
    b: &'a [u8],
}

impl Fb<'_> {
    fn bytes<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.b.get(at..at.checked_add(N)?)?.try_into().ok()
    }
    fn u8(&self, at: usize) -> Option<u8> {
        self.b.get(at).copied()
    }
    fn u16(&self, at: usize) -> Option<u16> {
        self.bytes::<2>(at).map(u16::from_le_bytes)
    }
    fn u32(&self, at: usize) -> Option<u32> {
        self.bytes::<4>(at).map(u32::from_le_bytes)
    }
    fn i32(&self, at: usize) -> Option<i32> {
        self.bytes::<4>(at).map(i32::from_le_bytes)
    }
    fn f32(&self, at: usize) -> Option<f32> {
        self.bytes::<4>(at).map(f32::from_le_bytes).filter(|v| v.is_finite())
    }
    /// The position an offset stored at `at` points to.
    fn deref(&self, at: usize) -> Option<usize> {
        at.checked_add(self.u32(at)? as usize).filter(|p| *p < self.b.len())
    }
    /// Where field `slot` of the table at `t` is stored (None: absent).
    fn field(&self, t: usize, slot: usize) -> Option<usize> {
        let vt = (t as i64).checked_sub(self.i32(t)? as i64)?;
        let vt = usize::try_from(vt).ok()?;
        let vsize = self.u16(vt)? as usize;
        let entry = 4 + 2 * slot;
        if entry + 2 > vsize {
            return None;
        }
        let off = self.u16(vt + entry)? as usize;
        (off != 0).then(|| t + off)
    }
    fn table(&self, t: usize, slot: usize) -> Option<usize> {
        self.deref(self.field(t, slot)?)
    }
    fn string_at(&self, at: usize) -> Option<String> {
        let s = self.deref(at)?;
        let len = self.u32(s)? as usize;
        let bytes = self.b.get(s.checked_add(4)?..s.checked_add(4)?.checked_add(len)?)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
    /// The element positions of a vector of offsets (at most 10 000).
    fn vector(&self, t: usize, slot: usize) -> Vec<usize> {
        let Some(v) = self.field(t, slot).and_then(|f| self.deref(f)) else { return Vec::new() };
        let n = self.u32(v).unwrap_or(0).min(10_000) as usize;
        (0..n).map(|i| v + 4 + 4 * i).take_while(|p| p + 4 <= self.b.len()).collect()
    }
    /// An RGB table whose channels are 255 unless stored.
    fn rgb(&self, t: usize) -> [u8; 3] {
        [0, 1, 2].map(|i| self.field(t, i).and_then(|f| self.u8(f)).unwrap_or(255))
    }
}

/// Decode a `Source Text` value (the bytes behind its base64).
pub(crate) fn decode(bytes: &[u8]) -> Option<Title> {
    if bytes.get(8..12)? != [0x44, 0x33, 0x22, 0x11] {
        return None;
    }
    let fb = Fb { b: bytes.get(12..)? };
    let root = fb.deref(0)?;
    let doc = fb.table(root, 0)?;
    let runs = fb.vector(doc, 0);
    let mut text = String::new();
    let mut style = None;
    for r in &runs {
        let Some(run) = fb.deref(*r) else { continue };
        if let Some(s) = fb.field(run, 0).and_then(|f| fb.string_at(f)) {
            text.push_str(&s);
        }
        if style.is_none() {
            style = fb.table(run, 1);
        }
    }
    let font = fb.vector(doc, 1).first().and_then(|p| fb.string_at(*p));
    let mut size = 100.0;
    let mut fill = [255; 3];
    let mut stroke = None;
    if let Some(st) = style {
        if let Some(v) = fb.field(st, 1).and_then(|f| fb.f32(f)) {
            size = v.clamp(0.1, 100_000.0);
        }
        if let Some(t) = fb.table(st, 2) {
            fill = fb.rgb(t);
        }
        let on = fb.field(st, 5).and_then(|f| fb.u8(f)) == Some(1);
        if on {
            let colour = fb.table(st, 4).map(|t| fb.rgb(t)).unwrap_or([255; 3]);
            let width = fb.field(st, 6).and_then(|f| fb.f32(f)).unwrap_or(0.0).clamp(0.0, 10_000.0);
            if width > 0.0 {
                stroke = Some((colour, width));
            }
        }
    }
    let dim = |slot| fb.field(doc, slot).and_then(|f| fb.f32(f)).filter(|v| *v > 0.0 && *v < 100_000.0);
    let area = match (fb.field(doc, 4).and_then(|f| fb.u32(f)), dim(2), dim(3)) {
        (Some(2), Some(w), Some(h)) => Some((w, h)),
        _ => None,
    };
    let align = fb.field(doc, 5).and_then(|f| fb.u32(f)).filter(|a| *a <= 2).unwrap_or(0);
    Some(Title { text, font, size, fill, stroke, area, align })
}

/// Base64 (standard alphabet; whitespace skipped, `=` ends the data) → bytes. None on any other
/// character.
pub(crate) fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Bytes → base64 (standard alphabet, `=` padding).
pub(crate) fn base64_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A PostScript font name as (family, style): `FatFrank-Heavy` → (`FatFrank`, `Heavy`),
/// `Lato-BoldItalic` → (`Lato`, `Bold Italic`), `Impact` → (`Impact`, `Regular`).
pub(crate) fn font_family_style(ps: &str) -> (String, String) {
    let (family, style) = match ps.rsplit_once('-') {
        Some((f, s)) if !f.is_empty() && !s.is_empty() => (f, s),
        _ => (ps, "Regular"),
    };
    // `BoldItalic` → `Bold Italic`
    let mut spaced = String::new();
    for (i, c) in style.chars().enumerate() {
        if i > 0 && c.is_uppercase() && !spaced.ends_with(' ') {
            spaced.push(' ');
        }
        spaced.push(c);
    }
    (family.to_string(), spaced)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `Source Text` value laid out like Premiere's for "50" (white fill, black 26 px stroke,
    /// FatFrank-Heavy at 399.41 px, centred in a 2125 × 539 box), built by hand.
    pub(crate) fn fifty() -> Vec<u8> {
        // FlatBuffer built back to front is tedious by hand; this one is assembled forwards with
        // absolute offsets and checked by the decoder test below.
        let mut b: Vec<u8> = Vec::new();
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b.resize(400, 0);
        // root offset → root table at 16 (vtable at 8: size 6, table 8, field 0 at +4)
        put32(&mut b, 0, 16);
        b[8..14].copy_from_slice(&[6, 0, 8, 0, 4, 0]);
        put32(&mut b, 16, 8); // soffset to vtable (16 - 8)
        put32(&mut b, 20, 60 - 20); // field 0 → doc table at 60
        // doc vtable at 24: slots 0..5 (size 4 + 2*6 = 16, table 28)
        let vt = [16u16, 28, 4, 8, 12, 16, 20, 24];
        for (i, v) in vt.iter().enumerate() {
            b[24 + 2 * i..26 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        put32(&mut b, 60, 60 - 24); // doc → its vtable
        put32(&mut b, 64, 120 - 64); // runs vector at 120
        put32(&mut b, 68, 200 - 68); // fonts vector at 200
        b[72..76].copy_from_slice(&2125f32.to_le_bytes());
        b[76..80].copy_from_slice(&539f32.to_le_bytes());
        put32(&mut b, 80, 2); // area text
        put32(&mut b, 84, 1); // centred
        // runs vector: 1 element → run table at 140
        put32(&mut b, 120, 1);
        put32(&mut b, 124, 140 - 124);
        // run vtable at 128: size 8, table 12, slots 0 (+4), 1 (+8)
        b[128..136].copy_from_slice(&[8, 0, 12, 0, 4, 0, 8, 0]);
        put32(&mut b, 140, 12); // → vtable at 128
        put32(&mut b, 144, 240 - 144); // text string at 240
        put32(&mut b, 148, 260 - 148); // style table at 260
        // fonts vector at 200: 1 string at 216
        put32(&mut b, 200, 1);
        put32(&mut b, 204, 216 - 204);
        put32(&mut b, 216, 14);
        b[220..234].copy_from_slice(b"FatFrank-Heavy");
        put32(&mut b, 240, 2);
        b[244..246].copy_from_slice(b"50");
        // style vtable at 300: slots 0..6 (size 18): 1 size +4, 4 stroke colour +8, 5 on +12, 6 width +16
        let svt = [18u16, 20, 0, 4, 0, 0, 8, 12, 16];
        for (i, v) in svt.iter().enumerate() {
            b[300 + 2 * i..302 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        put32(&mut b, 260, (260i32 - 300) as u32); // negative soffset: vtable after the table
        b[264..268].copy_from_slice(&399.41f32.to_le_bytes());
        put32(&mut b, 268, 340 - 268); // stroke colour table at 340
        b[272] = 1;
        b[276..280].copy_from_slice(&26f32.to_le_bytes());
        // colour table at 340, vtable at 330 (size 10, table 8, slots r +4, g +5, b +6): black
        b[330..340].copy_from_slice(&[10, 0, 8, 0, 4, 0, 5, 0, 6, 0]);
        put32(&mut b, 340, 10);
        // the 12-byte header
        let mut out = (b.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&[0x44, 0x33, 0x22, 0x11]);
        out.extend_from_slice(&b);
        out
    }

    #[test]
    fn a_premiere_title_is_decoded() {
        let t = decode(&fifty()).expect("decodes");
        assert_eq!(t.text, "50");
        assert_eq!(t.font.as_deref(), Some("FatFrank-Heavy"));
        assert!((t.size - 399.41).abs() < 1e-3);
        assert_eq!(t.fill, [255, 255, 255], "no fill stored: white");
        assert_eq!(t.stroke, Some(([0, 0, 0], 26.0)));
        assert_eq!((t.area, t.align), (Some((2125.0, 539.0)), 1));
    }

    #[test]
    fn damaged_values_never_panic() {
        let good = fifty();
        assert!(decode(&[]).is_none());
        assert!(decode(&good[..11]).is_none());
        for cut in 12..good.len() {
            let _ = decode(&good[..cut]);
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let mut b = good.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = 12 + (seed as usize) % (b.len() - 12);
                b[i] = (seed >> 32) as u8;
            }
            let _ = decode(&b);
        }
    }

    #[test]
    fn base64_round_trips() {
        for b in [&b""[..], b"O77\0\x01\0\0\0", b"\x01\0", b"abcd"] {
            assert_eq!(base64(&base64_encode(b)).unwrap(), b);
        }
        assert_eq!(base64_encode(&[0x4f, 0x37, 0x37, 0, 1, 0, 0, 0]), "Tzc3AAEAAAA=", "as Premiere writes a colour matte's");
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("AQA=").as_deref(), Some(&[1u8, 0][..]));
        assert_eq!(base64("SGVs\nbG8=").as_deref(), Some(&b"Hello"[..]));
        assert!(base64("not*base64").is_none());
    }

    #[test]
    fn postscript_names_split_into_family_and_style() {
        assert_eq!(font_family_style("FatFrank-Heavy"), ("FatFrank".into(), "Heavy".into()));
        assert_eq!(font_family_style("Lato-BoldItalic"), ("Lato".into(), "Bold Italic".into()));
        assert_eq!(font_family_style("Impact"), ("Impact".into(), "Regular".into()));
    }
}
