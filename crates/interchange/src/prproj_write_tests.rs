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

#[test]
fn clips_premiere_cannot_place_are_reported_not_written() {
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
    assert!(report.mentions("new.mov") && report.mentions("mxf"), "{report}");
    let back = read(&out);
    let seq = back.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(seq.video_tracks[0].items.len(), 2);
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
