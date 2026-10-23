# Keyboard parity with Premiere Pro

FilmCraft's **Premiere Pro Compatible** shortcut preset (Edit ▸ Keyboard Shortcuts…) follows
Premiere's default keyboard (macOS). The table lives in `crates/engine/src/shortcut_presets.rs`:
`PREMIERE` (application-wide keys and a few panel keys) and `PREMIERE_PANEL` (panel-scoped keys
that FilmCraft Default uses as well). `presets::premiere()` is the whole preset.

Every entry names a FilmCraft command id. Engine commands run through `Session::execute`; UI
commands (frames, panels, monitor zoom, Project and Text panel navigation) run through
`menus::invoke`. Both are reachable by id from the CLI (`filmcraft-cli exec <id>`), the control
channel (`engine.execute` / `ui.menu.invoke`) and MCP (`command_run`).

Tests: `crates/engine/src/keyboard_tests.rs` (no key used twice in one scope; behaviour of the
keyboard commands, with undo) and `crates/ui-egui/tests/it/keyboard_ui.rs` (no dangling ids in the
tables, no conflicts in FilmCraft Default or the Premiere preset, keys pressed in the headless app).

## Keyboard-only commands (M3.12)

Premiere has these as keys only (most are not in a menu). Modules: `filmcraft_engine::keyboard`
(engine) and `filmcraft_ui_egui::panels::keyboard` (UI); their module docs list each id.

| Group | Commands | Keys |
|---|---|---|
| Navigation | Go to Next/Previous Edit Point (targeted tracks), …on Any Track, Go to Selected Clip Start/End, Reveal Nested Sequence | Down/Up, Shift+Down/Up, Shift+Home/End, Cmd+Alt+F |
| Selection | Select Clip at Playhead, Select Next/Previous Clip, Select Find Box, Open Search | D, Cmd+Down/Up, Shift+F, Cmd+Shift+F |
| Trimming | Extend Previous/Next Edit To Playhead; Nudge Clip Selection Left/Right One/Five Frames, Up/Down; Slip and Slide Clip Selection Left/Right One/Five Frames | Shift+Q/W; Cmd+(Shift+)Left/Right, Alt+Up/Down; Cmd+Alt+(Shift+)Left/Right; Alt+(Shift+), / . (Timeline) |
| Targeting | Toggle All Video/Audio Targets, Toggle All Source Video/Audio; Toggle Target Video/Audio 1–8, Move All Video/Audio Targets Up/Down, Toggle Mute/Solo for targeted audio, Toggle Track Output for targeted video (unbound, as in Premiere) | Cmd+0/9, Cmd+Alt+0/9 |
| Tracks | Expand/Minimize All Tracks (again restores), Increase/Decrease Video/Audio Tracks Height, Show Next/Previous Screen | Shift+= / Shift+-, Cmd+= / Cmd+- / Alt+= / Alt+-, PageDown/PageUp (Timeline) |
| Monitors | Zoom Program/Source Monitor to 100%/Fit, Toggle Full Screen, Play In to Out with Preroll/Postroll, Play from Playhead to Out Point, Export Frame, Set/Clear Poster Frame | Cmd+Shift+1/0, Cmd+Alt+Shift+1/0, Ctrl+`, Shift+Space, Ctrl+Space, Shift+E, Cmd+P / Alt+P |
| Panels | Maximize or Restore Active Frame / Frame Under Cursor, Select Next/Previous Panel, Toggle Source/Program Monitor Focus, Workspaces 7–9 | Shift+`, `, Ctrl+Shift+. / ,, Alt+Shift+7/8/9 |
| Audio | Increase/Decrease Clip Volume (1 dB) and …Many (Settings ▸ Audio ▸ Large Volume Adjustment), Nudge Volume ±1/±3 dB, Toggle Audio During Scrubbing | ] / [, Shift+] / Shift+[, Shift+S |
| Graphics | Increase/Decrease Font Size and Leading by One/Five Units, Left/Center/Right align text, Begin Text Editing, Nudge Selected Object by one/five (graphic layers, else Motion position) | Cmd+Alt+(Shift+)Left/Right, Alt+(Shift+)Up/Down, Cmd+Shift+L/C/R, Cmd+Alt+', Cmd+(Shift+)arrows (Program, Properties) |
| Project panel | List/Icon/Toggle View, Hover Scrub, Thumbnail Size Next/Previous (and = / -), Move/Extend Selection, Mark In / Out of the hover-scrubbed clip | Cmd+PageUp/PageDown, Shift+\, Shift+H, Shift+] / Shift+[, arrows, Home/End, PageUp/PageDown, I / O |
| Text panel | Navigate/Select to Previous/Next Word and Line, Start/End of Segment, Delete (lift), Ripple Delete (extract), Show Program Transcript, Merge/Split Segments | arrows, Shift+arrows, Home/End, Shift+Home, Cmd+Shift+Down, Alt+Backspace, Backspace, Shift+X, Alt+M / Alt+S |
| Other | Help (F1), Quit (Cmd+Q), Media Browser ▸ Open In Source Monitor (imports the file first), Send to Media Encoder (= Send to Export Queue, Alt+Shift+M), Metadata panel Play/Loop | |

Existing commands that only lacked their Premiere key were added to the table too (Close, Close
Project, Import from Media Browser, Get Media File Properties, Paste Attributes, Find, Edit
Original, Make Subclip, Audio Channels, the Graphics New Layer / Arrange / Select Layer commands,
Program monitor Show Rulers / Show Guides / Snap / Lock Guides, New Bin From Selection, Paste and
Paste Insert in the Timeline, the Audio Track Mixer Loop).

FilmCraft Default keeps its own keys where they differ (Shift+E is Clip ▸ Enable there, so Export
Frame has no default key; the Premiere preset moves Shift+E to Export Frame and Enable to ⇧⌘E).

## Skipped Premiere default shortcuts

| Premiere command (key, scope) | Reason |
|---|---|
| Generative Media Tool (⇧V) | Adobe Firefly cloud generation; not part of FilmCraft. |
| Add to object / Subtract from object (⌘= / ⌘-) | Refine an AI object mask (Object Mask tool); FilmCraft has no AI object masks. |
| Change Draw Mode (⌥⌘L) | Premiere's shape draw modes for the Pen/shape tools; FilmCraft's shape tools have no draw modes. |
| Production panel: New Project, New Folder, Close Project, Make a Copy, Move Selection Home/End/Page Up/Page Down, Move To Trash, Open Project, Zoom In/Out (12 keys) | Productions (shared multi-project folders) are not implemented; there is no Production panel. |
| Search panel: Open in Source Monitor (⇧O) | No separate Search panel; Open Search (⇧⌘F) focuses the Project panel search. |
| Audio Track Mixer: Show/Hide Tracks… (⌥⌘T), Meter Input(s) Only (⌃⇧I) | The mixer has no track visibility dialog and no input metering (no hardware input recording). |
| Effect Controls: Remove Selected Effect (Delete), Loop During Audio-Only Playback (⌘L) | Effect Controls has no effect selection state and no audio-only playback mode; effects are removed from their context menu or `effects.remove`. |
| Effects panel: New Custom Bin (⌘/), Delete Custom Item (Delete) | The Effects panel has no custom bins (effect presets live in the Presets bin). |
| History panel: Delete (Delete) | History states cannot be selected and deleted individually. |
| Media Browser: Select Directory List / Select Media List (⇧← / ⇧→) | The directory tree and media list take no separate keyboard focus. |
| Project panel: Delete Selection with Options (⌘Delete) | Needs the "delete instances in sequences" dialog; plain Clear (Backspace) deletes. |
| Project panel: Next/Previous Column Field, Next/Previous Row Field (Tab, ⇧Tab, Return, ⇧Return) | The list view has no inline-editable metadata cells. |
| Text panel: Edit Segment (Return) | No inline transcript text editing (corrections go through `transcript.set`). |
| Text panel: Follow Active Monitor (⇧C), Show Source Transcript (⇧Z) | Only sequence transcripts exist; source clips have no transcript view. |
| Timeline: Set Work Area Bar In/Out Point (⌥[ / ⌥]) | FilmCraft has no work area bar (Premiere hides it by default; renders use In/Out). |

## How keys reach commands

- **Order.** The focused panel's keys and the application keys are tried together, most modifiers
  first (a press also matches a binding with fewer modifiers, since `Shift+;` arrives as `:`), and
  on the same keys the panel's binding wins: in the Timeline, `Cmd+Shift+W` is Close Project and
  `Cmd+W` is Close Sequence.
- **Ctrl off macOS.** On Windows and Linux, `Ctrl` is the command key, so a binding written with
  the macOS Control key (`Ctrl+M`, Add Range Marker to In and Out) lands on the same keys as a
  `Cmd` binding (`Cmd+M`, Export). The `Cmd` binding runs, as the shortcut editor resolves it.
- **Clear acts on the focused panel.** Edit ▸ Clear (`edit.clear`) and the Project panel's Clear
  (`project.delete`) are one key in Premiere: whichever of the two a key is bound to, it deletes
  project items when the Project panel has focus and clears timeline clips everywhere else, so a
  project item selected out of sight is never deleted from the Timeline.
- **Gaps.** Clicking empty track space between clips (or before the first clip) selects the gap
  (`timeline.selectGap`); with no clips selected, Clear and Ripple Delete close it, and
  right-clicking it offers Ripple Delete. Clips on other sync-locked tracks inside the gap keep it
  open (turn their Sync Lock off).
- **Delete keys.** Clear is `Backspace` and `Delete`; Ripple Delete is `Shift+Delete`, and
  `Alt+Backspace` in the Timeline (Premiere's keys).
- **Numeric keypad.** Keypad keys are keys of their own (`KeypadMinus`, `KeypadPlus`,
  `Keypad0`–`Keypad9`, `KeypadMultiply`, `KeypadDivide`, `KeypadDecimal`, `KeypadEnter`), shown as
  "Keypad −" etc. and recordable in the shortcut editor. Keypad + and − zoom the timeline by
  default; a keypad key with no binding of its own acts as the main key it types. egui does not tell
  the keypad apart, so FilmCraft builds with a patched egui-winit (`third_party/egui-winit`, see its
  `FILMCRAFT-PATCH.md`).

Tests: `crates/ui-egui/tests/timeline_keys_ui.rs` presses every application and Timeline key of
FilmCraft Default in the headless app and checks the command each one runs; it also covers gaps,
the focused-panel Clear, the keypad and recording keys in the shortcut editor.
`crates/ui-egui/tests/timeline_gestures_ui.rs` drives the timeline tools with the mouse (select,
marquee, move, Cmd-drag insert, Alt-drag copy, trim, ripple, roll, razor, slip, slide, rate stretch,
track select, ruler scrub) and the core editing keys.
