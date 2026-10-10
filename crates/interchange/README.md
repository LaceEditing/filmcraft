# filmcraft-interchange

Timeline interchange for FilmCraft: CMX 3600 EDL, Final Cut Pro 7 XML (xmeml), FCPXML,
OpenTimelineIO, AAF and OMF import and export, plus Avid ALE import and Premiere Pro projects
(`.prproj`: import, write-back, and new projects). Layer L2: no file I/O; media
references are strings and audio essence is supplied by the caller (the engine).

## Specifications

Clean-room: written from public specifications only. The AAF SDK, pyaaf2, the OMF Toolkit and
other implementations were not consulted. Where a specification leaves a layout open (how a
nested sequence is written, how several video tracks share a slot), what Premiere Pro does was
read off files it exports, never from the application itself.

| Format | Document | Edition | Used for |
|---|---|---|---|
| AAF | AMWA *AAF Object Specification* | v1.1 (2005) | classes, properties (ids shared with SMPTE ST 377-1 local tags), mob / slot / segment model, sequences and transitions, operation groups, parameters, definitions, descriptors, locators, essence data |
| AAF | AMWA *AAF Low-Level Container Specification* | v1.0.1 | objects as structured storages, `properties` streams, stored forms, strong / weak reference collections and their index streams, `referenced properties` |
| AAF | AMWA *AAF Edit Protocol* | v1.0 (AMWA AS-01) | top-level composition, mob chains (composition → master → file → physical source), operation definitions (dissolve, fade to black, audio gain / dissolve), usage codes |
| AAF | SMPTE RP 224 / RP 210 registers | — | data definition, class and type labels |
| AAF | Microsoft [MS-CFB] | rev. 10.0 | the container, see [`filmcraft-cfb`](../cfb/README.md) |
| OMF | Avid *OMF Interchange Specification* | Version 2.0 (1997) | classes (`HEAD`, `CMOB`, `MMOB`, `SMOB`, `MSLT`, `TRKD`, `SEQU`, `SCLP`, `FILL`, `TRAN`, `EFFE`, `ESLT`, `CVAL`, `VVAL`, `TCCP`, `WAVD`, `AIFD`, `WAVE`, `AIFC`, locators), property and type names |
| OMF | Apple *Bento Specification* | revision 1.0d5 | container label, TOC encoding, objects / properties / types, references |
| WAVE / AIFF | Microsoft RIFF WAVE; Apple AIFF 1.3 | — | embedded and separate audio files |

The other formats' editions are listed in the module docs (`edl`, `fcp7`, `fcpxml`, `otio`, `ale`).

## AAF and OMF

`comp` holds a format-neutral mob-style model (compositions of slots whose sequences of fillers,
source clips and overlapping transitions reference media through master and file source mobs).
`aaf` and `omf` serialise it; both import back into the same model and from there into FilmCraft
sequences and media. Mapping details (what each FilmCraft feature becomes) are in the `aaf` and
`omf` module docs. Times are kept in ticks in the model; compositions use the sequence frame rate
for picture and the sample rate for sound slots, so audio edits are sample-exact.

Embedded / consolidated audio (`essence`): `audio_needs` lists the media ranges an export
references (per media item or per clip, with handles); the engine decodes or renders them and
passes `AudioEssence` (embedded PCM or a written file) back in `MediaOptions`.

Not represented: speed changes and frame holds (exported at 100 % with a report entry),
graphics and synthetic media (gaps), video effects other than transitions, clip
markers (written to the master mob, so they come back as media markers), OMF video and markers.

## Nested sequences

For the formats Premiere Pro exports (FCP7 XML, OTIO, EDL, AAF, OMF) a nested sequence is written
the way Premiere Pro 26.5.2 writes it, as seen in the files it exports for a sequence with a nest.
Premiere Pro does not write FCPXML.

| Format | Written as | On import into FilmCraft |
|---|---|---|
| FCP7 XML | a clip item holding the nested `<sequence>` (by id after the first use) | a sequence the clips use |
| FCPXML | a `ref-clip` to a `media` resource holding the sequence | a sequence the clips use |
| OTIO | a `Stack` inside the track | a sequence the clips use |
| EDL | one event: reel `AX` in every reel mode, the nested sequence's name as the clip name, its own time as source timecode (the report says the nest's edit is not in the EDL) | an ordinary clip of that name |
| AAF | a composition mob of its own (not tagged top-level); the nest's clips are source clips that point at it, at its first track of their kind. With embedded or consolidated audio the media inside the nest is prepared too (`NestNeeds::Inside`) | a sequence the clips use, read once however many clips use it |
| OMF | the nested sequence's sound, mixed by the engine with what is on the clip (`NestNeeds::Render`), as one clip named after the sequence. Without that essence: a gap, and the report names the sequence | media in the document |

A sequence that claims to be inside itself (a damaged project or file) is a gap with a report
entry in both directions; nothing recurses without a bound.

The AAF reader also accepts Premiere Pro's layout of a composition: all video tracks in ONE slot
(a nested scope with a segment per track, lowest first), and "top-level" tagged on nested
compositions too (the top-level ones are those no other composition uses).

## Graphics and adjustment layers

No format here has an equivalent of a FilmCraft graphic (title) clip or an adjustment layer, and
none of the exports brings one back. Each such clip is named in the export `Report` as a warning
(`graphic clip "Title" at frame 48 …`, one line per clip), with what was written instead:

| Format | Written as | On import into FilmCraft |
|---|---|---|
| FCP7 XML | a clip item without media (its layers as FilmCraft filter ids) | not read back; the import report names the skipped clip item |
| FCPXML | a clip without media | not read back |
| OTIO | a clip with a `MissingReference` (effects in the `filmcraft` metadata) | offline media |
| EDL | an ordinary event with the reel the reel mode gives | an ordinary clip |
| AAF | a gap | nothing |

OMF carries no video at all.

## Premiere Pro projects (`.prproj`)

Import (`prproj`), write-back into the project they came from (`prproj_write`), and new projects
for FilmCraft projects that did not come from Premiere (`prproj_new`). There is no
published specification; like the Premiere-exported XML layouts above, everything was read off
project files saved by Premiere Pro, never from the application. The observations come from 461
projects saved by Premiere Pro 2025–2026 (1 to 230 media files, up to ~4 000 clips each), which
also form the regression corpus (`examples/prproj_corpus.rs`, run against local projects; never
committed).

### Format

- A gzip member (RFC 1952, deflate per RFC 1951; inflated and written with `miniz_oxide`) holding
  UTF-8 XML whose root is `<PremiereData Version="3">`. Hand-made files may be plain XML.
- The root's children are a flat object table. Objects carry `ObjectID` (integer) or `ObjectUID`
  (UUID) and point at each other with `ObjectRef` / `ObjectURef`. Nested elements can reuse ids in
  private namespaces (a `Project`'s view state has its own `ObjectID="1"`), so only the root's
  children are indexed. Some opaque data is stored once and referenced by `BinaryHash`.
- Times are ticks at 254 016 000 000 per second, the same unit as `Tick`. Frame rates are ticks per
  frame (`8467200000` = 30 fps, `10594584000` = 23.976); audio rates ticks per sample (`5292000`
  = 48 kHz). `-91445760000000000` in a `StartKeyframe` means "no time" (a static value).
- Bins: `RootProjectItem` → `ProjectItemContainer/Items` → `BinProjectItem` (recursive) or
  `ClipProjectItem` → `MasterClip` → `Clips` → `VideoClip`/`AudioClip` → `Clip/Source` →
  `VideoMediaSource`/`AudioMediaSource` (`MediaSource/Media`) or `VideoSequenceSource`
  (`SequenceSource/Sequence`). A sequence is a `ClipProjectItem` whose clip source is a sequence.
  Several `Media` objects can stand for one file; FilmCraft makes one item of them.
- `Media`: `FilePath` / `ActualMediaFilePath` (absolute, as saved) and `RelativePath` (to the
  project, `\`-separated, sometimes nonsense such as `..\..\G:\…`). Import uses a plain relative
  path against the project's folder, else the absolute path; missing files come in offline.
- `Sequence/TrackGroups/TrackGroup/Second` → `VideoTrackGroup` / `AudioTrackGroup`
  (`TrackGroup/Tracks/Track[@Index]`, `TrackGroup/FrameRate`, `NextTrackID`, `FrameRect`,
  `PreviewItems` = cached renders) → `VideoClipTrack` / `AudioClipTrack` →
  `ClipTrack/ClipItems/TrackItems` and `ClipTrack/TransitionItems/TrackItems`. Premiere allows
  clips to overlap on a track; FilmCraft cannot, so the importer moves such a clip to a free track.
- A clip: `ClipTrackItem/TrackItem/Start` (absent = 0) and `End`; `SubClip` (`Name`,
  `MasterClip`) → `Clip` → `Clip/InPoint`, `OutPoint`, optional `PlaybackSpeed` (negative =
  reversed); `FrameHold` = 4 with `FrameHoldStart` (media ticks) is a hold on that frame.
  `HeadTransition` / `TailTransition` point at the transitions over the clip's start / end.
  Linked clips are `Link` objects listed in `Sequence/PersistentGroupContainer/LinkContainer`.
- Effects: `ClipTrackItem/ComponentOwner/Components` → component chain → components (video:
  `Component/DisplayName`, `MatchName`; audio: `AudioComponent/Component`, `FilterMatchName`) →
  `…ComponentParam` objects with `Name`, `StartKeyframe` (`time,value,…`) and `Keyframes`
  (`time,value,f2,f3,in speed,in influence,out speed,out influence;` repeated).
  **Keyframe times are source-media ticks** (checked on clips whose timeline and source positions
  differ); FilmCraft's are clip time: `(t − InPoint) / speed`. `f2` = 0 linear, 5 Bézier, with `f3`
  = 0 Bézier, 2 auto Bézier, 4 continuous Bézier (observed on ~500 000 keys). Points are `x:y`
  normalised to the frame (Position) or the source (Anchor Point). Motion is `AE.ADBE Motion` (its
  one boolean, Uniform Scale, has no name), Opacity `AE.ADBE Opacity`. Clip volume is the
  `Internal Volume …` component's `Level`, linear gain where `0.177828` (the value on every untouched
  clip) is 0 dB: `dB = 20·log10(level / 0.177828)` (an observed convention). A clip can carry a second
  Motion / Opacity / volume; only the first maps.
- Other effects map onto the FilmCraft effect of the same name (`Gaussian Blur (Legacy)` →
  Gaussian Blur): their numeric, angle, boolean and point parameters by name. A name an effect
  uses for several parameters (Lumetri's "Intensity") is not mapped. Choices, colours and the rest
  are listed in the import report and left as they are.
- Markers: `Clip/MarkerOwner/Markers` (before `Clip/Source`) → `Markers` object (class
  `bee50706-…`) → `Markers/Marker` (`First` = GUID, `Second` → `Marker` object with a
  `DVAMarker` JSON value: `mStartTime.ticks`, `mDuration.ticks`, `mName`, `mComment`, `mType`).
  Markers on a media master clip are source markers; on a sequence's master clip, sequence markers.
- Transitions: `VideoTransitionTrackItem` / `AudioTransitionTrackItem` → `TransitionTrackItem`
  (`TrackItem/Start`, `End`, `DisplayName`, `HasOutgoingClip`, `HasIncomingClip`); joined to the
  clips whose edges fall inside its range on the sides it has.

### Writing back

A project that came from Premiere is never regenerated: FilmCraft patches the one the sequences
came from, by byte range, so everything it did not change keeps its exact bytes (an unchanged project is written back
byte-identical: 455 of the 455 readable corpus projects). Every clip remembers its Premiere track
item (`TrackItem::origin`); media and sequence items their UIDs (`premiere.media`,
`premiere.sequence` metadata). The engine keeps a copy of each opened project so the version that
was opened is the one patched.

- Track clip lists are rewritten in timeline order. A clip from Premiere reuses its track item, so
  every effect and setting FilmCraft does not model survives. A second clip from the same origin
  (split, copy) becomes a copy of that track item and of the objects it owns (sub-clip, clip,
  component chain, components, parameters, channel content), with new ids, `ClipID`s and UUIDs.
  A clip without an origin copies a clip of the same media and gets FilmCraft's settings.
- Media Premiere does not have is added by copying an asset of the same file type (same importer)
  and pointing it at the file: paths (FilmCraft's mapped onto the project's drive letters, learned
  from media both know; a relative path from the new project), title, file key, durations, frame
  size and rate; cached peak/conform files and file history are dropped.
- Timing (`Start`, `End`, `InPoint`, `OutPoint`, `PlaybackSpeed`), frame holds and names are
  written where they changed. Parameters are compared in Premiere's terms (source-time keys,
  normalised points, linear gain) and rewritten only where they differ, so a trim does not touch
  source-timed keys; unchanged keys keep their exact text. Effects removed in FilmCraft are taken
  off; an effect added in FilmCraft copies a component of the same effect elsewhere in the project
(reported when there is none).
- Clips and transitions FilmCraft never imported (sources it does not understand, transitions
  joined to no clip) are kept where they are. A clip the importer moved off its track because it
  overlapped another goes back to its track while its timing is unchanged.
- Transitions follow their clips; new ones copy a transition of the same name in the project, and
  the clips they join get `HeadTransition` / `TailTransition`. Links are rebuilt (links FilmCraft
  cannot express, such as one-clip links, are kept). Markers that match keep their GUIDs; a
  sequence or media without a marker list gets one. Extra tracks copy the last track with the
  mixer objects it owns. Cached renders of edited sequences are dropped. A nested sequence's own
  length follows its new end.
- Before anything is returned the result must parse, have unique ids, resolve every
  `ObjectRef` / `ObjectURef` / `BinaryHash` the original resolved, and import back into exactly
  the clips FilmCraft wrote. Otherwise the export is an error, never a file.

### New projects

A FilmCraft project that did not come from Premiere becomes a new `.prproj` (`prproj_new`):

- A *seed* document is built with the objects a project saved by Premiere has before any clip is
  placed, laid out as observed: the project and its settings objects (`ProjectSettings`, five
  `CompileSettings`, scratch disks, ingest, workspace), the root bin (`Node/ID` 1000000) and bins,
  and per media file a `ClipProjectItem` → `MasterClip` (logging info, a default audio component
  chain, a `VideoClip` and/or `AudioClip`, audio channel groups) → `VideoMediaSource` /
  `AudioMediaSource` → `Media` (`FilePath` / `ActualMediaFilePath` in the Premiere machine's
  terms, `RelativePath` from the project's folder, importer `1fa18bfa-…`, a new `FileKey`; stills
  `Infinite`, `IsStill`, 12-hour length) with its `VideoStream` / `AudioStream`. Mono files keep
  mono clips (`[{"channellabel":0}]`, channel type 0); everything else is stereo.
- Per sequence: its master clip (an `AudioClip` and a `VideoClip` of `Audio`/`VideoSequenceSource`),
  the `Sequence` ("Custom" editing mode `9678af98-…`, its preview preset, work area), an empty
  `LinkContainer`, the video, audio and data track groups, one `VideoClipTrack` per video track and
  one `AudioClipTrack` per audio track (each with a fader + meter chain and a stereo panner at
  0.5), and the master `AudioMixTrack` (fader, meter, default panner, an inlet listing the tracks;
  audio track ids start at 2, the master is 1). Track faders hold the track volume, linear with 1.0
  = 0 dB (upper bound 5.6234 = +15 dB), as observed.
- For each media file and nested sequence a sequence uses, a *template* clip per kind of track:
  a `VideoClipTrackItem` with explicit `AE.ADBE Motion` (11 parameters, with its private data) and
  `AE.ADBE Opacity` components, or an `AudioClipTrackItem` with `Internal Volume Stereo` +
  `Internal Channel Volume Stereo` (Bypass, Left, Right and 30 more levels) or `Internal Volume
  Mono`. (Clips Premiere places itself carry no components and `DefaultMotion` / `DefaultVol`;
  explicit ones are what it saves once a value changes, and what the writer can set.)
- Effects and transitions need Premiere's own layout of their component. They are borrowed from
  Premiere projects the user opened in FilmCraft (the kept copies): one component per effect (with
  its parameters, renumbered, binary payloads inlined) or one transition track item per transition
  name. Only components all of whose settings FilmCraft maps are borrowed (group headings, an
  audio effect's processing state and a Bypass that is off hold no setting), so no value of the
  borrowed clip leaks in; a model without animation is preferred.
- `prproj_write::export` then writes the sequences into the seed exactly as into a project from
  Premiere (copies of the templates and borrowed layouts for every clip, timing, speed, holds,
  Motion / Opacity / volume, effects, transitions, links, markers) and verifies the result; the
  templates and borrowed layouts, which nothing refers to, are dropped last.
- Left out, and listed in the report: effects and transitions with no layout to borrow, graphics /
  titles, adjustment layers, generated media (clips of them), captions, submixes, track effects,
  pan, mute and solo, time remapping, clip pan / channel volume, blend modes, masks, audio gain.
- Media paths: FilmCraft's local paths are written as the Premiere machine sees them, with the
  drive mapping the engine learns from the Premiere projects it has opened (a media path
  `L:\Editing\a.mp4` in one of them that exists here as `/mnt/LocalDisk/Editing/a.mp4` maps
  `/mnt/LocalDisk` to `L:`). Without one, the local path is written; Premiere then finds media kept
  with the project through `RelativePath`, or asks to locate it.

`examples/prproj_recreate.rs` re-creates every project of a corpus this way (as a FilmCraft
project, layouts borrowed from other projects of the corpus) and requires every sequence's clips
back: 85 of the 85 local projects with a sequence.

In the app the project is written by File ▸ Export ▸ Premiere Pro Project, and by File ▸ Save
(Ctrl+S) on a project opened from Premiere that was not saved as a FilmCraft project: Settings ▸
General ▸ Save a project opened from Premiere Pro chooses a `.prproj` beside the original (the
default; the first Save asks where, later ones write there), over the original (keeping
`<file>.before-filmcraft`), or a FilmCraft project (`file.save {path?}`; engine
`interchange::save_as_premiere`). A Save there leaves the project saved (not dirty). Any other
project becomes a new Premiere project through File ▸ Export ▸ Premiere Pro Project or Save As /
Save a Copy with a `.prproj` name; after Save As, Save writes that file again
(`interchange::create_prproj`). A `.prproj` FilmCraft did not write there is kept as
`<file>.before-filmcraft`.

## Tests

- `tests/aaf_omf.rs`: AAF round trips at 25, 29.97 DF, 23.976 and 59.94 DF (clips, gaps,
  two-sided and one-sided transitions, links, markers, start timecodes, constant and keyframed
  gain, media markers and timecode); 512-byte sectors; breakout to mono; embedded trimmed audio
  (sample data and source offsets); separate consolidated files with a video mixdown; OMF round
  trips with embedded audio, separate AIFF files and breakout; empty sequences; truncation and
  random corruption never panic. Nested sequences: AAF round trips as compositions (nests in
  nests, a nest used twice, another frame and sample rate, breakout to mono), OMF with the
  rendered sound, the audio needs of nests, a sequence inside itself.
- `tests/uncarried_clips.rs`: every format's export report names each graphic clip and adjustment
  layer once, as a warning; a sequence without them reports nothing; what a re-import gives back
  matches the report; the FCP7 XML import names a clip item whose file is not defined.
- `src/aaf/tests.rs`: written files checked against the required properties of every class
  written (Header, Identification, Mobs, slots, components, descriptors), weak references
  resolving into the dictionary, source clips resolving to mobs and slots, the Edit Protocol
  transition rules and sequence length arithmetic; every stored form round-trips through the
  container; a file rewritten into Premiere Pro's layout (video tracks as layers of one slot,
  nested compositions tagged top-level) reads back as the same edit.
- `src/omf/tests.rs`: the Bento label and TOC, the required properties of every OMF class
  written, reference resolution, embedded WAVE data, sequence lengths in samples.
