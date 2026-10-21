//! Premiere Pro titles, read and written: the `Source Text` value of a text layer (component
//! `AE.ADBE Text`). The value is an 8-byte length, the marker `44 33 22 11`, then a FlatBuffers
//! buffer. Its layout was worked out from projects Premiere 26.5 saved (a "title kit": one title
//! per setting, each changed alone from a plain one, with the values typed recorded), not from
//! the application.
//!
//! Tables and their fields (vtable slots):
//! - root: 0 the document; 1, 4 small flags on template titles.
//! - document: 0 the text runs; 1 the fonts' PostScript names (runs refer to them by index); 2, 3
//!   the paragraph box's width and height (point text has none); 4 alignment (absent left, 1
//!   right, 2 centre); 6 leading; 9 vertical text; shadow: 10 colour, 11 on, 12 opacity (absent
//!   75), 13 angle (135), 14 distance (7), 15 size (0), 16 blur (40); background: 17 colour, 18 on,
//!   19 opacity (75), 20 size (0), 34 corner radius (0); 30 the number of strokes; 40 the
//!   shadow's gradient; 26, 43, 44 flags written on every title.
//! - run: 0 its text (`\r` between lines); 1 its style.
//! - style: 0 font index; 1 size (absent 100); 2 fill colour (absent white); 4 stroke colour; 5
//!   stroke on; 6 stroke width (written 4 on every run); 7 kerning after the run; 8 tracking; 10
//!   baseline shift; 12 caps (1 small, 2 all); 14 faux bold; 15 faux italic; 16 underline; 17
//!   further strokes; 20 fill type (1 linear gradient); 21 fill gradient; 23 stroke gradient; 24
//!   (2 on every run).
//! - colour: 0 red, 1 green, 2 blue bytes (a fill's channels are 255 when absent).
//! - gradient: 1 colour stops (0 colour, 1 position, 2 midpoint); 2 opacity stops.
//! - stroke (further strokes): 0 colour, 1 on, 2 width, 4 gradient, 5 position.
//!
//! [`Tree`] reads a value into these typed tables, keeping fields it does not know when they are
//! plain numbers (a field whose meaning is unknown and that holds a reference is dropped and
//! counted in [`Tree::dropped`]), and writes any tree back as a valid buffer.

use std::collections::BTreeMap;

use filmcraft_project::{EffectInstance, Param, ParamValue};

/// A table: field slot → value.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tab {
    pub f: BTreeMap<u16, Fv>,
}

/// A field value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fv {
    U8(u8),
    U16(u16),
    U32(u32),
    F32(f32),
    Str(String),
    Tab(Tab),
    Tabs(Vec<Tab>),
    Strs(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum T {
    Root,
    Doc,
    Run,
    Style,
    Rgb,
    Grad,
    Stop,
    Stroke,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    U8,
    U32,
    F32,
    Str,
    Tab(T),
    Tabs(T),
    Strs,
}

/// What each known field holds.
fn kind(t: T, slot: u16) -> Option<Kind> {
    use Kind::*;
    Some(match (t, slot) {
        (T::Root, 0) => Tab(T::Doc),
        (T::Root, 1 | 4) => U8,
        (T::Doc, 0) => Tabs(T::Run),
        (T::Doc, 1) => Strs,
        (T::Doc, 2 | 3 | 6 | 12 | 13 | 14 | 15 | 16 | 19 | 20 | 34) => F32,
        (T::Doc, 4 | 5 | 30) => U32,
        (T::Doc, 9 | 11 | 18 | 26 | 38 | 43 | 44) => U8,
        (T::Doc, 10 | 17) => Tab(T::Rgb),
        (T::Doc, 40) => Tab(T::Grad),
        (T::Run, 0) => Str,
        (T::Run, 1) => Tab(T::Style),
        (T::Style, 0 | 12 | 20 | 24) => U32,
        (T::Style, 1 | 6 | 7 | 8 | 10) => F32,
        (T::Style, 2 | 4) => Tab(T::Rgb),
        (T::Style, 5 | 14 | 15 | 16) => U8,
        (T::Style, 17) => Tabs(T::Stroke),
        (T::Style, 21 | 23) => Tab(T::Grad),
        (T::Rgb, 0..=2) => U8,
        (T::Grad, 1 | 2) => Tabs(T::Stop),
        (T::Stop, 0) => Tab(T::Rgb),
        (T::Stop, 1 | 2) => F32,
        (T::Stroke, 0) => Tab(T::Rgb),
        (T::Stroke, 1) => U8,
        (T::Stroke, 2) => F32,
        (T::Stroke, 4) => Tab(T::Grad),
        (T::Stroke, 5) => U32,
        _ => return None,
    })
}

/// At most this many runs, strokes, stops or fonts in a vector, and this much nesting.
const MAX_VEC: usize = 10_000;
const MAX_DEPTH: usize = 16;

/// Bounds-checked reads over untrusted bytes.
struct Rd<'a> {
    b: &'a [u8],
    dropped: usize,
}

impl Rd<'_> {
    fn bytes<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.b.get(at..at.checked_add(N)?)?.try_into().ok()
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
    fn deref(&self, at: usize) -> Option<usize> {
        at.checked_add(self.u32(at)? as usize).filter(|p| *p < self.b.len())
    }
    /// The fields of the table at `t`: slot → (position, width up to the next field).
    fn fields(&self, t: usize) -> Option<Vec<(u16, usize, usize)>> {
        let vt = usize::try_from((t as i64).checked_sub(i64::from(self.i32(t)?))?).ok()?;
        let vsize = self.u16(vt)? as usize;
        let tsize = self.u16(vt + 2)? as usize;
        if vsize < 4 || tsize < 4 {
            return None;
        }
        let mut present: Vec<(usize, u16)> = Vec::new();
        for i in 0..(vsize - 4) / 2 {
            let off = self.u16(vt + 4 + 2 * i)? as usize;
            if off != 0 && off < tsize {
                present.push((off, i as u16));
            }
        }
        present.sort_unstable();
        let mut out = Vec::new();
        for (k, (off, slot)) in present.iter().enumerate() {
            let next = present.get(k + 1).map_or(tsize, |n| n.0);
            out.push((*slot, t + off, next.saturating_sub(*off)));
        }
        Some(out)
    }
    fn string(&self, at: usize) -> Option<String> {
        let s = self.deref(at)?;
        let len = self.u32(s)? as usize;
        let start = s.checked_add(4)?;
        Some(String::from_utf8_lossy(self.b.get(start..start.checked_add(len)?)?).into_owned())
    }
    fn vector(&self, at: usize) -> Option<Vec<usize>> {
        let v = self.deref(at)?;
        let n = (self.u32(v)? as usize).min(MAX_VEC);
        let out: Vec<usize> = (0..n).map(|i| v + 4 + 4 * i).take_while(|p| p + 4 <= self.b.len()).collect();
        (out.len() == n).then_some(out)
    }
    /// Whether the 4 bytes at `at` look like a reference to a table, vector or string (so an
    /// unknown field holding it cannot be kept as a plain number).
    fn looks_like_ref(&self, at: usize) -> bool {
        let Some(p) = self.deref(at) else { return false };
        p > at && p + 4 <= self.b.len()
    }
    fn table(&mut self, t: usize, ty: T, depth: usize) -> Option<Tab> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut tab = Tab::default();
        for (slot, at, width) in self.fields(t)? {
            let v = match kind(ty, slot) {
                Some(Kind::U8) => self.b.get(at).copied().map(Fv::U8),
                Some(Kind::U32) => self.u32(at).map(Fv::U32),
                Some(Kind::F32) => self.bytes::<4>(at).map(f32::from_le_bytes).filter(|v| v.is_finite()).map(Fv::F32),
                Some(Kind::Str) => self.string(at).map(Fv::Str),
                Some(Kind::Strs) => self.vector(at).map(|v| Fv::Strs(v.into_iter().filter_map(|e| self.string(e)).collect())),
                Some(Kind::Tab(c)) => self.deref(at).and_then(|p| self.table(p, c, depth + 1)).map(Fv::Tab),
                Some(Kind::Tabs(c)) => self.vector(at).map(|v| {
                    let mut out = Vec::new();
                    for e in v {
                        if let Some(t) = self.deref(e).and_then(|p| self.table(p, c, depth + 1)) {
                            out.push(t);
                        }
                    }
                    Fv::Tabs(out)
                }),
                // a field FilmCraft does not know: kept when it is a plain number
                None => match width {
                    1 => self.b.get(at).copied().map(Fv::U8),
                    2 | 3 => self.u16(at).map(Fv::U16),
                    _ if self.looks_like_ref(at) => {
                        self.dropped += 1;
                        None
                    }
                    _ => self.u32(at).map(Fv::U32),
                },
            };
            if let Some(v) = v {
                tab.f.insert(slot, v);
            }
        }
        Some(tab)
    }
}

/// A decoded `Source Text` value: the document and the root's other fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tree {
    pub root: Tab,
    pub doc: Tab,
    /// Unknown fields holding references, which could not be kept.
    pub dropped: usize,
}

const MARKER: [u8; 4] = [0x44, 0x33, 0x22, 0x11];

impl Tree {
    /// Read a `Source Text` value (the bytes behind its base64).
    pub fn read(bytes: &[u8]) -> Option<Tree> {
        if bytes.get(8..12)? != MARKER {
            return None;
        }
        let b = bytes.get(12..)?;
        let mut rd = Rd { b, dropped: 0 };
        let root_at = rd.deref(0)?;
        let mut root = rd.table(root_at, T::Root, 0)?;
        let Some(Fv::Tab(doc)) = root.f.remove(&0) else { return None };
        Some(Tree { root, doc, dropped: rd.dropped })
    }

    /// The value's bytes: header and buffer.
    pub fn write(&self) -> Vec<u8> {
        let mut root = self.root.clone();
        root.f.insert(0, Fv::Tab(self.doc.clone()));
        let mut w = Wr { b: vec![0; 4] };
        let at = w.table(&root);
        w.patch(0, at);
        let mut out = (w.b.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&MARKER);
        out.extend_from_slice(&w.b);
        out
    }

    /// A plain title as Premiere makes one (Graphics ▸ New Layer ▸ Text): `text` in `font` at 100,
    /// white, left aligned, point text.
    pub fn new_title(text: &str, font: &str) -> Tree {
        let mut run = Tab::default();
        run.f.insert(0, Fv::Str(text.to_string()));
        run.f.insert(1, Fv::Tab(new_style()));
        let mut doc = Tab::default();
        doc.f.insert(0, Fv::Tabs(vec![run]));
        doc.f.insert(1, Fv::Strs(vec![font.to_string()]));
        doc.f.insert(26, Fv::U8(1));
        doc.f.insert(40, Fv::Tab(Tab::default()));
        doc.f.insert(43, Fv::U8(0));
        doc.f.insert(44, Fv::U8(1));
        Tree { root: Tab::default(), doc, dropped: 0 }
    }
}

/// A run's style as Premiere writes it for a new title.
fn new_style() -> Tab {
    let mut st = Tab::default();
    st.f.insert(6, Fv::F32(4.0));
    st.f.insert(21, Fv::Tab(Tab::default()));
    st.f.insert(23, Fv::Tab(Tab::default()));
    st.f.insert(24, Fv::U32(2));
    st
}

fn tab(t: &Tab, slot: u16) -> Option<&Tab> {
    match t.f.get(&slot)? {
        Fv::Tab(x) => Some(x),
        _ => None,
    }
}
fn f32_of(t: &Tab, slot: u16) -> Option<f32> {
    match t.f.get(&slot)? {
        Fv::F32(v) => Some(*v),
        _ => None,
    }
}
fn u32_of(t: &Tab, slot: u16) -> Option<u32> {
    match t.f.get(&slot)? {
        Fv::U32(v) => Some(*v),
        Fv::U16(v) => Some(u32::from(*v)),
        Fv::U8(v) => Some(u32::from(*v)),
        _ => None,
    }
}
fn flag(t: &Tab, slot: u16) -> bool {
    u32_of(t, slot).is_some_and(|v| v != 0)
}
/// A colour table: channels absent are `absent` (255 for fills).
fn rgb_of(t: &Tab, absent: u8) -> [u8; 3] {
    [0u16, 1, 2].map(|i| match t.f.get(&i) {
        Some(Fv::U8(v)) => *v,
        _ => absent,
    })
}
fn rgb_tab(c: [u8; 3]) -> Tab {
    let mut t = Tab::default();
    for (i, v) in c.into_iter().enumerate() {
        t.f.insert(i as u16, Fv::U8(v));
    }
    t
}

// -------------------------------------------------------------------------------------------
// Writing a buffer
// -------------------------------------------------------------------------------------------

/// Writes tables front to back: each vtable just before its table, the table's referenced values
/// after it (references point forwards, as FlatBuffers requires).
struct Wr {
    b: Vec<u8>,
}

impl Wr {
    fn align(&mut self, n: usize) {
        while !self.b.len().is_multiple_of(n) {
            self.b.push(0);
        }
    }
    fn put(&mut self, at: usize, v: &[u8]) {
        if let Some(d) = self.b.get_mut(at..at + v.len()) {
            d.copy_from_slice(v);
        }
    }
    /// Point the reference at `at` to `target` (after it).
    fn patch(&mut self, at: usize, target: usize) {
        let d = u32::try_from(target.saturating_sub(at)).unwrap_or(0);
        self.put(at, &d.to_le_bytes());
    }
    fn string(&mut self, s: &str) -> usize {
        self.align(4);
        let at = self.b.len();
        let bytes = s.as_bytes();
        self.b.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(0).to_le_bytes());
        self.b.extend_from_slice(bytes);
        self.b.push(0);
        at
    }
    fn vector(&mut self, n: usize) -> (usize, Vec<usize>) {
        self.align(4);
        let at = self.b.len();
        self.b.extend_from_slice(&u32::try_from(n).unwrap_or(0).to_le_bytes());
        let slots: Vec<usize> = (0..n).map(|i| at + 4 + 4 * i).collect();
        self.b.resize(at + 4 + 4 * n, 0);
        (at, slots)
    }
    fn table(&mut self, t: &Tab) -> usize {
        // inline layout: 4-byte fields (numbers and references), then 2-byte, then 1-byte ones
        let width = |v: &Fv| match v {
            Fv::U8(_) => 1,
            Fv::U16(_) => 2,
            _ => 4,
        };
        let mut order: Vec<(&u16, &Fv)> = t.f.iter().collect();
        order.sort_by_key(|(s, v)| (std::cmp::Reverse(width(v)), **s));
        let mut offsets: Vec<(u16, usize)> = Vec::new();
        let mut size = 4usize;
        for (s, v) in &order {
            offsets.push((**s, size));
            size += width(v);
        }
        let table_size = size.next_multiple_of(4);
        let max_slot = t.f.keys().max().map_or(0, |m| usize::from(*m) + 1);
        // the vtable
        self.align(2);
        let vt = self.b.len();
        let vsize = 4 + 2 * max_slot;
        self.b.extend_from_slice(&u16::try_from(vsize).unwrap_or(4).to_le_bytes());
        self.b.extend_from_slice(&u16::try_from(table_size).unwrap_or(4).to_le_bytes());
        for slot in 0..max_slot {
            let off = offsets.iter().find(|(s, _)| usize::from(*s) == slot).map_or(0, |o| o.1);
            self.b.extend_from_slice(&u16::try_from(off).unwrap_or(0).to_le_bytes());
        }
        // the table
        self.align(4);
        let at = self.b.len();
        let soff = i32::try_from(at - vt).unwrap_or(0);
        self.b.extend_from_slice(&soff.to_le_bytes());
        self.b.resize(at + table_size, 0);
        let mut later: Vec<(usize, &Fv)> = Vec::new();
        for ((_, v), (_, off)) in order.iter().zip(&offsets) {
            let p = at + off;
            match v {
                Fv::U8(x) => self.put(p, &[*x]),
                Fv::U16(x) => self.put(p, &x.to_le_bytes()),
                Fv::U32(x) => self.put(p, &x.to_le_bytes()),
                Fv::F32(x) => self.put(p, &x.to_le_bytes()),
                other => later.push((p, other)),
            }
        }
        for (p, v) in later {
            let target = match v {
                Fv::Str(s) => self.string(s),
                Fv::Tab(c) => self.table(c),
                Fv::Strs(list) => {
                    let (vec, slots) = self.vector(list.len());
                    for (slot, s) in slots.into_iter().zip(list) {
                        let sp = self.string(s);
                        self.patch(slot, sp);
                    }
                    vec
                }
                Fv::Tabs(list) => {
                    let (vec, slots) = self.vector(list.len());
                    for (slot, c) in slots.into_iter().zip(list) {
                        let tp = self.table(c);
                        self.patch(slot, tp);
                    }
                    vec
                }
                _ => continue,
            };
            self.patch(p, target);
        }
        at
    }
}

// -------------------------------------------------------------------------------------------
// Titles ↔ FilmCraft text layers
// -------------------------------------------------------------------------------------------

/// A PostScript font name as (family, style): an installed face's own names when there is one,
/// else read from the name (`Arial-BoldMT` → `Arial`, `Bold`; `TimesNewRomanPSMT` → `Times New
/// Roman`, `Regular`).
pub(crate) fn font_names(ps: &str) -> (String, String) {
    if let Some(n) = filmcraft_text::fonts::by_postscript(ps) {
        return n;
    }
    let (family, style) = match ps.split_once('-') {
        Some((f, s)) if !f.is_empty() && !s.is_empty() => (f, s),
        _ => (ps, "Regular"),
    };
    let strip = |s: &str| {
        let s = s.strip_suffix("PSMT").or_else(|| s.strip_suffix("MT")).or_else(|| s.strip_suffix("PS")).unwrap_or(s);
        s.to_string()
    };
    // `TimesNewRoman` → `Times New Roman`, `BoldItalic` → `Bold Italic`
    let spaced = |s: &str| {
        let mut out = String::new();
        let chars: Vec<char> = s.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            let prev_lower = i > 0 && chars.get(i - 1).is_some_and(|p| p.is_lowercase());
            if i > 0 && c.is_uppercase() && prev_lower {
                out.push(' ');
            }
            out.push(*c);
        }
        out
    };
    let style = spaced(&strip(style));
    // the family as installed (`TimesNewRoman` → `Times New Roman`), else as the name has it
    let squashed = strip(family);
    let installed = filmcraft_text::fonts::families()
        .into_iter()
        .map(|(f, _)| f)
        .find(|f| f.chars().filter(|c| !c.is_whitespace()).collect::<String>().eq_ignore_ascii_case(&squashed));
    (installed.unwrap_or(squashed), if style.is_empty() { "Regular".into() } else { style })
}

/// The PostScript name to save for a family and style: the installed face's, else one made from
/// the names (`Arial`, `Bold` → `Arial-Bold`).
pub(crate) fn postscript_for(family: &str, style: &str) -> String {
    if let Some(ps) = filmcraft_text::fonts::postscript_name(family, style) {
        return ps;
    }
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    if style.is_empty() || style.eq_ignore_ascii_case("Regular") { squash(family) } else { format!("{}-{}", squash(family), squash(style)) }
}

const ALIGN_LEFT: u32 = 0;

/// Premiere's alignment → FilmCraft's (`ALIGN_OPTS`: left, center, right, justify).
fn align_in(v: u32) -> u32 {
    match v {
        2 => 1,
        1 => 2,
        _ => ALIGN_LEFT,
    }
}
fn align_out(v: u32) -> Option<u32> {
    match v {
        1 => Some(2),
        2 => Some(1),
        _ => None,
    }
}
/// Premiere's caps (1 small, 2 all) ↔ FilmCraft's (`CAPS_OPTS`: normal, all, small).
fn caps_swap(v: u32) -> u32 {
    match v {
        1 => 2,
        2 => 1,
        _ => 0,
    }
}

fn color(c: [u8; 3]) -> ParamValue {
    ParamValue::Color([f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0])
}
fn bytes_of(v: &ParamValue) -> Option<[u8; 3]> {
    match v {
        ParamValue::Color(c) => Some([c[0], c[1], c[2]].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)),
        _ => None,
    }
}

/// What a title holds that a FilmCraft text layer cannot (reported when importing; lost if the
/// title is written back from FilmCraft).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Unread {
    pub gradient: bool,
    pub kerning: bool,
    pub more_strokes: bool,
}

/// A title as a FilmCraft text layer's parameters (into `e`, a fresh `graphic_text` instance) and
/// its per-character styles.
pub(crate) fn to_layer(tree: &Tree, e: &mut EffectInstance) -> Unread {
    let mut unread = Unread::default();
    let doc = &tree.doc;
    let fonts: Vec<String> = match doc.f.get(&1) {
        Some(Fv::Strs(v)) => v.clone(),
        _ => Vec::new(),
    };
    let runs: Vec<&Tab> = match doc.f.get(&0) {
        Some(Fv::Tabs(v)) => v.iter().collect(),
        _ => Vec::new(),
    };
    let mut set = |id: &str, v: ParamValue| {
        e.params.insert(id.to_string(), Param::new(v));
    };
    // the text, with FilmCraft's line breaks
    let mut text = String::new();
    let mut spans: Vec<(usize, usize, Option<&Tab>)> = Vec::new();
    for r in &runs {
        let t = match r.f.get(&0) {
            Some(Fv::Str(s)) => s.replace("\r\n", "\n").replace('\r', "\n"),
            _ => String::new(),
        };
        let start = text.chars().count();
        text.push_str(&t);
        spans.push((start, text.chars().count(), tab(r, 1)));
    }
    set("text", ParamValue::Text(text));
    let empty = Tab::default();
    let base = spans.first().and_then(|s| s.2).unwrap_or(&empty);
    // the layer's formatting: the first run's
    let font_of = |st: &Tab| fonts.get(u32_of(st, 0).unwrap_or(0) as usize).map(|ps| font_names(ps));
    if let Some((family, style)) = font_of(base) {
        set("font", ParamValue::Text(family));
        set("font_style", ParamValue::Text(style));
    }
    set("size", ParamValue::Float(f64::from(f32_of(base, 1).unwrap_or(100.0))));
    if let Some(t) = tab(base, 2) {
        set("fill_color", color(rgb_of(t, 255)));
    }
    let stroke_on = flag(base, 5);
    set("stroke", ParamValue::Bool(stroke_on));
    if stroke_on {
        set("stroke_color", color(tab(base, 4).map_or([255; 3], |t| rgb_of(t, 255))));
        set("stroke_width", ParamValue::Float(f64::from(f32_of(base, 6).unwrap_or(4.0))));
    }
    // a second stroke
    if let Some(Fv::Tabs(more)) = base.f.get(&17) {
        if let Some(s) = more.first() {
            set("stroke2", ParamValue::Bool(flag(s, 1)));
            set("stroke2_color", color(tab(s, 0).map_or([255; 3], |t| rgb_of(t, 255))));
            set("stroke2_width", ParamValue::Float(f64::from(f32_of(s, 2).unwrap_or(4.0))));
        }
        unread.more_strokes = more.len() > 1;
    }
    if let Some(v) = f32_of(base, 8) {
        set("tracking", ParamValue::Float(f64::from(v)));
    }
    if let Some(v) = f32_of(base, 10) {
        set("baseline_shift", ParamValue::Float(f64::from(v)));
    }
    if let Some(v) = u32_of(base, 12) {
        set("caps", ParamValue::Choice(caps_swap(v)));
    }
    set("faux_bold", ParamValue::Bool(flag(base, 14)));
    set("faux_italic", ParamValue::Bool(flag(base, 15)));
    set("underline", ParamValue::Bool(flag(base, 16)));
    unread.gradient |= u32_of(base, 20).is_some_and(|v| v != 0);
    unread.kerning |= runs.iter().filter_map(|r| tab(r, 1)).any(|st| f32_of(st, 7).is_some());
    // paragraph
    set("align", ParamValue::Choice(align_in(u32_of(doc, 4).unwrap_or(0))));
    if let (Some(w), Some(h)) = (f32_of(doc, 2).filter(|w| *w > 0.0), f32_of(doc, 3)) {
        set("box_width", ParamValue::Float(f64::from(w)));
        set("box_height", ParamValue::Float(f64::from(h.max(0.0))));
        // vertical paragraph alignment (observed: 1 = centre; absent = top)
        set("valign", ParamValue::Choice(u32_of(doc, 5).filter(|v| *v <= 2).unwrap_or(0)));
    }
    if let Some(v) = f32_of(doc, 6) {
        set("leading", ParamValue::Float(f64::from(v)));
    }
    set("vertical", ParamValue::Bool(flag(doc, 9)));
    // Premiere places paragraph text by its box's top-left corner and point text by its first
    // baseline (Position at the alignment point; measured in Premiere 26.5: "Base" at 960,540
    // sits on y = 540)
    let paragraph = f32_of(doc, 2).is_some_and(|w| w > 0.0) && f32_of(doc, 3).is_some();
    set("top_left_origin", ParamValue::Bool(paragraph));
    // shadow and background, with Premiere's values for what is not stored
    if flag(doc, 11) {
        set("shadow", ParamValue::Bool(true));
        set("shadow_color", color(tab(doc, 10).map_or([0x3f; 3], |t| rgb_of(t, 255))));
        set("shadow_opacity", ParamValue::Float(f64::from(f32_of(doc, 12).unwrap_or(75.0))));
        set("shadow_angle", ParamValue::Float(f64::from(f32_of(doc, 13).unwrap_or(135.0))));
        set("shadow_distance", ParamValue::Float(f64::from(f32_of(doc, 14).unwrap_or(7.0))));
        set("shadow_size", ParamValue::Float(f64::from(f32_of(doc, 15).unwrap_or(0.0))));
        set("shadow_blur", ParamValue::Float(f64::from(f32_of(doc, 16).unwrap_or(40.0))));
    }
    if flag(doc, 18) {
        set("background", ParamValue::Bool(true));
        set("background_color", color(tab(doc, 17).map_or([0x3f; 3], |t| rgb_of(t, 255))));
        set("background_opacity", ParamValue::Float(f64::from(f32_of(doc, 19).unwrap_or(75.0))));
        set("background_size", ParamValue::Float(f64::from(f32_of(doc, 20).unwrap_or(0.0))));
        set("background_radius", ParamValue::Float(f64::from(f32_of(doc, 34).unwrap_or(0.0))));
    }
    // the other runs: per-character styles where they differ from the first
    let mut char_runs = Vec::new();
    for (start, end, st) in spans.iter().skip(1) {
        let Some(st) = st else { continue };
        let mut cs = filmcraft_project::graphic_design::CharStyle::default();
        if u32_of(st, 0).unwrap_or(0) != u32_of(base, 0).unwrap_or(0)
            && let Some((family, style)) = font_of(st)
        {
            cs.font = Some(family);
            cs.font_style = Some(style);
        }
        let size = f32_of(st, 1).unwrap_or(100.0);
        if (size - f32_of(base, 1).unwrap_or(100.0)).abs() > 1e-3 {
            cs.size = Some(size);
        }
        let fill = tab(st, 2).map_or([255; 3], |t| rgb_of(t, 255));
        if fill != tab(base, 2).map_or([255; 3], |t| rgb_of(t, 255)) {
            cs.fill = Some([f32::from(fill[0]) / 255.0, f32::from(fill[1]) / 255.0, f32::from(fill[2]) / 255.0, 1.0]);
        }
        for (slot, field) in [(14u16, &mut cs.faux_bold), (15, &mut cs.faux_italic), (16, &mut cs.underline)] {
            if flag(st, slot) != flag(base, slot) {
                *field = Some(flag(st, slot));
            }
        }
        if f32_of(st, 8) != f32_of(base, 8) {
            cs.tracking = Some(f32_of(st, 8).unwrap_or(0.0));
        }
        if f32_of(st, 10) != f32_of(base, 10) {
            cs.baseline_shift = Some(f32_of(st, 10).unwrap_or(0.0));
        }
        if u32_of(st, 12) != u32_of(base, 12) {
            cs.caps = Some(caps_swap(u32_of(st, 12).unwrap_or(0)));
        }
        if !cs.is_empty() {
            char_runs.push(filmcraft_project::graphic_design::StyleRun { start: *start, end: *end, style: cs });
        }
    }
    if !char_runs.is_empty() {
        e.layer.get_or_insert_with(Default::default).runs = char_runs;
    }
    unread
}

/// FilmCraft's text layer (its values at `t`) as a title: on `base` (the title it was imported
/// from, which keeps what FilmCraft does not hold) or a new one. `box_height` is the paragraph
/// box's height to save when the layer is paragraph text and `base` has none.
pub(crate) fn from_layer(e: &EffectInstance, t: filmcraft_time::Tick, base: Option<&Tree>, box_height: f32) -> Tree {
    let p = |id: &str| e.params.get(id).map(|p| p.value_at(t));
    let text_of = |id: &str| match p(id) {
        Some(ParamValue::Text(s)) => s,
        _ => String::new(),
    };
    let f64_of = |id: &str| match p(id) {
        Some(ParamValue::Float(v)) if v.is_finite() => Some(v as f32),
        _ => None,
    };
    let bool_of = |id: &str| matches!(p(id), Some(ParamValue::Bool(true)));
    let choice_of = |id: &str| match p(id) {
        Some(ParamValue::Choice(c)) => c,
        _ => 0,
    };
    let text = text_of("text");
    // a font the title already names keeps Premiere's own name for it (the font need not be
    // installed here)
    let base_fonts: Vec<String> = match base.map(|b| b.doc.f.get(&1)) {
        Some(Some(Fv::Strs(v))) => v.clone(),
        _ => Vec::new(),
    };
    let ps_for = |family: &str, style: &str| -> String {
        base_fonts
            .iter()
            .find(|ps| {
                let (f, s) = font_names(ps);
                f.eq_ignore_ascii_case(family) && s.eq_ignore_ascii_case(style)
            })
            .cloned()
            .unwrap_or_else(|| postscript_for(family, style))
    };
    let ps = ps_for(&text_of("font"), &text_of("font_style"));
    let mut tree = base.cloned().unwrap_or_else(|| Tree::new_title(&text, &ps));
    // the base's first run style keeps what FilmCraft does not hold (gradients, the default
    // stroke width Premiere writes)
    let base_style = match tree.doc.f.get(&0) {
        Some(Fv::Tabs(r)) => r.first().and_then(|r| tab(r, 1)).cloned(),
        _ => None,
    }
    .unwrap_or_else(new_style);
    let mut fonts: Vec<String> = vec![ps.clone()];
    let mut style = base_style.clone();
    style.f.remove(&0);
    let put = |st: &mut Tab, slot: u16, v: Option<Fv>| match v {
        Some(v) => {
            st.f.insert(slot, v);
        }
        None => {
            st.f.remove(&slot);
        }
    };
    let size = f64_of("size").unwrap_or(100.0);
    put(&mut style, 1, ((size - 100.0).abs() > 1e-4).then_some(Fv::F32(size)));
    let fill = p("fill_color").as_ref().and_then(bytes_of).unwrap_or([255; 3]);
    put(&mut style, 2, (fill != [255; 3]).then(|| Fv::Tab(rgb_tab(fill))));
    if bool_of("stroke") {
        let c = p("stroke_color").as_ref().and_then(bytes_of).unwrap_or([0; 3]);
        put(&mut style, 4, Some(Fv::Tab(rgb_tab(c))));
        put(&mut style, 5, Some(Fv::U8(1)));
        put(&mut style, 6, Some(Fv::F32(f64_of("stroke_width").unwrap_or(4.0))));
    } else {
        put(&mut style, 5, None);
        put(&mut style, 4, None);
        put(&mut style, 6, Some(Fv::F32(4.0)));
    }
    if bool_of("stroke2") {
        let mut s2 = match style.f.get(&17) {
            Some(Fv::Tabs(v)) => v.first().cloned().unwrap_or_default(),
            _ => Tab::default(),
        };
        let c = p("stroke2_color").as_ref().and_then(bytes_of).unwrap_or([255; 3]);
        s2.f.insert(0, Fv::Tab(rgb_tab(c)));
        s2.f.insert(1, Fv::U8(1));
        s2.f.insert(2, Fv::F32(f64_of("stroke2_width").unwrap_or(10.0)));
        s2.f.entry(4).or_insert_with(|| Fv::Tab(Tab::default()));
        s2.f.entry(5).or_insert(Fv::U32(2));
        put(&mut style, 17, Some(Fv::Tabs(vec![s2])));
    } else {
        put(&mut style, 17, None);
    }
    let nz = |v: Option<f32>| v.filter(|x| x.abs() > 1e-6).map(Fv::F32);
    put(&mut style, 8, nz(f64_of("tracking")));
    put(&mut style, 10, nz(f64_of("baseline_shift")));
    put(&mut style, 12, Some(caps_swap(choice_of("caps"))).filter(|c| *c != 0).map(Fv::U32));
    put(&mut style, 14, bool_of("faux_bold").then_some(Fv::U8(1)));
    put(&mut style, 15, bool_of("faux_italic").then_some(Fv::U8(1)));
    put(&mut style, 16, bool_of("underline").then_some(Fv::U8(1)));
    // runs: the text split where per-character styles change
    let chars: Vec<char> = text.replace('\n', "\r").chars().collect();
    let runs_in = filmcraft_project::graphic_design::normalize_runs(&e.layer.as_ref().map(|l| l.runs.clone()).unwrap_or_default(), chars.len());
    let mut cuts: Vec<usize> = vec![0, chars.len()];
    for r in &runs_in {
        cuts.push(r.start);
        cuts.push(r.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1].min(chars.len()));
        if a >= b && !(chars.is_empty() && runs.is_empty()) {
            continue;
        }
        let mut st = style.clone();
        if let Some(r) = runs_in.iter().find(|r| r.start <= a && b <= r.end) {
            let cs = &r.style;
            if cs.font.is_some() || cs.font_style.is_some() {
                let fam = cs.font.clone().unwrap_or_else(|| text_of("font"));
                let sty = cs.font_style.clone().unwrap_or_else(|| text_of("font_style"));
                let ps = ps_for(&fam, &sty);
                let i = fonts.iter().position(|f| *f == ps).unwrap_or_else(|| {
                    fonts.push(ps.clone());
                    fonts.len() - 1
                });
                put(&mut st, 0, (i > 0).then_some(Fv::U32(i as u32)));
            }
            if let Some(s) = cs.size {
                put(&mut st, 1, ((s - 100.0).abs() > 1e-4).then_some(Fv::F32(s)));
            }
            if let Some(c) = cs.fill {
                let c = [c[0], c[1], c[2]].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
                put(&mut st, 2, (c != [255; 3]).then(|| Fv::Tab(rgb_tab(c))));
            }
            for (slot, v) in [(14u16, cs.faux_bold), (15, cs.faux_italic), (16, cs.underline)] {
                if let Some(v) = v {
                    put(&mut st, slot, v.then_some(Fv::U8(1)));
                }
            }
            if let Some(v) = cs.tracking {
                put(&mut st, 8, nz(Some(v)));
            }
            if let Some(v) = cs.baseline_shift {
                put(&mut st, 10, nz(Some(v)));
            }
            if let Some(v) = cs.caps {
                put(&mut st, 12, Some(caps_swap(v)).filter(|c| *c != 0).map(Fv::U32));
            }
        }
        let mut run = Tab::default();
        run.f.insert(0, Fv::Str(chars.get(a..b).map(|c| c.iter().collect()).unwrap_or_default()));
        run.f.insert(1, Fv::Tab(st));
        runs.push(run);
    }
    let doc = &mut tree.doc;
    doc.f.insert(0, Fv::Tabs(runs));
    doc.f.insert(1, Fv::Strs(fonts));
    let mut dput = |slot: u16, v: Option<Fv>| match v {
        Some(v) => {
            doc.f.insert(slot, v);
        }
        None => {
            doc.f.remove(&slot);
        }
    };
    dput(4, align_out(choice_of("align")).map(Fv::U32));
    match f64_of("box_width").filter(|w| *w > 0.0) {
        Some(w) => {
            let h = f64_of("box_height").filter(|h| *h > 0.0).or_else(|| f32_of_doc(base, 3)).unwrap_or(box_height.max(1.0));
            dput(2, Some(Fv::F32(w)));
            dput(3, Some(Fv::F32(h)));
            dput(5, Some(choice_of("valign")).filter(|v| (1..=2).contains(v)).map(Fv::U32));
        }
        None => {
            dput(2, None);
            dput(3, None);
            dput(5, None);
        }
    }
    dput(6, nz(f64_of("leading")));
    dput(9, bool_of("vertical").then_some(Fv::U8(1)));
    if bool_of("shadow") {
        let c = p("shadow_color").as_ref().and_then(bytes_of).unwrap_or([0; 3]);
        dput(10, Some(Fv::Tab(rgb_tab(c))));
        dput(11, Some(Fv::U8(1)));
        for (slot, id, absent) in
            [(12u16, "shadow_opacity", 75.0f32), (13, "shadow_angle", 135.0), (14, "shadow_distance", 7.0), (15, "shadow_size", 0.0), (16, "shadow_blur", 40.0)]
        {
            let v = f64_of(id).unwrap_or(absent);
            dput(slot, ((v - absent).abs() > 1e-4).then_some(Fv::F32(v)));
        }
    } else {
        dput(11, None);
    }
    if bool_of("background") {
        let c = p("background_color").as_ref().and_then(bytes_of).unwrap_or([0; 3]);
        dput(17, Some(Fv::Tab(rgb_tab(c))));
        dput(18, Some(Fv::U8(1)));
        for (slot, id, absent) in [(19u16, "background_opacity", 75.0f32), (20, "background_size", 0.0), (34, "background_radius", 0.0)] {
            let v = f64_of(id).unwrap_or(absent);
            dput(slot, ((v - absent).abs() > 1e-4).then_some(Fv::F32(v)));
        }
    } else {
        dput(18, None);
    }
    dput(30, bool_of("stroke2").then_some(Fv::U32(2)));
    tree
}

fn f32_of_doc(t: Option<&Tree>, slot: u16) -> Option<f32> {
    t.and_then(|t| f32_of(&t.doc, slot)).filter(|v| *v > 0.0)
}

/// Premiere's `BinaryHash` for a value: 28 hex digits of a hash of the bytes, then the length plus
/// 12 as 8 hex digits (`75f3b046-f02d-8dde-4fe3-9a910000014c` for 320 bytes), as Premiere writes
/// them; identical values share one.
pub(crate) fn binary_hash(bytes: &[u8]) -> String {
    let fnv = |seed: u64| {
        let mut h = seed;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    };
    let (a, b) = (fnv(0xcbf2_9ce4_8422_2325), fnv(0x8422_2325_cbf2_9ce4));
    let hex = format!("{a:016x}{b:016x}");
    let len = (bytes.len() as u64).saturating_add(12) & 0xffff_ffff;
    format!("{}-{}-{}-{}-{}{len:08x}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..24])
}

#[cfg(test)]
#[path = "prproj_title_tests.rs"]
mod tests;
