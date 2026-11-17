//! Edit / Clip / File menu commands for project items and clips (M3.10): labels and Select Label
//! Group, Paste / Remove Attributes, Select All Matching, Remove Unused, Consolidate Duplicates,
//! Sequence From Clip, Bin From Selection, Offline File, Close Project, Save All, Make / Edit
//! Subclip, Modify ▸ Audio Channels / Timecode, Video Options (Frame Hold Options, Add Frame Hold,
//! Insert Frame Hold Segment, Field Options, Time Interpolation, Fit / Fill frame), Audio Options
//! (Breakout to Mono, Extract Audio) and Replace With Clip.
//!
//! Every command mutates through `Session::edit` / `edit_sequence`, so it is one undo step.
//! [`apply_layout`] places these commands (and a few existing ones) in Premiere's menu order.

use std::collections::{BTreeMap, BTreeSet};

use filmcraft_edit as edit;
use filmcraft_project::{
    AudioChannelMap, AudioChannels, BinEntry, ClipId, FieldOptions, FieldProcessing, ItemId, ItemKind, Label, MediaClip, MediaRef, ParamValue, Project,
    TimeInterpolation, TrackItem, TrackKind,
};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange, parse_timecode};
use serde_json::{Value, json};

use crate::commands::{
    CommandSpec, always, bad, bool_p, clips_p, f64_p, has_project_selection, has_selection, has_seq, item_p, place_item, source_range, str_p, time_p, u64_p,
    with_links,
};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

fn spec(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
    enabled: Enabled,
    run: Run,
) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut, params, enabled, run, journal: true, named: crate::commands::NamedKeys::NONE }
}

macro_rules! label_cmd {
    ($id:literal, $name:literal, $l:expr) => {
        CommandSpec {
            id: $id,
            label: $name,
            menu: &["Edit", "Label"],
            shortcut: None,
            params: r#"{"items":[id]?,"clips":[id]?}"#,
            enabled: can_label,
            run: |s, p| apply_label(s, p, $l),
            journal: true,
            named: crate::commands::NamedKeys::NONE,
        }
    };
}

mod audio_options;
mod common;
mod edit_menu;
mod enabled;
mod file;
mod replace;
mod subclip;
mod video_options;

use audio_options::{breakout_to_mono, extract_audio};
pub use common::{SourceView, source_view};
pub(crate) use common::{collapse_history, collect_ids};
pub use edit_menu::is_attribute_effect;
use edit_menu::{apply_label, consolidate_duplicates, paste_attributes, remove_attributes, remove_unused, select_all_matching, select_label_group};
use enabled::{
    can_label, can_make_subclip, can_paste_attributes, can_replace_from_bin, can_replace_from_source, has_audio_items, has_audio_target, has_clip_items,
    has_extractable, has_media_selection, has_project_items, has_subclip,
};
pub(crate) use file::item_range;
use file::{bin_from_selection, close_project, offline_file, sequence_from_clip};
use replace::{Replace, replace_with};
pub use subclip::subclip_defaults;
use subclip::{audio_channels, edit_subclip, make_subclip, modify_timecode};
pub(crate) use video_options::add_frame_hold;
use video_options::{field_options, fit_fill, frame_hold_options, insert_frame_hold_segment, set_interpolation};

/// The new commands.
pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        // ---------------- Timeline: a clip's own fades (the handles at its top corners) ----------------
        spec(
            "clip.setFade",
            "Clip Fade",
            &[],
            None,
            r#"{"clip":id,"edge":"in"|"out","duration":ticks?,"frames":i64?,"shape":-100..100?} (duration 0 removes the fade)"#,
            has_seq,
            set_fade,
        ),
        // ---------------- File ----------------
        spec("file.newSequenceFromClip", "Sequence From Clip", &["File", "New"], None, r#"{"items":[id]?}"#, has_clip_items, sequence_from_clip),
        spec(
            "file.newBinFromSelection",
            "Bin From Selection",
            &["File", "New"],
            Some("Shift+B"),
            r#"{"items":[id]?,"name":str?}"#,
            has_project_selection,
            bin_from_selection,
        ),
        spec(
            "file.newOfflineFile",
            "Offline File…",
            &["File", "New"],
            None,
            r#"{"name":str?,"fileName":str?,"tapeName":str?,"video":bool=true,"audio":bool=true,"width":u32?,"height":u32?,"fps":f64?,"sampleRate":u32=48000,"channels":u32=2,"timecode":"HH:MM:SS:FF"?,"seconds":f64=10,"description":str?}"#,
            always,
            offline_file,
        ),
        spec("file.closeProject", "Close Project", &["File"], Some("Cmd+Shift+W"), r#"{"force":bool?}"#, always, close_project),
        spec("file.saveAll", "Save All", &["File"], None, "{}", always, |s, _| s.execute("file.save", json!({}))),
        // ---------------- Edit ----------------
        spec(
            "edit.pasteAttributes",
            "Paste Attributes…",
            &["Edit"],
            Some("Cmd+Alt+V"),
            r#"{"clips":[id]?,"motion":bool=true,"opacity":bool=true,"timeRemapping":bool=true,"volume":bool=true,"channelVolume":bool=true,"panner":bool=true,"effects":bool|[effectId]=true,"scaleTimes":bool=true}"#,
            can_paste_attributes,
            paste_attributes,
        ),
        spec(
            "edit.removeAttributes",
            "Remove Attributes…",
            &["Edit"],
            None,
            r#"{"clips":[id]?,"motion":bool=true,"opacity":bool=true,"timeRemapping":bool=true,"volume":bool=true,"channelVolume":bool=true,"panner":bool=true,"effects":bool|[effectId]=true}"#,
            has_selection,
            remove_attributes,
        ),
        spec("edit.selectAllMatching", "Select All Matching", &["Edit"], None, r#"{"clips":[id]?}"#, has_selection, select_all_matching),
        spec("edit.selectLabelGroup", "Select Label Group", &["Edit", "Label"], None, "{}", can_label, select_label_group),
        label_cmd!("edit.label.violet", "Violet", Label::Violet),
        label_cmd!("edit.label.iris", "Iris", Label::Iris),
        label_cmd!("edit.label.caribbean", "Caribbean", Label::Caribbean),
        label_cmd!("edit.label.lavender", "Lavender", Label::Lavender),
        label_cmd!("edit.label.cerulean", "Cerulean", Label::Cerulean),
        label_cmd!("edit.label.forest", "Forest", Label::Forest),
        label_cmd!("edit.label.rose", "Rose", Label::Rose),
        label_cmd!("edit.label.mango", "Mango", Label::Mango),
        label_cmd!("edit.label.purple", "Purple", Label::Purple),
        label_cmd!("edit.label.blue", "Blue", Label::Blue),
        label_cmd!("edit.label.teal", "Teal", Label::Teal),
        label_cmd!("edit.label.magenta", "Magenta", Label::Magenta),
        label_cmd!("edit.label.tan", "Tan", Label::Tan),
        label_cmd!("edit.label.green", "Green", Label::Green),
        label_cmd!("edit.label.brown", "Brown", Label::Brown),
        label_cmd!("edit.label.yellow", "Yellow", Label::Yellow),
        spec("edit.removeUnused", "Remove Unused", &["Edit"], None, "{}", has_project_items, remove_unused),
        spec("edit.consolidateDuplicates", "Consolidate Duplicates", &["Edit"], None, "{}", has_project_items, consolidate_duplicates),
        // ---------------- Clip ----------------
        spec(
            "clip.makeSubclip",
            "Make Subclip…",
            &["Clip"],
            Some("Cmd+U"),
            r#"{"item":id?,"clip":id?,"name":str?,"start":ticks?,"end":ticks?,"startFrame":i64?,"endFrame":i64?,"restrictTrims":bool=true}"#,
            can_make_subclip,
            make_subclip,
        ),
        spec(
            "clip.editSubclip",
            "Edit Subclip…",
            &["Clip"],
            None,
            r#"{"item":id?,"start":ticks?,"end":ticks?,"startFrame":i64?,"endFrame":i64?,"restrictTrims":bool?,"convertToMaster":bool?}"#,
            has_subclip,
            edit_subclip,
        ),
        spec(
            "clip.audioChannels",
            "Audio Channels…",
            &["Clip", "Modify"],
            Some("Shift+G"),
            r#"{"items":[id]?,"format":"mono|stereo|5.1|adaptive","clips":[[channel]]?,"channels":[channel]?}"#,
            has_audio_target,
            audio_channels,
        ),
        spec(
            "clip.modifyTimecode",
            "Timecode…",
            &["Clip", "Modify"],
            None,
            r#"{"item":id?,"timecode":"HH:MM:SS:FF"?,"frame":i64?,"tapeName":str?,"reset":bool?}"#,
            has_media_selection,
            modify_timecode,
        ),
        spec(
            "clip.frameHoldOptions",
            "Frame Hold Options…",
            &["Clip", "Video Options"],
            None,
            r#"{"clips":[id]?,"enabled":bool=true,"holdOn":"sourceTimecode|sequenceTime|in|out|playhead"?,"time":ticks?,"timecode":str?,"holdFilters":bool=false}"#,
            has_selection,
            frame_hold_options,
        ),
        spec(
            "clip.insertFrameHoldSegment",
            "Insert Frame Hold Segment",
            &["Clip", "Video Options"],
            None,
            r#"{"clip":id?,"time":ticks?,"seconds":f64=2}"#,
            has_seq,
            insert_frame_hold_segment,
        ),
        spec(
            "clip.fieldOptions",
            "Field Options…",
            &["Clip", "Video Options"],
            None,
            r#"{"clips":[id]?,"reverseFieldDominance":bool=false,"processing":"none|alwaysDeinterlace|flickerRemoval"?}"#,
            has_selection,
            field_options,
        ),
        spec(
            "clip.timeInterpolation.frameSampling",
            "Frame Sampling",
            &["Clip", "Video Options", "Time Interpolation"],
            None,
            r#"{"clips":[id]?}"#,
            has_selection,
            |s, p| set_interpolation(s, p, TimeInterpolation::FrameSampling),
        ),
        spec(
            "clip.timeInterpolation.frameBlending",
            "Frame Blending",
            &["Clip", "Video Options", "Time Interpolation"],
            None,
            r#"{"clips":[id]?}"#,
            has_selection,
            |s, p| set_interpolation(s, p, TimeInterpolation::FrameBlending),
        ),
        spec(
            "clip.timeInterpolation.opticalFlow",
            "Optical Flow",
            &["Clip", "Video Options", "Time Interpolation"],
            None,
            r#"{"clips":[id]?}"#,
            has_selection,
            |s, p| set_interpolation(s, p, TimeInterpolation::OpticalFlow),
        ),
        spec(
            "clip.setTimeInterpolation",
            "Set Time Interpolation",
            &[],
            None,
            r#"{"clips":[id]?,"mode":"frameSampling|frameBlending|opticalFlow"}"#,
            has_selection,
            |s, p| {
                let m = str_p(p, "mode").and_then(TimeInterpolation::from_name).ok_or_else(|| bad("clip.setTimeInterpolation", "need `mode`"))?;
                set_interpolation(s, p, m)
            },
        ),
        spec("clip.fitToFrame", "Fit to frame", &["Clip", "Video Options"], None, r#"{"clips":[id]?}"#, has_selection, |s, p| fit_fill(s, p, false)),
        spec("clip.fillFrame", "Fill frame", &["Clip", "Video Options"], None, r#"{"clips":[id]?}"#, has_selection, |s, p| fit_fill(s, p, true)),
        spec("clip.breakoutToMono", "Breakout to Mono", &["Clip", "Audio Options"], None, r#"{"items":[id]?}"#, has_audio_items, breakout_to_mono),
        spec("clip.extractAudio", "Extract Audio", &["Clip", "Audio Options"], None, r#"{"items":[id]?,"dir":str?}"#, has_extractable, extract_audio),
        spec("clip.replaceFromSource", "From Source Monitor", &["Clip", "Replace With Clip"], None, r#"{"clips":[id]?}"#, can_replace_from_source, |s, p| {
            replace_with(s, p, Replace::Source)
        }),
        spec(
            "clip.replaceFromSourceMatchFrame",
            "From Source Monitor, Match Frame",
            &["Clip", "Replace With Clip"],
            None,
            r#"{"clips":[id]?}"#,
            can_replace_from_source,
            |s, p| replace_with(s, p, Replace::MatchFrame),
        ),
        spec(
            "clip.replaceFromBin",
            "From Bin",
            &["Clip", "Replace With Clip"],
            None,
            r#"{"clips":[id]?,"item":id?,"keepSourceIn":bool=false}"#,
            can_replace_from_bin,
            |s, p| replace_with(s, p, Replace::Bin),
        ),
    ]
}

/// Where a run of commands goes in the menus (the registry order is the menu order).
enum At {
    After(&'static str),
    Before(&'static str),
}

const LABEL_IDS: [&str; 16] = [
    "edit.label.violet",
    "edit.label.iris",
    "edit.label.caribbean",
    "edit.label.lavender",
    "edit.label.cerulean",
    "edit.label.forest",
    "edit.label.rose",
    "edit.label.mango",
    "edit.label.purple",
    "edit.label.blue",
    "edit.label.teal",
    "edit.label.magenta",
    "edit.label.tan",
    "edit.label.green",
    "edit.label.brown",
    "edit.label.yellow",
];

/// Add [`commands`] to the registry and arrange the File / Edit / Clip menus in Premiere's order
/// (moving a few existing commands into their submenus' order too).
pub(crate) fn apply_layout(v: &mut Vec<CommandSpec>) {
    let mut pool = commands();
    let mut edit_tail: Vec<&'static str> = vec!["edit.selectLabelGroup"];
    edit_tail.extend(LABEL_IDS);
    edit_tail.extend(["edit.removeUnused", "edit.consolidateDuplicates"]);
    let layout: Vec<(At, Vec<&'static str>)> = vec![
        (At::After("file.newSequence"), vec!["file.newSequenceFromClip"]),
        (At::After("file.newBin"), vec!["file.newBinFromSelection", "file.newOfflineFile"]),
        (At::Before("file.save"), vec!["file.closeProject"]),
        (At::After("file.saveCopy"), vec!["file.saveAll"]),
        (At::After("edit.pasteInsert"), vec!["edit.pasteAttributes", "edit.removeAttributes"]),
        (At::After("edit.selectAll"), vec!["edit.selectAllMatching"]),
        (At::After("edit.duplicate"), edit_tail),
        (
            At::After("clip.rename"),
            vec![
                "clip.makeSubclip",
                "clip.editSubclip",
                "clip.audioChannels",
                "clip.interpretFootage",
                "clip.modifyTimecode",
                "clip.frameHoldOptions",
                "clip.frameHold",
                "clip.insertFrameHoldSegment",
                "clip.fieldOptions",
                "clip.timeInterpolation.frameSampling",
                "clip.timeInterpolation.frameBlending",
                "clip.timeInterpolation.opticalFlow",
                "clip.scaleToFrameSize",
                "clip.fitToFrame",
                "clip.fillFrame",
                "clip.audioGain",
                "clip.breakoutToMono",
                "clip.extractAudio",
            ],
        ),
        (At::After("source.overwrite"), vec!["clip.replaceFromSource", "clip.replaceFromSourceMatchFrame", "clip.replaceFromBin"]),
    ];
    for (at, ids) in layout {
        let mut run = Vec::new();
        for id in ids {
            if let Some(i) = v.iter().position(|c| c.id == id) {
                run.push(v.remove(i));
            } else if let Some(i) = pool.iter().position(|c| c.id == id) {
                run.push(pool.remove(i));
            }
        }
        let idx = match at {
            At::After(a) => v.iter().position(|c| c.id == a).map(|i| i + 1),
            At::Before(b) => v.iter().position(|c| c.id == b),
        }
        .unwrap_or(v.len());
        v.splice(idx..idx, run);
    }
    v.extend(pool);
}

/// A clip's fade in or out (Premiere's fade handles): a transition on that one clip's edge — a
/// Custom Fade on audio, a Cross Dissolve to nothing on video — made, resized, or (duration 0)
/// removed. The two fades of a clip together fit in it; an edge already joined to the next clip by
/// a transition has no fade of its own. An audio fade's `shape` (Fade Value) bends its curve; as
/// in Premiere, bending another fade (Constant Power…) makes it a Custom Fade of its curve.
pub(crate) fn set_fade(s: &mut Session, p: &Value) -> Result<Value> {
    let clip = crate::commands::clip_p(p, "clip").ok_or_else(|| bad("clip.setFade", "need `clip`"))?;
    let fade_in = match str_p(p, "edge") {
        Some("in") => true,
        Some("out") => false,
        _ => return Err(bad("clip.setFade", "`edge` is `in` or `out`")),
    };
    let rate = s.sequence_rate();
    let want = match (p.get("duration").and_then(Value::as_i64), p.get("frames").and_then(Value::as_i64)) {
        (Some(d), _) => Tick(d),
        (None, Some(f)) => rate.tick_of(f.clamp(0, 1 << 32)),
        _ => return Err(bad("clip.setFade", "need `duration` (ticks) or `frames`")),
    };
    let shape = f64_p(p, "shape").filter(|v| v.is_finite()).map(|v| v.clamp(-100.0, 100.0));
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (track, it) = seq.find_item(clip).ok_or_else(|| bad("clip.setFade", format!("no clip {}", clip.0)))?;
    let tr = seq.track(track).ok_or_else(|| bad("clip.setFade", "no track"))?;
    let audio = tr.kind == TrackKind::Audio;
    let mine = |t: &filmcraft_project::Transition| if fade_in { t.to == Some(clip) && t.from.is_none() } else { t.from == Some(clip) && t.to.is_none() };
    if tr.transitions.iter().any(|t| if fade_in { t.to == Some(clip) && t.from.is_some() } else { t.from == Some(clip) && t.to.is_some() }) {
        return Err(bad("clip.setFade", "that edge already has a transition with the neighbouring clip"));
    }
    // room left by the other fade
    let other = tr.transitions.iter().find(|t| if fade_in { t.from == Some(clip) && t.to.is_none() } else { t.to == Some(clip) && t.from.is_none() });
    let room = it.duration - other.map_or(Tick::ZERO, |t| t.duration);
    let dur = rate.snap(want.clamp(Tick::ZERO, room.max(Tick::ZERO))).min(room.max(Tick::ZERO));
    let existing = tr.transitions.iter().find(|t| mine(t)).map(|t| t.id);
    let (start, end) = (it.start, it.end());
    if existing.is_none() && dur <= Tick::ZERO {
        return Ok(json!({"clip": clip.0, "fade": Value::Null}));
    }
    let label = if fade_in { "Fade In" } else { "Fade Out" };
    let id = s.edit_sequence(label, move |q, ctx, _| {
        let t = q.track_mut(track).ok_or_else(|| EngineError::Other("no track".into()))?;
        if t.locked {
            return Err(EngineError::Other("the track is locked".into()));
        }
        let at = if fade_in { start } else { end - dur };
        match existing.and_then(|id| t.transitions.iter().position(|x| x.id == id)) {
            Some(i) if dur <= Tick::ZERO => {
                t.transitions.remove(i);
                Ok(None)
            }
            Some(i) => {
                let x = &mut t.transitions[i];
                (x.start, x.duration) = (at, dur);
                if let (Some(v), true) = (shape, audio) {
                    if x.effect.effect != filmcraft_project::effect::CUSTOM_FADE {
                        let linear = matches!(filmcraft_render::transitions::FadeCurve::of(&x.effect), filmcraft_render::transitions::FadeCurve::Linear(_));
                        let curve = if linear { filmcraft_project::effect::FADE_LINEAR } else { filmcraft_project::effect::FADE_COSINE };
                        let def = filmcraft_project::find_effect(filmcraft_project::effect::CUSTOM_FADE)
                            .ok_or_else(|| EngineError::Other("no custom_fade effect".into()))?;
                        x.effect = def.instance();
                        x.effect.params.insert("curve".into(), filmcraft_project::Param::new(ParamValue::Choice(curve)));
                    }
                    x.effect.params.insert("shape".into(), filmcraft_project::Param::new(ParamValue::Float(v)));
                }
                Ok(Some(x.id))
            }
            None => {
                let def = if audio { filmcraft_project::effect::CUSTOM_FADE } else { "cross_dissolve" };
                let mut effect = filmcraft_project::find_effect(def).map(|d| d.instance()).ok_or_else(|| EngineError::Other(format!("no {def} effect")))?;
                if let (Some(v), true) = (shape, audio) {
                    effect.params.insert("shape".into(), filmcraft_project::Param::new(ParamValue::Float(v)));
                }
                let (from, to) = if fade_in { (None, Some(clip)) } else { (Some(clip), None) };
                let align = if fade_in { filmcraft_project::TransitionAlign::StartAtCut } else { filmcraft_project::TransitionAlign::EndAtCut };
                let x =
                    filmcraft_project::Transition { id: filmcraft_project::TransitionId(0), effect, start: at, duration: dur, from, to, align, reverse: false };
                Ok(Some(edit::add_transition(q, track, x, ctx)?))
            }
        }
    })?;
    Ok(json!({"clip": clip.0, "fade": id.map(|i| i.0), "duration": dur.0}))
}
