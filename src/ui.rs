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
        let name = char_name_from_path(&path).unwrap_or_else(|| "You".into());
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
        self.engine.self_name = name;
        self.lines_seen = 0;
        self.selected_encounter = None;
        self.selected_combatant = None;
        self.config.last_log = Some(path);
        self.config.save();
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

    /// Encounter prêt pour l'affichage (pets fusionnés si activé).
    fn for_display(&self, enc: &Encounter) -> Encounter {
        if self.config.merge_pets {
            enc.merged(&self.effective_owners())
        } else {
            enc.clone()
        }
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
        self.config.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_lines();
        self.engine.tick(Self::now_epoch());

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
        let mut changed = false;
        changed |= ui
            .add(
                egui::Slider::new(&mut self.config.overlay_opacity, 0.2..=1.0)
                    .text("Opacité"),
            )
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut self.config.overlay_rows, 3..=15).text("Barres max"))
            .changed();
        changed |= ui
            .checkbox(&mut self.config.overlay_show_heals, "Afficher HPS au lieu de DPS")
            .changed();
        if ui
            .checkbox(
                &mut self.config.overlay_click_through,
                "Click-through (l'overlay laisse passer les clics — déplaçable uniquement quand désactivé)",
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
                .filter(|(_, c)| metric_total(c, state.metric) > 0)
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
                        .filter(|(_, c)| metric_total(c, state.metric) > 0)
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

impl App {
    fn show_overlay(&mut self, ctx: &egui::Context) {
        let overlay_id = egui::ViewportId::from_hash_of("eq2_overlay");
        let rows = self.config.overlay_rows;
        let height = 46.0 + rows as f32 * 22.0 + 26.0;

        let enc = self
            .engine
            .display_encounter()
            .cloned()
            .map(|e| self.for_display(&e));
        let live = self.engine.current.is_some();
        let opacity = self.config.overlay_opacity;
        let show_heals = self.config.overlay_show_heals;
        let self_name = self.self_name().map(|s| s.to_string());
        let toasts: Vec<String> = self
            .trigger_engine
            .toasts
            .iter()
            .map(|t| t.text.clone())
            .collect();
        let click_through = self.config.overlay_click_through;
        let need_passthrough_cmd = !self.passthrough_sent;

        ctx.show_viewport_immediate(
            overlay_id,
            egui::ViewportBuilder::default()
                .with_title("EQ2 Overlay")
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top()
                .with_taskbar(false)
                .with_inner_size([340.0, height]),
            move |ctx, _class| {
                if need_passthrough_cmd {
                    ctx.send_viewport_cmd_to(
                        overlay_id,
                        egui::ViewportCommand::MousePassthrough(click_through),
                    );
                }

                let bg = Color32::from_rgba_unmultiplied(
                    12,
                    12,
                    18,
                    (opacity * 235.0) as u8,
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

                            // Barre de titre (zone de drag)
                            let title = match &enc {
                                Some(e) => format!(
                                    "{} — {}{}",
                                    e.title(),
                                    fmt_duration(e.duration()),
                                    if live { "" } else { " (fini)" }
                                ),
                                None => "EQ2 Tools — en attente".to_string(),
                            };
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if show_heals { "HPS" } else { "DPS" })
                                            .small()
                                            .strong()
                                            .color(Color32::from_rgb(241, 196, 15)),
                                    );
                                    ui.label(
                                        RichText::new(title).small().color(Color32::WHITE),
                                    );
                                })
                                .response;
                            let drag = ui.interact(
                                resp.rect.expand(4.0),
                                ui.id().with("drag"),
                                egui::Sense::drag(),
                            );
                            if drag.dragged() {
                                ctx.send_viewport_cmd_to(
                                    overlay_id,
                                    egui::ViewportCommand::StartDrag,
                                );
                            }
                            ui.separator();

                            // Barres
                            if let Some(e) = &enc {
                                let ranking = if show_heals {
                                    e.heal_ranking()
                                } else {
                                    e.damage_ranking()
                                };
                                let top = ranking
                                    .first()
                                    .map(|(_, c)| {
                                        if show_heals { c.healing } else { c.damage }
                                    })
                                    .unwrap_or(1)
                                    .max(1);
                                for (i, (name, c)) in
                                    ranking.iter().take(rows).enumerate()
                                {
                                    let (val, per_sec) = if show_heals {
                                        (c.healing, e.hps_of(c))
                                    } else {
                                        (c.damage, e.dps_of(c))
                                    };
                                    let frac = val as f64 / top as f64;
                                    let color = BAR_COLORS[i % BAR_COLORS.len()];
                                    let is_self =
                                        self_name.as_deref() == Some(name.as_str());
                                    bar_row(
                                        ui,
                                        name,
                                        &format!(
                                            "{}  ({})",
                                            fmt_f64(per_sec),
                                            fmt_num(val)
                                        ),
                                        frac as f32,
                                        color,
                                        is_self,
                                    );
                                }
                            }

                            // Toasts triggers
                            for t in &toasts {
                                ui.label(
                                    RichText::new(format!("🔔 {t}"))
                                        .color(Color32::from_rgb(241, 196, 15))
                                        .strong(),
                                );
                            }
                        });
                    });
            },
        );
        self.passthrough_sent = true;
    }
}

fn bar_row(
    ui: &mut egui::Ui,
    name: &str,
    value: &str,
    frac: f32,
    color: Color32,
    highlight: bool,
) {
    let height = 18.0;
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
    let name_color = if highlight {
        Color32::from_rgb(241, 196, 15)
    } else {
        Color32::WHITE
    };
    painter.text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0),
        name_color,
    );
    painter.text(
        rect.right_center() - egui::vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(12.0),
        Color32::WHITE,
    );
    ui.add_space(2.0);
}
