//! Persistance de l'historique des encounters, par personnage/serveur.
//! Fichiers JSON dans `history/` à côté de l'exécutable.

use crate::combat::Encounter;
use std::path::PathBuf;

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn default_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("history")))
        .unwrap_or_else(|| PathBuf::from("history"))
}

pub fn file_for(server: &str, character: &str) -> PathBuf {
    default_dir().join(format!("{}_{}.json", sanitize(server), sanitize(character)))
}

pub fn load_from(path: &PathBuf) -> Vec<Encounter> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn load(server: &str, character: &str) -> Vec<Encounter> {
    load_from(&file_for(server, character))
}

pub fn save_to(path: &PathBuf, history: &[Encounter], cap: usize) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // On ne garde que les `cap` derniers encounters.
    let slice = if history.len() > cap {
        &history[history.len() - cap..]
    } else {
        history
    };
    if let Ok(json) = serde_json::to_string(slice) {
        let _ = std::fs::write(path, json);
    }
}

pub fn save(server: &str, character: &str, history: &[Encounter], cap: usize) {
    save_to(&file_for(server, character), history, cap);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatEngine;
    use crate::parser::Parser;

    #[test]
    fn roundtrip() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        for l in [
            "(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage.",
            "(1001)[Tue May 26 17:42:27 2026] a rat hits YOU for 10 crushing damage.",
            "(1002)[Tue May 26 17:42:28 2026] Pawkod has been slain by a rat!",
        ] {
            engine.process(&parser.parse_line(l).unwrap());
        }
        engine.tick(u64::MAX - 100);
        assert_eq!(engine.history.len(), 1);

        let tmp = std::env::temp_dir().join("eq2_tools_test_history.json");
        save_to(&tmp, &engine.history, 500);
        let loaded = load_from(&tmp);
        let _ = std::fs::remove_file(&tmp);

        assert_eq!(loaded.len(), 1);
        let e = &loaded[0];
        assert_eq!(e.start, 1000);
        assert_eq!(e.combatants["Pawkod"].damage, 100);
        assert_eq!(e.combatants["Pawkod"].deaths, 1);
        // Le death report et les arêtes survivent au roundtrip.
        assert_eq!(e.deaths_log.len(), 1);
        assert_eq!(e.deaths_log[0].victim, "Pawkod");
        assert_eq!(e.deaths_log[0].killer, "a rat");
        assert_eq!(e.deaths_log[0].hits.len(), 1);
        assert!(e.attacks["Pawkod"].contains("a rat"));
        // Les séries temporelles aussi (clés u64 → JSON string → u64).
        assert_eq!(e.combatants["Pawkod"].dmg_series.get(&1000), Some(&100));
    }

    #[test]
    fn cap_keeps_most_recent() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(2);
        for i in 0..5u64 {
            let t = 1000 + i * 100;
            let line = format!(
                "({t})[Tue May 26 17:42:26 2026] YOU hit a rat for {} crushing damage.",
                100 + i
            );
            engine.process(&parser.parse_line(&line).unwrap());
            engine.tick(t + 50);
        }
        assert_eq!(engine.history.len(), 5);
        let tmp = std::env::temp_dir().join("eq2_tools_test_cap.json");
        save_to(&tmp, &engine.history, 2);
        let loaded = load_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].start, 1400); // les plus récents
    }
}
