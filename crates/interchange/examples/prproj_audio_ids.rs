//! Identify Premiere's audio effects (saved by GUID only) by their parameter names (development
//! tool, not shipped): `cargo run --release -p filmcraft-interchange --example prproj_audio_ids -- KIT.prproj [OTHER.prproj…]`
//!
//! Prints each GUID with its parameter names, a display name where some project carries one, and
//! the FilmCraft audio effect whose parameter labels match best.

fn main() {
    let mut guids: std::collections::BTreeMap<String, (Vec<String>, Option<String>)> = std::collections::BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        for (guid, names, label) in filmcraft_interchange::prproj_audio_components(&bytes) {
            let e = guids.entry(guid).or_insert((names.clone(), None));
            if e.0.is_empty() {
                e.0 = names;
            }
            if label.is_some() {
                e.1 = label;
            }
        }
    }
    let defs: Vec<_> = filmcraft_project::effect_defs().iter().filter(|d| d.kind == filmcraft_project::EffectKind::Audio).collect();
    for (guid, (names, label)) in guids {
        let set: std::collections::BTreeSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let mut best: Vec<(f64, &str, &str)> = defs
            .iter()
            .map(|d| {
                let labels: std::collections::BTreeSet<String> = d.params.iter().map(|p| p.label.to_lowercase()).collect();
                let inter = set.intersection(&labels).count() as f64;
                let union = set.union(&labels).count().max(1) as f64;
                (inter / union, d.id, d.name)
            })
            .collect();
        best.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top: Vec<String> = best.iter().take(2).map(|(s, id, n)| format!("{n} ({id}) {s:.2}")).collect();
        println!("{guid}\t{}\t[{}]\t{}", label.as_deref().unwrap_or("-"), names.join(", "), top.join(" | "));
    }
}
