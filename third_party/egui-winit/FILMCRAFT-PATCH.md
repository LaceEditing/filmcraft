# egui-winit 0.36.2, patched for FilmCraft

Upstream: <https://github.com/emilk/egui/tree/main/crates/egui-winit> (MIT OR Apache-2.0,
© Emil Ernerfeldt and the egui contributors; `LICENSE-MIT`, `LICENSE-APACHE`). Used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

The one change (`src/lib.rs`, `State::on_keyboard_input`, marked "FilmCraft patch"): upstream
discards winit's key location, so the numeric keypad's keys arrive as the main keys (Keypad − is
`Key::Minus`, Keypad 5 is `Key::Num5`) and cannot be bound apart from them. Here keypad key events
carry `physical_key: Some(Key::F35)` as a marker, and keypad `*` (which egui has no key for)
arrives as `Key::F34`. The logical key is unchanged, so everything that ignores the marker behaves
as before. FilmCraft reads the marker in `crates/ui-egui` (`menus::KEYPAD_MARKER`) to offer the
Keypad keys in Edit ▸ Keyboard Shortcuts.

To update egui: copy the new egui-winit release over this directory and re-apply the patch (or
drop it once egui reports key locations itself).
