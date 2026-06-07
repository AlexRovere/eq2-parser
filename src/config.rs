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
    /// Ligne de texte libre affichée en bas de l'overlay.
    pub overlay_custom_text: String,
    pub triggers: Vec<Trigger>,
    /// Fusionner les pets dans leur propriétaire à l'affichage.
    pub merge_pets: bool,
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
            triggers: Vec::new(),
            merge_pets: true,
            pet_assignments: HashMap::new(),
        }
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
