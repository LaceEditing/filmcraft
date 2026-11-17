# egui-winit 0.37.0, patched for FilmCraft

Source: <https://github.com/storytold/egui/tree/6d518fc799681571963842f95b55a6b68431b15a/crates/egui-winit>,
the commit the workspace pins the rest of the egui family to (a fork of
<https://github.com/emilk/egui>; MIT OR Apache-2.0, © Emil Ernerfeldt and the egui contributors;
`LICENSE-MIT`, `LICENSE-APACHE`). Used through `[patch.crates-io]` in the workspace `Cargo.toml`
in place of that commit's own egui-winit.

`Cargo.toml` is `Cargo.toml.orig` with the egui workspace's inherited values written out (the
versions `cargo metadata` resolves for that commit); the lints table, the example and its
dev-dependency are left out.

The one change (`src/lib.rs`, marked "FilmCraft patch"): egui-winit discards winit's key location,
so the numeric keypad's keys arrive as the main keys (Keypad − is `Key::Minus`, Keypad 5 is
`Key::Num5`) and cannot be bound apart from them. Here keypad key events, presses and releases,
carry `physical_key: Some(Key::F35)` as a marker (`State::on_keyboard_input`,
`State::release_keyboard_input`, `is_numpad_key`), and keypad `*` (which egui has no key for)
arrives as `Key::F34`. The logical key is unchanged, so everything that ignores the marker behaves
as before. FilmCraft reads the marker in `crates/ui-egui` (`menus::KEYPAD_MARKER`) to offer the
Keypad keys in Edit ▸ Keyboard Shortcuts.

To update egui: when the workspace's egui pin moves, copy that commit's `crates/egui-winit` over
this directory, write out `Cargo.toml` again and re-apply the patch (or drop the directory once
egui reports key locations itself).
