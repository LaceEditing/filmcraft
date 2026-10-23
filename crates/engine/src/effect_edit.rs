//! Effect Controls editing beyond one parameter at a time: several keyframes at once (delete,
//! interpolation; moving them is `effects.moveKeyframes`), copy / paste of keyframes and of effects
//! between clips, removing several effects and reordering them.
//!
//! Keyframes are addressed as `{"effect": index, "param": id, "mask": n?, "mediaTime": ticks}`
//! (the media time the keyframe is stored at). Every edit is one undo step.
//!
//! - **Copy keyframes** keeps their spacing; **paste** puts the first one at the playhead, into the
//!   same parameter of the same effect (on another clip: the first effect of that kind).
//! - **Copy effects** keeps their keyframes relative to the clip's In point; **paste** adds them to
//!   the target clips (Motion, Opacity and the other fixed effects replace the clip's own).
//! - **Move effect** reorders the standard effects; the fixed ones (Motion, Opacity, Time
//!   Remapping, Volume…) and graphic layers keep their places.

use std::collections::BTreeMap;

use filmcraft_project::{ClipId, EffectInstance, Interpolation, Keyframe, Param, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, clip_p, has_seq, str_p, u64_p};
use crate::presets::{EffectPreset, KeyframeMode};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

/// What Copy put on the Effect Controls clipboard.
#[derive(Clone, Debug, PartialEq)]
pub enum FxClipboard {
    /// Effects (keyframe times relative to the source clip's In point).
    Effects(EffectPreset),
    /// Keyframes of parameters (times relative to the earliest copied keyframe).
    Keyframes(Vec<CopiedParam>),
}

/// Keyframes copied from one parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct CopiedParam {
    pub clip: ClipId,
    /// Index of the effect on the clip it was copied from, and its kind.
    pub effect: usize,
    pub effect_id: String,
    pub mask: Option<usize>,
    pub param: String,
    pub keys: Vec<Keyframe>,
}

/// A keyframe reference from command parameters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyRef {
    effect: usize,
    mask: Option<usize>,
    param: String,
    time: Tick,
}

/// At most this many keyframes per command (hostile input).
const MAX_KEYS: usize = 100_000;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: has_seq, run, journal: true }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("effects.deleteKeyframes", "Delete Keyframes", r#"{"clip":id,"keys":[{"effect":index,"param":str,"mask":n?,"mediaTime":ticks}]}"#, delete_keys),
        spec(
            "effects.setKeyframesInterpolation",
            "Keyframe Interpolation",
            r#"{"clip":id,"keys":[{"effect":index,"param":str,"mask":n?,"mediaTime":ticks}],"interpolation":"linear|bezier|autoBezier|continuousBezier|hold|easeIn|easeOut"}"#,
            interp_keys,
        ),
        spec("effects.copyKeyframes", "Copy Keyframes", r#"{"clip":id,"keys":[{"effect":index,"param":str,"mask":n?,"mediaTime":ticks}]}"#, copy_keys),
        spec("effects.pasteKeyframes", "Paste Keyframes", r#"{"clip":id?,"time":ticks? (sequence time; default the playhead)}"#, paste_keys),
        spec("effects.copy", "Copy Effects", r#"{"clip":id,"indices":[n]}"#, copy_effects),
        spec("effects.paste", "Paste Effects", r#"{"clips":[id]?}"#, paste_effects),
        spec("effects.move", "Move Effect", r#"{"clip":id,"index":n,"to":n (index among the clip's effects)}"#, move_effect),
    ]
}

/// Interpolation by name (`linear`, `bezier`, `autoBezier`, `continuousBezier`, `hold`, `easeIn`,
/// `easeOut`; anything else is linear).
pub(crate) fn interpolation(name: &str) -> Interpolation {
    match name.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
        "bezier" => Interpolation::Bezier,
        "autobezier" => Interpolation::AutoBezier,
        "continuousbezier" => Interpolation::ContinuousBezier,
        "hold" => Interpolation::Hold,
        "easein" => Interpolation::EaseIn,
        "easeout" => Interpolation::EaseOut,
        _ => Interpolation::Linear,
    }
}

fn keys_p(p: &Value, cmd: &str) -> Result<Vec<KeyRef>> {
    let a = p.get("keys").and_then(Value::as_array).ok_or_else(|| bad(cmd, "need `keys`"))?;
    let mut v = Vec::new();
    for k in a.iter().take(MAX_KEYS) {
        let effect = k.get("effect").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "each key needs `effect` (an index)"))? as usize;
        let param = k.get("param").and_then(Value::as_str).ok_or_else(|| bad(cmd, "each key needs `param`"))?.to_string();
        let mask = k.get("mask").and_then(Value::as_u64).map(|m| m as usize);
        let time = Tick(k.get("mediaTime").and_then(Value::as_i64).ok_or_else(|| bad(cmd, "each key needs `mediaTime`"))?);
        v.push(KeyRef { effect, mask, param, time });
    }
    v.sort();
    v.dedup();
    if v.is_empty() {
        return Err(bad(cmd, "no keyframes given"));
    }
    Ok(v)
}

/// The keys grouped by parameter: (effect, mask, param) → times.
fn by_param(keys: &[KeyRef]) -> BTreeMap<(usize, Option<usize>, String), Vec<Tick>> {
    let mut m: BTreeMap<(usize, Option<usize>, String), Vec<Tick>> = BTreeMap::new();
    for k in keys {
        m.entry((k.effect, k.mask, k.param.clone())).or_default().push(k.time);
    }
    m
}

fn param_mut<'a>(it: &'a mut TrackItem, effect: usize, mask: Option<usize>, pid: &str) -> Option<&'a mut Param> {
    let e = it.effects.get_mut(effect)?;
    match mask {
        Some(m) => e.masks.get_mut(m)?.param_mut(pid),
        None => e.params.get_mut(pid),
    }
}

fn key_json(effect: usize, mask: Option<usize>, param: &str, time: Tick) -> Value {
    let mut k = json!({"effect": effect, "param": param, "mediaTime": time.0});
    if let Some(m) = mask {
        k["mask"] = json!(m);
    }
    k
}

fn delete_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let c = clip_p(p, "clip").ok_or_else(|| bad("effects.deleteKeyframes", "need `clip`"))?;
    let keys = keys_p(p, "effects.deleteKeyframes")?;
    let n = s.edit_sequence("Delete Keyframes", |q, _, _| {
        let (_, it) = q.find_item_mut(c).ok_or(filmcraft_edit::EditError::NoItem(c))?;
        let mut n = 0;
        for ((effect, mask, pid), times) in by_param(&keys) {
            let Some(prm) = param_mut(it, effect, mask, &pid) else { continue };
            for t in times {
                if prm.remove_keyframe_at(t) {
                    n += 1;
                }
            }
        }
        if n == 0 {
            return Err(bad("effects.deleteKeyframes", "none of those keyframes exist"));
        }
        Ok(n)
    })?;
    Ok(json!({"deleted": n}))
}

fn interp_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let c = clip_p(p, "clip").ok_or_else(|| bad("effects.setKeyframesInterpolation", "need `clip`"))?;
    let keys = keys_p(p, "effects.setKeyframesInterpolation")?;
    let kind = interpolation(str_p(p, "interpolation").unwrap_or("linear"));
    s.edit_sequence("Keyframe Interpolation", |q, _, _| {
        let (_, it) = q.find_item_mut(c).ok_or(filmcraft_edit::EditError::NoItem(c))?;
        for ((effect, mask, pid), times) in by_param(&keys) {
            let Some(prm) = param_mut(it, effect, mask, &pid) else { continue };
            for k in prm.keyframes.iter_mut().filter(|k| times.contains(&k.time)) {
                k.interp = kind;
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn find_clip(s: &Session, c: ClipId, cmd: &str) -> Result<(TrackKind, TrackItem)> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (tid, it) = q.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    let kind = q.track(tid).map(|t| t.kind).unwrap_or(TrackKind::Video);
    Ok((kind, it.clone()))
}

fn copy_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let c = clip_p(p, "clip").ok_or_else(|| bad("effects.copyKeyframes", "need `clip`"))?;
    let keys = keys_p(p, "effects.copyKeyframes")?;
    let (_, it) = find_clip(s, c, "effects.copyKeyframes")?;
    let mut out: Vec<CopiedParam> = Vec::new();
    for ((effect, mask, pid), times) in by_param(&keys) {
        let Some(e) = it.effects.get(effect) else { continue };
        let prm = match mask {
            Some(m) => e.masks.get(m).and_then(|x| x.param(&pid)),
            None => e.params.get(&pid),
        };
        let Some(prm) = prm else { continue };
        let ks: Vec<Keyframe> = prm.keyframes.iter().filter(|k| times.contains(&k.time)).cloned().collect();
        if !ks.is_empty() {
            out.push(CopiedParam { clip: c, effect, effect_id: e.effect.clone(), mask, param: pid, keys: ks });
        }
    }
    let Some(first) = out.iter().flat_map(|cp| cp.keys.iter().map(|k| k.time)).min() else {
        return Err(bad("effects.copyKeyframes", "none of those keyframes exist"));
    };
    for cp in &mut out {
        for k in &mut cp.keys {
            k.time -= first;
        }
    }
    let n: usize = out.iter().map(|cp| cp.keys.len()).sum();
    s.state.fx_clipboard = Some(FxClipboard::Keyframes(out));
    Ok(json!({"copied": n}))
}

/// The clip Effect Controls shows: the first selected video clip, else the first selected clip.
fn shown_clip(s: &Session) -> Option<ClipId> {
    let q = s.active_sequence()?;
    let sel = &s.state.selection;
    sel.iter().copied().find(|c| q.find_item(*c).and_then(|(t, _)| q.track(t)).is_some_and(|t| t.kind == TrackKind::Video)).or_else(|| sel.first().copied())
}

fn paste_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(FxClipboard::Keyframes(copied)) = s.state.fx_clipboard.clone() else {
        return Err(bad("effects.pasteKeyframes", "no keyframes have been copied"));
    };
    let c = clip_p(p, "clip").or_else(|| shown_clip(s)).ok_or_else(|| bad("effects.pasteKeyframes", "select a clip"))?;
    let at = crate::commands::time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let pasted = s.edit_sequence("Paste Keyframes", |q, _, _| {
        let (_, it) = q.find_item_mut(c).ok_or(filmcraft_edit::EditError::NoItem(c))?;
        let mt = it.source_time_at(at.clamp(it.start, it.end() - Tick(1)));
        let mut pasted = Vec::new();
        for cp in &copied {
            // the same effect on the same clip, else the first effect of that kind
            let idx = if cp.clip == c && it.effects.get(cp.effect).is_some_and(|e| e.effect == cp.effect_id) {
                Some(cp.effect)
            } else {
                it.effects.iter().position(|e| e.effect == cp.effect_id)
            };
            let Some(idx) = idx else { continue };
            let Some(prm) = param_mut(it, idx, cp.mask, &cp.param) else { continue };
            for k in &cp.keys {
                if std::mem::discriminant(&k.value) != std::mem::discriminant(&prm.value) {
                    continue;
                }
                let mut nk = k.clone();
                nk.time = Tick(mt.0.saturating_add(k.time.0));
                if prm.keyframes.is_empty() {
                    prm.value = nk.value.clone();
                }
                prm.keyframes.retain(|x| x.time != nk.time);
                let at = prm.keyframes.partition_point(|x| x.time < nk.time);
                pasted.push(key_json(idx, cp.mask, &cp.param, nk.time));
                prm.keyframes.insert(at, nk);
            }
        }
        if pasted.is_empty() {
            return Err(bad("effects.pasteKeyframes", "the clip has no parameter those keyframes fit"));
        }
        Ok(pasted)
    })?;
    Ok(json!({"keys": pasted}))
}

fn copy_effects(s: &mut Session, p: &Value) -> Result<Value> {
    let c = clip_p(p, "clip").ok_or_else(|| bad("effects.copy", "need `clip`"))?;
    let (_, it) = find_clip(s, c, "effects.copy")?;
    let idx: Vec<usize> = p
        .get("indices")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect::<Vec<usize>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|i| it.effects.get(*i).is_some_and(|e| !filmcraft_project::graphic::is_layer(e)))
        .collect();
    if idx.is_empty() {
        return Err(bad("effects.copy", "no effects to copy (graphic layers are not copied as effects)"));
    }
    let mt = it.source_time_at(s.playhead().clamp(it.start, it.end() - Tick(1)));
    let size = filmcraft_render::source_size(&s.project, it.item)
        .or_else(|| s.active_sequence().map(|q| (q.settings.width, q.settings.height)))
        .unwrap_or((1920, 1080));
    let preset = crate::presets::capture(&it, &idx, "Clipboard", "", Some(KeyframeMode::AnchorToIn), mt, size);
    let names: Vec<String> = preset.effects.iter().map(|e| e.def().map(|d| d.name).unwrap_or(e.effect.as_str()).to_string()).collect();
    s.state.fx_clipboard = Some(FxClipboard::Effects(preset));
    Ok(json!({"copied": names}))
}

fn paste_effects(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(FxClipboard::Effects(preset)) = s.state.fx_clipboard.clone() else {
        return Err(bad("effects.paste", "no effects have been copied"));
    };
    let clips: Vec<ClipId> = match p.get("clips").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ClipId)).collect(),
        None => s.state.selection.clone(),
    };
    if clips.is_empty() {
        return Err(bad("effects.paste", "select clips first"));
    }
    let audio = preset.effects.iter().all(|e| e.def().is_some_and(|d| d.kind == filmcraft_project::EffectKind::Audio));
    let sizes: Vec<(ClipId, (u32, u32))> = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let fallback = (q.settings.width, q.settings.height);
        clips.iter().filter_map(|c| q.find_item(*c).map(|(_, it)| (*c, filmcraft_render::source_size(&s.project, it.item).unwrap_or(fallback)))).collect()
    };
    let n = s.edit_sequence("Paste Effects", |q, _, _| {
        let mut n = 0;
        for t in q.all_tracks_mut() {
            if (t.kind == TrackKind::Audio) != audio {
                continue;
            }
            for it in t.items.iter_mut().filter(|i| clips.contains(&i.id)) {
                let size = sizes.iter().find(|(c, _)| *c == it.id).map(|x| x.1).unwrap_or((1920, 1080));
                let fx = crate::presets::instantiate(&preset, it, size);
                crate::presets::put_effects(it, fx);
                n += 1;
            }
        }
        if n == 0 {
            return Err(bad("effects.paste", "the copied effects don't fit the selected clips (video/audio)"));
        }
        Ok(n)
    })?;
    Ok(json!({"pasted": n}))
}

/// Whether an effect can be reordered (a standard effect, not a fixed one or a graphic layer).
pub fn movable(e: &EffectInstance) -> bool {
    e.def().is_some_and(|d| !d.intrinsic) && !filmcraft_project::graphic::is_layer(e)
}

fn move_effect(s: &mut Session, p: &Value) -> Result<Value> {
    let c = clip_p(p, "clip").ok_or_else(|| bad("effects.move", "need `clip`"))?;
    let from = u64_p(p, "index").ok_or_else(|| bad("effects.move", "need `index`"))? as usize;
    let to = u64_p(p, "to").ok_or_else(|| bad("effects.move", "need `to`"))? as usize;
    s.edit_sequence("Move Effect", |q, _, _| {
        let (_, it) = q.find_item_mut(c).ok_or(filmcraft_edit::EditError::NoItem(c))?;
        let (Some(a), Some(b)) = (it.effects.get(from), it.effects.get(to)) else {
            return Err(bad("effects.move", "no effect at that index"));
        };
        if !movable(a) || !movable(b) {
            return Err(bad("effects.move", "fixed effects and graphic layers keep their places"));
        }
        // only across standard effects: every effect between the two must be movable too
        if !it.effects.get(from.min(to)..=from.max(to)).is_some_and(|span| span.iter().all(movable)) {
            return Err(bad("effects.move", "an effect can only move among the standard effects"));
        }
        let e = it.effects.remove(from);
        it.effects.insert(to, e);
        Ok(())
    })?;
    Ok(json!({"index": to}))
}
