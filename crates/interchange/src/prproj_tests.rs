use super::*;
use crate::prproj_fx::UNITY_LEVEL;
use crate::{detect, import_with};
use filmcraft_project::{Interpolation, ParamValue};
use filmcraft_project::{ItemKind, MediaRef};

pub(crate) const S: i64 = TICKS_PER_SECOND;

/// A small project in the observed `.prproj` shape (original test data, written by hand).
pub(crate) fn doc() -> String {
    let unity = UNITY_LEVEL;
    let minus6 = unity * 10f64.powf(-6.0 / 20.0);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
  <Project ObjectRef="1"/>
  <RootProjectItem ObjectUID="root">
    <ProjectItem><Name>Root Bin</Name></ProjectItem>
    <ProjectItemContainer><Items><Item Index="0" ObjectURef="bin1"/><Item Index="1" ObjectURef="seqpi"/></Items></ProjectItemContainer>
  </RootProjectItem>
  <BinProjectItem ObjectUID="bin1">
    <ProjectItem><Name>Footage</Name></ProjectItem>
    <ProjectItemContainer><Items><Item Index="0" ObjectURef="mpi"/></Items></ProjectItemContainer>
  </BinProjectItem>
  <ClipProjectItem ObjectUID="mpi"><ProjectItem><Name>interview.mov</Name></ProjectItem><MasterClip ObjectURef="mmc"/></ClipProjectItem>
  <MasterClip ObjectUID="mmc"><Clips><Clip Index="0" ObjectRef="10"/><Clip Index="1" ObjectRef="16"/></Clips></MasterClip>
  <AudioClip ObjectID="16"><Clip><Source ObjectRef="76"/></Clip></AudioClip>
  <VideoClip ObjectID="10"><Clip><Source ObjectRef="11"/><MarkerOwner><Markers ObjectRef="12"/></MarkerOwner></Clip></VideoClip>
  <VideoMediaSource ObjectID="11"><MediaSource><Media ObjectURef="media1"/></MediaSource></VideoMediaSource>
  <Markers ObjectID="12"><Markers><Marker Index="0"><First>m1</First><Second ObjectRef="13"/></Marker></Markers></Markers>
  <Marker ObjectID="13"><DVAMarker>{{"DVAMarker":{{"mMarkerID":"m1","mStartTime":{{"ticks":{s1}}},"mName":"Laugh","mComment":"keep this","mType":"Comment"}}}}</DVAMarker></Marker>
  <Media ObjectUID="media1">
    <AudioStream ObjectRef="15"/><VideoStream ObjectRef="14"/>
    <RelativePath>.\Footage\interview.mov</RelativePath>
    <FilePath>L:\Edit\Footage\interview.mov</FilePath>
    <Title>interview.mov</Title>
  </Media>
  <VideoStream ObjectID="14"><FrameRate>8467200000</FrameRate><Duration>{s60}</Duration><FrameRect>0,0,2560,1440</FrameRect></VideoStream>
  <AudioStream ObjectID="15"><FrameRate>5292000</FrameRate><AudioChannelLayout>[{{"channellabel":100}},{{"channellabel":101}}]</AudioChannelLayout><Duration>{s60}</Duration></AudioStream>

  <ClipProjectItem ObjectUID="seqpi"><ProjectItem><Name>Main Edit</Name></ProjectItem><MasterClip ObjectURef="smc"/></ClipProjectItem>
  <MasterClip ObjectUID="smc"><Clips><Clip Index="0" ObjectRef="20"/></Clips></MasterClip>
  <VideoClip ObjectID="20"><Clip><Source ObjectRef="21"/><MarkerOwner><Markers ObjectRef="22"/></MarkerOwner></Clip></VideoClip>
  <VideoSequenceSource ObjectID="21"><SequenceSource><Sequence ObjectURef="seq1"/></SequenceSource></VideoSequenceSource>
  <Markers ObjectID="22"><Markers><Marker Index="0"><First>m2</First><Second ObjectRef="23"/></Marker></Markers></Markers>
  <Marker ObjectID="23"><DVAMarker>{{"DVAMarker":{{"mMarkerID":"m2","mStartTime":{{"ticks":{s2}}},"mName":"Act 2","mType":"Chapter"}}}}</DVAMarker></Marker>

  <Sequence ObjectUID="seq1">
    <Name>Main Edit</Name>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="30"/></TrackGroup>
      <TrackGroup><Second ObjectRef="31"/></TrackGroup>
    </TrackGroups>
    <PersistentGroupContainer Version="1"><LinkContainer Version="1"><Links Version="1"><Link Index="0" ObjectRef="80" /></Links></LinkContainer></PersistentGroupContainer>
  </Sequence>
  <VideoTrackGroup ObjectID="30"><TrackGroup><Tracks><Track Index="0" ObjectURef="vt1"/></Tracks><FrameRate>8467200000</FrameRate></TrackGroup><FrameRect>0,0,1920,1080</FrameRect></VideoTrackGroup>
  <AudioTrackGroup ObjectID="31"><TrackGroup><Tracks><Track Index="0" ObjectURef="at1"/></Tracks><FrameRate>5292000</FrameRate></TrackGroup></AudioTrackGroup>

  <VideoClipTrack ObjectUID="vt1"><ClipTrack>
    <Track><Name>Cam</Name><Index>0</Index></Track>
    <ClipItems><TrackItems><TrackItem Index="0" ObjectRef="40"/><TrackItem Index="1" ObjectRef="50"/></TrackItems></ClipItems>
    <TransitionItems><TrackItems><TrackItem Index="0" ObjectRef="60"/></TrackItems></TransitionItems>
  </ClipTrack></VideoClipTrack>

  <VideoClipTrackItem ObjectID="40"><ClipTrackItem>
    <ComponentOwner><Components ObjectRef="43"/></ComponentOwner>
    <TrackItem><Start>0</Start><End>{s5}</End></TrackItem>
    <SubClip ObjectRef="41"/>
  </ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="41"><Clip ObjectRef="42"/><Name>interview.mov</Name></SubClip>
  <VideoClip ObjectID="42"><Clip><Source ObjectRef="11"/><InPoint>{s10}</InPoint><OutPoint>{s15}</OutPoint></Clip></VideoClip>
  <VideoComponentChain ObjectID="43"><ComponentChain><Components><Component Index="0" ObjectRef="44"/><Component Index="1" ObjectRef="48"/></Components></ComponentChain></VideoComponentChain>
  <VideoFilterComponent ObjectID="44">
    <Component><Params><Param Index="0" ObjectRef="46"/><Param Index="1" ObjectRef="45"/><Param Index="2" ObjectRef="47"/></Params><DisplayName>Motion</DisplayName></Component>
    <MatchName>AE.ADBE Motion</MatchName>
  </VideoFilterComponent>
  <VideoComponentParam ObjectID="45"><Name>Scale</Name><IsTimeVarying>true</IsTimeVarying><StartKeyframe>-91445760000000000,100,0,0,0,0,0,0</StartKeyframe><Keyframes>{s10},100,0,0,0,0.1,1.5,0.1;{s12},120,0,0,1.5,0.1,0,0.1;</Keyframes></VideoComponentParam>
  <PointComponentParam ObjectID="46"><Name>Position</Name><StartKeyframe>-91445760000000000,0.25:0.5,0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe></PointComponentParam>
  <VideoComponentParam ObjectID="47"><Name> </Name><StartKeyframe>-91445760000000000,false,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="48"><Component><Params/><DisplayName>RSMB Pro</DisplayName></Component><MatchName>RE:Vision RSMB Pro</MatchName></VideoFilterComponent>

  <VideoClipTrackItem ObjectID="50"><ClipTrackItem>
    <ComponentOwner><Components ObjectRef="53"/></ComponentOwner>
    <TrackItem><Start>{s5}</Start><End>{s7}</End></TrackItem>
    <SubClip ObjectRef="51"/>
  </ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="51"><Clip ObjectRef="52"/><Name>backwards</Name></SubClip>
  <VideoClip ObjectID="52"><Clip><Source ObjectRef="11"/><InPoint>{s20}</InPoint><OutPoint>{s24}</OutPoint><PlaybackSpeed>-2</PlaybackSpeed></Clip></VideoClip>
  <VideoComponentChain ObjectID="53"><ComponentChain><Components><Component Index="0" ObjectRef="54"/><Component Index="1" ObjectRef="56"/><Component Index="2" ObjectRef="100"/></Components></ComponentChain></VideoComponentChain>
  <VideoFilterComponent ObjectID="100"><Component><Params><Param Index="0" ObjectRef="101"/><Param Index="1" ObjectRef="102"/><Param Index="2" ObjectRef="103"/></Params><ID>3</ID><DisplayName>Tint</DisplayName></Component><MatchName>AE.ADBE Tint</MatchName></VideoFilterComponent>
  <VideoComponentParam ObjectID="101"><Name>Map Black To</Name><StartKeyframe>-91445760000000000,18374686479671623680,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="102"><Name>Map White To</Name><StartKeyframe>-91445760000000000,18374966857753992192,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="103"><Name>Amount to Tint</Name><StartKeyframe>-91445760000000000,100.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="56"><Component><Params><Param Index="0" ObjectRef="57"/><Param Index="1" ObjectRef="58"/><Param Index="2" ObjectRef="59"/></Params><ID>2</ID><DisplayName>Gaussian Blur (Legacy)</DisplayName></Component><MatchName>AE.ADBE Gaussian Blur</MatchName></VideoFilterComponent>
  <VideoComponentParam ObjectID="57"><Name>Blurriness</Name><IsTimeVarying>true</IsTimeVarying><StartKeyframe>-91445760000000000,0,0,0,0,0,0,0</StartKeyframe><Keyframes>{s20},0,5,0,0,0.33333333333333331,0,0.33333333333333331;{s22},40,5,2,0,0.25,0,0.5;</Keyframes></VideoComponentParam>
  <VideoComponentParam ObjectID="58"><Name>Blur Dimensions</Name><StartKeyframe>-91445760000000000,1,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="59"><Name>Repeat Edge Pixels</Name><StartKeyframe>-91445760000000000,true,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="54"><Component><Params><Param Index="0" ObjectRef="55"/></Params><DisplayName>Opacity</DisplayName></Component><MatchName>AE.ADBE Opacity</MatchName></VideoFilterComponent>
  <VideoComponentParam ObjectID="55"><Name>Opacity</Name><StartKeyframe>-91445760000000000,50,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>

  <VideoTransitionTrackItem ObjectID="60"><TransitionTrackItem>
    <TrackItem><Start>{s4_5}</Start><End>{s5_5}</End></TrackItem>
    <DisplayName>Cross Dissolve</DisplayName><HasOutgoingClip>true</HasOutgoingClip><HasIncomingClip>true</HasIncomingClip>
  </TransitionTrackItem></VideoTransitionTrackItem>

  <AudioClipTrack ObjectUID="at1"><ClipTrack>
    <Track><Index>0</Index></Track>
    <ClipItems><TrackItems><TrackItem Index="0" ObjectRef="70"/></TrackItems></ClipItems>
  </ClipTrack></AudioClipTrack>
  <AudioClipTrackItem ObjectID="70"><ClipTrackItem>
    <ComponentOwner><Components ObjectRef="73"/></ComponentOwner>
    <TrackItem><End>{s5}</End><Start>0</Start></TrackItem>
    <SubClip ObjectRef="71"/>
  </ClipTrackItem></AudioClipTrackItem>
  <SubClip ObjectID="71"><Clip ObjectRef="72"/><Name>interview.mov</Name></SubClip>
  <AudioClip ObjectID="72"><Clip><Source ObjectRef="76"/><InPoint>{s10}</InPoint><OutPoint>{s15}</OutPoint></Clip></AudioClip>
  <AudioMediaSource ObjectID="76"><MediaSource><Media ObjectURef="media1"/></MediaSource></AudioMediaSource>
  <AudioComponentChain ObjectID="73"><ComponentChain><Components><Component Index="0" ObjectRef="74"/></Components></ComponentChain></AudioComponentChain>
  <AudioFilterComponent ObjectID="74"><AudioComponent><Component><Params><Param Index="0" ObjectRef="75"/></Params></Component></AudioComponent><FilterMatchName>Internal Volume Stereo</FilterMatchName></AudioFilterComponent>
  <AudioComponentParam ObjectID="75"><Name>Level</Name><StartKeyframe>-91445760000000000,{minus6},0,0,0,0,0,0</StartKeyframe></AudioComponentParam>

  <Link ObjectID="80"><TrackItemGroup><TrackItems><TrackItem Index="0" ObjectRef="40"/><TrackItem Index="1" ObjectRef="70"/></TrackItems></TrackItemGroup></Link>
</PremiereData>
"#,
        s1 = S,
        s2 = 2 * S,
        s5 = 5 * S,
        s7 = 7 * S,
        s10 = 10 * S,
        s12 = 12 * S,
        s15 = 15 * S,
        s20 = 20 * S,
        s22 = 22 * S,
        s24 = 24 * S,
        s60 = 60 * S,
        s4_5 = 9 * S / 2,
        s5_5 = 11 * S / 2,
    )
}

/// Wrap `data` in a gzip member (RFC 1952) with a file name field, as `.prproj` files are.
pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 0xff];
    out.extend_from_slice(b"project.xml\0");
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend_from_slice(&[0; 8]); // CRC-32 and size: not checked by the reader
    out
}

pub(crate) fn import(bytes: &[u8], base: Option<&str>) -> (Imported, Report) {
    let opts = ImportOptions { base_dir: base.map(str::to_string), name: Some("Test".into()), ..Default::default() };
    import_with(bytes, Format::Prproj, &opts).unwrap()
}

#[test]
fn detects_gzipped_and_plain_projects() {
    let plain = doc();
    let gz = gzip(plain.as_bytes());
    assert_eq!(detect(&gz, Some("prproj")), Some(Format::Prproj));
    assert_eq!(detect(plain.as_bytes(), Some("prproj")), Some(Format::Prproj));
    assert_eq!(detect(plain.as_bytes(), None), Some(Format::Prproj));
    // a gzip file that is not called .prproj is not claimed
    assert_eq!(detect(&gz, Some("gz")), None);
    assert_eq!(Format::from_extension(".PRPROJ"), Some(Format::Prproj));
}

#[test]
fn imports_bins_media_sequences_clips_and_effects() {
    let (imp, report) = import(&gzip(doc().as_bytes()), Some("/edits/show"));
    let p = &imp.project;

    // bins and media
    let footage = p.root.children.iter().find_map(|e| match e {
        filmcraft_project::BinEntry::Bin(b) if b.name == "Footage" => Some(b),
        _ => None,
    });
    let footage = footage.expect("Footage bin");
    assert!(footage.children.iter().any(|e| matches!(e, filmcraft_project::BinEntry::Item(id) if p.item(*id).is_some_and(|i| i.name == "interview.mov"))));
    let media = p.items.values().find(|i| i.name == "interview.mov").expect("media item");
    let ItemKind::Media(m) = &media.kind else { panic!("not media") };
    let MediaRef::File { path } = &m.media else { panic!("not a file") };
    assert_eq!(path, "/edits/show/Footage/interview.mov");
    let v = m.info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.frame_rate), (2560, 1440, FrameRate::new(30, 1)));
    assert_eq!(m.info.audio().map(|a| (a.sample_rate, a.channels)), Some((48_000, 2)));
    assert_eq!(m.markers.len(), 1);
    assert_eq!((m.markers[0].start, m.markers[0].name.as_str(), m.markers[0].comment.as_str()), (Tick(S), "Laugh", "keep this"));

    // the sequence
    assert_eq!(imp.sequences.len(), 1);
    let seq_item = p.item(imp.sequences[0]).unwrap();
    assert_eq!(seq_item.name, "Main Edit");
    let seq = p.sequence(imp.sequences[0]).unwrap();
    assert_eq!((seq.settings.width, seq.settings.height, seq.settings.frame_rate, seq.settings.sample_rate), (1920, 1080, FrameRate::new(30, 1), 48_000));
    assert_eq!(seq.markers.len(), 1);
    assert_eq!((seq.markers[0].start, seq.markers[0].kind), (Tick(2 * S), MarkerKind::Chapter));

    let v1 = &seq.video_tracks[0];
    assert_eq!(v1.name, "Cam");
    assert_eq!(v1.items.len(), 2);
    let (a, b) = (&v1.items[0], &v1.items[1]);
    assert_eq!((a.start, a.duration, a.source_in, a.speed, a.reverse), (Tick(0), Tick(5 * S), Tick(10 * S), 1.0, false));
    assert_eq!((b.start, b.duration, b.source_in, b.speed, b.reverse), (Tick(5 * S), Tick(2 * S), Tick(20 * S), 2.0, true));

    // Motion: Scale keyframes stay at their media times (10 s, 12 s), where FilmCraft plays them
    let scale = a.effect("motion").and_then(|e| e.params.get("scale")).unwrap();
    let keys: Vec<(Tick, f64)> = scale.keyframes.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect();
    assert_eq!(keys, vec![(Tick(10 * S), 100.0), (Tick(12 * S), 120.0)]);
    // (one second into the clip it is on its way: before, keys came in at 0 s / 2 s and every
    // keyed clip with an In point showed its last value throughout)
    let halfway = scale.f64_at(a.effect_time_at(a.start + Tick(S)));
    assert!(halfway > 100.5 && halfway < 119.5, "{halfway}");
    let pos = a.effect("motion").and_then(|e| e.params.get("position")).and_then(|p| p.value.as_vec2()).unwrap();
    assert_eq!((pos.x, pos.y), (480.0, 540.0));
    assert_eq!(a.effect("motion").and_then(|e| e.params.get("uniform_scale")).map(|p| p.value.clone()), Some(ParamValue::Bool(false)));
    assert_eq!(b.effect("opacity").and_then(|e| e.params.get("opacity")).and_then(|p| p.value.as_f64()), Some(50.0));

    // the dissolve sits across the cut between the two clips
    assert_eq!(v1.transitions.len(), 1);
    let t = &v1.transitions[0];
    assert_eq!((t.from, t.to, t.align, t.start, t.duration), (Some(a.id), Some(b.id), TransitionAlign::CenterAtCut, Tick(9 * S / 2), Tick(S)));

    // audio: linked to the first video clip, -6 dB
    let a1 = &seq.audio_tracks[0].items[0];
    assert!(a.link.is_some());
    assert_eq!(a.link, a1.link);
    let level = a1.effect("volume").and_then(|e| e.params.get("level")).and_then(|p| p.value.as_f64()).unwrap();
    assert!((level + 6.0).abs() < 1e-6, "level {level}");

    assert!(report.mentions("RSMB Pro"), "{report}");

    // a FilmCraft effect with the same name: eased keyframes at their media times (20 s / 22 s,
    // which the clip plays at 2x), a boolean, and the choice reported
    let blur = b.effect("gaussian_blur").expect("Gaussian Blur imported");
    let keys = &blur.params["blurriness"].keyframes;
    assert_eq!(keys.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect::<Vec<_>>(), vec![(Tick(20 * S), 0.0), (Tick(22 * S), 40.0)]);
    assert_eq!((keys[0].interp, keys[1].interp), (Interpolation::Bezier, Interpolation::AutoBezier));
    assert_eq!((keys[1].in_influence, keys[1].out_influence), (0.25, 0.5));
    assert_eq!(blur.params["repeat_edge"].value, ParamValue::Bool(true));
    assert_eq!(blur.params["dimensions"].value, ParamValue::Choice(1), "0-based popup index");
    assert!(!report.mentions("Blur Dimensions"), "{report}");
    let tint = b.effect("tint").expect("Tint imported");
    assert_eq!(tint.params["black"].value, ParamValue::Color([0.0, 0.0, 0.0, 1.0]));
    let white = tint.params["white"].value.as_color().unwrap();
    assert!(
        (white[0] - 1.0).abs() < 1e-6 && (white[1] - 156.0 / 255.0).abs() < 1e-6 && (white[2] - 156.0 / 255.0).abs() < 1e-6 && (white[3] - 1.0).abs() < 1e-6,
        "{white:?}"
    );

    // round-trip bookkeeping: origins and Premiere ids
    assert_eq!((a.origin.as_deref(), b.origin.as_deref(), a1.origin.as_deref()), (Some("40"), Some("50"), Some("70")));
    assert_eq!(seq_item.metadata.get(META_SEQUENCE).map(String::as_str), Some("seq1"));
    assert!(seq_item.metadata.contains_key(META_FINGERPRINT));
    assert_eq!(media.metadata.get(META_MEDIA).map(String::as_str), Some("media1"));
}

/// A parameter whose stopwatch Premiere turned off keeps its keyframes in the file but plays its
/// static value (one user's project had 17 of them, animating in FilmCraft only); audio
/// parameters never write the flag, and their keyframes always play.
#[test]
fn keyframes_of_a_stopped_stopwatch_do_not_play() {
    let on = doc();
    let flag = "<Name>Scale</Name><IsTimeVarying>true</IsTimeVarying>";
    assert!(on.contains(flag));
    for (what, off) in
        [("absent", on.replace(flag, "<Name>Scale</Name>")), ("false", on.replace(flag, "<Name>Scale</Name><IsTimeVarying>false</IsTimeVarying>"))]
    {
        let (imp, _) = import(off.as_bytes(), Some("/edits/show"));
        let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
        let scale = seq.video_tracks[0].items[0].effect("motion").and_then(|e| e.params.get("scale")).unwrap();
        assert!(scale.keyframes.is_empty(), "IsTimeVarying {what}: {:?}", scale.keyframes);
        assert_eq!(scale.value.as_f64(), Some(100.0), "IsTimeVarying {what}: the StartKeyframe value");
    }
    // the clip's Volume Level with keyframes and no flag, as Premiere writes it: it animates
    let at = on.find("<Name>Level</Name>").unwrap();
    let end = at + on[at..].find("</StartKeyframe>").unwrap() + "</StartKeyframe>".len();
    let keys = format!("<Keyframes>{},0.17782794100389229,0,0,0,0,0,0;{},0.017782794100389229,0,0,0,0,0,0;</Keyframes>", 10 * S, 12 * S);
    let keyed = format!("{}{keys}{}", &on[..end], &on[end..]);
    let (imp, _) = import(keyed.as_bytes(), Some("/edits/show"));
    let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let level = seq.audio_tracks[0].items[0].effect("volume").and_then(|e| e.params.get("level")).unwrap();
    let db: Vec<f64> = level.keyframes.iter().filter_map(|k| k.value.as_f64()).collect();
    assert_eq!(db.len(), 2, "{:?}", level.keyframes);
    assert!(db[0].abs() < 1e-6 && (db[1] + 20.0).abs() < 1e-6, "{db:?}");
}

/// Premiere's keyframe handles carry a speed (in Scale's case, tenths of a percent per second):
/// a Bézier key at the straight line's speed plays as a line, one with no speed on the arriving
/// side eases in, and Hold holds. Written back, a key gets its speed recomputed for its segment.
#[test]
fn keyframe_speeds_and_hold_come_in_as_premiere_plays_them() {
    let (t0, t1) = (10 * S, 12 * S);
    let scale_keys = format!("<Keyframes>{t0},100,0,0,0,0.1,1.5,0.1;{t1},120,0,0,1.5,0.1,0,0.1;</Keyframes>");
    let base = doc();
    assert!(base.contains(&scale_keys));
    let sixth = 1.0 / 6.0;
    let with = |keys: String| base.replace(&scale_keys, &format!("<Keyframes>{keys}</Keyframes>"));
    let scale_at = |doc: &str, secs: f64| {
        let (imp, _) = import(doc.as_bytes(), Some("/edits/show"));
        let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
        let a = &seq.video_tracks[0].items[0];
        let p = a.effect("motion").and_then(|e| e.params.get("scale")).unwrap();
        p.f64_at(a.effect_time_at(a.start + Tick::from_seconds_f64(secs)))
    };
    // 20 % over 2 s is 10 %/s, stored as 1
    let line = with(format!("{t0},100,5,0,0,{sixth},1,{sixth};{t1},120,5,0,1,{sixth},0,{sixth};"));
    assert!((scale_at(&line, 1.0) - 110.0).abs() < 1e-6 && (scale_at(&line, 0.5) - 105.0).abs() < 1e-6, "{}", scale_at(&line, 0.5));
    let ease_in = with(format!("{t0},100,5,0,0,{sixth},1,{sixth};{t1},120,5,0,0,0.33333333333333331,0,{sixth};"));
    let half = scale_at(&ease_in, 1.0);
    assert!((half - (100.0 + 20.0 * 0.635)).abs() < 0.2, "{half}");
    let hold = with(format!("{t0},100,4,0,0,{sixth},0,{sixth};{t1},120,4,0,0,{sixth},0,{sixth};"));
    assert_eq!((scale_at(&hold, 1.0), scale_at(&hold, 1.99)), (100.0, 100.0));

    // edited in FilmCraft (the second key to 140): the line stays a line, its speed now 20 %/s
    let (mut imp, _) = import(line.as_bytes(), Some("/edits/show"));
    let sid = imp.project.sequences().next().map(|i| i.id).unwrap();
    let a = &mut imp.project.sequence_mut(sid).unwrap().video_tracks[0].items[0];
    let p = a.effect_mut("motion").and_then(|e| e.params.get_mut("scale")).unwrap();
    p.keyframes[1].value = ParamValue::Float(140.0);
    let (out, _) = crate::prproj_write::export(&imp.project, line.as_bytes(), &[], None).unwrap();
    let out = String::from_utf8(out).unwrap();
    let written = out.lines().find(|l| l.contains("<Name>Scale</Name>")).unwrap_or_default();
    assert!(written.contains(&format!("{t0},100,5,0,0,{sixth},2,")), "the leaving speed is now 2 (20 %/s) too: {written}");
    assert!(written.contains(&format!("{t1},140,5,0,2,")), "the arriving speed is 2: {written}");
    assert!((scale_at(&out, 1.0) - 120.0).abs() < 1e-6, "{}", scale_at(&out, 1.0));
}

#[test]
fn media_without_a_project_folder_keeps_the_absolute_path() {
    let (imp, _) = import(doc().as_bytes(), None);
    let paths: Vec<&str> = imp
        .project
        .items
        .values()
        .filter_map(|i| match &i.kind {
            ItemKind::Media(m) => match &m.media {
                MediaRef::File { path } => Some(path.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(paths, vec![r"L:\Edit\Footage\interview.mov"]);
}

#[test]
fn damaged_and_hostile_documents_never_panic() {
    let opts = ImportOptions::default();
    let good = gzip(doc().as_bytes());
    // truncated anywhere in the gzip stream or header
    for n in [1, 2, 9, 10, 14, 30, good.len() / 2, good.len() - 9] {
        let _ = import_with(&good[..n], Format::Prproj, &opts);
    }
    assert!(import_with(b"\x1f\x8b\x07\x00garbage", Format::Prproj, &opts).is_err());
    assert!(import_with(b"<xmeml version=\"4\"/>", Format::Prproj, &opts).is_err());
    assert!(matches!(import_with(b"<PremiereData Version=\"3\"/>", Format::Prproj, &opts), Err(Error::Empty)));

    // absurd numbers: inverted / overflowing ranges, zero and negative rates, broken rectangles,
    // unparsable keyframes, a link to a missing object and a sequence that contains itself
    let hostile = doc()
        .replace("<End>{}</End>", "")
        .replace(&format!("<End>{}</End>", 7 * S), &format!("<End>{}</End>", i64::MAX))
        .replace("<FrameRate>8467200000</FrameRate>", "<FrameRate>0</FrameRate>")
        .replace("<FrameRate>5292000</FrameRate>", "<FrameRate>-1</FrameRate>")
        .replace("0,0,1920,1080", "9,9,1,x")
        .replace("120,0,0", "NaN,0,0")
        .replace("<PlaybackSpeed>-2</PlaybackSpeed>", "<PlaybackSpeed>inf</PlaybackSpeed>")
        .replace(r#"<Source ObjectRef="11"/><InPoint>"#, r#"<Source ObjectRef="21"/><InPoint>"#)
        .replace(r#"<TrackItem Index="1" ObjectRef="70"/>"#, r#"<TrackItem Index="1" ObjectRef="999"/>"#);
    let (imp, _) = import_with(hostile.as_bytes(), Format::Prproj, &opts).unwrap();
    let seq =
        imp.project.sequence(imp.sequences.first().copied().unwrap_or(ItemId(0))).or_else(|| imp.project.sequences().next().and_then(|i| i.as_sequence()));
    assert!(seq.is_some());
}

#[test]
fn premiere_projects_are_import_only() {
    assert!(!Format::ALL.contains(&Format::Prproj));
    let (imp, _) = import(doc().as_bytes(), None);
    let seq = imp.sequences[0];
    assert!(crate::export(&imp.project, seq, Format::Prproj, &Default::default()).is_err());
}

#[test]
fn rates_from_ticks_per_frame_are_exact() {
    assert_eq!(rate_of(8_467_200_000), Some(FrameRate::new(30, 1)));
    assert_eq!(rate_of(10_594_584_000), Some(FrameRate::new(24_000, 1001)));
    assert_eq!(rate_of(4_237_833_600), Some(FrameRate::new(60_000, 1001)));
    assert_eq!(rate_of(0), None);
    assert_eq!(rate_of(1), None);
    assert_eq!(frame_rect(Some("0,0,2560,1440")), Some((2560, 1440)));
    assert_eq!(frame_rect(Some("10,10,5,5")), None);
}

#[test]
fn two_media_objects_for_one_file_become_one_item() {
    // the audio clip's source points at a second Media object with the same file
    let d = doc()
        .replace(r#"<AudioMediaSource ObjectID="76"><MediaSource><Media ObjectURef="media1"/>"#, r#"<AudioMediaSource ObjectID="76"><MediaSource><Media ObjectURef="media2"/>"#)
        .replace(
            "</PremiereData>",
            r#"<Media ObjectUID="media2"><RelativePath>.\Footage\interview.mov</RelativePath><FilePath>L:\Edit\Footage\interview.mov</FilePath></Media></PremiereData>"#,
        );
    let (imp, _) = import(d.as_bytes(), Some("/edits/show"));
    let media = imp.project.items.values().filter(|i| matches!(i.kind, ItemKind::Media(_))).count();
    assert_eq!(media, 1);
    let seq = imp.project.sequence(imp.sequences[0]).unwrap();
    assert_eq!(seq.audio_tracks[0].items[0].item, seq.video_tracks[0].items[0].item);
}

#[test]
fn the_sequence_premiere_had_open_comes_first_and_stills_get_no_placeholder_duration() {
    // a second, unrelated sequence listed first in the document, and Premiere's open-sequence hint
    let d = doc().replace(
        "<Sequence ObjectUID=\"seq1\">",
        "<Sequence ObjectUID=\"seq0\"><Name>Scratch</Name></Sequence>\n  <Sequence ObjectUID=\"seq1\"><Node><Properties><MZ.PrefixKey.OpenSequenceGuidList.1>seq1</MZ.PrefixKey.OpenSequenceGuidList.1></Properties></Node>",
    );
    let (imp, _) = import(d.as_bytes(), None);
    assert_eq!(imp.sequences.len(), 2);
    assert_eq!(imp.project.item(imp.sequences[0]).map(|i| i.name.as_str()), Some("Main Edit"));

    let still = doc().replace(r"Footage\interview.mov</RelativePath>", r"Footage\poster.png</RelativePath>");
    let (imp, _) = import(still.as_bytes(), Some("/edits/show"));
    let m = imp.project.items.values().find_map(|i| match &i.kind {
        ItemKind::Media(m) => Some(m),
        _ => None,
    });
    let m = m.unwrap();
    assert_eq!(m.info.kind, MediaKind::Still);
    // only as long as its longest use (15 s of source), not Premiere's 60 s / 12 h placeholder
    assert_eq!(m.info.duration, Tick(24 * S));
}

fn base64_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// The test project with a title on a second video track, made the way Premiere saves one: a
/// "Graphic" media without a file (the graphics importer, a still stream the canvas size) and a
/// text layer component whose `Source Text` holds the title (stored once, by hash).
pub(crate) fn doc_with_title() -> String {
    let src = base64_encode(&crate::prproj_text::tests::fifty());
    let title = format!(
        r#"
  <VideoClipTrack ObjectUID="vt2"><ClipTrack><Track><Index>1</Index></Track><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="900"/></TrackItems></ClipItems></ClipTrack></VideoClipTrack>
  <VideoClipTrackItem ObjectID="900"><ClipTrackItem><ComponentOwner><Components ObjectRef="901"/></ComponentOwner><TrackItem><Start>{s1}</Start><End>{s3}</End></TrackItem><SubClip ObjectRef="902"/></ClipTrackItem></VideoClipTrackItem>
  <VideoComponentChain ObjectID="901"><ComponentChain><Components><Component Index="0" ObjectRef="910"/></Components></ComponentChain></VideoComponentChain>
  <SubClip ObjectID="902"><Clip ObjectRef="903"/><MasterClip ObjectURef="gmc"/><Name>Graphic</Name></SubClip>
  <VideoClip ObjectID="903"><Clip><Source ObjectRef="904"/><InPoint>0</InPoint><OutPoint>{s2}</OutPoint></Clip></VideoClip>
  <MasterClip ObjectUID="gmc"><Clips><Clip Index="0" ObjectRef="905"/></Clips><Name>Graphic</Name></MasterClip>
  <VideoClip ObjectID="905"><Clip><Source ObjectRef="904"/></Clip></VideoClip>
  <VideoMediaSource ObjectID="904"><MediaSource><Media ObjectURef="gmedia"/></MediaSource></VideoMediaSource>
  <Media ObjectUID="gmedia"><VideoStream ObjectRef="906"/><FilePath>1196574294</FilePath><ImplementationID>42008e7a-de6f-4270-96de-7e287abb9b4b</ImplementationID><Title>Graphic</Title><Infinite>true</Infinite></Media>
  <VideoStream ObjectID="906"><FrameRate>8467200000</FrameRate><FrameRect>0,0,2560,1440</FrameRect><IsStill>true</IsStill></VideoStream>
  <VideoFilterComponent ObjectID="910"><Component><Params><Param Index="0" ObjectRef="911"/><Param Index="1" ObjectRef="912"/><Param Index="2" ObjectRef="913"/><Param Index="3" ObjectRef="914"/></Params><DisplayName>Text</DisplayName><InstanceName>50</InstanceName></Component><MatchName>AE.ADBE Text</MatchName></VideoFilterComponent>
  <ArbVideoComponentParam ObjectID="911"><Name>Source Text</Name><StartKeyframeValue Encoding="base64" BinaryHash="tt-1"/></ArbVideoComponentParam>
  <PointComponentParam ObjectID="912"><Name>Position</Name><StartKeyframe>-91445760000000000,0.0946:0.5449,0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe></PointComponentParam>
  <VideoComponentParam ObjectID="913"><Name>Scale</Name><StartKeyframe>-91445760000000000,100.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="914"><Name>Horizontal Scale</Name><StartKeyframe>-91445760000000000,100.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <Blob ObjectID="915"><Data Encoding="base64" BinaryHash="tt-1">{src}</Data></Blob>
</PremiereData>"#,
        s1 = S,
        s2 = 2 * S,
        s3 = 3 * S
    );
    doc()
        .replace(
            r#"<Tracks><Track Index="0" ObjectURef="vt1"/></Tracks>"#,
            r#"<Tracks><Track Index="0" ObjectURef="vt1"/><Track Index="1" ObjectURef="vt2"/></Tracks>"#,
        )
        .replace("</PremiereData>", &title)
}

#[test]
fn premiere_titles_come_in_as_graphics_with_their_text() {
    let (imp, report) = import(doc_with_title().as_bytes(), Some("/edits/show"));
    let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let clip = seq.video_tracks[1].items.first().expect("the title clip");
    let item = imp.project.item(clip.item).unwrap();
    assert!(matches!(item.kind, filmcraft_project::ItemKind::Graphic { width: 2560, height: 1440, .. }), "{:?}", item.kind);
    let e = clip.effect("graphic_text").expect("a text layer");
    let v = |id: &str| e.params.get(id).map(|p| p.value.clone()).unwrap();
    assert_eq!(v("text"), ParamValue::Text("50".into()));
    assert_eq!((v("font"), v("font_style")), (ParamValue::Text("FatFrank".into()), ParamValue::Text("Heavy".into())));
    assert!(matches!(v("size"), ParamValue::Float(s) if (s - 399.41).abs() < 1e-2));
    assert_eq!(
        (v("stroke"), v("stroke_width"), v("box_width"), v("align")),
        (ParamValue::Bool(true), ParamValue::Float(26.0), ParamValue::Float(2125.0), ParamValue::Choice(1))
    );
    assert_eq!(v("top_left_origin"), ParamValue::Bool(true));
    let ParamValue::Vec2(p) = v("position") else { panic!() };
    assert!((p.x - 0.0946 * 2560.0).abs() < 0.01 && (p.y - 0.5449 * 1440.0).abs() < 0.01, "{p:?}");
    assert!(!report.to_string().contains("Source Text"), "{report}");
    assert!(!report.to_string().contains("no file path"), "not offline: {report}");
}

/// A mask component as Premiere saves one (`AE.ADBE AEMask2`): the path by hash, Position /
/// Anchor Point, Feather, Expansion, Inverted and Type (parameters found by `ParameterID`).
fn mask_xml(id: u32, path_hash: &str, path_keys: Option<&str>, pos: &str, anchor: &str, feather: f64, expansion: f64, inverted: bool, kind: u32) -> String {
    let p = |n: u32| id + n;
    let keys = path_keys.map(|k| format!("<Keyframes>{k}</Keyframes>")).unwrap_or_default();
    format!(
        r#"
  <VideoFilterComponent ObjectID="{id}"><Component><Params><Param Index="0" ObjectRef="{p1}"/><Param Index="1" ObjectRef="{p2}"/><Param Index="2" ObjectRef="{p3}"/><Param Index="3" ObjectRef="{p4}"/><Param Index="4" ObjectRef="{p5}"/><Param Index="5" ObjectRef="{p6}"/><Param Index="6" ObjectRef="{p7}"/></Params><DisplayName>Mask2</DisplayName><InstanceName>01</InstanceName></Component><MatchName>AE.ADBE AEMask2</MatchName></VideoFilterComponent>
  <ArbVideoComponentParam ObjectID="{p1}"><Name>Path</Name><ParameterControlType>22</ParameterControlType><ParameterID>7</ParameterID><StartKeyframePosition>-91445760000000000</StartKeyframePosition>{keys}<StartKeyframeValue Encoding="base64" BinaryHash="{path_hash}"/></ArbVideoComponentParam>
  <PointComponentParam ObjectID="{p2}"><Name>Position</Name><ParameterID>9</ParameterID><StartKeyframe>-91445760000000000,{pos},0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe></PointComponentParam>
  <PointComponentParam ObjectID="{p3}"><Name>Anchor Point</Name><ParameterID>10</ParameterID><StartKeyframe>-91445760000000000,{anchor},0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe></PointComponentParam>
  <VideoComponentParam ObjectID="{p4}"><Name>Feather</Name><ParameterID>14</ParameterID><StartKeyframe>-91445760000000000,{feather}.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="{p5}"><Name>Expansion</Name><ParameterID>16</ParameterID><StartKeyframe>-91445760000000000,{expansion}.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="{p6}"><ParameterID>17</ParameterID><StartKeyframe>-91445760000000000,{inverted},0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="{p7}"><Name>Type</Name><ParameterControlType>10</ParameterControlType><ParameterID>22</ParameterID><StartKeyframe>-91445760000000000,{kind},0,0,0,0,0,0</StartKeyframe></VideoComponentParam>"#,
        p1 = p(1),
        p2 = p(2),
        p3 = p(3),
        p4 = p(4),
        p5 = p(5),
        p6 = p(6),
        p7 = p(7),
    )
}

/// The test project with masks: a moved ellipse on the second clip's Gaussian Blur, an Object
/// Mask on its Opacity, and an inverted rectangle whose path is keyed on a Tint of the first clip.
pub(crate) fn doc_with_masks() -> String {
    use filmcraft_project::MaskPath;
    const ELLIPSE: &str = "AgAAAAEAAABWeQAAAAIAAAAEAAAAAQAAAELE7T5glO4+VsyYPmCU7j4XXiE/YJTuPgEAAADNzkM/gNkZP83OQz9EwwY/zc5DP73vLD8BAAAAQsTtPtFoPD8XXiE/0Wg8P1bMmD7RaDw/AQAAANjVJz6A2Rk/2NUnPr3vLD/Y1Sc+RMMGPwE=";
    let rect = |x0: f64, x1: f64| base64_encode(&crate::prproj_mask::encode(&MaskPath::rect(x0, 0.1, x1, 0.3), 7));
    let keys = format!("{},{};{},{};", 10 * S, rect(0.1, 0.2), 12 * S, rect(0.5, 0.6));
    let extra = [
        mask_xml(500, "ellipse-1", None, "0.5:0.6", "0.46439:0.600975", 44.0, -56.0, false, 1),
        mask_xml(520, "object-1", None, "0.5:0.5", "0.5:0.5", 0.0, 0.0, false, 4),
        mask_xml(540, "rect-1", Some(&keys), "0.5:0.5", "0.5:0.5", 13.0, 4.0, true, 2),
    ]
    .concat();
    let tail = format!(
        r#"{extra}
  <VideoFilterComponent ObjectID="560"><Component><Params><Param Index="0" ObjectRef="561"/></Params><ID>4</ID><DisplayName>Tint</DisplayName></Component><SubComponents><SubComponent Index="0" ObjectRef="540"/></SubComponents><MatchName>AE.ADBE Tint</MatchName></VideoFilterComponent>
  <VideoComponentParam ObjectID="561"><Name>Amount to Tint</Name><StartKeyframe>-91445760000000000,100.,0,0,0,0,0,0</StartKeyframe></VideoComponentParam>
  <Blob ObjectID="570"><Data Encoding="base64" BinaryHash="ellipse-1">{ELLIPSE}</Data></Blob>
  <Blob ObjectID="571"><Data Encoding="base64" BinaryHash="object-1">AgAAAAAAAAA=</Data></Blob>
  <Blob ObjectID="572"><Data Encoding="base64" BinaryHash="rect-1">{r}</Data></Blob>
</PremiereData>"#,
        r = rect(0.1, 0.2)
    );
    doc()
        .replace(
            r#"<DisplayName>Gaussian Blur (Legacy)</DisplayName></Component>"#,
            r#"<DisplayName>Gaussian Blur (Legacy)</DisplayName></Component><SubComponents><SubComponent Index="0" ObjectRef="500"/></SubComponents>"#,
        )
        .replace(
            r#"<DisplayName>Opacity</DisplayName></Component>"#,
            r#"<DisplayName>Opacity</DisplayName></Component><SubComponents><SubComponent Index="0" ObjectRef="520"/></SubComponents>"#,
        )
        .replace(
            r#"<Component Index="1" ObjectRef="48"/></Components>"#,
            r#"<Component Index="1" ObjectRef="48"/><Component Index="2" ObjectRef="560"/></Components>"#,
        )
        .replace("</PremiereData>", &tail)
}

/// Masks on effects come in (a masked Gaussian Blur used to blur the whole clip): the path in
/// source pixels with the mask's own transform applied, Feather, Expansion, Inverted, path
/// keyframes in clip time. Premiere's Object Masks have no path; they are reported.
#[test]
fn effect_masks_come_in_with_their_shape_and_settings() {
    use filmcraft_geom::Vec2;
    let (imp, report) = import(doc_with_masks().as_bytes(), Some("/edits/show"));
    let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let (first, second) = (&seq.video_tracks[0].items[0], &seq.video_tracks[0].items[1]);
    // the ellipse, centred on the mask's Position (0.5, 0.6 of the 2560 × 1440 source)
    let blur = second.effect("gaussian_blur").expect("the blur");
    assert_eq!(blur.masks.len(), 1, "{report}");
    let m = &blur.masks[0];
    let path = m.path_at(filmcraft_time::Tick(0));
    let c = path.centroid();
    assert!((c - Vec2::new(1280.0, 864.0)).length() < 0.5, "{c:?}");
    assert!((path.vertices[1].p.x - c.x - 0.300_48 * 2560.0).abs() < 0.5, "{:?}", path.vertices[1]);
    assert_eq!((m.feather.f64_at(filmcraft_time::Tick(0)), m.expansion.f64_at(filmcraft_time::Tick(0)), m.inverted), (44.0, -56.0, false));
    // the Object Mask is not carried over, and says so
    assert!(second.effect("opacity").is_some_and(|o| o.masks.is_empty()));
    assert!(report.to_string().contains("Object mask"), "{report}");
    // the keyed rectangle: inverted, keys at media times 10 and 12 s
    let tint = first.effect("tint").expect("the tint");
    let r = &tint.masks[0];
    assert!(r.inverted);
    assert_eq!(r.path.keyframes.iter().map(|k| k.time).collect::<Vec<_>>(), vec![filmcraft_time::Tick(10 * S), filmcraft_time::Tick(12 * S)]);
    let at = |t| r.path_at(t).vertices[0].p;
    assert!((at(filmcraft_time::Tick(10 * S)) - Vec2::new(256.0, 144.0)).length() < 1e-3);
    assert!((at(filmcraft_time::Tick(11 * S)) - Vec2::new(0.3 * 2560.0, 144.0)).length() < 1e-3, "halfway");
}

/// Premiere's synthetic media: an adjustment layer (a master clip marked `IsAdjustmentLayer` over
/// "Black Video") and a Color Matte (its colour in `ImporterPrefs`) come in as FilmCraft's
/// adjustment layer and colour matte, not as empty graphics.
#[test]
fn adjustment_layers_and_color_mattes_come_in_as_themselves() {
    let extra = r#"
  <VideoClipTrack ObjectUID="vt2"><ClipTrack><Track><Index>1</Index></Track><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="700"/><TrackItem Index="1" ObjectRef="710"/></TrackItems></ClipItems></ClipTrack></VideoClipTrack>
  <VideoClipTrackItem ObjectID="700"><ClipTrackItem><TrackItem><Start>0</Start><End>254016000000</End></TrackItem><SubClip ObjectRef="701"/></ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="701"><Clip ObjectRef="702"/><MasterClip ObjectURef="adjmc"/><Name>Adjustment Layer</Name></SubClip>
  <VideoClip ObjectID="702"><Clip><Source ObjectRef="703"/><InPoint>0</InPoint><OutPoint>254016000000</OutPoint></Clip></VideoClip>
  <MasterClip ObjectUID="adjmc"><Clips><Clip Index="0" ObjectRef="702"/></Clips><Name>Adjustment Layer</Name><IsAdjustmentLayer>true</IsAdjustmentLayer></MasterClip>
  <VideoMediaSource ObjectID="703"><MediaSource><Media ObjectURef="blak"/></MediaSource></VideoMediaSource>
  <Media ObjectUID="blak"><VideoStream ObjectRef="704"/><FilePath>1112293707</FilePath><ImplementationID>42008e7a-de6f-4270-96de-7e287abb9b4b</ImplementationID><Title>Black Video</Title><Infinite>true</Infinite></Media>
  <VideoStream ObjectID="704"><FrameRate>8467200000</FrameRate><FrameRect>0,0,1920,1080</FrameRect><IsStill>true</IsStill></VideoStream>
  <VideoClipTrackItem ObjectID="710"><ClipTrackItem><TrackItem><Start>254016000000</Start><End>508032000000</End></TrackItem><SubClip ObjectRef="711"/></ClipTrackItem></VideoClipTrackItem>
  <SubClip ObjectID="711"><Clip ObjectRef="712"/><MasterClip ObjectURef="colmc"/><Name>Color Matte</Name></SubClip>
  <VideoClip ObjectID="712"><Clip><Source ObjectRef="713"/><InPoint>0</InPoint><OutPoint>254016000000</OutPoint></Clip></VideoClip>
  <MasterClip ObjectUID="colmc"><Clips><Clip Index="0" ObjectRef="712"/></Clips><Name>Color Matte</Name></MasterClip>
  <VideoMediaSource ObjectID="713"><MediaSource><Media ObjectURef="colr"/></MediaSource></VideoMediaSource>
  <Media ObjectUID="colr"><VideoStream ObjectRef="714"/><ImporterPrefs Encoding="base64" BinaryHash="cp-1">/+QAAAEAAAA=</ImporterPrefs><FilePath>1129270354</FilePath><ImplementationID>42008e7a-de6f-4270-96de-7e287abb9b4b</ImplementationID><Title>Color Matte</Title><Infinite>true</Infinite></Media>
  <VideoStream ObjectID="714"><FrameRate>8467200000</FrameRate><FrameRect>0,0,1920,1080</FrameRect><IsStill>true</IsStill></VideoStream>
</PremiereData>"#;
    let doc = doc()
        .replace(
            r#"<Tracks><Track Index="0" ObjectURef="vt1"/></Tracks>"#,
            r#"<Tracks><Track Index="0" ObjectURef="vt1"/><Track Index="1" ObjectURef="vt2"/></Tracks>"#,
        )
        .replace("</PremiereData>", extra);
    let (imp, report) = import(doc.as_bytes(), Some("/edits/show"));
    let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let v2 = &seq.video_tracks[1].items;
    assert_eq!(v2.len(), 2, "{report}");
    let adj = imp.project.item(v2[0].item).unwrap();
    assert!(matches!(adj.kind, ItemKind::AdjustmentLayer { width: 1920, height: 1080, .. }), "{:?}", adj.kind);
    let matte = imp.project.item(v2[1].item).unwrap();
    let Some(MediaRef::Generator(filmcraft_media::Generator::ColorMatte { color })) = matte.as_media().map(|m| &m.media) else { panic!("{:?}", matte.kind) };
    assert!((color[0] - 1.0).abs() < 1e-6 && (color[1] - 228.0 / 255.0).abs() < 1e-6 && color[2] == 0.0, "{color:?}");
    // both keep their Premiere media, for writing back
    assert!(adj.metadata.contains_key(META_MEDIA) && matte.metadata.contains_key(META_MEDIA));
    let (out, rep) = crate::prproj_write::export(&imp.project, doc.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), doc, "an unchanged round trip is the same document ({rep})");
}

/// Premiere's current Gaussian Blur (`AE.Impact_Blur_FX`, "Amount") comes in as FilmCraft's
/// Gaussian Blur with a Blurriness (it imported with none, so a masked blur did nothing), and
/// an unchanged project still writes back unchanged.
#[test]
fn the_current_gaussian_blur_amount_comes_in() {
    let doc = doc()
        .replace(
            r#"<DisplayName>Gaussian Blur (Legacy)</DisplayName></Component><MatchName>AE.ADBE Gaussian Blur</MatchName>"#,
            r#"<DisplayName>Gaussian Blur</DisplayName></Component><MatchName>AE.Impact_Blur_FX</MatchName>"#,
        )
        .replace(
            r#"<Name>Blurriness</Name><IsTimeVarying>true</IsTimeVarying><StartKeyframe>-91445760000000000,0,0,0,0,0,0,0</StartKeyframe><Keyframes>"#,
            r#"<Name>Amount</Name><IsTimeVarying>true</IsTimeVarying><StartKeyframe>-91445760000000000,20.,0,0,0,0,0,0</StartKeyframe><Keyframes>"#,
        );
    assert!(doc.contains("AE.Impact_Blur_FX") && doc.contains("<Name>Amount</Name>"));
    let (imp, report) = import(doc.as_bytes(), Some("/edits/show"));
    let seq = imp.project.sequences().next().and_then(|i| i.as_sequence()).unwrap();
    let blur = seq.video_tracks[0].items[1].effect("gaussian_blur").expect("the blur");
    let p = &blur.params["blurriness"];
    // keyed 0 → 40 Amount: Blurriness × 1.624 / 0.285 (measured against a Premiere render)
    let ParamValue::Float(end) = p.keyframes.last().unwrap().value else { panic!() };
    assert!((end - 40.0 * 1.624 / 0.285).abs() < 1e-6, "{end}");
    assert!(!report.to_string().contains("approximately"), "{report}");
    let (out, rep) = crate::prproj_write::export(&imp.project, doc.as_bytes(), &[], None).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), doc, "unchanged ({rep})");
}

/// Premiere escapes `RelativePath` twice (`&amp;amp;` for `&`): the path read is the file's real
/// name, so the media is found (it used to keep `&amp;` and be written back as `&amp;amp;` in
/// every path, offline in Premiere).
#[test]
fn ampersands_in_media_paths_are_read_once() {
    let text = doc()
        .replace(r"<RelativePath>.\Footage\interview.mov</RelativePath>", r"<RelativePath>.\Footage\Morning &amp;amp; Night.mov</RelativePath>")
        .replace(r"<FilePath>L:\Edit\Footage\interview.mov</FilePath>", r"<FilePath>L:\Edit\Footage\Morning &amp; Night.mov</FilePath>");
    let path_of = |imp: &crate::Imported| -> String {
        imp.project
            .items
            .values()
            .find_map(|i| match &i.kind {
                ItemKind::Media(m) => match &m.media {
                    MediaRef::File { path } => Some(path.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap()
    };
    let (imp, _) = import(text.as_bytes(), Some("/edits/show"));
    assert_eq!(path_of(&imp), "/edits/show/Footage/Morning & Night.mov");
    let (imp, _) = import(text.as_bytes(), None);
    assert_eq!(path_of(&imp), r"L:\Edit\Footage\Morning & Night.mov");
    // an earlier FilmCraft wrote the absolute path escaped twice too
    let twice = text.replace(r"Morning &amp; Night.mov</FilePath>", r"Morning &amp;amp; Night.mov</FilePath>");
    let (imp, _) = import(twice.as_bytes(), None);
    assert_eq!(path_of(&imp), r"L:\Edit\Footage\Morning & Night.mov");
}

/// Sequence markers in Premiere's own place (the `Sequence`'s `MarkerOwner`) are read with the
/// ones on its master clip; clip gain (`Gain` on an audio clip, linear) is read in dB.
#[test]
fn sequence_markers_on_the_sequence_and_clip_gain_are_read() {
    let text = doc()
        .replace(
            "<Sequence ObjectUID=\"seq1\">\n    <Name>Main Edit</Name>",
            "<Sequence ObjectUID=\"seq1\">\n    <MarkerOwner Version=\"1\"><Markers ObjectRef=\"990\"/></MarkerOwner>\n    <Name>Main Edit</Name>",
        )
        .replace(
            "</PremiereData>",
            "  <Markers ObjectID=\"990\"><Markers><Marker Index=\"0\"><First>m9</First><Second ObjectRef=\"991\"/></Marker></Markers></Markers>\n  <Marker ObjectID=\"991\"><DVAMarker>{\"DVAMarker\":{\"mMarkerID\":\"m9\",\"mStartTime\":{\"ticks\":762048000000},\"mName\":\"On the sequence\",\"mType\":\"Comment\"}}</DVAMarker></Marker>\n</PremiereData>",
        )
        .replace(
            "<AudioClip ObjectID=\"72\"><Clip><Source ObjectRef=\"76\"/><InPoint>2540160000000</InPoint><OutPoint>3810240000000</OutPoint></Clip></AudioClip>",
            "<AudioClip ObjectID=\"72\"><Clip><Source ObjectRef=\"76\"/><InPoint>2540160000000</InPoint><OutPoint>3810240000000</OutPoint></Clip><Gain>1.4125375446227544</Gain></AudioClip>",
        );
    assert!(text.contains("<Gain>"), "the fixture changed");
    let (imp, _) = import(text.as_bytes(), None);
    let seq = imp.project.sequences().find_map(|i| i.as_sequence()).unwrap();
    let names: Vec<&str> = seq.markers.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Act 2", "On the sequence"]);
    let gain = seq.audio_tracks[0].items[0].gain_db;
    assert!((gain - 3.0).abs() < 1e-6, "{gain}");
}

/// The Audio Track Mixer's settings and the clip Panner as Premiere saves them (Round 5 B3): a
/// track's fader (`Volume`, a linear gain) and panner (`Balance` / `Pan`, 0…1 with the centre at
/// 0.5), `ChannelType` 0 for a mono track, the Mix's fader on the group's `MasterTrack`, and a
/// clip's own `Panner`. A setting at 1 (hard right) is saved with no value.
#[test]
fn track_mix_master_and_clip_pan_are_read() {
    let class = "a714635e-a628-4b27-9d59-77eba47dbc1a";
    let fader = |id: u32, gain: Option<f64>| {
        let value = gain.map(|g| format!("<StartKeyframe>-91445760000000000,{g},0,0,0,0,0,0</StartKeyframe>")).unwrap_or_default();
        format!(
            "<AudioComponentChain ObjectID=\"{id}\"><ComponentChain><Components><Component Index=\"0\" ObjectRef=\"{}\"/></Components></ComponentChain></AudioComponentChain>\n  <AudioFader ObjectID=\"{}\"><AudioComponent><Component><Params><Param Index=\"0\" ObjectRef=\"{}\"/></Params></Component></AudioComponent></AudioFader>\n  <AudioComponentParam ObjectID=\"{}\" ClassID=\"{class}\">{value}<Name>Volume</Name></AudioComponentParam>\n",
            id + 1,
            id + 1,
            id + 2,
            id + 2
        )
    };
    let panner = |id: u32, tag: &str, name: &str, value: Option<&str>| {
        let value =
            value.map(|v| format!("<StartKeyframe>-91445760000000000,{v},0,0,0,0,0,0</StartKeyframe><CurrentValue>{v}</CurrentValue>")).unwrap_or_default();
        format!(
            "<{tag} ObjectID=\"{id}\"><PanProcessor><AudioComponent><Component><Params><Param Index=\"0\" ObjectRef=\"{}\"/></Params></Component></AudioComponent></PanProcessor></{tag}>\n  <AudioComponentParam ObjectID=\"{}\" ClassID=\"{class}\">{value}<IsTimeVarying>false</IsTimeVarying><Name>{name}</Name><IsInverted>true</IsInverted></AudioComponentParam>\n",
            id + 1,
            id + 1
        )
    };
    let objects = [
        // A2: a mono track, its fader at −6 dB, panned hard left; A1 has no mixer objects
        "  <AudioClipTrack ObjectUID=\"at2\"><ClipTrack><Track><Index>1</Index></Track></ClipTrack><AudioTrack><ComponentOwner><Components ObjectRef=\"900\"/></ComponentOwner><Panner ObjectRef=\"910\"/><ChannelType>0</ChannelType></AudioTrack></AudioClipTrack>\n".to_string(),
        fader(900, Some(10f64.powf(-6.0 / 20.0))),
        panner(910, "MonoToStereoPanProcessor", "Pan", Some("0.")),
        // the Mix at −3 dB
        "  <AudioMixTrack ObjectID=\"920\"><AudioTrack><ComponentOwner><Components ObjectRef=\"930\"/></ComponentOwner></AudioTrack></AudioMixTrack>\n".to_string(),
        fader(930, Some(10f64.powf(-3.0 / 20.0))),
        // the A1 clip's Balance at +100: no value
        panner(940, "StereoToStereoPanProcessor", "Balance", None),
    ]
    .concat();
    let text = doc()
        .replace(
            "<Track Index=\"0\" ObjectURef=\"at1\"/></Tracks><FrameRate>5292000</FrameRate></TrackGroup>",
            "<Track Index=\"0\" ObjectURef=\"at1\"/><Track Index=\"1\" ObjectURef=\"at2\"/></Tracks><FrameRate>5292000</FrameRate></TrackGroup><MasterTrack ObjectRef=\"920\"/>",
        )
        .replace("</ClipTrackItem></AudioClipTrackItem>", "</ClipTrackItem><Panner ObjectRef=\"940\"/></AudioClipTrackItem>")
        .replace("</PremiereData>", &format!("{objects}</PremiereData>"));
    assert!(text.contains("MasterTrack") && text.contains("Panner ObjectRef=\"940\""), "the fixture changed");
    let (imp, _) = import(text.as_bytes(), None);
    let seq = imp.project.sequences().find_map(|i| i.as_sequence()).unwrap();
    let (a1, a2) = (&seq.audio_tracks[0], &seq.audio_tracks[1]);
    assert_eq!((a1.channels, a1.volume_db, a1.pan), (filmcraft_project::AudioChannels::Stereo, 0.0, 0.0));
    assert_eq!((a2.channels, a2.pan), (filmcraft_project::AudioChannels::Mono, -100.0));
    assert!((a2.volume_db + 6.0).abs() < 1e-9, "{}", a2.volume_db);
    assert!((seq.master_volume_db + 3.0).abs() < 1e-9, "{}", seq.master_volume_db);
    let balance = a1.items[0].effect("panner").and_then(|e| e.param("balance")).map(|p| p.value.clone());
    assert_eq!(balance, Some(ParamValue::Float(100.0)));
    // a clip without a Panner keeps FilmCraft's centred default
    let (imp, _) = import(doc().as_bytes(), None);
    let seq = imp.project.sequences().find_map(|i| i.as_sequence()).unwrap();
    assert!(seq.audio_tracks[0].items[0].effect("panner").and_then(|e| e.param("balance")).is_none_or(|p| p.value == ParamValue::Float(0.0)));
    assert_eq!(seq.master_volume_db, 0.0);
}
