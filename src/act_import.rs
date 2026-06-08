//! Import de configurations ACT (Advanced Combat Tracker, EQ2).
//!
//! - `<SpellTimers>` (capacité → durée fixe) → [`MechEntry`] : la base de
//!   mécaniques (période = `Timer`, avance = `WarningValue`, boss = `Category`).
//! - `<CustomTriggers>` (regex → son/TTS/timer) → [`Trigger`].
//!
//! Les différences de dialecte sont gérées : entités XML (`&lt;`…), templates
//! `${nom}`/`$1` → `{nom}`/`{1}`, et un nettoyage des grossièretés/noms perso.

use crate::mechanics::{AlertMode, MechEntry, MechKind, MechSource};
use crate::triggers::Trigger;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct ImportResult {
    pub triggers: Vec<Trigger>,
    pub mechanics: Vec<MechEntry>,
}

/// Décode les entités XML d'une valeur d'attribut.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#xA;", "\n")
        .replace("&#xD;", "")
        .replace("&#x9;", "\t")
        .replace("&amp;", "&")
}

/// Lit l'attribut `key="..."` d'un élément (les valeurs n'ont pas de `"` brut).
fn attr(elem: &str, key: &str) -> Option<String> {
    let pat = format!("{key}=\"");
    let start = elem.find(&pat)? + pat.len();
    let rest = &elem[start..];
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

/// `${nom}` → `{nom}` et `$1` → `{1}` (templates de capture).
fn convert_template(s: &str) -> String {
    static NAMED: OnceLock<Regex> = OnceLock::new();
    static NUM: OnceLock<Regex> = OnceLock::new();
    let named = NAMED.get_or_init(|| Regex::new(r"\$\{(\w+)\}").unwrap());
    let num = NUM.get_or_init(|| Regex::new(r"\$(\d)").unwrap());
    let s = named.replace_all(s, "{$1}");
    num.replace_all(&s, "{$1}").into_owned()
}

const PROFANITY: &[&str] = &[
    "bitch", "bitches", "fuck", "fucking", "dipshit", "tard", "retard", "shit",
    "twat", "bastard", "dick", "ass ", "asshole", "motherfucker", "cunt",
];

/// Retire les grossièretés d'un message (insensible à la casse), nettoie les espaces.
fn sanitize(msg: &str) -> String {
    let mut out = msg.to_string();
    let lower = out.to_lowercase();
    if PROFANITY.iter().any(|w| lower.contains(w)) {
        for w in PROFANITY {
            // Remplacement insensible à la casse, mot approximatif.
            let re = Regex::new(&format!("(?i){}", regex::escape(w))).unwrap();
            out = re.replace_all(&out, "").into_owned();
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Déduit un type de mécanique depuis le nom de la capacité (cosmétique).
fn infer_kind(name: &str) -> MechKind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["death", "deathtouch", "harm touch", "annihil", "touch of death", "slay"]) {
        MechKind::Lethal
    } else if has(&[
        "nova", "storm", "stomp", "breath", "barrage", "cloud", "aura", "quake",
        "rain", "wave", "tempest", "vortex", "maelstrom", "fissure", "circle",
        "firestorm", "typhoon", "winds", "field",
    ]) {
        MechKind::Aoe
    } else if has(&[
        "crush", "smash", "strike", "slam", "cleave", "pummel", "swipe", "bite",
        "claw", "touch", "fist", "punch", "slice", "blow", "stomp",
    ]) {
        MechKind::TankBuster
    } else {
        MechKind::Other
    }
}

fn alert_of(start: &str, warn: &str) -> AlertMode {
    let s = format!("{start} {warn}").to_lowercase();
    if s.contains("tts") {
        AlertMode::Tts
    } else if !start.trim().is_empty() || !warn.trim().is_empty() {
        AlertMode::Sound
    } else {
        AlertMode::Inherit
    }
}

/// Une valeur de son ACT qui n'est ni vide ni « tts » est un texte d'alerte.
fn alert_text(warn: &str) -> String {
    let t = warn.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("tts") || t.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        sanitize(&convert_template(t))
    }
}

pub fn parse_act_xml(xml: &str) -> ImportResult {
    static SPELL: OnceLock<Regex> = OnceLock::new();
    static TRIGGER: OnceLock<Regex> = OnceLock::new();
    let spell_re = SPELL.get_or_init(|| Regex::new(r"<Spell\b[^>]*/>").unwrap());
    let trigger_re = TRIGGER.get_or_init(|| Regex::new(r"<Trigger\b[^>]*/>").unwrap());

    // 1) Table des durées de spell timers (Name → secondes), pour résoudre les
    //    timers référencés par les CustomTriggers.
    let mut spell_durations: HashMap<String, u64> = HashMap::new();
    for m in spell_re.find_iter(xml) {
        let e = m.as_str();
        if let (Some(name), Some(timer)) = (attr(e, "Name"), attr(e, "Timer")) {
            if !name.is_empty() {
                spell_durations
                    .entry(name)
                    .or_insert_with(|| timer.parse().unwrap_or(0));
            }
        }
    }

    // 2) SpellTimers → mécaniques (dédup par (mob, capacité)).
    let mut mechanics = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for m in spell_re.find_iter(xml) {
        let e = m.as_str();
        let Some(name) = attr(e, "Name") else { continue };
        if name.trim().is_empty() {
            continue;
        }
        let mob = attr(e, "Category").unwrap_or_default().trim().to_string();
        let mob = if mob.eq_ignore_ascii_case("General") {
            String::new()
        } else {
            mob
        };
        if !seen.insert((mob.clone(), name.clone())) {
            continue;
        }
        let period: f64 = attr(e, "Timer").and_then(|t| t.parse().ok()).unwrap_or(0.0);
        let lead: u64 = attr(e, "WarningValue")
            .and_then(|t| t.parse().ok())
            .unwrap_or(5)
            .min(60);
        let start = attr(e, "StartWav").unwrap_or_default();
        let warn = attr(e, "WarningWav").unwrap_or_default();
        let enabled = attr(e, "Checked").map(|c| c == "True").unwrap_or(true);
        mechanics.push(MechEntry {
            zone: String::new(),
            mob,
            ability: name.clone(),
            period,
            lead,
            kind: infer_kind(&name),
            message: alert_text(&warn),
            alert: alert_of(&start, &warn),
            enabled,
            source: MechSource::Bundled,
            ..Default::default()
        });
    }

    // 3) CustomTriggers → triggers.
    let mut triggers = Vec::new();
    for m in trigger_re.find_iter(xml) {
        let e = m.as_str();
        let Some(pattern) = attr(e, "Regex") else { continue };
        if pattern.trim().is_empty() {
            continue;
        }
        let sound_type = attr(e, "SoundType").unwrap_or_default();
        let sound_data = attr(e, "SoundData").unwrap_or_default();
        let category = attr(e, "Category").unwrap_or_default().trim().to_string();
        let timer_on = attr(e, "Timer").map(|t| t == "True").unwrap_or(false);
        let timer_name = attr(e, "TimerName").unwrap_or_default();
        let active = attr(e, "Active").map(|a| a == "True").unwrap_or(true);

        let text = sanitize(&convert_template(&sound_data));
        let (tts, message, sound) = match sound_type.as_str() {
            "3" => (true, text.clone(), None),
            "2" => (false, String::new(), Some(PathBuf::from(sound_data))),
            _ => (false, text.clone(), None),
        };
        let timer_secs = if timer_on {
            spell_durations.get(&timer_name).copied().unwrap_or(0)
        } else {
            0
        };
        // Libellé : « Boss: message » (ou pattern abrégé).
        let label_src = if !message.is_empty() {
            message.clone()
        } else if !timer_name.is_empty() {
            timer_name.clone()
        } else {
            pattern.chars().take(40).collect()
        };
        let name = if category.is_empty() {
            label_src.clone()
        } else {
            format!("{category}: {label_src}")
        };

        triggers.push(Trigger {
            name: name.chars().take(80).collect(),
            pattern,
            enabled: active,
            sound,
            show_toast: true,
            tts,
            message,
            timer_secs,
            timer_label: timer_name,
            cooldown_secs: 0,
        });
    }

    ImportResult { triggers, mechanics }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<Config><CustomTriggers>
<Trigger Active="True" Regex="(?&lt;player&gt;\w+) has gone linkdead" SoundData="${player} linkdead" SoundType="3" Category=" General" Timer="False" TimerName="Player" Tabbed="False" />
<Trigger Active="True" Regex=".*?A massive stone is about to hit you from above.*" SoundData="Move Bitch" SoundType="3" Category=" General" Timer="False" TimerName="" Tabbed="False" />
<Trigger Active="True" Regex="Dagarn begins to shake violently" SoundData="Get Out" SoundType="3" Category="Betrayal" Timer="True" TimerName="Dagarn" Tabbed="False" />
</CustomTriggers><SpellTimers>
<Spell Checked="True" Name="Dagarn" Timer="50" WarningValue="7" StartWav="" WarningWav="" Category=" General" />
<Spell Checked="True" Name="Tainted Blood" Timer="38" WarningValue="10" StartWav="" WarningWav="tts" Category="Berik Bloodfist" />
</SpellTimers></Config>"#;

    #[test]
    fn parses_spelltimers_to_mechanics() {
        let r = parse_act_xml(SAMPLE);
        let tb = r
            .mechanics
            .iter()
            .find(|m| m.ability == "Tainted Blood")
            .unwrap();
        assert_eq!(tb.period, 38.0);
        assert_eq!(tb.lead, 10);
        assert_eq!(tb.mob, "Berik Bloodfist");
        assert_eq!(tb.alert, AlertMode::Tts);
    }

    #[test]
    fn converts_templates_and_sanitizes() {
        let r = parse_act_xml(SAMPLE);
        let ld = r
            .triggers
            .iter()
            .find(|t| t.pattern.contains("linkdead"))
            .unwrap();
        assert!(ld.tts);
        assert_eq!(ld.message, "{player} linkdead");
        // Grossièreté retirée.
        let stone = r
            .triggers
            .iter()
            .find(|t| t.pattern.contains("massive stone"))
            .unwrap();
        assert_eq!(stone.message, "Move");
    }

    #[test]
    fn resolves_timer_duration_from_spell() {
        let r = parse_act_xml(SAMPLE);
        let dagarn = r
            .triggers
            .iter()
            .find(|t| t.pattern.contains("Dagarn begins"))
            .unwrap();
        // Le timer du trigger récupère la durée du <Spell Name="Dagarn">.
        assert_eq!(dagarn.timer_secs, 50);
        assert_eq!(dagarn.timer_label, "Dagarn");
    }
}
