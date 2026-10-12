//! Which FilmCraft effects and transitions Premiere projects have layouts for (development tool,
//! not shipped).
//!
//! `cargo run --release -p filmcraft-interchange --example prproj_layouts -- FILE.prproj…`
//!
//! Prints the layouts found (with how many projects have each) and the FilmCraft effects none of
//! the projects has, which a project opened from Premiere cannot get when they are added.

use std::collections::BTreeMap;

fn main() {
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = 0;
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        files += 1;
        for l in filmcraft_interchange::prproj_layouts(&bytes, false) {
            *found.entry(l.key).or_insert(0) += 1;
        }
    }
    println!("== {files} projects, {} layouts", found.len());
    for (k, n) in &found {
        println!("  {n:>4}  {k}");
    }
    let mut missing: Vec<String> = filmcraft_project::effect_defs()
        .iter()
        .filter(|d| !d.intrinsic && matches!(d.kind, filmcraft_project::EffectKind::Video | filmcraft_project::EffectKind::Audio))
        .filter(|d| !filmcraft_project::graphic::is_layer_id(d.id))
        .filter(|d| !found.contains_key(&format!("effect.{}", d.id)))
        .map(|d| format!("{:?}  {}  ({})", d.kind, d.name, d.id))
        .collect();
    missing.sort();
    println!("== {} FilmCraft effects with no layout in these projects", missing.len());
    for m in missing {
        println!("  {m}");
    }
    let mut transitions: Vec<String> = filmcraft_project::effect_defs()
        .iter()
        .filter(|d| matches!(d.kind, filmcraft_project::EffectKind::VideoTransition | filmcraft_project::EffectKind::AudioTransition))
        .filter(|d| {
            let kind = if d.kind == filmcraft_project::EffectKind::AudioTransition { "audio" } else { "video" };
            !found.contains_key(&format!("transition.{kind}.{}", d.name.to_lowercase()))
        })
        .map(|d| format!("{:?}  {}  ({})", d.kind, d.name, d.id))
        .collect();
    transitions.sort();
    println!("== {} FilmCraft transitions with no layout in these projects", transitions.len());
    for t in transitions {
        println!("  {t}");
    }
}
