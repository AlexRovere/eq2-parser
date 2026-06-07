//! Triggers personnalisés façon ACT : regex sur le message brut → son + toast overlay.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub name: String,
    pub pattern: String,
    pub enabled: bool,
    /// Chemin d'un fichier audio (wav/mp3/ogg). None = bip par défaut.
    pub sound: Option<PathBuf>,
    /// Afficher un toast dans l'overlay.
    pub show_toast: bool,
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            name: "Nouveau trigger".into(),
            pattern: String::new(),
            enabled: true,
            sound: None,
            show_toast: true,
        }
    }
}

/// Toast affiché dans l'overlay.
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub created: std::time::Instant,
}

pub enum SoundCmd {
    Beep,
    Play(PathBuf),
}

pub struct TriggerEngine {
    compiled: Vec<(usize, Regex)>,
    pub toasts: Vec<Toast>,
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
    }

    pub fn check(&mut self, message: &str, triggers: &[Trigger]) {
        for (idx, re) in &self.compiled {
            if re.is_match(message) {
                let Some(t) = triggers.get(*idx) else { continue };
                if t.show_toast {
                    self.toasts.push(Toast {
                        text: t.name.clone(),
                        created: std::time::Instant::now(),
                    });
                }
                let cmd = match &t.sound {
                    Some(p) => SoundCmd::Play(p.clone()),
                    None => SoundCmd::Beep,
                };
                let _ = self.sound_tx.send(cmd);
            }
        }
        // Purge les toasts de plus de 5 s.
        self.toasts
            .retain(|t| t.created.elapsed() < Duration::from_secs(5));
    }

    pub fn test_sound(&self, sound: &Option<PathBuf>) {
        let cmd = match sound {
            Some(p) => SoundCmd::Play(p.clone()),
            None => SoundCmd::Beep,
        };
        let _ = self.sound_tx.send(cmd);
    }
}

fn audio_loop(rx: Receiver<SoundCmd>) {
    // OutputStream n'est pas Send : créé et conservé dans ce thread.
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
        // Pas de périphérique audio : on draine silencieusement.
        while rx.recv().is_ok() {}
        return;
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            SoundCmd::Beep => {
                use rodio::source::{SineWave, Source};
                if let Ok(sink) = rodio::Sink::try_new(&handle) {
                    sink.append(
                        SineWave::new(880.0)
                            .take_duration(Duration::from_millis(220))
                            .amplify(0.25),
                    );
                    sink.detach();
                }
            }
            SoundCmd::Play(path) => {
                if let Ok(file) = std::fs::File::open(&path) {
                    if let Ok(src) = rodio::Decoder::new(BufReader::new(file)) {
                        if let Ok(sink) = rodio::Sink::try_new(&handle) {
                            sink.append(src);
                            sink.detach();
                        }
                    }
                }
            }
        }
    }
}
