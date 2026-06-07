//! Parsing des lignes de log EQ2 vers des événements de combat structurés.
//!
//! Format d'une ligne :
//! `(1779810105)[Tue May 26 17:41:45 2026] <message>`
//!
//! Les regex sont calibrées sur de vrais logs (client EN).

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub enum MissKind {
    Miss,
    Parry,
    Riposte,
    Dodge,
    Block,
    Deflect,
    Resist,
}

impl MissKind {
    pub fn label(&self) -> &'static str {
        match self {
            MissKind::Miss => "raté",
            MissKind::Parry => "parade",
            MissKind::Riposte => "riposte",
            MissKind::Dodge => "esquive",
            MissKind::Block => "bloc",
            MissKind::Deflect => "déflection",
            MissKind::Resist => "résisté",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogEvent {
    Damage {
        attacker: String,
        ability: Option<String>,
        target: String,
        amount: u64,
        damage_type: String,
        crit: bool,
    },
    /// `X hits YOU but fails to inflict any damage.`
    FailedHit { attacker: String, target: String },
    Miss {
        attacker: String,
        target: String,
        kind: MissKind,
    },
    Heal {
        healer: String,
        ability: String,
        target: String,
        amount: u64,
        crit: bool,
    },
    /// `X's Mend refreshes Y for 745 mana points.` — power replenish.
    PowerRefresh {
        source: String,
        ability: String,
        target: String,
        amount: u64,
        crit: bool,
    },
    WardApplied {
        caster: String,
        ability: String,
        target: String,
        amount: Option<u64>,
        crit: bool,
    },
    /// Dégâts sans attaquant (chute, environnement) : `YOU are hit for 257 falling damage.`
    EnvDamage { target: String, amount: u64 },
    /// Consommation d'un ward : créditée comme "soin effectif" au propriétaire du ward.
    Absorb {
        ability: String,
        target: String,
        amount: u64,
        remaining: u64,
    },
    Threat {
        source: String,
        target: String,
        amount: u64,
    },
    Kill { killer: String, victim: String },
    Slain { victim: String, killer: String },
    StartFight,
    StopFight,
    /// `You send your pet in for the attack!` — signal pour l'auto-détection du pet.
    PetSendAttack,
    /// Nom vu dans un lien de chat `\aPC` : c'est un joueur, jamais un pet.
    PlayerSeen { name: String },
    /// `You have entered <Zone>.` — changement de zone.
    ZoneEnter { zone: String },
}

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub epoch: u64,
    pub message: String,
    pub event: Option<LogEvent>,
}

struct Patterns {
    line: Regex,
    dmg_your: Regex,
    dmg_you_plain: Regex,
    dmg_ability: Regex,
    dmg_plain: Regex,
    env_hit: Regex,
    failed_hit: Regex,
    miss: Regex,
    heal_your: Regex,
    heal_other: Regex,
    power_your: Regex,
    power_other: Regex,
    ward_your: Regex,
    ward_other: Regex,
    absorb: Regex,
    threat: Regex,
    kill_you: Regex,
    kill_other: Regex,
    slain: Regex,
}

static PATTERNS: OnceLock<Patterns> = OnceLock::new();

/// Nombre EQ2 : `93`, `21,385`, ou abrégé `515.9M` / `1.2B` (option client).
const NUM: &str = r"(\d[\d,]*(?:\.\d+)?[KMBT]?)";
const HIT_VERBS: &str = r"(?:hits|hit|multi attacks|double attacks|flurries|aoe attacks)";
/// Liste de dégâts, possiblement multi-types :
/// `109 heat, 4 magic, 3 mental and 3 divine` ou `25 piercing and 6 poison` ou `93 crushing`.
const DMG_LIST: &str = r"(\d[\d,]*(?:\.\d+)?[KMBT]? \w+(?:, \d[\d,]*(?:\.\d+)?[KMBT]? \w+)*(?: and \d[\d,]*(?:\.\d+)?[KMBT]? \w+)?)";

/// Somme les composantes d'une liste de dégâts et retourne (total, type principal).
fn parse_damage_list(s: &str) -> (u64, String) {
    static COMPONENT: OnceLock<Regex> = OnceLock::new();
    let re = COMPONENT.get_or_init(|| {
        Regex::new(r"(\d[\d,]*(?:\.\d+)?[KMBT]?) (\w+)").unwrap()
    });
    let mut total = 0u64;
    let mut primary = String::new();
    for c in re.captures_iter(s) {
        total += parse_num(&c[1]);
        if primary.is_empty() {
            primary = c[2].to_string();
        }
    }
    (total, primary)
}

fn patterns() -> &'static Patterns {
    PATTERNS.get_or_init(|| {
        let crit = r"(?:(a critical of|a Legendary critical of|a Fabled critical of|a Mythical critical of) )?";
        Patterns {
            line: Regex::new(r"^\((\d+)\)\[[^\]]+\] (.*)$").unwrap(),
            // YOUR <ability> hits <target> for [a critical of] <liste> damage.
            dmg_your: Regex::new(&format!(
                r"^YOUR (.+?) {HIT_VERBS} (.+?) for {crit}{DMG_LIST} damage\.$"
            ))
            .unwrap(),
            // YOU hit <target> for [a critical of] <liste> damage.
            dmg_you_plain: Regex::new(&format!(
                r"^YOU {HIT_VERBS} (.+?) for {crit}{DMG_LIST} damage\.$"
            ))
            .unwrap(),
            // <attacker>'s <ability> hits <target> for [a critical of] <liste> damage.
            // `'s?` : gère aussi les noms finissant en s (`Andreas' Faithful Swing`).
            dmg_ability: Regex::new(&format!(
                r"^(.+?)'s? (.+?) {HIT_VERBS} (.+?) for {crit}{DMG_LIST} damage\.$"
            ))
            .unwrap(),
            // <attacker> hits <target> for [a critical of] <liste> damage.
            dmg_plain: Regex::new(&format!(
                r"^(.+?) {HIT_VERBS} (.+?) for {crit}{DMG_LIST} damage\.$"
            ))
            .unwrap(),
            // <target> is/are hit for <liste> damage. (chute, environnement)
            env_hit: Regex::new(&format!(
                r"^(.+?) (?:is|are) hit for {crit}{DMG_LIST} damage\.$"
            ))
            .unwrap(),
            failed_hit: Regex::new(
                r"^(.+?) (?:hits|hit) (.+?) but fails? to inflict any damage\.$",
            )
            .unwrap(),
            // <attacker> tries to <verb> <target>, but [misses | <target> parries...].
            miss: Regex::new(
                r"^(.+?) tries to \w+ (.+?), but (?:(misses)|.+? (parries|parry|ripostes|riposte|dodges|dodge|blocks|block|deflects|deflect)|(.+? resists))[.!]?$",
            )
            .unwrap(),
            heal_your: Regex::new(&format!(
                r"^YOUR (.+?) (?:critically )?heals (.+?) for {crit}{NUM} (?:hit points?|points? of power|power)\.$"
            ))
            .unwrap(),
            heal_other: Regex::new(&format!(
                r"^(.+?)'s? (.+?) (?:critically )?heals (.+?) for {crit}{NUM} (?:hit points?|points? of power|power)\.$"
            ))
            .unwrap(),
            power_your: Regex::new(&format!(
                r"^YOUR (.+?) (?:critically )?refreshes (.+?) for {crit}{NUM} mana points\.$"
            ))
            .unwrap(),
            power_other: Regex::new(&format!(
                r"^(.+?)'s? (.+?) (?:critically )?refreshes (.+?) for {crit}{NUM} mana points\.$"
            ))
            .unwrap(),
            // YOUR <ability> has applied to <target> as a [critical] ward[ for <n>][.]
            ward_your: Regex::new(&format!(
                r"^YOUR (.+?) has applied to (.+?) as a (critical )?ward(?: for {NUM})?\.?$"
            ))
            .unwrap(),
            ward_other: Regex::new(&format!(
                r"^(.+?)'s? (.+?) has applied to (.+?) as a (critical )?ward(?: for {NUM})?\.?$"
            ))
            .unwrap(),
            // <owner>'s <ability> absorbs <n> point(s) of damage from being done to <target>. (<n> point(s) remaining)
            absorb: Regex::new(&format!(
                r"^.+?'s? (.+?) absorbs {NUM} points? of damage from being done to (.+?)\. \({NUM} points? remaining\)$"
            ))
            .unwrap(),
            threat: Regex::new(&format!(
                r"^(?:YOUR|(.+?)'s) (?:.+?) increases (?:YOUR|.+?) hate with (.+?) for {NUM} threat\.$"
            ))
            .unwrap(),
            kill_you: Regex::new(r"^You have killed (.+?)\.$").unwrap(),
            kill_other: Regex::new(r"^(.+?) has killed (.+?)\.$").unwrap(),
            slain: Regex::new(r"^(.+?) (?:has|have) been slain by (.+?)!$").unwrap(),
        }
    })
}

fn parse_num(s: &str) -> u64 {
    let s = s.replace(',', "");
    let (digits, mult) = match s.as_bytes().last() {
        Some(b'K') => (&s[..s.len() - 1], 1e3),
        Some(b'M') => (&s[..s.len() - 1], 1e6),
        Some(b'B') => (&s[..s.len() - 1], 1e9),
        Some(b'T') => (&s[..s.len() - 1], 1e12),
        _ => (s.as_str(), 1.0),
    };
    digits
        .parse::<f64>()
        .map(|v| (v * mult) as u64)
        .unwrap_or(0)
}

/// Parseur principal. `self_name` = nom du personnage (depuis le nom du fichier log)
/// pour résoudre YOU/YOUR.
pub struct Parser {
    pub self_name: String,
}

impl Parser {
    pub fn new(self_name: impl Into<String>) -> Self {
        Self { self_name: self_name.into() }
    }

    fn resolve<'a>(&'a self, name: &'a str) -> String {
        if name == "YOU" || name == "YOUR" || name == "You" || name == "you" {
            self.self_name.clone()
        } else {
            name.to_string()
        }
    }

    pub fn parse_line(&self, raw: &str) -> Option<ParsedLine> {
        let p = patterns();
        let caps = p.line.captures(raw)?;
        let epoch: u64 = caps[1].parse().ok()?;
        let msg = caps[2].to_string();
        let event = self.parse_message(&msg);
        Some(ParsedLine { epoch, message: msg, event })
    }

    pub fn parse_message(&self, msg: &str) -> Option<LogEvent> {
        let p = patterns();

        // Lignes système fréquentes — early-out bon marché.
        match msg {
            "You start fighting." => return Some(LogEvent::StartFight),
            "You stop fighting." => return Some(LogEvent::StopFight),
            "You send your pet in for the attack!" => return Some(LogEvent::PetSendAttack),
            _ => {}
        }
        if let Some(zone) = msg
            .strip_prefix("You have entered ")
            .and_then(|z| z.strip_suffix('.'))
        {
            return Some(LogEvent::ZoneEnter { zone: zone.to_string() });
        }
        // Les lignes de chat commencent par \aPC, \aNPC… : on en extrait juste
        // le nom des joueurs (utile pour l'attribution des pets), puis on ignore.
        if msg.starts_with("\\a") {
            if let Some(rest) = msg.strip_prefix("\\aPC ") {
                // `\aPC -1 Alibabar:Alibabar\/a tells ...`
                if let Some(name) = rest
                    .split_once(' ')
                    .and_then(|(_, r)| r.split_once(':'))
                    .map(|(n, _)| n)
                {
                    if !name.is_empty() {
                        return Some(LogEvent::PlayerSeen { name: name.to_string() });
                    }
                }
            }
            return None;
        }

        if msg.starts_with("YOUR ") {
            if let Some(c) = p.dmg_your.captures(msg) {
                let (amount, damage_type) = parse_damage_list(&c[4]);
                return Some(LogEvent::Damage {
                    attacker: self.self_name.clone(),
                    ability: Some(c[1].to_string()),
                    target: self.resolve(&c[2]),
                    crit: c.get(3).is_some(),
                    amount,
                    damage_type,
                });
            }
            if let Some(c) = p.heal_your.captures(msg) {
                return Some(LogEvent::Heal {
                    healer: self.self_name.clone(),
                    ability: c[1].to_string(),
                    target: self.resolve(&c[2]),
                    crit: c.get(3).is_some(),
                    amount: parse_num(&c[4]),
                });
            }
            if let Some(c) = p.ward_your.captures(msg) {
                return Some(LogEvent::WardApplied {
                    caster: self.self_name.clone(),
                    ability: c[1].to_string(),
                    target: self.resolve(&c[2]),
                    crit: c.get(3).is_some(),
                    amount: c.get(4).map(|m| parse_num(m.as_str())),
                });
            }
            if let Some(c) = p.power_your.captures(msg) {
                return Some(LogEvent::PowerRefresh {
                    source: self.self_name.clone(),
                    ability: c[1].to_string(),
                    target: self.resolve(&c[2]),
                    crit: c.get(3).is_some(),
                    amount: parse_num(&c[4]),
                });
            }
        }

        if msg.starts_with("YOU ") {
            if let Some(c) = p.dmg_you_plain.captures(msg) {
                let (amount, damage_type) = parse_damage_list(&c[3]);
                return Some(LogEvent::Damage {
                    attacker: self.self_name.clone(),
                    ability: None,
                    target: self.resolve(&c[1]),
                    crit: c.get(2).is_some(),
                    amount,
                    damage_type,
                });
            }
        }

        if msg.contains(" absorbs ") {
            if let Some(c) = p.absorb.captures(msg) {
                return Some(LogEvent::Absorb {
                    ability: c[1].to_string(),
                    amount: parse_num(&c[2]),
                    target: self.resolve(&c[3]),
                    remaining: parse_num(&c[4]),
                });
            }
        }

        if msg.contains(" damage.") {
            if let Some(c) = p.env_hit.captures(msg) {
                let (amount, _) = parse_damage_list(&c[3]);
                return Some(LogEvent::EnvDamage {
                    target: self.resolve(&c[1]),
                    amount,
                });
            }
            if let Some(c) = p.dmg_ability.captures(msg) {
                let (amount, damage_type) = parse_damage_list(&c[5]);
                return Some(LogEvent::Damage {
                    attacker: self.resolve(&c[1]),
                    ability: Some(c[2].to_string()),
                    target: self.resolve(&c[3]),
                    crit: c.get(4).is_some(),
                    amount,
                    damage_type,
                });
            }
            if let Some(c) = p.dmg_plain.captures(msg) {
                let (amount, damage_type) = parse_damage_list(&c[4]);
                return Some(LogEvent::Damage {
                    attacker: self.resolve(&c[1]),
                    ability: None,
                    target: self.resolve(&c[2]),
                    crit: c.get(3).is_some(),
                    amount,
                    damage_type,
                });
            }
            if let Some(c) = p.failed_hit.captures(msg) {
                return Some(LogEvent::FailedHit {
                    attacker: self.resolve(&c[1]),
                    target: self.resolve(&c[2]),
                });
            }
        }

        if msg.contains(" tries to ") {
            if let Some(c) = p.miss.captures(msg) {
                let kind = if c.get(3).is_some() {
                    MissKind::Miss
                } else if let Some(k) = c.get(4) {
                    // Couvre la 3e personne ("parries") et la forme YOU ("parry").
                    match k.as_str() {
                        s if s.starts_with("parr") => MissKind::Parry,
                        s if s.starts_with("ripost") => MissKind::Riposte,
                        s if s.starts_with("dodge") => MissKind::Dodge,
                        s if s.starts_with("block") => MissKind::Block,
                        s if s.starts_with("deflect") => MissKind::Deflect,
                        _ => MissKind::Miss,
                    }
                } else {
                    MissKind::Resist
                };
                return Some(LogEvent::Miss {
                    attacker: self.resolve(&c[1]),
                    target: self.resolve(&c[2]),
                    kind,
                });
            }
        }

        if msg.contains(" heals ") {
            if let Some(c) = p.heal_other.captures(msg) {
                return Some(LogEvent::Heal {
                    healer: self.resolve(&c[1]),
                    ability: c[2].to_string(),
                    target: self.resolve(&c[3]),
                    crit: c.get(4).is_some(),
                    amount: parse_num(&c[5]),
                });
            }
        }

        if msg.contains(" refreshes ") {
            if let Some(c) = p.power_other.captures(msg) {
                return Some(LogEvent::PowerRefresh {
                    source: self.resolve(&c[1]),
                    ability: c[2].to_string(),
                    target: self.resolve(&c[3]),
                    crit: c.get(4).is_some(),
                    amount: parse_num(&c[5]),
                });
            }
        }

        if msg.contains(" as a ") {
            if let Some(c) = p.ward_other.captures(msg) {
                return Some(LogEvent::WardApplied {
                    caster: self.resolve(&c[1]),
                    ability: c[2].to_string(),
                    target: self.resolve(&c[3]),
                    crit: c.get(4).is_some(),
                    amount: c.get(5).map(|m| parse_num(m.as_str())),
                });
            }
        }

        if msg.contains(" threat.") {
            if let Some(c) = p.threat.captures(msg) {
                let source = c
                    .get(1)
                    .map(|m| self.resolve(m.as_str()))
                    .unwrap_or_else(|| self.self_name.clone());
                return Some(LogEvent::Threat {
                    source,
                    target: self.resolve(&c[2]),
                    amount: parse_num(&c[3]),
                });
            }
        }

        if msg.contains(" killed ") {
            if let Some(c) = p.kill_you.captures(msg) {
                return Some(LogEvent::Kill {
                    killer: self.self_name.clone(),
                    victim: c[1].to_string(),
                });
            }
            if let Some(c) = p.kill_other.captures(msg) {
                return Some(LogEvent::Kill {
                    killer: self.resolve(&c[1]),
                    victim: c[2].to_string(),
                });
            }
        }

        if msg.contains(" slain ") {
            if let Some(c) = p.slain.captures(msg) {
                return Some(LogEvent::Slain {
                    victim: self.resolve(&c[1]),
                    killer: self.resolve(&c[2]),
                });
            }
        }

        None
    }
}

/// Extrait le nom du personnage depuis un chemin `eq2log_<Nom>.txt`.
pub fn char_name_from_path(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.strip_prefix("eq2log_").map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Parser {
        Parser::new("Pawkod")
    }

    #[test]
    fn parses_line_envelope() {
        let l = p()
            .parse_line("(1779810146)[Tue May 26 17:42:26 2026] Vicolin J'Viniurden hits a Sablevein destroyer for 109 crushing damage.")
            .unwrap();
        assert_eq!(l.epoch, 1779810146);
        assert!(matches!(l.event, Some(LogEvent::Damage { .. })));
    }

    #[test]
    fn damage_plain_other() {
        let e = p()
            .parse_message("Larinil V'Zeraana hits a Sablevein destroyer for 93 crushing damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Larinil V'Zeraana".into(),
                ability: None,
                target: "a Sablevein destroyer".into(),
                amount: 93,
                damage_type: "crushing".into(),
                crit: false,
            }
        );
    }

    #[test]
    fn damage_ability_other_with_apostrophe_name() {
        let e = p()
            .parse_message("Vicolin J'Viniurden's Ruin hits a Sablevein destroyer for 38 slashing damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Vicolin J'Viniurden".into(),
                ability: Some("Ruin".into()),
                target: "a Sablevein destroyer".into(),
                amount: 38,
                damage_type: "slashing".into(),
                crit: false,
            }
        );
    }

    #[test]
    fn damage_you_plain() {
        let e = p()
            .parse_message("YOU hit a Sablevein crumbler for 2 slashing damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Pawkod".into(),
                ability: None,
                target: "a Sablevein crumbler".into(),
                amount: 2,
                damage_type: "slashing".into(),
                crit: false,
            }
        );
    }

    #[test]
    fn damage_your_ability() {
        let e = p()
            .parse_message("YOUR Insidious Whisper hits a Sablevein crumbler for 3 disease damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Pawkod".into(),
                ability: Some("Insidious Whisper".into()),
                target: "a Sablevein crumbler".into(),
                amount: 3,
                damage_type: "disease".into(),
                crit: false,
            }
        );
    }

    #[test]
    fn damage_crit_with_commas() {
        let e = p()
            .parse_message("YOUR Plaguebringer hits Patriae Vykel for a critical of 21,385 disease damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Pawkod".into(),
                ability: Some("Plaguebringer".into()),
                target: "Patriae Vykel".into(),
                amount: 21385,
                damage_type: "disease".into(),
                crit: true,
            }
        );
    }

    #[test]
    fn damage_you_crit_plain() {
        let e = p()
            .parse_message("YOU hit Patriae Vykel for a critical of 2,925 crushing damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Pawkod".into(),
                ability: None,
                target: "Patriae Vykel".into(),
                amount: 2925,
                damage_type: "crushing".into(),
                crit: true,
            }
        );
    }

    #[test]
    fn damage_ability_crit_other() {
        let e = p()
            .parse_message("Unag's Holy Avenger's Vengeance hits a flame twister for a critical of 3,557 divine damage.")
            .unwrap();
        // Heuristique du premier `'s ` : attacker = "Unag", ability = "Holy Avenger's Vengeance"
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Unag".into(),
                ability: Some("Holy Avenger's Vengeance".into()),
                target: "a flame twister".into(),
                amount: 3557,
                damage_type: "divine".into(),
                crit: true,
            }
        );
    }

    #[test]
    fn damage_multi_type() {
        // 2 composantes
        let e = p()
            .parse_message("a blightfang hatchling hits Talosin for 25 piercing and 6 poison damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "a blightfang hatchling".into(),
                ability: None,
                target: "Talosin".into(),
                amount: 31,
                damage_type: "piercing".into(),
                crit: false,
            }
        );

        // 4 composantes, avec ability
        let e = p()
            .parse_message("Holly Windstalker's Double Shot hits YOU for 109 heat, 4 magic, 3 mental and 3 divine damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Damage {
                attacker: "Holly Windstalker".into(),
                ability: Some("Double Shot".into()),
                target: "Pawkod".into(),
                amount: 119,
                damage_type: "heat".into(),
                crit: false,
            }
        );
    }

    #[test]
    fn possessive_name_ending_in_s() {
        let e = p()
            .parse_message("Andreas' Faithful Swing heals Andreas for 2 hit points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Heal {
                healer: "Andreas".into(),
                ability: "Faithful Swing".into(),
                target: "Andreas".into(),
                amount: 2,
                crit: false,
            }
        );
    }

    #[test]
    fn power_refresh() {
        let e = p()
            .parse_message("YOUR Overclocked Manastone refreshes YOU for 745 mana points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::PowerRefresh {
                source: "Pawkod".into(),
                ability: "Overclocked Manastone".into(),
                target: "Pawkod".into(),
                amount: 745,
                crit: false,
            }
        );

        let e = p()
            .parse_message("Dakshesh, the Displaced's Mend refreshes Dakshesh, the Displaced for 2.2M mana points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::PowerRefresh {
                source: "Dakshesh, the Displaced".into(),
                ability: "Mend".into(),
                target: "Dakshesh, the Displaced".into(),
                amount: 2_200_000,
                crit: false,
            }
        );
    }

    #[test]
    fn abbreviated_numbers() {
        // Format abrégé du client (option "abbreviate numbers")
        let e = p()
            .parse_message("Dakshesh, the Displaced's Fanatical Healing heals YOU for 515.9M hit points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Heal {
                healer: "Dakshesh, the Displaced".into(),
                ability: "Fanatical Healing".into(),
                target: "Pawkod".into(),
                amount: 515_900_000,
                crit: false,
            }
        );

        let e = p()
            .parse_message("YOUR Plaguebringer hits a dragon for a critical of 1.2B disease damage.")
            .unwrap();
        assert!(matches!(e, LogEvent::Damage { amount: 1_200_000_000, crit: true, .. }));
    }

    #[test]
    fn env_damage() {
        let e = p()
            .parse_message("YOU are hit for 257 falling damage.")
            .unwrap();
        assert_eq!(e, LogEvent::EnvDamage { target: "Pawkod".into(), amount: 257 });

        let e = p().parse_message("Jinku is hit for 0 crushing damage.").unwrap();
        assert_eq!(e, LogEvent::EnvDamage { target: "Jinku".into(), amount: 0 });
    }

    #[test]
    fn failed_hit() {
        let e = p()
            .parse_message("a Sablevein crumbler hits YOU but fails to inflict any damage.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::FailedHit {
                attacker: "a Sablevein crumbler".into(),
                target: "Pawkod".into(),
            }
        );
    }

    #[test]
    fn miss_and_avoidance() {
        let e = p()
            .parse_message("a Sablevein crumbler tries to crush YOU, but misses.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Miss {
                attacker: "a Sablevein crumbler".into(),
                target: "Pawkod".into(),
                kind: MissKind::Miss,
            }
        );

        let e = p()
            .parse_message("a Sablevein destroyer tries to crush Vicolin J'Viniurden, but Vicolin J'Viniurden parries.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Miss {
                attacker: "a Sablevein destroyer".into(),
                target: "Vicolin J'Viniurden".into(),
                kind: MissKind::Parry,
            }
        );

        let e = p()
            .parse_message("a Sablevein destroyer tries to crush Vicolin J'Viniurden, but Vicolin J'Viniurden ripostes.")
            .unwrap();
        assert!(matches!(e, LogEvent::Miss { kind: MissKind::Riposte, .. }));
    }

    #[test]
    fn heal_your_and_other() {
        let e = p()
            .parse_message("YOUR Greater Regrowth heals Aewaryr for 29 hit points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Heal {
                healer: "Pawkod".into(),
                ability: "Greater Regrowth".into(),
                target: "Aewaryr".into(),
                amount: 29,
                crit: false,
            }
        );

        let e = p()
            .parse_message("Alibabar's Greater Regrowth heals YOU for 29 hit points.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Heal {
                healer: "Alibabar".into(),
                ability: "Greater Regrowth".into(),
                target: "Pawkod".into(),
                amount: 29,
                crit: false,
            }
        );
    }

    #[test]
    fn ward_applied_variants() {
        // Avec montant, sans point final (vu dans les vrais logs)
        let e = p()
            .parse_message("YOUR Dozekar's Resilience has applied to Galym as a critical ward for 727,324,608")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::WardApplied {
                caster: "Pawkod".into(),
                ability: "Dozekar's Resilience".into(),
                target: "Galym".into(),
                amount: Some(727_324_608),
                crit: true,
            }
        );

        // Sans montant
        let e = p()
            .parse_message("YOUR Aura of Leadership has applied to Ganu as a critical ward.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::WardApplied {
                caster: "Pawkod".into(),
                ability: "Aura of Leadership".into(),
                target: "Ganu".into(),
                amount: None,
                crit: true,
            }
        );

        // Ward simple pour 0
        let e = p()
            .parse_message("YOUR Runic Armor has applied to Aewaryr as a ward for 0.")
            .unwrap();
        assert!(matches!(
            e,
            LogEvent::WardApplied { amount: Some(0), crit: false, .. }
        ));
    }

    #[test]
    fn absorb() {
        let e = p()
            .parse_message("a Sablevein crumbler's Planar Power absorbs 1 points of damage from being done to a Sablevein crumbler. (8 points remaining)")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Absorb {
                ability: "Planar Power".into(),
                amount: 1,
                target: "a Sablevein crumbler".into(),
                remaining: 8,
            }
        );
    }

    #[test]
    fn threat() {
        let e = p()
            .parse_message("YOUR Insidious Whisper increases YOUR hate with a Sablevein crumbler for 101 threat.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Threat {
                source: "Pawkod".into(),
                target: "a Sablevein crumbler".into(),
                amount: 101,
            }
        );
    }

    #[test]
    fn kills() {
        let e = p().parse_message("You have killed a Sabertooth pup.").unwrap();
        assert_eq!(
            e,
            LogEvent::Kill { killer: "Pawkod".into(), victim: "a Sabertooth pup".into() }
        );

        let e = p()
            .parse_message("Vicolin J'Viniurden has killed a Sablevein destroyer.")
            .unwrap();
        assert_eq!(
            e,
            LogEvent::Kill {
                killer: "Vicolin J'Viniurden".into(),
                victim: "a Sablevein destroyer".into(),
            }
        );
    }

    #[test]
    fn fight_state() {
        assert_eq!(p().parse_message("You start fighting."), Some(LogEvent::StartFight));
        assert_eq!(p().parse_message("You stop fighting."), Some(LogEvent::StopFight));
    }

    #[test]
    fn ignores_chat_and_system() {
        // Les lignes de chat PC identifient les joueurs (exclus de la détection pet)
        assert_eq!(
            p().parse_message(r#"\aPC -1 Stars:Stars\/a tells General (3), "one million dollars""#),
            Some(LogEvent::PlayerSeen { name: "Stars".into() })
        );
        // Les autres liens \a sont ignorés
        assert_eq!(
            p().parse_message(r#"\aNPC 15261 a Sabertooth pup:a Sabertooth pup\/a says in Gnollish, "Spin!""#),
            None
        );
        assert_eq!(
            p().parse_message("You have entered Darklight Wood."),
            Some(LogEvent::ZoneEnter { zone: "Darklight Wood".into() })
        );
        assert_eq!(
            p().parse_message("Your faction standing with The Great Herd got better."),
            None
        );
    }

    #[test]
    fn char_name_extraction() {
        assert_eq!(
            char_name_from_path(std::path::Path::new(
                r"X:\jeux\steam\steamapps\common\EverQuest 2\logs\Halls of Fate\eq2log_Pawkod.txt"
            )),
            Some("Pawkod".to_string())
        );
    }
}
