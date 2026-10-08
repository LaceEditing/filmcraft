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
  <VideoComponentParam ObjectID="45"><Name>Scale</Name><StartKeyframe>-91445760000000000,100,0,0,0,0,0,0</StartKeyframe><Keyframes>{s10},100,0,0,0,0.1,1.5,0.1;{s12},120,0,0,1.5,0.1,0,0.1;</Keyframes></VideoComponentParam>
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
  <VideoComponentParam ObjectID="57"><Name>Blurriness</Name><StartKeyframe>-91445760000000000,0,0,0,0,0,0,0</StartKeyframe><Keyframes>{s20},0,5,0,0,0.33333333333333331,0,0.33333333333333331;{s22},40,5,2,0,0.25,0,0.5;</Keyframes></VideoComponentParam>
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
    assert_eq!(m.info.audio.as_ref().map(|a| (a.sample_rate, a.channels)), Some((48_000, 2)));
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

    // Motion: Scale keyframes move from source time (10 s, 12 s) to clip time (0 s, 2 s)
    let scale = a.effect("motion").and_then(|e| e.params.get("scale")).unwrap();
    let keys: Vec<(Tick, f64)> = scale.keyframes.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect();
    assert_eq!(keys, vec![(Tick(0), 100.0), (Tick(2 * S), 120.0)]);
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

    // a FilmCraft effect with the same name: eased keyframes in clip time (the clip plays its
    // source at 2x, so source 20 s / 22 s are clip 0 s / 1 s), a boolean, and the choice reported
    let blur = b.effect("gaussian_blur").expect("Gaussian Blur imported");
    let keys = &blur.params["blurriness"].keyframes;
    assert_eq!(keys.iter().map(|k| (k.time, k.value.as_f64().unwrap())).collect::<Vec<_>>(), vec![(Tick(0), 0.0), (Tick(S), 40.0)]);
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
