//! Minage hors-ligne des mécaniques : passe sur TOUS les logs d'un répertoire,
//! apprend les capacités ennemies récurrentes/impactantes et agrège les
//! observations entre personnages (plusieurs membres loggant le même boss).
//!
//! Usage :
//!   cargo run --release --example mine_mechanics -- <dossier_logs> [--write]
//!
//! `--write` écrit les mécaniques chronométrées dans `assets/mechanics.json`
//! (la base communautaire embarquée).

#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/mechanics.rs"]
mod mechanics;
#[path = "../src/optimizer.rs"]
mod optimizer;
#[path = "../src/combat.rs"]
mod combat;

use combat::{fmt_num, CombatEngine};
use mechanics::{MechSource, MechanicsDb};
use parser::{char_name_from_path, Parser};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

fn collect_logs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_logs(&p, out);
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("eq2log_") && name.ends_with(".txt") {
                out.push(p);
            }
        }
    }
}

fn mine_file(path: &Path) -> MechanicsDb {
    let name = char_name_from_path(path).unwrap_or_else(|| "You".into());
    let parser = Parser::new(name.clone());
    let mut engine = CombatEngine::new(6);
    engine.self_name = name;
    // Minage à froid : on ignore la base embarquée/locale pour repartir de zéro
    // (sinon les mécaniques déjà embarquées, en source Bundled, seraient exclues
    // du résultat et le minage se « viderait » build après build).
    engine.mech.db.entries.clear();

    let Ok(file) = std::fs::File::open(path) else {
        return MechanicsDb::default();
    };
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(pl) = parser.parse_line(line) {
            engine.process(&pl);
        }
    }
    engine.tick(u64::MAX - 100); // clôt le dernier encounter → apprentissage
    // On ne renvoie que les entrées apprises (la base embarquée est vide ici).
    let mut db = MechanicsDb::default();
    for e in &engine.mech.db.entries {
        if matches!(e.source, MechSource::Learned) {
            db.entries.push(e.clone());
        }
    }
    db
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .unwrap_or_else(|| r"X:\jeux\steam\steamapps\common\EverQuest 2\logs".to_string());
    let write = args.any(|a| a == "--write");
    let root = PathBuf::from(root);

    let mut logs = Vec::new();
    collect_logs(&root, &mut logs);
    logs.sort();
    println!("Logs trouvés : {}", logs.len());

    let start = std::time::Instant::now();
    let mut global = MechanicsDb::default();
    for path in &logs {
        let db = mine_file(path);
        let n = db.entries.len();
        global.absorb_db(&db);
        println!(
            "  {:<48} {} mécaniques",
            path.file_name().unwrap().to_string_lossy(),
            n
        );
    }
    println!("Durée minage : {:?}\n", start.elapsed());

    // Tri par impact.
    global
        .entries
        .sort_by(|a, b| b.impact_score().partial_cmp(&a.impact_score()).unwrap());

    let noteworthy: Vec<_> = global.entries.iter().filter(|e| e.is_noteworthy()).collect();
    let timed: Vec<_> = noteworthy.iter().filter(|e| e.is_timed()).collect();
    println!(
        "Mécaniques apprises : {} (marquantes : {}, chronométrées : {})\n",
        global.entries.len(),
        noteworthy.len(),
        timed.len()
    );

    println!(
        "{:<30} {:<7} {:<11} {:>6} {:>4} {:>5} {:>6} {:>4} {:>4}  {}",
        "Capacité", "Type", "Zone", "Pér.", "Éch.", "Cibl", "Max", "Let", "Cast", "Mob"
    );
    println!("{}", "-".repeat(112));
    for e in noteworthy.iter().take(60) {
        let zone = if e.zone.is_empty() { "(globale)" } else { &e.zone };
        let period = if e.is_timed() {
            format!("{:.0}s", e.period)
        } else {
            "-".into()
        };
        println!(
            "{:<30} {:<7} {:<11} {:>6} {:>4} {:>5} {:>6} {:>4} {:>4}  {}",
            truncate(&e.ability, 30),
            e.kind.label(),
            truncate(zone, 11),
            period,
            e.samples,
            e.max_targets,
            fmt_num(e.max_hit),
            e.lethal,
            e.casts_seen,
            truncate(&e.mob, 26),
        );
    }

    if write {
        // On n'embarque que les mécaniques chronométrées (exploitables en compte à rebours).
        let mut bundle = MechanicsDb::default();
        for e in &global.entries {
            // Seuil de confiance pour la base communautaire : chronométrée, marquante
            // et assez d'échantillons concordants (on évite le bruit de bas niveau).
            if e.is_timed() && e.is_noteworthy() && e.samples >= 5 {
                let mut e = e.clone();
                e.source = MechSource::Bundled;
                bundle.entries.push(e);
            }
        }
        // Fusion (et non écrasement) dans la base existante : on préserve les
        // entrées déjà présentes (ex. importées d'ACT) et on cumule les minées.
        let path = PathBuf::from("assets/mechanics.json");
        let mut db = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| MechanicsDb::from_str(&s))
            .unwrap_or_default();
        let before = db.entries.len();
        db.absorb_db(&bundle);
        db.save_to(&path);
        println!(
            "\n✓ {} mécaniques minées (confiance ≥5) fusionnées : {before} → {} dans {}",
            bundle.entries.len(),
            db.entries.len(),
            path.display()
        );
    } else {
        println!("\n(relancer avec --write pour générer assets/mechanics.json)");
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max - 1).collect();
        format!("{t}…")
    }
}
