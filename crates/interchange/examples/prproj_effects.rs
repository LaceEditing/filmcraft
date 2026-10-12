//! The effects Premiere projects use, and whether FilmCraft can model a clip on each (development
//! tool, not shipped): `cargo run --release -p filmcraft-interchange --example prproj_effects -- FILE.prproj…`

fn main() {
    let mut seen: std::collections::BTreeMap<String, (usize, String)> = std::collections::BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        for (label, verdict) in filmcraft_interchange::prproj_effect_census(&bytes) {
            let e = seen.entry(label).or_insert((0, verdict));
            e.0 += 1;
        }
    }
    for (label, (n, verdict)) in seen {
        println!("{n:>6}  {label}  →  {verdict}");
    }
}
