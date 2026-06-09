//! Interface : fenêtre principale (Live / Encounters / Triggers / Settings)
//! + overlay DPS transparent toujours au-dessus.

use crate::combat::{fmt_duration, fmt_f64, fmt_num, CombatEngine, Encounter};
use crate::config::Config;
use crate::export;
use crate::mechanics::{AlertMode, MechEntry, MechKind, MechSource};
use crate::parser::{char_name_from_path, Parser};
use crate::tailer::{discover_logs, Tailer};
use crate::triggers::{BeepKind, Trigger, TriggerEngine};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Couleur stable par joueur (hash du nom) — identique partout :
/// overlay, graphes, jauges des tables. Ne change pas avec le tri.
fn player_color(name: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let hue = (h % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.62, 0.86, 1.0).into()
}

/// Applique le thème de l'application (palette, arrondis, espacements).
fn apply_theme(ctx: &egui::Context, light: bool) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing.y = 6.0;
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let v = &mut style.visuals;
    if light {
        v.panel_fill = Color32::from_rgb(244, 245, 248);
        v.window_fill = Color32::from_rgb(250, 250, 252);
        v.faint_bg_color = Color32::from_rgb(233, 235, 240);
    } else {
        v.panel_fill = Color32::from_rgb(16, 17, 22);
        v.window_fill = Color32::from_rgb(20, 21, 27);
        v.extreme_bg_color = Color32::from_rgb(11, 12, 16);
        v.faint_bg_color = Color32::from_rgb(24, 25, 32);
    }
    v.selection.bg_fill = Color32::from_rgb(52, 152, 219).gamma_multiply(0.55);
    v.hyperlink_color = Color32::from_rgb(52, 120, 190);
    v.widgets.noninteractive.corner_radius = 4.into();
    v.widgets.inactive.corner_radius = 4.into();
    v.widgets.hovered.corner_radius = 4.into();
    v.widgets.active.corner_radius = 4.into();
    ctx.set_style(style);
}

const BAR_COLORS: [Color32; 10] = [
    Color32::from_rgb(231, 76, 60),
    Color32::from_rgb(241, 196, 15),
    Color32::from_rgb(46, 204, 113),
    Color32::from_rgb(52, 152, 219),
    Color32::from_rgb(155, 89, 182),
    Color32::from_rgb(230, 126, 34),
    Color32::from_rgb(26, 188, 156),
    Color32::from_rgb(236, 112, 99),
    Color32::from_rgb(93, 173, 226),
    Color32::from_rgb(171, 235, 198),
];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Live,
    Encounters,
    Triggers,
    Mechanics,
    Optimizer,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
enum Metric {
    Dps,
    Hps,
    Power,
    Taken,
}

impl Metric {
    fn label(&self) -> &'static str {
        match self {
            Metric::Dps => "DPS",
            Metric::Hps => "HPS",
            Metric::Power => "Power",
            Metric::Taken => "Dégâts subis",
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum GraphMode {
    /// Une ligne par combattant.
    PerPlayer,
    /// Aires empilées par sort, pour le combattant sélectionné.
    PerAbility,
}

struct GraphState {
    metric: Metric,
    mode: GraphMode,
    /// Fenêtre de lissage en secondes (moyenne glissante).
    smooth: u64,
    cumulative: bool,
    selected: BTreeSet<String>,
    /// Superposer l'encounter épinglé (lignes pointillées).
    overlay_pinned: bool,
    /// Demande d'export PNG du graphe (déclenche un screenshot du viewport).
    want_png: bool,
    /// Rect du dernier plot rendu (pour le recadrage du PNG).
    last_plot_rect: Option<egui::Rect>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            metric: Metric::Dps,
            mode: GraphMode::PerPlayer,
            smooth: 5,
            cumulative: false,
            selected: BTreeSet::new(),
            overlay_pinned: false,
            want_png: false,
            last_plot_rect: None,
        }
    }
}

pub struct App {
    config: Config,
    parser: Option<Parser>,
    engine: CombatEngine,
    trigger_engine: TriggerEngine,
    tailer: Option<Tailer>,
    rx: Option<Receiver<String>>,
    tab: Tab,
    /// None = encounter courant (live), Some(i) = historique[i].
    selected_encounter: Option<usize>,
    selected_combatant: Option<String>,
    available_logs: Vec<PathBuf>,
    lines_seen: u64,
    passthrough_sent: bool,
    graph_state: GraphState,
    /// Feedback visuel après copie presse-papiers.
    copied_at: Option<Instant>,
    /// Encounter épinglé pour comparaison (index dans l'historique).
    compare_pin: Option<usize>,
    /// Screenshot en attente pour l'export PNG du graphe.
    awaiting_screenshot: bool,
    /// Serveur du log suivi (pour le fichier d'historique).
    current_server: String,
    /// Auto-sauvegarde de l'historique : état au dernier save.
    last_hist_len: usize,
    last_hist_save: Instant,
    /// Dernier scan du répertoire de logs (suivi auto du perso actif).
    last_log_scan: Instant,
    /// Fractions animées des barres de l'overlay : (section, nom) → valeur lissée.
    bar_anim: HashMap<(&'static str, String), f32>,
    /// Tri par table : id → (colonne, descendant).
    sort_states: HashMap<&'static str, (usize, bool)>,
    /// Filtres texte.
    filter_combatant: String,
    filter_ability: String,
    filter_encounters: String,
    filter_log: String,
    /// Pseudo-encounter « Session entière » sélectionné.
    session_selected: bool,
    /// Cache de l'agrégat de session : (longueur d'historique, agrégat).
    session_cache: Option<(usize, Encounter)>,
    /// Zone sélectionnée (agrégat par zone).
    selected_zone: Option<String>,
    /// Cache de l'agrégat de zone : (longueur d'historique, zone, agrégat).
    zone_cache: Option<(usize, String, Encounter)>,
    /// Nom saisi pour enregistrer un profil d'overlay.
    profile_name: String,
    /// Mise à jour : canal de notification + état.
    update_tx: std::sync::mpsc::Sender<crate::update::UpdateInfo>,
    update_rx: Receiver<crate::update::UpdateInfo>,
    update_available: Option<crate::update::UpdateInfo>,
    update_err_rx: Option<Receiver<String>>,
    update_error: Option<String>,
    updating: bool,
    /// Fenêtre « Nouveautés » (changelog embarqué).
    show_changelog: bool,
    /// Mécaniques déjà alertées : capacité → epoch du cast annoncé (anti-spam).
    mech_warned: HashMap<String, u64>,
    /// Filtre texte de la base de mécaniques.
    filter_mech: String,
    /// Dernière sauvegarde de la base de mécaniques apprise.
    last_mech_save: Instant,
    /// Base de sorts bundlée (cast/recast/type) pour l'optimisation.
    spell_db: crate::optimizer::SpellDb,
    /// Personnage sélectionné dans l'onglet Optimisation (auto = perso suivi).
    opt_char: Option<String>,
    /// Classe sélectionnée dans l'onglet Optimisation (pour la planif hors combat).
    opt_class: Option<String>,
    /// Dernier perso vu dans l'onglet Optimisation (re-détecte la classe au changement).
    opt_char_prev: Option<String>,
    /// Overlays en cours de déplacement : on n'impose pas la position pendant le
    /// drag (sinon ça se bat avec l'OS et l'overlay tremble).
    overlay_dragging: bool,
    mech_dragging: bool,
}

/// Changelog embarqué dans l'exécutable (mis à jour à chaque release).
const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// Pack de triggers raid génériques embarqué (ready check, death prevents…).
const TRIGGERS_PACK: &str = include_str!("../assets/triggers_pack.json");

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut config = Config::load();
        apply_theme(&cc.egui_ctx, config.light_mode);
        let trigger_engine = TriggerEngine::new(&config.triggers);
        let mut engine = CombatEngine::new(config.encounter_timeout);
        engine.anchor = config.encounter_anchor;
        // Premier lancement / mauvais chemin : détecte le répertoire EQ2.
        let mut available_logs = discover_logs(&config.logs_dir);
        if available_logs.is_empty() {
            if let Some(dir) = crate::tailer::detect_logs_dir() {
                config.logs_dir = dir;
                config.save();
                available_logs = discover_logs(&config.logs_dir);
            }
        }
        let config_seen_version = config.last_seen_version.clone();
        // Auto-update : nettoie l'ancien exe et vérifie les releases GitHub.
        crate::update::cleanup_old();
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        if !cfg!(debug_assertions) {
            crate::update::spawn_check(update_tx.clone());
        }
        let mut app = Self {
            config,
            parser: None,
            engine,
            trigger_engine,
            tailer: None,
            rx: None,
            tab: Tab::Live,
            selected_encounter: None,
            selected_combatant: None,
            available_logs,
            lines_seen: 0,
            passthrough_sent: false,
            graph_state: GraphState::default(),
            copied_at: None,
            compare_pin: None,
            awaiting_screenshot: false,
            current_server: String::new(),
            last_hist_len: 0,
            last_hist_save: Instant::now(),
            last_log_scan: Instant::now(),
            bar_anim: HashMap::new(),
            sort_states: HashMap::new(),
            filter_combatant: String::new(),
            filter_ability: String::new(),
            filter_encounters: String::new(),
            filter_log: String::new(),
            session_selected: false,
            session_cache: None,
            selected_zone: None,
            zone_cache: None,
            profile_name: String::new(),
            update_tx,
            update_rx,
            update_available: None,
            update_err_rx: None,
            update_error: None,
            updating: false,
            // Après une mise à jour (ou au premier lancement), montre les nouveautés.
            show_changelog: config_seen_version != crate::update::CURRENT_VERSION,
            mech_warned: HashMap::new(),
            filter_mech: String::new(),
            last_mech_save: Instant::now(),
            spell_db: crate::optimizer::SpellDb::bundled(),
            opt_char: None,
            opt_class: None,
            opt_char_prev: None,
            overlay_dragging: false,
            mech_dragging: false,
        };
        // Attache automatique : le log le plus récemment écrit (perso actif),
        // sinon le dernier log suivi.
        if app.config.auto_attach_latest {
            if let Some(latest) = app.available_logs.first().cloned() {
                app.attach(latest, cc.egui_ctx.clone());
            }
        } else if let Some(last) = app.config.last_log.clone() {
            if last.exists() {
                app.attach(last, cc.egui_ctx.clone());
            }
        }
        app
    }

    fn attach(&mut self, path: PathBuf, ctx: egui::Context) {
        // Sauvegarde l'historique du personnage précédent avant de changer.
        self.save_history();

        let name = char_name_from_path(&path).unwrap_or_else(|| "You".into());
        let server = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        self.parser = Some(Parser::new(name.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        self.tailer = Some(Tailer::start(
            path.clone(),
            self.config.import_existing,
            tx,
            ctx,
        ));
        self.rx = Some(rx);
        self.engine = CombatEngine::new(self.config.encounter_timeout);
        self.engine.anchor = self.config.encounter_anchor;
        self.engine.self_name = name.clone();
        // Recharge l'historique persisté de ce personnage.
        if self.config.persist_history {
            self.engine.history = crate::history::load(&server, &name);
        }
        self.current_server = server;
        self.last_hist_len = self.engine.history.len();
        // Détection auto de la classe du perso à l'attache, depuis l'historique
        // chargé (sorts qu'il a lancés), si pas déjà connue.
        if !self.config.char_class.contains_key(&name) {
            let mut abilities: std::collections::HashSet<String> = std::collections::HashSet::new();
            for enc in &self.engine.history {
                if let Some(c) = enc.combatants.get(&name) {
                    abilities.extend(c.abilities.keys().cloned());
                }
            }
            if let Some(cls) = self.spell_db.infer_class(abilities.iter()) {
                self.config.char_class.insert(name.clone(), cls);
            }
        }
        // Le perso a changé : l'onglet Optimisation re-sélectionne sa classe.
        self.opt_char = None;
        self.opt_class = None;
        self.lines_seen = 0;
        self.selected_encounter = None;
        self.selected_combatant = None;
        self.compare_pin = None;
        self.config.last_log = Some(path);
        self.config.save();
    }

    fn save_history(&mut self) {
        if !self.config.persist_history || self.current_server.is_empty() {
            return;
        }
        let Some(name) = self.self_name().map(|s| s.to_string()) else { return };
        crate::history::save(
            &self.current_server,
            &name,
            &self.engine.history,
            self.config.history_cap,
        );
        self.last_hist_len = self.engine.history.len();
        self.last_hist_save = Instant::now();
    }

    fn drain_lines(&mut self) {
        let Some(rx) = &self.rx else { return };
        let Some(parser) = &self.parser else { return };
        // Borne par frame pour garder l'UI fluide pendant un import massif.
        for _ in 0..100_000 {
            match rx.try_recv() {
                Ok(line) => {
                    self.lines_seen += 1;
                    if let Some(parsed) = parser.parse_line(&line) {
                        self.engine.process(&parsed);
                        self.trigger_engine
                            .check(&parsed.message, &self.config.triggers);
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn now_epoch() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn self_name(&self) -> Option<&str> {
        self.parser.as_ref().map(|p| p.self_name.as_str())
    }

    /// Carte pet → owner effective : auto-détection + assignations manuelles (prioritaires).
    fn effective_owners(&self) -> HashMap<String, String> {
        let mut owners = self.engine.auto_pets.clone();
        owners.extend(self.config.pet_assignments.clone());
        owners
    }

    /// Prédit les prochains casts de mécaniques et déclenche les alertes
    /// (toast + son/voix) quelques secondes avant. Anti-spam par identité de cast.
    fn process_mechanic_alerts(&mut self) {
        if !self.config.mechanics_enabled {
            return;
        }
        let now = Self::now_epoch();
        let preds = self.engine.mech.predictions(now);
        let default_alert = self.config.mech_default_alert;
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for p in &preds {
            seen.insert(p.ability.clone());
            // Fenêtre d'alerte : on annonce quand il reste ≤ lead s (et pas trop tard).
            if p.eta > p.lead as f64 || p.eta < -1.5 {
                continue;
            }
            // Identité du cast à venir (epoch arrondi) : une alerte par cycle.
            let target = (now as f64 + p.eta).round().max(0.0) as u64;
            if self.mech_warned.get(&p.ability) == Some(&target) {
                continue;
            }
            self.mech_warned.insert(p.ability.clone(), target);
            let eta_s = p.eta.max(0.0).round() as i64;
            let text = if p.message.trim().is_empty() {
                if eta_s > 0 {
                    format!("{} {} dans {eta_s}s", p.kind.icon(), p.ability)
                } else {
                    format!("{} {} !", p.kind.icon(), p.ability)
                }
            } else {
                p.message.replace("{eta}", &eta_s.to_string())
            };
            self.trigger_engine.toasts.push(crate::triggers::Toast {
                text: text.clone(),
                created: Instant::now(),
            });
            let mode = match p.alert {
                AlertMode::Inherit => default_alert,
                m => m,
            };
            match mode {
                AlertMode::Sound => self.trigger_engine.test_sound(&None, BeepKind::Alarm),
                AlertMode::Tts => self.trigger_engine.test_tts(&text),
                _ => {}
            }
        }
        // Oublie les capacités qui ne sont plus suivies (fin de combat).
        self.mech_warned.retain(|k, _| seen.contains(k));
    }

    /// Importe une config ACT (XML) : SpellTimers → mécaniques, CustomTriggers →
    /// triggers (dédupliqués par pattern). Retourne (mécaniques, triggers) ajoutés.
    fn import_act_pack(&mut self, xml: &str) -> (usize, usize) {
        let res = crate::act_import::parse_act_xml(xml);
        // Mécaniques : fusion sans écraser l'existant.
        let mut add = crate::mechanics::MechanicsDb::default();
        add.entries = res.mechanics;
        let before = self.engine.mech.db.entries.len();
        self.engine.mech.db.merge(&add, false);
        let mech_added = self.engine.mech.db.entries.len() - before;
        if mech_added > 0 {
            self.engine.mech.dirty = true;
            self.engine.mech.save_if_dirty();
        }
        // Triggers : ajoute ceux dont le pattern est nouveau.
        let existing: std::collections::HashSet<String> =
            self.config.triggers.iter().map(|t| t.pattern.clone()).collect();
        let mut trig_added = 0;
        for t in res.triggers {
            if !t.pattern.is_empty() && !existing.contains(&t.pattern) {
                self.config.triggers.push(t);
                trig_added += 1;
            }
        }
        if trig_added > 0 {
            self.trigger_engine.recompile(&self.config.triggers);
            self.config.save();
        }
        (mech_added, trig_added)
    }

    /// Ajoute le pack de triggers de base embarqué (sans doublon de pattern).
    fn add_base_triggers(&mut self) -> usize {
        let Ok(pack) = serde_json::from_str::<Vec<Trigger>>(TRIGGERS_PACK) else {
            return 0;
        };
        let existing: std::collections::HashSet<String> =
            self.config.triggers.iter().map(|t| t.pattern.clone()).collect();
        let mut added = 0;
        for t in pack {
            if !existing.contains(&t.pattern) {
                self.config.triggers.push(t);
                added += 1;
            }
        }
        if added > 0 {
            self.trigger_engine.recompile(&self.config.triggers);
            self.config.save();
        }
        added
    }

    /// Écran d'accueil : guide l'utilisateur jusqu'au premier combat parsé.
    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("⚔ EQ2 Parser").size(26.0));
            ui.label(RichText::new("Combat parser & overlay pour EverQuest II").weak());
            ui.add_space(20.0);

            let ok = Color32::from_rgb(46, 204, 113);
            let warn = Color32::from_rgb(241, 196, 15);
            let step = |ui: &mut egui::Ui, done: bool, text: String| {
                ui.label(
                    RichText::new(format!("{}  {}", if done { "✅" } else { "⬜" }, text))
                        .size(15.0)
                        .color(if done { ok } else { warn }),
                );
                ui.add_space(6.0);
            };

            let dir_ok = !self.available_logs.is_empty();
            step(
                ui,
                dir_ok,
                if dir_ok {
                    format!("Répertoire EQ2 trouvé : {}", self.config.logs_dir.display())
                } else {
                    "Répertoire EQ2 introuvable — Settings → 🔍 Détecter".to_string()
                },
            );

            let attached = self.tailer.is_some();
            step(
                ui,
                attached,
                match (self.self_name(), &self.current_server) {
                    (Some(n), s) if attached && !s.is_empty() => {
                        format!("Personnage suivi : {n} @ {s}")
                    }
                    _ => "Aucun personnage suivi".to_string(),
                },
            );

            let logging = self.lines_seen > 0;
            step(
                ui,
                logging,
                if logging {
                    format!("Log actif — {} lignes lues", self.lines_seen)
                } else {
                    "En jeu, tape  /log  pour activer l'écriture du log".to_string()
                },
            );

            ui.add_space(14.0);
            ui.label(
                RichText::new(
                    "Dès ton premier combat, le DPS s'affiche ici et sur l'overlay.\n\
                     Clic droit sur l'overlay pour le personnaliser.",
                )
                .weak(),
            );
            if !dir_ok && ui.button("🔍 Détecter l'installation EQ2").clicked() {
                if let Some(dir) = crate::tailer::detect_logs_dir() {
                    self.config.logs_dir = dir;
                    self.available_logs = discover_logs(&self.config.logs_dir);
                    self.config.save();
                }
            }
        });
    }

    /// Fenêtre « ✨ Nouveautés » : changelog embarqué, affichée une fois
    /// après chaque mise à jour (et au premier lancement).
    fn show_changelog_window(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        let mut open = true;
        // Borne la fenêtre à l'écran : sur petit écran, sans ça, le contenu pousse
        // la fenêtre au-delà du bord et le bouton/scroll deviennent inaccessibles.
        let screen = ctx.screen_rect().size();
        let max_w = (screen.x - 32.0).clamp(280.0, 520.0);
        let max_h = (screen.y - 40.0).max(200.0);
        // Hauteur du scroll = ce qui reste sous le titre, le séparateur et le bouton.
        let scroll_h = (max_h - 96.0).max(120.0);
        egui::Window::new(format!(
            "✨ Nouveautés — v{}",
            crate::update::CURRENT_VERSION
        ))
        .id(egui::Id::new("changelog_window"))
        .open(&mut open)
        .collapsible(false)
        .default_size([520.0_f32.min(max_w), 420.0_f32.min(max_h)])
        .max_width(max_w)
        .max_height(max_h)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(scroll_h)
                .show(ui, |ui| {
                for line in CHANGELOG.lines() {
                    if line.starts_with("# ") {
                        continue; // titre du fichier
                    } else if let Some(h) = line.strip_prefix("## ") {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(h)
                                .size(16.0)
                                .strong()
                                .color(Color32::from_rgb(241, 196, 15)),
                        );
                        ui.add_space(2.0);
                    } else if let Some(b) = line.strip_prefix("- ") {
                        ui.label(format!("  •  {b}"));
                    } else if !line.trim().is_empty() {
                        ui.label(line);
                    }
                }
                ui.add_space(8.0);
            });
            ui.separator();
            ui.vertical_centered(|ui| {
                if ui.button("C'est noté !").clicked() {
                    self.show_changelog = false;
                }
            });
        });
        if !open {
            self.show_changelog = false;
        }
        // Marque cette version comme vue (ne réapparaîtra pas).
        if !self.show_changelog
            && self.config.last_seen_version != crate::update::CURRENT_VERSION
        {
            self.config.last_seen_version = crate::update::CURRENT_VERSION.to_string();
            self.config.save();
        }
    }

    /// Toasts dans la fenêtre principale (les mêmes que sur l'overlay).
    fn show_main_toasts(&self, ctx: &egui::Context) {
        if self.trigger_engine.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("main_toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.trigger_engine.toasts {
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(20, 21, 27, 240))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(241, 196, 15)))
                        .corner_radius(6.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("🔔 {}", t.text))
                                    .color(Color32::from_rgb(241, 196, 15)),
                            );
                        });
                }
            });
    }

    /// Agrégat « session entière », mis en cache tant que l'historique ne change pas.
    fn session_aggregate(&mut self) -> Encounter {
        let len = self.engine.history.len();
        if self
            .session_cache
            .as_ref()
            .is_none_or(|(cached_len, _)| *cached_len != len)
        {
            self.session_cache =
                Some((len, crate::combat::aggregate_session(&self.engine.history)));
        }
        self.session_cache.as_ref().unwrap().1.clone()
    }

    /// Agrégat d'une zone (stats par zone), mis en cache.
    fn zone_aggregate(&mut self, zone: &str) -> Encounter {
        let len = self.engine.history.len();
        if self
            .zone_cache
            .as_ref()
            .is_none_or(|(cached_len, z, _)| *cached_len != len || z != zone)
        {
            let agg = crate::combat::aggregate_session(
                self.engine.history.iter().filter(|e| e.zone == zone),
            );
            self.zone_cache = Some((len, zone.to_string(), agg));
        }
        self.zone_cache.as_ref().unwrap().2.clone()
    }

    /// Encounter prêt pour l'affichage : pets fusionnés + filtre alliés/ennemis.
    fn for_display(&self, enc: &Encounter) -> Encounter {
        let owners = self.effective_owners();
        let mut out = if self.config.merge_pets {
            enc.merged(&owners)
        } else {
            enc.clone()
        };
        if !self.config.show_enemies {
            // Calculé sur l'encounter brut (les arêtes y référencent les pets).
            let mut allies = crate::combat::compute_allies(
                enc,
                &self.engine.self_name,
                &self.engine.known_players,
                &owners,
            );
            // Les noms de joueurs EQ2 sont en un seul mot : un allié multi-mots
            // est un PNJ (mercenaire, PNJ de quête) — masqué si demandé.
            if self.config.hide_npcs {
                allies.retain(|n| !n.contains(' '));
            }
            out.allies = Some(allies);
        }
        out
    }

    /// Détail d'un encounter : tables, breakdown, comparaison, graphe.
    fn encounter_detail(&mut self, ui: &mut egui::Ui, enc: &Encounter) {
        let pinned: Option<Encounter> = self
            .compare_pin
            .and_then(|i| self.engine.history.get(i).cloned())
            .map(|e| self.for_display(&e));
        // Pas de comparaison avec soi-même.
        let pinned = pinned.filter(|_| {
            !(self.tab == Tab::Encounters && self.selected_encounter == self.compare_pin)
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.encounter_table(ui, enc);
            if let Some(name) = self.selected_combatant.clone() {
                ui.separator();
                self.ability_breakdown(ui, enc, &name);
            }
            // Rapports de mort (alliés uniquement, sauf si les ennemis sont affichés).
            let deaths: Vec<&crate::combat::DeathRecord> = enc
                .deaths_log
                .iter()
                .filter(|d| enc.visible(&d.victim) || enc.allies.is_none())
                .collect();
            if !deaths.is_empty() {
                ui.separator();
                egui::CollapsingHeader::new(format!("💀 Morts ({})", deaths.len()))
                    .default_open(deaths.len() <= 3)
                    .show(ui, |ui| {
                        for (di, d) in deaths.iter().enumerate() {
                            death_report(ui, enc, d, di);
                        }
                    });
            }

            if let Some(p) = &pinned {
                ui.separator();
                egui::CollapsingHeader::new(format!(
                    "⚖ Comparaison avec « {} » ({})",
                    p.title(),
                    fmt_duration(p.duration())
                ))
                .default_open(true)
                .show(ui, |ui| {
                    let mut st = *self.sort_states.entry("cmp").or_insert((2, true));
                    comparison_table(ui, p, enc, &mut st);
                    self.sort_states.insert("cmp", st);
                });
            }
            ui.separator();
            egui::CollapsingHeader::new("📈 Graphe")
                .default_open(true)
                .show(ui, |ui| {
                    graph_section(
                        ui,
                        enc,
                        pinned.as_ref(),
                        self.selected_combatant.as_deref(),
                        &mut self.graph_state,
                    );
                });

            // Log brut (combats de la session courante uniquement — non persisté).
            if !enc.raw_lines.is_empty() {
                ui.separator();
                egui::CollapsingHeader::new(format!(
                    "📜 Log brut ({} lignes)",
                    enc.raw_lines.len()
                ))
                .default_open(false)
                .show(ui, |ui| {
                    filter_box(ui, &mut self.filter_log, "filtrer les lignes…");
                    let filter = self.filter_log.to_lowercase();
                    let lines: Vec<&(u64, String)> = enc
                        .raw_lines
                        .iter()
                        .filter(|(_, m)| {
                            filter.is_empty() || m.to_lowercase().contains(&filter)
                        })
                        .collect();
                    egui::ScrollArea::vertical()
                        .id_salt("raw_log")
                        .max_height(300.0)
                        .show_rows(ui, 16.0, lines.len(), |ui, range| {
                            for (epoch, msg) in &lines[range] {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "[{}]",
                                            fmt_duration(epoch.saturating_sub(enc.start))
                                        ))
                                        .weak()
                                        .monospace()
                                        .size(11.0),
                                    );
                                    ui.label(
                                        RichText::new(msg.as_str()).monospace().size(11.0),
                                    );
                                });
                            }
                        });
                });
            }
        });
    }

    /// Recadre le screenshot du viewport sur le rect du graphe et sauvegarde en PNG.
    fn save_graph_png(&mut self, ctx: &egui::Context, img: &egui::ColorImage) {
        let Some(rect) = self.graph_state.last_plot_rect else { return };
        let ppp = ctx.pixels_per_point();
        let x0 = (rect.min.x * ppp).max(0.0) as usize;
        let y0 = (rect.min.y * ppp).max(0.0) as usize;
        let x1 = ((rect.max.x * ppp) as usize).min(img.width());
        let y1 = ((rect.max.y * ppp) as usize).min(img.height());
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut buf = Vec::with_capacity(w * h * 4);
        for y in y0..y1 {
            for x in x0..x1 {
                let c = img.pixels[y * img.width() + x];
                buf.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
            }
        }
        if let Some(p) = rfd::FileDialog::new()
            .set_file_name("graphe.png")
            .add_filter("PNG", &["png"])
            .save_file()
        {
            if let Some(im) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                let _ = im.save(p);
            }
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent : nécessaire pour l'overlay ; la fenêtre principale
        // repeint son fond via ses panels.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_history();
        self.engine.mech.save_if_dirty();
        self.config.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_lines();
        self.engine.tick(Self::now_epoch());
        self.trigger_engine.tick();
        self.process_mechanic_alerts();

        // Sauvegarde throttlée de la base de mécaniques apprise.
        if self.engine.mech.dirty && self.last_mech_save.elapsed() > Duration::from_secs(30) {
            self.engine.mech.save_if_dirty();
            self.last_mech_save = Instant::now();
        }

        // Raccourcis clavier : Ctrl+1..5 = onglets, Échap = fermer le breakdown.
        ctx.input_mut(|i| {
            use egui::{Key, Modifiers};
            if i.consume_key(Modifiers::CTRL, Key::Num1) {
                self.tab = Tab::Live;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num2) {
                self.tab = Tab::Encounters;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num3) {
                self.tab = Tab::Triggers;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num4) {
                self.tab = Tab::Mechanics;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num5) {
                self.tab = Tab::Optimizer;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num6) {
                self.tab = Tab::Settings;
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                self.selected_combatant = None;
            }
        });

        // Notifications de mise à jour.
        if let Ok(info) = self.update_rx.try_recv() {
            self.update_available = Some(info);
        }
        if let Some(rx) = &self.update_err_rx {
            if let Ok(err) = rx.try_recv() {
                self.update_error = Some(err);
                self.updating = false;
                self.update_err_rx = None;
            }
        }

        // Auto-sauvegarde de l'historique : nouveaux encounters + throttle 20 s.
        if self.engine.history.len() != self.last_hist_len
            && self.last_hist_save.elapsed() > Duration::from_secs(20)
        {
            self.save_history();
        }

        // Suivi auto du perso actif : si un autre log devient nettement plus
        // récent (relog sur un autre personnage), on bascule dessus.
        if self.config.auto_attach_latest
            && self.last_log_scan.elapsed() > Duration::from_secs(15)
        {
            self.last_log_scan = Instant::now();
            self.available_logs = discover_logs(&self.config.logs_dir);
            if let Some(latest) = self.available_logs.first().cloned() {
                let current = self.tailer.as_ref().map(|t| t.path.clone());
                let should_switch = match &current {
                    None => true,
                    Some(cur) if cur != &latest => {
                        let mtime = |p: &PathBuf| {
                            std::fs::metadata(p).and_then(|m| m.modified()).ok()
                        };
                        match (mtime(&latest), mtime(cur)) {
                            // Bascule seulement si l'autre log a > 60 s d'avance,
                            // pour ne pas osciller entre deux logs actifs.
                            (Some(l), Some(c)) => {
                                l > c + Duration::from_secs(60)
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                };
                if should_switch {
                    let name = char_name_from_path(&latest).unwrap_or_default();
                    self.attach(latest, ctx.clone());
                    self.trigger_engine.toasts.push(crate::triggers::Toast {
                        text: format!("Suivi auto : {name}"),
                        created: Instant::now(),
                    });
                }
            }
        }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Live, "⚔ Live");
                ui.selectable_value(&mut self.tab, Tab::Encounters, "📜 Encounters");
                ui.selectable_value(&mut self.tab, Tab::Triggers, "🔔 Triggers");
                ui.selectable_value(&mut self.tab, Tab::Mechanics, "⏱ Mécaniques");
                ui.selectable_value(&mut self.tab, Tab::Optimizer, "🎯 Optimisation");
                ui.selectable_value(&mut self.tab, Tab::Settings, "⚙ Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Bascule de thème clair/sombre.
                    let theme_icon = if self.config.light_mode { "🌙" } else { "☀" };
                    if ui
                        .button(theme_icon)
                        .on_hover_text(if self.config.light_mode {
                            "Passer en mode sombre"
                        } else {
                            "Passer en mode clair"
                        })
                        .clicked()
                    {
                        self.config.light_mode = !self.config.light_mode;
                        apply_theme(ctx, self.config.light_mode);
                        self.config.save();
                    }
                    // Bandeau de mise à jour.
                    if self.updating {
                        ui.label(
                            RichText::new("⬇ Mise à jour en cours…")
                                .color(Color32::from_rgb(241, 196, 15)),
                        );
                        ui.spinner();
                    } else if let Some(info) = self.update_available.clone() {
                        if ui
                            .button(
                                RichText::new(format!("✨ {} disponible — Mettre à jour", info.version))
                                    .color(Color32::from_rgb(46, 204, 113)),
                            )
                            .on_hover_text(
                                "Télécharge la nouvelle version, remplace l'exe et relance \
                                 (config, triggers et historique conservés).",
                            )
                            .clicked()
                        {
                            self.updating = true;
                            self.update_error = None;
                            let (etx, erx) = std::sync::mpsc::channel();
                            self.update_err_rx = Some(erx);
                            std::thread::spawn(move || {
                                if let Err(e) = crate::update::apply(&info.url) {
                                    let _ = etx.send(e);
                                }
                            });
                        }
                    }
                    if let Some(err) = &self.update_error {
                        ui.label(
                            RichText::new(format!("⚠ {err}"))
                                .color(Color32::from_rgb(231, 76, 60)),
                        )
                        .on_hover_text("La mise à jour a échoué — l'app actuelle reste intacte.");
                    }
                    let mut ov = self.config.overlay_enabled;
                    if ui.checkbox(&mut ov, "Overlay").changed() {
                        self.config.overlay_enabled = ov;
                        self.config.save();
                    }
                    if let Some(t) = &self.tailer {
                        ui.label(
                            RichText::new(format!(
                                "📄 {} ({} lignes)",
                                t.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                                self.lines_seen
                            ))
                            .weak(),
                        );
                    } else {
                        ui.label(RichText::new("Aucun log suivi — voir Settings").weak());
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Live => self.ui_live(ui),
            Tab::Encounters => self.ui_encounters(ui),
            Tab::Triggers => self.ui_triggers(ui),
            Tab::Mechanics => self.ui_mechanics(ui),
            Tab::Optimizer => self.ui_optimizer(ui),
            Tab::Settings => self.ui_settings(ui, ctx),
        });

        self.show_main_toasts(ctx);
        self.show_changelog_window(ctx);

        if self.config.overlay_enabled {
            self.show_overlay(ctx);
        } else {
            self.passthrough_sent = false;
        }
        if self.config.mech_overlay_window && self.config.mechanics_enabled {
            self.show_mech_overlay(ctx);
        }

        // Export PNG du graphe : screenshot du viewport puis recadrage.
        if self.graph_state.want_png {
            self.graph_state.want_png = false;
            self.awaiting_screenshot = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if self.awaiting_screenshot {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                self.awaiting_screenshot = false;
                self.save_graph_png(ctx, &img);
            }
        }

        // Repaint régulier pour le timer de combat même sans nouvelle ligne.
        ctx.request_repaint_after(Duration::from_millis(400));
    }
}

// ---------------------------------------------------------------------------
// Onglets
// ---------------------------------------------------------------------------

impl App {
    fn ui_live(&mut self, ui: &mut egui::Ui) {
        let Some(raw) = self.engine.display_encounter().cloned() else {
            self.ui_welcome(ui);
            return;
        };
        let enc = self.for_display(&raw);
        let live = self.engine.current.is_some();
        ui.horizontal(|ui| {
            ui.heading(enc.title());
            ui.label(
                RichText::new(format!(
                    "{} — {}",
                    fmt_duration(enc.duration()),
                    if live { "EN COURS" } else { "terminé" }
                ))
                .color(if live {
                    Color32::from_rgb(46, 204, 113)
                } else {
                    Color32::GRAY
                }),
            );
            ui.label(format!("Total : {}", fmt_num(enc.total_damage())));
            self.export_toolbar(ui, &enc);
        });
        ui.separator();
        self.encounter_detail(ui, &enc);
    }

    fn ui_encounters(&mut self, ui: &mut egui::Ui) {
        egui::SidePanel::left("enc_list")
            .resizable(true)
            .default_width(260.0)
            .show_inside(ui, |ui| {
                ui.heading("Historique");
                filter_box(ui, &mut self.filter_encounters, "filtrer par nom de mob…");
                ui.separator();
                let filter = self.filter_encounters.clone();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // Pseudo-encounter : toute la session cumulée.
                    if !self.engine.history.is_empty() {
                        let label = format!(
                            "Σ Session entière ({} combats)",
                            self.engine.history.len()
                        );
                        if ui
                            .selectable_label(self.session_selected, RichText::new(label).strong())
                            .clicked()
                        {
                            self.session_selected = true;
                            self.selected_zone = None;
                            self.selected_encounter = None;
                            self.selected_combatant = None;
                        }
                        ui.separator();
                    }
                    if self.engine.current.is_some() {
                        let sel = self.selected_encounter.is_none()
                            && !self.session_selected
                            && self.selected_zone.is_none();
                        if ui.selectable_label(sel, "▶ Combat en cours").clicked() {
                            self.selected_encounter = None;
                            self.session_selected = false;
                            self.selected_zone = None;
                            self.selected_combatant = None;
                        }
                    }
                    // Nombre de combats par zone (pour les en-têtes).
                    let mut zone_counts: HashMap<&str, usize> = HashMap::new();
                    for enc in &self.engine.history {
                        *zone_counts.entry(enc.zone.as_str()).or_default() += 1;
                    }
                    let mut shown = 0;
                    let mut selected: Option<usize> = None;
                    let mut zone_clicked: Option<String> = None;
                    let mut last_zone: Option<&str> = None;
                    for (i, enc) in self.engine.history.iter().enumerate().rev() {
                        if !matches_filter(&enc.title(), &filter) {
                            continue;
                        }
                        // En-tête de zone cliquable → stats agrégées de la zone.
                        if last_zone != Some(enc.zone.as_str()) {
                            last_zone = Some(enc.zone.as_str());
                            let zone = if enc.zone.is_empty() {
                                "(zone inconnue)"
                            } else {
                                enc.zone.as_str()
                            };
                            let count = zone_counts.get(enc.zone.as_str()).copied().unwrap_or(0);
                            ui.add_space(4.0);
                            let sel = self.selected_zone.as_deref() == Some(enc.zone.as_str());
                            if ui
                                .selectable_label(
                                    sel,
                                    RichText::new(format!("🗺 {zone} ({count})"))
                                        .small()
                                        .color(Color32::from_rgb(93, 173, 226)),
                                )
                                .on_hover_text("Clic : stats agrégées de la zone")
                                .clicked()
                            {
                                zone_clicked = Some(enc.zone.clone());
                            }
                        }
                        shown += 1;
                        let label = format!(
                            "{} — {} ({})",
                            enc.title(),
                            fmt_num(enc.total_damage()),
                            fmt_duration(enc.duration())
                        );
                        let sel = self.selected_encounter == Some(i)
                            && !self.session_selected
                            && self.selected_zone.is_none();
                        if ui.selectable_label(sel, label).clicked() {
                            selected = Some(i);
                        }
                    }
                    if let Some(z) = zone_clicked {
                        self.selected_zone = Some(z);
                        self.session_selected = false;
                        self.selected_encounter = None;
                        self.selected_combatant = None;
                    }
                    if let Some(i) = selected {
                        self.selected_encounter = Some(i);
                        self.session_selected = false;
                        self.selected_zone = None;
                        self.selected_combatant = None;
                    }
                    if shown == 0 && self.engine.current.is_none() {
                        ui.label(RichText::new("(vide)").weak());
                    }
                });
            });

        let raw = if self.session_selected {
            Some(self.session_aggregate())
        } else if let Some(zone) = self.selected_zone.clone() {
            Some(self.zone_aggregate(&zone))
        } else {
            match self.selected_encounter {
                Some(i) => self.engine.history.get(i).cloned(),
                None => self.engine.display_encounter().cloned(),
            }
        };
        let Some(raw) = raw else {
            ui.centered_and_justified(|ui| {
                ui.label("Sélectionne un encounter à gauche.");
            });
            return;
        };
        let enc = self.for_display(&raw);
        ui.horizontal(|ui| {
            if self.session_selected {
                ui.heading(format!(
                    "Σ Session entière ({} combats)",
                    self.engine.history.len()
                ));
            } else if let Some(zone) = &self.selected_zone {
                let count = self
                    .engine
                    .history
                    .iter()
                    .filter(|e| &e.zone == zone)
                    .count();
                let zone_label = if zone.is_empty() { "(zone inconnue)" } else { zone.as_str() };
                ui.heading(format!("🗺 {zone_label} ({count} combats)"));
            } else {
                ui.heading(enc.title());
                if !enc.zone.is_empty() {
                    ui.label(
                        RichText::new(format!("🗺 {}", enc.zone))
                            .color(Color32::from_rgb(93, 173, 226)),
                    );
                }
            }
            ui.label(format!(
                "{} — total {}",
                fmt_duration(enc.duration()),
                fmt_num(enc.total_damage())
            ));
            // Épingler un encounter de l'historique pour le comparer aux autres.
            if let Some(i) = self.selected_encounter {
                if self.compare_pin == Some(i) {
                    if ui
                        .button("📌 Épinglé")
                        .on_hover_text("Cliquer pour désépingler")
                        .clicked()
                    {
                        self.compare_pin = None;
                    }
                } else if ui
                    .button("📌 Comparer")
                    .on_hover_text(
                        "Épingle cet encounter ; ouvre ensuite un autre encounter pour les comparer",
                    )
                    .clicked()
                {
                    self.compare_pin = Some(i);
                }
            }
            self.export_toolbar(ui, &enc);
        });
        if !enc.kills.is_empty() {
            ui.label(
                RichText::new(format!("Kills : {}", enc.kills.join(", ")))
                    .weak()
                    .small(),
            );
        }
        ui.separator();
        self.encounter_detail(ui, &enc);
    }

    fn ui_triggers(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Triggers");
            if ui.button("➕ Ajouter").clicked() {
                // Nouveau trigger en haut de la liste (plus visible).
                self.config.triggers.insert(0, Trigger::default());
            }
            if ui
                .button("📦 Pack de base")
                .on_hover_text(
                    "Ajoute des triggers raid génériques : ready check, bannière, \
                     death prevents, debuffs de classe à recast, manastone…",
                )
                .clicked()
            {
                let n = self.add_base_triggers();
                self.trigger_engine.toasts.push(crate::triggers::Toast {
                    text: format!("{n} triggers de base ajoutés"),
                    created: Instant::now(),
                });
            }
            if ui
                .button("🗡 Pack ACT")
                .on_hover_text(
                    "Importe une config ACT (.xml) : ses timers de boss vont dans \
                     Mécaniques, ses triggers ici.",
                )
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Config ACT", &["xml"])
                    .pick_file()
                {
                    if let Ok(xml) = std::fs::read_to_string(&p) {
                        let (m, t) = self.import_act_pack(&xml);
                        self.trigger_engine.toasts.push(crate::triggers::Toast {
                            text: format!("ACT importé : {m} mécaniques, {t} triggers"),
                            created: Instant::now(),
                        });
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("📥 Importer")
                    .on_hover_text("Ajoute les triggers d'un fichier JSON (pack partagé)")
                    .clicked()
                {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Pack de triggers JSON", &["json"])
                        .pick_file()
                    {
                        if let Ok(s) = std::fs::read_to_string(&p) {
                            match serde_json::from_str::<Vec<Trigger>>(
                                s.trim_start_matches('\u{feff}'),
                            ) {
                                Ok(imported) => {
                                    let n = imported.len();
                                    self.config.triggers.extend(imported);
                                    self.trigger_engine.recompile(&self.config.triggers);
                                    self.config.save();
                                    self.trigger_engine.toasts.push(
                                        crate::triggers::Toast {
                                            text: format!("{n} triggers importés"),
                                            created: Instant::now(),
                                        },
                                    );
                                }
                                Err(e) => {
                                    self.trigger_engine.toasts.push(
                                        crate::triggers::Toast {
                                            text: format!("Import échoué : {e}"),
                                            created: Instant::now(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                if !self.config.triggers.is_empty()
                    && ui
                        .button("📤 Exporter")
                        .on_hover_text("Sauvegarde tous les triggers en JSON (partageable)")
                        .clicked()
                {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("triggers-eq2.json")
                        .add_filter("Pack de triggers JSON", &["json"])
                        .save_file()
                    {
                        if let Ok(json) = serde_json::to_string_pretty(&self.config.triggers)
                        {
                            let _ = std::fs::write(p, json);
                        }
                    }
                }
            });
        });
        ui.label(
            RichText::new(
                "Regex testée sur chaque ligne du log. Groupes de capture : `(?<who>\\w+) casts` \
                 puis `{who}` (ou `{1}`) dans le message/label. Ex : `has been slain by (?<killer>.+)!`",
            )
            .weak()
            .small(),
        );
        ui.separator();

        let mut changed = false;
        let mut to_remove: Option<usize> = None;
        let mut to_test: Option<(Option<PathBuf>, BeepKind)> = None;
        let mut to_test_tts: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, t) in self.config.triggers.iter_mut().enumerate() {
                // En-tête compact : état + nom + badges + aperçu du pattern.
                let mut badges = String::new();
                if t.tts {
                    badges.push_str("  🗣");
                }
                if t.timer_secs > 0 {
                    badges.push_str(&format!("  ⏱{}s", t.timer_secs));
                }
                if t.cooldown_secs > 0 {
                    badges.push_str(&format!("  🔁{}s", t.cooldown_secs));
                }
                if t.sound.is_some() {
                    badges.push_str("  🎵");
                }
                let pattern_preview: String = t.pattern.chars().take(42).collect();
                let title = format!(
                    "{}  {}{}   {}",
                    if t.enabled { "✅" } else { "⛔" },
                    if t.name.trim().is_empty() { "(sans nom)" } else { t.name.as_str() },
                    badges,
                    pattern_preview
                );
                egui::CollapsingHeader::new(title)
                    .id_salt(("trigger", i))
                    .default_open(false)
                    .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut t.enabled, "Actif").changed();
                        ui.label("Nom :");
                        changed |= ui
                            .add(egui::TextEdit::singleline(&mut t.name).desired_width(180.0))
                            .changed();
                        ui.label("Pattern :");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut t.pattern)
                                    .desired_width(280.0)
                                    .font(egui::TextStyle::Monospace),
                            )
                            .changed();
                        if regex::Regex::new(&t.pattern).is_err() {
                            ui.label(RichText::new("regex invalide").color(Color32::RED));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Message :");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut t.message)
                                    .hint_text("toast/TTS — {1} ou {nom} = capture ; vide = nom du trigger")
                                    .desired_width(340.0),
                            )
                            .changed();
                        changed |= ui.checkbox(&mut t.tts, "🗣 TTS").changed();
                        if ui.button("🔊 Test TTS").clicked() {
                            let msg = if t.message.trim().is_empty() { &t.name } else { &t.message };
                            to_test_tts = Some(msg.clone());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("⏱ Timer :");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut t.timer_secs)
                                    .range(0..=3600)
                                    .suffix(" s"),
                            )
                            .on_hover_text("0 = pas de timer ; sinon compte à rebours dans l'overlay")
                            .changed();
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut t.timer_label)
                                    .hint_text("label du timer ({nom} ok ; vide = nom)")
                                    .desired_width(200.0),
                            )
                            .changed();
                        ui.label("Cooldown :");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut t.cooldown_secs)
                                    .range(0..=3600)
                                    .suffix(" s"),
                            )
                            .on_hover_text("Ne pas re-déclencher pendant N secondes")
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut t.show_toast, "Toast overlay").changed();
                        let sound_label = t
                            .sound
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("(bip)");
                        ui.label(format!("Son : {sound_label}"));
                        if ui.button("📂").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Audio", &["wav", "mp3", "ogg", "flac"])
                                .pick_file()
                            {
                                t.sound = Some(p);
                                changed = true;
                            }
                        }
                        if t.sound.is_some() && ui.button("✖ bip").clicked() {
                            t.sound = None;
                            changed = true;
                        }
                        // Choix du bip intégré quand aucun fichier audio.
                        if t.sound.is_none() {
                            egui::ComboBox::from_id_salt(("beep", i))
                                .selected_text(t.beep.label())
                                .show_ui(ui, |ui| {
                                    for b in BeepKind::ALL {
                                        if ui
                                            .selectable_label(t.beep == b, b.label())
                                            .clicked()
                                        {
                                            t.beep = b;
                                            changed = true;
                                        }
                                    }
                                });
                        }
                        if ui.button("▶ Tester").clicked() {
                            to_test = Some((t.sound.clone(), t.beep));
                        }
                        if ui.button("🗑 Supprimer").clicked() {
                            to_remove = Some(i);
                        }
                    });
                });
            }
        });

        if let Some((s, b)) = to_test {
            self.trigger_engine.test_sound(&s, b);
        }
        if let Some(msg) = to_test_tts {
            self.trigger_engine.test_tts(&msg);
        }
        if let Some(i) = to_remove {
            self.config.triggers.remove(i);
            changed = true;
        }
        if changed {
            self.trigger_engine.recompile(&self.config.triggers);
            self.config.save();
        }
    }

    fn ui_optimizer(&mut self, ui: &mut egui::Ui) {
        use crate::optimizer::{report, Scenario};

        // --- Personnage (optionnel) : auto sur le perso suivi ---
        // On inclut le perso suivi même sans combat encore mesuré, pour afficher
        // sa classe (détectée à l'attache) et ses sorts dès l'ouverture.
        let mut chars = self.engine.prof.known_chars();
        let me = self.engine.self_name.clone();
        if !me.is_empty() && !chars.contains(&me) {
            chars.push(me);
            chars.sort();
        }
        if self
            .opt_char
            .as_ref()
            .is_some_and(|c| !chars.contains(c))
        {
            self.opt_char = None;
        }
        if self.opt_char.is_none() {
            self.opt_char = chars
                .iter()
                .find(|c| **c == self.engine.self_name)
                .or_else(|| chars.first())
                .cloned();
        }
        let char_opt = self.opt_char.clone();

        // Données mesurées (vide si pas de combat) + classe devinée.
        let obs = char_opt
            .as_ref()
            .map(|c| self.engine.prof.live(c))
            .unwrap_or_default();
        let class_hint = self.spell_db.infer_class(obs.keys());
        // Classe : re-détectée à chaque changement de perso (suit le perso), tout
        // en laissant un choix manuel persister tant qu'on ne change pas de perso.
        let char_changed = self.opt_char_prev != char_opt;
        self.opt_char_prev = char_opt.clone();
        // Mémorise la classe devinée pour ce perso (persistée) : on la retrouve
        // ensuite même sans combat dans la session courante.
        if let (Some(c), Some(h)) = (&char_opt, &class_hint) {
            if self.config.char_class.get(c) != Some(h) {
                self.config.char_class.insert(c.clone(), h.clone());
                self.config.save();
            }
        }
        // Classe par défaut = celle stockée pour ce perso, sinon devinée.
        let stored = char_opt
            .as_ref()
            .and_then(|c| self.config.char_class.get(c).cloned());
        if char_changed && char_opt.is_some() {
            if let Some(cl) = stored.clone().or_else(|| class_hint.clone()) {
                self.opt_class = Some(cl);
            }
        }
        if self.opt_class.is_none() {
            self.opt_class = stored.or_else(|| class_hint.clone());
        }
        let classes = self.spell_db.classes();

        ui.horizontal_wrapped(|ui| {
            ui.heading("🎯 Optimisation");
            ui.separator();
            ui.label("Perso :");
            let char_label = char_opt.clone().unwrap_or_else(|| "— aucun —".into());
            egui::ComboBox::from_id_salt("opt_char")
                .selected_text(char_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.opt_char, None, "— aucun —");
                    for c in &chars {
                        ui.selectable_value(&mut self.opt_char, Some(c.clone()), c);
                    }
                });
            ui.separator();
            ui.label("Classe :");
            let class_label = self
                .opt_class
                .clone()
                .unwrap_or_else(|| "— choisir —".into());
            egui::ComboBox::from_id_salt("opt_class")
                .selected_text(class_label)
                .show_ui(ui, |ui| {
                    for c in &classes {
                        ui.selectable_value(&mut self.opt_class, Some(c.clone()), c);
                    }
                });
        });
        let class = self.opt_class.clone();

        // --- Stats joueur (par perso, ou par classe en mode planif) ---
        let stats_key = char_opt
            .clone()
            .unwrap_or_else(|| format!("class:{}", class.as_deref().unwrap_or("?")));
        let mut stats = self
            .config
            .player_stats
            .get(&stats_key)
            .cloned()
            .unwrap_or_default();
        let mut stats_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Casting speed %");
            stats_changed |= ui
                .add(egui::DragValue::new(&mut stats.casting_speed).range(0.0..=200.0).speed(1.0))
                .on_hover_text("Bonus de vitesse d'incantation (réduit le temps de cast).")
                .changed();
            ui.separator();
            ui.label("Reuse %");
            stats_changed |= ui
                .add(egui::DragValue::new(&mut stats.reuse_speed).range(0.0..=200.0).speed(1.0))
                .on_hover_text("Bonus de réutilisation (réduit les recasts).")
                .changed();
            ui.separator();
            ui.label("Recovery s");
            stats_changed |= ui
                .add(egui::DragValue::new(&mut stats.recovery).range(0.0..=2.0).speed(0.05))
                .on_hover_text("Temps de récupération ajouté après le cast.")
                .changed();
        });
        if stats_changed {
            self.config.player_stats.insert(stats_key, stats.clone());
            self.config.save();
        }

        // --- Scénario (filtres) ---
        let mut scenario_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Scénario :");
            if ui.button("Mono").clicked() {
                self.config.opt_targets = 1;
                scenario_changed = true;
            }
            ui.label("cibles");
            scenario_changed |= ui
                .add(egui::DragValue::new(&mut self.config.opt_targets).range(1..=24))
                .changed();
            ui.separator();
            scenario_changed |= ui
                .checkbox(&mut self.config.opt_linked, "cibles liées")
                .on_hover_text(
                    "Cibles d'un même encounter (liées) : les sorts AE (vert) touchent tout le \
                     groupe. Décoché : les AE ne touchent qu'une cible. Les AoE (bleu, zone) \
                     touchent toujours tout, lié ou non.",
                )
                .changed();
            ui.separator();
            scenario_changed |= ui
                .checkbox(&mut self.config.opt_hide_unknown, "masquer hors-base")
                .on_hover_text(
                    "Masquer les sorts absents de la base (procs, pets, sorts récents au cast \
                     inféré). Par défaut tout ce que tu as lancé est affiché.",
                )
                .changed();
        });
        if scenario_changed {
            self.config.save();
        }
        ui.label(
            RichText::new(
                "Dégâts auto depuis tes logs (bleu), sinon saisis-les à la main (tooltip). \
                 Clique un en-tête pour trier, survole-le pour l'aide.",
            )
            .weak()
            .size(12.0),
        );
        // Légende : rôles (quoi faire) + couleurs de type.
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Rôles :").weak().size(12.0));
            ui.label(RichText::new("🔁 entretenir (DoT)").color(Color32::from_rgb(46, 204, 113)).size(12.0));
            ui.label(RichText::new("⏳ cooldown").color(Color32::from_rgb(230, 126, 34)).size(12.0));
            ui.label(RichText::new("▶ filler").color(Color32::from_rgb(150, 160, 175)).size(12.0));
            ui.separator();
            ui.label(RichText::new("Type :").weak().size(12.0));
            ui.label(RichText::new("● mono").color(Color32::from_rgb(231, 76, 60)).size(12.0));
            ui.label(RichText::new("● AoE zone").color(Color32::from_rgb(52, 152, 219)).size(12.0));
            ui.label(RichText::new("● AE lié").color(Color32::from_rgb(46, 204, 113)).size(12.0));
            ui.separator();
            ui.label(RichText::new("orange = saisi/CD").color(Color32::from_rgb(230, 126, 34)).size(12.0));
        });

        if class.is_none() && obs.is_empty() {
            ui.separator();
            ui.label(
                RichText::new(
                    "Choisis ta classe pour afficher ses sorts (et saisir les dégâts), ou lance \
                     un combat pour un remplissage automatique.",
                )
                .weak(),
            );
            return;
        }

        // --- Tableau ---
        let sc = Scenario {
            targets: self.config.opt_targets,
            linked: self.config.opt_linked,
        };
        let mut rows = report(
            &obs,
            &self.spell_db,
            class_hint.as_deref().or(class.as_deref()),
            &stats,
            &sc,
            &self.config.cast_overrides,
            &self.config.spell_damage,
            class.as_deref(),
            true,
        );
        // On affiche par défaut tout ce que le joueur a lancé (y compris les
        // sorts hors-base) ; « masquer hors-base » ne garde que les reconnus.
        if self.config.opt_hide_unknown {
            rows.retain(|r| r.from_db || self.config.cast_overrides.contains_key(&r.ability));
        }
        if rows.is_empty() {
            ui.separator();
            ui.label(RichText::new("Aucun sort à afficher.").weak());
            return;
        }

        // Tri : colonne choisie, sorts masqués toujours en bas.
        let key = self.config.opt_sort_key.clone();
        let desc = self.config.opt_sort_desc;
        let val = |r: &crate::optimizer::SpellRow| -> f64 {
            match key.as_str() {
                "dmg" => r.dmg_per_cast,
                "crit" => r.crit_rate,
                "targets" => r.avg_targets,
                "cast" => r.cast_eff as f64,
                "recast" => r.recast_eff.unwrap_or(0.0) as f64,
                "sustained" => r.sustained_dps,
                _ => r.efficiency,
            }
        };
        {
            let hidden = &self.config.opt_hidden;
            rows.sort_by(|a, b| {
                hidden
                    .contains(&a.ability)
                    .cmp(&hidden.contains(&b.ability))
                    .then_with(|| {
                        let o = val(a).total_cmp(&val(b));
                        if desc { o.reverse() } else { o }
                    })
                    .then_with(|| a.ability.cmp(&b.ability))
            });
        }
        // Maxima (sorts visibles) pour l'échelle des barres.
        let max_eff = rows
            .iter()
            .filter(|r| !self.config.opt_hidden.contains(&r.ability))
            .map(|r| r.efficiency)
            .fold(0.0_f64, f64::max);
        let max_sus = rows
            .iter()
            .filter(|r| !self.config.opt_hidden.contains(&r.ability))
            .map(|r| r.sustained_dps)
            .fold(0.0_f64, f64::max);
        let sort_eff = key == "eff";
        let sort_sus = key == "sustained";

        // --- Diagnostic de rotation (sur les sorts mesurés) ---
        if let Some(c) = &char_opt {
            let ct = self.engine.prof.combat_time(c) as f64;
            let diag = crate::optimizer::diagnose(&rows, ct);
            if diag.total_casts > 0 {
                egui::CollapsingHeader::new("📊 Diagnostic de rotation")
                    .id_salt("opt_diag")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mins = diag.combat_time as u64 / 60;
                        let secs = diag.combat_time as u64 % 60;
                        ui.label(format!(
                            "Combat analysé : {mins}:{secs:02} · {} casts · ~{:.0}s à caster",
                            diag.total_casts, diag.cast_time
                        ));
                        ui.add(
                            egui::ProgressBar::new(diag.gcd_util as f32)
                                .desired_width(260.0)
                                .text(format!(
                                    "Activité GCD {:.0}% · temps mort {:.0}%",
                                    diag.gcd_util * 100.0,
                                    (1.0 - diag.gcd_util) * 100.0
                                )),
                        )
                        .on_hover_text(
                            "Part du combat réellement passée à caster. Le reste = temps mort \
                             (déplacements, attente, GCD perdus).",
                        );
                        if diag.low_yield_frac > 0.0 {
                            ui.label(
                                RichText::new(format!(
                                    "{:.0}% de ton temps de cast sur des sorts à faible rendement.",
                                    diag.low_yield_frac * 100.0
                                ))
                                .color(Color32::from_rgb(230, 180, 60)),
                            );
                        }
                        if !diag.underused.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("À mieux entretenir (DoT / cooldowns) :").strong(),
                            );
                            for u in &diag.underused {
                                ui.label(format!(
                                    "• {} : entretien ~{:.0}% ({} cast(s) / ~{:.0}) → ~{} dégâts \
                                     en plus si maintenu",
                                    u.ability,
                                    u.uptime * 100.0,
                                    u.casts,
                                    u.expected,
                                    fmt_f64(u.lost_damage)
                                ));
                            }
                        }
                    });
                ui.separator();
            }
        }

        // Réinitialiser les surcharges manuelles des sorts affichés.
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("↺ Réinit. surcharges")
                .on_hover_text("Efface les cast/dégâts saisis à la main pour les sorts affichés.")
                .clicked()
            {
                for r in &rows {
                    self.config.cast_overrides.remove(&r.ability);
                    self.config.spell_damage.remove(&r.ability);
                }
                self.config.save();
            }
        });
        ui.separator();

        let mut to_save = false;
        let mut new_sort: Option<&str> = None;
        egui::ScrollArea::vertical()
            .id_salt("opt_scroll")
            .show(ui, |ui| {
                egui::Grid::new("opt_table")
                    .num_columns(11)
                    .striped(true)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        // En-têtes (cliquables = triables, survol = aide).
                        let headers: [(&str, Option<&str>, &str); 11] = [
                            ("", None, "Masquer (renvoie en bas de la liste)."),
                            ("Sort", None, ""),
                            (
                                "Rôle",
                                None,
                                "Comment l'utiliser : 🔁 Entretenir (DoT), ⏳ Cooldown (presser \
                                 dès dispo), ▶ Filler (sur un GCD libre).",
                            ),
                            (
                                "Type",
                                None,
                                "Forme des dégâts : mono (rouge), AoE zone (bleu, touche tout), \
                                 AE encounter (vert, mobs liés).",
                            ),
                            ("Dég/cast", Some("dmg"), "Dégâts par cast (mesurés en bleu, ou saisis)."),
                            ("% crit", Some("crit"), "Taux de critique observé."),
                            ("Cibles", Some("targets"), "Cibles moyennes touchées par cast."),
                            (
                                "Cast",
                                Some("cast"),
                                "Temps de cast de base (éditable). GCD = cast effectif + recovery.",
                            ),
                            ("Recast", Some("recast"), "Cooldown effectif. ⟳ orange = cooldown long."),
                            (
                                "Eff/GCD",
                                Some("eff"),
                                "Valeur d'un cast sur un GCD libre = dégâts / (cast + recovery). \
                                 Sert de priorité : « je lance quoi maintenant ? ».",
                            ),
                            (
                                "DPS soutenu",
                                Some("sustained"),
                                "Contribution réelle dans la durée = dégâts / intervalle utile \
                                 (reuse et durée du DoT inclus).",
                            ),
                        ];
                        for (label, k, tip) in headers {
                            match k {
                                Some(k) => {
                                    let arrow = if key == k {
                                        if desc { " ▼" } else { " ▲" }
                                    } else {
                                        ""
                                    };
                                    let resp = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(format!("{label}{arrow}")).strong(),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(tip);
                                    if resp.clicked() {
                                        new_sort = Some(k);
                                    }
                                }
                                None => {
                                    let l = ui.label(RichText::new(label).strong());
                                    if !tip.is_empty() {
                                        l.on_hover_text(tip);
                                    }
                                }
                            }
                        }
                        ui.end_row();

                        // Barre + valeur dans une cellule de largeur fixe.
                        let bar_cell =
                            |ui: &mut egui::Ui, v: f64, maxv: f64, active: bool, dim: bool| {
                                let h = ui.spacing().interact_size.y;
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(94.0, h),
                                    egui::Sense::hover(),
                                );
                                if v <= 0.0 {
                                    ui.painter().text(
                                        rect.left_center() + egui::vec2(4.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        "-",
                                        egui::FontId::proportional(13.0),
                                        ui.visuals().weak_text_color(),
                                    );
                                    return;
                                }
                                let frac = if maxv > 0.0 {
                                    (v / maxv).clamp(0.0, 1.0) as f32
                                } else {
                                    0.0
                                };
                                let base = if active {
                                    Color32::from_rgb(46, 204, 113)
                                } else {
                                    Color32::from_rgb(120, 130, 150)
                                };
                                let bar = if dim { base.gamma_multiply(0.4) } else { base };
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        rect.min,
                                        egui::vec2(rect.width() * frac, rect.height()),
                                    ),
                                    2.0,
                                    bar.gamma_multiply(0.30),
                                );
                                let txt = if active {
                                    base
                                } else if dim {
                                    ui.visuals().weak_text_color()
                                } else {
                                    ui.visuals().text_color()
                                };
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(4.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    fmt_f64(v),
                                    egui::FontId::proportional(13.0),
                                    txt,
                                );
                            };

                        for r in &rows {
                            // Case masquer (envoie en bas + grise).
                            let mut hidden = self.config.opt_hidden.contains(&r.ability);
                            if ui
                                .checkbox(&mut hidden, "")
                                .on_hover_text("Masquer ce sort (le renvoie en bas, grisé).")
                                .changed()
                            {
                                if hidden {
                                    self.config.opt_hidden.insert(r.ability.clone());
                                } else {
                                    self.config.opt_hidden.remove(&r.ability);
                                }
                                to_save = true;
                            }
                            let dim = hidden;

                            let hover = if r.observed {
                                format!(
                                    "Mesuré : {} cast(s) · {} de dégâts totaux\n\
                                     Dégâts/cast au scénario : {}\nCast effectif : {:.2} s",
                                    r.casts,
                                    fmt_f64(r.total_damage as f64),
                                    fmt_f64(r.scenario_dmg),
                                    r.cast_eff,
                                )
                            } else {
                                "Pas encore vu en combat : saisis les dégâts du tooltip dans la \
                                 colonne Dég/cast."
                                    .to_string()
                            };
                            let hover = format!(
                                "{hover}\nIntervalle utile : {:.1} s{}",
                                r.interval,
                                if r.is_dot { " (DoT à entretenir)" } else { "" },
                            );
                            let name = if dim {
                                RichText::new(&r.ability).weak()
                            } else {
                                RichText::new(&r.ability).strong()
                            };
                            ui.label(name).on_hover_text(hover);

                            // Rôle : comment utiliser le sort (la vraie info actionnable).
                            let (role_icon, role_col, role_hover) = if r.is_dot {
                                (
                                    "🔁",
                                    Color32::from_rgb(46, 204, 113),
                                    "Entretenir : applique le DoT et laisse-le tiquer ; rafraîchis \
                                     avant qu'il tombe.",
                                )
                            } else if r.long_cd {
                                (
                                    "⏳",
                                    Color32::from_rgb(230, 126, 34),
                                    "Cooldown : gros sort à presser dès qu'il est disponible.",
                                )
                            } else {
                                (
                                    "▶",
                                    Color32::from_rgb(150, 160, 175),
                                    "Filler : à lancer sur un GCD libre quand rien de prioritaire.",
                                )
                            };
                            let role_col = if dim { role_col.gamma_multiply(0.5) } else { role_col };
                            ui.label(RichText::new(role_icon).color(role_col))
                                .on_hover_text(role_hover);

                            // Type coloré : mono = rouge, AoE (zone) = bleu, AE = vert.
                            let type_col = if r.kind.contains("AoE") {
                                Color32::from_rgb(52, 152, 219)
                            } else if r.kind.contains("AE") {
                                Color32::from_rgb(46, 204, 113)
                            } else {
                                Color32::from_rgb(231, 76, 60)
                            };
                            let type_col = if dim { type_col.gamma_multiply(0.5) } else { type_col };
                            ui.label(RichText::new(r.kind).color(type_col));

                            // Dégâts : mesurés (auto, bleu) ou éditables (saisie manuelle).
                            if r.observed {
                                ui.label(
                                    RichText::new(fmt_f64(r.dmg_per_cast))
                                        .color(Color32::from_rgb(52, 152, 219)),
                                )
                                .on_hover_text("Mesuré dans les logs");
                            } else {
                                let overridden = self.config.spell_damage.contains_key(&r.ability);
                                let mut d = r.dmg_per_cast;
                                let resp = ui
                                    .scope(|ui| {
                                        if overridden {
                                            ui.visuals_mut().override_text_color =
                                                Some(Color32::from_rgb(230, 126, 34));
                                        }
                                        ui.add(
                                            egui::DragValue::new(&mut d)
                                                .range(0.0..=1.0e9)
                                                .speed(100.0),
                                        )
                                    })
                                    .inner;
                                if resp.changed() {
                                    if d > 0.0 {
                                        self.config.spell_damage.insert(r.ability.clone(), d);
                                    } else {
                                        self.config.spell_damage.remove(&r.ability);
                                    }
                                    to_save = true;
                                }
                            }

                            if r.observed {
                                ui.label(format!("{:.0}%", r.crit_rate));
                                ui.label(format!("{:.1}", r.avg_targets));
                            } else {
                                ui.label("-");
                                ui.label("-");
                            }

                            // Cast de base éditable (surcharge manuelle, orange si surchargé).
                            let cast_over = self.config.cast_overrides.contains_key(&r.ability);
                            let mut base = r.base_cast;
                            let resp = ui
                                .scope(|ui| {
                                    if cast_over {
                                        ui.visuals_mut().override_text_color =
                                            Some(Color32::from_rgb(230, 126, 34));
                                    }
                                    ui.add(
                                        egui::DragValue::new(&mut base)
                                            .range(0.0..=15.0)
                                            .speed(0.1)
                                            .suffix(if r.from_db { "" } else { " ?" }),
                                    )
                                })
                                .inner;
                            if resp.changed() {
                                self.config.cast_overrides.insert(r.ability.clone(), base);
                                to_save = true;
                            }

                            // Recast : orange si cooldown long.
                            match r.recast_eff {
                                Some(rc) if r.long_cd => {
                                    ui.label(
                                        RichText::new(format!("⟳ {rc:.0}"))
                                            .color(Color32::from_rgb(230, 126, 34)),
                                    )
                                    .on_hover_text("Cooldown long : à lancer dès qu'il est dispo.");
                                }
                                Some(rc) => {
                                    ui.label(format!("{rc:.0}"));
                                }
                                None => {
                                    ui.label("-");
                                }
                            };

                            // Barres Eff/GCD et DPS soutenu (la colonne triée en vert).
                            bar_cell(ui, r.efficiency, max_eff, sort_eff, dim);
                            bar_cell(ui, r.sustained_dps, max_sus, sort_sus, dim);
                            ui.end_row();
                        }
                    });
            });

        // Application du tri choisi par clic d'en-tête.
        if let Some(k) = new_sort {
            if self.config.opt_sort_key == k {
                self.config.opt_sort_desc = !self.config.opt_sort_desc;
            } else {
                self.config.opt_sort_key = k.to_string();
                self.config.opt_sort_desc = true;
            }
            to_save = true;
        }
        if to_save {
            self.config.save();
        }
    }

    fn ui_mechanics(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("mech_scroll")
            .show(ui, |ui| {
                // Réglages globaux.
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .checkbox(&mut self.config.mechanics_enabled, "Activer les mécaniques")
                        .changed()
                    {
                        self.config.save();
                    }
                    ui.separator();
                    ui.label("Alerte par défaut :");
                    let mut a = self.config.mech_default_alert;
                    if alert_combo(ui, "mech_default_alert", &mut a, false) {
                        self.config.mech_default_alert = a;
                        self.config.save();
                    }
                    ui.separator();
                    if ui
                        .checkbox(&mut self.config.mech_overlay, "Décompte dans l'overlay DPS")
                        .changed()
                    {
                        self.config.save();
                    }
                    ui.separator();
                    if ui
                        .checkbox(
                            &mut self.config.mech_overlay_window,
                            "Overlay mécaniques dédié",
                        )
                        .on_hover_text(
                            "Fenêtre overlay séparée (déplaçable, always-on-top) listant les \
                             prochaines mécaniques de boss en compte à rebours.",
                        )
                        .changed()
                    {
                        self.config.save();
                    }
                });
                ui.label(
                    RichText::new(
                        "Sans base de sorts : l'app apprend des logs les capacités ennemies \
                         récurrentes et impactantes (AoE, tank buster, mortelles) puis prévient \
                         avant le prochain cast. Tu peux aussi en ajouter à la main et partager \
                         ta base (Exporter).",
                    )
                    .weak()
                    .size(12.0),
                );
                ui.separator();

                // --- Prédictions live ---
                let now = Self::now_epoch();
                let zone = self.engine.mech.zone.clone();
                let zlabel = if zone.is_empty() {
                    "(zone inconnue)".to_string()
                } else {
                    zone.clone()
                };
                let preds = self.engine.mech.predictions(now);
                ui.heading(format!("⏱ Prochaines mécaniques — {zlabel}"));
                if preds.is_empty() {
                    ui.label(
                        RichText::new(
                            "Aucune mécanique chronométrée en cours. Elles apparaissent dès \
                             qu'un cast récurrent connu est repéré dans le combat.",
                        )
                        .weak(),
                    );
                } else {
                    egui::Grid::new("mech_preds")
                        .num_columns(3)
                        .striped(true)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            for p in preds.iter().take(12) {
                                ui.label(format!("{} {}", p.kind.icon(), p.kind.label()));
                                ui.label(RichText::new(&p.ability).strong());
                                let frac = (p.eta / p.period.max(1.0)).clamp(0.0, 1.0) as f32;
                                let urgent = p.eta <= p.lead as f64;
                                let col = if urgent {
                                    Color32::from_rgb(231, 76, 60)
                                } else {
                                    Color32::from_rgb(46, 204, 113)
                                };
                                let txt = if p.eta < 0.0 {
                                    "maintenant".to_string()
                                } else {
                                    format!("{:.0}s", p.eta)
                                };
                                gauge_cell(ui, &txt, frac, col);
                                ui.end_row();
                            }
                        });
                }
                ui.separator();

                // --- Base de mécaniques (éditeur) ---
                let mut do_save = false;
                ui.horizontal_wrapped(|ui| {
                    ui.heading(format!("📖 Base ({})", self.engine.mech.db.entries.len()));
                    if ui.button("➕ Ajouter").clicked() {
                        self.engine.mech.db.entries.insert(
                            0,
                            MechEntry {
                                source: MechSource::Manual,
                                ability: "Nouvelle capacité".into(),
                                zone: zone.clone(),
                                period: 30.0,
                                ..Default::default()
                            },
                        );
                        do_save = true;
                    }
                    if ui
                        .button("📥 Importer")
                        .on_hover_text("Fusionne un fichier mechanics.json partagé dans ta base.")
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file()
                        {
                            if let Ok(s) = std::fs::read_to_string(&p) {
                                if let Some(db) = crate::mechanics::MechanicsDb::from_str(&s) {
                                    self.engine.mech.db.absorb_db(&db);
                                    do_save = true;
                                }
                            }
                        }
                    }
                    if ui
                        .button("📤 Exporter")
                        .on_hover_text(
                            "Exporte ta base pour la partager (à m'envoyer pour l'intégrer \
                             à la base communautaire des prochaines versions).",
                        )
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_file_name("mechanics.json")
                            .add_filter("JSON", &["json"])
                            .save_file()
                        {
                            self.engine.mech.db.save_to(&p);
                        }
                    }
                    if ui
                        .button("🧹 Nettoyer")
                        .on_hover_text(
                            "Supprime les mécaniques apprises de mobs non-boss (trash : \
                             « a golem », etc.). Ne touche ni aux manuelles ni aux bundlées.",
                        )
                        .clicked()
                    {
                        let before = self.engine.mech.db.entries.len();
                        self.engine.mech.db.entries.retain(|e| {
                            !(matches!(e.source, MechSource::Learned)
                                && !crate::mechanics::is_named_mob(&e.mob))
                        });
                        let removed = before - self.engine.mech.db.entries.len();
                        if removed > 0 {
                            do_save = true;
                        }
                        self.trigger_engine.toasts.push(crate::triggers::Toast {
                            text: format!("{removed} mécanique(s) trash supprimée(s)"),
                            created: Instant::now(),
                        });
                    }
                    if ui
                        .button("🗡 Pack ACT")
                        .on_hover_text(
                            "Importe une config ACT (.xml) : ses timers de boss deviennent \
                             des mécaniques (et ses triggers vont dans l'onglet Triggers).",
                        )
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Config ACT", &["xml"])
                            .pick_file()
                        {
                            if let Ok(xml) = std::fs::read_to_string(&p) {
                                let (m, t) = self.import_act_pack(&xml);
                                self.trigger_engine.toasts.push(crate::triggers::Toast {
                                    text: format!(
                                        "ACT importé : {m} mécaniques, {t} triggers"
                                    ),
                                    created: Instant::now(),
                                });
                            }
                        }
                    }
                });
                filter_box(ui, &mut self.filter_mech, "filtrer (capacité, zone, mob)…");
                let filter = self.filter_mech.to_lowercase();

                // Tri par impact, filtre texte.
                let mut idxs: Vec<usize> = (0..self.engine.mech.db.entries.len()).collect();
                idxs.sort_by(|&a, &b| {
                    self.engine.mech.db.entries[b]
                        .impact_score()
                        .partial_cmp(&self.engine.mech.db.entries[a].impact_score())
                        .unwrap()
                });
                let mut to_delete: Option<usize> = None;
                for i in idxs {
                    let e = &self.engine.mech.db.entries[i];
                    if !filter.is_empty()
                        && !(e.ability.to_lowercase().contains(&filter)
                            || e.zone.to_lowercase().contains(&filter)
                            || e.mob.to_lowercase().contains(&filter))
                    {
                        continue;
                    }
                    let zlbl = if e.zone.is_empty() {
                        "(toutes zones)".to_string()
                    } else {
                        e.zone.clone()
                    };
                    let period_lbl = if e.is_timed() {
                        format!("toutes les {:.0}s", e.period)
                    } else {
                        "non chronométrée".to_string()
                    };
                    let src = match e.source {
                        MechSource::Bundled => "📦",
                        MechSource::Learned => "🧠",
                        MechSource::Manual => "✍",
                    };
                    let dot = if e.enabled { "" } else { " (désactivée)" };
                    let header =
                        format!("{src} {} {} — {zlbl} · {period_lbl}{dot}", e.kind.icon(), e.ability);
                    egui::CollapsingHeader::new(header)
                        .id_salt(("mech_entry", i))
                        .show(ui, |ui| {
                            let e = &mut self.engine.mech.db.entries[i];
                            let mut ch = false;
                            ui.horizontal(|ui| {
                                ch |= ui.checkbox(&mut e.enabled, "Activée").changed();
                                if ui.button("🗑 Supprimer").clicked() {
                                    to_delete = Some(i);
                                }
                            });
                            egui::Grid::new(("mech_grid", i))
                                .num_columns(2)
                                .spacing([8.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label("Capacité");
                                    ch |= ui.text_edit_singleline(&mut e.ability).changed();
                                    ui.end_row();
                                    ui.label("Zone");
                                    ch |= ui.text_edit_singleline(&mut e.zone).changed();
                                    ui.end_row();
                                    ui.label("Mob");
                                    ch |= ui.text_edit_singleline(&mut e.mob).changed();
                                    ui.end_row();
                                    ui.label("Type");
                                    ch |= kind_combo(ui, i, &mut e.kind);
                                    ui.end_row();
                                    ui.label("Période (s)");
                                    ch |= ui
                                        .add(
                                            egui::DragValue::new(&mut e.period)
                                                .range(0.0..=600.0)
                                                .speed(0.5),
                                        )
                                        .on_hover_text("0 = non chronométrée (pas de décompte)")
                                        .changed();
                                    ui.end_row();
                                    ui.label("Avance d'alerte (s)");
                                    ch |= ui
                                        .add(egui::DragValue::new(&mut e.lead).range(0..=60))
                                        .changed();
                                    ui.end_row();
                                    ui.label("Alerte");
                                    let mut a = e.alert;
                                    if alert_combo(ui, ("alert", i), &mut a, true) {
                                        e.alert = a;
                                        ch = true;
                                    }
                                    ui.end_row();
                                    ui.label("Message");
                                    ch |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut e.message)
                                                .hint_text("auto — {eta} = secondes restantes"),
                                        )
                                        .changed();
                                    ui.end_row();
                                });
                            ui.label(
                                RichText::new(format!(
                                    "🧠 {} casts vus · {} échantillons · max {} · {} cibles · {} morts",
                                    e.casts_seen,
                                    e.samples,
                                    fmt_num(e.max_hit),
                                    e.max_targets,
                                    e.lethal
                                ))
                                .weak()
                                .size(11.0),
                            );
                            if ch {
                                do_save = true;
                            }
                        });
                }
                if let Some(i) = to_delete {
                    self.engine.mech.db.entries.remove(i);
                    do_save = true;
                }
                if do_save {
                    self.engine.mech.dirty = true;
                    self.engine.mech.save_if_dirty();
                    self.last_mech_save = Instant::now();
                }
            });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .show(ui, |ui| {
                self.ui_settings_inner(ui, ctx);
            });
    }

    fn ui_settings_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        section(ui, "📄 Fichier de log & personnage", true, |ui| {
            self.settings_log(ui, ctx)
        });
        section(ui, "⚔ Combat", false, |ui| self.settings_combat(ui));
        section(ui, "💾 Historique", false, |ui| self.settings_history(ui));
        section(ui, "🐾 Pets", false, |ui| self.settings_pets(ui));
        section(ui, "🎨 Overlay", false, |ui| self.settings_overlay(ui));
        section(ui, "🔄 Mises à jour", false, |ui| self.settings_updates(ui));
        #[cfg(debug_assertions)]
        section(ui, "🧪 Debug (tests)", true, |ui| self.settings_debug(ui));
    }

    #[cfg(debug_assertions)]
    fn settings_debug(&mut self, ui: &mut egui::Ui) {
        use crate::mechanics::MechKind;
        let now = Self::now_epoch();
        ui.label(
            RichText::new(
                "Injecte des mécaniques de test récurrentes (visibles dans l'onglet Mécaniques, \
                 l'overlay dédié, et déclenchent les alertes selon ton réglage).",
            )
            .weak()
            .size(12.0),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ AoE 15s").clicked() {
                self.engine.mech.debug_add("Test AoE", MechKind::Aoe, 15.0, now);
            }
            if ui.button("+ Tank buster 25s").clicked() {
                self.engine
                    .mech
                    .debug_add("Test Tank Buster", MechKind::TankBuster, 25.0, now);
            }
            if ui.button("+ Mortel 40s").clicked() {
                self.engine
                    .mech
                    .debug_add("Test Mortel", MechKind::Lethal, 40.0, now);
            }
            if ui.button("+ Burst 12s").clicked() {
                self.engine
                    .mech
                    .debug_add("Test Burst", MechKind::Burst, 12.0, now);
            }
            if ui.button("🧹 Clear").clicked() {
                self.engine.mech.debug_clear();
            }
        });
        ui.separator();
        if ui
            .button("⚔ Combat de test")
            .on_hover_text(
                "Injecte un encounter factice (toi + un allié + un boss) pour tester l'overlay \
                 DPS, les classements et le graphe.",
            )
            .clicked()
        {
            self.debug_fake_combat();
        }
    }

    #[cfg(debug_assertions)]
    fn debug_fake_combat(&mut self) {
        let name = if self.engine.self_name.is_empty() {
            "Testeur".to_string()
        } else {
            self.engine.self_name.clone()
        };
        self.engine.self_name = name.clone();
        let parser = crate::parser::Parser::new(name.clone());
        let now = Self::now_epoch();
        let date = "Tue May 26 17:42:26 2026";
        for k in 0..20u64 {
            let t = now.saturating_sub(20) + k;
            for l in [
                format!(
                    "({t})[{date}] YOU hit Boss de test for {} crushing damage.",
                    4000 + k * 50
                ),
                format!(
                    "({t})[{date}] Allié's Flamboiement hits Boss de test for {} heat damage.",
                    3000 + k * 30
                ),
                format!("({t})[{date}] Boss de test hits {name} for 1500 crushing damage."),
            ] {
                if let Some(p) = parser.parse_line(&l) {
                    self.engine.process(&p);
                }
            }
        }
    }

    fn settings_log(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label("Répertoire logs EQ2 :");
            ui.label(RichText::new(self.config.logs_dir.display().to_string()).monospace());
            if ui.button("📂 Changer").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.config.logs_dir = d;
                    self.available_logs = discover_logs(&self.config.logs_dir);
                    self.config.save();
                }
            }
            if ui.button("🔄 Rafraîchir").clicked() {
                self.available_logs = discover_logs(&self.config.logs_dir);
            }
            if ui
                .button("🔍 Détecter")
                .on_hover_text(
                    "Cherche l'installation EQ2 : bibliothèques Steam \
                     (libraryfolders.vdf) et chemins usuels sur tous les disques.",
                )
                .clicked()
            {
                match crate::tailer::detect_logs_dir() {
                    Some(dir) => {
                        self.config.logs_dir = dir;
                        self.available_logs = discover_logs(&self.config.logs_dir);
                        self.config.save();
                    }
                    None => {
                        self.trigger_engine.toasts.push(crate::triggers::Toast {
                            text: "Installation EQ2 introuvable — choisis le dossier \
                                   logs manuellement (📂)"
                                .into(),
                            created: Instant::now(),
                        });
                    }
                }
            }
        });
        if self.available_logs.is_empty() {
            ui.label(
                RichText::new(
                    "⚠ Aucun log trouvé. En jeu, tape /log pour activer le logging \
                     (le fichier apparaît dans <EverQuest 2>\\logs\\<Serveur>\\).",
                )
                .color(Color32::from_rgb(241, 196, 15)),
            );
        }

        ui.checkbox(
            &mut self.config.import_existing,
            "Relire tout le fichier à l'attache (import de l'historique)",
        );
        if ui
            .checkbox(
                &mut self.config.auto_attach_latest,
                "Suivre automatiquement le perso actif (le log le plus récent)",
            )
            .on_hover_text(
                "Au lancement et en continu : si un autre log devient le plus récent \
                 (relog sur un autre perso), l'app bascule dessus toute seule. \
                 Décoche pour figer le choix manuel.",
            )
            .changed()
        {
            self.config.save();
        }

        ui.add_space(6.0);
        ui.label("Personnages détectés (du plus récent au plus ancien) :");
        let mut attach: Option<PathBuf> = None;
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for log in &self.available_logs {
                let server = log
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                let name = char_name_from_path(log).unwrap_or_else(|| "?".into());
                let active = self.tailer.as_ref().is_some_and(|t| &t.path == log);
                let label = format!("{name} @ {server}");
                if ui.selectable_label(active, label).clicked() {
                    attach = Some(log.clone());
                }
            }
        });
        if let Some(p) = attach {
            self.attach(p, ctx.clone());
        }
    }

    fn settings_combat(&mut self, ui: &mut egui::Ui) {
        if ui
            .add(
                egui::Slider::new(&mut self.config.encounter_timeout, 3..=30)
                    .text("Timeout encounter (s)"),
            )
            .changed()
        {
            self.engine.timeout = self.config.encounter_timeout;
            self.config.save();
        }
        if ui
            .checkbox(
                &mut self.config.encounter_anchor,
                "Clore sur mon activité (ignorer les combats voisins)",
            )
            .on_hover_text(
                "Le combat se clôt quand toi/ton groupe arrêtez, même si des joueurs hors \
                 groupe ou des PNJ se battent à côté. Désactive pour suivre tout le combat \
                 de la zone.",
            )
            .changed()
        {
            self.engine.anchor = self.config.encounter_anchor;
            self.config.save();
        }
        if ui
            .checkbox(
                &mut self.config.show_enemies,
                "Afficher les ennemis (mobs) dans les classements",
            )
            .on_hover_text(
                "Par défaut, seuls les alliés apparaissent (inférence : qui attaque qui, \
                 qui soigne qui).",
            )
            .changed()
        {
            self.config.save();
        }
        if ui
            .checkbox(
                &mut self.config.hide_npcs,
                "Masquer les PNJ alliés (mercenaires, PNJ de quête)",
            )
            .on_hover_text(
                "Les noms de joueurs EQ2 sont en un seul mot : tout allié au nom \
                 à plusieurs mots est un PNJ. Décoche si tu veux voir les mercenaires.",
            )
            .changed()
        {
            self.config.save();
        }

    }

    fn settings_history(&mut self, ui: &mut egui::Ui) {
        let mut hist_changed = false;
        hist_changed |= ui
            .checkbox(
                &mut self.config.persist_history,
                "Sauvegarder l'historique sur disque (rechargé au lancement)",
            )
            .changed();
        hist_changed |= ui
            .add(
                egui::Slider::new(&mut self.config.history_cap, 50..=2000)
                    .text("Encounters conservés max"),
            )
            .changed();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} encounters en mémoire — fichier : history\\{}_{}.json",
                    self.engine.history.len(),
                    self.current_server,
                    self.self_name().unwrap_or("?")
                ))
                .weak()
                .small(),
            );
            if ui.small_button("💾 Sauvegarder maintenant").clicked() {
                self.save_history();
            }
            if ui.small_button("🗑 Vider l'historique").clicked() {
                self.engine.history.clear();
                self.save_history();
                self.selected_encounter = None;
                self.compare_pin = None;
            }
        });
        if hist_changed {
            self.config.save();
        }

    }

    fn settings_pets(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(
                &mut self.config.merge_pets,
                "Fusionner les pets dans leur propriétaire (tables, overlay, exports, graphe)",
            )
            .changed()
        {
            self.config.save();
        }
        let manual: Vec<(String, String)> = self
            .config
            .pet_assignments
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !manual.is_empty() {
            ui.label("Assignations manuelles (clic droit sur un combattant pour en ajouter) :");
            for (pet, owner) in manual {
                ui.horizontal(|ui| {
                    ui.label(format!("🐾 {pet} → {owner}"));
                    if ui.small_button("✖").clicked() {
                        self.config.pet_assignments.remove(&pet);
                        self.config.save();
                    }
                });
            }
        }
        if !self.engine.auto_pets.is_empty() {
            ui.label(RichText::new(format!(
                "Auto-détectés : {}",
                self.engine
                    .auto_pets
                    .iter()
                    .map(|(p, o)| format!("{p} → {o}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .weak());
            if ui.small_button("Oublier les auto-détections").clicked() {
                self.engine.auto_pets.clear();
            }
        }

    }

    fn settings_overlay(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("💡 Clic droit sur l'overlay lui-même pour ces réglages en jeu.")
                .weak()
                .small(),
        );
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.config.overlay_opacity, 0.1..=1.0)
                        .text("Transparence"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.config.overlay_scale, 0.6..=2.0)
                        .text("Taille du texte"),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.config.overlay_rows, 1..=15)
                        .text("Joueurs max"),
                )
                .changed();
            ui.label(
                RichText::new("(taille : grip ↘ en bas à droite de l'overlay)")
                    .weak()
                    .small(),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Couleur de fond :");
            changed |= ui.color_edit_button_srgb(&mut self.config.overlay_bg).changed();
            ui.label("Accent (toi, texte custom) :");
            changed |= ui
                .color_edit_button_srgb(&mut self.config.overlay_accent)
                .changed();
        });
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut self.config.overlay_title_stats, "Titre détaillé")
                .on_hover_text("Durée + total dégâts + DPS raid + kills dans la barre de titre")
                .changed();
            changed |= ui.checkbox(&mut self.config.overlay_show_dps, "DPS").changed();
            changed |= ui.checkbox(&mut self.config.overlay_show_hps, "HPS").changed();
            changed |= ui
                .checkbox(&mut self.config.overlay_show_power, "Power")
                .changed();
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Texte custom (variables)").strong());
        ui.label(
            RichText::new(
                "Syntaxe : {{dps}} = toi · {{dps:Nom}} = un joueur · {{dps:1}} = rang 1. \
                 Multi-lignes accepté.",
            )
            .weak()
            .small(),
        );
        ui.horizontal_top(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut self.config.overlay_custom_text)
                        .hint_text("ex : hps {{hps}} — je tape {{dps}} ({{crit}} crit)\ntop : {{name:1}} à {{dps:1}}")
                        .desired_rows(2)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            ui.menu_button("➕ Variable", |ui| {
                ui.set_min_width(320.0);
                for (var, desc) in crate::template::VARIABLES {
                    if ui.button(format!("{var}  —  {desc}")).clicked() {
                        self.config.overlay_custom_text.push_str(var);
                        changed = true;
                        ui.close_menu();
                    }
                }
            });
        });
        changed |= ui
            .checkbox(
                &mut self.config.overlay_text_top,
                "Afficher le texte en haut (sous le titre) plutôt qu'en bas",
            )
            .changed();
        ui.horizontal(|ui| {
            ui.label("Format des barres :");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.config.overlay_bar_format)
                        .hint_text("vide = auto « 4691 (93.8k · 52.8%) » · ex : {{dps}} · {{pct}}")
                        .desired_width(300.0)
                        .font(egui::TextStyle::Monospace),
                )
                .on_hover_text(
                    "Texte de droite de chaque barre — variables résolues sur le joueur \
                     de la barre ({{dps}}, {{dmg}}, {{pct}}, {{crit}}, {{maxhit}}…).",
                )
                .changed();
            ui.label("Format du titre :");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.config.overlay_title_format)
                        .hint_text("vide = auto · ex : {{target}} — {{time}} | raid {{raiddps}}")
                        .desired_width(300.0)
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
        });
        // Aperçu live sur l'encounter affiché.
        if !self.config.overlay_custom_text.trim().is_empty() {
            let enc = self
                .engine
                .display_encounter()
                .cloned()
                .map(|e| self.for_display(&e));
            let preview = crate::template::render(
                &self.config.overlay_custom_text,
                enc.as_ref(),
                self.self_name(),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("Aperçu :").weak());
                ui.label(
                    RichText::new(preview)
                        .italics()
                        .color(Color32::from_rgb(241, 196, 15)),
                );
            });
        }
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut self.config.overlay_locked, "🔒 Verrouiller position/taille")
                .changed();
            changed |= ui
                .checkbox(&mut self.config.overlay_fade_hover, "👻 Fondu au survol")
                .changed();
        });
        if ui
            .checkbox(
                &mut self.config.overlay_click_through,
                "Click-through (l'overlay laisse passer les clics — réglages uniquement ici quand actif)",
            )
            .changed()
        {
            changed = true;
            self.passthrough_sent = false; // force le renvoi de la commande
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Profils d'overlay").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.profile_name)
                    .hint_text("nom du profil (ex : raid, solo)…")
                    .desired_width(160.0),
            );
            if ui.button("💾 Enregistrer les réglages actuels").clicked()
                && !self.profile_name.trim().is_empty()
            {
                let name = self.profile_name.trim().to_string();
                let profile = self.config.capture_profile(&name);
                // Remplace un profil de même nom, sinon ajoute.
                if let Some(existing) = self
                    .config
                    .overlay_profiles
                    .iter_mut()
                    .find(|p| p.name == name)
                {
                    *existing = profile;
                } else {
                    self.config.overlay_profiles.push(profile);
                }
                self.profile_name.clear();
                changed = true;
            }
        });
        let mut to_apply: Option<usize> = None;
        let mut to_delete: Option<usize> = None;
        for (i, p) in self.config.overlay_profiles.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("• {}", p.name));
                if ui.small_button("Appliquer").clicked() {
                    to_apply = Some(i);
                }
                if ui.small_button("🗑").clicked() {
                    to_delete = Some(i);
                }
            });
        }
        if let Some(i) = to_apply {
            let p = self.config.overlay_profiles[i].clone();
            self.config.apply_profile(&p);
            self.passthrough_sent = false;
            changed = true;
        }
        if let Some(i) = to_delete {
            self.config.overlay_profiles.remove(i);
            changed = true;
        }

        if changed {
            self.config.save();
        }

    }

    fn settings_updates(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!(
                "Version actuelle : v{}",
                crate::update::CURRENT_VERSION
            ));
            if ui.button("🔄 Vérifier maintenant").clicked() {
                crate::update::spawn_check(self.update_tx.clone());
            }
            if ui.button("📋 Nouveautés").clicked() {
                self.show_changelog = true;
            }
            match (&self.update_available, &self.update_error) {
                (Some(info), _) => {
                    ui.label(
                        RichText::new(format!("✨ {} disponible (bouton en haut à droite)", info.version))
                            .color(Color32::from_rgb(46, 204, 113)),
                    );
                }
                (None, Some(err)) => {
                    ui.label(RichText::new(format!("⚠ {err}")).color(Color32::from_rgb(231, 76, 60)));
                }
                _ => {
                    ui.label(RichText::new("— à jour (ou vérification en cours)").weak());
                }
            }
        });
        ui.label(
            RichText::new(format!(
                "Les releases sont publiées sur github.com/{} — pousser un tag vX.Y.Z déclenche le build.",
                crate::update::REPO
            ))
            .weak()
            .small(),
        );
    }
}

// ---------------------------------------------------------------------------
// Tables partagées, export, graphe
// ---------------------------------------------------------------------------

impl App {
    fn export_toolbar(&mut self, ui: &mut egui::Ui, enc: &Encounter) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("💾 JSON")
                .on_hover_text("Exporter l'encounter complet (séries incluses)")
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name(format!("{}.json", safe_filename(&enc.title())))
                    .add_filter("JSON", &["json"])
                    .save_file()
                {
                    let _ = std::fs::write(p, export::json(enc));
                }
            }
            if ui.button("💾 CSV").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name(format!("{}.csv", safe_filename(&enc.title())))
                    .add_filter("CSV", &["csv"])
                    .save_file()
                {
                    let _ = std::fs::write(p, export::csv(enc));
                }
            }
            if ui
                .button("📋 Markdown")
                .on_hover_text("Copie un tableau Markdown dans le presse-papiers")
                .clicked()
            {
                ui.ctx().copy_text(export::markdown(enc));
                self.copied_at = Some(Instant::now());
            }
            if ui
                .button("📋 Chat")
                .on_hover_text("Copie une ligne compacte à coller dans le chat du jeu")
                .clicked()
            {
                ui.ctx().copy_text(export::chat_line(enc));
                self.copied_at = Some(Instant::now());
            }
            if let Some(t) = self.copied_at {
                if t.elapsed() < Duration::from_secs(2) {
                    ui.label(
                        RichText::new("✓ copié").color(Color32::from_rgb(46, 204, 113)),
                    );
                } else {
                    self.copied_at = None;
                }
            }
        });
    }

    /// Menu contextuel d'un combattant : assignation pet → propriétaire.
    fn pet_context_menu(&mut self, resp: &egui::Response, enc: &Encounter, name: &str) {
        resp.context_menu(|ui| {
            let assigned = self.config.pet_assignments.get(name).cloned();
            if let Some(owner) = &assigned {
                ui.label(format!("🐾 pet de {owner}"));
                if ui.button("❌ Retirer l'assignation").clicked() {
                    self.config.pet_assignments.remove(name);
                    self.config.save();
                    ui.close_menu();
                }
                ui.separator();
            }
            ui.menu_button("🐾 Assigner comme pet de…", |ui| {
                // Candidats propriétaires : noms en un seul mot (les PJ EQ2),
                // différents du combattant cliqué.
                let mut candidates: Vec<&String> = enc
                    .combatants
                    .keys()
                    .filter(|n| n.as_str() != name && !n.contains(' '))
                    .collect();
                candidates.sort();
                let self_name = self.self_name().map(|s| s.to_string());
                if let Some(sn) = &self_name {
                    if sn != name && ui.button(format!("⭐ {sn} (moi)")).clicked() {
                        self.config.pet_assignments.insert(name.into(), sn.clone());
                        self.config.save();
                        ui.close_menu();
                    }
                }
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for cand in candidates {
                        if Some(cand.as_str()) == self_name.as_deref() {
                            continue;
                        }
                        if ui.button(cand.as_str()).clicked() {
                            self.config
                                .pet_assignments
                                .insert(name.into(), cand.clone());
                            self.config.save();
                            ui.close_menu();
                        }
                    }
                });
            });
        });
    }

    fn encounter_table(&mut self, ui: &mut egui::Ui, enc: &Encounter) {
        use egui_extras::{Column, TableBuilder};
        let self_name = self.self_name().map(|s| s.to_string());

        let filter = self.filter_combatant.clone();
        filter_box(ui, &mut self.filter_combatant, "filtrer les combattants…");

        let mut ranking: Vec<(String, crate::combat::Combatant)> = enc
            .damage_ranking()
            .into_iter()
            .filter(|(n, _)| matches_filter(n, &filter))
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        let mut heals: Vec<(String, crate::combat::Combatant)> = enc
            .heal_ranking()
            .into_iter()
            .filter(|(n, _)| matches_filter(n, &filter))
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();

        let mut st_dmg = *self.sort_states.entry("dmg").or_insert((1, true));
        sort_rows(
            &mut ranking,
            st_dmg,
            |r| r.0.clone(),
            |r, col| match col {
                1 | 2 | 3 => r.1.damage as f64,
                4 => r.1.crit_rate(),
                5 => r.1.max_hit as f64,
                6 => r.1.hits as f64,
                _ => 0.0,
            },
        );

        ui.label(RichText::new("Dégâts").strong());
        TableBuilder::new(ui)
            .id_salt("dmg_table")
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder())
            .header(20.0, |mut h| {
                sortable_headers(
                    &mut h,
                    &["Nom", "Dégâts", "DPS", "%", "Crit %", "Max hit", "Hits"],
                    &mut st_dmg,
                );
            })
            .body(|mut body| {
                let total = enc.total_damage().max(1);
                let max_dmg = ranking.iter().map(|r| r.1.damage).max().unwrap_or(1).max(1);
                for (name, c) in &ranking {
                    let is_self = self_name.as_deref() == Some(name.as_str());
                    let has_pets = c.abilities.keys().any(|k| k.starts_with("🐾"));
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            let display = if has_pets {
                                format!("{name} 🐾")
                            } else {
                                name.clone()
                            };
                            let txt = if is_self {
                                RichText::new(display).color(Color32::from_rgb(241, 196, 15))
                            } else {
                                RichText::new(display)
                            };
                            let resp = ui.selectable_label(
                                self.selected_combatant.as_deref() == Some(name.as_str()),
                                txt,
                            );
                            if resp.clicked() {
                                self.selected_combatant = Some(name.clone());
                            }
                            self.pet_context_menu(&resp, enc, name);
                        });
                        row.col(|ui| {
                            gauge_cell(
                                ui,
                                &fmt_num(c.damage),
                                c.damage as f32 / max_dmg as f32,
                                player_color(name),
                            );
                        });
                        row.col(|ui| {
                            mono(ui, fmt_f64(enc.dps_of(c)));
                        });
                        row.col(|ui| {
                            mono(ui, format!("{:.1}", c.damage as f64 / total as f64 * 100.0));
                        });
                        row.col(|ui| {
                            mono(ui, format!("{:.1}", c.crit_rate()));
                        });
                        row.col(|ui| {
                            mono(ui, fmt_num(c.max_hit));
                        });
                        row.col(|ui| {
                            mono(ui, format!("{}", c.hits));
                        });
                    });
                }
            });

        self.sort_states.insert("dmg", st_dmg);

        let mut st_heal = *self.sort_states.entry("heal").or_insert((1, true));
        sort_rows(
            &mut heals,
            st_heal,
            |r| r.0.clone(),
            |r, _| r.1.healing as f64,
        );
        if !heals.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Soins (heals + wards)").strong());
            TableBuilder::new(ui)
                .id_salt("heal_table")
                .striped(true)
                .vscroll(false)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder())
                .header(20.0, |mut h| {
                    sortable_headers(&mut h, &["Nom", "Soins", "HPS", ""], &mut st_heal);
                })
                .body(|mut body| {
                    let max_heal =
                        heals.iter().map(|r| r.1.healing).max().unwrap_or(1).max(1);
                    for (name, c) in &heals {
                        body.row(18.0, |mut row| {
                            row.col(|ui| {
                                let resp = ui.selectable_label(
                                    self.selected_combatant.as_deref()
                                        == Some(name.as_str()),
                                    name.as_str(),
                                );
                                if resp.clicked() {
                                    self.selected_combatant = Some(name.clone());
                                }
                                self.pet_context_menu(&resp, enc, name);
                            });
                            row.col(|ui| {
                                gauge_cell(
                                    ui,
                                    &fmt_num(c.healing),
                                    c.healing as f32 / max_heal as f32,
                                    player_color(name),
                                );
                            });
                            row.col(|ui| {
                                mono(ui, fmt_f64(enc.hps_of(c)));
                            });
                            row.col(|_| {});
                        });
                    }
                });
        }

        self.sort_states.insert("heal", st_heal);

        let mut power: Vec<(String, crate::combat::Combatant)> = enc
            .power_ranking()
            .into_iter()
            .filter(|(n, _)| matches_filter(n, &filter))
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        let mut st_power = *self.sort_states.entry("power").or_insert((1, true));
        sort_rows(
            &mut power,
            st_power,
            |r| r.0.clone(),
            |r, _| r.1.power as f64,
        );
        if !power.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Power replenish").strong());
            TableBuilder::new(ui)
                .id_salt("power_table")
                .striped(true)
                .vscroll(false)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder())
                .header(20.0, |mut h| {
                    sortable_headers(&mut h, &["Nom", "Power", "Power/s", ""], &mut st_power);
                })
                .body(|mut body| {
                    for (name, c) in &power {
                        body.row(18.0, |mut row| {
                            row.col(|ui| {
                                let resp = ui.selectable_label(
                                    self.selected_combatant.as_deref()
                                        == Some(name.as_str()),
                                    name.as_str(),
                                );
                                if resp.clicked() {
                                    self.selected_combatant = Some(name.clone());
                                }
                                self.pet_context_menu(&resp, enc, name);
                            });
                            row.col(|ui| {
                                ui.label(fmt_num(c.power));
                            });
                            row.col(|ui| {
                                ui.label(fmt_f64(enc.pps_of(c)));
                            });
                            row.col(|_| {});
                        });
                    }
                });
        }
        self.sort_states.insert("power", st_power);
    }
}

/// En-têtes de table cliquables : clic = trier, re-clic = inverser.
fn sortable_headers(
    h: &mut egui_extras::TableRow<'_, '_>,
    labels: &[&str],
    st: &mut (usize, bool),
) {
    for (i, t) in labels.iter().enumerate() {
        h.col(|ui| {
            if t.is_empty() {
                return;
            }
            let active = st.0 == i;
            let arrow = if active {
                if st.1 { " ⏷" } else { " ⏶" }
            } else {
                ""
            };
            if ui
                .selectable_label(active, RichText::new(format!("{t}{arrow}")).strong())
                .clicked()
            {
                if active {
                    st.1 = !st.1;
                } else {
                    // Numérique : descendant par défaut ; nom : ascendant.
                    *st = (i, i != 0);
                }
            }
        });
    }
}

/// Trie des lignes (nom, valeur de tri par colonne) selon l'état de tri.
fn sort_rows<T>(rows: &mut [T], st: (usize, bool), name: impl Fn(&T) -> String, key: impl Fn(&T, usize) -> f64) {
    if st.0 == 0 {
        rows.sort_by(|a, b| name(a).to_lowercase().cmp(&name(b).to_lowercase()));
    } else {
        rows.sort_by(|a, b| {
            key(a, st.0)
                .partial_cmp(&key(b, st.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    if st.1 {
        rows.reverse();
    }
}

/// Champ de filtre texte compact avec bouton d'effacement.
fn filter_box(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.label("🔎");
        ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(hint)
                .desired_width(180.0),
        );
        if !text.is_empty() && ui.small_button("✖").clicked() {
            text.clear();
        }
    });
}

fn matches_filter(name: &str, filter: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
}

/// Section de réglages repliable avec titre stylé.
fn section(ui: &mut egui::Ui, title: &str, open: bool, content: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).size(17.0).strong())
        .default_open(open)
        .show(ui, content);
    ui.add_space(6.0);
}

/// Combo de sélection du type de mécanique. Retourne `true` si modifié.
fn kind_combo(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut MechKind) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("kind", id))
        .selected_text(format!("{} {}", value.icon(), value.label()))
        .show_ui(ui, |ui| {
            for k in [
                MechKind::Aoe,
                MechKind::TankBuster,
                MechKind::Lethal,
                MechKind::Burst,
                MechKind::Other,
            ] {
                if ui
                    .selectable_label(*value == k, format!("{} {}", k.icon(), k.label()))
                    .clicked()
                {
                    *value = k;
                    changed = true;
                }
            }
        });
    changed
}

/// Combo de sélection du mode d'alerte. `include_inherit` ajoute « Par défaut ».
fn alert_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    value: &mut AlertMode,
    include_inherit: bool,
) -> bool {
    let mut changed = false;
    let opts: &[AlertMode] = if include_inherit {
        &[AlertMode::Inherit, AlertMode::Visual, AlertMode::Sound, AlertMode::Tts]
    } else {
        &[AlertMode::Visual, AlertMode::Sound, AlertMode::Tts]
    };
    egui::ComboBox::from_id_salt(("alert", id))
        .selected_text(value.label())
        .show_ui(ui, |ui| {
            for m in opts {
                if ui.selectable_label(*value == *m, m.label()).clicked() {
                    *value = *m;
                    changed = true;
                }
            }
        });
    changed
}

/// Cellule-jauge : barre de progression colorée derrière la valeur (à la ACT).
fn gauge_cell(ui: &mut egui::Ui, text: &str, frac: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(80.0), 16.0),
        egui::Sense::hover(),
    );
    let text_color = ui.visuals().strong_text_color();
    let track = ui.visuals().faint_bg_color;
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, track);
    let fill = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * frac.clamp(0.0, 1.0), rect.height()),
    );
    painter.rect_filled(fill, 2.0, color.gamma_multiply(0.45));
    painter.text(
        rect.right_center() - egui::vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        text,
        egui::FontId::monospace(12.0),
        text_color,
    );
}

fn mono(ui: &mut egui::Ui, text: String) {
    ui.label(RichText::new(text).monospace());
}

fn safe_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

// ---------------------------------------------------------------------------
// Graphe temporel
// ---------------------------------------------------------------------------

/// Valeurs lissées (ou moyenne cumulée) d'une série, échantillonnées chaque seconde.
fn sample_series(
    series: &std::collections::BTreeMap<u64, u64>,
    start: u64,
    dur: u64,
    smooth: u64,
    cumulative: bool,
) -> Vec<f64> {
    let mut out = Vec::with_capacity(dur as usize + 1);
    let mut cumul: u64 = 0;
    for t in 0..=dur {
        let epoch = start + t;
        let y = if cumulative {
            cumul += series.get(&epoch).copied().unwrap_or(0);
            cumul as f64 / (t + 1) as f64
        } else {
            let lo = epoch.saturating_sub(smooth - 1);
            let sum: u64 = series.range(lo..=epoch).map(|(_, v)| v).sum();
            sum as f64 / smooth as f64
        };
        out.push(y);
    }
    out
}

fn metric_series<'a>(
    c: &'a crate::combat::Combatant,
    metric: Metric,
) -> &'a std::collections::BTreeMap<u64, u64> {
    match metric {
        Metric::Dps => &c.dmg_series,
        Metric::Hps => &c.heal_series,
        Metric::Power => &c.power_series,
        Metric::Taken => &c.taken_series,
    }
}

fn metric_total(c: &crate::combat::Combatant, metric: Metric) -> u64 {
    match metric {
        Metric::Dps => c.damage,
        Metric::Hps => c.healing,
        Metric::Power => c.power,
        Metric::Taken => c.damage_taken,
    }
}

fn graph_section(
    ui: &mut egui::Ui,
    enc: &Encounter,
    pinned: Option<&Encounter>,
    selected_combatant: Option<&str>,
    state: &mut GraphState,
) {
    use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, Polygon};

    // Contrôles
    ui.horizontal(|ui| {
        for m in [Metric::Dps, Metric::Hps, Metric::Power, Metric::Taken] {
            ui.selectable_value(&mut state.metric, m, m.label());
        }
        ui.separator();
        ui.selectable_value(&mut state.mode, GraphMode::PerPlayer, "Par joueur");
        ui.selectable_value(&mut state.mode, GraphMode::PerAbility, "Par sort");
        ui.separator();
        if ui
            .button("📷 PNG")
            .on_hover_text("Exporter le graphe en image PNG")
            .clicked()
        {
            state.want_png = true;
        }
    });
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(&mut state.smooth, 1..=15).text("Lissage (s)"));
        ui.checkbox(&mut state.cumulative, "Cumulé (moyenne depuis le début)");
        if pinned.is_some() {
            ui.checkbox(&mut state.overlay_pinned, "⚖ Superposer l'épinglé (pointillés)");
        }
    });

    let start = enc.start;
    let dur = enc.duration();
    let smooth = state.smooth.max(1);

    let mut lines: Vec<(String, Color32, LineStyle, Vec<[f64; 2]>)> = Vec::new();
    let mut polygons: Vec<(String, Color32, Vec<[f64; 2]>)> = Vec::new();

    match state.mode {
        GraphMode::PerPlayer => {
            // Candidats triés selon la métrique
            let mut candidates: Vec<(&String, &crate::combat::Combatant)> = enc
                .combatants
                .iter()
                .filter(|(n, c)| metric_total(c, state.metric) > 0 && enc.visible(n))
                .collect();
            candidates.sort_by_key(|(_, c)| std::cmp::Reverse(metric_total(c, state.metric)));

            if candidates.is_empty() {
                ui.label(RichText::new("(aucune donnée pour cette métrique)").weak());
                return;
            }

            // Sélection par défaut : top 5 si rien de pertinent n'est coché.
            let names: Vec<String> = candidates.iter().map(|(n, _)| (*n).clone()).collect();
            if !state.selected.iter().any(|s| names.contains(s)) {
                state.selected = names.iter().take(5).cloned().collect();
            }

            // Filtres joueurs
            ui.horizontal_wrapped(|ui| {
                for name in names.iter().take(14) {
                    let mut on = state.selected.contains(name);
                    if ui.checkbox(&mut on, name.as_str()).changed() {
                        if on {
                            state.selected.insert(name.clone());
                        } else {
                            state.selected.remove(name);
                        }
                    }
                }
            });

            for (name, c) in candidates.iter() {
                if !state.selected.contains(name.as_str()) {
                    continue;
                }
                let ys = sample_series(
                    metric_series(c, state.metric),
                    start,
                    dur,
                    smooth,
                    state.cumulative,
                );
                let pts = ys.iter().enumerate().map(|(t, y)| [t as f64, *y]).collect();
                lines.push((
                    (*name).clone(),
                    player_color(name),
                    LineStyle::Solid,
                    pts,
                ));
            }

            // Superposition de l'encounter épinglé (aligné sur t=0, pointillés).
            if state.overlay_pinned {
                if let Some(p) = pinned {
                    let pdur = p.duration();
                    let mut pcand: Vec<(&String, &crate::combat::Combatant)> = p
                        .combatants
                        .iter()
                        .filter(|(n, c)| metric_total(c, state.metric) > 0 && p.visible(n))
                        .collect();
                    pcand.sort_by_key(|(_, c)| {
                        std::cmp::Reverse(metric_total(c, state.metric))
                    });
                    for (name, c) in pcand.iter() {
                        if !state.selected.contains(name.as_str()) {
                            continue;
                        }
                        let ys = sample_series(
                            metric_series(c, state.metric),
                            p.start,
                            pdur,
                            smooth,
                            state.cumulative,
                        );
                        let pts =
                            ys.iter().enumerate().map(|(t, y)| [t as f64, *y]).collect();
                        lines.push((
                            format!("{name} (épinglé)"),
                            player_color(name).gamma_multiply(0.6),
                            LineStyle::dashed_dense(),
                            pts,
                        ));
                    }
                }
            }
        }
        GraphMode::PerAbility => {
            let Some(sel) = selected_combatant else {
                ui.label(
                    RichText::new(
                        "Sélectionne un combattant dans la table pour voir ses sorts empilés.",
                    )
                    .weak(),
                );
                return;
            };
            let Some(c) = enc.combatants.get(sel) else {
                ui.label(RichText::new(format!("« {sel} » absent de cet encounter.")).weak());
                return;
            };
            ui.label(
                RichText::new(format!("Sorts de {sel} (aires empilées, top 8)")).weak(),
            );
            let mut abs: Vec<(&String, &crate::combat::AbilityStats)> =
                c.abilities.iter().collect();
            abs.sort_by_key(|(_, a)| std::cmp::Reverse(a.damage + a.healing + a.power));
            abs.truncate(8);

            // Empilement : le plus gros sort en bas.
            let mut lower = vec![0.0f64; dur as usize + 1];
            for (i, (ab_name, ab)) in abs.iter().enumerate() {
                let ys = sample_series(&ab.series, start, dur, smooth, state.cumulative);
                let upper: Vec<f64> =
                    lower.iter().zip(&ys).map(|(l, y)| l + y).collect();
                // Polygone = frontière haute (gauche→droite) + basse (droite→gauche).
                let mut pts: Vec<[f64; 2]> = upper
                    .iter()
                    .enumerate()
                    .map(|(t, y)| [t as f64, *y])
                    .collect();
                pts.extend(
                    lower
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(t, y)| [t as f64, *y]),
                );
                polygons.push((
                    (*ab_name).clone(),
                    BAR_COLORS[i % BAR_COLORS.len()],
                    pts,
                ));
                lower = upper;
            }
        }
    }

    let resp = Plot::new("combat_graph")
        .legend(Legend::default())
        .height(280.0)
        .allow_drag(false)
        .allow_scroll(false)
        .x_axis_formatter(|mark, _range| fmt_duration(mark.value.max(0.0) as u64))
        .y_axis_formatter(|mark, _range| fmt_f64(mark.value.max(0.0)))
        .label_formatter(|name, value| {
            format!(
                "{name}\n{} — {}",
                fmt_duration(value.x.max(0.0) as u64),
                fmt_f64(value.y.max(0.0))
            )
        })
        .show(ui, |plot_ui| {
            for (name, color, pts) in polygons {
                plot_ui.polygon(
                    Polygon::new(name, PlotPoints::from(pts))
                        .fill_color(color.gamma_multiply(0.5))
                        .stroke(egui::Stroke::new(1.0, color)),
                );
            }
            for (name, color, style, pts) in lines {
                plot_ui.line(
                    Line::new(name, PlotPoints::from(pts))
                        .color(color)
                        .style(style)
                        .width(1.8),
                );
            }
            // Marqueurs de morts (croix rouges sur la ligne de base).
            let death_pts: Vec<[f64; 2]> = enc
                .deaths_log
                .iter()
                .filter(|d| enc.visible(&d.victim))
                .map(|d| [d.epoch.saturating_sub(enc.start) as f64, 0.0])
                .collect();
            if !death_pts.is_empty() {
                plot_ui.points(
                    egui_plot::Points::new("💀 morts", PlotPoints::from(death_pts))
                        .shape(egui_plot::MarkerShape::Cross)
                        .radius(7.0)
                        .color(Color32::from_rgb(231, 76, 60)),
                );
            }
        });
    state.last_plot_rect = Some(resp.response.rect);
}

/// Rapport de mort : qui, quand, par qui, avec les derniers coups encaissés.
fn death_report(ui: &mut egui::Ui, enc: &Encounter, d: &crate::combat::DeathRecord, idx: usize) {
    use egui_extras::{Column, TableBuilder};
    let offset = d.epoch.saturating_sub(enc.start);
    let total: u64 = d.hits.iter().map(|h| h.amount).sum();
    egui::CollapsingHeader::new(format!(
        "💀 {} — t+{} — tué par {} ({} encaissés en {} s)",
        d.victim,
        fmt_duration(offset),
        d.killer,
        fmt_num(total),
        12
    ))
    .id_salt(("death", idx))
    .default_open(deaths_default_open(enc))
    .show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt(("death_table", idx))
            .striped(true)
            .vscroll(false)
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(200.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder())
            .header(18.0, |mut h| {
                for t in ["t", "Attaquant", "Sort / CA", "Dégâts", ""] {
                    h.col(|ui| {
                        ui.label(RichText::new(t).strong());
                    });
                }
            })
            .body(|mut body| {
                for hit in &d.hits {
                    body.row(17.0, |mut row| {
                        row.col(|ui| {
                            let dt = d.epoch.saturating_sub(hit.epoch);
                            ui.label(if dt == 0 {
                                "mort".to_string()
                            } else {
                                format!("-{dt} s")
                            });
                        });
                        row.col(|ui| {
                            ui.label(&hit.attacker);
                        });
                        row.col(|ui| {
                            ui.label(hit.ability.as_deref().unwrap_or("(auto-attack)"));
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(fmt_num(hit.amount))
                                    .color(Color32::from_rgb(231, 76, 60)),
                            );
                        });
                        row.col(|_| {});
                    });
                }
            });
    });
}

/// Ouvre le détail par défaut quand il n'y a qu'une ou deux morts.
fn deaths_default_open(enc: &Encounter) -> bool {
    enc.deaths_log.len() <= 2
}

/// Table de comparaison : encounter épinglé (A) vs affiché (B).
fn comparison_table(
    ui: &mut egui::Ui,
    pinned: &Encounter,
    current: &Encounter,
    st: &mut (usize, bool),
) {
    use egui_extras::{Column, TableBuilder};

    ui.label(
        RichText::new(format!(
            "A = épinglé ({}, {}) · B = affiché ({}, {})",
            pinned.title(),
            fmt_duration(pinned.duration()),
            current.title(),
            fmt_duration(current.duration())
        ))
        .weak()
        .small(),
    );

    // Union des combattants : (nom, dps A, dps B).
    let mut names: Vec<String> = current
        .damage_ranking()
        .iter()
        .map(|(n, _)| (*n).clone())
        .collect();
    for (n, _) in pinned.damage_ranking() {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names.truncate(15);
    let mut rows: Vec<(String, Option<f64>, Option<f64>)> = names
        .into_iter()
        .map(|name| {
            let a = pinned.combatants.get(&name).map(|c| pinned.dps_of(c));
            let b = current.combatants.get(&name).map(|c| current.dps_of(c));
            (name, a, b)
        })
        .collect();
    sort_rows(
        &mut rows,
        *st,
        |r| r.0.clone(),
        |r, col| match col {
            1 => r.1.unwrap_or(-1.0),
            2 => r.2.unwrap_or(-1.0),
            3 => match (r.1, r.2) {
                (Some(a), Some(b)) if a > 0.0 => (b - a) / a,
                _ => f64::NEG_INFINITY,
            },
            _ => 0.0,
        },
    );

    TableBuilder::new(ui)
        .id_salt("cmp_table")
        .striped(true)
        .vscroll(false)
        .column(Column::auto().at_least(160.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(80.0))
        .column(Column::remainder())
        .header(20.0, |mut h| {
            sortable_headers(
                &mut h,
                &["Nom", "DPS A (épinglé)", "DPS B (affiché)", "Δ %", ""],
                st,
            );
        })
        .body(|mut body| {
            for (name, a, b) in &rows {
                let (a, b) = (*a, *b);
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label(name);
                    });
                    row.col(|ui| {
                        ui.label(a.map_or("—".into(), fmt_f64));
                    });
                    row.col(|ui| {
                        ui.label(b.map_or("—".into(), fmt_f64));
                    });
                    row.col(|ui| match (a, b) {
                        (Some(a), Some(b)) if a > 0.0 => {
                            let delta = (b - a) / a * 100.0;
                            let color = if delta >= 0.0 {
                                Color32::from_rgb(46, 204, 113)
                            } else {
                                Color32::from_rgb(231, 76, 60)
                            };
                            ui.label(
                                RichText::new(format!("{delta:+.1} %")).color(color),
                            );
                        }
                        _ => {
                            ui.label("—");
                        }
                    });
                    row.col(|_| {});
                });
            }
        });
}

impl App {
    fn ability_breakdown(&mut self, ui: &mut egui::Ui, enc: &Encounter, name: &str) {
    use egui_extras::{Column, TableBuilder};
    let Some(c) = enc.combatants.get(name) else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Breakdown — {name}")).strong());
        filter_box(ui, &mut self.filter_ability, "filtrer les sorts…");
    });
    let filter = self.filter_ability.clone();
    let mut abilities: Vec<_> = c
        .abilities
        .iter()
        .filter(|(n, _)| matches_filter(n, &filter))
        .collect();
    let mut st = *self.sort_states.entry("ability").or_insert((4, true));
    sort_rows(
        &mut abilities,
        st,
        |r| r.0.clone(),
        |r, col| match col {
            1 => r.1.damage as f64,
            2 => r.1.healing as f64,
            3 => r.1.power as f64,
            4 => (r.1.damage + r.1.healing + r.1.power) as f64,
            5 => r.1.hits as f64,
            6 => {
                if r.1.hits == 0 {
                    0.0
                } else {
                    r.1.crits as f64 / r.1.hits as f64
                }
            }
            7 => r.1.max_hit as f64,
            _ => 0.0,
        },
    );
    let total = (c.damage + c.healing + c.power).max(1);

    TableBuilder::new(ui)
        .id_salt("ability_table")
        .striped(true)
        .vscroll(false)
        .column(Column::auto().at_least(220.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(80.0))
        .column(Column::remainder())
        .header(20.0, |mut h| {
            sortable_headers(
                &mut h,
                &["Sort / CA", "Dégâts", "Soins", "Power", "%", "Hits", "Crit %", "Max"],
                &mut st,
            );
        })
        .body(|mut body| {
            for (ab_name, ab) in abilities {
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label(ab_name);
                    });
                    row.col(|ui| {
                        ui.label(fmt_num(ab.damage));
                    });
                    row.col(|ui| {
                        ui.label(fmt_num(ab.healing));
                    });
                    row.col(|ui| {
                        ui.label(fmt_num(ab.power));
                    });
                    row.col(|ui| {
                        ui.label(format!(
                            "{:.1}",
                            (ab.damage + ab.healing + ab.power) as f64 / total as f64
                                * 100.0
                        ));
                    });
                    row.col(|ui| {
                        ui.label(format!("{}", ab.hits));
                    });
                    row.col(|ui| {
                        let rate = if ab.hits == 0 {
                            0.0
                        } else {
                            ab.crits as f64 / ab.hits as f64 * 100.0
                        };
                        ui.label(format!("{rate:.1}"));
                    });
                    row.col(|ui| {
                        ui.label(fmt_num(ab.max_hit));
                    });
                });
            }
        });
    self.sort_states.insert("ability", st);

    // Défense : attaques adverses évitées, par type.
    if !c.avoids_by_kind.is_empty() {
        let total: u32 = c.avoids_by_kind.values().sum();
        let detail = c
            .avoids_by_kind
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(" · ");
        ui.label(
            RichText::new(format!("🛡 Évitements : {detail}  (total {total})"))
                .color(Color32::from_rgb(46, 204, 113)),
        );
    }
    if !c.misses_by_kind.is_empty() {
        let total: u32 = c.misses_by_kind.values().sum();
        let detail = c
            .misses_by_kind
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let acc = if c.hits + total > 0 {
            c.hits as f64 / (c.hits + total) as f64 * 100.0
        } else {
            100.0
        };
        ui.label(
            RichText::new(format!(
                "⚔ Attaques évitées par l'adversaire : {detail}  (précision {acc:.1} %)"
            ))
            .weak(),
        );
    }
    if !c.resists_by_school.is_empty() {
        let total: u32 = c.resists_by_school.values().sum();
        let detail = c
            .resists_by_school
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(" · ");
        ui.label(
            RichText::new(format!("🔮 Sorts résistés par école : {detail}  (total {total})"))
                .color(Color32::from_rgb(155, 89, 182)),
        );
    }

    // Détail par cible et par type.
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        target_table(ui, "🎯 Dégâts par cible", &c.damage_by_target);
        target_table(ui, "🛡 Reçus par attaquant", &c.taken_by_attacker);
    });
    ui.horizontal_top(|ui| {
        target_table(ui, "💥 Dégâts par type", &c.damage_by_type);
        target_table(ui, "🩸 Reçus par type", &c.taken_by_type);
    });
    ui.horizontal_top(|ui| {
        target_table(ui, "💚 Soins par bénéficiaire", &c.heals_by_target);
        target_table(ui, "💙 Soins reçus de", &c.heals_received_from);
    });
    }
}

/// Mini-table « nom → montant (%) », triée décroissante, top 12.
fn target_table(ui: &mut egui::Ui, title: &str, map: &std::collections::BTreeMap<String, u64>) {
    if map.is_empty() {
        return;
    }
    let total: u64 = map.values().sum::<u64>().max(1);
    let mut rows: Vec<(&String, &u64)> = map.iter().collect();
    rows.sort_by_key(|(_, v)| std::cmp::Reverse(**v));
    ui.group(|ui| {
        ui.set_min_width(300.0);
        ui.label(RichText::new(title).strong());
        for (name, v) in rows.iter().take(12) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name.as_str()).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}  ({:.1} %)",
                            fmt_num(**v),
                            **v as f64 / total as f64 * 100.0
                        ))
                        .small(),
                    );
                });
            });
        }
        if rows.len() > 12 {
            ui.label(RichText::new(format!("… +{} autres", rows.len() - 12)).weak().small());
        }
    });
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

/// Une section de barres de l'overlay (DPS, HPS ou Power), pré-calculée.
struct OverlaySection {
    label: &'static str,
    /// (rang dans le classement, nom, texte de droite pré-rendu, fraction de barre, est_soi)
    rows: Vec<(usize, String, String, f32, bool)>,
}

impl App {
    /// Overlay dédié aux mécaniques de boss : fenêtre séparée, always-on-top,
    /// listant les prochains casts chronométrés en barres de compte à rebours.
    fn show_mech_overlay(&mut self, ctx: &egui::Context) {
        let id = egui::ViewportId::from_hash_of("eq2_mech_overlay");
        let now = Self::now_epoch();
        let bars: Vec<(String, f32, f64, bool)> = self
            .engine
            .mech
            .predictions(now)
            .into_iter()
            .filter(|p| p.eta <= 60.0)
            .take(8)
            .map(|p| {
                let frac = (p.eta / p.period.max(1.0)).clamp(0.0, 1.0) as f32;
                (
                    format!("{} {}", p.kind.icon(), p.ability),
                    frac,
                    p.eta.max(0.0),
                    p.eta <= p.lead as f64,
                )
            })
            .collect();

        let was_dragging = self.mech_dragging;
        let cfg = &mut self.config;
        let s = cfg.overlay_scale.clamp(0.6, 2.5);
        let bg = Color32::from_rgba_unmultiplied(
            cfg.overlay_bg[0],
            cfg.overlay_bg[1],
            cfg.overlay_bg[2],
            (cfg.overlay_opacity * 235.0) as u8,
        );
        let mut changed = false;
        let mut drag_now = false;

        let mut builder = egui::ViewportBuilder::default()
            .with_title("EQ2 Mécaniques")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(true)
            .with_min_inner_size([160.0, 50.0])
            .with_inner_size([cfg.mech_overlay_width, cfg.mech_overlay_height]);
        // Pendant le déplacement, l'OS pilote la position : ne pas la réimposer.
        if !was_dragging {
            if let Some((x, y)) = cfg.mech_overlay_pos {
                builder = builder.with_position([x, y]);
            }
        }

        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            let actual = ctx.input(|i| i.screen_rect().size());
            if (actual.x - cfg.mech_overlay_width).abs() > 1.0
                || (actual.y - cfg.mech_overlay_height).abs() > 1.0
            {
                cfg.mech_overlay_width = actual.x;
                cfg.mech_overlay_height = actual.y;
                changed = true;
            }
            if !was_dragging {
                if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                    let pos = (rect.min.x, rect.min.y);
                    if cfg
                        .mech_overlay_pos
                        .is_none_or(|p| (p.0 - pos.0).abs() > 1.0 || (p.1 - pos.1).abs() > 1.0)
                    {
                        cfg.mech_overlay_pos = Some(pos);
                        changed = true;
                    }
                }
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let frame = egui::Frame::new().fill(bg).corner_radius(8.0).inner_margin(8.0);
                    frame.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let resp = ui
                            .horizontal(|ui| {
                                ui.label(
                                    RichText::new("⏱ Mécaniques")
                                        .size(11.0 * s)
                                        .color(Color32::WHITE),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("✖")
                                                        .size(11.0 * s)
                                                        .color(Color32::from_rgb(231, 76, 60)),
                                                )
                                                .frame(false),
                                            )
                                            .on_hover_text("Masquer (réactivable dans Settings)")
                                            .clicked()
                                        {
                                            cfg.mech_overlay_window = false;
                                            changed = true;
                                        }
                                    },
                                );
                            })
                            .response;
                        let mut drag_rect = resp.rect.expand2(egui::vec2(0.0, 4.0));
                        drag_rect.max.x -= 26.0 * s;
                        let interact = ui.interact(
                            drag_rect,
                            ui.id().with("mech_drag"),
                            egui::Sense::click_and_drag(),
                        );
                        if interact.dragged() {
                            drag_now = true;
                            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::StartDrag);
                        }
                        ui.separator();
                        if bars.is_empty() {
                            ui.label(
                                RichText::new("Aucune mécanique chronométrée en cours.")
                                    .size(10.0 * s)
                                    .weak(),
                            );
                        } else {
                            for (label, frac, eta, urgent) in &bars {
                                let col = if *urgent {
                                    Color32::from_rgb(231, 76, 60)
                                } else {
                                    Color32::from_rgb(46, 204, 113)
                                };
                                bar_row(ui, label, &format!("{eta:.0} s"), *frac, col, Color32::WHITE, s);
                            }
                        }
                    });
                });
        });

        if changed {
            cfg.save();
        }
        self.mech_dragging = drag_now;
    }

    fn show_overlay(&mut self, ctx: &egui::Context) {
        let overlay_id = egui::ViewportId::from_hash_of("eq2_overlay");

        let enc = self
            .engine
            .display_encounter()
            .cloned()
            .map(|e| self.for_display(&e));
        let live = self.engine.current.is_some();
        let self_name = self.self_name().map(|s| s.to_string());
        let toasts: Vec<String> = self
            .trigger_engine
            .toasts
            .iter()
            .map(|t| t.text.clone())
            .collect();
        // Timers déclenchés : (label, restant, total).
        let timers: Vec<(String, f32, f32)> = self
            .trigger_engine
            .timers
            .iter()
            .map(|t| (t.label.clone(), t.remaining(), t.total))
            .collect();
        // Comptes à rebours de mécaniques (optionnels) : (label, fraction restante, eta, urgent).
        let mech_bars: Vec<(String, f32, f64, bool)> =
            if self.config.mech_overlay && self.config.mechanics_enabled {
                let now = Self::now_epoch();
                self.engine
                    .mech
                    .predictions(now)
                    .into_iter()
                    .filter(|p| p.eta <= 30.0)
                    .take(4)
                    .map(|p| {
                        let frac = (p.eta / p.period.max(1.0)).clamp(0.0, 1.0) as f32;
                        (
                            format!("{} {}", p.kind.icon(), p.ability),
                            frac,
                            p.eta.max(0.0),
                            p.eta <= p.lead as f64,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let need_passthrough_cmd = !self.passthrough_sent;
        let was_dragging = self.overlay_dragging;

        let cfg = &mut self.config;
        let s = cfg.overlay_scale.clamp(0.6, 2.5);
        let rows_max = cfg.overlay_rows;

        // Pré-calcul des sections affichées. Le texte de droite de chaque barre
        // est soit le format auto « 4691 (93.8k · 52.8%) », soit le template
        // custom rendu sur le joueur de la barre.
        let bar_format = cfg.overlay_bar_format.trim().to_string();
        let mut sections: Vec<OverlaySection> = Vec::new();
        if let Some(e) = &enc {
            let mk = |label: &'static str,
                      ranking: Vec<(&String, &crate::combat::Combatant)>,
                      per_sec: &dyn Fn(&crate::combat::Combatant) -> f64,
                      total: &dyn Fn(&crate::combat::Combatant) -> u64|
             -> Vec<(usize, String, String, f32, bool)> {
                let sec_total: u64 = ranking.iter().map(|(_, c)| total(c)).sum::<u64>().max(1);
                let top = ranking.first().map(|(_, c)| total(c)).unwrap_or(1).max(1);
                let mk_row = |rank: usize, n: &String, c: &crate::combat::Combatant| {
                    let value = if bar_format.is_empty() {
                        format!(
                            "{label}: {} (Total: {} - {:.1}%)",
                            fmt_f64(per_sec(c)),
                            fmt_num(total(c)),
                            total(c) as f64 / sec_total as f64 * 100.0
                        )
                    } else {
                        crate::template::render(&bar_format, Some(e), Some(n.as_str()))
                    };
                    (
                        rank,
                        n.clone(),
                        value,
                        (total(c) as f64 / top as f64) as f32,
                        self_name.as_deref() == Some(n.as_str()),
                    )
                };
                let mut rows: Vec<(usize, String, String, f32, bool)> = ranking
                    .iter()
                    .take(rows_max)
                    .enumerate()
                    .map(|(i, (n, c))| mk_row(i + 1, n, c))
                    .collect();
                // Si je suis hors du top affiché, ma ligne remplace la dernière
                // barre (avec mon vrai rang) — on doit toujours se voir.
                if let Some(sn) = self_name.as_deref() {
                    let my_pos = ranking.iter().position(|(n, _)| n.as_str() == sn);
                    if let Some(pos) = my_pos {
                        if pos >= rows_max && rows_max > 0 {
                            let (n, c) = ranking[pos];
                            if let Some(last) = rows.last_mut() {
                                *last = mk_row(pos + 1, n, c);
                            }
                        }
                    }
                }
                rows
            };
            if cfg.overlay_show_dps {
                sections.push(OverlaySection {
                    label: "DPS",
                    rows: mk("DPS", e.damage_ranking(), &|c| e.dps_of(c), &|c| c.damage),
                });
            }
            if cfg.overlay_show_hps {
                sections.push(OverlaySection {
                    label: "HPS",
                    rows: mk("HPS", e.heal_ranking(), &|c| e.hps_of(c), &|c| c.healing),
                });
            }
            if cfg.overlay_show_power {
                sections.push(OverlaySection {
                    label: "Power",
                    rows: mk("Power", e.power_ranking(), &|c| e.pps_of(c), &|c| c.power),
                });
            }
            sections.retain(|sec| !sec.rows.is_empty());
        }

        // Animation des barres : interpolation douce vers la fraction cible.
        {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            let k = 1.0 - (-8.0 * dt).exp();
            let mut animating = false;
            let mut live_keys: std::collections::HashSet<(&'static str, String)> =
                std::collections::HashSet::new();
            for sec in &mut sections {
                for row in &mut sec.rows {
                    let key = (sec.label, row.1.clone());
                    let cur = self.bar_anim.entry(key.clone()).or_insert(row.3);
                    *cur += (row.3 - *cur) * k;
                    if (row.3 - *cur).abs() > 0.004 {
                        animating = true;
                    }
                    row.3 = *cur;
                    live_keys.insert(key);
                }
            }
            self.bar_anim.retain(|k, _| live_keys.contains(k));
            if animating {
                ctx.request_repaint();
            }
        }

        // Barre de titre : template custom si défini, sinon format auto.
        let title = if !cfg.overlay_title_format.trim().is_empty() {
            crate::template::render(
                &cfg.overlay_title_format,
                enc.as_ref(),
                self_name.as_deref(),
            )
        } else {
            match &enc {
            Some(e) => {
                let base = format!(
                    "{} — {}{}",
                    e.title(),
                    fmt_duration(e.duration()),
                    if live { "" } else { " (fini)" }
                );
                if cfg.overlay_title_stats {
                    let raid_dps = e.total_damage() as f64 / e.duration() as f64;
                    let kills = if e.kills.is_empty() {
                        String::new()
                    } else {
                        format!("  •  {} kill{}", e.kills.len(), if e.kills.len() > 1 { "s" } else { "" })
                    };
                    format!(
                        "{base}  •  {} dmg  •  {} raid{kills}",
                        fmt_num(e.total_damage()),
                        fmt_f64(raid_dps)
                    )
                } else {
                    base
                }
            }
            None => "EQ2 Parser — en attente".to_string(),
            }
        };

        // Texte custom : rendu du template ({{dps}}, {{hps:1}}, …) sur l'encounter affiché.
        let custom_rendered = if cfg.overlay_custom_text.trim().is_empty() {
            String::new()
        } else {
            crate::template::render(
                &cfg.overlay_custom_text,
                enc.as_ref(),
                self_name.as_deref(),
            )
        };
        let custom_lines: Vec<String> = custom_rendered
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();

        // Espace réservé en bas (texte custom si en bas + toasts) pour l'auto-fit.
        let row_h = 20.0 * s;
        let custom_h = if cfg.overlay_text_top {
            0.0
        } else {
            custom_lines.len() as f32 * 18.0 * s
        };
        let reserved_bottom = custom_h + toasts.len() as f32 * 18.0 * s;

        let mut changed = false;
        let mut passthrough_toggled = false;
        let mut drag_now = false;

        let mut builder = egui::ViewportBuilder::default()
            .with_title("EQ2 Overlay")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(true)
            .with_min_inner_size([180.0, 60.0])
            .with_inner_size([cfg.overlay_width, cfg.overlay_height]);
        // Pendant le déplacement, l'OS pilote la position : ne pas la réimposer
        // (sinon l'overlay tremble en se battant avec le drag).
        if !was_dragging {
            if let Some((x, y)) = cfg.overlay_pos {
                builder = builder.with_position([x, y]);
            }
        }

        ctx.show_viewport_immediate(
            overlay_id,
            builder,
            |ctx, _class| {
                // Suit la taille et la position réelles de la fenêtre.
                let actual = ctx.input(|i| i.screen_rect().size());
                if (actual.x - cfg.overlay_width).abs() > 1.0
                    || (actual.y - cfg.overlay_height).abs() > 1.0
                {
                    cfg.overlay_width = actual.x;
                    cfg.overlay_height = actual.y;
                    changed = true;
                }
                // Suivi de position : pas pendant le drag (l'OS pilote, et on
                // éviterait sinon une sauvegarde disque à chaque frame).
                if !was_dragging {
                    if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                        let pos = (rect.min.x, rect.min.y);
                        if cfg
                            .overlay_pos
                            .is_none_or(|p| (p.0 - pos.0).abs() > 1.0 || (p.1 - pos.1).abs() > 1.0)
                        {
                            cfg.overlay_pos = Some(pos);
                            changed = true;
                        }
                    }
                }
                // Fondu au survol : on voit ce que l'overlay cache sans le déplacer.
                let hovered = ctx.input(|i| i.pointer.has_pointer());
                let fade = if cfg.overlay_fade_hover && hovered && !cfg.overlay_click_through
                {
                    0.25
                } else {
                    1.0
                };
                if need_passthrough_cmd {
                    ctx.send_viewport_cmd_to(
                        overlay_id,
                        egui::ViewportCommand::MousePassthrough(cfg.overlay_click_through),
                    );
                }

                let bg = Color32::from_rgba_unmultiplied(
                    cfg.overlay_bg[0],
                    cfg.overlay_bg[1],
                    cfg.overlay_bg[2],
                    (cfg.overlay_opacity * 235.0 * fade) as u8,
                );
                let accent = Color32::from_rgb(
                    cfg.overlay_accent[0],
                    cfg.overlay_accent[1],
                    cfg.overlay_accent[2],
                );

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let frame = egui::Frame::new()
                            .fill(bg)
                            .corner_radius(8.0)
                            .inner_margin(8.0);
                        frame.show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.set_opacity(fade);

                            // Barre de titre : drag + clic droit = réglages rapides,
                            // croix à droite pour masquer l'overlay.
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.label(
                                        RichText::new(title.as_str())
                                            .size(11.0 * s)
                                            .color(Color32::WHITE),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("✖")
                                                            .size(11.0 * s)
                                                            .color(Color32::from_rgb(
                                                                231, 76, 60,
                                                            )),
                                                    )
                                                    .frame(false),
                                                )
                                                .on_hover_text(
                                                    "Masquer l'overlay (réactivable \
                                                     via la case Overlay de la fenêtre \
                                                     principale)",
                                                )
                                                .clicked()
                                            {
                                                cfg.overlay_enabled = false;
                                                changed = true;
                                            }
                                        },
                                    );
                                })
                                .response;
                            // Zone de drag : la barre de titre moins la croix.
                            let mut drag_rect = resp.rect.expand2(egui::vec2(0.0, 4.0));
                            drag_rect.max.x -= 26.0 * s;
                            let interact = ui.interact(
                                drag_rect,
                                ui.id().with("drag"),
                                egui::Sense::click_and_drag(),
                            );
                            if interact.dragged() && !cfg.overlay_locked {
                                drag_now = true;
                                ctx.send_viewport_cmd_to(
                                    overlay_id,
                                    egui::ViewportCommand::StartDrag,
                                );
                            }
                            overlay_quick_menu(
                                &interact,
                                cfg,
                                &mut changed,
                                &mut passthrough_toggled,
                            );
                            ui.separator();

                            // Timers déclenchés (comptes à rebours).
                            for (label, remaining, total) in &timers {
                                bar_row(
                                    ui,
                                    &format!("⏱ {label}"),
                                    &format!("{remaining:.0} s"),
                                    remaining / total.max(1.0),
                                    Color32::from_rgb(230, 126, 34),
                                    Color32::WHITE,
                                    s,
                                );
                            }

                            // Comptes à rebours de mécaniques (prochains casts).
                            for (label, frac, eta, urgent) in &mech_bars {
                                let col = if *urgent {
                                    Color32::from_rgb(231, 76, 60)
                                } else {
                                    Color32::from_rgb(46, 204, 113)
                                };
                                bar_row(
                                    ui,
                                    label,
                                    &format!("{eta:.0} s"),
                                    *frac,
                                    col,
                                    Color32::WHITE,
                                    s,
                                );
                            }

                            // Texte custom en haut (sous le titre).
                            if cfg.overlay_text_top {
                                for line in &custom_lines {
                                    ui.label(
                                        RichText::new(line.as_str())
                                            .size(11.0 * s)
                                            .italics()
                                            .color(accent),
                                    );
                                }
                            }

                            // Sections de barres — auto-fit : on s'arrête quand
                            // la hauteur restante est épuisée.
                            let show_headers = sections.len() > 1;
                            'sections: for sec in &sections {
                                if show_headers {
                                    if ui.available_height() < 16.0 * s + row_h + reserved_bottom
                                    {
                                        break 'sections;
                                    }
                                    ui.label(
                                        RichText::new(sec.label)
                                            .size(10.0 * s)
                                            .strong()
                                            .color(accent),
                                    );
                                }
                                for (rank, name, value, frac, is_self) in &sec.rows {
                                    if ui.available_height() < row_h + reserved_bottom {
                                        continue 'sections;
                                    }
                                    bar_row(
                                        ui,
                                        &format!("{rank}. {name}"),
                                        value,
                                        *frac,
                                        player_color(name),
                                        if *is_self { accent } else { Color32::WHITE },
                                        s,
                                    );
                                }
                            }

                            // Texte custom en bas (template rendu, multi-lignes).
                            if !cfg.overlay_text_top {
                                for line in &custom_lines {
                                    ui.label(
                                        RichText::new(line.as_str())
                                            .size(11.0 * s)
                                            .italics()
                                            .color(accent),
                                    );
                                }
                            }

                            // Toasts triggers.
                            for t in &toasts {
                                ui.label(
                                    RichText::new(format!("🔔 {t}"))
                                        .size(12.0 * s)
                                        .color(accent)
                                        .strong(),
                                );
                            }
                        });

                        // Grip de redimensionnement (coin bas-droit), comme une fenêtre.
                        if cfg.overlay_locked {
                            return;
                        }
                        let screen = ctx.screen_rect();
                        let grip = 16.0;
                        let grip_rect = egui::Rect::from_min_max(
                            screen.max - egui::vec2(grip, grip),
                            screen.max,
                        );
                        let grip_resp = ui.interact(
                            grip_rect,
                            ui.id().with("resize_grip"),
                            egui::Sense::drag(),
                        );
                        if grip_resp.drag_started() {
                            ctx.send_viewport_cmd_to(
                                overlay_id,
                                egui::ViewportCommand::BeginResize(
                                    egui::viewport::ResizeDirection::SouthEast,
                                ),
                            );
                        }
                        if grip_resp.hovered() {
                            ctx.set_cursor_icon(egui::CursorIcon::ResizeSouthEast);
                        }
                        // Trois traits diagonaux, plus visibles au survol.
                        let alpha = if grip_resp.hovered() { 200 } else { 90 };
                        let gc = Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
                        let p = ui.painter();
                        for k in 1..=3 {
                            let off = k as f32 * 4.0;
                            p.line_segment(
                                [
                                    egui::pos2(screen.max.x - off, screen.max.y - 2.0),
                                    egui::pos2(screen.max.x - 2.0, screen.max.y - off),
                                ],
                                egui::Stroke::new(1.5, gc),
                            );
                        }
                    });
            },
        );

        if passthrough_toggled {
            self.passthrough_sent = false;
        } else {
            self.passthrough_sent = true;
        }
        if changed {
            self.config.save();
        }
        self.overlay_dragging = drag_now;
    }
}

/// Menu clic droit de l'overlay : tous les réglages en accès rapide.
fn overlay_quick_menu(
    resp: &egui::Response,
    cfg: &mut Config,
    changed: &mut bool,
    passthrough_toggled: &mut bool,
) {
    resp.context_menu(|ui| {
        ui.set_min_width(260.0);
        ui.label(RichText::new("Réglages overlay").strong());
        // Profils commutables.
        if !cfg.overlay_profiles.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Profil :");
                let profiles = cfg.overlay_profiles.clone();
                for p in &profiles {
                    if ui.small_button(p.name.as_str()).clicked() {
                        cfg.apply_profile(p);
                        *changed = true;
                        *passthrough_toggled = true;
                        ui.close_menu();
                    }
                }
            });
        }
        ui.separator();

        *changed |= ui
            .add(egui::Slider::new(&mut cfg.overlay_opacity, 0.1..=1.0).text("Transparence"))
            .changed();
        *changed |= ui
            .add(egui::Slider::new(&mut cfg.overlay_scale, 0.6..=2.0).text("Taille du texte"))
            .changed();
        *changed |= ui
            .add(egui::Slider::new(&mut cfg.overlay_rows, 1..=15).text("Joueurs max"))
            .changed();
        ui.label(
            RichText::new("↘ Redimensionne la fenêtre par le grip en bas à droite.")
                .weak()
                .small(),
        );

        ui.horizontal(|ui| {
            ui.label("Fond :");
            *changed |= ui.color_edit_button_srgb(&mut cfg.overlay_bg).changed();
            ui.label("Accent :");
            *changed |= ui.color_edit_button_srgb(&mut cfg.overlay_accent).changed();
        });
        ui.separator();

        ui.label(RichText::new("Contenu").strong());
        *changed |= ui
            .checkbox(&mut cfg.overlay_title_stats, "Titre détaillé (total, DPS raid, kills)")
            .changed();
        *changed |= ui.checkbox(&mut cfg.overlay_show_dps, "Barres DPS").changed();
        *changed |= ui.checkbox(&mut cfg.overlay_show_hps, "Barres HPS").changed();
        *changed |= ui
            .checkbox(&mut cfg.overlay_show_power, "Barres Power")
            .changed();
        ui.horizontal(|ui| {
            ui.label("Texte :");
            *changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut cfg.overlay_custom_text)
                        .hint_text("ex : hps {{hps}} — top {{name:1}} {{dps:1}}")
                        .desired_rows(2)
                        .desired_width(190.0),
                )
                .changed();
            ui.menu_button("➕", |ui| {
                ui.set_min_width(280.0);
                for (var, desc) in crate::template::VARIABLES {
                    if ui.button(format!("{var}  —  {desc}")).clicked() {
                        cfg.overlay_custom_text.push_str(var);
                        *changed = true;
                        ui.close_menu();
                    }
                }
            });
        });
        *changed |= ui
            .checkbox(&mut cfg.overlay_text_top, "Texte en haut (sous le titre)")
            .changed();
        ui.horizontal(|ui| {
            ui.label("Barres :");
            *changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut cfg.overlay_bar_format)
                        .hint_text("vide = auto · ex : {{dps}} · {{pct}}")
                        .desired_width(190.0),
                )
                .on_hover_text(
                    "Côté droit de chaque barre. Variables résolues sur le joueur \
                     de la barre : {{dps}} {{dmg}} {{pct}} {{crit}}…",
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Titre :");
            *changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut cfg.overlay_title_format)
                        .hint_text("vide = auto · ex : {{target}} {{time}} | {{raiddps}}")
                        .desired_width(190.0),
                )
                .changed();
        });
        ui.separator();

        *changed |= ui
            .checkbox(&mut cfg.overlay_locked, "🔒 Verrouiller position/taille")
            .changed();
        *changed |= ui
            .checkbox(&mut cfg.overlay_fade_hover, "👻 Fondu au survol de la souris")
            .changed();
        if ui
            .checkbox(&mut cfg.overlay_click_through, "Click-through")
            .on_hover_text(
                "L'overlay laisse passer les clics. À désactiver depuis Settings \
                 (le clic droit ne marchera plus ici !)",
            )
            .changed()
        {
            *changed = true;
            *passthrough_toggled = true;
        }
        if ui.button("✖ Masquer l'overlay").clicked() {
            cfg.overlay_enabled = false;
            *changed = true;
            ui.close_menu();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn bar_row(
    ui: &mut egui::Ui,
    name: &str,
    value: &str,
    frac: f32,
    color: Color32,
    name_color: Color32,
    scale: f32,
) {
    let height = 18.0 * scale;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    // Fond
    painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(255, 255, 255, 10));
    // Remplissage
    let fill = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * frac.clamp(0.0, 1.0), height),
    );
    painter.rect_filled(fill, 3.0, color.gamma_multiply(0.55));
    // Texte
    painter.text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0 * scale),
        name_color,
    );
    painter.text(
        rect.right_center() - egui::vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(12.0 * scale),
        Color32::WHITE,
    );
    ui.add_space(2.0);
}
