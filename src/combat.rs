//! Moteur d'encounters façon ACT : un encounter démarre à la première action
//! offensive et se termine après `timeout` secondes d'inactivité.

use crate::parser::{LogEvent, ParsedLine};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Default, Clone, Serialize)]
pub struct AbilityStats {
    pub damage: u64,
    pub healing: u64,
    pub power: u64,
    pub hits: u32,
    pub crits: u32,
    pub max_hit: u64,
    /// Série temporelle (epoch → montant, dégâts/soins/power confondus)
    /// pour le graphe empilé par sort.
    pub series: BTreeMap<u64, u64>,
}

impl AbilityStats {
    fn absorb(&mut self, o: &AbilityStats) {
        self.damage += o.damage;
        self.healing += o.healing;
        self.power += o.power;
        self.hits += o.hits;
        self.crits += o.crits;
        self.max_hit = self.max_hit.max(o.max_hit);
        for (t, v) in &o.series {
            *self.series.entry(*t).or_default() += v;
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Combatant {
    pub damage: u64,
    pub healing: u64,
    /// Power replenish (mana rendu aux autres / à soi).
    pub power: u64,
    pub damage_taken: u64,
    pub heal_received: u64,
    pub hits: u32,
    pub crits: u32,
    /// Attaques de ce combattant évitées/ratées.
    pub misses: u32,
    pub max_hit: u64,
    pub deaths: u32,
    pub kills: u32,
    pub threat: u64,
    pub abilities: BTreeMap<String, AbilityStats>,
    /// Séries temporelles (epoch seconde → montant) pour le graphe.
    pub dmg_series: BTreeMap<u64, u64>,
    pub heal_series: BTreeMap<u64, u64>,
    pub taken_series: BTreeMap<u64, u64>,
    pub power_series: BTreeMap<u64, u64>,
}

impl Combatant {
    pub fn crit_rate(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.crits as f64 / self.hits as f64 * 100.0
        }
    }

    /// Fusionne `other` (un pet) dans ce combattant. `pet_name` préfixe ses sorts.
    fn absorb_pet(&mut self, other: &Combatant, pet_name: &str) {
        self.damage += other.damage;
        self.healing += other.healing;
        self.power += other.power;
        self.damage_taken += other.damage_taken;
        self.heal_received += other.heal_received;
        self.hits += other.hits;
        self.crits += other.crits;
        self.misses += other.misses;
        self.max_hit = self.max_hit.max(other.max_hit);
        self.deaths += other.deaths;
        self.kills += other.kills;
        self.threat += other.threat;
        for (ab, st) in &other.abilities {
            self.abilities
                .entry(format!("🐾 {pet_name}: {ab}"))
                .or_default()
                .absorb(st);
        }
        for (t, v) in &other.dmg_series {
            *self.dmg_series.entry(*t).or_default() += v;
        }
        for (t, v) in &other.heal_series {
            *self.heal_series.entry(*t).or_default() += v;
        }
        for (t, v) in &other.taken_series {
            *self.taken_series.entry(*t).or_default() += v;
        }
        for (t, v) in &other.power_series {
            *self.power_series.entry(*t).or_default() += v;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Encounter {
    pub start: u64,
    pub end: u64,
    pub finished: bool,
    pub combatants: BTreeMap<String, Combatant>,
    pub kills: Vec<String>,
}

impl Encounter {
    fn new(start: u64) -> Self {
        Self {
            start,
            end: start,
            finished: false,
            combatants: BTreeMap::new(),
            kills: Vec::new(),
        }
    }

    pub fn duration(&self) -> u64 {
        (self.end - self.start).max(1)
    }

    pub fn total_damage(&self) -> u64 {
        self.combatants.values().map(|c| c.damage).sum()
    }

    /// Titre = l'entité ayant encaissé le plus de dégâts (généralement le mob principal).
    pub fn title(&self) -> String {
        self.combatants
            .iter()
            .max_by_key(|(_, c)| c.damage_taken)
            .map(|(n, _)| n.clone())
            .or_else(|| self.kills.last().cloned())
            .unwrap_or_else(|| "Combat".to_string())
    }

    pub fn dps_of(&self, c: &Combatant) -> f64 {
        c.damage as f64 / self.duration() as f64
    }

    pub fn hps_of(&self, c: &Combatant) -> f64 {
        c.healing as f64 / self.duration() as f64
    }

    /// Combattants triés par dégâts décroissants (seulement ceux qui ont agi).
    pub fn damage_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(_, c)| c.damage > 0)
            .collect();
        v.sort_by(|a, b| b.1.damage.cmp(&a.1.damage));
        v
    }

    pub fn heal_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(_, c)| c.healing > 0)
            .collect();
        v.sort_by(|a, b| b.1.healing.cmp(&a.1.healing));
        v
    }

    pub fn power_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(_, c)| c.power > 0)
            .collect();
        v.sort_by(|a, b| b.1.power.cmp(&a.1.power));
        v
    }

    pub fn pps_of(&self, c: &Combatant) -> f64 {
        c.power as f64 / self.duration() as f64
    }

    /// Vue avec les pets fusionnés dans leur propriétaire (`owners` : pet → owner).
    /// Retourne un Encounter équivalent ; les sorts des pets sont préfixés `🐾 <pet>:`.
    pub fn merged(&self, owners: &HashMap<String, String>) -> Encounter {
        if owners.is_empty() || !self.combatants.keys().any(|n| owners.contains_key(n)) {
            return self.clone();
        }
        let mut out = Encounter {
            start: self.start,
            end: self.end,
            finished: self.finished,
            combatants: BTreeMap::new(),
            kills: self.kills.clone(),
        };
        // D'abord les non-pets (pour que le propriétaire existe), puis les pets.
        for (name, c) in &self.combatants {
            if !owners.contains_key(name) {
                out.combatants.insert(name.clone(), c.clone());
            }
        }
        for (name, c) in &self.combatants {
            if let Some(owner) = owners.get(name) {
                if owner == name {
                    out.combatants.insert(name.clone(), c.clone());
                } else {
                    out.combatants
                        .entry(owner.clone())
                        .or_default()
                        .absorb_pet(c, name);
                }
            }
        }
        out
    }
}

pub struct CombatEngine {
    /// Secondes d'inactivité avant clôture d'un encounter.
    pub timeout: u64,
    pub current: Option<Encounter>,
    pub history: Vec<Encounter>,
    /// Propriétaire connu du dernier ward posé par (ability) — pour créditer les absorbs.
    ward_owners: BTreeMap<String, String>,
    /// Nom du personnage suivi (pour l'attribution auto des pets).
    pub self_name: String,
    /// Pets auto-détectés : pet → propriétaire.
    pub auto_pets: HashMap<String, String>,
    /// Fenêtre d'auto-détection ouverte par "You send your pet in for the attack!".
    pet_window_until: Option<u64>,
    /// Joueurs vus dans le chat (`\aPC`) : jamais des pets.
    pub known_players: std::collections::HashSet<String>,
}

impl CombatEngine {
    pub fn new(timeout: u64) -> Self {
        Self {
            timeout,
            current: None,
            history: Vec::new(),
            ward_owners: BTreeMap::new(),
            self_name: String::new(),
            auto_pets: HashMap::new(),
            pet_window_until: None,
            known_players: std::collections::HashSet::new(),
        }
    }

    fn close_current(&mut self) {
        if let Some(mut enc) = self.current.take() {
            enc.finished = true;
            // On ne garde pas les "encounters" sans aucun dégât (buffs hors combat…)
            if enc.total_damage() > 0 {
                self.history.push(enc);
            }
        }
    }

    /// À appeler régulièrement avec l'heure courante (epoch) pour clore
    /// l'encounter actif après timeout, même sans nouvelle ligne de log.
    pub fn tick(&mut self, now: u64) {
        let expired = self
            .current
            .as_ref()
            .is_some_and(|e| now > e.end + self.timeout);
        if expired {
            self.close_current();
        }
    }

    fn ensure_encounter(&mut self, epoch: u64) -> &mut Encounter {
        let expired = self
            .current
            .as_ref()
            .is_some_and(|e| epoch > e.end + self.timeout);
        if expired {
            self.close_current();
        }
        if self.current.is_none() {
            self.current = Some(Encounter::new(epoch));
        }
        self.current.as_mut().unwrap()
    }

    pub fn process(&mut self, line: &ParsedLine) {
        let Some(event) = &line.event else { return };
        let epoch = line.epoch;

        match event {
            LogEvent::Damage { attacker, ability, target, amount, crit, .. } => {
                // Auto-détection de pet dans les 4 s après "You send your pet in
                // for the attack!" : nouvel attaquant dont le nom ressemble à un
                // pet généré par EQ2 (un seul mot, capitalisé — ex. "Hadoken"),
                // qui n'attaque pas le joueur.
                if let Some(until) = self.pet_window_until {
                    if epoch <= until {
                        let is_new = self
                            .current
                            .as_ref()
                            .is_none_or(|e| !e.combatants.contains_key(attacker));
                        let pet_like = !attacker.contains(' ')
                            && attacker.chars().next().is_some_and(|c| c.is_uppercase());
                        if is_new
                            && pet_like
                            && attacker != &self.self_name
                            && target != &self.self_name
                            && !self.self_name.is_empty()
                            && !self.known_players.contains(attacker)
                        {
                            self.auto_pets
                                .insert(attacker.clone(), self.self_name.clone());
                            self.pet_window_until = None;
                        }
                    } else {
                        self.pet_window_until = None;
                    }
                }
                let enc = self.ensure_encounter(epoch);
                enc.end = epoch;
                {
                    let a = enc.combatants.entry(attacker.clone()).or_default();
                    a.damage += amount;
                    a.hits += 1;
                    if *crit {
                        a.crits += 1;
                    }
                    a.max_hit = a.max_hit.max(*amount);
                    let key = ability.clone().unwrap_or_else(|| "(auto-attack)".into());
                    let ab = a.abilities.entry(key).or_default();
                    ab.damage += amount;
                    ab.hits += 1;
                    if *crit {
                        ab.crits += 1;
                    }
                    ab.max_hit = ab.max_hit.max(*amount);
                    *ab.series.entry(epoch).or_default() += amount;
                    *a.dmg_series.entry(epoch).or_default() += amount;
                }
                let t = enc.combatants.entry(target.clone()).or_default();
                t.damage_taken += amount;
                *t.taken_series.entry(epoch).or_default() += amount;
            }
            LogEvent::FailedHit { attacker, target } => {
                let enc = self.ensure_encounter(epoch);
                enc.end = epoch;
                let a = enc.combatants.entry(attacker.clone()).or_default();
                a.hits += 1;
                enc.combatants.entry(target.clone()).or_default();
            }
            LogEvent::Miss { attacker, .. } => {
                // Ne démarre pas un encounter à lui seul, mais compte si combat en cours.
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        let a = enc.combatants.entry(attacker.clone()).or_default();
                        a.misses += 1;
                    }
                }
            }
            LogEvent::Heal { healer, ability, target, amount, crit } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        let h = enc.combatants.entry(healer.clone()).or_default();
                        h.healing += amount;
                        if *crit {
                            h.crits += 1;
                        }
                        let ab = h.abilities.entry(ability.clone()).or_default();
                        ab.healing += amount;
                        ab.hits += 1;
                        *ab.series.entry(epoch).or_default() += amount;
                        *h.heal_series.entry(epoch).or_default() += amount;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.heal_received += amount;
                    }
                }
            }
            LogEvent::WardApplied { caster, ability, .. } => {
                // Mémorise le propriétaire du ward pour créditer les absorbs futurs.
                self.ward_owners.insert(ability.clone(), caster.clone());
            }
            LogEvent::Absorb { ability, target, amount, .. } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        let owner = self
                            .ward_owners
                            .get(ability)
                            .cloned()
                            .unwrap_or_else(|| format!("({ability})"));
                        let o = enc.combatants.entry(owner).or_default();
                        o.healing += amount;
                        let ab = o.abilities.entry(format!("{ability} (ward)")).or_default();
                        ab.healing += amount;
                        ab.hits += 1;
                        *ab.series.entry(epoch).or_default() += amount;
                        *o.heal_series.entry(epoch).or_default() += amount;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.heal_received += amount;
                    }
                }
            }
            LogEvent::Threat { source, amount, .. } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        let s = enc.combatants.entry(source.clone()).or_default();
                        s.threat += amount;
                    }
                }
            }
            LogEvent::Kill { killer, victim } => {
                if let Some(enc) = self.current.as_mut() {
                    enc.end = epoch;
                    enc.kills.push(victim.clone());
                    let k = enc.combatants.entry(killer.clone()).or_default();
                    k.kills += 1;
                    if let Some(v) = enc.combatants.get_mut(victim) {
                        v.deaths += 1;
                    }
                }
            }
            LogEvent::Slain { victim, .. } => {
                if let Some(enc) = self.current.as_mut() {
                    enc.end = epoch;
                    let v = enc.combatants.entry(victim.clone()).or_default();
                    v.deaths += 1;
                }
            }
            LogEvent::PowerRefresh { source, ability, target, amount, crit } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        let s = enc.combatants.entry(source.clone()).or_default();
                        s.power += amount;
                        if *crit {
                            s.crits += 1;
                        }
                        let ab = s.abilities.entry(format!("{ability} (power)")).or_default();
                        ab.power += amount;
                        ab.hits += 1;
                        *ab.series.entry(epoch).or_default() += amount;
                        *s.power_series.entry(epoch).or_default() += amount;
                        enc.combatants.entry(target.clone()).or_default();
                    }
                }
            }
            LogEvent::PetSendAttack => {
                self.pet_window_until = Some(epoch + 4);
            }
            LogEvent::PlayerSeen { name } => {
                // Un joueur n'est jamais un pet : corrige rétroactivement
                // les fausses détections.
                self.auto_pets.remove(name);
                self.known_players.insert(name.clone());
            }
            LogEvent::EnvDamage { target, amount } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.damage_taken += amount;
                        *t.taken_series.entry(epoch).or_default() += amount;
                    }
                }
            }
            LogEvent::StartFight | LogEvent::StopFight => {}
        }
    }

    /// Encounter à afficher : l'actif sinon le dernier de l'historique.
    pub fn display_encounter(&self) -> Option<&Encounter> {
        self.current.as_ref().or_else(|| self.history.last())
    }
}

pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

pub fn fmt_f64(n: f64) -> String {
    if n >= 1_000_000_000.0 {
        format!("{:.2}B", n / 1e9)
    } else if n >= 1_000_000.0 {
        format!("{:.2}M", n / 1e6)
    } else if n >= 10_000.0 {
        format!("{:.1}k", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

pub fn fmt_duration(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn feed(engine: &mut CombatEngine, parser: &Parser, lines: &[&str]) {
        for l in lines {
            if let Some(p) = parser.parse_line(l) {
                engine.process(&p);
            }
        }
    }

    #[test]
    fn encounter_lifecycle_and_dps() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        feed(
            &mut engine,
            &parser,
            &[
                "(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage.",
                "(1002)[Tue May 26 17:42:28 2026] YOUR Bash hits a rat for a critical of 200 crushing damage.",
                "(1004)[Tue May 26 17:42:30 2026] Friend hits a rat for 50 slashing damage.",
                "(1004)[Tue May 26 17:42:30 2026] You have killed a rat.",
            ],
        );
        let enc = engine.current.as_ref().unwrap();
        assert_eq!(enc.duration(), 4);
        let me = &enc.combatants["Pawkod"];
        assert_eq!(me.damage, 300);
        assert_eq!(me.hits, 2);
        assert_eq!(me.crits, 1);
        assert_eq!(me.max_hit, 200);
        assert_eq!(me.kills, 1);
        assert_eq!(enc.combatants["Friend"].damage, 50);
        assert_eq!(enc.combatants["a rat"].damage_taken, 350);
        assert_eq!(enc.title(), "a rat");
        assert_eq!(enc.dps_of(me), 75.0);

        // Timeout → nouvel encounter
        feed(
            &mut engine,
            &parser,
            &["(1020)[Tue May 26 17:42:46 2026] YOU hit a bat for 10 crushing damage."],
        );
        assert_eq!(engine.history.len(), 1);
        assert!(engine.history[0].finished);
        assert_eq!(engine.current.as_ref().unwrap().title(), "a bat");
    }

    #[test]
    fn ward_absorb_credited_to_owner() {
        let parser = Parser::new("Galym");
        let mut engine = CombatEngine::new(6);
        feed(
            &mut engine,
            &parser,
            &[
                "(1000)[Tue May 26 17:42:26 2026] YOUR Dozekar's Resilience has applied to Galym as a critical ward for 1,000",
                "(1001)[Tue May 26 17:42:27 2026] a mob hits Galym for 50 crushing damage.",
                "(1002)[Tue May 26 17:42:28 2026] Galym's Dozekar's Resilience absorbs 500 points of damage from being done to Galym. (500 points remaining)",
            ],
        );
        let enc = engine.current.as_ref().unwrap();
        // L'absorb est crédité à Galym (poseur du ward) comme soin effectif
        assert_eq!(enc.combatants["Galym"].healing, 500);
        assert_eq!(enc.combatants["Galym"].heal_received, 500);
    }

    #[test]
    fn pet_auto_detection_and_merge() {
        let parser = Parser::new("Tiskina");
        let mut engine = CombatEngine::new(6);
        engine.self_name = "Tiskina".into();
        feed(
            &mut engine,
            &parser,
            &[
                // Le joueur engage la cible
                "(1000)[Sun May  4 11:13:16 2025] YOUR Burning Agony hits a Sabertooth miner for 95 heat damage.",
                // Ordre d'attaque au pet, puis nouvel attaquant sur la même cible
                "(1001)[Sun May  4 11:13:17 2025] You send your pet in for the attack!",
                "(1002)[Sun May  4 11:13:18 2025] Hadoken's Shocking Flames hits a Sabertooth miner for 188 heat damage.",
                "(1003)[Sun May  4 11:13:19 2025] Hadoken's Searing Flames hits a Sabertooth miner for 45 heat damage.",
            ],
        );
        assert_eq!(engine.auto_pets.get("Hadoken"), Some(&"Tiskina".to_string()));

        let enc = engine.current.as_ref().unwrap();
        // Vue brute : pet séparé
        assert_eq!(enc.combatants["Hadoken"].damage, 233);
        assert_eq!(enc.combatants["Tiskina"].damage, 95);

        // Vue fusionnée : pet dans le propriétaire, sorts préfixés
        let merged = enc.merged(&engine.auto_pets);
        assert!(!merged.combatants.contains_key("Hadoken"));
        let t = &merged.combatants["Tiskina"];
        assert_eq!(t.damage, 328);
        assert!(t.abilities.contains_key("🐾 Hadoken: Shocking Flames"));
        // Séries temporelles fusionnées
        assert_eq!(t.dmg_series.get(&1002), Some(&188));
        assert_eq!(t.dmg_series.get(&1000), Some(&95));
    }

    #[test]
    fn pet_window_ignores_mob_attacking_player() {
        let parser = Parser::new("Tiskina");
        let mut engine = CombatEngine::new(6);
        engine.self_name = "Tiskina".into();
        feed(
            &mut engine,
            &parser,
            &[
                "(1000)[Sun May  4 11:13:16 2025] YOUR Burning Agony hits a Sabertooth miner for 95 heat damage.",
                "(1001)[Sun May  4 11:13:17 2025] You send your pet in for the attack!",
                // Un mob (article + espace dans le nom) frappe le joueur dans la
                // fenêtre : exclu (pas la forme d'un nom de pet, et cible = soi).
                "(1002)[Sun May  4 11:13:18 2025] a wandering gnoll hits Tiskina for 10 crushing damage.",
                // Un PJ nommé frappe le joueur (PvP) : exclu car cible = soi.
                "(1003)[Sun May  4 11:13:19 2025] Backstabber hits Tiskina for 10 piercing damage.",
            ],
        );
        assert!(engine.auto_pets.is_empty());
    }

    #[test]
    fn tick_closes_after_timeout() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        feed(
            &mut engine,
            &parser,
            &["(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage."],
        );
        assert!(engine.current.is_some());
        engine.tick(1003);
        assert!(engine.current.is_some());
        engine.tick(1010);
        assert!(engine.current.is_none());
        assert_eq!(engine.history.len(), 1);
    }
}
