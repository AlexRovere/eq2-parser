//! Configuration persistée en JSON à côté de l'exécutable.

use crate::mechanics::AlertMode;
use crate::optimizer::PlayerStats;
use crate::triggers::Trigger;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Répertoire `logs` d'EverQuest II.
    pub logs_dir: PathBuf,
    /// Dernier fichier de log suivi.
    pub last_log: Option<PathBuf>,
    /// Secondes d'inactivité avant clôture d'un encounter.
    pub encounter_timeout: u64,
    /// Clore le combat sur l'activité de ton camp (toi/pets/alliés + ennemis
    /// engagés) : ignore les combats voisins (joueurs hors groupe, PNJ).
    pub encounter_anchor: bool,
    /// Relire tout le fichier à l'attache (import historique).
    pub import_existing: bool,
    /// Suivre automatiquement le log le plus récemment écrit (le perso actif).
    pub auto_attach_latest: bool,
    pub overlay_enabled: bool,
    pub overlay_opacity: f32,
    pub overlay_click_through: bool,
    /// Nombre de barres max par section dans l'overlay.
    pub overlay_rows: usize,
    /// Échelle globale de l'overlay (police, barres). 1.0 = normal.
    pub overlay_scale: f32,
    /// Largeur de l'overlay en points.
    pub overlay_width: f32,
    /// Hauteur de l'overlay en points (redimensionnable via le grip).
    pub overlay_height: f32,
    /// Couleur de fond (RGB) — l'alpha vient de `overlay_opacity`.
    pub overlay_bg: [u8; 3],
    /// Couleur d'accent (ton personnage, texte custom).
    pub overlay_accent: [u8; 3],
    /// Sections affichées.
    pub overlay_show_dps: bool,
    pub overlay_show_hps: bool,
    pub overlay_show_power: bool,
    /// Barre de titre détaillée : durée + total + DPS raid + kills.
    pub overlay_title_stats: bool,
    /// Ligne de texte libre (template à variables) de l'overlay.
    pub overlay_custom_text: String,
    /// Format custom du côté droit des barres (template, vide = auto
    /// « 4691 (93.8k · 52.8%) »). Variables résolues sur le joueur de la barre.
    pub overlay_bar_format: String,
    /// Format custom de la barre de titre (template, vide = auto).
    pub overlay_title_format: String,
    /// Afficher le texte custom en haut (sous le titre) plutôt qu'en bas.
    pub overlay_text_top: bool,
    /// Position de l'overlay à l'écran (persistée).
    pub overlay_pos: Option<(f32, f32)>,
    /// Verrouiller la position/taille (pas de drag ni de resize accidentels).
    pub overlay_locked: bool,
    /// L'overlay devient quasi transparent quand la souris le survole.
    pub overlay_fade_hover: bool,
    pub triggers: Vec<Trigger>,
    /// Profils d'overlay nommés (raid compact, solo détaillé…).
    pub overlay_profiles: Vec<OverlayProfile>,
    /// Fusionner les pets dans leur propriétaire à l'affichage.
    pub merge_pets: bool,
    /// Afficher aussi les ennemis (mobs) dans les classements.
    pub show_enemies: bool,
    /// Masquer les PNJ alliés (mercenaires, PNJ de quête — noms à plusieurs mots).
    pub hide_npcs: bool,
    /// Sauvegarder l'historique des encounters sur disque.
    pub persist_history: bool,
    /// Nombre max d'encounters conservés par personnage.
    pub history_cap: usize,
    /// Assignations manuelles : pet → propriétaire (prioritaires sur l'auto-détection).
    pub pet_assignments: HashMap<String, String>,
    /// Dernière version dont les nouveautés ont été vues (fenêtre changelog).
    pub last_seen_version: String,
    /// Thème clair (par défaut : sombre).
    pub light_mode: bool,
    /// Détection et alerte des mécaniques ennemies récurrentes.
    pub mechanics_enabled: bool,
    /// Mode d'alerte par défaut des mécaniques (surchargeable par mécanique).
    pub mech_default_alert: AlertMode,
    /// Afficher les comptes à rebours de mécaniques dans l'overlay DPS.
    pub mech_overlay: bool,
    /// Overlay dédié aux mécaniques de boss (fenêtre séparée).
    pub mech_overlay_window: bool,
    pub mech_overlay_pos: Option<(f32, f32)>,
    pub mech_overlay_width: f32,
    pub mech_overlay_height: f32,
    /// Optimisation : stats offensives saisies par personnage.
    pub player_stats: HashMap<String, PlayerStats>,
    /// Optimisation : temps de cast de base surchargés à la main, par sort.
    pub cast_overrides: HashMap<String, f32>,
    /// Optimisation : dégâts par cast saisis à la main (sorts pas encore vus
    /// en combat), indexés par nom de sort.
    pub spell_damage: HashMap<String, f64>,
    /// Optimisation : nombre de cibles du scénario courant.
    pub opt_targets: u32,
    /// Optimisation : durée de combat type (s) pour escompter les DoT qui
    /// n'iront pas à leur terme. 0 = auto (médiane de l'historique) ; si
    /// l'historique est vide, pas d'escompte (DoT supposés complets).
    pub opt_fight_secs: f32,
    /// Optimisation : cibles liées (même encounter) pour le scaling AoE.
    pub opt_linked: bool,
    /// Optimisation : masquer les sorts hors-base (procs/pets/cast inféré).
    /// Par défaut faux : on montre TOUT ce que le joueur a lancé, même si la
    /// base ne connaît pas le sort (sinon des vrais casts disparaîtraient).
    pub opt_hide_unknown: bool,
    /// Optimisation : colonne de tri (dmg/crit/targets/cast/recast/eff/sustained).
    pub opt_sort_key: String,
    /// Optimisation : tri décroissant (défaut) ou croissant.
    pub opt_sort_desc: bool,
    /// Optimisation : sorts masqués (renvoyés en bas de liste, grisés).
    pub opt_hidden: std::collections::HashSet<String>,
    /// Classe détectée par personnage (auto-remplie à la détection du log).
    pub char_class: HashMap<String, String>,
    /// Couleur custom (RGB) par joueur pour les barres d'overlay et les courbes
    /// du graphe. Absent = couleur auto dérivée du hash du nom.
    pub player_colors: HashMap<String, [u8; 3]>,
    /// Overlay « rotation live » (fenêtre séparée : quoi caster maintenant).
    pub rotation_overlay: bool,
    pub rotation_overlay_pos: Option<(f32, f32)>,
    pub rotation_overlay_width: f32,
    pub rotation_overlay_height: f32,
    /// Nombre de prochains sorts affichés dans l'overlay rotation.
    pub rotation_count: usize,
    /// Pré-alerte (s) avant qu'un DoT ne tombe (fenêtre d'anticipation).
    pub rotation_lead: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            logs_dir: PathBuf::from(r"X:\jeux\steam\steamapps\common\EverQuest 2\logs"),
            last_log: None,
            encounter_timeout: 6,
            encounter_anchor: true,
            import_existing: false,
            auto_attach_latest: true,
            overlay_enabled: true,
            overlay_opacity: 0.85,
            overlay_click_through: false,
            overlay_rows: 8,
            overlay_scale: 1.0,
            overlay_width: 340.0,
            overlay_height: 240.0,
            overlay_bg: [12, 12, 18],
            overlay_accent: [241, 196, 15],
            overlay_show_dps: true,
            overlay_show_hps: false,
            overlay_show_power: false,
            overlay_title_stats: true,
            overlay_custom_text: String::new(),
            overlay_bar_format: String::new(),
            overlay_title_format: String::new(),
            overlay_text_top: true,
            overlay_pos: None,
            overlay_locked: false,
            overlay_fade_hover: true,
            triggers: Vec::new(),
            overlay_profiles: Vec::new(),
            merge_pets: true,
            show_enemies: false,
            hide_npcs: true,
            persist_history: true,
            history_cap: 500,
            pet_assignments: HashMap::new(),
            last_seen_version: String::new(),
            light_mode: false,
            mechanics_enabled: true,
            mech_default_alert: AlertMode::Sound,
            mech_overlay: false,
            mech_overlay_window: false,
            mech_overlay_pos: None,
            mech_overlay_width: 250.0,
            mech_overlay_height: 180.0,
            player_stats: HashMap::new(),
            cast_overrides: HashMap::new(),
            spell_damage: HashMap::new(),
            opt_targets: 1,
            opt_fight_secs: 0.0,
            opt_linked: true,
            opt_hide_unknown: false,
            opt_sort_key: "eff".into(),
            opt_sort_desc: true,
            opt_hidden: std::collections::HashSet::new(),
            char_class: HashMap::new(),
            player_colors: HashMap::new(),
            rotation_overlay: false,
            rotation_overlay_pos: None,
            rotation_overlay_width: 230.0,
            rotation_overlay_height: 170.0,
            rotation_count: 4,
            rotation_lead: 2.0,
        }
    }
}

/// Snapshot nommé des réglages d'overlay, commutable en un clic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayProfile {
    pub name: String,
    pub opacity: f32,
    pub click_through: bool,
    pub rows: usize,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    pub bg: [u8; 3],
    pub accent: [u8; 3],
    pub show_dps: bool,
    pub show_hps: bool,
    pub show_power: bool,
    pub title_stats: bool,
    pub custom_text: String,
    pub bar_format: String,
    pub title_format: String,
    pub text_top: bool,
}

impl Default for OverlayProfile {
    fn default() -> Self {
        let c = Config::default();
        Self {
            name: "Profil".into(),
            opacity: c.overlay_opacity,
            click_through: c.overlay_click_through,
            rows: c.overlay_rows,
            scale: c.overlay_scale,
            width: c.overlay_width,
            height: c.overlay_height,
            bg: c.overlay_bg,
            accent: c.overlay_accent,
            show_dps: c.overlay_show_dps,
            show_hps: c.overlay_show_hps,
            show_power: c.overlay_show_power,
            title_stats: c.overlay_title_stats,
            custom_text: c.overlay_custom_text,
            bar_format: c.overlay_bar_format,
            title_format: c.overlay_title_format,
            text_top: c.overlay_text_top,
        }
    }
}

impl Config {
    /// Capture les réglages d'overlay actuels sous forme de profil.
    pub fn capture_profile(&self, name: &str) -> OverlayProfile {
        OverlayProfile {
            name: name.to_string(),
            opacity: self.overlay_opacity,
            click_through: self.overlay_click_through,
            rows: self.overlay_rows,
            scale: self.overlay_scale,
            width: self.overlay_width,
            height: self.overlay_height,
            bg: self.overlay_bg,
            accent: self.overlay_accent,
            show_dps: self.overlay_show_dps,
            show_hps: self.overlay_show_hps,
            show_power: self.overlay_show_power,
            title_stats: self.overlay_title_stats,
            custom_text: self.overlay_custom_text.clone(),
            bar_format: self.overlay_bar_format.clone(),
            title_format: self.overlay_title_format.clone(),
            text_top: self.overlay_text_top,
        }
    }

    /// Applique un profil aux réglages d'overlay.
    pub fn apply_profile(&mut self, p: &OverlayProfile) {
        self.overlay_opacity = p.opacity;
        self.overlay_click_through = p.click_through;
        self.overlay_rows = p.rows;
        self.overlay_scale = p.scale;
        self.overlay_width = p.width;
        self.overlay_height = p.height;
        self.overlay_bg = p.bg;
        self.overlay_accent = p.accent;
        self.overlay_show_dps = p.show_dps;
        self.overlay_show_hps = p.show_hps;
        self.overlay_show_power = p.show_power;
        self.overlay_title_stats = p.title_stats;
        self.overlay_custom_text = p.custom_text.clone();
        self.overlay_bar_format = p.bar_format.clone();
        self.overlay_title_format = p.title_format.clone();
        self.overlay_text_top = p.text_top;
    }
}

/// Chemin d'un fichier de config (`<nom>`) à côté de l'exécutable.
fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn config_path() -> PathBuf {
    beside_exe("eq2-parser.json")
}

/// Ancien nom (avant le renommage du projet) — lu en repli pour migrer.
fn legacy_config_path() -> PathBuf {
    beside_exe("eq2-tools.json")
}

impl Config {
    pub fn load() -> Self {
        // Nouveau fichier, sinon migration depuis l'ancien `eq2-tools.json`.
        let raw = std::fs::read_to_string(config_path())
            .or_else(|_| std::fs::read_to_string(legacy_config_path()));
        raw.ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), s);
        }
    }
}
