//! Valide l'optimiseur sur un vrai log : parse le fichier, profile les sorts
//! du perso, et affiche le classement par efficacité (scénario configurable).
//!
//! Usage : cargo run --release --example opt_report -- <log> [cibles] [linked]

#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/mechanics.rs"]
mod mechanics;
#[path = "../src/optimizer.rs"]
mod optimizer;
#[path = "../src/combat.rs"]
mod combat;

use optimizer::{diagnose, report, PlayerStats, Scenario, SpellDb};
use parser::{char_name_from_path, Parser};
use std::io::{BufRead, BufReader};

fn main() {
    let path = std::env::args().nth(1).expect("usage: opt_report <log> [cibles] [linked]");
    let targets: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let linked: bool = std::env::args().nth(3).map(|s| s != "0").unwrap_or(true);
    let path = std::path::PathBuf::from(path);
    let name = char_name_from_path(&path).unwrap_or_else(|| "You".into());
    let p = Parser::new(name.clone());
    let mut engine = combat::CombatEngine::new(6);
    engine.self_name = name.clone();

    let file = std::fs::File::open(&path).expect("open log");
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).unwrap() == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(pl) = p.parse_line(line) {
            engine.process(&pl);
        }
    }
    engine.prof.flush();

    let db = SpellDb::bundled();
    let obs = engine.prof.live(&name);
    let class = db.infer_class(obs.keys());
    let stats = PlayerStats::default();
    let sc = Scenario { targets, linked };
    let rows = report(
        &obs,
        &db,
        class.as_deref(),
        &stats,
        &sc,
        &Default::default(),
        &Default::default(),
        None,
        false,
    );

    println!(
        "Perso : {name} | classe devinée : {} | scénario : {targets} cible(s){}",
        class.as_deref().unwrap_or("?"),
        if linked { " liées" } else { "" }
    );
    println!(
        "{:<24} {:<8} {:>10} {:>6} {:>6} {:>7} {:>11} {:>11}",
        "Sort", "Type", "Dég/cast", "Cast", "Reuse", "Interv", "Eff/GCD", "DPS soutenu"
    );
    for r in rows.iter().take(30) {
        println!(
            "{:<24} {:<8} {:>10.0} {:>5.1}s {:>5.0} {:>6.1}s {:>11.0} {:>11.0}{}",
            trunc(&r.ability, 24),
            r.kind,
            r.dmg_per_cast,
            r.cast_eff,
            r.recast_eff.unwrap_or(0.0),
            r.interval,
            r.efficiency,
            r.sustained_dps,
            if r.from_db { "" } else { "  (inféré)" }
        );
    }

    let ct = engine.prof.combat_time(&name) as f64;
    let diag = diagnose(&rows, ct);
    println!("\n=== Diagnostic ===");
    println!(
        "Combat : {:.0}s | {} casts | {:.0}s à caster | activité GCD {:.0}% | faible rendement {:.0}%",
        diag.combat_time,
        diag.total_casts,
        diag.cast_time,
        diag.gcd_util * 100.0,
        diag.low_yield_frac * 100.0
    );
    println!("À mieux entretenir (DoT/cooldowns) :");
    for u in &diag.underused {
        println!(
            "  {:<24} entretien {:>3.0}% ({} / ~{:.0}) -> ~{:.0} dégâts en plus",
            trunc(&u.ability, 24),
            u.uptime * 100.0,
            u.casts,
            u.expected,
            u.lost_damage
        );
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}
