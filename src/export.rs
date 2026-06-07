//! Exports d'un encounter : ligne chat EQ2, Markdown, CSV, JSON.

use crate::combat::{fmt_duration, fmt_f64, fmt_num, Encounter};

/// Une ligne compacte à coller dans le chat du jeu (limite ~250 caractères).
pub fn chat_line(enc: &Encounter) -> String {
    let mut out = format!("{} ({})", enc.title(), fmt_duration(enc.duration()));
    for (name, c) in enc.damage_ranking() {
        let part = format!(" | {} {}", short_name(name), fmt_f64(enc.dps_of(c)));
        if out.len() + part.len() > 240 {
            break;
        }
        out.push_str(&part);
    }
    out
}

/// Prénom seul pour compacter la ligne chat (les PJ EQ2 n'ont qu'un mot de toute façon).
fn short_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// Tableau Markdown complet (dégâts + soins).
pub fn markdown(enc: &Encounter) -> String {
    let mut out = format!(
        "## {} — {} (total {})\n\n",
        enc.title(),
        fmt_duration(enc.duration()),
        fmt_num(enc.total_damage())
    );
    out.push_str("| # | Nom | Dégâts | DPS | % | Crit % | Max hit | Hits |\n");
    out.push_str("|---|-----|--------|-----|---|--------|---------|------|\n");
    let total = enc.total_damage().max(1);
    for (i, (name, c)) in enc.damage_ranking().iter().enumerate() {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:.1} | {:.1} | {} | {} |\n",
            i + 1,
            name,
            fmt_num(c.damage),
            fmt_f64(enc.dps_of(c)),
            c.damage as f64 / total as f64 * 100.0,
            c.crit_rate(),
            fmt_num(c.max_hit),
            c.hits
        ));
    }
    let heals = enc.heal_ranking();
    if !heals.is_empty() {
        out.push_str("\n### Soins\n\n| Nom | Soins | HPS |\n|-----|-------|-----|\n");
        for (name, c) in heals {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                name,
                fmt_num(c.healing),
                fmt_f64(enc.hps_of(c))
            ));
        }
    }
    let power = enc.power_ranking();
    if !power.is_empty() {
        out.push_str("\n### Power replenish\n\n| Nom | Power | Power/s |\n|-----|-------|---------|\n");
        for (name, c) in power {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                name,
                fmt_num(c.power),
                fmt_f64(enc.pps_of(c))
            ));
        }
    }
    out
}

/// CSV : une ligne par combattant.
pub fn csv(enc: &Encounter) -> String {
    let mut out = String::from(
        "name,damage,dps,damage_pct,crit_pct,max_hit,hits,healing,hps,power,damage_taken,deaths,kills\n",
    );
    let total = enc.total_damage().max(1);
    let mut all: Vec<_> = enc.combatants.iter().collect();
    all.sort_by(|a, b| b.1.damage.cmp(&a.1.damage));
    for (name, c) in all {
        out.push_str(&format!(
            "\"{}\",{},{:.1},{:.2},{:.2},{},{},{},{:.1},{},{},{},{}\n",
            name.replace('"', "\"\""),
            c.damage,
            enc.dps_of(c),
            c.damage as f64 / total as f64 * 100.0,
            c.crit_rate(),
            c.max_hit,
            c.hits,
            c.healing,
            enc.hps_of(c),
            c.power,
            c.damage_taken,
            c.deaths,
            c.kills
        ));
    }
    out
}

/// JSON complet (combatants, abilities, séries temporelles).
pub fn json(enc: &Encounter) -> String {
    serde_json::to_string_pretty(enc).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatEngine;
    use crate::parser::Parser;

    fn sample() -> Encounter {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        for l in [
            "(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage.",
            "(1002)[Tue May 26 17:42:28 2026] Wizzy's Fusion hits a rat for a critical of 5,000 heat damage.",
            "(1003)[Tue May 26 17:42:29 2026] Healer's Salve heals YOU for 50 hit points.",
        ] {
            engine.process(&parser.parse_line(l).unwrap());
        }
        engine.current.take().unwrap()
    }

    #[test]
    fn chat_line_compact() {
        let enc = sample();
        let line = chat_line(&enc);
        assert!(line.starts_with("a rat (0:03)"));
        assert!(line.contains("Wizzy"));
        assert!(line.contains("Pawkod"));
        assert!(line.len() <= 250);
    }

    #[test]
    fn markdown_has_tables() {
        let enc = sample();
        let md = markdown(&enc);
        assert!(md.contains("| 1 | Wizzy |"));
        assert!(md.contains("### Soins"));
        assert!(md.contains("| Healer |"));
    }

    #[test]
    fn csv_and_json_well_formed() {
        let enc = sample();
        let c = csv(&enc);
        assert!(c.lines().count() >= 3); // header + 2+ combattants
        let j = json(&enc);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(v["combatants"]["Wizzy"]["damage"].as_u64() == Some(5000));
    }
}
