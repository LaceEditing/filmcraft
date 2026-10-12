//! Premiere-style docking: a tree of splits and tab groups. Panels are rounded frames separated by
//! thin gutters; each group has a tab strip (active tab bright, with a panel menu "≡"); the focused
//! panel gets a blue outline. Gutters drag to resize; workspaces are serialized trees.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::icons::{self, Icon};
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Project,
    MediaBrowser,
    Libraries,
    Info,
    Effects,
    Markers,
    History,
    Source,
    EffectControls,
    AudioClipMixer,
    Metadata,
    Program,
    Timeline,
    Tools,
    AudioMeters,
    AudioTrackMixer,
    LumetriColor,
    LumetriScopes,
    EssentialGraphics,
    EssentialSound,
    Properties,
    Text,
    Events,
    Progress,
    ReferenceMonitor,
    Timecode,
}

impl PanelKind {
    pub const ALL: [PanelKind; 26] = [
        PanelKind::Project,
        PanelKind::MediaBrowser,
        PanelKind::Libraries,
        PanelKind::Info,
        PanelKind::Effects,
        PanelKind::Markers,
        PanelKind::History,
        PanelKind::Source,
        PanelKind::EffectControls,
        PanelKind::AudioClipMixer,
        PanelKind::Metadata,
        PanelKind::Program,
        PanelKind::Timeline,
        PanelKind::Tools,
        PanelKind::AudioMeters,
        PanelKind::AudioTrackMixer,
        PanelKind::LumetriColor,
        PanelKind::LumetriScopes,
        PanelKind::EssentialGraphics,
        PanelKind::EssentialSound,
        PanelKind::Properties,
        PanelKind::Text,
        PanelKind::Events,
        PanelKind::Progress,
        PanelKind::ReferenceMonitor,
        PanelKind::Timecode,
    ];
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::Project => "Project",
            PanelKind::MediaBrowser => "Media Browser",
            PanelKind::Libraries => "Libraries",
            PanelKind::Info => "Info",
            PanelKind::Effects => "Effects",
            PanelKind::Markers => "Markers",
            PanelKind::History => "History",
            PanelKind::Source => "Source",
            PanelKind::EffectControls => "Effect Controls",
            PanelKind::AudioClipMixer => "Audio Clip Mixer",
            PanelKind::Metadata => "Metadata",
            PanelKind::Program => "Program",
            PanelKind::Timeline => "Timeline",
            PanelKind::Tools => "Tools",
            PanelKind::AudioMeters => "Audio Meters",
            PanelKind::AudioTrackMixer => "Audio Track Mixer",
            PanelKind::LumetriColor => "Lumetri Color",
            PanelKind::LumetriScopes => "Lumetri Scopes",
            PanelKind::EssentialGraphics => "Essential Graphics",
            PanelKind::EssentialSound => "Essential Sound",
            PanelKind::Properties => "Properties",
            PanelKind::Text => "Text",
            PanelKind::Events => "Events",
            PanelKind::Progress => "Progress",
            PanelKind::ReferenceMonitor => "Reference Monitor",
            PanelKind::Timecode => "Timecode",
        }
    }
    pub fn id(self) -> String {
        format!("{self:?}")
    }
    pub fn from_name(s: &str) -> Option<PanelKind> {
        let n = s.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Self::ALL.iter().copied().find(|p| format!("{p:?}").to_ascii_lowercase() == n || p.title().to_ascii_lowercase().replace(' ', "") == n)
    }
    /// Window-menu shortcut (Premiere: Shift+1..9).
    pub fn window_shortcut(self) -> Option<&'static str> {
        match self {
            PanelKind::Project => Some("Shift+1"),
            PanelKind::Source => Some("Shift+2"),
            PanelKind::Timeline => Some("Shift+3"),
            PanelKind::Program => Some("Shift+4"),
            PanelKind::EffectControls => Some("Shift+5"),
            PanelKind::AudioClipMixer => Some("Shift+9"),
            PanelKind::Effects => Some("Shift+7"),
            PanelKind::MediaBrowser => Some("Shift+8"),
            PanelKind::AudioTrackMixer => Some("Shift+6"),
            _ => None,
        }
    }
    /// Narrow chrome-less panels (no tab strip; a slim grip instead).
    pub fn compact(self) -> bool {
        matches!(self, PanelKind::Tools | PanelKind::AudioMeters)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SplitSize {
    Ratio(f32),
    /// First child has a fixed size in points.
    FixedA(f32),
    /// Second child has a fixed size in points.
    FixedB(f32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    /// `vertical`: children stacked top/bottom; else side by side.
    Split {
        vertical: bool,
        size: SplitSize,
        a: Box<DockNode>,
        b: Box<DockNode>,
    },
    Tabs {
        panels: Vec<PanelKind>,
        active: usize,
    },
}

fn tabs(p: &[PanelKind], active: usize) -> DockNode {
    DockNode::Tabs { panels: p.to_vec(), active }
}
fn hsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: false, size, a: Box::new(a), b: Box::new(b) }
}
fn vsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: true, size, a: Box::new(a), b: Box::new(b) }
}

pub const WORKSPACES: [&str; 9] = ["Editing", "Assembly", "Color", "Effects", "Audio", "Captions and Graphics", "Learning", "Review", "All Panels"];

/// The user's workspaces file, beside `preferences.json` in the data directory.
pub const WORKSPACES_FILE: &str = "workspaces.json";

/// Window ▸ Workspaces: the user's own workspaces and saved changes to the built-in ones, and the
/// workspace in use (reopened at launch).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePrefs {
    pub saved: Vec<SavedWorkspace>,
    pub current: String,
    /// Each workspace as last arranged (panels moved, closed, undocked, resized), kept until
    /// Reset to Saved Layout or Save Changes: what a workspace reopens with, at launch too.
    pub live: Vec<SavedWorkspace>,
}

/// A named layout. `layout` is a [`DockNode`] kept as JSON, so one unreadable entry (a panel a
/// newer version added) doesn't lose the others.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedWorkspace {
    pub name: String,
    pub layout: serde_json::Value,
    /// Undocked panel groups ([`FloatingGroup`]s), JSON for the same reason.
    pub floating: serde_json::Value,
}

impl SavedWorkspace {
    pub fn new(name: &str, dock: &DockNode, floating: &[FloatingGroup]) -> Self {
        SavedWorkspace {
            name: name.to_string(),
            layout: serde_json::to_value(dock).unwrap_or_default(),
            floating: serde_json::to_value(floating).unwrap_or_default(),
        }
    }
    /// The dock tree (None when it can't be read).
    pub fn dock(&self) -> Option<DockNode> {
        serde_json::from_value(self.layout.clone()).ok()
    }
    /// The undocked groups (none when there are none or they can't be read).
    pub fn floating(&self) -> Vec<FloatingGroup> {
        serde_json::from_value(self.floating.clone()).unwrap_or_default()
    }
}

impl WorkspacePrefs {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

/// A workspace's command-id key: `Captions and Graphics` → `captionsandgraphics`.
pub fn slug(name: &str) -> String {
    name.to_ascii_lowercase().replace(' ', "")
}

pub fn is_builtin(name: &str) -> bool {
    WORKSPACES.contains(&name)
}

/// Every workspace, built-in and the user's own, in one alphabetical list (Premiere's Window ▸
/// Workspaces).
pub fn names(prefs: &WorkspacePrefs) -> Vec<String> {
    let mut v: Vec<String> = WORKSPACES.iter().map(|s| s.to_string()).collect();
    for s in &prefs.saved {
        if !v.contains(&s.name) {
            v.push(s.name.clone());
        }
    }
    v.sort_by_key(|n| n.to_lowercase());
    v
}

/// The workspace called `query` (any case, spaces optional).
pub fn find(prefs: &WorkspacePrefs, query: &str) -> Option<String> {
    names(prefs).into_iter().find(|n| slug(n) == slug(query))
}

/// The saved layout of a workspace: the user's saved version, else the built-in default.
pub fn saved_layout(prefs: &WorkspacePrefs, name: &str) -> DockNode {
    prefs.saved.iter().find(|s| s.name == name).and_then(SavedWorkspace::dock).unwrap_or_else(|| workspace(name))
}

/// A workspace as it opens: as last arranged, else its saved layout (dock and undocked groups).
pub fn current_layout(prefs: &WorkspacePrefs, name: &str) -> (DockNode, Vec<FloatingGroup>) {
    if let Some(l) = prefs.live.iter().find(|s| s.name == name)
        && let Some(d) = l.dock()
    {
        return (d, l.floating());
    }
    let floating = prefs.saved.iter().find(|s| s.name == name).map(SavedWorkspace::floating).unwrap_or_default();
    (saved_layout(prefs, name), floating)
}

/// The default layout of a named workspace.
pub fn workspace(name: &str) -> DockNode {
    use PanelKind::*;
    use SplitSize::*;
    let bottom_editing = || hsplit(FixedA(64.0), tabs(&[Tools], 0), hsplit(FixedB(116.0), tabs(&[Timeline], 0), tabs(&[AudioMeters], 0)));
    match name {
        "Assembly" => vsplit(
            Ratio(0.52),
            hsplit(
                Ratio(0.46),
                tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer, Metadata], 0), tabs(&[Program], 0)),
            ),
            bottom_editing(),
        ),
        "Color" => hsplit(
            FixedB(340.0),
            vsplit(
                Ratio(0.58),
                hsplit(Ratio(0.42), tabs(&[LumetriScopes, Source, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, Info, Markers, History], 0), bottom_editing()),
            ),
            tabs(&[LumetriColor, Effects], 0),
        ),
        "Effects" => hsplit(
            FixedB(320.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer], 1), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Info, Markers, History], 0), bottom_editing()),
            ),
            vsplit(Ratio(0.5), tabs(&[Effects], 0), tabs(&[EffectControls, EssentialGraphics], 1)),
        ),
        "Audio" => hsplit(
            FixedB(320.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.5), tabs(&[AudioTrackMixer, Source, AudioClipMixer, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Effects, Markers, History], 0), bottom_editing()),
            ),
            tabs(&[EssentialSound], 0),
        ),
        "Captions and Graphics" => hsplit(
            FixedB(330.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.4), tabs(&[Text, Source, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Libraries], 0), bottom_editing()),
            ),
            tabs(&[EssentialGraphics, Properties], 0),
        ),
        "All Panels" => vsplit(
            Ratio(0.5),
            hsplit(
                Ratio(0.5),
                tabs(&[Source, EffectControls, AudioClipMixer, Metadata, LumetriScopes, Text], 0),
                hsplit(Ratio(0.6), tabs(&[Program], 0), tabs(&[LumetriColor, EssentialGraphics, EssentialSound, Properties, AudioTrackMixer], 0)),
            ),
            hsplit(Ratio(0.26), tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0), bottom_editing()),
        ),
        // Editing (default), Learning, Review — Premiere 26 factory layout:
        // top: Source group | Program | Properties; bottom: Project group | Tools | Timeline | Meters.
        _ => vsplit(
            Ratio(0.57),
            hsplit(
                FixedB(450.0),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer, Metadata], 0), tabs(&[Program], 0)),
                tabs(&[Properties, EssentialGraphics, Text], 0),
            ),
            hsplit(
                Ratio(0.29),
                tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0),
                hsplit(FixedA(64.0), tabs(&[Tools], 0), hsplit(FixedB(116.0), tabs(&[Timeline], 0), tabs(&[AudioMeters], 0))),
            ),
        ),
    }
}

impl DockNode {
    /// Every panel in the tree.
    pub fn panels(&self, out: &mut Vec<PanelKind>) {
        match self {
            DockNode::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
            DockNode::Tabs { panels, .. } => out.extend(panels.iter().copied()),
        }
    }
    pub fn contains(&self, p: PanelKind) -> bool {
        let mut v = Vec::new();
        self.panels(&mut v);
        v.contains(&p)
    }
    /// Make `p` the active tab of its group. Returns false if not present.
    pub fn activate(&mut self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.activate(p) || b.activate(p),
            DockNode::Tabs { panels, active } => {
                if let Some(i) = panels.iter().position(|x| *x == p) {
                    *active = i;
                    true
                } else {
                    false
                }
            }
        }
    }
    pub fn is_visible(&self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.is_visible(p) || b.is_visible(p),
            DockNode::Tabs { panels, active } => panels.get(*active) == Some(&p),
        }
    }
    /// Close a panel (remove its tab; empty groups collapse their split).
    pub fn close(&mut self, p: PanelKind) {
        if let DockNode::Split { a, b, .. } = self {
            a.close(p);
            b.close(p);
            let empty = |n: &DockNode| matches!(n, DockNode::Tabs { panels, .. } if panels.is_empty());
            if empty(a) {
                *self = (**b).clone();
            } else if empty(b) {
                *self = (**a).clone();
            }
        } else if let DockNode::Tabs { panels, active } = self {
            panels.retain(|x| *x != p);
            *active = (*active).min(panels.len().saturating_sub(1));
        }
    }
    /// Put the Timeline panel back where the workspaces keep it (left of the audio meters) after
    /// it was closed, so opening a sequence always has somewhere to show it.
    pub fn restore_timeline(&mut self) {
        if self.contains(PanelKind::Timeline) {
            return;
        }
        fn beside_meters(n: &mut DockNode) -> bool {
            match n {
                DockNode::Split { a, b, .. } => beside_meters(a) || beside_meters(b),
                DockNode::Tabs { panels, .. } if panels.as_slice() == [PanelKind::AudioMeters] => {
                    *n = hsplit(SplitSize::FixedB(116.0), tabs(&[PanelKind::Timeline], 0), tabs(&[PanelKind::AudioMeters], 0));
                    true
                }
                DockNode::Tabs { .. } => false,
            }
        }
        if !beside_meters(self) {
            self.open_near(PanelKind::Timeline, PanelKind::Project);
        }
    }
    /// Add a panel as a tab next to `near` (or into the first group).
    pub fn open_near(&mut self, p: PanelKind, near: PanelKind) {
        if self.contains(p) {
            self.activate(p);
            return;
        }
        fn add(n: &mut DockNode, p: PanelKind, near: PanelKind) -> bool {
            match n {
                DockNode::Split { a, b, .. } => add(a, p, near) || add(b, p, near),
                DockNode::Tabs { panels, active } => {
                    if panels.contains(&near) {
                        panels.push(p);
                        *active = panels.len() - 1;
                        true
                    } else {
                        false
                    }
                }
            }
        }
        if !add(self, p, near) && !add(self, p, PanelKind::Project) {
            add(self, p, PanelKind::Program);
        }
    }
}

/// Where dragged panels land on a tab group (or on the dock area's outer edge): as tabs of the
/// group, or beside it in a new split.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl Zone {
    pub fn from_name(s: &str) -> Option<Zone> {
        Some(match s.to_ascii_lowercase().as_str() {
            "center" | "centre" | "tab" | "tabs" => Zone::Center,
            "left" => Zone::Left,
            "right" => Zone::Right,
            "top" => Zone::Top,
            "bottom" => Zone::Bottom,
            _ => return None,
        })
    }
}

/// A panel group undocked into a window of its own (Premiere's Undock Panel / Undock Panel Group).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FloatingGroup {
    /// Stable id of its window.
    pub id: u64,
    pub panels: Vec<PanelKind>,
    pub active: usize,
    /// Window size (points).
    pub size: [f32; 2],
}

impl Default for FloatingGroup {
    fn default() -> Self {
        FloatingGroup { id: 0, panels: Vec::new(), active: 0, size: [640.0, 420.0] }
    }
}

impl DockNode {
    /// The panels of the tab group holding `p`, and its active index.
    pub fn group_of(&self, p: PanelKind) -> Option<(Vec<PanelKind>, usize)> {
        match self {
            DockNode::Split { a, b, .. } => a.group_of(p).or_else(|| b.group_of(p)),
            DockNode::Tabs { panels, active } => panels.contains(&p).then(|| (panels.clone(), *active)),
        }
    }

    /// Whether the dock holds no panel at all.
    pub fn is_empty(&self) -> bool {
        let mut v = Vec::new();
        self.panels(&mut v);
        v.is_empty()
    }

    /// Insert `panels` (not in the tree) at `zone` of the group holding `target`; `index` places
    /// them among the group's tabs (Center; None = after the last). `active` is the one shown.
    fn insert_at(&mut self, panels: &[PanelKind], active: usize, target: PanelKind, zone: Zone, index: Option<usize>) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.insert_at(panels, active, target, zone, index) || b.insert_at(panels, active, target, zone, index),
            DockNode::Tabs { panels: here, active: here_active } => {
                if !here.contains(&target) {
                    return false;
                }
                let shown = panels.get(active).or(panels.first()).copied();
                if zone == Zone::Center {
                    let at = index.unwrap_or(here.len()).min(here.len());
                    for (k, p) in panels.iter().enumerate() {
                        here.insert(at + k, *p);
                    }
                    *here_active = shown.and_then(|s| here.iter().position(|x| *x == s)).unwrap_or(*here_active);
                    return true;
                }
                let new = tabs(panels, active.min(panels.len().saturating_sub(1)));
                let old = std::mem::replace(self, tabs(&[], 0));
                let (a, b) = if matches!(zone, Zone::Left | Zone::Top) { (new, old) } else { (old, new) };
                *self = DockNode::Split { vertical: matches!(zone, Zone::Top | Zone::Bottom), size: SplitSize::Ratio(0.5), a: Box::new(a), b: Box::new(b) };
                true
            }
        }
    }

    /// Where `p` sits, as a place to put it back: a panel of its group and Center when it shares
    /// the group, else a panel of the neighbouring part of the layout and the side `p` is on.
    pub fn home_of(&self, p: PanelKind) -> Option<(PanelKind, Zone)> {
        fn first(n: &DockNode) -> Option<PanelKind> {
            match n {
                DockNode::Split { a, b, .. } => first(a).or_else(|| first(b)),
                DockNode::Tabs { panels, .. } => panels.first().copied(),
            }
        }
        fn find(n: &DockNode, p: PanelKind) -> Option<(PanelKind, Zone)> {
            match n {
                DockNode::Tabs { panels, .. } => panels.iter().copied().find(|x| *x != p).filter(|_| panels.contains(&p)).map(|x| (x, Zone::Center)),
                DockNode::Split { vertical, a, b, .. } => {
                    let alone = |c: &DockNode| matches!(c, DockNode::Tabs { panels, .. } if panels.as_slice() == [p]);
                    if alone(a) {
                        return first(b).map(|x| (x, if *vertical { Zone::Top } else { Zone::Left }));
                    }
                    if alone(b) {
                        return first(a).map(|x| (x, if *vertical { Zone::Bottom } else { Zone::Right }));
                    }
                    find(a, p).or_else(|| find(b, p))
                }
            }
        }
        find(self, p)
    }

    /// Move `moving` (one panel, or a whole group with `active` shown) to `zone` of the group
    /// holding `target`. Panels not in the tree (a floating group) are docked. Returns false and
    /// leaves the layout as it was when there is nothing to do (the target is among the moved
    /// panels, or it is not in the dock).
    pub fn move_panels(&mut self, moving: &[PanelKind], active: usize, target: PanelKind, zone: Zone, index: Option<usize>) -> bool {
        if moving.is_empty() || moving.contains(&target) || !self.contains(target) {
            return false;
        }
        // a tab moved along its own group: the index counts the tabs as they were
        let mut index = index;
        if let (Some(i), Some((group, _))) = (index, self.group_of(target)) {
            let before = group.iter().take(i).filter(|p| moving.contains(p)).count();
            index = Some(i - before);
        }
        let mut t = self.clone();
        for p in moving {
            t.close(*p);
        }
        if !t.insert_at(moving, active, target, zone, index) {
            return false;
        }
        *self = t;
        true
    }

    /// Move `moving` to an outer edge of the dock area: a full-height column (Left / Right) or a
    /// full-width row (Top / Bottom); Center docks them as a tab group beside the first group.
    pub fn move_to_edge(&mut self, moving: &[PanelKind], active: usize, zone: Zone) -> bool {
        if moving.is_empty() {
            return false;
        }
        let mut t = self.clone();
        for p in moving {
            t.close(*p);
        }
        let new = tabs(moving, active.min(moving.len() - 1));
        if t.is_empty() {
            *self = new;
            return true;
        }
        let (vertical, size, first) = match zone {
            Zone::Left => (false, SplitSize::Ratio(0.25), true),
            Zone::Right => (false, SplitSize::Ratio(0.75), false),
            Zone::Top => (true, SplitSize::Ratio(0.3), true),
            Zone::Bottom | Zone::Center => (true, SplitSize::Ratio(0.7), false),
        };
        let (a, b) = if first { (new, t) } else { (t, new) };
        *self = DockNode::Split { vertical, size, a: Box::new(a), b: Box::new(b) };
        true
    }
}

/// One laid-out tab group.
pub struct Group {
    pub path: String,
    pub rect: Rect,
    pub content: Rect,
    pub panels: Vec<PanelKind>,
    pub active: usize,
}

/// Actions produced by interacting with the dock chrome.
#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    Activate(PanelKind),
    Focus(PanelKind),
    Close(PanelKind),
    PanelMenu(PanelKind, egui::Pos2),
    /// A sequence tab of the Timeline panel was clicked / closed (project item id).
    OpenSequence(u64),
    CloseSequence(u64),
    /// A sequence tab was dragged past a neighbour: its new place among the sequence tabs.
    MoveSequence(u64, usize),
    /// A tab (or a whole group, by the empty part of its strip) started being dragged to a new
    /// place: the panels and the one shown.
    StartDrag(Vec<PanelKind>, usize),
}

/// Size of a split's first child in `avail` points: the requested size, keeping both children at
/// least 20 points when there is room (and never more than `avail`). Must not panic for any size:
/// a tiny window (or browser canvas) gives splits less than 40 points.
fn split_first(size: SplitSize, avail: f32) -> f32 {
    let want = match size {
        SplitSize::Ratio(r) => avail * r,
        SplitSize::FixedA(px) => px,
        SplitSize::FixedB(px) => avail - px,
    };
    let lo = 20.0_f32.min(avail * 0.5);
    let hi = (avail - 20.0).max(lo);
    want.clamp(lo, hi)
}

/// Lay out the tree into group rects (with `gap` gutters) and handle gutter dragging.
pub fn layout(ui: &mut egui::Ui, node: &mut DockNode, rect: Rect, t: &Tokens, path: &str, out: &mut Vec<Group>, reg: &mut crate::automation::Registry) {
    match node {
        DockNode::Tabs { panels, active } => {
            let tab_h = if panels.len() == 1 && panels[0].compact() { 8.0 } else { t.tab_h };
            let content = Rect::from_min_max(pos2(rect.min.x, rect.min.y + tab_h), rect.max);
            out.push(Group { path: path.to_string(), rect, content, panels: panels.clone(), active: *active });
        }
        DockNode::Split { vertical, size, a, b } => {
            let g = t.gap;
            let total = if *vertical { rect.height() } else { rect.width() };
            let avail = (total - g).max(0.0);
            let first = split_first(*size, avail);
            let (ra, gutter, rb) = if *vertical {
                (
                    Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + first)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first), pos2(rect.max.x, rect.min.y + first + g)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first + g), rect.max),
                )
            } else {
                (
                    Rect::from_min_max(rect.min, pos2(rect.min.x + first, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first, rect.min.y), pos2(rect.min.x + first + g, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first + g, rect.min.y), rect.max),
                )
            };
            // gutter drag (a slightly larger hit area than the visible gap)
            let hit = gutter.expand2(if *vertical { vec2(0.0, 3.0) } else { vec2(3.0, 0.0) });
            let id = egui::Id::new(("dock-gutter", path.to_string()));
            let resp = ui.interact(hit, id, Sense::drag());
            reg.add(&format!("dock.gutter.{path}"), hit, "gutter");
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if *vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            }
            if resp.dragged() {
                let d = if *vertical { resp.drag_delta().y } else { resp.drag_delta().x };
                let nf = (first + d).clamp(40.0, (avail - 40.0).max(40.0));
                *size = match *size {
                    SplitSize::Ratio(_) => SplitSize::Ratio(nf / avail.max(1.0)),
                    SplitSize::FixedA(_) => SplitSize::FixedA(nf),
                    SplitSize::FixedB(_) => SplitSize::FixedB(avail - nf),
                };
            }
            if resp.dragged() || resp.hovered() {
                ui.painter().rect_filled(gutter, 0.0, t.focus.gamma_multiply(if resp.dragged() { 0.9 } else { 0.4 }));
            }
            layout(ui, a, ra, t, &format!("{path}a"), out, reg);
            layout(ui, b, rb, t, &format!("{path}b"), out, reg);
        }
    }
}

/// Draw a group's frame + tab strip. Returns actions (tab clicks, panel menu, focus).
/// The sequences open in the Timeline panel, in tab order: `(project item id, name)`.
#[derive(Debug, Clone, Default)]
pub struct SeqTabs {
    pub open: Vec<(u64, String)>,
    pub active: Option<u64>,
}

pub fn draw_group_chrome(
    ui: &mut egui::Ui,
    g: &Group,
    focused: PanelKind,
    seqs: &SeqTabs,
    t: &Tokens,
    reg: &mut crate::automation::Registry,
) -> (Vec<DockAction>, Vec<(PanelKind, Rect)>) {
    let mut actions = Vec::new();
    let mut drawn: Vec<(PanelKind, Rect)> = Vec::new();
    let painter = ui.painter().clone();
    painter.rect_filled(g.rect, t.radius, t.panel_bg);
    let active_panel = g.panels.get(g.active).copied();
    let compact = g.panels.len() == 1 && g.panels[0].compact();
    if compact {
        // grip dots: drag the panel to a new place
        let c = pos2(g.rect.center().x, g.rect.min.y + 4.0);
        let grip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), 8.0));
        let gr = ui.interact(grip, egui::Id::new(("group-grip", g.path.clone())), Sense::click_and_drag());
        let gr = gr.on_hover_text("Drag to move this panel; right-click for its panel menu");
        reg.add(&format!("panel.grip.{}", g.panels.first().map(|p| p.id()).unwrap_or_default()), grip, "move panel");
        if gr.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if gr.drag_started() {
            actions.push(DockAction::StartDrag(g.panels.clone(), g.active));
        }
        for dx in [-4.0, 0.0, 4.0] {
            painter.circle_filled(c + vec2(dx, 0.0), 1.0, if gr.hovered() { t.tab_text_active } else { t.text_faint });
        }
        if let Some(p) = g.panels.first().copied() {
            // right-click: the panel menu (Close Panel, Undock Panel, …), as on a tab
            if gr.secondary_clicked() {
                actions.push(DockAction::Focus(p));
                actions.push(DockAction::PanelMenu(p, gr.interact_pointer_pos().unwrap_or(grip.left_bottom())));
            }
            // × at the right end of the grip, shown while the pointer is over the panel's top
            let cr = Rect::from_center_size(pos2(grip.max.x - 7.0, grip.center().y), vec2(6.0, 6.0));
            let cresp = ui.interact(cr.expand(3.0), egui::Id::new(("grip-x", g.path.clone())), Sense::click());
            reg.add(&format!("panel.close.{}", p.id()), cr.expand(3.0), "close panel");
            let near = ui.rect_contains_pointer(Rect::from_min_max(g.rect.min, pos2(g.rect.max.x, g.rect.min.y + 24.0)));
            if near || cresp.hovered() {
                let cc = if cresp.hovered() { t.tab_text_active } else { t.tab_text };
                painter.line_segment([cr.left_top(), cr.right_bottom()], Stroke::new(1.1, cc));
                painter.line_segment([cr.right_top(), cr.left_bottom()], Stroke::new(1.1, cc));
            }
            if cresp.clicked() {
                actions.push(DockAction::Close(p));
            }
        }
    } else {
        let strip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), t.tab_h));
        // the Timeline panel shows one tab per open sequence (its own tab names the sequence)
        struct Tab {
            panel: PanelKind,
            seq: Option<u64>,
            title: String,
            active: bool,
        }
        let mut tabs: Vec<Tab> = Vec::new();
        for (i, p) in g.panels.iter().enumerate() {
            let active = i == g.active;
            if *p == PanelKind::Timeline && active && !seqs.open.is_empty() {
                tabs.extend(seqs.open.iter().map(|(id, name)| Tab { panel: *p, seq: Some(*id), title: name.clone(), active: seqs.active == Some(*id) }));
            } else if *p == PanelKind::Timeline {
                let name = seqs.open.iter().find(|(id, _)| seqs.active == Some(*id)).map(|(_, n)| n.clone());
                tabs.push(Tab { panel: *p, seq: None, title: name.unwrap_or_else(|| "Timeline: (no sequences)".into()), active });
            } else {
                tabs.push(Tab { panel: *p, seq: None, title: p.title().to_string(), active });
            }
        }
        let activate = |tab: &Tab, actions: &mut Vec<DockAction>| match tab.seq {
            Some(id) => actions.push(DockAction::OpenSequence(id)),
            None => actions.push(DockAction::Activate(tab.panel)),
        };
        let active_at = tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let text_y = strip.min.y + 16.0;
        // Which tabs fit. The active tab is always shown: tabs before it are left out until it
        // fits, then as many after it as there is room for. Tabs that are left out are reached
        // through the » list at the right end of the strip.
        let widths: Vec<f32> = tabs
            .iter()
            .map(|tab| {
                let label = painter.layout_no_wrap(tab.title.clone(), Tokens::ui(12.0), t.tab_text).size().x;
                label
                    + 16.0
                    + if tab.active { 20.0 } else { 0.0 }
                    + if tab.active && tab.seq.is_some() { 16.0 } else { 0.0 }
                    + if tab.seq.is_none() { CLOSE_W } else { 0.0 }
            })
            .collect();
        let span = |from: usize, to: usize| widths.get(from..=to).map_or(0.0, |w| w.iter().sum::<f32>() + 8.0 * w.len() as f32);
        let room = strip.width() - 12.0;
        let all_fit = span(0, tabs.len().saturating_sub(1)) <= room;
        let room = if all_fit { room } else { room - 24.0 };
        let mut first = 0;
        while first < active_at && span(first, active_at) > room {
            first += 1;
        }
        let mut last = active_at.max(first);
        while last + 1 < tabs.len() && span(first, last + 1) <= room {
            last += 1;
        }
        let list_id = egui::Id::new(("tab-list", g.path.clone()));
        let more = (!all_fit).then(|| Rect::from_min_size(pos2(strip.max.x - 22.0, strip.min.y + 6.0), vec2(18.0, 20.0)));
        let mut list_opened = false;
        if let Some(r) = more {
            let resp = ui.interact(r, egui::Id::new(("tab-overflow", g.path.clone())), Sense::click());
            let name = if g.panels.contains(&PanelKind::Timeline) { "timeline.tabs.more".to_string() } else { format!("panel.tabs.more.{}", g.path) };
            reg.add(&name, r, "more tabs");
            let c = if resp.hovered() { t.tab_text_active } else { t.tab_text };
            icons::paint(&painter, r.shrink(4.0).translate(vec2(-2.0, 0.0)), Icon::ChevronRight, c);
            icons::paint(&painter, r.shrink(4.0).translate(vec2(2.0, 0.0)), Icon::ChevronRight, c);
            if resp.clicked() {
                let open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
                ui.ctx().data_mut(|d| d.insert_temp(list_id, !open));
                list_opened = !open;
            }
        }
        let mut x = strip.min.x + 12.0;
        // sequence tabs as drawn (for dragging one past its neighbours), and the one being dragged
        let mut seq_rects: Vec<(u64, Rect)> = Vec::new();
        let mut dragged: Option<u64> = None;
        for (i, tab) in tabs.iter().enumerate().take(last + 1).skip(first) {
            let p = &tab.panel;
            let is_active = tab.active;
            let galley = painter.layout_no_wrap(tab.title.clone(), Tokens::ui(12.0), if is_active { t.tab_text_active } else { t.tab_text });
            let close_w = if is_active && tab.seq.is_some() { 16.0 } else { 0.0 };
            let w = widths.get(i).copied().unwrap_or(0.0);
            let tab_rect = Rect::from_min_size(pos2(x, strip.min.y), vec2(w, t.tab_h));
            // a sequence tab keeps its identity when the tabs are reordered under the pointer
            let resp = match tab.seq {
                Some(id) => ui.interact(tab_rect, egui::Id::new(("tab-seq", g.path.clone(), id)), Sense::click_and_drag()),
                None => ui.interact(tab_rect, egui::Id::new(("tab", g.path.clone(), p.id())), Sense::click_and_drag()),
            };
            drawn.push((*p, tab_rect));
            if tab.seq.is_none() && resp.drag_started() {
                actions.push(DockAction::StartDrag(vec![*p], 0));
            }
            if let Some(id) = tab.seq {
                seq_rects.push((id, tab_rect));
                if resp.dragged() {
                    dragged = Some(id);
                    // pulled out of the strip: the Timeline panel itself moves
                    if let Some(py) = ui.input(|i| i.pointer.latest_pos()).map(|p| p.y)
                        && (py > strip.max.y + 24.0 || py < strip.min.y - 12.0)
                    {
                        actions.push(DockAction::StartDrag(vec![*p], 0));
                    }
                }
            }
            if let Some(id) = tab.seq {
                reg.add(&format!("timeline.tab.{id}"), tab_rect, &tab.title);
            }
            if tab.seq.is_none() || is_active {
                reg.add(&format!("panel.tab.{}", p.id()), tab_rect, p.title());
            }
            let label_x = tab_rect.min.x + 8.0 + close_w;
            let label_w = galley.size().x;
            let col = if is_active || resp.hovered() { t.tab_text_active } else { t.tab_text };
            painter.galley_with_override_text_color(pos2(label_x, text_y - galley.size().y / 2.0), galley, col);
            let mut closed = false;
            if tab.seq.is_none() {
                // × closes the panel (shown on the active tab and under the pointer)
                let cr = Rect::from_center_size(pos2(tab_rect.max.x - CLOSE_W / 2.0 - 2.0, text_y), vec2(7.0, 7.0));
                let cresp = ui.interact(cr.expand(4.0), egui::Id::new(("tab-x", g.path.clone(), p.id())), Sense::click());
                reg.add(&format!("panel.close.{}", p.id()), cr.expand(4.0), "close panel");
                if is_active || resp.hovered() || cresp.hovered() {
                    let cc = if cresp.hovered() { t.tab_text_active } else { t.tab_text };
                    painter.line_segment([cr.left_top(), cr.right_bottom()], Stroke::new(1.1, cc));
                    painter.line_segment([cr.right_top(), cr.left_bottom()], Stroke::new(1.1, cc));
                }
                if cresp.clicked() {
                    actions.push(DockAction::Close(*p));
                    closed = true;
                }
            }
            if let Some(id) = tab.seq.filter(|_| is_active) {
                // × closes the sequence's tab (the sequence stays in the project)
                let cr = Rect::from_center_size(pos2(tab_rect.min.x + 10.0, text_y), vec2(8.0, 8.0));
                let cresp = ui.interact(cr.expand(4.0), egui::Id::new(("tab-close", g.path.clone(), id)), Sense::click());
                reg.add(&format!("timeline.tab.{id}.close"), cr.expand(4.0), "close sequence");
                let cc = if cresp.hovered() { t.tab_text_active } else { t.tab_text };
                painter.line_segment([cr.left_top(), cr.right_bottom()], Stroke::new(1.2, cc));
                painter.line_segment([cr.right_top(), cr.left_bottom()], Stroke::new(1.2, cc));
                if cresp.clicked() {
                    actions.push(DockAction::CloseSequence(id));
                    closed = true;
                }
            }
            if is_active {
                let mr = Rect::from_center_size(pos2(label_x + label_w + 12.0, text_y), vec2(12.0, 10.0));
                let mresp = ui.interact(mr.expand(3.0), egui::Id::new(("tab-menu", g.path.clone())), Sense::click());
                reg.add(&format!("panel.menu.{}", p.id()), mr, "panel menu");
                let mc = if mresp.hovered() { t.tab_text_active } else { t.tab_text };
                icons::paint(&painter, Rect::from_center_size(mr.center(), vec2(16.0, 16.0)), Icon::Hamburger, mc);
                // 1 pt underline spanning label + ≡, 23 pt below the frame top
                let uy = strip.min.y + 23.0;
                painter.line_segment([pos2(label_x, uy), pos2(mr.max.x, uy)], Stroke::new(1.0, t.tab_text_active));
                if mresp.clicked() {
                    actions.push(DockAction::PanelMenu(*p, mr.left_bottom()));
                }
            }
            if (resp.clicked() || resp.drag_started()) && !closed {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(*p));
            }
            // a right-click shows the tab and opens its panel menu (as in Premiere; a sequence
            // tab's menu starts with Sequence Settings…)
            if resp.secondary_clicked() {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(*p));
                let at = resp.interact_pointer_pos().unwrap_or(tab_rect.left_bottom());
                actions.push(DockAction::PanelMenu(*p, at));
            }
            if resp.middle_clicked() {
                // a sequence tab closes its sequence; the Timeline panel itself stays
                match tab.seq.or(seqs.active.filter(|_| *p == PanelKind::Timeline)) {
                    Some(id) => actions.push(DockAction::CloseSequence(id)),
                    None => actions.push(DockAction::Close(*p)),
                }
            }
            x += w + 8.0;
        }
        // the strip right of the tabs: drag the whole group to a new place
        let grip_end = more.map_or(strip.max.x, |r| r.min.x - 2.0);
        if grip_end - x > 12.0 {
            let grip = Rect::from_min_max(pos2(x, strip.min.y), pos2(grip_end, strip.max.y));
            let gr = ui.interact(grip, egui::Id::new(("group-grip", g.path.clone())), Sense::drag());
            reg.add(&format!("panel.grip.{}", g.panels.get(g.active).map(|p| p.id()).unwrap_or_default()), grip, "move panel group");
            if gr.drag_started() {
                actions.push(DockAction::StartDrag(g.panels.clone(), g.active));
            }
        }
        // Dragging a sequence tab sideways: it changes places with a neighbour when the pointer
        // passes that neighbour's middle.
        if let Some(id) = dragged
            && let Some(px) = ui.input(|i| i.pointer.latest_pos()).map(|p| p.x)
            && let Some(at) = seq_rects.iter().position(|(s, _)| *s == id)
            && let Some(index) = seqs.open.iter().position(|(s, _)| *s == id)
        {
            let before = at.checked_sub(1).and_then(|k| seq_rects.get(k));
            let after = seq_rects.get(at + 1);
            if before.is_some_and(|(_, r)| px < r.center().x) {
                actions.push(DockAction::MoveSequence(id, index.saturating_sub(1)));
            } else if after.is_some_and(|(_, r)| px > r.center().x) {
                actions.push(DockAction::MoveSequence(id, index + 1));
            }
        }
        // the » list: every tab of the group, the shown one ticked
        let list_open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
        if let Some(r) = more.filter(|_| list_open) {
            let mut picked = None;
            let area = egui::Area::new(list_id.with("area")).order(egui::Order::Foreground).pivot(Align2::RIGHT_TOP).fixed_pos(r.right_bottom()).show(
                ui.ctx(),
                |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(160.0);
                        // many open sequences: the list scrolls instead of leaving the window
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for tab in &tabs {
                                let label = if tab.active { format!("✓ {}", tab.title) } else { tab.title.clone() };
                                let resp = ui.button(label);
                                match tab.seq {
                                    Some(id) => reg.add(&format!("timeline.tabs.list.{id}"), resp.rect, &tab.title),
                                    None => reg.add(&format!("panel.tabs.list.{}", tab.panel.id()), resp.rect, &tab.title),
                                }
                                if resp.clicked() {
                                    picked = Some(tab);
                                }
                            }
                        });
                    });
                },
            );
            if let Some(tab) = picked {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(tab.panel));
            }
            if picked.is_some() || (!list_opened && area.response.clicked_elsewhere()) {
                ui.ctx().data_mut(|d| d.remove::<bool>(list_id));
            }
        } else if list_open {
            ui.ctx().data_mut(|d| d.remove::<bool>(list_id));
        }
    }
    // focus outline
    if active_panel == Some(focused) {
        painter.rect_stroke(g.rect, 0.0, Stroke::new(1.0, t.focus), StrokeKind::Inside);
    }
    // clicking anywhere in the panel focuses it
    if let Some(p) = active_panel
        && ui.rect_contains_pointer(g.rect)
        && ui.input(|i| i.pointer.any_pressed())
    {
        actions.push(DockAction::Focus(p));
    }
    (actions, drawn)
}

/// Room for a tab's close ×.
const CLOSE_W: f32 = 14.0;

/// What a dragged panel or group would dock onto at the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropTarget {
    /// `zone` of the group holding `target` (`index`: the tab position in its strip).
    Group { target: PanelKind, zone: Zone, index: Option<usize>, highlight: Rect },
    /// An outer edge of the dock area.
    Edge { zone: Zone, highlight: Rect },
}

/// Pointer within this many points of the dock area's outer edge docks to the edge.
pub const EDGE_BAND: f32 = 14.0;

/// The drop target at `pos`: the dock area's outer edge, a group's tab strip (a tab at the
/// position between its tabs), or the group's centre / one of its four sides (the outer quarter
/// of each side, as Premiere's drop zones). `tab_rects` are the tabs drawn per group path.
/// None over nothing, or over the dragged group itself (`moving` = its panels).
pub fn drop_target(
    groups: &[Group],
    area: Rect,
    pos: egui::Pos2,
    tab_h: f32,
    tab_rects: &[(String, Vec<(PanelKind, Rect)>)],
    moving: &[PanelKind],
) -> Option<DropTarget> {
    if !area.contains(pos) {
        return None;
    }
    let edge = |zone: Zone| {
        let h = match zone {
            Zone::Left => Rect::from_min_max(area.min, pos2(area.min.x + area.width() * 0.25, area.max.y)),
            Zone::Right => Rect::from_min_max(pos2(area.max.x - area.width() * 0.25, area.min.y), area.max),
            Zone::Top => Rect::from_min_max(area.min, pos2(area.max.x, area.min.y + area.height() * 0.3)),
            _ => Rect::from_min_max(pos2(area.min.x, area.max.y - area.height() * 0.3), area.max),
        };
        DropTarget::Edge { zone, highlight: h }
    };
    if pos.x < area.min.x + EDGE_BAND {
        return Some(edge(Zone::Left));
    }
    if pos.x > area.max.x - EDGE_BAND {
        return Some(edge(Zone::Right));
    }
    if pos.y > area.max.y - EDGE_BAND {
        return Some(edge(Zone::Bottom));
    }
    let g = groups.iter().find(|g| g.rect.contains(pos) && !g.panels.is_empty())?;
    let whole = !moving.is_empty() && g.panels.iter().all(|p| moving.contains(p));
    let target = g.panels.iter().copied().find(|p| !moving.contains(p))?;
    let strip = Rect::from_min_max(g.rect.min, pos2(g.rect.max.x, g.rect.min.y + tab_h));
    if strip.contains(pos) {
        if whole {
            return None;
        }
        let tabs = tab_rects.iter().find(|(path, _)| *path == g.path).map(|(_, t)| t.as_slice()).unwrap_or(&[]);
        // before the first tab whose middle is right of the pointer, else after the last
        let index = tabs.iter().find(|(_, r)| pos.x < r.center().x).and_then(|(p, _)| g.panels.iter().position(|x| x == p)).unwrap_or(g.panels.len());
        return Some(DropTarget::Group { target, zone: Zone::Center, index: Some(index), highlight: strip });
    }
    let r = g.content;
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let d =
        [(Zone::Left, (pos.x - r.min.x) / w), (Zone::Right, (r.max.x - pos.x) / w), (Zone::Top, (pos.y - r.min.y) / h), (Zone::Bottom, (r.max.y - pos.y) / h)];
    let (zone, near) = d.iter().copied().fold((Zone::Center, f32::INFINITY), |acc, (z, v)| if v < acc.1 { (z, v) } else { acc });
    let zone = if near < 0.25 { zone } else { Zone::Center };
    if whole {
        // a group cannot dock beside or into itself
        return None;
    }
    let rr = g.rect;
    let highlight = match zone {
        Zone::Center => r,
        Zone::Left => Rect::from_min_max(rr.min, pos2(rr.center().x, rr.max.y)),
        Zone::Right => Rect::from_min_max(pos2(rr.center().x, rr.min.y), rr.max),
        Zone::Top => Rect::from_min_max(rr.min, pos2(rr.max.x, rr.center().y)),
        Zone::Bottom => Rect::from_min_max(pos2(rr.min.x, rr.center().y), rr.max),
    };
    Some(DropTarget::Group { target, zone, index: None, highlight })
}

/// Placeholder body for panels that are not implemented yet.
pub fn placeholder(ui: &mut egui::Ui, rect: Rect, t: &Tokens, text: &str) {
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, Tokens::ui(12.0), t.text_faint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_first_never_panics_on_tiny_areas() {
        for avail in [0.0, 1.0, 15.0, 20.0, 35.0, 39.9, 40.0, 100.0, 2000.0] {
            for size in [
                SplitSize::Ratio(0.0),
                SplitSize::Ratio(0.5),
                SplitSize::Ratio(1.0),
                SplitSize::FixedA(300.0),
                SplitSize::FixedB(300.0),
                SplitSize::FixedA(0.0),
            ] {
                let f = split_first(size, avail);
                assert!((0.0..=avail).contains(&f), "{size:?} in {avail}: {f}");
            }
        }
        assert_eq!(split_first(SplitSize::FixedA(300.0), 1000.0), 300.0);
        assert_eq!(split_first(SplitSize::FixedB(300.0), 1000.0), 700.0);
        assert_eq!(split_first(SplitSize::Ratio(0.0), 1000.0), 20.0);
    }

    #[test]
    fn workspaces_contain_core_panels() {
        for w in WORKSPACES {
            let d = workspace(w);
            assert!(d.contains(PanelKind::Timeline), "{w}");
            assert!(d.contains(PanelKind::Program), "{w}");
        }
    }

    #[test]
    fn close_and_open() {
        let mut d = workspace("Editing");
        d.close(PanelKind::Metadata);
        assert!(!d.contains(PanelKind::Metadata));
        d.open_near(PanelKind::LumetriColor, PanelKind::Program);
        assert!(d.is_visible(PanelKind::LumetriColor));
        let s = serde_json::to_string(&d).unwrap();
        let back: DockNode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    use PanelKind::*;

    fn group(d: &DockNode, p: PanelKind) -> Vec<PanelKind> {
        d.group_of(p).unwrap().0
    }

    #[test]
    fn a_tab_moves_into_another_group_at_a_position() {
        let mut d = workspace("Editing");
        assert!(d.move_panels(&[Effects], 0, Program, Zone::Center, Some(0)));
        assert_eq!(group(&d, Program), [Effects, Program]);
        assert!(d.is_visible(Effects), "the moved tab is shown");
        assert!(!group(&d, Project).contains(&Effects));
        // reordering inside a group: Libraries before Project
        let mut d = workspace("Editing");
        assert!(d.move_panels(&[Libraries], 0, Project, Zone::Center, Some(0)));
        assert_eq!(&group(&d, Project)[..3], &[Libraries, Project, MediaBrowser]);
        // to the end of its own group (index counts the tabs as they were)
        let mut d = workspace("Editing");
        let n = group(&d, Project).len();
        assert!(d.move_panels(&[Project], 0, History, Zone::Center, Some(n)));
        assert_eq!(group(&d, History).last(), Some(&Project));
    }

    #[test]
    fn a_tab_dropped_on_a_side_splits_that_group() {
        let mut d = workspace("Editing");
        assert!(d.move_panels(&[Effects], 0, Timeline, Zone::Top, None));
        assert_eq!(group(&d, Effects), [Effects]);
        assert_eq!(group(&d, Timeline), [Timeline]);
        // the Timeline's place in the tree is now a vertical split with Effects above it
        fn find(n: &DockNode) -> bool {
            match n {
                DockNode::Split { vertical: true, a, b, .. } if **a == tabs(&[Effects], 0) && **b == tabs(&[Timeline], 0) => true,
                DockNode::Split { a, b, .. } => find(a) || find(b),
                _ => false,
            }
        }
        assert!(find(&d), "{d:?}");
        // a panel taken from a group of two beside the other one
        let mut d = DockNode::Tabs { panels: vec![Source, Program], active: 1 };
        assert!(d.move_panels(&[Source], 0, Program, Zone::Left, None));
        assert_eq!(d, hsplit(SplitSize::Ratio(0.5), tabs(&[Source], 0), tabs(&[Program], 0)));
    }

    #[test]
    fn whole_groups_move_and_empty_splits_collapse() {
        let mut d = workspace("Editing");
        let (props, _) = d.group_of(Properties).unwrap();
        let before = d.clone();
        assert!(d.move_panels(&props, 1, Timeline, Zone::Right, None));
        assert_eq!(group(&d, Properties), props, "the group keeps its tabs");
        assert!(d.is_visible(EssentialGraphics), "and the tab it showed");
        let mut all = Vec::new();
        d.panels(&mut all);
        let mut was = Vec::new();
        before.panels(&mut was);
        all.sort_by_key(|p| p.id());
        was.sort_by_key(|p| p.id());
        assert_eq!(all, was, "no panel lost or duplicated");
        // nothing to do: onto itself, onto a panel that is not docked
        let mut d = workspace("Editing");
        let same = d.clone();
        assert!(!d.move_panels(&[Program], 0, Program, Zone::Left, None));
        assert!(!d.move_panels(&[Program], 0, LumetriColor, Zone::Left, None));
        assert!(!d.move_panels(&[], 0, Program, Zone::Left, None));
        assert_eq!(d, same);
    }

    #[test]
    fn edges_of_the_dock_area_take_full_columns_and_rows() {
        let mut d = workspace("Editing");
        assert!(d.move_to_edge(&[Effects], 0, Zone::Left));
        let DockNode::Split { vertical: false, a, .. } = &d else { panic!("{d:?}") };
        assert_eq!(**a, tabs(&[Effects], 0));
        assert!(d.move_to_edge(&[History], 0, Zone::Bottom));
        let DockNode::Split { vertical: true, b, .. } = &d else { panic!("{d:?}") };
        assert_eq!(**b, tabs(&[History], 0));
        // docking a floating panel back into an empty dock
        let mut e = DockNode::Tabs { panels: vec![], active: 0 };
        assert!(e.is_empty());
        assert!(e.move_to_edge(&[Program], 0, Zone::Right));
        assert_eq!(e, tabs(&[Program], 0));
    }

    #[test]
    fn drop_targets_follow_the_pointer() {
        let a = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
        let ga = Group {
            path: "a".into(),
            rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 600.0)),
            content: Rect::from_min_max(pos2(0.0, 30.0), pos2(500.0, 600.0)),
            panels: vec![Source, EffectControls],
            active: 0,
        };
        let gb = Group {
            path: "b".into(),
            rect: Rect::from_min_max(pos2(504.0, 0.0), pos2(1000.0, 600.0)),
            content: Rect::from_min_max(pos2(504.0, 30.0), pos2(1000.0, 600.0)),
            panels: vec![Program],
            active: 0,
        };
        let groups = [ga, gb];
        let strips = vec![(
            "a".to_string(),
            vec![(Source, Rect::from_min_max(pos2(10.0, 0.0), pos2(80.0, 30.0))), (EffectControls, Rect::from_min_max(pos2(90.0, 0.0), pos2(200.0, 30.0)))],
        )];
        let at = |x: f32, y: f32, moving: &[PanelKind]| drop_target(&groups, a, pos2(x, y), 30.0, &strips, moving);
        // centre of Program, its left quarter, its top quarter
        assert!(matches!(at(750.0, 300.0, &[Source]), Some(DropTarget::Group { target: Program, zone: Zone::Center, .. })));
        assert!(matches!(at(530.0, 300.0, &[Source]), Some(DropTarget::Group { target: Program, zone: Zone::Left, .. })));
        assert!(matches!(at(750.0, 60.0, &[Source]), Some(DropTarget::Group { target: Program, zone: Zone::Top, .. })));
        // the tab strip: between the two tabs, after the last
        assert!(matches!(at(85.0, 10.0, &[Program]), Some(DropTarget::Group { zone: Zone::Center, index: Some(1), .. })));
        assert!(matches!(at(400.0, 10.0, &[Program]), Some(DropTarget::Group { zone: Zone::Center, index: Some(2), .. })));
        // the outer edge of the dock area
        assert!(matches!(at(4.0, 300.0, &[Program]), Some(DropTarget::Edge { zone: Zone::Left, .. })));
        assert!(matches!(at(996.0, 300.0, &[Source]), Some(DropTarget::Edge { zone: Zone::Right, .. })));
        // a group over itself, a single panel over its own group: nothing
        assert_eq!(at(750.0, 300.0, &[Program]), None);
        assert_eq!(at(250.0, 300.0, &[Source, EffectControls]), None);
        // one tab of a group over the rest of it: beside it
        assert!(matches!(at(20.0 + 14.0, 300.0, &[Source]), Some(DropTarget::Group { target: EffectControls, zone: Zone::Left, .. })));
        assert_eq!(at(1200.0, 300.0, &[Source]), None);
    }

    #[test]
    fn a_panel_remembers_its_place() {
        let d = workspace("Editing");
        // in a shared group: back into it
        assert_eq!(d.home_of(Effects), Some((Project, Zone::Center)));
        // alone: beside its neighbour, on its side
        assert_eq!(d.home_of(Program), Some((Source, Zone::Right)));
        assert_eq!(d.home_of(AudioMeters), Some((Timeline, Zone::Right)));
        // closing then putting it back there restores the layout
        let mut e = d.clone();
        e.close(Program);
        assert!(e.move_panels(&[Program], 0, Source, Zone::Right, None));
        assert_eq!(e, d);
        assert_eq!(d.home_of(LumetriColor), None);
    }
}
