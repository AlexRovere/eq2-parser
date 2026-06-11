//! Optimisation de séquence de sorts (Piste A).
//!
//! On classe les sorts offensifs du joueur par « efficacité » = dégâts par
//! seconde de GCD, à la manière du classeur EQ2 Spell Efficiency, mais en se
//! basant sur le réel observé dans les logs plutôt que sur une saisie manuelle.
//!
//! Deux sources se combinent :
//!   - le `Profiler` mesure dans les logs, par sort, les dégâts par cast, le
//!     taux de crit, le nombre de cibles et la cadence (détection AoE/DoT) ;
//!   - la base de sorts bundlée (`spells.json`, extraite du wiki) fournit le
//!     temps de cast, le recast et le type (mono/encounter/pbaoe, dd/dot).
//!
//! Le temps de cast effectif applique les stats du joueur (casting speed),
//! et peut être surchargé à la main pour les sorts absents de la base.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Plafond EQ2 (approx.) du bonus de vitesse d'incantation, en %.
pub const CASTING_SPEED_CAP: f32 = 100.0;
/// Plafond du bonus de réutilisation, en %.
pub const REUSE_SPEED_CAP: f32 = 100.0;
/// Plancher de temps de cast (GCD mini EQ2), en secondes.
pub const MIN_CAST: f32 = 0.5;
/// Récupération (recovery) par défaut ajoutée au temps de cast, en secondes.
pub const DEFAULT_RECOVERY: f32 = 0.5;

// ---------------------------------------------------------------------------
// Base de sorts bundlée
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Single,
    Encounter,
    Pbaoe,
    Other,
}

impl Target {
    fn parse(s: &str) -> Self {
        match s {
            "single" => Target::Single,
            "encounter" => Target::Encounter,
            "pbaoe" => Target::Pbaoe,
            _ => Target::Other,
        }
    }
    /// Nombre de cibles réellement touchées dans le scénario donné.
    fn effective_targets(self, sc: &Scenario, observed: f64) -> f64 {
        match self {
            Target::Single => 1.0,
            // AoE au sol : plafonnée à 8 cibles dans EQ2.
            Target::Pbaoe => (sc.targets as f64).min(8.0).max(1.0),
            // AoE d'encounter : touche tout le groupe lié, sinon une seule cible.
            Target::Encounter => {
                if sc.linked {
                    (sc.targets as f64).max(1.0)
                } else {
                    1.0
                }
            }
            // Type inconnu : on garde ce qu'on a observé.
            Target::Other => observed.max(1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mech {
    Dd,
    Dot,
    Other,
}

impl Mech {
    fn parse(s: &str) -> Self {
        match s {
            "dd" => Mech::Dd,
            "dot" => Mech::Dot,
            _ => Mech::Other,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawSpell {
    name: String,
    class: String,
    target: Option<String>,
    mechanic: Option<String>,
    cast: Option<f32>,
    recast: Option<f32>,
    duration: Option<f32>,
    damage_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawSpells {
    spells: Vec<RawSpell>,
}

#[derive(Debug, Clone)]
pub struct SpellInfo {
    #[allow(dead_code)]
    pub name: String,
    pub class: String,
    pub cast: f32,
    pub recast: Option<f32>,
    /// Durée du DoT (s) ; None pour les directs / instantanés.
    pub duration: Option<f32>,
    pub target: Target,
    pub mechanic: Mech,
    #[allow(dead_code)]
    pub damage_type: Option<String>,
}

/// Base de sorts indexée par nom normalisé (insensible à la casse, sans le
/// suffixe de classe « (Warlock) » que le wiki ajoute aux homonymes).
pub struct SpellDb {
    by_name: HashMap<String, Vec<SpellInfo>>,
    by_class: HashMap<String, HashSet<String>>,
}

fn normalize(name: &str) -> String {
    let base = name.split(" (").next().unwrap_or(name);
    base.trim().to_lowercase()
}

impl SpellDb {
    pub fn bundled() -> Self {
        let raw: RawSpells =
            serde_json::from_str(include_str!("../assets/spells.json")).unwrap_or(RawSpells {
                spells: Vec::new(),
            });
        let mut by_name: HashMap<String, Vec<SpellInfo>> = HashMap::new();
        let mut by_class: HashMap<String, HashSet<String>> = HashMap::new();
        for s in raw.spells {
            // Seuls les sorts qui font des dégâts nous intéressent ici, mais on
            // garde tout : la résolution de classe profite des buffs aussi.
            let key = normalize(&s.name);
            if matches!(s.mechanic.as_deref(), Some("dd") | Some("dot")) {
                by_class
                    .entry(s.class.clone())
                    .or_default()
                    .insert(key.clone());
            }
            by_name.entry(key).or_default().push(SpellInfo {
                name: s.name,
                class: s.class.clone(),
                cast: s.cast.unwrap_or(0.0),
                recast: s.recast,
                duration: s.duration,
                target: Target::parse(s.target.as_deref().unwrap_or("")),
                mechanic: Mech::parse(s.mechanic.as_deref().unwrap_or("")),
                damage_type: s.damage_type,
            });
        }
        Self { by_name, by_class }
    }

    /// Sort correspondant à un nom de log, en préférant la classe indiquée
    /// (désambiguïse les homonymes entre classes).
    pub fn lookup(&self, ability: &str, class_hint: Option<&str>) -> Option<&SpellInfo> {
        let cands = self.by_name.get(&normalize(ability))?;
        if let Some(cls) = class_hint {
            if let Some(hit) = cands.iter().find(|c| c.class == cls) {
                return Some(hit);
            }
        }
        cands.first()
    }

    /// Liste triée des classes connues de la base.
    pub fn classes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_class.keys().cloned().collect();
        v.sort();
        v
    }

    /// Sorts offensifs (dd/dot) d'une classe donnée.
    pub fn class_spells(&self, class: &str) -> Vec<&SpellInfo> {
        let mut out: Vec<&SpellInfo> = self
            .by_name
            .values()
            .flatten()
            .filter(|s| s.class == class && matches!(s.mechanic, Mech::Dd | Mech::Dot))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Devine la classe d'un personnage d'après les sorts qu'il a lancés.
    pub fn infer_class<'a>(&self, abilities: impl Iterator<Item = &'a String>) -> Option<String> {
        let cast: HashSet<String> = abilities.map(|a| normalize(a)).collect();
        let mut best: Option<(usize, &String)> = None;
        for (cls, names) in &self.by_class {
            let score = cast.iter().filter(|n| names.contains(*n)).count();
            if score > 0 && best.is_none_or(|(b, _)| score > b) {
                best = Some((score, cls));
            }
        }
        best.map(|(_, c)| c.clone())
    }
}

// ---------------------------------------------------------------------------
// Profiler : mesure par sort depuis les logs
// ---------------------------------------------------------------------------

/// Observations cumulées d'un sort sur toute la session analysée.
#[derive(Debug, Clone, Default)]
pub struct AbilityObs {
    pub casts: u32,
    pub total_damage: u64,
    /// Somme des cibles touchées par cast (pour la moyenne).
    pub total_targets: u64,
    pub hits: u32,
    pub crits: u32,
    pub max_hit: u64,
    /// Fréquence de tick inférée (s) ; 0 = sort direct (pas de DoT).
    pub freq: f32,
    pub last_seen: u64,
}

impl AbilityObs {
    pub fn dmg_per_cast(&self) -> f64 {
        if self.casts == 0 {
            0.0
        } else {
            self.total_damage as f64 / self.casts as f64
        }
    }
    pub fn avg_targets(&self) -> f64 {
        if self.casts == 0 {
            1.0
        } else {
            (self.total_targets as f64 / self.casts as f64).max(1.0)
        }
    }
    pub fn crit_rate(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.crits as f64 / self.hits as f64 * 100.0
        }
    }
    pub fn is_dot(&self) -> bool {
        self.freq > 0.0
    }
}

#[derive(Default)]
struct CastScratch {
    per_target: HashMap<String, Vec<u64>>,
    amount: u64,
    crits: u32,
    hits: u32,
    max_hit: u64,
}

/// Mesure par personnage les profils de sorts au fil des encounters.
#[derive(Default)]
pub struct Profiler {
    /// Cumul session : personnage → sort → observations.
    pub chars: HashMap<String, HashMap<String, AbilityObs>>,
    /// Temps de combat actif cumulé par personnage (s).
    pub time: HashMap<String, u64>,
    /// Brouillon de l'encounter en cours : (perso, sort) → coups bruts.
    scratch: HashMap<(String, String), CastScratch>,
    /// Dernier tick observé par (perso, sort) — sert à repérer les réapplications.
    last_hit: HashMap<(String, String), u64>,
    /// Dernière (ré)application détectée par (perso, sort) : état live pour la
    /// rotation. Survit au `flush` (au contraire du `scratch`) pour rester
    /// disponible entre deux ticks d'un même combat.
    last_cast: HashMap<(String, String), u64>,
    pub dirty: bool,
}

impl Profiler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre un coup d'un sort lancé par un joueur.
    pub fn observe(&mut self, epoch: u64, attacker: &str, ability: &str, target: &str, amount: u64, crit: bool) {
        // État live pour la rotation : repère les (ré)applications. Un sort
        // direct ré-applique à chaque coup ; un DoT seulement après un trou
        // supérieur à sa cadence de tick (sinon ses ticks réguliers passeraient
        // pour autant de recasts).
        let key = (attacker.to_string(), ability.to_string());
        let freq = self
            .chars
            .get(attacker)
            .and_then(|m| m.get(ability))
            .map(|o| o.freq)
            .unwrap_or(0.0);
        let gap = if freq > 0.0 { (freq * 2.0).max(2.0) } else { 4.0 };
        let new_app = self
            .last_hit
            .get(&key)
            .is_none_or(|&p| epoch.saturating_sub(p) as f32 > gap);
        if new_app {
            self.last_cast.insert(key.clone(), epoch);
        }
        self.last_hit.insert(key, epoch);

        let s = self
            .scratch
            .entry((attacker.to_string(), ability.to_string()))
            .or_default();
        s.per_target.entry(target.to_string()).or_default().push(epoch);
        s.amount += amount;
        s.hits += 1;
        if crit {
            s.crits += 1;
        }
        s.max_hit = s.max_hit.max(amount);
        self.dirty = true;
    }

    /// Temps de combat actif d'un perso (cumul + encounter en cours).
    pub fn combat_time(&self, ch: &str) -> u64 {
        let mut t = *self.time.get(ch).unwrap_or(&0);
        if let Some((lo, hi)) = self.scratch_span(ch) {
            t += hi - lo;
        }
        t
    }

    /// Étendue temporelle des coups d'un perso dans l'encounter en cours.
    fn scratch_span(&self, ch: &str) -> Option<(u64, u64)> {
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        for ((c, _), sc) in &self.scratch {
            if c != ch {
                continue;
            }
            for times in sc.per_target.values() {
                for &ts in times {
                    lo = lo.min(ts);
                    hi = hi.max(ts);
                }
            }
        }
        (lo != u64::MAX).then_some((lo, hi))
    }

    /// Replie le brouillon de l'encounter dans le cumul session, puis le vide.
    pub fn flush(&mut self) {
        // Temps de combat actif par perso = étendue de ses coups dans l'encounter.
        let mut span: HashMap<String, (u64, u64)> = HashMap::new();
        for ((ch, _), sc) in &self.scratch {
            for times in sc.per_target.values() {
                for &ts in times {
                    let e = span.entry(ch.clone()).or_insert((ts, ts));
                    e.0 = e.0.min(ts);
                    e.1 = e.1.max(ts);
                }
            }
        }
        for (ch, (lo, hi)) in span {
            *self.time.entry(ch).or_default() += hi - lo;
        }
        let scratch = std::mem::take(&mut self.scratch);
        for ((ch, ability), sc) in scratch {
            let obs = fold(&sc);
            let dst = self
                .chars
                .entry(ch)
                .or_default()
                .entry(ability)
                .or_default();
            dst.casts += obs.casts;
            dst.total_damage += obs.total_damage;
            dst.total_targets += obs.total_targets;
            dst.hits += obs.hits;
            dst.crits += obs.crits;
            dst.max_hit = dst.max_hit.max(obs.max_hit);
            if obs.freq > 0.0 {
                dst.freq = obs.freq;
            }
            dst.last_seen = dst.last_seen.max(obs.last_seen);
        }
    }

    /// Vue cumul + brouillon de l'encounter courant, sans rien modifier
    /// (pour l'affichage live pendant un combat en cours).
    pub fn live(&self, ch: &str) -> HashMap<String, AbilityObs> {
        let mut out = self.chars.get(ch).cloned().unwrap_or_default();
        for ((c, ability), sc) in &self.scratch {
            if c != ch {
                continue;
            }
            let obs = fold(sc);
            let dst = out.entry(ability.clone()).or_default();
            dst.casts += obs.casts;
            dst.total_damage += obs.total_damage;
            dst.total_targets += obs.total_targets;
            dst.hits += obs.hits;
            dst.crits += obs.crits;
            dst.max_hit = dst.max_hit.max(obs.max_hit);
            if obs.freq > 0.0 {
                dst.freq = obs.freq;
            }
            dst.last_seen = dst.last_seen.max(obs.last_seen);
        }
        out
    }

    /// État live de la rotation : dernier instant de (ré)application par sort
    /// pour un personnage (clé = nom de sort).
    pub fn last_casts(&self, ch: &str) -> HashMap<String, u64> {
        self.last_cast
            .iter()
            .filter(|((c, _), _)| c == ch)
            .map(|((_, a), t)| (a.clone(), *t))
            .collect()
    }

    /// Personnages profilés ayant lancé au moins un sort (cumul ou en cours).
    pub fn known_chars(&self) -> Vec<String> {
        let mut set: HashSet<String> = self.chars.keys().cloned().collect();
        for (c, _) in self.scratch.keys() {
            set.insert(c.clone());
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    }
}

/// Médiane d'une liste (modifie l'ordre).
fn median(v: &mut [u64]) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// Reconstruit les casts d'un sort à partir des coups bruts d'un encounter.
fn fold(sc: &CastScratch) -> AbilityObs {
    // 1) Fréquence de tick : médiane des petits écarts entre coups d'une même
    //    cible (un DoT tique à intervalle régulier ; un direct n'a pas d'écart court).
    let mut small_gaps: Vec<u64> = Vec::new();
    for times in sc.per_target.values() {
        let mut t = times.clone();
        t.sort_unstable();
        for w in t.windows(2) {
            let g = w[1] - w[0];
            if g >= 1 && g <= 6 {
                small_gaps.push(g);
            }
        }
    }
    let freq = median(&mut small_gaps);

    // 2) Applications (= (re)lancements) : pour un direct, chaque coup ; pour un
    //    DoT, le premier tick puis chaque tick dont l'écart dépasse ~1,8x freq.
    let reapply = (freq as f64 * 1.8).ceil() as u64;
    let mut apps: Vec<(u64, String)> = Vec::new();
    for (target, times) in &sc.per_target {
        let mut t = times.clone();
        t.sort_unstable();
        let mut prev: Option<u64> = None;
        for &ts in &t {
            let new_app = match prev {
                None => true,
                Some(p) => freq == 0 || ts.saturating_sub(p) > reapply,
            };
            if new_app {
                apps.push((ts, target.clone()));
            }
            prev = Some(ts);
        }
    }

    // 3) Clustering par seconde : des applications simultanées sur plusieurs
    //    cibles (AoE) comptent pour un seul cast touchant N cibles.
    apps.sort_by_key(|a| a.0);
    let mut casts = 0u32;
    let mut total_targets = 0u64;
    let mut last_seen = 0u64;
    let mut i = 0;
    while i < apps.len() {
        let start = apps[i].0;
        let mut targets: HashSet<&str> = HashSet::new();
        while i < apps.len() && apps[i].0 <= start + 1 {
            targets.insert(apps[i].1.as_str());
            last_seen = last_seen.max(apps[i].0);
            i += 1;
        }
        casts += 1;
        total_targets += targets.len() as u64;
    }

    AbilityObs {
        casts: casts.max(1),
        total_damage: sc.amount,
        total_targets: total_targets.max(1),
        hits: sc.hits,
        crits: sc.crits,
        max_hit: sc.max_hit,
        freq: freq as f32,
        last_seen,
    }
}

// ---------------------------------------------------------------------------
// Modèle d'efficacité
// ---------------------------------------------------------------------------

/// Scénario de combat choisi par l'utilisateur (filtres).
#[derive(Debug, Clone, Copy)]
pub struct Scenario {
    pub targets: u32,
    pub linked: bool,
}

/// Stats offensives du personnage (saisies à la main, persistées par perso).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerStats {
    /// Bonus de vitesse d'incantation (%).
    pub casting_speed: f32,
    /// Bonus de réutilisation (%).
    pub reuse_speed: f32,
    /// Récupération ajoutée au temps de cast (s).
    pub recovery: f32,
}

impl Default for PlayerStats {
    fn default() -> Self {
        Self {
            casting_speed: 0.0,
            reuse_speed: 0.0,
            recovery: DEFAULT_RECOVERY,
        }
    }
}

impl PlayerStats {
    /// Temps de cast effectif après bonus de vitesse, plancher GCD inclus.
    pub fn effective_cast(&self, base: f32) -> f32 {
        let bonus = self.casting_speed.clamp(0.0, CASTING_SPEED_CAP);
        (base / (1.0 + bonus / 100.0)).max(MIN_CAST)
    }
    /// Recast effectif après bonus de réutilisation.
    pub fn effective_recast(&self, base: f32) -> f32 {
        let bonus = self.reuse_speed.clamp(0.0, REUSE_SPEED_CAP);
        base / (1.0 + bonus / 100.0)
    }
}

/// Une ligne du tableau d'optimisation.
#[derive(Debug, Clone)]
pub struct SpellRow {
    pub ability: String,
    pub kind: &'static str,
    pub dmg_per_cast: f64,
    pub scenario_dmg: f64,
    pub crit_rate: f64,
    pub avg_targets: f64,
    pub base_cast: f32,
    pub cast_eff: f32,
    /// GCD plein du sort = cast effectif + recovery.
    pub gcd: f32,
    pub recast_eff: Option<f32>,
    pub total_damage: u64,
    pub casts: u32,
    /// Valeur par GCD libre : dégâts au scénario / (cast effectif + recovery).
    pub efficiency: f64,
    /// Contribution réaliste : dégâts au scénario / intervalle utile entre
    /// deux casts (= max(GCD, reuse, durée du DoT)). Borne les DoT et cooldowns.
    pub sustained_dps: f64,
    /// Intervalle utile entre deux casts (s).
    pub interval: f32,
    /// Sort à entretenir (DoT) ou en cooldown long (> 20 s).
    pub is_dot: bool,
    pub long_cd: bool,
    /// Temps de cast issu de la base (vs inféré/surchargé).
    pub from_db: bool,
    /// Dégâts mesurés dans les logs (vs saisis à la main).
    pub observed: bool,
}

fn kind_label(target: Target, mech: Mech, obs: Option<&AbilityObs>) -> &'static str {
    // Terminologie EQ2 : AE = « area encounter » (vert, ne touche que les mobs
    // liés d'un encounter) ; AoE = zone au sol (bleu, touche tout dans la zone,
    // même sans link). Un type inconnu touchant plusieurs cibles est traité AoE.
    let dot = mech == Mech::Dot || (mech == Mech::Other && obs.is_some_and(|o| o.is_dot()));
    let area = match target {
        Target::Encounter => Some("AE"),
        Target::Pbaoe => Some("AoE"),
        Target::Other if obs.is_some_and(|o| o.avg_targets() > 1.5) => Some("AoE"),
        _ => None,
    };
    match (dot, area) {
        (true, Some("AE")) => "DoT AE",
        (true, Some(_)) => "DoT AoE",
        (true, None) => "DoT",
        (false, Some("AE")) => "AE",
        (false, Some(_)) => "AoE",
        (false, None) => "Direct",
    }
}

/// Accumulateur de fusion observé (logs) / base (classe) par nom normalisé.
struct Acc<'a> {
    display: String,
    obs: Option<&'a AbilityObs>,
    info: Option<&'a SpellInfo>,
}

/// Construit le tableau d'optimisation, trié par efficacité.
///
/// Fusionne les sorts mesurés dans les logs (`obs`) et, si `class_filter` est
/// donné avec `include_unobserved`, tous les sorts offensifs de cette classe
/// (pour planifier hors combat). Les dégâts viennent des logs s'ils existent,
/// sinon de la saisie manuelle (`manual`, indexé par nom affiché).
#[allow(clippy::too_many_arguments)]
pub fn report(
    obs: &HashMap<String, AbilityObs>,
    db: &SpellDb,
    class_hint: Option<&str>,
    stats: &PlayerStats,
    sc: &Scenario,
    overrides: &HashMap<String, f32>,
    manual: &HashMap<String, f64>,
    class_filter: Option<&str>,
    include_unobserved: bool,
) -> Vec<SpellRow> {
    // Indice de classe pour la résolution : la classe choisie prime sur la devinée.
    let hint = class_filter.or(class_hint);

    // Fusion par nom normalisé : observé d'abord, puis la base de la classe.
    let mut accs: HashMap<String, Acc> = HashMap::new();
    for (ability, o) in obs {
        if o.total_damage == 0 {
            continue; // sorts non offensifs (résistés seuls, etc.)
        }
        let info = db.lookup(ability, hint);
        // Si une classe est filtrée, on écarte les sorts CONNUS d'une autre
        // classe (ex. Soulrot du necro ne doit pas polluer la liste d'un wizard).
        // Les sorts inconnus de la base (procs/pets) restent liés au perso.
        if let (Some(cls), Some(i)) = (class_filter, info) {
            if i.class != cls {
                continue;
            }
        }
        accs.entry(normalize(ability)).or_insert(Acc {
            display: ability.clone(),
            obs: Some(o),
            info,
        });
    }
    if include_unobserved {
        if let Some(cls) = class_filter {
            for info in db.class_spells(cls) {
                accs.entry(normalize(&info.name)).or_insert(Acc {
                    display: info.name.clone(),
                    obs: None,
                    info: Some(info),
                });
            }
        }
    }

    let mut rows = Vec::new();
    for a in accs.into_values() {
        let info = a.info;
        let target = info.map(|i| i.target).unwrap_or(Target::Other);
        let mech = info.map(|i| i.mechanic).unwrap_or(Mech::Other);
        let kind = kind_label(target, mech, a.obs);

        // Source des dégâts : logs si mesurés, sinon saisie manuelle.
        let (per_target, dmg_per_cast, observed, crit_rate, avg_targets, total_damage, casts) =
            if let Some(o) = a.obs {
                let pt = o.total_damage as f64 / o.total_targets.max(1) as f64;
                (pt, o.dmg_per_cast(), true, o.crit_rate(), o.avg_targets(), o.total_damage, o.casts)
            } else {
                let m = manual.get(&a.display).copied().unwrap_or(0.0);
                (m, m, false, 0.0, 1.0, 0, 0)
            };

        let eff_targets = target.effective_targets(sc, avg_targets);
        let scenario_dmg = per_target * eff_targets;

        // Temps de cast : surcharge manuelle > base > inféré (1 s par défaut).
        let (base_cast, from_db) = if let Some(ov) = overrides.get(&a.display) {
            (*ov, info.is_some())
        } else if let Some(i) = info {
            (i.cast, true)
        } else {
            (1.0, false)
        };
        let cast_eff = stats.effective_cast(base_cast);
        let recast_eff = info.and_then(|i| i.recast).map(|r| stats.effective_recast(r));
        let gcd = cast_eff + stats.recovery;
        let efficiency = scenario_dmg / gcd as f64;

        // Intervalle utile : un DoT se re-applique au mieux à sa durée ; tout
        // sort est borné par son reuse et par le GCD. Le DPS soutenu en découle.
        let is_dot = kind.contains("DoT");
        let dot_dur = if is_dot { info.and_then(|i| i.duration).unwrap_or(0.0) } else { 0.0 };
        let reuse = recast_eff.unwrap_or(0.0);
        let interval = gcd.max(reuse).max(dot_dur).max(0.1);
        let sustained_dps = scenario_dmg / interval as f64;
        let long_cd = reuse > 20.0;

        rows.push(SpellRow {
            ability: a.display,
            kind,
            dmg_per_cast,
            scenario_dmg,
            crit_rate,
            avg_targets,
            base_cast,
            cast_eff,
            gcd,
            recast_eff,
            total_damage,
            casts,
            efficiency,
            sustained_dps,
            interval,
            is_dot,
            long_cd,
            from_db,
            observed,
        });
    }
    // Efficacité décroissante ; à égalité (sorts vides), tri alphabétique.
    rows.sort_by(|a, b| {
        b.efficiency
            .total_cmp(&a.efficiency)
            .then_with(|| a.ability.cmp(&b.ability))
    });
    rows
}

// ---------------------------------------------------------------------------
// Diagnostic : rotation observée vs optimale
// ---------------------------------------------------------------------------

/// Un sort à entretenir (DoT/cooldown) lancé moins souvent que possible.
#[derive(Debug, Clone)]
pub struct Underused {
    pub ability: String,
    pub casts: u32,
    pub expected: f64,
    /// Taux d'entretien observé (casts / attendus), borné à 1.
    pub uptime: f64,
    pub lost_damage: f64,
}

/// Bilan d'une rotation observée.
#[derive(Debug, Clone, Default)]
pub struct Diagnostic {
    pub combat_time: f64,
    pub total_casts: u32,
    /// Temps total passé à caster (s).
    pub cast_time: f64,
    /// Part du temps de combat réellement passée à caster (0..1).
    pub gcd_util: f64,
    /// Part du temps de cast passée sur des sorts sous la médiane d'efficacité.
    pub low_yield_frac: f64,
    pub underused: Vec<Underused>,
}

/// Analyse les sorts mesurés sur `combat_time` secondes : utilisation du GCD,
/// sorts sous-utilisés (castés moins souvent que possible), GCD à faible rendement.
pub fn diagnose(rows: &[SpellRow], combat_time: f64) -> Diagnostic {
    let obs: Vec<&SpellRow> = rows
        .iter()
        .filter(|r| r.observed && r.dmg_per_cast > 0.0)
        .collect();
    if obs.is_empty() || combat_time <= 0.0 {
        return Diagnostic { combat_time, ..Default::default() };
    }
    let total_casts: u32 = obs.iter().map(|r| r.casts).sum();
    let cast_time: f64 = obs.iter().map(|r| r.casts as f64 * r.gcd as f64).sum();
    let gcd_util = (cast_time / combat_time).clamp(0.0, 1.0);

    // Part du temps de cast sur les sorts sous la médiane d'efficacité.
    let mut effs: Vec<f64> = obs.iter().map(|r| r.efficiency).collect();
    effs.sort_by(|a, b| a.total_cmp(b));
    let median = effs[effs.len() / 2];
    let low_time: f64 = obs
        .iter()
        .filter(|r| r.efficiency < median)
        .map(|r| r.casts as f64 * r.gcd as f64)
        .sum();
    let low_yield_frac = if cast_time > 0.0 { low_time / cast_time } else { 0.0 };

    // Sous-utilisation : seulement les sorts à ENTRETENIR (DoT/cooldown), dont
    // l'intervalle utile dépasse nettement le GCD. Pour un filler spammable,
    // « casts attendus » n'a pas de sens (tous les sorts se partagent les GCD).
    let mut underused = Vec::new();
    for r in &obs {
        // On ne juge que ce qui se vise à ~100% : entretien d'un DoT, ou gros
        // cooldown à presser dès dispo. Un nuke à reuse court partage les GCD
        // et n'a pas vocation à être lancé en continu. L'intervalle doit aussi
        // être réellement gated (durée/reuse connus), pas un proc spammable.
        if !(r.is_dot || r.long_cd) || r.interval <= r.gcd * 1.5 {
            continue;
        }
        let expected = combat_time / r.interval.max(0.1) as f64;
        if expected < 2.0 {
            continue; // trop peu d'occasions pour juger
        }
        let uptime = (r.casts as f64 / expected).min(1.0);
        if uptime < 0.7 {
            let missed = (expected - r.casts as f64).max(0.0);
            underused.push(Underused {
                ability: r.ability.clone(),
                casts: r.casts,
                expected,
                uptime,
                lost_damage: missed * r.dmg_per_cast,
            });
        }
    }
    underused.sort_by(|a, b| b.lost_damage.total_cmp(&a.lost_damage));
    underused.truncate(6);

    Diagnostic { combat_time, total_casts, cast_time, gcd_util, low_yield_frac, underused }
}

// ---------------------------------------------------------------------------
// Piste C : rotation live (« quoi caster maintenant »)
// ---------------------------------------------------------------------------

/// Rôle d'une suggestion de cast (détermine couleur et priorité dans l'overlay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastRole {
    /// DoT tombé ou sur le point de tomber : à rafraîchir.
    Refresh,
    /// Gros cooldown prêt : à presser.
    Cooldown,
    /// Sort de remplissage au meilleur rendement, disponible.
    Filler,
}

impl CastRole {
    pub fn icon(self) -> &'static str {
        match self {
            CastRole::Refresh => "🔁",
            CastRole::Cooldown => "⏳",
            CastRole::Filler => "▶",
        }
    }
}

/// Une entrée de la file « prochains sorts ».
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub ability: String,
    pub role: CastRole,
    /// Détail court ("à poser", "tombé", "tombe dans 2 s", "prêt").
    pub note: String,
}

/// Construit la file des prochains casts conseillés à l'instant `now`.
///
/// Priorité : (1) DoT tombé ou à moins de `lead` s de tomber → rafraîchir ;
/// (2) gros cooldown prêt → presser ; (3) sinon le meilleur filler hors
/// cooldown. Se fonde sur le dernier instant de cast réel par sort
/// (`last_cast`, fourni par `Profiler::last_casts`).
pub fn next_casts(
    rows: &[SpellRow],
    last_cast: &HashMap<String, u64>,
    now: u64,
    lead: f32,
    max: usize,
) -> Vec<Suggestion> {
    let elapsed = |ability: &str| -> Option<f32> {
        last_cast.get(ability).map(|&t| now.saturating_sub(t) as f32)
    };

    let mut refresh: Vec<(f32, Suggestion)> = Vec::new();
    let mut cooldown: Vec<(f64, Suggestion)> = Vec::new();
    let mut filler: Vec<(f64, Suggestion)> = Vec::new();

    for r in rows {
        if r.scenario_dmg <= 0.0 {
            continue; // pas de modèle de dégâts : rien à conseiller.
        }
        let e = elapsed(&r.ability);
        if r.is_dot {
            // Cycle ≈ intervalle utile du DoT (sa durée en pratique).
            let cycle = r.interval.max(1.0);
            let remaining = match e {
                None => -1.0, // jamais posé → à poser
                Some(x) => cycle - x,
            };
            if remaining <= lead {
                let note = if e.is_none() {
                    "à poser".to_string()
                } else if remaining <= 0.0 {
                    "tombé".to_string()
                } else {
                    format!("tombe dans {remaining:.0} s")
                };
                refresh.push((
                    remaining,
                    Suggestion { ability: r.ability.clone(), role: CastRole::Refresh, note },
                ));
            }
        } else if r.long_cd {
            let reuse = r.recast_eff.unwrap_or(r.interval).max(0.1);
            if e.is_none_or(|x| x >= reuse) {
                cooldown.push((
                    r.scenario_dmg,
                    Suggestion { ability: r.ability.clone(), role: CastRole::Cooldown, note: "prêt".to_string() },
                ));
            }
        } else {
            // Filler : disponible s'il n'est pas en reuse court.
            let reuse = r.recast_eff.unwrap_or(r.gcd).max(0.1);
            if e.is_none_or(|x| x >= reuse) {
                filler.push((
                    r.efficiency,
                    Suggestion { ability: r.ability.clone(), role: CastRole::Filler, note: String::new() },
                ));
            }
        }
    }

    // Le plus urgent d'abord dans chaque catégorie.
    refresh.sort_by(|a, b| a.0.total_cmp(&b.0));
    cooldown.sort_by(|a, b| b.0.total_cmp(&a.0));
    filler.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut out: Vec<Suggestion> = Vec::new();
    out.extend(refresh.into_iter().map(|(_, s)| s));
    out.extend(cooldown.into_iter().map(|(_, s)| s));
    // Un seul filler (le meilleur dispo) suffit comme garniture de fin de file.
    if let Some((_, s)) = filler.into_iter().next() {
        out.push(s);
    }
    out.truncate(max.max(1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_direct_aoe_into_one_cast() {
        // Un direct AoE touche 3 cibles la même seconde = 1 cast, 3 cibles.
        let mut sc = CastScratch::default();
        for t in ["A", "B", "C"] {
            sc.per_target.insert(t.into(), vec![100]);
            sc.amount += 500;
            sc.hits += 1;
        }
        let o = fold(&sc);
        assert_eq!(o.casts, 1);
        assert_eq!(o.total_targets, 3);
        assert_eq!(o.freq, 0.0);
    }

    #[test]
    fn folds_dot_ticks_into_one_cast() {
        // Un DoT qui tique toutes les 2 s pendant 8 s = 1 cast (1 cible).
        let mut sc = CastScratch::default();
        sc.per_target
            .insert("Boss".into(), vec![10, 12, 14, 16, 18]);
        sc.amount = 5000;
        sc.hits = 5;
        let o = fold(&sc);
        assert_eq!(o.casts, 1);
        assert!(o.freq > 0.0, "freq devrait être détectée");
    }

    #[test]
    fn folds_recast_dot_into_two_casts() {
        // Même DoT relancé après une longue pause = 2 casts.
        let mut sc = CastScratch::default();
        sc.per_target
            .insert("Boss".into(), vec![10, 12, 14, 60, 62, 64]);
        sc.amount = 6000;
        sc.hits = 6;
        let o = fold(&sc);
        assert_eq!(o.casts, 2);
    }

    #[test]
    fn observed_spell_does_not_leak_into_other_classes() {
        // Soulrot (necro) observé ne doit apparaître que sous la classe necro,
        // jamais quand on filtre sur une autre classe (wizard).
        let db = SpellDb::bundled();
        let mut obs = HashMap::new();
        obs.insert(
            "Soulrot".to_string(),
            AbilityObs {
                casts: 5,
                total_damage: 50_000,
                total_targets: 5,
                hits: 25,
                ..Default::default()
            },
        );
        let stats = PlayerStats::default();
        let sc = Scenario { targets: 1, linked: true };
        let necro = report(&obs, &db, None, &stats, &sc, &Default::default(), &Default::default(), Some("necromancer"), false);
        assert!(necro.iter().any(|r| r.ability == "Soulrot"));
        let wiz = report(&obs, &db, None, &stats, &sc, &Default::default(), &Default::default(), Some("wizard"), false);
        assert!(!wiz.iter().any(|r| r.ability == "Soulrot"));
    }

    fn row(ability: &str, dmg: f64, casts: u32, interval: f32, is_dot: bool, long_cd: bool) -> SpellRow {
        SpellRow {
            ability: ability.into(),
            kind: "DoT",
            dmg_per_cast: dmg,
            scenario_dmg: dmg,
            crit_rate: 0.0,
            avg_targets: 1.0,
            base_cast: 1.0,
            cast_eff: 1.0,
            gcd: 1.5,
            recast_eff: None,
            total_damage: dmg as u64 * casts as u64,
            casts,
            efficiency: dmg / 1.5,
            sustained_dps: dmg / interval as f64,
            interval,
            is_dot,
            long_cd,
            from_db: true,
            observed: true,
        }
    }

    #[test]
    fn diagnose_flags_only_maintained_underused() {
        let rows = vec![
            // DoT 24 s entretenu seulement à ~25 % sur 240 s → signalé.
            row("DoT lent", 1000.0, 2, 24.0, true, false),
            // Filler spammable (intervalle = GCD) → jamais "sous-utilisé".
            row("Filler", 800.0, 100, 1.5, false, false),
        ];
        let d = diagnose(&rows, 240.0);
        assert_eq!(d.underused.len(), 1);
        assert_eq!(d.underused[0].ability, "DoT lent");
        assert!(d.underused[0].uptime < 0.5);
        assert!(d.gcd_util > 0.0 && d.gcd_util <= 1.0);
    }

    #[test]
    fn next_casts_prioritizes_fallen_dot() {
        // Un DoT (cycle 12 s) posé à t=0, un filler spammable. À t=100 le DoT
        // est tombé depuis longtemps → il doit passer devant le filler.
        let rows = vec![
            row("DoT lent", 1000.0, 1, 12.0, true, false),
            row("Filler", 500.0, 1, 1.5, false, false),
        ];
        let mut last = HashMap::new();
        last.insert("DoT lent".to_string(), 0u64);
        last.insert("Filler".to_string(), 0u64);
        let s = next_casts(&rows, &last, 100, 2.0, 3);
        assert!(!s.is_empty());
        assert_eq!(s[0].ability, "DoT lent");
        assert_eq!(s[0].role, CastRole::Refresh);
        // Le filler suit (disponible, meilleur rendement restant).
        assert!(s.iter().any(|x| x.ability == "Filler" && x.role == CastRole::Filler));
    }

    #[test]
    fn next_casts_holds_active_dot() {
        // Le même DoT vient d'être posé (t=98, cycle 12) → pas encore à
        // rafraîchir : seul le filler reste conseillé.
        let rows = vec![
            row("DoT lent", 1000.0, 1, 12.0, true, false),
            row("Filler", 500.0, 1, 1.5, false, false),
        ];
        let mut last = HashMap::new();
        last.insert("DoT lent".to_string(), 98u64);
        last.insert("Filler".to_string(), 90u64);
        let s = next_casts(&rows, &last, 100, 2.0, 3);
        assert!(s.iter().all(|x| x.ability != "DoT lent"));
        assert_eq!(s.first().map(|x| x.ability.as_str()), Some("Filler"));
    }

    #[test]
    fn effective_cast_applies_haste_and_floor() {
        let s = PlayerStats {
            casting_speed: 100.0,
            ..Default::default()
        };
        assert!((s.effective_cast(2.0) - 1.0).abs() < 1e-3);
        // Plancher GCD : 0.5 s mini même avec un cast de base court.
        assert!((s.effective_cast(0.5) - 0.5).abs() < 1e-3);
    }
}
