//! Configuration persistée en JSON à côté de l'exécutable.

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
    /// Relire tout le fichier à l'attache (import historique).
    pub import_existing: bool,
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
    /// Afficher le texte custom en haut (sous le titre) plutôt qu'en bas.
    pub overlay_text_top: bool,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            logs_dir: PathBuf::from(r"X:\jeux\steam\steamapps\common\EverQuest 2\logs"),
            last_log: None,
            encounter_timeout: 6,
            import_existing: false,
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
            overlay_text_top: true,
            triggers: Vec::new(),
            overlay_profiles: Vec::new(),
            merge_pets: true,
            show_enemies: false,
            hide_npcs: true,
            persist_history: true,
            history_cap: 500,
            pet_assignments: HashMap::new(),
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
        self.overlay_text_top = p.text_top;
    }
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("eq2-tools.json")))
        .unwrap_or_else(|| PathBuf::from("eq2-tools.json"))
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), s);
        }
    }
}
