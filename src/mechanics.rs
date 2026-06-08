//! Apprentissage automatique des mécaniques ennemies récurrentes, sans base de
//! données de sorts : on observe les capacités *nommées* qu'un ennemi lance sur
//! nos alliés, on mesure leur récurrence (intervalle stable) et leur impact
//! (AoE, tank buster, létal), puis on prédit le prochain cast.
//!
//! Trois sources partagent le même format `MechEntry` :
//! - `Bundled` : base communautaire embarquée dans l'exe (`include_str!`),
//! - `Learned` : ce que l'app déduit toute seule des logs (persisté localement),
//! - `Manual`  : ce que l'utilisateur renseigne à la main.
//!
//! La base locale (`mechanics.json`, à côté de l'exe) fusionne le tout ; le bouton
//! d'export produit un JSON que l'on replie dans la base embarquée des versions
//! suivantes.

use crate::combat::Encounter;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

/// Fenêtre (s) pour regrouper les ticks d'une même salve (mesure de la largeur AoE).
const SALVO_WINDOW: u64 = 2;
/// Période plancher : en dessous, c'est une auto-attaque / un DoT, pas une mécanique.
const MIN_PERIOD: f64 = 5.0;
/// Nombre d'intervalles conservés par mécanique (fenêtre d'apprentissage).
const MAX_INTERVALS: usize = 60;
/// Multiplicateur du coup entrant médian au-delà duquel un coup est « gros ».
const HEAVY_MULT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechKind {
    /// Touche plusieurs alliés à la fois.
    Aoe,
    /// Très gros coup concentré sur une cible (le tank en général).
    TankBuster,
    /// A déjà figuré dans un rapport de mort.
    Lethal,
    /// Gros pic de dégâts, sans être franchement AoE ni mortel.
    Burst,
    /// Récurrente mais sans profil d'impact marqué (souvent saisie manuelle).
    Other,
}

impl Default for MechKind {
    fn default() -> Self {
        MechKind::Other
    }
}

impl MechKind {
    pub fn label(&self) -> &'static str {
        match self {
            MechKind::Aoe => "AoE",
            MechKind::TankBuster => "Tank buster",
            MechKind::Lethal => "Mortel",
            MechKind::Burst => "Burst",
            MechKind::Other => "Mécanique",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            MechKind::Aoe => "💥",
            MechKind::TankBuster => "🛡",
            MechKind::Lethal => "☠",
            MechKind::Burst => "🔥",
            MechKind::Other => "⚠",
        }
    }

    /// Priorité de gravité : on ne « rétrograde » jamais une mécanique.
    fn severity(&self) -> u8 {
        match self {
            MechKind::Lethal => 4,
            MechKind::Aoe => 3,
            MechKind::TankBuster => 2,
            MechKind::Burst => 1,
            MechKind::Other => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechSource {
    Bundled,
    Learned,
    Manual,
}

impl Default for MechSource {
    fn default() -> Self {
        MechSource::Manual
    }
}

/// Comment alerter quand une mécanique arrive. `Inherit` = défaut global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertMode {
    /// Suit le réglage global de l'application.
    Inherit,
    /// Bandeau visuel seul.
    Visual,
    /// Bandeau + bip.
    Sound,
    /// Bandeau + synthèse vocale.
    Tts,
}

impl Default for AlertMode {
    fn default() -> Self {
        AlertMode::Inherit
    }
}

impl AlertMode {
    pub fn label(&self) -> &'static str {
        match self {
            AlertMode::Inherit => "Par défaut",
            AlertMode::Visual => "Visuel",
            AlertMode::Sound => "Son",
            AlertMode::Tts => "Voix",
        }
    }
}

/// Définition d'une mécanique : commune aux trois sources, sérialisable, partageable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MechEntry {
    /// Zone où la mécanique a été observée (`""` = toutes zones).
    pub zone: String,
    /// Mob source principal (`""` = n'importe quel mob de la zone).
    pub mob: String,
    /// Nom de la capacité (clé d'identité avec la zone).
    pub ability: String,
    /// Période estimée en secondes (`0` = non chronométrée).
    pub period: f64,
    /// Secondes d'avance pour l'alerte avant le prochain cast.
    pub lead: u64,
    pub kind: MechKind,
    /// Texte d'alerte custom (`""` = automatique « <icône> <capacité> dans N »).
    pub message: String,
    /// Mode d'alerte (surcharge le défaut global).
    pub alert: AlertMode,
    /// Mécanique active (suivie et alertée).
    pub enabled: bool,
    pub source: MechSource,
    // --- Métadonnées d'apprentissage (confiance / impact) ---
    /// Intervalles inter-cast observés (au sein d'un même pull), capés.
    pub intervals: Vec<u64>,
    /// Nombre d'intervalles concordants avec la période (échantillons).
    pub samples: u32,
    /// Nombre total de casts observés.
    pub casts_seen: u32,
    /// Plus gros coup d'un seul tick.
    pub max_hit: u64,
    /// Largeur AoE max : cibles distinctes touchées en une salve.
    pub max_targets: u32,
    /// Combien de fois la capacité est apparue dans un rapport de mort.
    pub lethal: u32,
}

impl Default for MechEntry {
    fn default() -> Self {
        Self {
            zone: String::new(),
            mob: String::new(),
            ability: String::new(),
            period: 0.0,
            lead: 5,
            kind: MechKind::Other,
            message: String::new(),
            alert: AlertMode::Inherit,
            enabled: true,
            source: MechSource::Manual,
            intervals: Vec::new(),
            samples: 0,
            casts_seen: 0,
            max_hit: 0,
            max_targets: 0,
            lethal: 0,
        }
    }
}

impl MechEntry {
    /// Mécanique chronométrée (période exploitable pour un compte à rebours) ?
    pub fn is_timed(&self) -> bool {
        self.period >= MIN_PERIOD
    }

    /// Assez marquante pour être surfacée (alerte/affichage) ?
    /// (Utilisé par l'outil de minage hors-ligne pour filtrer la base communautaire.)
    #[allow(dead_code)]
    pub fn is_noteworthy(&self) -> bool {
        self.lethal > 0
            || self.max_targets >= 3
            || matches!(self.kind, MechKind::TankBuster | MechKind::Burst)
            || (self.is_timed() && matches!(self.source, MechSource::Manual))
    }

    /// Score d'impact pour le classement (létal > AoE large > gros coup).
    pub fn impact_score(&self) -> f64 {
        let mut s = self.kind.severity() as f64 * 1000.0;
        s += (self.lethal as f64) * 500.0;
        s += (self.max_targets as f64) * 100.0;
        s += (self.max_hit as f64).log10().max(0.0) * 10.0;
        s += self.samples as f64;
        s
    }

    /// Recalcule la période depuis les intervalles accumulés.
    pub fn reestimate(&mut self) {
        let (p, s) = estimate_period(&self.intervals);
        self.period = p;
        self.samples = s;
    }

    /// Cumule les observations d'une autre entrée de même clé (agrégation multi-logs).
    /// N'écrase pas une entrée manuelle (période/kind figés par l'utilisateur).
    pub fn absorb(&mut self, other: &MechEntry) {
        let manual = matches!(self.source, MechSource::Manual);
        if self.mob.is_empty() {
            self.mob = other.mob.clone();
        }
        self.intervals.extend(other.intervals.iter().copied());
        if self.intervals.len() > MAX_INTERVALS {
            let drop = self.intervals.len() - MAX_INTERVALS;
            self.intervals.drain(0..drop);
        }
        self.casts_seen += other.casts_seen;
        self.max_hit = self.max_hit.max(other.max_hit);
        self.max_targets = self.max_targets.max(other.max_targets);
        self.lethal += other.lethal;
        if !manual && other.kind.severity() >= self.kind.severity() {
            self.kind = other.kind;
        }
        if !manual {
            self.reestimate();
        }
    }
}

/// Base de mécaniques : collection plate, fusion par clé (zone, capacité).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MechanicsDb {
    pub entries: Vec<MechEntry>,
}

impl MechanicsDb {
    /// Base embarquée (communautaire) compilée dans l'exe.
    pub fn bundled() -> Self {
        const RAW: &str = include_str!("../assets/mechanics.json");
        serde_json::from_str(RAW).unwrap_or_default()
    }

    pub fn from_str(s: &str) -> Option<Self> {
        serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
    }

    /// Clé d'identité : (zone, mob, capacité). Le mob discrimine les boss qui
    /// partagent un nom de capacité (« Assault » n'a pas la même période partout).
    fn index_of(&self, zone: &str, mob: &str, ability: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.zone == zone && e.mob == mob && e.ability == ability)
    }

    pub fn find(&self, zone: &str, mob: &str, ability: &str) -> Option<&MechEntry> {
        self.index_of(zone, mob, ability).map(|i| &self.entries[i])
    }

    /// Insère ou remplace une entrée par sa clé.
    pub fn upsert(&mut self, entry: MechEntry) {
        match self.index_of(&entry.zone, &entry.mob, &entry.ability) {
            Some(i) => self.entries[i] = entry,
            None => self.entries.push(entry),
        }
    }

    /// Fusionne `other` : les entrées absentes sont ajoutées. Les présentes ne sont
    /// écrasées que si `prefer_other` (utilisé pour faire primer le local sur l'embarqué).
    pub fn merge(&mut self, other: &MechanicsDb, prefer_other: bool) {
        for e in &other.entries {
            match self.index_of(&e.zone, &e.mob, &e.ability) {
                Some(i) => {
                    if prefer_other {
                        self.entries[i] = e.clone();
                    }
                }
                None => self.entries.push(e.clone()),
            }
        }
    }

    /// Cumule les observations d'une autre base (agrégation multi-logs / multi-perso).
    pub fn absorb_db(&mut self, other: &MechanicsDb) {
        for e in &other.entries {
            match self.index_of(&e.zone, &e.mob, &e.ability) {
                Some(i) => self.entries[i].absorb(e),
                None => self.entries.push(e.clone()),
            }
        }
    }

    pub fn save_to(&self, path: &PathBuf) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Chemin de la base locale, à côté de l'exécutable.
pub fn local_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("mechanics.json")))
        .unwrap_or_else(|| PathBuf::from("mechanics.json"))
}

/// Charge la base de travail : embarquée, puis recouverte par la base locale.
pub fn load_db() -> MechanicsDb {
    let mut db = MechanicsDb::bundled();
    if let Ok(s) = std::fs::read_to_string(local_path()) {
        if let Some(local) = MechanicsDb::from_str(&s) {
            db.merge(&local, true);
        }
    }
    db
}

/// Un tick encaissé brut, en attente de résolution alliés/ennemis.
#[derive(Debug, Clone)]
struct RawCast {
    epoch: u64,
    attacker: String,
    ability: String,
    target: String,
    amount: u64,
}

/// Apprenant : alimente la base depuis le flux de combat et prédit les prochains casts.
pub struct Learner {
    /// Base de travail (embarquée + locale + apprentissages).
    pub db: MechanicsDb,
    /// Zone courante (qualifie les mécaniques apprises).
    pub zone: String,
    /// Capacités nommées encaissées dans le combat en cours (résolues à la clôture).
    scratch: Vec<RawCast>,
    /// Dernier cast vu par capacité (epoch, attaquant) dans le combat en cours.
    last_cast: HashMap<String, (u64, String)>,
    /// Y a-t-il eu un apprentissage depuis la dernière sauvegarde ?
    pub dirty: bool,
}

/// Prédiction live d'un prochain cast.
#[derive(Debug, Clone)]
pub struct Prediction {
    pub ability: String,
    pub kind: MechKind,
    pub period: f64,
    /// Secondes avant le prochain cast (négatif = en retard / imminent).
    pub eta: f64,
    pub lead: u64,
    pub message: String,
    pub alert: AlertMode,
}

impl Learner {
    pub fn new() -> Self {
        Self {
            db: load_db(),
            zone: String::new(),
            scratch: Vec::new(),
            last_cast: HashMap::new(),
            dirty: false,
        }
    }

    /// À appeler pour chaque coup à capacité nommée (l'enemy/ally est tranché à la clôture).
    pub fn observe(&mut self, epoch: u64, attacker: &str, ability: &str, target: &str, amount: u64) {
        if self.scratch.len() < 40_000 {
            self.scratch.push(RawCast {
                epoch,
                attacker: attacker.to_string(),
                ability: ability.to_string(),
                target: target.to_string(),
                amount,
            });
        }
        self.last_cast
            .insert(ability.to_string(), (epoch, attacker.to_string()));
    }

    /// Réinitialise l'état lié au combat (à la clôture d'un encounter).
    pub fn reset_encounter(&mut self) {
        self.scratch.clear();
        self.last_cast.clear();
    }

    /// Analyse l'encounter clos et met à jour les mécaniques apprises.
    /// `allies` = ensemble des alliés calculé pour cet encounter.
    pub fn learn_from(&mut self, enc: &Encounter, allies: &BTreeSet<String>) {
        if self.scratch.is_empty() {
            return;
        }
        let zone = if self.zone.is_empty() {
            enc.zone.clone()
        } else {
            self.zone.clone()
        };

        // Référence : coup entrant médian sur les alliés (échelle d'impact relative,
        // indépendante du niveau / des PV).
        let mut incoming: Vec<u64> = self
            .scratch
            .iter()
            .filter(|c| allies.contains(&c.target) && !allies.contains(&c.attacker))
            .map(|c| c.amount)
            .collect();
        let reference = if incoming.is_empty() {
            0.0
        } else {
            incoming.sort_unstable();
            incoming[incoming.len() / 2] as f64
        };

        // Groupe les casts ennemi (∉ alliés) → allié (∈ alliés) par capacité.
        let mut by_ability: BTreeMap<String, Vec<&RawCast>> = BTreeMap::new();
        for c in &self.scratch {
            if !allies.contains(&c.attacker) && allies.contains(&c.target) {
                by_ability.entry(c.ability.clone()).or_default().push(c);
            }
        }

        for (ability, mut casts) in by_ability {
            casts.sort_by_key(|c| c.epoch);

            // Détection des salves : nouveau cast si > SALVO_WINDOW depuis le début de salve.
            let mut salvo_starts: Vec<u64> = Vec::new();
            let mut salvo_start = 0u64;
            let mut salvo_targets: BTreeSet<&str> = BTreeSet::new();
            let mut max_targets = 0u32;
            let mut max_hit = 0u64;
            let mut top_mob = String::new();
            let mut mob_dmg: BTreeMap<&str, u64> = BTreeMap::new();

            for c in &casts {
                if salvo_starts.is_empty() || c.epoch > salvo_start + SALVO_WINDOW {
                    salvo_starts.push(c.epoch);
                    salvo_start = c.epoch;
                    salvo_targets.clear();
                }
                salvo_targets.insert(c.target.as_str());
                max_targets = max_targets.max(salvo_targets.len() as u32);
                max_hit = max_hit.max(c.amount);
                *mob_dmg.entry(c.attacker.as_str()).or_default() += c.amount;
            }
            if let Some((m, _)) = mob_dmg.iter().max_by_key(|(_, v)| **v) {
                top_mob = m.to_string();
            }

            // Intervalles intra-pull (entre salves successives).
            let intervals: Vec<u64> = salvo_starts
                .windows(2)
                .map(|w| w[1] - w[0])
                .filter(|&d| d > 0)
                .collect();

            // Létalité : la capacité figure-t-elle dans un rapport de mort allié ?
            let lethal = enc
                .deaths_log
                .iter()
                .filter(|d| allies.contains(&d.victim))
                .filter(|d| d.hits.iter().any(|h| h.ability.as_deref() == Some(ability.as_str())))
                .count() as u32;

            // Classification d'impact (relative à la référence du pull).
            let heavy = reference > 0.0 && max_hit as f64 >= reference * HEAVY_MULT;
            let kind = if lethal > 0 {
                MechKind::Lethal
            } else if max_targets >= 3 {
                MechKind::Aoe
            } else if heavy && max_targets <= 2 {
                MechKind::TankBuster
            } else if heavy {
                MechKind::Burst
            } else {
                MechKind::Other
            };

            let noteworthy = lethal > 0 || max_targets >= 3 || heavy;
            if !noteworthy {
                continue;
            }

            // Upsert dans la base apprise (cumul des observations), identifiée
            // par (zone, mob, capacité).
            let mut entry = self
                .db
                .find(&zone, &top_mob, &ability)
                .cloned()
                .unwrap_or_else(|| MechEntry {
                    zone: zone.clone(),
                    mob: top_mob.clone(),
                    ability: ability.clone(),
                    source: MechSource::Learned,
                    ..Default::default()
                });
            // Une entrée manuelle n'est pas écrasée par l'apprentissage (on enrichit juste les stats).
            let manual = matches!(entry.source, MechSource::Manual);

            if entry.mob.is_empty() {
                entry.mob = top_mob;
            }
            entry.intervals.extend(intervals);
            if entry.intervals.len() > MAX_INTERVALS {
                let drop = entry.intervals.len() - MAX_INTERVALS;
                entry.intervals.drain(0..drop);
            }
            entry.casts_seen += salvo_starts.len() as u32;
            entry.max_hit = entry.max_hit.max(max_hit);
            entry.max_targets = entry.max_targets.max(max_targets);
            entry.lethal += lethal;
            if !manual && kind.severity() >= entry.kind.severity() {
                entry.kind = kind;
            }

            let (period, samples) = estimate_period(&entry.intervals);
            if !manual {
                entry.period = period;
            }
            entry.samples = samples;

            self.db.upsert(entry);
            self.dirty = true;
        }
    }

    /// Prédictions live à l'instant `now` (epoch) : une par capacité chronométrée
    /// effectivement vue dans le combat en cours. Quand plusieurs entrées portent
    /// le même nom de capacité (boss différents), on choisit la meilleure selon
    /// l'attaquant observé puis la zone courante.
    pub fn predictions(&self, now: u64) -> Vec<Prediction> {
        let mut out = Vec::new();
        for (ability, (last, attacker)) in &self.last_cast {
            // Candidats : entrées actives, chronométrées, de cette capacité.
            let mut best: Option<&MechEntry> = None;
            let mut best_score = -1i32;
            for e in &self.db.entries {
                if !e.enabled || !e.is_timed() || &e.ability != ability {
                    continue;
                }
                // Score de pertinence : mob exact > zone exacte > générique.
                let score = if !e.mob.is_empty() && &e.mob == attacker {
                    3
                } else if !e.zone.is_empty() && e.zone == self.zone {
                    2
                } else if e.mob.is_empty() && e.zone.is_empty() {
                    1
                } else {
                    0
                };
                if score > best_score {
                    best_score = score;
                    best = Some(e);
                }
            }
            let Some(e) = best else { continue };
            // Prochain cast attendu = dernier + période (on saute les périodes passées).
            let mut next = *last as f64 + e.period;
            while next < now as f64 - e.period * 0.5 {
                next += e.period;
            }
            out.push(Prediction {
                ability: e.ability.clone(),
                kind: e.kind,
                period: e.period,
                eta: next - now as f64,
                lead: e.lead,
                message: e.message.clone(),
                alert: e.alert,
            });
        }
        out.sort_by(|a, b| a.eta.partial_cmp(&b.eta).unwrap());
        out
    }

    /// Sauvegarde la base locale si elle a changé.
    pub fn save_if_dirty(&mut self) {
        if self.dirty {
            self.db.save_to(&local_path());
            self.dirty = false;
        }
    }
}

impl Default for Learner {
    fn default() -> Self {
        Self::new()
    }
}

/// Estime une période robuste depuis des intervalles : médiane si ≥60 % des
/// intervalles s'y accordent (±30 %) et qu'il y a au moins 3 échantillons concordants.
/// Retourne `(0.0, 0)` si rien de stable.
fn estimate_period(intervals: &[u64]) -> (f64, u32) {
    if intervals.len() < 3 {
        return (0.0, 0);
    }
    let mut s: Vec<u64> = intervals.to_vec();
    s.sort_unstable();
    let med = s[s.len() / 2] as f64;
    if med < MIN_PERIOD {
        return (0.0, 0);
    }
    let on = s
        .iter()
        .filter(|&&d| {
            let r = d as f64 / med;
            r > 0.7 && r < 1.3
        })
        .count();
    if on >= 3 && on as f64 / s.len() as f64 >= 0.6 {
        (med, on as u32)
    } else {
        (0.0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatEngine;
    use crate::parser::Parser;
    use std::collections::HashSet;

    fn feed(engine: &mut CombatEngine, parser: &Parser, lines: &[String]) {
        for l in lines {
            if let Some(p) = parser.parse_line(l) {
                engine.process(&p);
            }
        }
    }

    #[test]
    fn estimate_period_stable_and_noisy() {
        // Stable autour de 30 s.
        assert_eq!(estimate_period(&[30, 31, 29, 30, 30]).0, 30.0);
        // Trop court (DoT) → rejeté.
        assert_eq!(estimate_period(&[2, 2, 3, 2]).0, 0.0);
        // Chaotique → rejeté.
        assert_eq!(estimate_period(&[10, 45, 12, 60]).0, 0.0);
    }

    #[test]
    fn learns_periodic_aoe_and_predicts() {
        let parser = Parser::new("Tank");
        let mut engine = CombatEngine::new(6);
        engine.self_name = "Tank".into();
        // Le boss lance "Flame Nova" toutes les 30 s sur 3 alliés (AoE). Le DPS
        // continu (filler toutes les 3 s) maintient un seul encounter, comme en raid.
        // Le tank engage le boss (faction) et un soigneur soigne le tank (allié).
        let mut events: Vec<(u64, String)> = vec![
            (1000, "Healy's Mend heals Tank for 50 hit points.".to_string()),
        ];
        for s in (1000u64..=1130).step_by(3) {
            events.push((s, "YOU hit a dread boss for 100 crushing damage.".to_string()));
        }
        for k in 0..5u64 {
            let t = 1000 + k * 30;
            events.push((t, "a dread boss's Flame Nova hits Tank for 1000 heat damage.".to_string()));
            events.push((t, "a dread boss's Flame Nova hits Healy for 1000 heat damage.".to_string()));
            events.push((t, "a dread boss's Flame Nova hits Dpser for 1000 heat damage.".to_string()));
        }
        events.sort_by_key(|(t, _)| *t);
        let lines: Vec<String> = events
            .into_iter()
            .map(|(t, m)| format!("({t})[Tue May 26 17:42:26 2026] {m}"))
            .collect();
        feed(&mut engine, &parser, &lines);
        // Clôt l'encounter → déclenche l'apprentissage.
        engine.tick(2000);

        let entry = engine
            .mech
            .db
            .find("", "a dread boss", "Flame Nova")
            .expect("Flame Nova appris");
        assert_eq!(entry.period, 30.0);
        assert_eq!(entry.kind, MechKind::Aoe);
        assert!(entry.max_targets >= 3);
        assert!(entry.is_noteworthy());
    }

    #[test]
    fn ignores_ally_abilities() {
        let parser = Parser::new("Tank");
        let mut engine = CombatEngine::new(6);
        engine.self_name = "Tank".into();
        let mut lines = vec!["(1000)[x] YOU hit a boss for 100 crushing damage.".to_string()];
        // Un allié spamme un sort périodique sur le boss : ne doit jamais devenir une mécanique.
        for k in 0..6u64 {
            let t = 1000 + k * 10;
            lines.push(format!("({t})[x] Wizzy's Fireball hits a boss for 9000 heat damage."));
        }
        feed(&mut engine, &parser, &lines);
        engine.tick(2000);
        assert!(!engine.mech.db.entries.iter().any(|e| e.ability == "Fireball"));
        let _ = HashSet::<String>::new();
    }

    #[test]
    fn predicts_next_cast() {
        let mut l = Learner::new();
        l.db.entries.clear();
        l.zone = "Z".into();
        l.db.entries.push(MechEntry {
            zone: "Z".into(),
            ability: "Nova".into(),
            period: 30.0,
            lead: 5,
            kind: MechKind::Aoe,
            source: MechSource::Bundled,
            ..Default::default()
        });
        // Un cast vu à t=1000 → prochain attendu à t=1030.
        l.observe(1000, "boss", "Nova", "Tank", 500);
        let preds = l.predictions(1010);
        assert_eq!(preds.len(), 1);
        assert!((preds[0].eta - 20.0).abs() < 0.01, "eta = {}", preds[0].eta);
        // Sans cast vu, aucune prédiction (on ne devine pas le premier cast).
        l.reset_encounter();
        assert!(l.predictions(1010).is_empty());
    }

    #[test]
    fn db_merge_prefers_local() {
        let mut base = MechanicsDb::default();
        base.entries.push(MechEntry {
            zone: "Z".into(),
            ability: "A".into(),
            period: 10.0,
            source: MechSource::Bundled,
            ..Default::default()
        });
        let mut local = MechanicsDb::default();
        local.entries.push(MechEntry {
            zone: "Z".into(),
            ability: "A".into(),
            period: 42.0,
            source: MechSource::Manual,
            ..Default::default()
        });
        base.merge(&local, true);
        assert_eq!(base.entries.len(), 1);
        assert_eq!(base.find("Z", "", "A").unwrap().period, 42.0);
    }
}
