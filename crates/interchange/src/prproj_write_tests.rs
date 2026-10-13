use super::*;
use crate::prproj::tests::{S, doc, doc_with_masks, gzip, import};
use filmcraft_project::{ClipId, ItemKind, Param, ParamValue, Track, TrackId};
use filmcraft_time::Tick;

fn seq_id(p: &Project) -> ItemId {
    p.items.values().find(|i| i.name == "Main Edit").map(|i| i.id).unwrap()
}

fn read(bytes: &[u8]) -> Project {
    let (xml, _) = prproj_graph::decompress(bytes).unwrap();
    import(&xml, Some("/edits/show")).0.project
}

#[test]
fn exporting_an_unchanged_project_gives_back_the_same_document() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), Some("/edits/show"));
    let (out, report) = export(&imp.project, plain.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), plain, "{report}");

    // gzip in, gzip out, same XML inside
    let gz = gzip(plain.as_bytes());
    let (out, _) = export(&imp.project, &gz, &[], None).unwrap();
    assert!(out.starts_with(&[0x1f, 0x8b]));
    assert_eq!(prproj_graph::decompress(&out).unwrap().0, plain.as_bytes());
}

#[test]
fn edits_round_trip_through_the_premiere_project() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), Some("/edits/show"));
    let mut p = imp.project;
    let sid = seq_id(&p);
    let new_track = TrackId(p.alloc_id());
    let new_clip = ClipId(p.alloc_id());
    let split_id = ClipId(p.alloc_id());
    {
        let seq = p.sequence_mut(sid).unwrap();
        let v1 = &mut seq.video_tracks[0];
        // trim the first clip's head by one second (keyframes keep their clip times)
        let a = &mut v1.items[0];
        a.start = Tick(S);
        a.duration = Tick(4 * S);
        a.source_in = Tick(11 * S);
        let scale = a.effect_mut("motion").unwrap().params.get_mut("scale").unwrap();
        scale.keyframes[1].value = ParamValue::Float(150.0);
        // split the reversed 2x clip at 6 s; take the blur off the left half
        let b = v1.items[1].clone();
        let mut right = b.clone();
        right.id = split_id;
        right.start = Tick(6 * S);
        right.duration = Tick(S);
        right.source_in = Tick(22 * S);
        let left = &mut v1.items[1];
        left.duration = Tick(S);
        left.effects.retain(|e| e.effect != "gaussian_blur");
        v1.items.push(right);
        // a new clip (no Premiere origin) of the same media on a new track
        let mut fresh = v1.items[0].clone();
        fresh.id = new_clip;
        fresh.origin = None;
        fresh.link = None;
        fresh.start = Tick(10 * S);
        fresh.duration = Tick(S);
        fresh.source_in = Tick(30 * S);
        fresh.effects = filmcraft_project::effect::intrinsic_video();
        let mut v2 = Track::new(new_track, TrackKind::Video, "Video 2".into());
        v2.items.push(fresh);
        seq.video_tracks.push(v2);
        // 3 dB louder dialogue
        let a1 = &mut seq.audio_tracks[0].items[0];
        a1.effect_mut("volume").unwrap().params.insert("level".into(), Param::new(ParamValue::Float(-3.0)));
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();

    let v1 = &seq.video_tracks[0];
    let got: Vec<(i64, i64, i64, f64, bool)> = v1.items.iter().map(|c| (c.start.0, c.duration.0, c.source_in.0, c.speed, c.reverse)).collect();
    assert_eq!(got, vec![(S, 4 * S, 11 * S, 1.0, false), (5 * S, S, 20 * S, 2.0, true), (6 * S, S, 22 * S, 2.0, true)], "{report}");
    let (a, left, right) = (&v1.items[0], &v1.items[1], &v1.items[2]);
    let keys: Vec<(Tick, f64)> = a.effect("motion").unwrap().params["scale"].keyframes.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect();
    assert_eq!(keys, vec![(Tick(0), 100.0), (Tick(2 * S), 150.0)]);
    assert!(left.effect("gaussian_blur").is_none());
    assert!(right.effect("gaussian_blur").is_some(), "the copy keeps the blur");
    assert_ne!(left.origin, right.origin, "the right half is a new Premiere object");
    // the dissolve still joins the first clip and the left half
    assert_eq!(v1.transitions.len(), 1);
    assert_eq!((v1.transitions[0].from, v1.transitions[0].to), (Some(a.id), Some(left.id)));

    // the new clip copies another clip of the same media but carries none of its settings
    let v2 = &seq.video_tracks[1];
    assert_eq!(v2.items.len(), 1);
    let fresh = &v2.items[0];
    assert_eq!((fresh.start.0, fresh.duration.0, fresh.source_in.0), (10 * S, S, 30 * S));
    let scale = fresh.effect("motion").and_then(|e| e.params.get("scale"));
    assert!(scale.is_none_or(|p| p.keyframes.is_empty() && p.value.as_f64() == Some(100.0)), "{scale:?}");
    assert!(fresh.effects.iter().all(|e| e.effect != "gaussian_blur"));

    let a1 = &seq.audio_tracks[0].items[0];
    let level = a1.effect("volume").unwrap().params["level"].value.as_f64().unwrap();
    assert!((level + 3.0).abs() < 1e-9, "{level}");
    assert_eq!(a.link, a1.link);
    assert!(a.link.is_some());

    // untouched objects keep their bytes: the media and its streams are written exactly as before
    let text = String::from_utf8(out).unwrap();
    for keep in [r#"<VideoStream ObjectID="14">"#, r#"<RelativePath>.\Footage\interview.mov</RelativePath>"#, r#"<Marker ObjectID="13">"#] {
        assert!(text.contains(keep), "{keep}");
    }
}

/// Media FilmCraft added that no file in the project can serve as a model for is built from
/// scratch (as in a new project).
#[test]
fn media_without_a_model_is_built_from_scratch() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    // a media item Premiere never saw
    let other =
        p.add_item("new.mov", filmcraft_project::Label::Iris, p.items.values().find(|i| i.name == "interview.mov").map(|i| i.kind.clone()).unwrap(), None);
    if let Some(it) = p.item_mut(other) {
        it.metadata.clear();
        if let ItemKind::Media(m) = &mut it.kind {
            // no .mxf in the Premiere project to model it on
            m.media = filmcraft_project::MediaRef::File { path: "/edits/show/new.mxf".into() };
        }
    }
    let seq = p.sequence_mut(sid).unwrap();
    let mut c = seq.video_tracks[0].items[0].clone();
    c.id = ClipId(9_999);
    c.item = other;
    c.origin = None;
    c.start = Tick(20 * S);
    seq.video_tracks[0].items.push(c);
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(!report.mentions("not written"), "{report}");
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(seq.video_tracks[0].items.len(), 3, "{report}");
    let added = seq.video_tracks[0].items.iter().find(|c| c.start == Tick(20 * S)).unwrap();
    assert_eq!(back.item(added.item).unwrap().name, "new.mov");
    assert!(
        matches!(&back.item(added.item).unwrap().kind, ItemKind::Media(m) if matches!(&m.media, filmcraft_project::MediaRef::File { path } if path.ends_with("new.mxf")))
    );
}

#[test]
fn a_project_that_did_not_come_from_premiere_is_refused() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    for it in p.items.values_mut() {
        it.metadata.clear();
    }
    assert!(export(&p, plain.as_bytes(), &[], None).is_err());
    // and a damaged original is an error, not a panic
    let (imp, _) = import(plain.as_bytes(), None);
    assert!(export(&imp.project, b"\x1f\x8b\x08\x00garbage", &[], None).is_err());
    assert!(export(&imp.project, b"<xmeml/>", &[], None).is_err());
}

#[test]
fn premiere_clips_filmcraft_never_showed_are_kept() {
    // a clip whose source FilmCraft does not understand (a generated matte, say)
    let plain = doc()
        .replace(
            r#"<TrackItem Index="1" ObjectRef="50"/></TrackItems></ClipItems>"#,
            r#"<TrackItem Index="1" ObjectRef="50"/><TrackItem Index="2" ObjectRef="90"/></TrackItems></ClipItems>"#,
        )
        .replace(
            "</PremiereData>",
            &format!(
                r#"<VideoClipTrackItem ObjectID="90"><ClipTrackItem><TrackItem><Start>{}</Start><End>{}</End></TrackItem><SubClip ObjectRef="91"/></ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="91"><Clip ObjectRef="92"/><Name>Color Matte</Name></SubClip>
  <VideoClip ObjectID="92"><Clip><Source ObjectRef="93"/></Clip></VideoClip>
  <ColorMatteSource ObjectID="93"/>
</PremiereData>"#,
                8 * S,
                9 * S
            ),
        );
    let (imp, report) = import(plain.as_bytes(), None);
    assert!(report.mentions("Color Matte"), "{report}");
    let mut p = imp.project;
    let sid = seq_id(&p);
    // delete the first clip; the matte FilmCraft never showed must stay
    p.sequence_mut(sid).unwrap().video_tracks[0].items.remove(0);
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains(r#"<TrackItems><TrackItem Index="0" ObjectRef="50" /><TrackItem Index="1" ObjectRef="90" /></TrackItems>"#), "{report}");
}

#[test]
fn media_added_in_filmcraft_is_added_to_the_premiere_project() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), Some("/edits/show"));
    let mut p = imp.project;
    let sid = seq_id(&p);
    // a new file FilmCraft probed: 20 s, 1920x1080 at 30 fps, stereo 48 kHz
    let mut kind = p.items.values().find(|i| i.name == "interview.mov").map(|i| i.kind.clone()).unwrap();
    if let ItemKind::Media(m) = &mut kind {
        m.media = filmcraft_project::MediaRef::File { path: "/edits/show/Footage/broll.mov".into() };
        m.info.duration = Tick(20 * S);
        if let Some(v) = m.info.video.as_mut() {
            (v.width, v.height) = (1920, 1080);
        }
        m.markers.clear();
    }
    let broll = p.add_item("broll.mov", filmcraft_project::Label::Iris, kind, None);
    let seq = p.sequence_mut(sid).unwrap();
    let mut v = seq.video_tracks[0].items[0].clone();
    v.id = ClipId(9_001);
    v.item = broll;
    v.origin = None;
    v.link = Some(77);
    v.start = Tick(10 * S);
    v.duration = Tick(2 * S);
    v.source_in = Tick(3 * S);
    v.effects = filmcraft_project::effect::intrinsic_video();
    let mut a = seq.audio_tracks[0].items[0].clone();
    a.id = ClipId(9_002);
    a.item = broll;
    a.origin = None;
    a.link = Some(77);
    a.start = Tick(10 * S);
    a.duration = Tick(2 * S);
    a.source_in = Tick(3 * S);
    a.effects = filmcraft_project::effect::intrinsic_audio();
    seq.video_tracks[0].items.push(v);
    seq.audio_tracks[0].items.push(a);

    let (out, report) = export(&p, plain.as_bytes(), &[], Some("/edits/show")).unwrap();
    let text = String::from_utf8(out.clone()).unwrap();
    assert!(text.contains(r"<FilePath>L:\Edit\Footage\broll.mov</FilePath>"), "{report}");
    assert!(text.contains(r"<RelativePath>.\Footage\broll.mov</RelativePath>"));
    assert!(text.contains("<FrameRect>0,0,1920,1080</FrameRect>"));
    let back = read(&out);
    let item = back.items.values().find(|i| i.name == "broll.mov").expect("the new media is in the project");
    let ItemKind::Media(m) = &item.kind else { panic!() };
    assert_eq!(m.media, filmcraft_project::MediaRef::File { path: "/edits/show/Footage/broll.mov".into() });
    assert_eq!(m.info.duration, Tick(20 * S));
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let nv = seq.video_tracks[0].items.iter().find(|c| c.item == item.id).expect("video clip of the new media");
    let na = seq.audio_tracks[0].items.iter().find(|c| c.item == item.id).expect("audio clip of the new media");
    assert_eq!((nv.start.0, nv.duration.0, nv.source_in.0), (10 * S, 2 * S, 3 * S));
    assert_eq!((na.start.0, na.duration.0, na.source_in.0), (10 * S, 2 * S, 3 * S));
    assert!(nv.link.is_some() && nv.link == na.link);
    // the copied clip carries none of the model clip's settings
    assert!(nv.effects.iter().all(|e| e.effect != "gaussian_blur"));
    assert!(nv.effect("motion").and_then(|m| m.params.get("scale")).is_none_or(|p| p.keyframes.is_empty()));
    // and the original media is untouched
    assert!(text.contains(r"<FilePath>L:\Edit\Footage\interview.mov</FilePath>"));
}

#[test]
fn markers_edited_in_filmcraft_are_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let seq = p.sequence_mut(sid).unwrap();
        seq.markers[0].start = Tick(3 * S);
        seq.markers[0].name = "Act Two".into();
        let mut m = seq.markers[0].clone();
        m.id = filmcraft_project::MarkerId(9_100);
        m.start = Tick(5 * S);
        m.duration = Tick(S);
        m.name = "Laugh \"big\" <here>".into();
        m.comment = "keep & tighten".into();
        m.kind = filmcraft_project::MarkerKind::Comment;
        seq.markers.push(m);
    }
    // the source marker on the interview is deleted
    let media = p.items.values().find(|i| i.name == "interview.mov").map(|i| i.id).unwrap();
    if let Some(m) = p.item_mut(media).and_then(|i| i.as_media_mut()) {
        m.markers.clear();
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let got: Vec<(i64, i64, &str, &str)> = seq.markers.iter().map(|m| (m.start.0, m.duration.0, m.name.as_str(), m.comment.as_str())).collect();
    assert_eq!(got, vec![(3 * S, 0, "Act Two", ""), (5 * S, S, "Laugh \"big\" <here>", "keep & tighten")], "{report}");
    let m = back.items.values().find(|i| i.name == "interview.mov").and_then(|i| i.as_media()).unwrap();
    assert!(m.markers.is_empty());
}

#[test]
fn transitions_added_in_filmcraft_are_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    let (left_id, right_id) = {
        let seq = p.sequence_mut(sid).unwrap();
        let v1 = &mut seq.video_tracks[0];
        // split the second clip at 6 s and dissolve between the halves
        let mut right = v1.items[1].clone();
        right.id = ClipId(9_200);
        right.start = Tick(6 * S);
        right.duration = Tick(S);
        right.source_in = Tick(22 * S);
        v1.items[1].duration = Tick(S);
        let left = v1.items[1].id;
        v1.items.push(right);
        let mut t = v1.transitions[0].clone();
        t.id = filmcraft_project::TransitionId(9_300);
        t.start = Tick(6 * S - S / 4);
        t.duration = Tick(S / 2);
        t.from = Some(left);
        t.to = Some(ClipId(9_200));
        v1.transitions.push(t);
        (left, ClipId(9_200))
    };
    let _ = (left_id, right_id);
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(report.mentions("added a Cross Dissolve"), "{report}");
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let v1 = &seq.video_tracks[0];
    assert_eq!(v1.transitions.len(), 2, "{report}");
    let t = v1.transitions.iter().find(|t| t.start == Tick(6 * S - S / 4)).expect("the new dissolve");
    let (l, r) = (v1.items.iter().find(|c| c.start == Tick(5 * S)).unwrap(), v1.items.iter().find(|c| c.start == Tick(6 * S)).unwrap());
    assert_eq!((t.from, t.to, t.duration), (Some(l.id), Some(r.id), Tick(S / 2)));
    // both clips point back at it
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("<TailTransition ObjectRef=").count() + text.matches("<HeadTransition ObjectRef=").count(), 2);
}

#[test]
fn the_first_marker_of_a_sequence_creates_its_marker_list() {
    // the sequence has no markers (and so no marker list) in Premiere
    let plain = doc().replace(
        r#"<VideoClip ObjectID="20"><Clip><Source ObjectRef="21"/><MarkerOwner><Markers ObjectRef="22"/></MarkerOwner></Clip></VideoClip>"#,
        r#"<VideoClip ObjectID="20"><Clip><Source ObjectRef="21"/></Clip></VideoClip>"#,
    );
    assert!(!plain.contains(r#"<Markers ObjectRef="22"/>"#));
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    assert!(p.sequence(sid).unwrap().markers.is_empty());
    p.sequence_mut(sid).unwrap().markers.push(filmcraft_project::Marker {
        id: filmcraft_project::MarkerId(9_500),
        start: Tick(4 * S),
        duration: Tick(0),
        name: "First".into(),
        comment: String::new(),
        kind: filmcraft_project::MarkerKind::Comment,
        color: filmcraft_project::Label::Green,
    });
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(seq.markers.iter().map(|m| (m.start.0, m.name.as_str())).collect::<Vec<_>>(), vec![(4 * S, "First")], "{report}");
}

#[test]
fn effects_choices_and_colours_added_in_filmcraft_are_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let seq = p.sequence_mut(sid).unwrap();
        let v1 = &mut seq.video_tracks[0];
        // clip 1 gets a Gaussian Blur (vertical only) and a Tint, modelled on clip 2's
        let a = &mut v1.items[0];
        let mut blur = filmcraft_project::find_effect("gaussian_blur").unwrap().instance();
        blur.params.insert("blurriness".into(), Param::new(ParamValue::Float(25.0)));
        blur.params.insert("dimensions".into(), Param::new(ParamValue::Choice(2)));
        let mut tint = filmcraft_project::find_effect("tint").unwrap().instance();
        tint.params.insert("white".into(), Param::new(ParamValue::Color([1.0, 0.5, 0.0, 1.0])));
        tint.params.insert("amount".into(), Param::new(ParamValue::Float(40.0)));
        a.effects.push(blur);
        a.effects.push(tint);
        // clip 2's blur switches to horizontal
        let b = &mut v1.items[1];
        b.effect_mut("gaussian_blur").unwrap().params.insert("dimensions".into(), Param::new(ParamValue::Choice(1)));
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(report.mentions("added \"Gaussian Blur\"") && report.mentions("added \"Tint\""), "{report}");
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let (a, b) = (&seq.video_tracks[0].items[0], &seq.video_tracks[0].items[1]);
    let blur = a.effect("gaussian_blur").expect("blur written");
    assert_eq!(blur.params["blurriness"].value, ParamValue::Float(25.0));
    assert_eq!(blur.params["dimensions"].value, ParamValue::Choice(2));
    let tint = a.effect("tint").expect("tint written");
    let w = tint.params["white"].value.as_color().unwrap();
    assert!((w[0] - 1.0).abs() < 1e-6 && (w[1] - 0.5).abs() < 0.01 && w[2].abs() < 1e-6, "{w:?}");
    assert_eq!(tint.params["amount"].value, ParamValue::Float(40.0));
    assert_eq!(b.effect("gaussian_blur").unwrap().params["dimensions"].value, ParamValue::Choice(1));
    // the model clip keeps its own values
    assert_eq!(b.effect("tint").unwrap().params["amount"].value, ParamValue::Float(100.0));
    assert_eq!(a.effects.iter().filter(|e| !filmcraft_project::find_effect(&e.effect).is_some_and(|d| d.intrinsic)).count(), 2, "{report}");
}

#[test]
fn titles_round_trip_and_text_edits_are_reported() {
    let plain = crate::prproj::tests::doc_with_title();
    let (imp, _) = import(plain.as_bytes(), Some("/edits/show"));
    let (out, report) = export(&imp.project, plain.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), plain, "unchanged: {report}");
    // a moved title is written; its new text is reported, not written
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let clip = &mut p.sequence_mut(sid).unwrap().video_tracks[1].items[0];
        clip.start = Tick(4 * S);
        clip.effect_mut("graphic_text").unwrap().params.insert("text".into(), Param::new(ParamValue::Text("60".into())));
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(seq.video_tracks[1].items[0].start, Tick(4 * S));
    assert!(report.to_string().contains("changes to its text or formatting are not written"), "{report}");
    let t = seq.video_tracks[1].items[0].effect("graphic_text").unwrap();
    assert_eq!(t.params.get("text").map(|x| x.value.clone()), Some(ParamValue::Text("50".into())), "Premiere keeps its text");
}

/// Masks survive a round trip: untouched they stay byte-identical; Feather / Opacity / Expansion /
/// Inverted edits are written; a changed shape or an added mask is reported, not lost silently;
/// a Gaussian Blur added in FilmCraft does not inherit its model's mask.
#[test]
fn masks_round_trip_and_their_settings_are_written() {
    let plain = doc_with_masks();
    let (imp, _) = import(plain.as_bytes(), Some("/edits/show"));
    let (out, report) = export(&imp.project, plain.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), plain, "unchanged masks: same document ({report})");

    let mut p = imp.project.clone();
    let sid = seq_id(&p);
    {
        let seq = p.sequence_mut(sid).unwrap();
        let v1 = &mut seq.video_tracks[0];
        let m = &mut v1.items[1].effect_mut("gaussian_blur").unwrap().masks[0];
        m.feather = Param::new(ParamValue::Float(12.0));
        m.inverted = true;
        // clip 1 gets a blur in FilmCraft, modelled on clip 2's masked one
        v1.items[0].effects.push(filmcraft_project::find_effect("gaussian_blur").unwrap().instance());
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(!report.mentions("mask"), "{report}");
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let m = &seq.video_tracks[0].items[1].effect("gaussian_blur").unwrap().masks[0];
    assert_eq!((m.feather.f64_at(Tick(0)), m.inverted, m.expansion.f64_at(Tick(0))), (12.0, true, -56.0));
    let added = seq.video_tracks[0].items[0].effect("gaussian_blur").expect("blur added");
    assert!(added.masks.is_empty(), "the model's mask was copied");

    // a moved shape and an extra mask are reported
    let mut q = imp.project.clone();
    {
        let seq = q.sequence_mut(sid).unwrap();
        let blur = seq.video_tracks[0].items[1].effect_mut("gaussian_blur").unwrap();
        let moved = blur.masks[0].path_at(Tick(0)).transformed(&filmcraft_geom::Affine::translate(10.0, 0.0));
        blur.masks[0].path = Param::new(ParamValue::Path(moved));
        let tint = seq.video_tracks[0].items[0].effect_mut("tint").unwrap();
        let extra = tint.masks[0].clone();
        tint.masks.push(extra);
    }
    let (_, report) = export(&q, plain.as_bytes(), &[], None).unwrap();
    assert!(report.mentions("mask shape changed") && report.mentions("masks added to or removed from"), "{report}");
}

/// The test project as Premiere saves clips it never had a Motion, Opacity or Volume change on:
/// the second video clip and the audio clip have empty chains with `Default…` flags. Without
/// `tint`, the project has no Tint anywhere.
fn default_chains_doc(tint: bool) -> String {
    let mut d = doc()
        .replace(
            r#"<VideoComponentChain ObjectID="53"><ComponentChain><Components><Component Index="0" ObjectRef="54"/><Component Index="1" ObjectRef="56"/><Component Index="2" ObjectRef="100"/></Components></ComponentChain></VideoComponentChain>"#,
            r#"<VideoComponentChain ObjectID="53"><DefaultMotion>true</DefaultMotion><DefaultOpacity>true</DefaultOpacity><DefaultMotionComponentID>1</DefaultMotionComponentID><DefaultOpacityComponentID>2</DefaultOpacityComponentID><ComponentChain Version="3"><Node Version="1"></Node></ComponentChain></VideoComponentChain>"#,
        )
        .replace(
            r#"<AudioComponentChain ObjectID="73"><ComponentChain><Components><Component Index="0" ObjectRef="74"/></Components></ComponentChain></AudioComponentChain>"#,
            r#"<AudioComponentChain ObjectID="73"><DefaultVol>true</DefaultVol><DefaultVolumeComponentID>1</DefaultVolumeComponentID><DefaultChannelVolumeComponentID>2</DefaultChannelVolumeComponentID><ComponentChain Version="3"></ComponentChain><AudioChannelLayout>[{"channellabel":100},{"channellabel":101}]</AudioChannelLayout><ChannelType>1</ChannelType></AudioComponentChain>"#,
        );
    assert!(d.contains("<DefaultMotion>") && d.contains("<DefaultVol>"), "the fixture changed");
    if !tint {
        let lines: Vec<&str> =
            d.lines().filter(|l| !["\"100\"", "\"101\"", "\"102\"", "\"103\""].iter().any(|id| l.contains(&format!("ObjectID={id}")))).collect();
        d = lines.join("\n") + "\n";
    }
    d
}

/// Premiere saves no Motion, Opacity or clip Volume until one leaves its default; a value changed
/// first in FilmCraft gets the component Premiere would add, and Clip ▸ Enable is written.
#[test]
fn motion_opacity_volume_and_enable_changed_first_in_filmcraft_are_written() {
    let plain = default_chains_doc(true);
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    // nothing changed: nothing is added
    let (same, _) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(same).unwrap(), plain);
    let sid = seq_id(&p);
    {
        let seq = p.sequence_mut(sid).unwrap();
        let b = &mut seq.video_tracks[0].items[1];
        assert_eq!(b.effect("opacity").unwrap().params["opacity"].value, ParamValue::Float(100.0), "a default chain reads as the defaults");
        b.effect_mut("motion").unwrap().params.insert("scale".into(), Param::new(ParamValue::Float(150.0)));
        b.effect_mut("opacity").unwrap().params.insert("opacity".into(), Param::new(ParamValue::Float(40.0)));
        seq.video_tracks[0].items[0].enabled = false;
        let a = &mut seq.audio_tracks[0].items[0];
        a.effect_mut("volume").unwrap().params.insert("level".into(), Param::new(ParamValue::Float(-6.0)));
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let text = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    // as Premiere writes it: the flags of the components that now exist are gone, the channel
    // volume (still default) keeps its own
    assert!(!text.contains("<DefaultMotion>") && !text.contains("<DefaultOpacity>") && !text.contains("<DefaultVol>"), "{report}");
    assert!(text.contains("<DefaultChannelVolumeComponentID>2</DefaultChannelVolumeComponentID>"));
    assert_eq!(text.matches("<IsMuted>true</IsMuted>").count(), 1);
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let b = &seq.video_tracks[0].items[1];
    assert_eq!(b.effect("motion").unwrap().params["scale"].value, ParamValue::Float(150.0), "{report}");
    assert_eq!(b.effect("opacity").unwrap().params["opacity"].value, ParamValue::Float(40.0));
    assert!(!seq.video_tracks[0].items[0].enabled && b.enabled);
    let level = seq.audio_tracks[0].items[0].effect("volume").unwrap().params["level"].value.as_f64().unwrap();
    assert!((level + 6.0).abs() < 0.01, "{level}");

    // saved again with nothing changed: the same document
    let (imp2, _) = import(text.as_bytes(), None);
    let (again, _) = export(&imp2.project, text.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(prproj_graph::decompress(&again).unwrap().0).unwrap(), text);
    // enabled again: the flag goes
    let mut p2 = imp2.project;
    let sid2 = seq_id(&p2);
    p2.sequence_mut(sid2).unwrap().video_tracks[0].items[0].enabled = true;
    let (on, _) = export(&p2, text.as_bytes(), &[], None).unwrap();
    assert!(!String::from_utf8(prproj_graph::decompress(&on).unwrap().0).unwrap().contains("IsMuted"));
}

/// An effect the project has no instance of is laid out as in another Premiere project (and
/// reported when none has it); a clip with an empty chain gets a component list for it.
#[test]
fn effects_without_a_layout_here_borrow_one_from_other_projects() {
    let plain = default_chains_doc(false);
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let b = &mut p.sequence_mut(sid).unwrap().video_tracks[0].items[1];
        let mut tint = filmcraft_project::find_effect("tint").unwrap().instance();
        tint.params.insert("amount".into(), Param::new(ParamValue::Float(30.0)));
        b.effects.insert(0, tint);
    }
    // nowhere to borrow from: reported, the rest written
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(report.mentions("\"Tint\" added in FilmCraft was not written"), "{report}");
    assert!(read(&out).sequences().next().and_then(|i| i.as_sequence()).unwrap().video_tracks[0].items[1].effect("tint").is_none());
    // another project has one
    let models = vec![doc().into_bytes()];
    let (out, report) = export_with(&p, plain.as_bytes(), &[], None, &models).unwrap();
    assert!(report.mentions("borrowed from your other Premiere projects") && !report.mentions("was not written"), "{report}");
    let text = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    assert_eq!(text.matches("<MatchName>AE.ADBE Tint</MatchName>").count(), 1, "the borrowed model itself is not left behind");
    let back = read(&out);
    let b = &back.sequences().next().and_then(|i| i.as_sequence()).unwrap().video_tracks[0].items[1];
    assert_eq!(b.effect("tint").expect("tint written").params["amount"].value, ParamValue::Float(30.0));
}

/// Sequence Settings: the name, frame size and timebase are written; the sample rate and master
/// channels, which Premiere fixes when a sequence is made, are reported.
#[test]
fn sequence_settings_are_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    p.item_mut(sid).unwrap().name = "Final Cut".into();
    {
        let st = &mut p.sequence_mut(sid).unwrap().settings;
        st.width = 1080;
        st.height = 1920;
        st.frame_rate = filmcraft_time::FrameRate::new(25, 1);
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(!report.mentions("sample rate"), "{report}");
    let back = read(&out);
    let it = back.sequences().next().unwrap();
    let st = &it.as_sequence().unwrap().settings;
    assert_eq!(
        (it.name.as_str(), st.width, st.height, st.frame_rate.frame_duration()),
        ("Final Cut", 1080, 1920, filmcraft_time::FrameRate::new(25, 1).frame_duration())
    );
    p.sequence_mut(sid).unwrap().settings.sample_rate = 44_100;
    let (_, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(report.mentions("sample rate (44100 Hz) was not written"), "{report}");
}

/// Tracks added in FilmCraft are written even while empty; sequences and bins made in FilmCraft
/// are written too.
#[test]
fn added_tracks_new_sequences_and_bins_are_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    let (vt, at) = (TrackId(p.alloc_id()), TrackId(p.alloc_id()));
    {
        let seq = p.sequence_mut(sid).unwrap();
        seq.video_tracks.push(Track::new(vt, TrackKind::Video, "V2".into()));
        seq.audio_tracks.push(Track::new(at, TrackKind::Audio, "A2".into()));
    }
    let settings = p.sequence(sid).unwrap().settings.clone();
    p.new_sequence("Made Here", settings, 1, 1, None);
    let bin = filmcraft_project::BinId(p.alloc_id());
    p.root.children.push(filmcraft_project::BinEntry::Bin(filmcraft_project::Bin { id: bin, name: "Selects".into(), children: Vec::new() }));
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let back = read(&out);
    let seq = back.sequences().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    assert_eq!((seq.video_tracks.len(), seq.audio_tracks.len()), (2, 2), "{report}");
    assert!(back.sequences().any(|i| i.name == "Made Here"), "{report}");
    let bins: Vec<&str> =
        back.root.children.iter().filter_map(|e| if let filmcraft_project::BinEntry::Bin(b) = e { Some(b.name.as_str()) } else { None }).collect();
    assert!(bins.contains(&"Selects") && bins.contains(&"Footage"), "{bins:?} {report}");
}

/// A sequence made in FilmCraft with a clip of the project's media, an adjustment layer and a
/// colour matte made in FilmCraft, and the media moved into a new bin: all written, and read back.
#[test]
fn new_sequences_synthetic_items_and_bins_are_built() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    let settings = p.sequence(sid).unwrap().settings.clone();
    let media = p.items.values().find(|i| i.name == "interview.mov").map(|i| i.id).unwrap();
    let template = p.sequence(sid).unwrap().video_tracks[0].items[0].clone();
    let adj = p.add_item(
        "Grade",
        filmcraft_project::Label::Lavender,
        ItemKind::AdjustmentLayer { width: 1920, height: 1080, rate: filmcraft_time::FrameRate::new(30, 1), duration: Tick(i64::MAX / 4) },
        None,
    );
    let info = filmcraft_media::MediaInfo {
        name: "Orange".into(),
        kind: filmcraft_media::MediaKind::Still,
        duration: Tick(5 * S),
        audio: None,
        container: String::new(),
        start_timecode: None,
        file_size: None,
        video: Some(filmcraft_media::VideoStreamInfo {
            width: 1920,
            height: 1080,
            frame_rate: filmcraft_time::FrameRate::new(30, 1),
            par: (1, 1),
            codec: String::new(),
            pixel_format: String::new(),
            color: filmcraft_color::ColorInfo::REC709,
            has_alpha: false,
            bitrate: None,
            hdr: None,
        }),
    };
    let matte = p.add_item(
        "Orange",
        filmcraft_project::Label::Iris,
        ItemKind::Media(filmcraft_project::MediaClip {
            media: filmcraft_project::MediaRef::Generator(filmcraft_media::generators::Generator::ColorMatte { color: [1.0, 0.5, 0.0, 1.0] }),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let new_seq = p.new_sequence("Made Here", settings, 2, 1, None);
    {
        let q = p.sequence_mut(new_seq).unwrap();
        let mut c = template.clone();
        c.id = ClipId(70_001);
        c.origin = None;
        c.link = None;
        c.start = Tick(0);
        c.effect_mut("motion").unwrap().params.insert("scale".into(), Param::new(ParamValue::Float(130.0)));
        q.video_tracks[0].items.push(c.clone());
        let mut a = c.clone();
        a.id = ClipId(70_002);
        a.item = adj;
        // dragged left of where it was made: endless media, written from 0 at the same place
        a.source_in = Tick(-10 * S);
        a.effects = filmcraft_project::effect::intrinsic_video();
        q.video_tracks[1].items.push(a);
        let mut m = c;
        m.id = ClipId(70_003);
        m.item = matte;
        m.start = Tick(5 * S);
        m.source_in = Tick(0);
        m.effects = filmcraft_project::effect::intrinsic_video();
        q.video_tracks[0].items.push(m);
    }
    // the media goes into a new bin "Selects"
    fn take(b: &mut filmcraft_project::Bin, id: ItemId) -> bool {
        let before = b.children.len();
        b.children.retain(|e| !matches!(e, filmcraft_project::BinEntry::Item(i) if *i == id));
        before != b.children.len()
            || b.children.iter_mut().any(|e| match e {
                filmcraft_project::BinEntry::Bin(c) => take(c, id),
                _ => false,
            })
    }
    assert!(take(&mut p.root, media));
    let bin = filmcraft_project::BinId(p.alloc_id());
    p.root.children.push(filmcraft_project::BinEntry::Bin(filmcraft_project::Bin {
        id: bin,
        name: "Selects".into(),
        children: vec![filmcraft_project::BinEntry::Item(media)],
    }));

    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    assert!(!report.mentions("not written"), "{report}");
    let back = read(&out);
    let q = back.sequences().find(|i| i.name == "Made Here").and_then(|i| i.as_sequence()).expect("the new sequence");
    let v1: Vec<&str> = q.video_tracks[0].items.iter().map(|c| back.item(c.item).unwrap().name.as_str()).collect();
    assert_eq!(v1, ["interview.mov", "Orange"], "{report}");
    assert_eq!(q.video_tracks[0].items[0].effect("motion").unwrap().params["scale"].value, ParamValue::Float(130.0));
    assert!(matches!(back.item(q.video_tracks[1].items[0].item).unwrap().kind, ItemKind::AdjustmentLayer { .. }));
    assert_eq!((q.video_tracks[1].items[0].start, q.video_tracks[1].items[0].source_in), (Tick(0), Tick(0)));
    let orange = back.item(q.video_tracks[0].items[1].item).unwrap();
    let ItemKind::Media(m) = &orange.kind else { panic!("{orange:?}") };
    let filmcraft_project::MediaRef::Generator(filmcraft_media::generators::Generator::ColorMatte { color }) = &m.media else { panic!("{m:?}") };
    assert!((color[0] - 1.0).abs() < 0.01 && (color[1] - 0.5).abs() < 0.01 && color[2].abs() < 0.01, "{color:?}");
    // the bins: the media in "Selects", the new items at the root
    let selects = back.root.children.iter().find_map(|e| match e {
        filmcraft_project::BinEntry::Bin(b) if b.name == "Selects" => Some(b),
        _ => None,
    });
    let in_selects: Vec<&str> = selects
        .expect("Selects")
        .children
        .iter()
        .filter_map(|e| if let filmcraft_project::BinEntry::Item(i) = e { back.item(*i).map(|x| x.name.as_str()) } else { None })
        .collect();
    assert_eq!(in_selects, ["interview.mov"]);
    let text = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    // Premiere applies the adjustment layer only through clips marked as one (unmarked, it plays
    // its black video over everything below); a float's whole value has a point, as Premiere
    // writes it (this document's own Scale is written without, so it keeps that)
    assert_eq!(adjustment_clips(&text), (1, 0), "every adjustment layer clip is marked");
    assert!(float_values_without_point(&text).is_empty(), "{:?}", float_values_without_point(&text));
    // saved again unchanged: the same document
    let (again_imp, _) = import(text.as_bytes(), None);
    let (again, _) = export(&again_imp.project, text.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(prproj_graph::decompress(&again).unwrap().0).unwrap(), text);
}

/// Adjustment layer timeline clips (the `VideoClip` of a `SubClip` of a master clip marked
/// `IsAdjustmentLayer`): (marked `AdjustmentLayer`, not marked).
fn adjustment_clips(xml: &str) -> (usize, usize) {
    let d = crate::xml::parse(xml).unwrap();
    let root = d.root_element();
    let adj: Vec<&str> = root
        .children()
        .filter(|n| n.has_tag_name("MasterClip") && n.children().any(|c| c.has_tag_name("IsAdjustmentLayer") && c.text() == Some("true")))
        .filter_map(|n| n.attribute("ObjectUID"))
        .collect();
    let (mut yes, mut no) = (0, 0);
    for sub in root.children().filter(|n| n.has_tag_name("SubClip")) {
        if !sub.children().any(|c| c.has_tag_name("MasterClip") && c.attribute("ObjectURef").is_some_and(|u| adj.contains(&u))) {
            continue;
        }
        let id = sub.children().find(|c| c.has_tag_name("Clip")).and_then(|c| c.attribute("ObjectRef")).unwrap();
        let clip = root.children().find(|n| n.attribute("ObjectID") == Some(id)).unwrap();
        if clip.children().any(|c| c.has_tag_name("AdjustmentLayer") && c.text() == Some("true")) {
            yes += 1;
        } else {
            no += 1;
        }
    }
    (yes, no)
}

/// Float parameter values written without the point Premiere always writes (`150` for `150.`).
fn float_values_without_point(xml: &str) -> Vec<String> {
    let d = crate::xml::parse(xml).unwrap();
    d.root_element()
        .children()
        .filter(|n| n.has_tag_name("VideoComponentParam") && n.attribute("ClassID").is_some_and(|c| c.starts_with("fe47129e")))
        .filter_map(|n| n.children().find(|c| c.has_tag_name("StartKeyframe")).and_then(|c| c.text()))
        .filter(|t| t.split(',').nth(1).is_some_and(|v| !v.contains('.') && !v.contains('e')))
        .map(str::to_string)
        .collect()
}

/// A project an earlier FilmCraft saved with unmarked adjustment layer clips (Premiere showed
/// black under them) and a float written as `150`: saving it again repairs both, and says so.
#[test]
fn adjustment_layer_clips_saved_unmarked_are_repaired() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    let adj = p.add_item(
        "Zoom",
        filmcraft_project::Label::Lavender,
        ItemKind::AdjustmentLayer { width: 1920, height: 1080, rate: filmcraft_time::FrameRate::new(30, 1), duration: Tick(i64::MAX / 4) },
        None,
    );
    {
        let q = p.sequence_mut(sid).unwrap();
        let mut a = q.video_tracks[0].items[0].clone();
        a.id = ClipId(80_001);
        a.origin = None;
        a.link = None;
        a.item = adj;
        a.source_in = Tick(0);
        a.effects = filmcraft_project::effect::intrinsic_video();
        a.effect_mut("motion").unwrap().params.insert("scale".into(), Param::new(ParamValue::Float(150.0)));
        q.video_tracks.push(Track::new(TrackId(80_000), filmcraft_project::TrackKind::Video, "V3".into()));
        q.video_tracks.last_mut().unwrap().items.push(a);
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let good = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    assert_eq!(adjustment_clips(&good), (1, 0), "{report}");
    assert!(!report.mentions("earlier FilmCraft"), "nothing to repair in a fresh save: {report}");
    // as the earlier version wrote it
    let old = good.replace("<AdjustmentLayer>true</AdjustmentLayer>", "").replace(",150.,", ",150,");
    assert_eq!(adjustment_clips(&old), (0, 1));
    assert!(!float_values_without_point(&old).is_empty());
    let (old_imp, _) = import(old.as_bytes(), None);
    let (fixed, report) = export(&old_imp.project, old.as_bytes(), &[], None).unwrap();
    let fixed = String::from_utf8(prproj_graph::decompress(&fixed).unwrap().0).unwrap();
    assert_eq!(adjustment_clips(&fixed), (1, 0), "{report}");
    assert!(float_values_without_point(&fixed).is_empty(), "{:?}", float_values_without_point(&fixed));
    assert!(report.mentions("1 adjustment layer clip(s) saved by an earlier FilmCraft were marked"), "{report}");
    assert_eq!(fixed, good, "repaired to what a fresh save writes");
}

/// Layouts learned from a layout kit (effects at default settings) are models even when
/// FilmCraft does not map every setting; the settings FilmCraft changed that have no place are
/// reported.
#[test]
fn layout_kit_layouts_are_models_and_unwritten_settings_are_reported() {
    // a project with a Tint whose third setting has a name FilmCraft does not know
    let kit = doc().replace("<Name>Amount to Tint</Name>", "<Name>Something Else</Name>");
    assert!(crate::prproj_new::layouts_in(kit.as_bytes(), false).iter().all(|(n, _, _)| n.key() != "effect.tint"), "not a model unless at defaults");
    let learned = crate::prproj_new::layouts_in(kit.as_bytes(), true);
    let (_, _, tint) = learned.iter().find(|(n, _, _)| n.key() == "effect.tint").expect("learned at defaults");
    let model = crate::prproj_new::layout_document(tint, true).unwrap();
    assert!(model.contains(crate::prproj_new::AT_DEFAULTS));
    // a project without a Tint adds one modelled on the kit's
    let plain = default_chains_doc(false);
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let b = &mut p.sequence_mut(sid).unwrap().video_tracks[0].items[1];
        let mut t = filmcraft_project::find_effect("tint").unwrap().instance();
        t.params.insert("amount".into(), Param::new(ParamValue::Float(30.0)));
        b.effects.insert(0, t);
    }
    let (out, report) = export_with(&p, plain.as_bytes(), &[], None, &[model.into_bytes()]).unwrap();
    assert!(report.mentions("\"Tint\" on \"backwards\": Amount to Tint not written"), "{report}");
    assert!(read(&out).sequences().next().and_then(|i| i.as_sequence()).unwrap().video_tracks[0].items[1].effect("tint").is_some(), "{report}");
}

/// The fixture other crates' tests use is this document.
#[test]
fn the_shared_test_document_is_current() {
    assert_eq!(crate::prproj::test_document(), doc());
}

/// A clip that starts before its source (an In point below 0, as a nested sequence or a media
/// clip got in one user's project) is saved instead of failing the whole save: the empty part is
/// left out, so what is seen stays at the same sequence time.
#[test]
fn a_clip_starting_before_its_source_is_saved() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        // the first clip: 0–5 s of the sequence, its source from -3 s
        let a = &mut p.sequence_mut(sid).unwrap().video_tracks[0].items[0];
        assert_eq!((a.start, a.duration), (Tick(0), Tick(5 * S)));
        a.source_in = Tick(-3 * S);
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).expect("saved");
    assert!(report.mentions("started before its source"), "{report}");
    let back = read(&out);
    let a = &back.sequences().next().and_then(|i| i.as_sequence()).unwrap().video_tracks[0].items[0];
    // the source's 0 s was at sequence 3 s, and still is
    assert_eq!((a.start, a.duration, a.source_in), (Tick(3 * S), Tick(2 * S), Tick(0)), "{report}");
}

/// A marker added in FilmCraft goes on the `Sequence` (where Premiere lists sequence markers; on
/// the master clip it was invisible there); the master clip's own markers stay where they were.
/// Clip gain is written as Premiere's linear `Gain` and read back.
#[test]
fn new_sequence_markers_go_on_the_sequence_and_clip_gain_is_written() {
    let plain = doc();
    let (imp, _) = import(plain.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    {
        let q = p.sequence_mut(sid).unwrap();
        assert_eq!(q.markers.len(), 1, "the fixture's master clip marker");
        let mut m = q.markers[0].clone();
        m.name = "Round trip".into();
        m.start = Tick(3 * S);
        m.comment.clear();
        q.markers.push(m);
        q.audio_tracks[0].items[0].gain_db = 3.0;
    }
    let (out, report) = export(&p, plain.as_bytes(), &[], None).unwrap();
    let text = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    let d = crate::xml::parse(&text).unwrap();
    let root = d.root_element();
    let seq = root.children().find(|n| n.has_tag_name("Sequence")).unwrap();
    let owner = seq.children().find(|c| c.has_tag_name("MarkerOwner")).expect("a MarkerOwner on the Sequence");
    let id = owner.children().find(|c| c.has_tag_name("Markers")).and_then(|c| c.attribute("ObjectRef")).unwrap();
    let list = root.children().find(|n| n.attribute("ObjectID") == Some(id)).unwrap();
    let entries = list.descendants().filter(|n| n.has_tag_name("Marker")).count();
    assert_eq!(entries, 1, "only the new marker is on the Sequence: {report}");
    // the master clip's list still holds its own marker only
    let master_list = root.children().find(|n| n.attribute("ObjectID") == Some("22")).unwrap();
    assert_eq!(master_list.descendants().filter(|n| n.has_tag_name("Marker")).count(), 1);
    assert!(text.contains("<Gain>1.41253754462275"), "+3 dB as a linear gain");
    let back = read(&out);
    let q = back.sequences().find_map(|i| i.as_sequence()).unwrap();
    let names: Vec<&str> = q.markers.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Act 2", "Round trip"]);
    assert!((q.audio_tracks[0].items[0].gain_db - 3.0).abs() < 1e-6);
    // saved again unchanged: the same document
    let (again_imp, _) = import(text.as_bytes(), None);
    let (again, _) = export(&again_imp.project, text.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(prproj_graph::decompress(&again).unwrap().0).unwrap(), text);
}

/// Premiere's audio effects come in with their settings in FilmCraft's units (scales measured in
/// Premiere: Bass Boost −24…+24 dB, a Single-band Compressor's Threshold −60…0 dB; a setting at
/// the top of its range is saved with no value) and switched on; edits are written back on
/// Premiere's 0…1 scale.
#[test]
fn audio_effect_settings_read_and_write_in_their_units() {
    let comps = "<AudioFilterComponent ObjectID=\"900\"><AudioComponent><Component><Params><Param Index=\"0\" ObjectRef=\"901\"/><Param Index=\"1\" ObjectRef=\"902\"/></Params></Component></AudioComponent><FilterMatchName>Internal Bass</FilterMatchName></AudioFilterComponent>\n  \
         <AudioComponentParam ObjectID=\"901\" ClassID=\"32657501-3aa4-445f-a49b-d09ecb9fa1ae\"><Name>Bypass</Name></AudioComponentParam>\n  \
         <AudioComponentParam ObjectID=\"902\" ClassID=\"a714635e-a628-4b27-9d59-77eba47dbc1a\"><StartKeyframe>-91445760000000000,0.75,0,0,0,0,0,0</StartKeyframe><CurrentValue>0.75</CurrentValue><Name>Boost</Name></AudioComponentParam>\n  \
         <AudioFilterComponent ObjectID=\"910\"><AudioComponent><Component><Params><Param Index=\"0\" ObjectRef=\"911\"/><Param Index=\"1\" ObjectRef=\"912\"/></Params></Component></AudioComponent><FilterMatchName>2da3e2bc-5005-481c-bbeb-7689d3169edf</FilterMatchName></AudioFilterComponent>\n  \
         <AudioComponentParam ObjectID=\"911\" ClassID=\"32657501-3aa4-445f-a49b-d09ecb9fa1ae\"><Name>Bypass</Name></AudioComponentParam>\n  \
         <AudioComponentParam ObjectID=\"912\" ClassID=\"a714635e-a628-4b27-9d59-77eba47dbc1a\"><Name>Threshold</Name></AudioComponentParam>\n";
    let text = doc()
        .replace(
            "<Components><Component Index=\"0\" ObjectRef=\"74\"/></Components>",
            "<Components><Component Index=\"0\" ObjectRef=\"74\"/><Component Index=\"1\" ObjectRef=\"900\"/><Component Index=\"2\" ObjectRef=\"910\"/></Components>",
        )
        .replace("</PremiereData>", &format!("  {comps}</PremiereData>"));
    assert!(text.contains("ObjectRef=\"910\""), "the fixture changed");
    let (imp, report) = import(text.as_bytes(), None);
    let mut p = imp.project;
    let sid = seq_id(&p);
    let a1 = &p.sequence(sid).unwrap().audio_tracks[0].items[0];
    let bass = a1.effect("bass").expect("Bass");
    assert!(bass.enabled, "read, so switched on: {report}");
    assert_eq!(bass.params["boost"].value, ParamValue::Float(12.0), "0.75 of −24…+24 dB");
    let comp = a1.effect("single_band_compressor").expect("compressor");
    assert_eq!(comp.params["threshold"].value, ParamValue::Float(0.0), "no value: the top of −60…0 dB");
    {
        let a1 = &mut p.sequence_mut(sid).unwrap().audio_tracks[0].items[0];
        a1.effect_mut("bass").unwrap().params.insert("boost".into(), Param::new(ParamValue::Float(-12.0)));
        a1.effect_mut("single_band_compressor").unwrap().params.insert("threshold".into(), Param::new(ParamValue::Float(-30.0)));
    }
    let (out, report) = export(&p, text.as_bytes(), &[], None).unwrap();
    let written = String::from_utf8(prproj_graph::decompress(&out).unwrap().0).unwrap();
    assert!(written.contains("-91445760000000000,0.25,"), "−12 dB is 0.25: {report}");
    assert!(written.contains("-91445760000000000,0.5,"), "−30 dB is 0.5");
    let back = read(&out);
    let a1 = &back.sequences().find_map(|i| i.as_sequence()).unwrap().audio_tracks[0].items[0];
    assert_eq!(a1.effect("bass").unwrap().params["boost"].value, ParamValue::Float(-12.0));
    assert_eq!(a1.effect("single_band_compressor").unwrap().params["threshold"].value, ParamValue::Float(-30.0));
    // unchanged: the same document
    let (again_imp, _) = import(text.as_bytes(), None);
    let (again, _) = export(&again_imp.project, text.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(prproj_graph::decompress(&again).unwrap().0).unwrap(), text);
}
