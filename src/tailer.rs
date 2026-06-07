//! Tail temps réel d'un fichier de log EQ2 (thread + polling).
//! Gère la troncature/rotation du fichier et l'encodage non-UTF8 (lossy).

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

pub struct Tailer {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub path: PathBuf,
}

impl Tailer {
    /// Démarre le tail. `from_start = true` relit tout le fichier (import historique),
    /// sinon démarre à la fin (mode live).
    pub fn start(
        path: PathBuf,
        from_start: bool,
        tx: Sender<String>,
        ctx: eframe::egui::Context,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let path2 = path.clone();

        let handle = std::thread::spawn(move || {
            tail_loop(path2, from_start, tx, ctx, stop2);
        });

        Self { stop, handle: Some(handle), path }
    }
}

impl Drop for Tailer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn tail_loop(
    path: PathBuf,
    from_start: bool,
    tx: Sender<String>,
    ctx: eframe::egui::Context,
    stop: Arc<AtomicBool>,
) {
    let Ok(file) = File::open(&path) else { return };
    let mut reader = BufReader::new(file);
    let mut pos: u64 = if from_start {
        0
    } else {
        reader.seek(SeekFrom::End(0)).unwrap_or(0)
    };
    if from_start {
        let _ = reader.seek(SeekFrom::Start(0));
    }

    let mut buf: Vec<u8> = Vec::with_capacity(512);
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }

        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => {
                // Fin du fichier : détecte une éventuelle troncature, puis attend.
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() < pos {
                        // Fichier tronqué (nouveau /log) : on repart du début.
                        if let Ok(f) = File::open(&path) {
                            reader = BufReader::new(f);
                            pos = 0;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(n) => {
                pos += n as u64;
                // Ligne potentiellement incomplète (écriture en cours) :
                // on ne traite que les lignes terminées par \n.
                if buf.last() != Some(&b'\n') {
                    // Recule pour relire la ligne complète au prochain tour.
                    let _ = reader.seek(SeekFrom::Start(pos - n as u64));
                    pos -= n as u64;
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\r', '\n']);
                if !line.is_empty() && tx.send(line.to_string()).is_err() {
                    return; // récepteur fermé
                }
                ctx.request_repaint();
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Liste les fichiers `eq2log_*.txt` d'un répertoire logs EQ2 (récursif, 1 niveau serveur),
/// triés du plus récent au plus ancien.
pub fn discover_logs(logs_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(servers) = std::fs::read_dir(logs_dir) else { return out };
    for server in servers.flatten() {
        let p = server.path();
        if p.is_dir() {
            if let Ok(files) = std::fs::read_dir(&p) {
                for f in files.flatten() {
                    let fp = f.path();
                    let name = fp.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with("eq2log_") && name.ends_with(".txt") {
                        out.push(fp);
                    }
                }
            }
        }
    }
    out.sort_by_key(|p| {
        std::cmp::Reverse(
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok(),
        )
    });
    out
}
