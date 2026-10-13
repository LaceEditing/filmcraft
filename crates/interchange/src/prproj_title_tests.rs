use super::*;
use filmcraft_project::graphic_design::{CharStyle, StyleRun};
use filmcraft_time::Tick;

fn layer() -> EffectInstance {
    filmcraft_project::effect::find_effect(filmcraft_project::graphic::TEXT_LAYER).unwrap().instance()
}

fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
    e.params.insert(id.into(), Param::new(v));
}

fn get(e: &EffectInstance, id: &str) -> ParamValue {
    e.params.get(id).map(|p| p.value.clone()).unwrap_or_else(|| panic!("no {id}"))
}

/// Written and read back: the same document.
#[test]
fn a_new_title_round_trips() {
    let t = Tree::new_title("Hello\rWorld", "ArialMT");
    let bytes = t.write();
    assert_eq!(&bytes[8..12], &MARKER);
    assert_eq!(u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize, bytes.len() - 12, "the header holds the buffer's length");
    let back = Tree::read(&bytes).expect("reads");
    assert_eq!(back, t);
}

/// FilmCraft's text layer → a title → back: every setting Premiere stores comes back.
#[test]
fn text_layer_settings_survive_a_title() {
    let mut e = layer();
    set(&mut e, "text", ParamValue::Text("Big Small\nnext line".into()));
    set(&mut e, "font", ParamValue::Text("Inter".into()));
    set(&mut e, "font_style", ParamValue::Text("Regular".into()));
    set(&mut e, "size", ParamValue::Float(120.0));
    set(&mut e, "fill_color", ParamValue::Color([1.0, 128.0 / 255.0, 0.0, 1.0]));
    set(&mut e, "stroke", ParamValue::Bool(true));
    set(&mut e, "stroke_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0]));
    set(&mut e, "stroke_width", ParamValue::Float(12.0));
    set(&mut e, "stroke2", ParamValue::Bool(true));
    set(&mut e, "stroke2_color", ParamValue::Color([0.0, 1.0, 0.0, 1.0]));
    set(&mut e, "stroke2_width", ParamValue::Float(10.0));
    set(&mut e, "align", ParamValue::Choice(1));
    set(&mut e, "tracking", ParamValue::Float(100.0));
    set(&mut e, "leading", ParamValue::Float(50.0));
    set(&mut e, "baseline_shift", ParamValue::Float(20.0));
    set(&mut e, "caps", ParamValue::Choice(2));
    set(&mut e, "faux_bold", ParamValue::Bool(true));
    set(&mut e, "underline", ParamValue::Bool(true));
    set(&mut e, "box_width", ParamValue::Float(800.0));
    set(&mut e, "shadow", ParamValue::Bool(true));
    set(&mut e, "shadow_color", ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
    set(&mut e, "shadow_opacity", ParamValue::Float(100.0));
    set(&mut e, "shadow_angle", ParamValue::Float(45.0));
    set(&mut e, "shadow_distance", ParamValue::Float(30.0));
    set(&mut e, "shadow_size", ParamValue::Float(5.0));
    set(&mut e, "shadow_blur", ParamValue::Float(0.0));
    set(&mut e, "background", ParamValue::Bool(true));
    set(&mut e, "background_color", ParamValue::Color([1.0, 1.0, 0.0, 1.0]));
    set(&mut e, "background_opacity", ParamValue::Float(50.0));
    set(&mut e, "background_size", ParamValue::Float(20.0));
    set(&mut e, "background_radius", ParamValue::Float(8.0));
    // "Big" larger and red
    e.layer.get_or_insert_with(Default::default).runs =
        vec![StyleRun { start: 0, end: 3, style: CharStyle { size: Some(150.0), fill: Some([1.0, 0.0, 0.0, 1.0]), ..Default::default() } }];
    let tree = from_layer(&e, Tick(0), None, 300.0);
    let bytes = tree.write();
    let back = Tree::read(&bytes).expect("reads");
    assert_eq!(back.dropped, 0);
    let runs = match back.doc.f.get(&0) {
        Some(Fv::Tabs(r)) => r.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(runs.len(), 2, "split where the style changes");
    assert_eq!(runs[0].f.get(&0), Some(&Fv::Str("Big".into())));
    assert_eq!(runs[1].f.get(&0), Some(&Fv::Str(" Small\rnext line".into())), "Premiere's line breaks");
    let mut e2 = layer();
    let unread = to_layer(&back, &mut e2);
    assert_eq!(unread, Unread::default());
    // the first run is "Big": its style is the layer's; the rest is a character run
    assert_eq!(get(&e2, "size"), ParamValue::Float(150.0));
    assert_eq!(get(&e2, "text"), ParamValue::Text("Big Small\nnext line".into()));
    for id in [
        "stroke",
        "stroke_color",
        "stroke_width",
        "stroke2",
        "stroke2_color",
        "stroke2_width",
        "align",
        "tracking",
        "leading",
        "baseline_shift",
        "caps",
        "faux_bold",
        "underline",
        "box_width",
        "shadow",
        "shadow_color",
        "shadow_opacity",
        "shadow_angle",
        "shadow_distance",
        "shadow_size",
        "shadow_blur",
        "background",
        "background_color",
        "background_opacity",
        "background_size",
        "background_radius",
    ] {
        assert_eq!(get(&e2, id), get(&e, id), "{id}");
    }
    let runs = &e2.layer.as_ref().expect("character styles").runs;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].start, runs[0].end), (3, 19));
    assert_eq!(runs[0].style.size, Some(120.0));
    let fill = runs[0].style.fill.unwrap();
    assert!((fill[1] - 128.0 / 255.0).abs() < 1e-6, "{fill:?}");
}

/// Premiere's alignment and caps numbers differ from FilmCraft's lists.
#[test]
fn alignment_and_caps_map_both_ways() {
    for (fc, pr) in [(0u32, None), (1, Some(2u32)), (2, Some(1))] {
        assert_eq!(align_out(fc), pr);
        assert_eq!(align_in(pr.unwrap_or(0)), fc);
    }
    for (fc, pr) in [(0u32, 0u32), (1, 2), (2, 1)] {
        assert_eq!(caps_swap(fc), pr);
        assert_eq!(caps_swap(pr), fc);
    }
}

#[test]
fn postscript_names_read_without_the_font() {
    assert_eq!(font_names("Arial-BoldMT").0.replace(' ', ""), "Arial");
    assert_eq!(font_names("Lato-BoldItalic").1, "Bold Italic");
    if filmcraft_text::fonts::by_postscript("FatFrank-Heavy").is_none() {
        assert_eq!(font_names("FatFrank-Heavy"), ("FatFrank".into(), "Heavy".into()), "a family not installed keeps its name");
    }
    // a font not installed here keeps Premiere's own name when the title is written again
    let base = Tree::new_title("x", "TimesNewRomanPSMT");
    let mut e = layer();
    to_layer(&base, &mut e);
    let again = from_layer(&e, Tick(0), Some(&base), 100.0);
    assert_eq!(again.doc.f.get(&1), Some(&Fv::Strs(vec!["TimesNewRomanPSMT".into()])));
    let h = binary_hash(&[1, 2, 3]);
    assert_eq!(h.len(), 36);
    assert!(h.ends_with("0000000f"), "length + 12: {h}");
}

/// The hand-built fixture Premiere's layout was first read from ("50": centred, a 2125 × 539 box).
#[test]
fn the_first_premiere_title_reads() {
    let t = Tree::read(&crate::prproj_text::tests::fifty()).expect("reads");
    let mut e = layer();
    to_layer(&t, &mut e);
    assert_eq!(get(&e, "text"), ParamValue::Text("50".into()));
    assert_eq!(get(&e, "align"), ParamValue::Choice(1), "centre");
    assert_eq!(get(&e, "box_width"), ParamValue::Float(2125.0));
    assert_eq!(get(&e, "stroke"), ParamValue::Bool(true));
    assert_eq!(get(&e, "stroke_width"), ParamValue::Float(26.0));
}

#[test]
fn damaged_values_never_panic() {
    let good = Tree::new_title("Damaged", "ArialMT").write();
    assert!(Tree::read(&[]).is_none());
    for cut in 0..good.len() {
        let _ = Tree::read(&good[..cut]);
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..3000 {
        let mut b = good.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = 12 + (seed as usize) % (b.len() - 12);
            b[i] = (seed >> 32) as u8;
        }
        if let Some(t) = Tree::read(&b) {
            let mut e = layer();
            to_layer(&t, &mut e);
            let _ = from_layer(&e, Tick(0), Some(&t), 100.0).write();
        }
    }
}

/// Against titles Premiere saved (a local title kit: `FC_TITLE_KIT=…/Title Kit.prproj`, one title
/// per setting): each reads as set in Premiere, and written again reads the same.
#[test]
#[ignore = "needs a title kit saved by Premiere (FC_TITLE_KIT)"]
fn premiere_title_kit() {
    let Ok(path) = std::env::var("FC_TITLE_KIT") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let (xml, _) = crate::prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    let doc = crate::xml::parse(&text).unwrap();
    let root = doc.root_element();
    let payloads = crate::prproj::payloads(root);
    let ix = crate::prproj_graph::Index::new(root);
    let mut seen = 0;
    for ti in root.children().filter(|n| n.has_tag_name("VideoClipTrackItem")) {
        let Some(sub) = ix.follow(ti, &["ClipTrackItem", "SubClip"]) else { continue };
        let Some(name) = crate::xml::child_text(sub, "Name").filter(|n| n.len() == 3 && n.starts_with('T')) else { continue };
        let Some(chain) = ix.follow(ti, &["ClipTrackItem", "ComponentOwner", "Components"]) else { continue };
        for c in crate::prproj_fx::chain(&ix, chain).into_iter().filter(|c| c.match_name == "AE.ADBE Text") {
            let Some(p) = c.params.iter().copied().find(|p| crate::xml::child_text(*p, "Name") == Some("Source Text")) else { continue };
            let Some(v) = crate::xml::child(p, "StartKeyframeValue") else { continue };
            let Some(b) = crate::prproj::binary_in(&payloads, v) else { continue };
            let t = Tree::read(&b).unwrap_or_else(|| panic!("{name} reads"));
            assert_eq!(t.dropped, 0, "{name}: nothing unknown that refers elsewhere");
            let again = Tree::read(&t.write()).unwrap();
            assert_eq!(again, t, "{name}: written and read back");
            let mut e = layer();
            let unread = to_layer(&t, &mut e);
            let f = |id: &str| get(&e, id);
            let col = |c: [u8; 3]| ParamValue::Color([f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0]);
            match name {
                "T01" => assert_eq!(f("text"), ParamValue::Text("Base".into())),
                "T03" => assert_eq!(f("text"), ParamValue::Text("Line one\nLine two".into())),
                "T06" => assert!(matches!(f("font"), ParamValue::Text(t) if t.replace(' ', "") == "TimesNewRoman")),
                "T09" => assert_eq!(f("size"), ParamValue::Float(50.0)),
                "T10" => assert_eq!(f("size"), ParamValue::Float(237.5)),
                "T11" => assert_eq!(f("fill_color"), col([255, 128, 0])),
                "T12" => assert_eq!(f("fill_color"), col([0x33, 0x66, 0xcc])),
                "T14" => {
                    assert_eq!((f("stroke"), f("stroke_width"), f("stroke_color")), (ParamValue::Bool(true), ParamValue::Float(12.0), col([255, 0, 0])))
                }
                "T15" => {
                    assert_eq!((f("stroke2"), f("stroke2_width"), f("stroke2_color")), (ParamValue::Bool(true), ParamValue::Float(10.0), col([0, 255, 0])))
                }
                "T16" => assert_eq!(
                    (f("background"), f("background_opacity"), f("background_size")),
                    (ParamValue::Bool(true), ParamValue::Float(100.0), ParamValue::Float(10.0))
                ),
                "T17" => assert_eq!(
                    (f("background_color"), f("background_opacity"), f("background_size"), f("background_radius")),
                    (col([255, 255, 0]), ParamValue::Float(50.0), ParamValue::Float(20.0), ParamValue::Float(8.0))
                ),
                "T18" => assert_eq!(
                    (f("shadow"), f("shadow_opacity"), f("shadow_angle"), f("shadow_distance"), f("shadow_blur")),
                    (ParamValue::Bool(true), ParamValue::Float(75.0), ParamValue::Float(135.0), ParamValue::Float(10.0), ParamValue::Float(20.0))
                ),
                "T19" => assert_eq!(
                    (f("shadow_color"), f("shadow_opacity"), f("shadow_angle"), f("shadow_distance"), f("shadow_size"), f("shadow_blur")),
                    (
                        col([255, 0, 0]),
                        ParamValue::Float(100.0),
                        ParamValue::Float(45.0),
                        ParamValue::Float(30.0),
                        ParamValue::Float(5.0),
                        ParamValue::Float(0.0)
                    )
                ),
                "T20" => assert_eq!(f("align"), ParamValue::Choice(1)),
                "T21" => assert_eq!(f("align"), ParamValue::Choice(2)),
                "T22" => assert_eq!(f("tracking"), ParamValue::Float(100.0)),
                "T23" => assert_eq!(f("leading"), ParamValue::Float(50.0)),
                "T24" => assert!(unread.kerning, "kerning is not held"),
                "T25" => assert_eq!(f("baseline_shift"), ParamValue::Float(20.0)),
                "T26" => assert_eq!(f("caps"), ParamValue::Choice(1)),
                "T27" => assert_eq!(f("caps"), ParamValue::Choice(2)),
                "T28" => assert_eq!(f("faux_bold"), ParamValue::Bool(true)),
                "T29" => assert_eq!(f("faux_italic"), ParamValue::Bool(true)),
                "T30" => assert_eq!(f("underline"), ParamValue::Bool(true)),
                "T31" => assert_eq!(f("box_width"), ParamValue::Float(798.0)),
                "T32" => assert!(unread.gradient, "gradient fills are not held"),
                "T33" => {
                    let r = &e.layer.as_ref().unwrap().runs;
                    assert_eq!(f("size"), ParamValue::Float(150.0), "the first run is the layer's");
                    assert_eq!((r[0].start, r[0].end, r[0].style.size), (4, 9, Some(100.0)));
                }
                "T34" => {
                    let r = &e.layer.as_ref().unwrap().runs;
                    assert_eq!(r[0].style.font.as_deref().map(|f| f.replace(' ', "")).as_deref(), Some("TimesNewRoman"));
                }
                "T37" => assert_eq!(f("vertical"), ParamValue::Bool(true)),
                "T42" => assert_eq!(f("text"), ParamValue::Text("Hello There".into())),
                _ => {}
            }
            // FilmCraft's layer written as a title on the original reads back as the same layer
            let mut e3 = layer();
            to_layer(&Tree::read(&from_layer(&e, Tick(0), Some(&t), 100.0).write()).unwrap(), &mut e3);
            for (id, v) in &e.params {
                // (printed: an automatic position is NaN, never equal to itself)
                assert_eq!(format!("{:?}", e3.params.get(id).map(|p| &p.value)), format!("{:?}", Some(&v.value)), "{name}: {id}");
            }
            seen += 1;
        }
    }
    assert!(seen >= 35, "{seen} titles");
}
