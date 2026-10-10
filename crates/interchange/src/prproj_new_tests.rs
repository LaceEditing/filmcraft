use super::*;
use crate::common::{Builder, MediaSpec};
use filmcraft_project::{Label, MarkerId, MarkerKind, Param, ParamValue};
use filmcraft_time::{FrameRate, Tick};

const S: i64 = TICKS_PER_SECOND;

fn read(bytes: &[u8]) -> (Project, Report) {
    let mut report = Report::default();
    let opts = crate::ImportOptions { base_dir: Some("/edits/new".into()), ..Default::default() };
    let imported = crate::prproj::import(bytes, &opts, &mut report).unwrap();
    (imported.project, report)
}

fn marker(start: i64, name: &str) -> Marker {
    Marker {
        id: MarkerId(0),
        start: Tick(start),
        duration: Tick::ZERO,
        name: name.into(),
        comment: "note".into(),
        kind: MarkerKind::Comment,
        color: Label::Green,
    }
}

/// A FilmCraft project (no Premiere ids): a movie with stereo sound, a mono WAV, a still, a nested
/// sequence and a bin.
fn project() -> (Project, ItemId, ItemId) {
    let mut b = Builder::new("Fresh");
    let shots = b.bin("Shots", None);
    let movie = b.file_media(
        "m",
        "Interview.mp4",
        "/mnt/LocalDisk/Editing/Fresh/Interview.mp4",
        &MediaSpec { duration: Some(Tick(60 * S)), video: Some((1920, 1080, FrameRate::FPS_30)), audio: Some((48_000, 2)), ..Default::default() },
        Some(shots),
    );
    let wav = b.file_media(
        "w",
        "Voice.wav",
        "/mnt/LocalDisk/Editing/Fresh/audio/Voice.wav",
        &MediaSpec { duration: Some(Tick(30 * S)), audio: Some((44_100, 1)), ..Default::default() },
        None,
    );
    let still = b.file_media(
        "s",
        "Logo.png",
        "/home/me/Pictures/Logo.png",
        &MediaSpec { duration: Some(Tick(5 * S)), video: Some((800, 600, FrameRate::FPS_30)), kind: Some(MediaKind::Still), ..Default::default() },
        None,
    );
    let settings = crate::common::settings_for(FrameRate::FPS_30, 1920, 1080, false);
    let inner = b.reserve_sequence("Inner", settings.clone(), None);
    let main = b.reserve_sequence("Main Edit", settings.clone(), None);
    // the nested sequence: 4 s of the movie
    let mut seq = crate::common::empty_sequence(settings.clone());
    let c = b.clip(movie, TrackKind::Video, "Interview.mp4", Tick::ZERO, Tick(4 * S), Tick(20 * S));
    b.place(&mut seq, TrackKind::Video, 0, c);
    b.put_sequence(inner, seq);
    // the main sequence
    let mut seq = crate::common::empty_sequence(settings);
    let link = b.link_id();
    let mut v = b.clip(movie, TrackKind::Video, "Interview.mp4", Tick::ZERO, Tick(10 * S), Tick(5 * S));
    v.link = Some(link);
    let scale = v.effect_mut("motion").unwrap().params.get_mut("scale").unwrap();
    *scale = Param::new(ParamValue::Float(150.0));
    let mut a = b.clip(movie, TrackKind::Audio, "Interview.mp4", Tick::ZERO, Tick(10 * S), Tick(5 * S));
    a.link = Some(link);
    a.effect_mut("volume").unwrap().params.insert("level".into(), Param::new(ParamValue::Float(-6.0)));
    b.place(&mut seq, TrackKind::Video, 0, v);
    b.place(&mut seq, TrackKind::Audio, 0, a);
    // 2x speed, then a held frame
    let mut fast = b.clip(movie, TrackKind::Video, "Interview.mp4", Tick(10 * S), Tick(2 * S), Tick(30 * S));
    fast.speed = 2.0;
    b.place(&mut seq, TrackKind::Video, 0, fast);
    let mut held = b.clip(movie, TrackKind::Video, "Interview.mp4", Tick(12 * S), Tick(S), Tick(40 * S));
    held.frame_hold = Some(Tick(41 * S));
    b.place(&mut seq, TrackKind::Video, 0, held);
    let mut logo = b.clip(still, TrackKind::Video, "Logo.png", Tick(2 * S), Tick(3 * S), Tick::ZERO);
    logo.effect_mut("opacity").unwrap().params.insert("opacity".into(), Param::new(ParamValue::Float(50.0)));
    b.place(&mut seq, TrackKind::Video, 1, logo);
    let voice = b.clip(wav, TrackKind::Audio, "Voice.wav", Tick(3 * S), Tick(5 * S), Tick(S));
    b.place(&mut seq, TrackKind::Audio, 1, voice);
    let nest = b.clip(inner, TrackKind::Video, "Inner", Tick(13 * S), Tick(4 * S), Tick::ZERO);
    b.place(&mut seq, TrackKind::Video, 0, nest);
    seq.markers.push(marker(2 * S, "Intro"));
    seq.audio_tracks[0].volume_db = -3.0;
    b.put_sequence(main, seq);
    let mut p = b.finish().project;
    if let Some(m) = p.item_mut(movie).and_then(|i| i.as_media_mut()) {
        m.markers.push(marker(7 * S, "Laugh"));
    }
    (p, main, inner)
}

fn opts() -> CreateOptions {
    CreateOptions { dest_dir: Some("/mnt/LocalDisk/Editing/Fresh".into()), path_map: vec![("/mnt/LocalDisk".into(), "L:".into())], models: Vec::new() }
}

fn clips(p: &Project, name: &str, kind: TrackKind) -> Vec<(usize, i64, i64, i64, f64, Option<i64>, String)> {
    let seq = p.items.values().find(|i| i.name == name).and_then(|i| i.as_sequence()).unwrap();
    let mut v: Vec<_> = seq
        .tracks(kind)
        .iter()
        .enumerate()
        .flat_map(|(ti, t)| {
            t.items.iter().map(move |c| {
                (ti, c.start.0, c.duration.0, c.source_in.0, c.speed, c.frame_hold.map(|t| t.0), p.item(c.item).map(|i| i.name.clone()).unwrap_or_default())
            })
        })
        .collect();
    v.sort_by_key(|c| (c.0, c.1));
    v
}

#[test]
fn a_filmcraft_project_becomes_a_premiere_project() {
    let (p, _, _) = project();
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(bytes.starts_with(&[0x1f, 0x8b]), "Premiere saves gzip");
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(crate::prproj_write::problems(&text).is_empty(), "{:?}", crate::prproj_write::problems(&text));
    // the templates are gone: every track item is on a track
    let doc = crate::xml::parse(&text).unwrap();
    let items = doc.root_element().children().filter(|n| n.has_tag_name("VideoClipTrackItem") || n.has_tag_name("AudioClipTrackItem")).count();
    assert_eq!(items, 5 + 2 + 1, "{report}");

    let (back, _) = read(text.as_bytes());
    for (name, kind) in [("Main Edit", TrackKind::Video), ("Main Edit", TrackKind::Audio), ("Inner", TrackKind::Video)] {
        assert_eq!(clips(&back, name, kind), clips(&p, name, kind), "{name} {kind:?}");
    }
    // bins, names, media paths as the Windows machine sees them
    let shots = back.root.children.iter().find_map(|c| match c {
        BinEntry::Bin(b) if b.name == "Shots" => Some(b),
        _ => None,
    });
    assert!(shots.is_some_and(|b| b.children.len() == 1), "the movie is in its bin");
    assert!(text.contains("<FilePath>L:\\Editing\\Fresh\\Interview.mp4</FilePath>"));
    assert!(text.contains("<RelativePath>.\\audio\\Voice.wav</RelativePath>"));
    assert!(text.contains("<FilePath>/home/me/Pictures/Logo.png</FilePath>"), "no mapping: the local path");
    assert!(text.contains("<Infinite>true</Infinite>") && text.contains("<IsStill>true</IsStill>"));
}

#[test]
fn motion_opacity_volume_links_and_markers_are_written() {
    let (p, _, _) = project();
    let (bytes, report) = create(&p, &opts()).unwrap();
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let v1 = &seq.video_tracks[0];
    let first = v1.items.iter().find(|c| c.start.0 == 0).unwrap();
    assert_eq!(first.effect("motion").and_then(|e| e.params.get("scale")).map(|p| p.value.clone()), Some(ParamValue::Float(150.0)), "{report}");
    let logo = seq.video_tracks[1].items.first().unwrap();
    assert_eq!(logo.effect("opacity").and_then(|e| e.params.get("opacity")).map(|p| p.value.clone()), Some(ParamValue::Float(50.0)));
    let a1 = seq.audio_tracks[0].items.first().unwrap();
    let ParamValue::Float(level) = a1.effect("volume").and_then(|e| e.params.get("level")).map(|p| p.value.clone()).unwrap() else { panic!() };
    assert!((level + 6.0).abs() < 1e-6, "{level}");
    // the linked pair stays linked; the others are not linked
    assert!(first.link.is_some() && first.link == a1.link);
    assert!(seq.audio_tracks[1].items[0].link.is_none());
    assert_eq!(seq.markers.iter().map(|m| (m.start.0, m.name.as_str())).collect::<Vec<_>>(), vec![(2 * S, "Intro")]);
    let movie = back.items.values().find(|i| i.name == "Interview.mp4").and_then(|i| i.as_media()).unwrap();
    assert_eq!(movie.markers.iter().map(|m| (m.start.0, m.name.as_str(), m.comment.as_str())).collect::<Vec<_>>(), vec![(7 * S, "Laugh", "note")]);
    // the mono file stays mono; the track fader holds the track volume (-3 dB, linear)
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(text.contains("Internal Volume Mono") && text.contains("Internal Volume Stereo") && text.contains("Internal Channel Volume Stereo"));
    assert!(text.contains(&format!("<StartKeyframe>{NO_TIME},0.70794578438")), "-3 dB on the A1 fader");
}

#[test]
fn what_a_new_premiere_project_cannot_hold_is_reported() {
    let (mut p, main, _) = project();
    let blur = filmcraft_project::effect::find_effect("gaussian_blur").unwrap().instance();
    let seq = p.sequence_mut(main).unwrap();
    seq.video_tracks[0].items[0].effects.push(blur);
    seq.video_tracks[0].items[0].origin = Some("12".into());
    let (bytes, report) = create(&p, &opts()).unwrap();
    let notes = report.to_string();
    assert!(notes.contains("Gaussian Blur (1 clip)"), "{notes}");
    // the clip itself is still written
    let (back, _) = read(&bytes);
    assert_eq!(clips(&back, "Main Edit", TrackKind::Video), clips(&project().0, "Main Edit", TrackKind::Video));
}

#[test]
fn a_project_without_sequences_is_refused() {
    let mut b = Builder::new("Empty");
    b.file_media("m", "a.mp4", "/x/a.mp4", &MediaSpec { duration: Some(Tick(S)), video: Some((640, 480, FrameRate::FPS_30)), ..Default::default() }, None);
    assert!(create(&b.finish().project, &CreateOptions::default()).is_err());
}

#[test]
fn every_export_gets_its_own_ids() {
    let (p, _, _) = project();
    let uids = |b: &[u8]| -> HashSet<String> {
        let (xml, _) = prproj_graph::decompress(b).unwrap();
        let text = String::from_utf8(xml).unwrap();
        let doc = crate::xml::parse(&text).unwrap();
        doc.root_element().children().filter_map(|n| n.attribute("ObjectUID").map(str::to_string)).collect()
    };
    let (a, _) = create(&p, &opts()).unwrap();
    let (b, _) = create(&p, &opts()).unwrap();
    assert!(uids(&a).is_disjoint(&uids(&b)));
}

#[test]
fn paths_map_onto_the_premiere_machine() {
    let map = vec![("/mnt/LocalDisk".to_string(), "L:".to_string()), ("/mnt/LocalDisk/Shared".to_string(), "S:\\".to_string())];
    assert_eq!(premiere_path("/mnt/LocalDisk/Editing/a b.mp4", &map), "L:\\Editing\\a b.mp4");
    assert_eq!(premiere_path("/mnt/LocalDisk/Shared/x.wav", &map), "S:\\x.wav", "the longest prefix wins");
    assert_eq!(premiere_path("/mnt/LocalDiskOther/x.wav", &map), "/mnt/LocalDiskOther/x.wav", "whole path components only");
    assert_eq!(premiere_path("/home/x.wav", &map), "/home/x.wav");
}

/// The project with a blurred, tinted clip and a cross dissolve into the next one.
fn with_effects() -> Project {
    let (mut p, main, _) = project();
    let transition_id = filmcraft_project::TransitionId(p.alloc_id());
    let seq = p.sequence_mut(main).unwrap();
    let v1 = &mut seq.video_tracks[0];
    let mut blur = filmcraft_project::effect::find_effect("gaussian_blur").unwrap().instance();
    blur.params.insert("blurriness".into(), Param::new(ParamValue::Float(12.0)));
    v1.items[0].effects.push(blur);
    v1.items[1].effects.push(filmcraft_project::effect::find_effect("tint").unwrap().instance());
    let (from, to) = (v1.items[0].id, v1.items[1].id);
    let mut dissolve = crate::common::transition_effect("Cross Dissolve", false, &mut Report::default());
    dissolve.enabled = true;
    v1.transitions.push(filmcraft_project::Transition {
        id: transition_id,
        effect: dissolve,
        start: Tick(9 * S),
        duration: Tick(2 * S),
        from: Some(from),
        to: Some(to),
        align: filmcraft_project::TransitionAlign::CenterAtCut,
        reverse: false,
    });
    p
}

#[test]
fn effects_and_transitions_are_laid_out_as_in_premiere_projects() {
    let p = with_effects();
    let model = crate::prproj::tests::doc();
    let o = CreateOptions { models: vec![crate::prproj::tests::gzip(model.as_bytes())], ..opts() };
    let (bytes, report) = create(&p, &o).unwrap();
    let notes = report.to_string();
    assert!(notes.contains("2 effect(s) and 1 transition type(s)"), "{notes}");
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let v1 = &seq.video_tracks[0];
    let blur = v1.items[0].effect("gaussian_blur").expect("the blur is written");
    // FilmCraft's settings, not the borrowed clip's (which was animated)
    let level = blur.params.get("blurriness").unwrap();
    assert_eq!((level.value.clone(), level.keyframes.len()), (ParamValue::Float(12.0), 0));
    assert!(v1.items[1].effect("tint").is_some());
    let t = v1.transitions.first().expect("the dissolve is written");
    assert_eq!((t.start.0, t.duration.0, t.from, t.to), (9 * S, 2 * S, Some(v1.items[0].id), Some(v1.items[1].id)));
    // the borrowed layouts themselves are not left in the project
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert_eq!(text.matches("<DisplayName>Gaussian Blur (Legacy)</DisplayName>").count(), 1);
    assert_eq!(text.matches("<VideoTransitionTrackItem ").count(), 1);
}

#[test]
fn layouts_with_settings_filmcraft_lacks_are_not_borrowed() {
    let p = with_effects();
    // the model's blur has a setting FilmCraft does not know
    let model = crate::prproj::tests::doc().replace(
        r#"<Param Index="2" ObjectRef="59"/></Params><ID>2</ID>"#,
        r#"<Param Index="2" ObjectRef="59"/><Param Index="3" ObjectRef="900"/></Params><ID>2</ID>"#,
    );
    let model = model.replace("</PremiereData>", r#"<VideoComponentParam ObjectID="900"><Name>Secret Sauce</Name><StartKeyframe>-91445760000000000,7,0,0,0,0,0,0</StartKeyframe></VideoComponentParam></PremiereData>"#);
    let (bytes, report) = create(&p, &CreateOptions { models: vec![model.into_bytes()], ..opts() }).unwrap();
    let notes = report.to_string();
    assert!(notes.contains("Gaussian Blur (1 clip)") && notes.contains("1 effect(s) and 1 transition type(s)"), "{notes}");
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    assert!(seq.video_tracks[0].items[0].effect("gaussian_blur").is_none());
    assert!(!String::from_utf8(prproj_graph::decompress(&bytes).unwrap().0).unwrap().contains("Secret Sauce"));
}
