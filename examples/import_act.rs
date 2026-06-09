//! Import hors-ligne d'une config ACT : fusionne ses SpellTimers dans la base de
//! mécaniques embarquée (`assets/mechanics.json`) et exporte ses triggers
//! convertis dans `assets/act_triggers.json` (pack importable).
//!
//! Usage :
//!   cargo run --release --example import_act -- <config_ACT.xml> [--write]

#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/mechanics.rs"]
mod mechanics;
#[path = "../src/optimizer.rs"]
mod optimizer;
#[path = "../src/combat.rs"]
mod combat;
#[path = "../src/triggers.rs"]
mod triggers;
#[path = "../src/act_import.rs"]
mod act_import;

use mechanics::MechanicsDb;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let xml_path = args.next().expect("usage: import_act <config_ACT.xml> [--write]");
    let write = args.any(|a| a == "--write");

    let xml = std::fs::read_to_string(&xml_path).expect("lecture du XML ACT");
    let res = act_import::parse_act_xml(&xml);
    println!(
        "Converti : {} mécaniques (SpellTimers), {} triggers (CustomTriggers)",
        res.mechanics.len(),
        res.triggers.len()
    );

    // Aperçu : quelques mécaniques chronométrées.
    let mut timed: Vec<_> = res.mechanics.iter().filter(|m| m.is_timed()).collect();
    timed.sort_by(|a, b| a.mob.cmp(&b.mob));
    println!("\nExemples de timers de boss :");
    for m in timed.iter().take(15) {
        println!(
            "  {:<26} {:>4.0}s (avance {:>2}s)  [{}]",
            m.ability, m.period, m.lead, m.mob
        );
    }
    println!("  … {} mécaniques chronométrées au total", timed.len());

    if !write {
        println!("\n(relancer avec --write pour fusionner dans assets/)");
        return;
    }

    // Fusionne dans la base de mécaniques existante (mined + ACT).
    let mech_path = PathBuf::from("assets/mechanics.json");
    let mut db = std::fs::read_to_string(&mech_path)
        .ok()
        .and_then(|s| MechanicsDb::from_str(&s))
        .unwrap_or_default();
    let before = db.entries.len();
    let mut add = MechanicsDb::default();
    add.entries = res.mechanics.clone();
    db.merge(&add, false); // n'écrase pas les entrées existantes (ex. minées)
    db.save_to(&mech_path);
    println!(
        "\n✓ mécaniques : {before} → {} dans {}",
        db.entries.len(),
        mech_path.display()
    );

    // Exporte les triggers convertis en pack importable.
    let trig_path = PathBuf::from("assets/act_triggers.json");
    if let Ok(json) = serde_json::to_string_pretty(&res.triggers) {
        let _ = std::fs::write(&trig_path, json);
        println!("✓ {} triggers écrits dans {}", res.triggers.len(), trig_path.display());
    }
}
