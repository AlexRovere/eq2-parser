//! Interface : fenêtre principale (Live / Encounters / Triggers / Settings)
//! + overlay DPS transparent toujours au-dessus.

use crate::combat::{fmt_duration, fmt_f64, fmt_num, CombatEngine, Encounter};
use crate::config::Config;
use crate::export;
use crate::parser::{char_name_from_path, Parser};
use crate::tailer::{discover_logs, Tailer};
use crate::triggers::{Trigger, TriggerEngine};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = Config::load();
        let trigger_engine = TriggerEngine::new(&config.triggers);
        let engine = CombatEngine::new(config.encounter_timeout);
        let available_logs = discover_logs(&config.logs_dir);
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
        };
        // Réattache automatiquement le dernier log suivi.
        if let Some(last) = app.config.last_log.clone() {
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
        self.engine.self_name = name.clone();
        // Recharge l'historique persisté de ce personnage.
        if self.config.persist_history {
            self.engine.history = crate::history::load(&server, &name);
        }
        self.current_server = server;
        self.last_hist_len = self.engine.history.len();
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
            out.allies = Some(crate::combat::compute_allies(
                enc,
                &self.engine.self_name,
                &self.engine.known_players,
                &owners,
            ));
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
                ability_breakdown(ui, enc, &name);
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
                    comparison_table(ui, p, enc);
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
        self.config.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_lines();
        self.engine.tick(Self::now_epoch());

        // Auto-sauvegarde de l'historique : nouveaux encounters + throttle 20 s.
        if self.engine.history.len() != self.last_hist_len
            && self.last_hist_save.elapsed() > Duration::from_secs(20)
        {
            self.save_history();
        }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Live, "⚔ Live");
                ui.selectable_value(&mut self.tab, Tab::Encounters, "📜 Encounters");
                ui.selectable_value(&mut self.tab, Tab::Triggers, "🔔 Triggers");
                ui.selectable_value(&mut self.tab, Tab::Settings, "⚙ Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            Tab::Settings => self.ui_settings(ui, ctx),
        });

        if self.config.overlay_enabled {
            self.show_overlay(ctx);
        } else {
            self.passthrough_sent = false;
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
            ui.centered_and_justified(|ui| {
                ui.label("En attente de combat…");
            });
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
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.engine.current.is_some() {
                        let sel = self.selected_encounter.is_none();
                        if ui.selectable_label(sel, "▶ Combat en cours").clicked() {
                            self.selected_encounter = None;
                            self.selected_combatant = None;
                        }
                    }
                    for (i, enc) in self.engine.history.iter().enumerate().rev() {
                        let label = format!(
                            "{} — {} ({})",
                            enc.title(),
                            fmt_num(enc.total_damage()),
                            fmt_duration(enc.duration())
                        );
                        let sel = self.selected_encounter == Some(i);
                        if ui.selectable_label(sel, label).clicked() {
                            self.selected_encounter = Some(i);
                            self.selected_combatant = None;
                        }
                    }
                    if self.engine.history.is_empty() && self.engine.current.is_none() {
                        ui.label(RichText::new("(vide)").weak());
                    }
                });
            });

        let raw = match self.selected_encounter {
            Some(i) => self.engine.history.get(i).cloned(),
            None => self.engine.display_encounter().cloned(),
        };
        let Some(raw) = raw else {
            ui.centered_and_justified(|ui| {
                ui.label("Sélectionne un encounter à gauche.");
            });
            return;
        };
        let enc = self.for_display(&raw);
        ui.horizontal(|ui| {
            ui.heading(enc.title());
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
                self.config.triggers.push(Trigger::default());
            }
        });
        ui.label(
            RichText::new(
                "Regex testée sur chaque ligne du log. Ex : `Verex N'Za says` ou `has been slain`",
            )
            .weak()
            .small(),
        );
        ui.separator();

        let mut changed = false;
        let mut to_remove: Option<usize> = None;
        let mut to_test: Option<Option<PathBuf>> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, t) in self.config.triggers.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut t.enabled, "").changed();
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
                        if ui.button("▶ Tester").clicked() {
                            to_test = Some(t.sound.clone());
                        }
                        if ui.button("🗑 Supprimer").clicked() {
                            to_remove = Some(i);
                        }
                    });
                });
            }
        });

        if let Some(s) = to_test {
            self.trigger_engine.test_sound(&s);
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

    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Fichier de log");
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
        });

        ui.checkbox(
            &mut self.config.import_existing,
            "Relire tout le fichier à l'attache (import de l'historique)",
        );

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

        ui.separator();
        ui.heading("Combat");
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

        ui.separator();
        ui.heading("Historique");
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

        ui.separator();
        ui.heading("Pets");
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

        ui.separator();
        ui.heading("Overlay");
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
        if changed {
            self.config.save();
        }
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
        let ranking: Vec<(String, crate::combat::Combatant)> = enc
            .damage_ranking()
            .into_iter()
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        let heals: Vec<(String, crate::combat::Combatant)> = enc
            .heal_ranking()
            .into_iter()
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();

        ui.label(RichText::new("Dégâts").strong());
        TableBuilder::new(ui)
            .id_salt("dmg_table")
            .striped(true)
            .column(Column::auto().at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder())
            .header(20.0, |mut h| {
                for t in ["Nom", "Dégâts", "DPS", "%", "Crit %", "Max hit", "Hits"] {
                    h.col(|ui| {
                        ui.label(RichText::new(t).strong());
                    });
                }
            })
            .body(|mut body| {
                let total = enc.total_damage().max(1);
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
                            ui.label(fmt_num(c.damage));
                        });
                        row.col(|ui| {
                            ui.label(fmt_f64(enc.dps_of(c)));
                        });
                        row.col(|ui| {
                            ui.label(format!(
                                "{:.1}",
                                c.damage as f64 / total as f64 * 100.0
                            ));
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.1}", c.crit_rate()));
                        });
                        row.col(|ui| {
                            ui.label(fmt_num(c.max_hit));
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", c.hits));
                        });
                    });
                }
            });

        if !heals.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Soins (heals + wards)").strong());
            TableBuilder::new(ui)
                .id_salt("heal_table")
                .striped(true)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder())
                .header(20.0, |mut h| {
                    for t in ["Nom", "Soins", "HPS", ""] {
                        h.col(|ui| {
                            ui.label(RichText::new(t).strong());
                        });
                    }
                })
                .body(|mut body| {
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
                                ui.label(fmt_num(c.healing));
                            });
                            row.col(|ui| {
                                ui.label(fmt_f64(enc.hps_of(c)));
                            });
                            row.col(|_| {});
                        });
                    }
                });
        }

        let power: Vec<(String, crate::combat::Combatant)> = enc
            .power_ranking()
            .into_iter()
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        if !power.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Power replenish").strong());
            TableBuilder::new(ui)
                .id_salt("power_table")
                .striped(true)
                .column(Column::auto().at_least(160.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::auto().at_least(80.0))
                .column(Column::remainder())
                .header(20.0, |mut h| {
                    for t in ["Nom", "Power", "Power/s", ""] {
                        h.col(|ui| {
                            ui.label(RichText::new(t).strong());
                        });
                    }
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
    }
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

            for (i, (name, c)) in candidates.iter().enumerate() {
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
                    BAR_COLORS[i % BAR_COLORS.len()],
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
                    for (i, (name, c)) in pcand.iter().enumerate() {
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
                            BAR_COLORS[i % BAR_COLORS.len()].gamma_multiply(0.7),
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
fn comparison_table(ui: &mut egui::Ui, pinned: &Encounter, current: &Encounter) {
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

    // Union des combattants, ordonnés par DPS de l'encounter affiché.
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

    TableBuilder::new(ui)
        .id_salt("cmp_table")
        .striped(true)
        .column(Column::auto().at_least(160.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(80.0))
        .column(Column::remainder())
        .header(20.0, |mut h| {
            for t in ["Nom", "DPS A (épinglé)", "DPS B (affiché)", "Δ %", ""] {
                h.col(|ui| {
                    ui.label(RichText::new(t).strong());
                });
            }
        })
        .body(|mut body| {
            for name in &names {
                let a = pinned.combatants.get(name).map(|c| pinned.dps_of(c));
                let b = current.combatants.get(name).map(|c| current.dps_of(c));
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

fn ability_breakdown(ui: &mut egui::Ui, enc: &Encounter, name: &str) {
    use egui_extras::{Column, TableBuilder};
    let Some(c) = enc.combatants.get(name) else { return };
    ui.label(RichText::new(format!("Breakdown — {name}")).strong());
    let mut abilities: Vec<_> = c.abilities.iter().collect();
    abilities.sort_by(|a, b| {
        (b.1.damage + b.1.healing + b.1.power).cmp(&(a.1.damage + a.1.healing + a.1.power))
    });
    let total = (c.damage + c.healing + c.power).max(1);

    TableBuilder::new(ui)
        .id_salt("ability_table")
        .striped(true)
        .column(Column::auto().at_least(220.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(80.0))
        .column(Column::remainder())
        .header(20.0, |mut h| {
            for t in ["Sort / CA", "Dégâts", "Soins", "Power", "%", "Hits", "Crit %", "Max"] {
                h.col(|ui| {
                    ui.label(RichText::new(t).strong());
                });
            }
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
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

/// Une section de barres de l'overlay (DPS, HPS ou Power), pré-calculée.
struct OverlaySection {
    label: &'static str,
    /// (rang dans le classement, nom, valeur/s, total, est_soi)
    rows: Vec<(usize, String, f64, u64, bool)>,
}

impl App {
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
        let need_passthrough_cmd = !self.passthrough_sent;

        let cfg = &mut self.config;
        let s = cfg.overlay_scale.clamp(0.6, 2.5);
        let rows_max = cfg.overlay_rows;

        // Pré-calcul des sections affichées.
        let mut sections: Vec<OverlaySection> = Vec::new();
        if let Some(e) = &enc {
            let mk = |ranking: Vec<(&String, &crate::combat::Combatant)>,
                      per_sec: &dyn Fn(&crate::combat::Combatant) -> f64,
                      total: &dyn Fn(&crate::combat::Combatant) -> u64|
             -> Vec<(usize, String, f64, u64, bool)> {
                let mut rows: Vec<(usize, String, f64, u64, bool)> = ranking
                    .iter()
                    .take(rows_max)
                    .enumerate()
                    .map(|(i, (n, c))| {
                        (
                            i + 1,
                            (*n).clone(),
                            per_sec(c),
                            total(c),
                            self_name.as_deref() == Some(n.as_str()),
                        )
                    })
                    .collect();
                // Si je suis hors du top affiché, ma ligne remplace la dernière
                // barre (avec mon vrai rang) — on doit toujours se voir.
                if let Some(sn) = self_name.as_deref() {
                    let my_pos = ranking.iter().position(|(n, _)| n.as_str() == sn);
                    if let Some(pos) = my_pos {
                        if pos >= rows_max && rows_max > 0 {
                            let (n, c) = ranking[pos];
                            if let Some(last) = rows.last_mut() {
                                *last = (pos + 1, n.clone(), per_sec(c), total(c), true);
                            }
                        }
                    }
                }
                rows
            };
            if cfg.overlay_show_dps {
                sections.push(OverlaySection {
                    label: "DPS",
                    rows: mk(e.damage_ranking(), &|c| e.dps_of(c), &|c| c.damage),
                });
            }
            if cfg.overlay_show_hps {
                sections.push(OverlaySection {
                    label: "HPS",
                    rows: mk(e.heal_ranking(), &|c| e.hps_of(c), &|c| c.healing),
                });
            }
            if cfg.overlay_show_power {
                sections.push(OverlaySection {
                    label: "Power",
                    rows: mk(e.power_ranking(), &|c| e.pps_of(c), &|c| c.power),
                });
            }
            sections.retain(|sec| !sec.rows.is_empty());
        }

        // Barre de titre.
        let title = match &enc {
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
            None => "EQ2 Tools — en attente".to_string(),
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

        ctx.show_viewport_immediate(
            overlay_id,
            egui::ViewportBuilder::default()
                .with_title("EQ2 Overlay")
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top()
                .with_taskbar(false)
                .with_resizable(true)
                .with_min_inner_size([180.0, 60.0])
                .with_inner_size([cfg.overlay_width, cfg.overlay_height]),
            |ctx, _class| {
                // Suit la taille réelle de la fenêtre (resize utilisateur via le grip).
                let actual = ctx.input(|i| i.screen_rect().size());
                if (actual.x - cfg.overlay_width).abs() > 1.0
                    || (actual.y - cfg.overlay_height).abs() > 1.0
                {
                    cfg.overlay_width = actual.x;
                    cfg.overlay_height = actual.y;
                    changed = true;
                }
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
                    (cfg.overlay_opacity * 235.0) as u8,
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

                            // Barre de titre : drag + clic droit = réglages rapides.
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.label(
                                        RichText::new(title.as_str())
                                            .size(11.0 * s)
                                            .color(Color32::WHITE),
                                    );
                                })
                                .response;
                            let interact = ui.interact(
                                resp.rect.expand2(egui::vec2(
                                    ui.available_width().max(0.0),
                                    4.0,
                                )),
                                ui.id().with("drag"),
                                egui::Sense::click_and_drag(),
                            );
                            if interact.dragged() {
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
                                let top = sec.rows.first().map(|r| r.3).unwrap_or(1).max(1);
                                for (i, (rank, name, per_sec, total, is_self)) in
                                    sec.rows.iter().enumerate()
                                {
                                    if ui.available_height() < row_h + reserved_bottom {
                                        continue 'sections;
                                    }
                                    bar_row(
                                        ui,
                                        &format!("{rank}. {name}"),
                                        &format!("{}  ({})", fmt_f64(*per_sec), fmt_num(*total)),
                                        (*total as f64 / top as f64) as f32,
                                        BAR_COLORS[i % BAR_COLORS.len()],
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
        ui.separator();

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
