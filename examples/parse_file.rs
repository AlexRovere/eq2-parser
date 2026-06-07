//! Outil de validation : parse un fichier de log complet et affiche
//! la couverture du parser + les lignes "combat" non reconnues.
//!
//! Usage : cargo run --release --example parse_file -- <chemin_log>

#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/combat.rs"]
mod combat;

use parser::{char_name_from_path, LogEvent, Parser};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};

fn main() {
    let path = std::env::args().nth(1).expect("usage: parse_file <log>");
    let path = std::path::PathBuf::from(path);
    let name = char_name_from_path(&path).unwrap_or_else(|| "You".into());
    let p = Parser::new(name.clone());
    let mut engine = combat::CombatEngine::new(6);
    engine.self_name = name.clone();

    let file = std::fs::File::open(&path).expect("open log");
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();

    let mut total = 0u64;
    let mut parsed = 0u64;
    let mut by_kind: HashMap<&'static str, u64> = HashMap::new();
    let mut unmatched_combat: Vec<String> = Vec::new();

    let start = std::time::Instant::now();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).unwrap();
        if n == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        total += 1;
        if let Some(pl) = p.parse_line(line) {
            if let Some(ev) = &pl.event {
                parsed += 1;
                let kind = match ev {
                    LogEvent::Damage { .. } => "damage",
                    LogEvent::EnvDamage { .. } => "env_damage",
                    LogEvent::FailedHit { .. } => "failed_hit",
                    LogEvent::Miss { .. } => "miss",
                    LogEvent::Heal { .. } => "heal",
                    LogEvent::PowerRefresh { .. } => "power",
                    LogEvent::WardApplied { .. } => "ward",
                    LogEvent::Absorb { .. } => "absorb",
                    LogEvent::Threat { .. } => "threat",
                    LogEvent::Kill { .. } => "kill",
                    LogEvent::Slain { .. } => "slain",
                    LogEvent::StartFight | LogEvent::StopFight => "fight_state",
                    LogEvent::PetSendAttack => "pet_send",
                    LogEvent::PlayerSeen { .. } => "player_seen",
                    LogEvent::ZoneEnter { .. } => "zone",
                };
                *by_kind.entry(kind).or_default() += 1;
                engine.process(&pl);
            } else {
                // Heuristique : lignes qui ressemblent à du combat mais non parsées
                let m = &pl.message;
                if (m.contains(" damage.") || m.contains(" heals ") || m.contains(" hit points"))
                    && !m.starts_with("\\a")
                    && m != "Your skin heals as your scabs fade away."
                    && unmatched_combat.len() < 30
                {
                    unmatched_combat.push(m.to_string());
                }
            }
        }
    }
    let elapsed = start.elapsed();

    println!("Personnage : {name}");
    println!("Lignes     : {total} (parsées combat : {parsed})");
    println!("Durée      : {elapsed:?} ({:.0} lignes/s)", total as f64 / elapsed.as_secs_f64());
    println!("\nPar type d'événement :");
    let mut kinds: Vec<_> = by_kind.iter().collect();
    kinds.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (k, n) in kinds {
        println!("  {k:<12} {n}");
    }
    engine.tick(u64::MAX - 100); // clôt le dernier encounter
    println!("\nEncounters détectés : {}", engine.history.len());
    let mut top: Vec<_> = engine.history.iter().collect();
    top.sort_by_key(|e| std::cmp::Reverse(e.total_damage()));
    println!("Top 5 par dégâts totaux :");
    for e in top.iter().take(5) {
        println!(
            "  {} — {} dmg en {} ({} combattants)",
            e.title(),
            combat::fmt_num(e.total_damage()),
            combat::fmt_duration(e.duration()),
            e.combatants.len()
        );
    }
    if !engine.auto_pets.is_empty() {
        println!("\nPets auto-détectés :");
        for (pet, owner) in &engine.auto_pets {
            println!("  🐾 {pet} → {owner}");
        }
    }
    if !unmatched_combat.is_empty() {
        println!("\n⚠ Lignes 'combat' non reconnues (échantillon) :");
        for l in &unmatched_combat {
            println!("  {l}");
        }
    } else {
        println!("\n✓ Aucune ligne de combat non reconnue.");
    }
}
