//! Triggers personnalisés façon ACT : regex sur le message brut →
//! toast / son / TTS / timer, avec groupes de capture et cooldown.
//!
//! Le message et le label de timer sont des templates : `{1}`…`{9}` = groupes
//! de capture numérotés, `{nom}` = groupes nommés `(?<nom>…)`, `{0}` = match complet.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// Sons de bip intégrés, sélectionnables quand aucun fichier audio n'est choisi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeepKind {
    Simple,
    Low,
    High,
    Double,
    Triple,
    Rising,
    Alarm,
}

impl Default for BeepKind {
    fn default() -> Self {
        BeepKind::Simple
    }
}

impl BeepKind {
    pub const ALL: [BeepKind; 7] = [
        BeepKind::Simple,
        BeepKind::Low,
        BeepKind::High,
        BeepKind::Double,
        BeepKind::Triple,
        BeepKind::Rising,
        BeepKind::Alarm,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            BeepKind::Simple => "Bip simple",
            BeepKind::Low => "Bip grave",
            BeepKind::High => "Bip aigu",
            BeepKind::Double => "Double bip",
            BeepKind::Triple => "Triple bip",
            BeepKind::Rising => "Montée",
            BeepKind::Alarm => "Alarme",
        }
    }

    /// Séquence de notes `(fréquence Hz, durée ms)` ; fréquence 0 = silence.
    fn notes(&self) -> &'static [(f32, u64)] {
        match self {
            BeepKind::Simple => &[(880.0, 200)],
            BeepKind::Low => &[(440.0, 260)],
            BeepKind::High => &[(1320.0, 160)],
            BeepKind::Double => &[(880.0, 110), (0.0, 70), (880.0, 110)],
            BeepKind::Triple => {
                &[(988.0, 90), (0.0, 60), (988.0, 90), (0.0, 60), (988.0, 90)]
            }
            BeepKind::Rising => &[(660.0, 90), (784.0, 90), (1047.0, 150)],
            BeepKind::Alarm => {
                &[(740.0, 140), (0.0, 50), (1100.0, 150), (0.0, 50), (740.0, 140)]
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Trigger {
    pub name: String,
    pub pattern: String,
    pub enabled: bool,
    /// Chemin d'un fichier audio (wav/mp3/ogg). None = bip intégré (`beep`).
    pub sound: Option<PathBuf>,
    /// Bip intégré utilisé quand `sound` est None.
    pub beep: BeepKind,
    /// Afficher un toast dans l'overlay.
    pub show_toast: bool,
    /// Lire le message en synthèse vocale.
    pub tts: bool,
    /// Message du toast/TTS (template avec {1}, {nom}…). Vide = le nom du trigger.
    pub message: String,
    /// Lance un compte à rebours de N secondes dans l'overlay (0 = aucun).
    pub timer_secs: u64,
    /// Label du timer (template). Vide = le nom du trigger.
    pub timer_label: String,
    /// Ne pas re-déclencher pendant N secondes (0 = pas de cooldown).
    pub cooldown_secs: u64,
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            name: "Nouveau trigger".into(),
            pattern: String::new(),
            enabled: true,
            sound: None,
            beep: BeepKind::default(),
            show_toast: true,
            tts: false,
            message: String::new(),
            timer_secs: 0,
            timer_label: String::new(),
            cooldown_secs: 0,
        }
    }
}

/// Toast affiché dans l'overlay.
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub created: Instant,
}

/// Compte à rebours actif, affiché dans l'overlay.
#[derive(Debug, Clone)]
pub struct ActiveTimer {
    pub label: String,
    pub end: Instant,
    pub total: f32,
}

impl ActiveTimer {
    pub fn remaining(&self) -> f32 {
        self.end.saturating_duration_since(Instant::now()).as_secs_f32()
    }
}

pub enum SoundCmd {
    Beep(BeepKind),
    Play(PathBuf),
    Speak(String),
}

/// Remplace {0}, {1}… et {nom} par les groupes de capture.
fn expand(template: &str, caps: &regex::Captures, re: &Regex) -> String {
    let mut out = template.to_string();
    for i in (0..caps.len()).rev() {
        if let Some(m) = caps.get(i) {
            out = out.replace(&format!("{{{i}}}"), m.as_str());
        }
    }
    for name in re.capture_names().flatten() {
        if let Some(m) = caps.name(name) {
            out = out.replace(&format!("{{{name}}}"), m.as_str());
        }
    }
    out
}

pub struct TriggerEngine {
    compiled: Vec<(usize, Regex)>,
    pub toasts: Vec<Toast>,
    pub timers: Vec<ActiveTimer>,
    last_fired: std::collections::HashMap<usize, Instant>,
    sound_tx: Sender<SoundCmd>,
    _audio_thread: std::thread::JoinHandle<()>,
}

impl TriggerEngine {
    pub fn new(triggers: &[Trigger]) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let audio = std::thread::spawn(move || audio_loop(rx));
        let mut s = Self {
            compiled: Vec::new(),
            toasts: Vec::new(),
            timers: Vec::new(),
            last_fired: std::collections::HashMap::new(),
            sound_tx: tx,
            _audio_thread: audio,
        };
        s.recompile(triggers);
        s
    }

    /// À rappeler après toute modification de la liste des triggers.
    pub fn recompile(&mut self, triggers: &[Trigger]) {
        self.compiled = triggers
            .iter()
            .enumerate()
            .filter(|(_, t)| t.enabled && !t.pattern.is_empty())
            .filter_map(|(i, t)| Regex::new(&t.pattern).ok().map(|r| (i, r)))
            .collect();
        self.last_fired.clear();
    }

    pub fn check(&mut self, message: &str, triggers: &[Trigger]) {
        for (idx, re) in &self.compiled {
            let Some(t) = triggers.get(*idx) else { continue };
            // Cooldown anti-spam.
            if t.cooldown_secs > 0 {
                if let Some(last) = self.last_fired.get(idx) {
                    if last.elapsed() < Duration::from_secs(t.cooldown_secs) {
                        continue;
                    }
                }
            }
            let Some(caps) = re.captures(message) else { continue };
            self.last_fired.insert(*idx, Instant::now());

            let template = if t.message.trim().is_empty() { &t.name } else { &t.message };
            let msg = expand(template, &caps, re);

            if t.show_toast {
                self.toasts.push(Toast { text: msg.clone(), created: Instant::now() });
            }
            if t.tts {
                let _ = self.sound_tx.send(SoundCmd::Speak(msg.clone()));
            }
            match &t.sound {
                Some(p) => {
                    let _ = self.sound_tx.send(SoundCmd::Play(p.clone()));
                }
                None if !t.tts => {
                    // Bip intégré seulement si pas de TTS (sinon redondant).
                    let _ = self.sound_tx.send(SoundCmd::Beep(t.beep));
                }
                None => {}
            }
            if t.timer_secs > 0 {
                let label_template = if t.timer_label.trim().is_empty() {
                    &t.name
                } else {
                    &t.timer_label
                };
                self.timers.push(ActiveTimer {
                    label: expand(label_template, &caps, re),
                    end: Instant::now() + Duration::from_secs(t.timer_secs),
                    total: t.timer_secs as f32,
                });
            }
        }
        self.tick();
    }

    /// Purge les toasts, déclenche l'alerte de fin des timers. À appeler chaque frame.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let mut expired: Vec<String> = Vec::new();
        self.timers.retain(|t| {
            if t.end <= now {
                expired.push(t.label.clone());
                false
            } else {
                true
            }
        });
        for label in expired {
            self.toasts.push(Toast {
                text: format!("⏰ {label}"),
                created: Instant::now(),
            });
            let _ = self.sound_tx.send(SoundCmd::Beep(BeepKind::Double));
        }
        self.toasts
            .retain(|t| t.created.elapsed() < Duration::from_secs(5));
    }

    pub fn test_sound(&self, sound: &Option<PathBuf>, beep: BeepKind) {
        let cmd = match sound {
            Some(p) => SoundCmd::Play(p.clone()),
            None => SoundCmd::Beep(beep),
        };
        let _ = self.sound_tx.send(cmd);
    }

    pub fn test_tts(&self, text: &str) {
        let _ = self.sound_tx.send(SoundCmd::Speak(text.to_string()));
    }
}

fn audio_loop(rx: Receiver<SoundCmd>) {
    // OutputStream n'est pas Send : créé et conservé dans ce thread.
    let stream = rodio::OutputStream::try_default().ok();

    while let Ok(cmd) = rx.recv() {
        match cmd {
            SoundCmd::Beep(kind) => {
                use rodio::source::{SineWave, Source};
                if let Some((_, handle)) = &stream {
                    if let Ok(sink) = rodio::Sink::try_new(handle) {
                        for (freq, ms) in kind.notes() {
                            // Fréquence 0 = silence (séparateur entre bips).
                            sink.append(
                                SineWave::new(*freq)
                                    .take_duration(Duration::from_millis(*ms))
                                    .amplify(if *freq > 0.0 { 0.25 } else { 0.0 }),
                            );
                        }
                        sink.detach();
                    }
                }
            }
            SoundCmd::Play(path) => {
                if let Some((_, handle)) = &stream {
                    if let Ok(file) = std::fs::File::open(&path) {
                        if let Ok(src) = rodio::Decoder::new(BufReader::new(file)) {
                            if let Ok(sink) = rodio::Sink::try_new(handle) {
                                sink.append(src);
                                sink.detach();
                            }
                        }
                    }
                }
            }
            SoundCmd::Speak(text) => {
                // On recrée le moteur à chaque énoncé : le backend WinRT ne
                // rejoue pas de façon fiable quand on réutilise l'instance sur
                // ce thread (symptôme « ne marche que la première fois »). On
                // attend la fin de l'élocution pour garder le moteur vivant.
                if let Ok(mut engine) = tts::Tts::default() {
                    if engine.speak(&text, true).is_ok() {
                        // Plancher estimé d'après la longueur (au cas où is_speaking
                        // renverrait faux trop tôt et couperait la voix), puis on
                        // suit is_speaking jusqu'à la fin, plafonné à 15 s.
                        let floor = Duration::from_millis(
                            (400 + text.chars().count() as u64 * 70).min(12_000),
                        );
                        let start = Instant::now();
                        std::thread::sleep(Duration::from_millis(100));
                        while start.elapsed() < floor
                            || (engine.is_speaking().unwrap_or(false)
                                && start.elapsed() < Duration::from_secs(15))
                        {
                            std::thread::sleep(Duration::from_millis(60));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_captures() {
        let re = Regex::new(r"(?<who>\w+) casts (.+)\.").unwrap();
        let caps = re.captures("Darkmage casts Fireball.").unwrap();
        assert_eq!(
            expand("{who} incante {2} !", &caps, &re),
            "Darkmage incante Fireball !"
        );
        assert_eq!(expand("match: {0}", &caps, &re), "match: Darkmage casts Fireball.");
        // Placeholder inconnu : conservé.
        assert_eq!(expand("{foo}", &caps, &re), "{foo}");
    }

    #[test]
    fn trigger_fires_timer_and_toast_with_captures() {
        let triggers = vec![Trigger {
            name: "Adds".into(),
            pattern: r"(?<mob>.+?) summons reinforcements".into(),
            message: "{mob} appelle des adds !".into(),
            timer_secs: 30,
            timer_label: "adds de {mob}".into(),
            ..Default::default()
        }];
        let mut engine = TriggerEngine::new(&triggers);
        engine.check("the Overlord summons reinforcements!", &triggers);
        assert_eq!(engine.toasts.len(), 1);
        assert_eq!(engine.toasts[0].text, "the Overlord appelle des adds !");
        assert_eq!(engine.timers.len(), 1);
        assert_eq!(engine.timers[0].label, "adds de the Overlord");
        assert!(engine.timers[0].remaining() > 29.0);
    }

    #[test]
    fn cooldown_blocks_refire() {
        let triggers = vec![Trigger {
            name: "Spam".into(),
            pattern: "hits YOU".into(),
            cooldown_secs: 60,
            ..Default::default()
        }];
        let mut engine = TriggerEngine::new(&triggers);
        engine.check("a rat hits YOU for 5 crushing damage.", &triggers);
        engine.check("a rat hits YOU for 7 crushing damage.", &triggers);
        assert_eq!(engine.toasts.len(), 1); // le 2e est bloqué par le cooldown
    }
}
