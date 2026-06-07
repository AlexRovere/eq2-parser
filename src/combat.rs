//! Moteur d'encounters façon ACT : un encounter démarre à la première action
//! offensive et se termine après `timeout` secondes d'inactivité.

use crate::parser::{LogEvent, ParsedLine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
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
    /// Détail par cible : mes dégâts ventilés par victime.
    pub damage_by_target: BTreeMap<String, u64>,
    /// Vue tank : dégâts reçus ventilés par attaquant.
    pub taken_by_attacker: BTreeMap<String, u64>,
    /// Matrice de soins : mes soins ventilés par bénéficiaire.
    pub heals_by_target: BTreeMap<String, u64>,
    /// Soins reçus, ventilés par soigneur.
    pub heals_received_from: BTreeMap<String, u64>,
    /// Mes attaques évitées par l'adversaire, par type (parade, esquive…).
    pub misses_by_kind: BTreeMap<String, u32>,
    /// Attaques adverses que J'AI évitées, par type.
    pub avoids_by_kind: BTreeMap<String, u32>,
    /// Mes sorts résistés, par école de magie (heat, cold, disease…).
    pub resists_by_school: BTreeMap<String, u32>,
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
        for (k, v) in &other.damage_by_target {
            *self.damage_by_target.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.taken_by_attacker {
            *self.taken_by_attacker.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.heals_by_target {
            *self.heals_by_target.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.heals_received_from {
            *self.heals_received_from.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.misses_by_kind {
            *self.misses_by_kind.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.avoids_by_kind {
            *self.avoids_by_kind.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.resists_by_school {
            *self.resists_by_school.entry(k.clone()).or_default() += v;
        }
    }

    /// Fusionne `other` tel quel (agrégat de session), avec remapping temporel
    /// des séries pour concaténer les combats bout à bout.
    fn fold_remap(&mut self, other: &Combatant, remap: &dyn Fn(u64) -> u64) {
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
            let dst = self.abilities.entry(ab.clone()).or_default();
            dst.damage += st.damage;
            dst.healing += st.healing;
            dst.power += st.power;
            dst.hits += st.hits;
            dst.crits += st.crits;
            dst.max_hit = dst.max_hit.max(st.max_hit);
            for (t, v) in &st.series {
                *dst.series.entry(remap(*t)).or_default() += v;
            }
        }
        for (t, v) in &other.dmg_series {
            *self.dmg_series.entry(remap(*t)).or_default() += v;
        }
        for (t, v) in &other.heal_series {
            *self.heal_series.entry(remap(*t)).or_default() += v;
        }
        for (t, v) in &other.taken_series {
            *self.taken_series.entry(remap(*t)).or_default() += v;
        }
        for (t, v) in &other.power_series {
            *self.power_series.entry(remap(*t)).or_default() += v;
        }
        for (k, v) in &other.damage_by_target {
            *self.damage_by_target.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.taken_by_attacker {
            *self.taken_by_attacker.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.heals_by_target {
            *self.heals_by_target.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.heals_received_from {
            *self.heals_received_from.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.misses_by_kind {
            *self.misses_by_kind.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.avoids_by_kind {
            *self.avoids_by_kind.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.resists_by_school {
            *self.resists_by_school.entry(k.clone()).or_default() += v;
        }
    }
}

/// Un coup encaissé récemment — pour le rapport de mort.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentHit {
    pub epoch: u64,
    pub attacker: String,
    pub ability: Option<String>,
    pub amount: u64,
}

/// Rapport de mort : qui, quand, par qui, avec les derniers coups encaissés.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeathRecord {
    pub epoch: u64,
    pub victim: String,
    pub killer: String,
    pub hits: Vec<RecentHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Encounter {
    pub start: u64,
    pub end: u64,
    pub finished: bool,
    pub combatants: BTreeMap<String, Combatant>,
    pub kills: Vec<String>,
    /// Arêtes "x a attaqué y" — pour l'inférence alliés/ennemis.
    pub attacks: BTreeMap<String, BTreeSet<String>>,
    /// Arêtes "x a soigné/wardé/régénéré y" — même faction.
    pub assists: BTreeMap<String, BTreeSet<String>>,
    /// Rapports de mort (avec les derniers coups encaissés).
    pub deaths_log: Vec<DeathRecord>,
    /// Zone où le combat a eu lieu (`You have entered <Zone>.`).
    pub zone: String,
    /// Lignes brutes du combat (session courante uniquement, non persisté).
    #[serde(skip)]
    pub raw_lines: Vec<(u64, String)>,
    /// Ensemble des alliés, calculé à l'affichage (non persisté).
    /// `None` = pas de filtre, tout le monde est visible.
    #[serde(skip)]
    pub allies: Option<BTreeSet<String>>,
}

impl Default for Encounter {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Encounter {
    fn new(start: u64) -> Self {
        Self {
            start,
            end: start,
            finished: false,
            combatants: BTreeMap::new(),
            kills: Vec::new(),
            attacks: BTreeMap::new(),
            assists: BTreeMap::new(),
            deaths_log: Vec::new(),
            zone: String::new(),
            raw_lines: Vec::new(),
            allies: None,
        }
    }

    /// Un combattant est-il visible (filtre alliés/ennemis) ?
    pub fn visible(&self, name: &str) -> bool {
        self.allies.as_ref().is_none_or(|a| a.contains(name))
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

    /// Combattants triés par dégâts décroissants (seulement ceux qui ont agi,
    /// filtrés par le set d'alliés si présent).
    pub fn damage_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(n, c)| c.damage > 0 && self.visible(n))
            .collect();
        v.sort_by(|a, b| b.1.damage.cmp(&a.1.damage));
        v
    }

    pub fn heal_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(n, c)| c.healing > 0 && self.visible(n))
            .collect();
        v.sort_by(|a, b| b.1.healing.cmp(&a.1.healing));
        v
    }

    pub fn power_ranking(&self) -> Vec<(&String, &Combatant)> {
        let mut v: Vec<_> = self
            .combatants
            .iter()
            .filter(|(n, c)| c.power > 0 && self.visible(n))
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
            attacks: self.attacks.clone(),
            assists: self.assists.clone(),
            deaths_log: self.deaths_log.clone(),
            zone: self.zone.clone(),
            raw_lines: self.raw_lines.clone(),
            allies: self.allies.clone(),
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

/// Infère l'ensemble des alliés d'un encounter par propagation :
/// attaque = factions opposées, soin = même faction.
/// Graines : soi-même, joueurs vus dans le chat, pets et leurs propriétaires.
pub fn compute_allies(
    enc: &Encounter,
    self_name: &str,
    known_players: &HashSet<String>,
    pet_owners: &HashMap<String, String>,
) -> BTreeSet<String> {
    let mut ally: BTreeSet<String> = BTreeSet::new();
    let mut enemy: BTreeSet<String> = BTreeSet::new();

    if !self_name.is_empty() {
        ally.insert(self_name.to_string());
    }
    for name in enc.combatants.keys() {
        if known_players.contains(name) {
            ally.insert(name.clone());
        }
        if let Some(owner) = pet_owners.get(name) {
            ally.insert(name.clone());
            ally.insert(owner.clone());
        }
    }

    // Propagation jusqu'à stabilité (graphes minuscules : quelques itérations).
    for _ in 0..12 {
        let mut changed = false;
        let set_ally = |n: &String, ally: &mut BTreeSet<String>, enemy: &BTreeSet<String>| {
            if !enemy.contains(n) && ally.insert(n.clone()) {
                true
            } else {
                false
            }
        };
        for (att, targets) in &enc.attacks {
            for t in targets {
                if ally.contains(att) && !ally.contains(t) && enemy.insert(t.clone()) {
                    changed = true;
                }
                if ally.contains(t) && !ally.contains(att) && enemy.insert(att.clone()) {
                    changed = true;
                }
                if enemy.contains(att) {
                    changed |= set_ally(t, &mut ally, &enemy);
                }
                if enemy.contains(t) {
                    changed |= set_ally(att, &mut ally, &enemy);
                }
            }
        }
        for (healer, targets) in &enc.assists {
            for t in targets {
                if ally.contains(healer) {
                    changed |= set_ally(t, &mut ally, &enemy);
                }
                if ally.contains(t) {
                    changed |= set_ally(healer, &mut ally, &enemy);
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Non classés (aucune interaction avec un camp connu) : heuristique de nom —
    // les mobs EQ2 commencent généralement par un article minuscule.
    for name in enc.combatants.keys() {
        if !ally.contains(name) && !enemy.contains(name) {
            let mob_like = name.starts_with("a ")
                || name.starts_with("an ")
                || name.starts_with("the ")
                || name.chars().next().is_some_and(|c| c.is_lowercase());
            if !mob_like {
                ally.insert(name.clone());
            }
        }
    }

    ally
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
    pub known_players: HashSet<String>,
    /// Derniers coups encaissés par entité (fenêtre glissante ~15 s) — death report.
    recent_hits: HashMap<String, VecDeque<RecentHit>>,
    /// Zone courante (`You have entered <Zone>.`).
    pub current_zone: String,
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
            known_players: HashSet::new(),
            recent_hits: HashMap::new(),
            current_zone: String::new(),
        }
    }

    /// Enregistre un coup encaissé dans la fenêtre glissante du death report.
    fn push_recent_hit(&mut self, target: &str, hit: RecentHit) {
        let epoch = hit.epoch;
        let q = self.recent_hits.entry(target.to_string()).or_default();
        q.push_back(hit);
        while q
            .front()
            .is_some_and(|h| h.epoch + 15 < epoch || q.len() > 60)
        {
            q.pop_front();
        }
    }

    fn close_current(&mut self) {
        if let Some(mut enc) = self.current.take() {
            enc.finished = true;
            // On ne garde pas les "encounters" sans aucun dégât (buffs hors combat…)
            // et on déduplique (ré-import d'un log déjà en historique).
            let duplicate = self.history.iter().any(|e| {
                e.start == enc.start
                    && e.end == enc.end
                    && e.total_damage() == enc.total_damage()
            });
            if enc.total_damage() > 0 && !duplicate {
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
            let mut enc = Encounter::new(epoch);
            enc.zone = self.current_zone.clone();
            self.current = Some(enc);
        }
        self.current.as_mut().unwrap()
    }

    pub fn process(&mut self, line: &ParsedLine) {
        let epoch = line.epoch;
        let Some(event) = &line.event else {
            // Pas d'événement combat : on garde quand même la ligne dans le
            // log brut du combat en cours (chat, emotes…).
            if let Some(enc) = self.current.as_mut() {
                if epoch <= enc.end + self.timeout && enc.raw_lines.len() < 5000 {
                    enc.raw_lines.push((epoch, line.message.clone()));
                }
            }
            return;
        };

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
                self.push_recent_hit(
                    target,
                    RecentHit {
                        epoch,
                        attacker: attacker.clone(),
                        ability: ability.clone(),
                        amount: *amount,
                    },
                );
                let enc = self.ensure_encounter(epoch);
                enc.end = epoch;
                enc.attacks
                    .entry(attacker.clone())
                    .or_default()
                    .insert(target.clone());
                {
                    let a = enc.combatants.entry(attacker.clone()).or_default();
                    a.damage += amount;
                    a.hits += 1;
                    if *crit {
                        a.crits += 1;
                    }
                    a.max_hit = a.max_hit.max(*amount);
                    *a.damage_by_target.entry(target.clone()).or_default() += amount;
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
                *t.taken_by_attacker.entry(attacker.clone()).or_default() += amount;
            }
            LogEvent::FailedHit { attacker, target } => {
                let enc = self.ensure_encounter(epoch);
                enc.end = epoch;
                enc.attacks
                    .entry(attacker.clone())
                    .or_default()
                    .insert(target.clone());
                let a = enc.combatants.entry(attacker.clone()).or_default();
                a.hits += 1;
                enc.combatants.entry(target.clone()).or_default();
            }
            LogEvent::Miss { attacker, target, kind } => {
                // Ne démarre pas un encounter à lui seul, mais compte si combat en cours.
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        enc.attacks
                            .entry(attacker.clone())
                            .or_default()
                            .insert(target.clone());
                        let a = enc.combatants.entry(attacker.clone()).or_default();
                        a.misses += 1;
                        *a.misses_by_kind.entry(kind.label().into()).or_default() += 1;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        *t.avoids_by_kind.entry(kind.label().into()).or_default() += 1;
                    }
                }
            }
            LogEvent::Resist { attacker, target, ability, school } => {
                // Compte comme une attaque ratée (type « résisté ») + école.
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        enc.attacks
                            .entry(attacker.clone())
                            .or_default()
                            .insert(target.clone());
                        let a = enc.combatants.entry(attacker.clone()).or_default();
                        a.misses += 1;
                        *a.misses_by_kind.entry("résisté".into()).or_default() += 1;
                        *a.resists_by_school.entry(school.clone()).or_default() += 1;
                        // Le sort résisté apparaît dans le breakdown avec 0 dégât.
                        let ab = a.abilities.entry(ability.clone()).or_default();
                        ab.hits += 1;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        *t.avoids_by_kind.entry("résisté".into()).or_default() += 1;
                    }
                }
            }
            LogEvent::Heal { healer, ability, target, amount, crit } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        enc.assists
                            .entry(healer.clone())
                            .or_default()
                            .insert(target.clone());
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
                        *h.heals_by_target.entry(target.clone()).or_default() += amount;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.heal_received += amount;
                        *t.heals_received_from.entry(healer.clone()).or_default() += amount;
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
                        enc.assists
                            .entry(owner.clone())
                            .or_default()
                            .insert(target.clone());
                        let o = enc.combatants.entry(owner.clone()).or_default();
                        o.healing += amount;
                        let ab = o.abilities.entry(format!("{ability} (ward)")).or_default();
                        ab.healing += amount;
                        ab.hits += 1;
                        *ab.series.entry(epoch).or_default() += amount;
                        *o.heal_series.entry(epoch).or_default() += amount;
                        *o.heals_by_target.entry(target.clone()).or_default() += amount;
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.heal_received += amount;
                        *t.heals_received_from.entry(owner).or_default() += amount;
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
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        enc.kills.push(victim.clone());
                        let k = enc.combatants.entry(killer.clone()).or_default();
                        k.kills += 1;
                        if let Some(v) = enc.combatants.get_mut(victim) {
                            v.deaths += 1;
                        }
                    }
                }
            }
            LogEvent::Slain { victim, killer } => {
                // Rapport de mort : les coups encaissés dans les 12 dernières secondes.
                let hits: Vec<RecentHit> = self
                    .recent_hits
                    .get(victim)
                    .map(|q| {
                        q.iter()
                            .filter(|h| h.epoch + 12 >= epoch)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.end = epoch;
                        enc.deaths_log.push(DeathRecord {
                            epoch,
                            victim: victim.clone(),
                            killer: killer.clone(),
                            hits,
                        });
                        let v = enc.combatants.entry(victim.clone()).or_default();
                        v.deaths += 1;
                    }
                }
                self.recent_hits.remove(victim);
            }
            LogEvent::PowerRefresh { source, ability, target, amount, crit } => {
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        enc.assists
                            .entry(source.clone())
                            .or_default()
                            .insert(target.clone());
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
                self.push_recent_hit(
                    target,
                    RecentHit {
                        epoch,
                        attacker: "(environnement)".into(),
                        ability: None,
                        amount: *amount,
                    },
                );
                if let Some(enc) = self.current.as_mut() {
                    if epoch <= enc.end + self.timeout {
                        let t = enc.combatants.entry(target.clone()).or_default();
                        t.damage_taken += amount;
                        *t.taken_series.entry(epoch).or_default() += amount;
                    }
                }
            }
            LogEvent::ZoneEnter { zone } => {
                // Zoner termine le combat en cours.
                self.close_current();
                self.current_zone = zone.clone();
            }
            LogEvent::StartFight | LogEvent::StopFight => {}
        }

        // Log brut : conserve les lignes du combat en cours (cap 5000).
        if let Some(enc) = self.current.as_mut() {
            if epoch <= enc.end + self.timeout && enc.raw_lines.len() < 5000 {
                enc.raw_lines.push((epoch, line.message.clone()));
            }
        }
    }

    /// Encounter à afficher : l'actif sinon le dernier de l'historique.
    pub fn display_encounter(&self) -> Option<&Encounter> {
        self.current.as_ref().or_else(|| self.history.last())
    }
}

/// Agrège plusieurs encounters en un pseudo-encounter « session » :
/// les combats sont concaténés bout à bout (durée = somme des durées,
/// séries temporelles remappées pour les graphes).
pub fn aggregate_session<'a>(encs: impl IntoIterator<Item = &'a Encounter>) -> Encounter {
    let mut agg = Encounter::new(0);
    agg.finished = true;
    agg.zone = "Session".into();
    let mut offset: u64 = 0;
    for e in encs {
        let base = e.start;
        let remap = move |t: u64| offset + t.saturating_sub(base);
        for (name, c) in &e.combatants {
            agg.combatants
                .entry(name.clone())
                .or_default()
                .fold_remap(c, &remap);
        }
        for (a, ts) in &e.attacks {
            agg.attacks.entry(a.clone()).or_default().extend(ts.iter().cloned());
        }
        for (a, ts) in &e.assists {
            agg.assists.entry(a.clone()).or_default().extend(ts.iter().cloned());
        }
        agg.kills.extend(e.kills.iter().cloned());
        for d in &e.deaths_log {
            let mut d = d.clone();
            d.epoch = remap(d.epoch);
            agg.deaths_log.push(d);
        }
        // +1 : évite la collision entre la dernière seconde d'un combat
        // et la première du suivant dans les séries.
        offset += e.duration() + 1;
    }
    agg.end = offset.saturating_sub(1).max(1);
    agg
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
    fn faction_inference() {
        let parser = Parser::new("Tank");
        let mut engine = CombatEngine::new(6);
        engine.self_name = "Tank".into();
        feed(
            &mut engine,
            &parser,
            &[
                // Le tank engage le boss (mob nommé, capitalisé !)
                "(1000)[Tue May 26 17:42:26 2026] YOU hit Holly Windstalker for 100 crushing damage.",
                // Le boss tape le tank
                "(1001)[Tue May 26 17:42:27 2026] Holly Windstalker hits Tank for 500 crushing damage.",
                // Un DPS inconnu tape le boss → allié (attaque un ennemi)
                "(1002)[Tue May 26 17:42:28 2026] Wizzy's Fusion hits Holly Windstalker for 9,000 heat damage.",
                // Un soigneur soigne le tank → allié (soigne un allié)
                "(1003)[Tue May 26 17:42:29 2026] Healerguy's Salve heals Tank for 300 hit points.",
                // Un add article-minuscule tape le soigneur → ennemi
                "(1004)[Tue May 26 17:42:30 2026] a winged terror hits Healerguy for 50 crushing damage.",
            ],
        );
        let enc = engine.current.as_ref().unwrap();
        let allies = compute_allies(enc, "Tank", &engine.known_players, &HashMap::new());
        assert!(allies.contains("Tank"));
        assert!(allies.contains("Wizzy"));
        assert!(allies.contains("Healerguy"));
        // Le boss nommé/capitalisé est bien classé ennemi malgré son nom de PJ.
        assert!(!allies.contains("Holly Windstalker"));
        assert!(!allies.contains("a winged terror"));

        // Les classements filtrés ne montrent que les alliés.
        let mut display = enc.clone();
        display.allies = Some(allies);
        let names: Vec<&str> = display
            .damage_ranking()
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(names, vec!["Wizzy", "Tank"]);
    }

    #[test]
    fn death_report_records_recent_hits() {
        let parser = Parser::new("Tank");
        let mut engine = CombatEngine::new(10);
        engine.self_name = "Tank".into();
        feed(
            &mut engine,
            &parser,
            &[
                "(1000)[Tue May 26 17:42:26 2026] a dragon hits Tank for 1,000 crushing damage.",
                // Vieux coup (> 12 s avant la mort) : exclu du rapport
                "(1002)[Tue May 26 17:42:28 2026] a dragon's Tail Swipe hits Tank for 2,000 crushing damage.",
                "(1015)[Tue May 26 17:42:41 2026] a dragon's Flame Breath hits Tank for 5,000 heat damage.",
                "(1016)[Tue May 26 17:42:42 2026] Tank has been slain by a dragon!",
            ],
        );
        let enc = engine.current.as_ref().unwrap();
        assert_eq!(enc.deaths_log.len(), 1);
        let d = &enc.deaths_log[0];
        assert_eq!(d.victim, "Tank");
        assert_eq!(d.killer, "a dragon");
        // Seul le coup à -1 s est dans la fenêtre de 12 s.
        assert_eq!(d.hits.len(), 1);
        assert_eq!(d.hits[0].amount, 5000);
        assert_eq!(d.hits[0].ability.as_deref(), Some("Flame Breath"));
        assert_eq!(enc.combatants["Tank"].deaths, 1);
    }

    #[test]
    fn zone_tracking_and_per_target_maps() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        feed(
            &mut engine,
            &parser,
            &[
                "(900)[Tue May 26 17:42:00 2026] You have entered Darklight Wood.",
                "(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage.",
                "(1001)[Tue May 26 17:42:27 2026] YOU hit a bat for 40 crushing damage.",
                "(1002)[Tue May 26 17:42:28 2026] a rat hits YOU for 25 crushing damage.",
                "(1002)[Tue May 26 17:42:28 2026] a rat tries to crush YOU, but YOU parry.",
                "(1003)[Tue May 26 17:42:29 2026] Healer's Salve heals YOU for 50 hit points.",
            ],
        );
        let enc = engine.current.as_ref().unwrap();
        assert_eq!(enc.zone, "Darklight Wood");
        let me = &enc.combatants["Pawkod"];
        assert_eq!(me.damage_by_target["a rat"], 100);
        assert_eq!(me.damage_by_target["a bat"], 40);
        assert_eq!(me.taken_by_attacker["a rat"], 25);
        assert_eq!(me.heals_received_from["Healer"], 50);
        assert_eq!(me.avoids_by_kind["parade"], 1);
        assert_eq!(enc.combatants["a rat"].misses_by_kind["parade"], 1);
        assert_eq!(enc.combatants["Healer"].heals_by_target["Pawkod"], 50);
        // Log brut capturé
        assert!(enc.raw_lines.len() >= 5);
    }

    #[test]
    fn session_aggregate_concatenates() {
        let parser = Parser::new("Pawkod");
        let mut engine = CombatEngine::new(6);
        feed(
            &mut engine,
            &parser,
            &[
                // Combat 1 : 0..4 s, 300 dmg
                "(1000)[Tue May 26 17:42:26 2026] YOU hit a rat for 100 crushing damage.",
                "(1004)[Tue May 26 17:42:30 2026] YOU hit a rat for 200 crushing damage.",
                // Combat 2 (après timeout) : 100 dmg
                "(1100)[Tue May 26 17:44:06 2026] YOU hit a bat for 100 crushing damage.",
                "(1102)[Tue May 26 17:44:08 2026] YOU hit a bat for 50 crushing damage.",
            ],
        );
        engine.tick(2000);
        assert_eq!(engine.history.len(), 2);

        let agg = aggregate_session(&engine.history);
        let me = &agg.combatants["Pawkod"];
        assert_eq!(me.damage, 450);
        // Durée = somme des durées + 1 s de gap entre combats
        let d1 = engine.history[0].duration();
        let d2 = engine.history[1].duration();
        assert_eq!(agg.duration(), d1 + 1 + d2);
        // Séries remappées : combat 1 à t=0..4, combat 2 décalé après d1+1
        assert_eq!(me.dmg_series.get(&0), Some(&100));
        assert_eq!(me.dmg_series.get(&4), Some(&200));
        assert_eq!(me.dmg_series.get(&(d1 + 1)), Some(&100));
        assert_eq!(me.damage_by_target["a rat"], 300);
        assert_eq!(me.damage_by_target["a bat"], 150);
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
