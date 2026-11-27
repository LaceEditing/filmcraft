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
        url: String::new(),
        author: String::new(),
        custom_color: None,
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
    let (mut p, main, _) = project();
    {
        let q = p.sequence_mut(main).unwrap();
        q.audio_tracks[0].items[0].gain_db = -4.5;
        q.video_tracks[1].items[0].enabled = false;
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(!report.mentions("audio gain") && !report.mentions("disabled"), "{report}");
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
    // clip gain and Clip ▸ Enable off survive
    assert!((a1.gain_db + 4.5).abs() < 1e-6, "{}", a1.gain_db);
    assert!(!logo.enabled, "the disabled clip");
    // the sequence marker is on the Sequence, where Premiere lists it
    let d = crate::xml::parse(&text).unwrap();
    let s = d
        .root_element()
        .children()
        .find(|n| n.has_tag_name("Sequence") && n.children().any(|c| c.has_tag_name("Name") && c.text() == Some("Main Edit")))
        .unwrap();
    assert!(s.children().any(|c| c.has_tag_name("MarkerOwner")), "markers on the Sequence");
}

/// Track pan and mono tracks are written as Premiere saves them (Round 5 B3): a stereo track's
/// Balance, a mono track's `ChannelType` 0 and its Pan into the stereo mix.
#[test]
fn track_pan_and_mono_tracks_are_written() {
    let (mut p, main, _) = project();
    {
        let q = p.sequence_mut(main).unwrap();
        q.audio_tracks[0].pan = -40.0;
        q.audio_tracks[1].channels = AudioChannels::Mono;
        q.audio_tracks[1].pan = 100.0;
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(!report.mentions("pan") && !report.mentions("mono"), "{report}");
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(crate::prproj_write::problems(&text).is_empty(), "{:?}", crate::prproj_write::problems(&text));
    assert!(text.contains("<MonoToStereoPanProcessor") && text.contains("<Name>Pan</Name>"));
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let tracks: Vec<_> = seq.audio_tracks.iter().map(|t| (t.channels, t.pan)).collect();
    assert_eq!(&tracks[..2], &[(AudioChannels::Stereo, -40.0), (AudioChannels::Mono, 100.0)]);
    assert_eq!(clips(&back, "Main Edit", TrackKind::Audio), clips(&p, "Main Edit", TrackKind::Audio));
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

/// A title made in FilmCraft (two text layers) is written into a new project as Premiere writes
/// one (`GRFV` media, a text layer per FilmCraft layer) and reads back as the same title.
#[test]
fn titles_are_written_into_new_projects() {
    let (mut p, main, _) = project();
    let graphic = p.add_item("Title", Label::Rose, ItemKind::Graphic { width: 1920, height: 1080, rate: FrameRate::FPS_30 }, None);
    let layer = |text: &str, size: f64| {
        let mut e = filmcraft_project::find_effect(filmcraft_project::graphic::TEXT_LAYER).unwrap().instance();
        e.params.insert("text".into(), Param::new(ParamValue::Text(text.into())));
        e.params.insert("size".into(), Param::new(ParamValue::Float(size)));
        e.params.insert("top_left_origin".into(), Param::new(ParamValue::Bool(true)));
        e
    };
    {
        let q = p.sequence_mut(main).unwrap();
        let mut c = q.video_tracks[0].items[0].clone();
        c.id = filmcraft_project::ClipId(77_001);
        c.item = graphic;
        c.name = "Title".into();
        c.link = None;
        c.start = Tick(40 * S);
        c.duration = Tick(3 * S);
        c.source_in = Tick::ZERO;
        c.speed = 1.0;
        c.effects = filmcraft_project::effect::intrinsic_video();
        c.effects.push(layer("Big words", 140.0));
        c.effects.push(layer("small words", 50.0));
        q.video_tracks[0].items.push(c);
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(!report.mentions("graphics and titles"), "{report}");
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let c = seq.video_tracks[0].items.iter().find(|c| c.start == Tick(40 * S)).expect("the title clip");
    let texts: Vec<String> = c
        .effects
        .iter()
        .filter(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER)
        .map(|e| format!("{:?}/{:?}", e.params.get("text").map(|p| &p.value), e.params.get("size").map(|p| &p.value)))
        .collect();
    assert_eq!(texts.len(), 2, "{texts:?} {report}");
    assert!(texts.iter().any(|t| t.contains("Big words") && t.contains("140.0")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("small words") && t.contains("50.0")), "{texts:?}");
}

/// Set a clip's pan (its Panner's Balance), adding the Panner where the clip has none.
fn set_pan(c: &mut filmcraft_project::TrackItem, pan: f64) {
    if c.effect("panner").is_none()
        && let Some(def) = filmcraft_project::find_effect("panner")
    {
        c.effects.push(def.instance());
    }
    c.effect_mut("panner").unwrap().params.insert("balance".into(), Param::new(ParamValue::Float(pan)));
}

fn pan_of(c: &filmcraft_project::TrackItem) -> Option<ParamValue> {
    c.effect("panner").and_then(|e| e.params.get("balance")).map(|p| p.value.clone())
}

/// A clip's pan is written as Premiere Pro 2027 saves it: a pan processor of the clip's own (a
/// stereo clip's Balance, a mono clip's Pan, component id 4294967040), saved as (pan + 100) / 200
/// with no value at +100; a clip whose pan is centred has none.
#[test]
fn clip_pan_is_written() {
    let (mut p, main, _) = project();
    {
        let q = p.sequence_mut(main).unwrap();
        set_pan(&mut q.audio_tracks[0].items[0], -50.0);
        set_pan(&mut q.audio_tracks[1].items[0], 100.0);
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(!report.mentions("pan"), "{report}");
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(crate::prproj_write::problems(&text).is_empty(), "{:?}", crate::prproj_write::problems(&text));
    assert_eq!(text.matches("<ID>4294967040</ID>").count(), 2, "one panner per panned clip");
    assert!(text.contains(&format!(
        "<StartKeyframe>{NO_TIME},0.25,0,0,0,0,0,0</StartKeyframe><CurrentValue>0.25</CurrentValue><IsTimeVarying>false</IsTimeVarying><Name>Balance</Name><IsInverted>true</IsInverted>"
    )));
    // +100 on the mono clip: Pan with no value
    assert!(text.contains("<IsTimeVarying>false</IsTimeVarying><Name>Pan</Name><IsInverted>true</IsInverted>"));
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    assert_eq!(pan_of(&seq.audio_tracks[0].items[0]), Some(ParamValue::Float(-50.0)));
    assert_eq!(pan_of(&seq.audio_tracks[1].items[0]), Some(ParamValue::Float(100.0)));
    assert_eq!(clips(&back, "Main Edit", TrackKind::Audio), clips(&p, "Main Edit", TrackKind::Audio));

    // a centred pan writes no panner on the clip
    {
        let q = p.sequence_mut(main).unwrap();
        set_pan(&mut q.audio_tracks[0].items[0], 0.0);
        set_pan(&mut q.audio_tracks[1].items[0], 0.0);
    }
    let (bytes, _) = create(&p, &opts()).unwrap();
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    assert!(!String::from_utf8(xml).unwrap().contains("<ID>4294967040</ID>"));
}

/// A track's volume automation is written into its fader as keyframes at sequence times, on the
/// fader's own scale (a linear gain, 1 at 0 dB, 0 at silence), and comes back as it was; the
/// other lanes are reported.
#[test]
fn track_volume_automation_is_written_and_read() {
    use filmcraft_project::mixer::{LANE_PAN, LANE_VOLUME};
    use filmcraft_project::{Interpolation, Keyframe};
    let (mut p, main, _) = project();
    let key = |s: i64, db: f64| Keyframe::new(Tick(s * S), ParamValue::Float(db));
    {
        let q = p.sequence_mut(main).unwrap();
        let lane = q.audio_tracks[0].lane_mut(LANE_VOLUME).unwrap();
        lane.keyframes = vec![key(1, 0.0), key(3, -20.0), key(5, -96.0), key(6, 6.0)];
        lane.keyframes[1].interp = Interpolation::Hold;
        q.audio_tracks[1].lane_mut(LANE_PAN).unwrap().keyframes = vec![key(1, -50.0), key(2, 50.0)];
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(report.mentions("pan, mute and send automation of 1 track(s) was not written"), "{report}");
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(crate::prproj_write::problems(&text).is_empty(), "{:?}", crate::prproj_write::problems(&text));
    // 0 dB is 1, -20 dB 0.1 (held), silence 0, +6 dB 1.995…
    let written = text.split("<Keyframes>").nth(1).and_then(|s| s.split("</Keyframes>").next()).expect("the fader's keyframes");
    for key in [format!("{S},1.,0,0,0,0,0,0;"), format!("{},0.1,4,0,", 3 * S), format!("{},0.,0,0,0,0,0,0;", 5 * S), format!("{},1.99526231496", 6 * S)] {
        assert!(written.contains(&key), "{key} in {written}");
    }
    let (back, report) = read(&bytes);
    assert!(!report.mentions("automation"), "{report}");
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let lane = seq.audio_tracks[0].lane(LANE_VOLUME).expect("the volume lane");
    let got: Vec<(i64, f64, Interpolation)> = lane
        .keyframes
        .iter()
        .map(|k| (k.time.0 / S, if let ParamValue::Float(v) = k.value { (v * 1000.0).round() / 1000.0 } else { f64::NAN }, k.interp))
        .collect();
    assert_eq!(got, [(1, 0.0, Interpolation::Linear), (3, -20.0, Interpolation::Hold), (5, -96.0, Interpolation::Linear), (6, 6.0, Interpolation::Linear)]);
    assert!(seq.audio_tracks[1].lane(LANE_VOLUME).is_none_or(|l| l.keyframes.is_empty()));
}

/// A new project carries the object versions Premiere Pro 2027 saves (`Project` 46, `Clip` 19,
/// `Media` 31): stamped with 26.x's (45, 18, 30), 2027 asks to convert it into a copy first.
#[test]
fn a_new_project_is_stamped_as_premiere_2027_saves_one() {
    let (p, _, _) = project();
    let (bytes, _) = create(&p, &opts()).unwrap();
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    let d = crate::xml::parse(&text).unwrap();
    let root = d.root_element();
    let version = |tag: &str| -> Vec<&str> {
        let mut v: Vec<&str> =
            root.descendants().filter(|n| n.has_tag_name(tag) && n.attribute("ObjectRef").is_none()).filter_map(|n| n.attribute("Version")).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    assert_eq!(version("Clip"), ["19"]);
    assert_eq!(version("Media"), ["31"]);
    assert!(root.children().any(|n| n.has_tag_name("Project") && n.attribute("ObjectID").is_some() && n.attribute("Version") == Some("46")));
    assert_eq!(Versions::of(root), Versions::PREMIERE_2027);
}

/// Objects added to a project opened from Premiere take that document's versions: its own
/// `Clip` and `Media`, else those that go with its `Project`.
#[test]
fn additions_take_the_versions_of_the_document_they_join() {
    let of = |xml: &str| Versions::of(crate::xml::parse(xml).unwrap().root_element());
    let old = r#"<PremiereData Version="3"><Project ObjectRef="1"/><Project ObjectID="1" Version="45"/></PremiereData>"#;
    assert_eq!(of(old), Versions::PREMIERE_2026);
    let new = r#"<PremiereData Version="3"><Project ObjectRef="1"/><Project ObjectID="1" Version="46"/></PremiereData>"#;
    assert_eq!(of(new), Versions::PREMIERE_2027);
    // a document's own objects win over its Project's number
    let mixed = r#"<PremiereData Version="3"><Project ObjectID="1" Version="46"/><Media ObjectUID="m" Version="30"/><VideoClip ObjectID="2" Version="11"><Clip Version="18"/></VideoClip></PremiereData>"#;
    assert_eq!(of(mixed), Versions { clip: 18, media: 30 });
    // hostile numbers fall back to the Project's
    let odd = r#"<PremiereData Version="3"><Project ObjectID="1" Version="x"/><Media ObjectUID="m" Version="-4"/></PremiereData>"#;
    assert_eq!(of(odd), Versions::PREMIERE_2027);
}

/// Position keyframes carry their spatial part (`5,4` and four tangent values) as Premiere Pro
/// 2027 writes each point keyframe: written with the temporal fields only, Premiere played the
/// clip as a still at the first keyframe's position.
#[test]
fn position_keyframes_carry_their_spatial_part() {
    use filmcraft_project::Keyframe;
    let (mut p, main, _) = project();
    {
        let q = p.sequence_mut(main).unwrap();
        let motion = q.video_tracks[0].items[0].effect_mut("motion").unwrap();
        let pos = motion.params.get_mut("position").unwrap();
        pos.keyframes = vec![
            Keyframe::new(Tick(0), ParamValue::Vec2(filmcraft_geom::Vec2::new(480.0, 540.0))),
            Keyframe::new(Tick(2 * S), ParamValue::Vec2(filmcraft_geom::Vec2::new(1440.0, 540.0))),
        ];
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(crate::prproj_write::problems(&text).is_empty(), "{:?}", crate::prproj_write::problems(&text));
    let d = crate::xml::parse(&text).unwrap();
    let position = d
        .descendants()
        .find(|n| {
            n.has_tag_name("PointComponentParam")
                && n.children().any(|c| c.has_tag_name("Name") && c.text() == Some("Position"))
                && n.children().any(|c| c.has_tag_name("Keyframes"))
        })
        .expect("the keyframed Position");
    let keys = position.children().find(|c| c.has_tag_name("Keyframes")).and_then(|k| k.text()).unwrap();
    let keys: Vec<&str> = keys.split(';').filter(|k| !k.is_empty()).collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    for k in &keys {
        let f: Vec<&str> = k.split(',').collect();
        assert_eq!(f.len(), 14, "{k}");
        assert_eq!(&f[8..], ["5", "4", "0", "0", "0", "0"], "{k}");
    }
    assert!(keys[0].starts_with("0,0.25:0.5,") && keys[1].starts_with(&format!("{},0.75:0.5,", 2 * S)), "{keys:?}");
    // and it reads back
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let first = seq.video_tracks[0].items.iter().find(|c| c.start.0 == 0).unwrap();
    let pos = first.effect("motion").and_then(|e| e.params.get("position")).unwrap();
    assert_eq!(pos.keyframes.len(), 2, "{report}");
    assert_eq!(pos.keyframes[1].value, ParamValue::Vec2(filmcraft_geom::Vec2::new(1440.0, 540.0)));
}

/// A clip's blend mode is written into a new project (Premiere's code and its menu position) and
/// is not reported as left out: Premiere Pro 27.0 plays all 22 of the kit's modes within 1 code
/// value of FilmCraft's render.
#[test]
fn blend_modes_are_written_into_a_new_project() {
    let (mut p, main, _) = project();
    {
        let q = p.sequence_mut(main).unwrap();
        let opacity = q.video_tracks[0].items[0].effect_mut("opacity").unwrap();
        // Multiply: the menu's fourth mode, Premiere's code 17
        opacity.params.get_mut("blend").unwrap().value = ParamValue::Choice(3);
    }
    let (bytes, report) = create(&p, &opts()).unwrap();
    assert!(!report.mentions("blend"), "{report}");
    let (xml, _) = prproj_graph::decompress(&bytes).unwrap();
    let text = String::from_utf8(xml).unwrap();
    assert!(text.contains(&format!("<Name>Blend Mode</Name><ParameterControlType>10</ParameterControlType><ParameterID>2</ParameterID><DiscontinuousInterpolate>true</DiscontinuousInterpolate><StartKeyframe>{NO_TIME},17,")), "the code");
    let (back, _) = read(&bytes);
    let seq = back.items.values().find(|i| i.name == "Main Edit").and_then(|i| i.as_sequence()).unwrap();
    let first = seq.video_tracks[0].items.iter().find(|c| c.start.0 == 0).unwrap();
    assert_eq!(first.effect("opacity").and_then(|e| e.params.get("blend")).map(|p| p.value.clone()), Some(ParamValue::Choice(3)));
}
