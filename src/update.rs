//! Auto-update via les GitHub Releases du repo.
//!
//! - `spawn_check` interroge l'API GitHub en arrière-plan et signale
//!   une version plus récente que celle compilée.
//! - `apply` télécharge le nouvel exe, remplace l'actuel (rename de l'exe
//!   en cours d'exécution = autorisé sous Windows), relance et quitte.

use std::io::Read;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const REPO: &str = "AlexRovere/eq2-parser";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Tag de la release (ex. `v0.2.0`).
    pub version: String,
    /// URL de téléchargement direct de l'exe.
    pub url: String,
}

fn parse_ver(s: &str) -> (u64, u64, u64) {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.').map(|p| {
        p.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    });
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    parse_ver(remote) > parse_ver(local)
}

/// Vérifie en arrière-plan si une version plus récente existe.
/// Silencieux en cas d'échec (hors-ligne, rate-limit…).
pub fn spawn_check(tx: Sender<UpdateInfo>) {
    std::thread::spawn(move || {
        if let Some(info) = fetch_latest() {
            if is_newer(&info.version, CURRENT_VERSION) {
                let _ = tx.send(info);
            }
        }
    });
}

fn fetch_latest() -> Option<UpdateInfo> {
    let resp = ureq::get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))
    .set("User-Agent", "eq2-tools-updater")
    .timeout(Duration::from_secs(10))
    .call()
    .ok()?;
    let body: serde_json::Value = serde_json::from_reader(resp.into_reader()).ok()?;
    let version = body["tag_name"].as_str()?.to_string();
    let url = body["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        if name.ends_with(".exe") {
            a["browser_download_url"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    })?;
    Some(UpdateInfo { version, url })
}

/// Télécharge et installe la mise à jour, puis relance l'application.
/// Ne retourne que en cas d'erreur (succès = exit du process).
pub fn apply(url: &str) -> Result<(), String> {
    let resp = ureq::get(url)
        .set("User-Agent", "eq2-tools-updater")
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(|e| format!("téléchargement : {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("lecture : {e}"))?;
    if bytes.len() < 1_000_000 {
        return Err(format!(
            "fichier suspect ({} octets) — mise à jour annulée",
            bytes.len()
        ));
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new.exe");
    let old = exe.with_extension("old.exe");

    std::fs::write(&new, &bytes).map_err(|e| format!("écriture : {e}"))?;
    let _ = std::fs::remove_file(&old);
    // Sous Windows, on peut renommer un exe en cours d'exécution.
    std::fs::rename(&exe, &old).map_err(|e| format!("rotation : {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        // Rollback : on remet l'ancien en place.
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("installation : {e}"));
    }

    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| format!("relance : {e}"))?;
    std::process::exit(0);
}

/// Supprime le `.old.exe` laissé par une mise à jour précédente.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        // Tolérant aux suffixes (v0.2.0-beta) et au bruit.
        assert!(is_newer("v0.2.0-beta", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
