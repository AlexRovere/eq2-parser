//! Mini-moteur de template pour le texte custom de l'overlay.
//!
//! Syntaxe : `{{variable}}` ou `{{variable:cible}}` où `cible` est un nom de
//! combattant (insensible à la casse) ou un rang (`1` = premier du classement
//! de la métrique). Sans cible, la variable s'applique à ton personnage.
//!
//! Exemple : `hps: {{HPS}} - mon dps {{dps}} / top {{dps:1}} ({{name:1}})`

use crate::combat::{fmt_duration, fmt_f64, fmt_num, Combatant, Encounter};
use regex::Regex;
use std::sync::OnceLock;

/// (variable à insérer, description) — pour le menu de sélection dans l'UI.
pub const VARIABLES: &[(&str, &str)] = &[
    ("{{dps}}", "ton DPS (ou {{dps:Nom}} / {{dps:1}} pour le rang 1)"),
    ("{{hps}}", "tes soins par seconde"),
    ("{{pps}}", "ton power rendu par seconde"),
    ("{{dmg}}", "tes dégâts totaux"),
    ("{{heal}}", "tes soins totaux"),
    ("{{power}}", "ton power total rendu"),
    ("{{crit}}", "ton taux de critique"),
    ("{{maxhit}}", "ton plus gros coup"),
    ("{{rank}}", "ton rang au classement dégâts"),
    ("{{taken}}", "tes dégâts subis"),
    ("{{deaths}}", "tes morts"),
    ("{{name}}", "ton nom (ou {{name:1}} = nom du rang 1)"),
    ("{{target}}", "nom de l'encounter (mob principal)"),
    ("{{time}}", "durée du combat (m:ss)"),
    ("{{raiddps}}", "DPS total de l'encounter"),
    ("{{raidhps}}", "HPS total de l'encounter"),
    ("{{total}}", "dégâts totaux de l'encounter"),
    ("{{kills}}", "nombre de kills de l'encounter"),
];

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{\s*([A-Za-zÀ-ÿ]+)\s*(?::\s*([^}]+?)\s*)?\}\}").unwrap())
}

/// Classement utilisé pour résoudre un rang selon la variable.
fn ranking_for<'a>(enc: &'a Encounter, var: &str) -> Vec<(&'a String, &'a Combatant)> {
    match var {
        "hps" | "heal" | "soins" => enc.heal_ranking(),
        "pps" | "power" => enc.power_ranking(),
        _ => enc.damage_ranking(),
    }
}

/// Résout le combattant visé : rang (`1`…), nom, ou soi-même.
fn resolve_combatant<'a>(
    enc: &'a Encounter,
    var: &str,
    arg: Option<&str>,
    self_name: Option<&str>,
) -> Option<(&'a String, &'a Combatant)> {
    match arg {
        Some(a) => {
            if let Ok(rank) = a.parse::<usize>() {
                let ranking = ranking_for(enc, var);
                ranking.get(rank.saturating_sub(1)).copied()
            } else {
                enc.combatants
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(a))
            }
        }
        None => {
            let name = self_name?;
            enc.combatants
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
        }
    }
}

pub fn render(template: &str, enc: Option<&Encounter>, self_name: Option<&str>) -> String {
    re().replace_all(template, |caps: &regex::Captures| {
        let var = caps[1].to_lowercase();
        let arg = caps.get(2).map(|m| m.as_str());

        // Variables indépendantes de l'encounter.
        if matches!(var.as_str(), "name" | "nom") && arg.is_none() {
            return self_name.unwrap_or("—").to_string();
        }

        let Some(e) = enc else { return "—".to_string() };

        // Variables au niveau de l'encounter.
        match var.as_str() {
            "target" | "cible" | "titre" => return e.title(),
            "time" | "duree" | "durée" | "temps" => return fmt_duration(e.duration()),
            "raiddps" => {
                return fmt_f64(e.total_damage() as f64 / e.duration() as f64);
            }
            "raidhps" => {
                let total: u64 = e.combatants.values().map(|c| c.healing).sum();
                return fmt_f64(total as f64 / e.duration() as f64);
            }
            "total" | "totaldmg" => return fmt_num(e.total_damage()),
            "kills" => return e.kills.len().to_string(),
            _ => {}
        }

        // Variables liées à un combattant.
        let Some((name, c)) = resolve_combatant(e, &var, arg, self_name) else {
            return "—".to_string();
        };
        match var.as_str() {
            "name" | "nom" => name.to_string(),
            "dps" => fmt_f64(e.dps_of(c)),
            "hps" => fmt_f64(e.hps_of(c)),
            "pps" => fmt_f64(e.pps_of(c)),
            "dmg" | "damage" | "degats" | "dégâts" => fmt_num(c.damage),
            "heal" | "soins" => fmt_num(c.healing),
            "power" => fmt_num(c.power),
            "crit" => format!("{:.1}%", c.crit_rate()),
            "maxhit" | "max" => fmt_num(c.max_hit),
            "taken" | "subis" => fmt_num(c.damage_taken),
            "deaths" | "morts" => c.deaths.to_string(),
            "rank" | "rang" => {
                let ranking = e.damage_ranking();
                ranking
                    .iter()
                    .position(|(n, _)| n.as_str() == name.as_str())
                    .map(|p| (p + 1).to_string())
                    .unwrap_or_else(|| "—".to_string())
            }
            // Variable inconnue : on la laisse telle quelle pour signaler la typo.
            _ => caps[0].to_string(),
        }
    })
    .to_string()
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
            "(1004)[Tue May 26 17:42:30 2026] Healer's Salve heals YOU for 50 hit points.",
            "(1004)[Tue May 26 17:42:30 2026] You have killed a rat.",
        ] {
            engine.process(&parser.parse_line(l).unwrap());
        }
        engine.current.take().unwrap()
    }

    #[test]
    fn renders_self_and_encounter_vars() {
        let enc = sample();
        let out = render(
            "{{name}} dps {{DPS}} sur {{target}} en {{time}} ({{kills}} kill)",
            Some(&enc),
            Some("Pawkod"),
        );
        assert_eq!(out, "Pawkod dps 25 sur a rat en 0:04 (1 kill)");
    }

    #[test]
    fn renders_rank_and_named_player() {
        let enc = sample();
        // Rang 1 dégâts = Wizzy (5000), insensible à la casse pour les noms.
        let out = render(
            "top: {{name:1}} {{dps:1}} | wizzy crit {{crit:wizzy}} | mon rang {{rank}}",
            Some(&enc),
            Some("Pawkod"),
        );
        assert_eq!(out, "top: Wizzy 1250 | wizzy crit 100.0% | mon rang 2");
    }

    #[test]
    fn unknown_and_missing() {
        let enc = sample();
        // Variable inconnue conservée, joueur absent → tiret.
        let out = render("{{foo}} {{dps:Inconnu}}", Some(&enc), Some("Pawkod"));
        assert_eq!(out, "{{foo}} —");
        // Sans encounter : valeurs combat à tiret, nom ok.
        let out = render("{{name}} {{dps}}", None, Some("Pawkod"));
        assert_eq!(out, "Pawkod —");
    }

    #[test]
    fn multiline_and_spacing() {
        let enc = sample();
        let out = render(
            "hps: {{ HPS }}\nheals reçus de {{name:1}}",
            Some(&enc),
            Some("Pawkod"),
        );
        assert!(out.starts_with("hps: "));
        assert!(out.contains('\n'));
    }
}
