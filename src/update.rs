//! Auto-Updater der öffentlichen Version: neue Versionen kommen als GitHub-Release aus REPO.
//! Ablauf: höchstens einmal am Tag `releases/latest` abfragen -> update.json; auf Wunsch (Knopf im
//! Fenster) `lolstats.exe apply-update`: herunterladen, SHA-256 prüfen, exe tauschen, neu starten.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;

pub const REPO: &str = "DavidDev25/lolstats-app";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const UPDATE: &str = "update.json";
/// Name der exe und der Prüfsummen-Datei im Release
pub const ASSET: &str = "lolstats.exe";
pub const ASSET_SHA: &str = "lolstats.exe.sha256";
const CHECK_EVERY_MS: i64 = 24 * 3600 * 1000;
/// Obergrenze für den Download (Schutz vor falschen Dateien)
const MAX_BYTES: u64 = 60 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct UpdateInfo {
    /// letzte Prüfung (ms)
    pub checked: i64,
    /// neueste Version laut GitHub (ohne "v")
    pub latest: String,
    pub exe_url: String,
    pub sha_url: String,
    /// Release-Notizen und Seite des Releases
    pub notes: String,
    pub page: String,
    /// letzter Fehler (Prüfung oder Installation), leer = ok
    pub error: String,
}

impl UpdateInfo {
    pub fn available(&self) -> bool {
        newer(&self.latest, VERSION) && !self.exe_url.is_empty() && !self.sha_url.is_empty()
    }
    pub fn due(&self, now: i64) -> bool {
        now - self.checked >= CHECK_EVERY_MS
    }
}

/// true, wenn Version `a` neuer ist als `b` ("1.2.10" > "1.2.9", führendes "v" erlaubt).
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.trim().trim_start_matches('v').split(['.', '-']).map(|x| x.parse().unwrap_or(0)).collect() };
    let (x, y) = (parse(a), parse(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if p != q {
            return p > q;
        }
    }
    false
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Erste 64 Hex-Zeichen einer .sha256-Datei ("<hash>  lolstats.exe" oder nur der Hash).
pub fn parse_sha(text: &str) -> Option<String> {
    let h: String = text.split_whitespace().next()?.to_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// HTTPS mit normaler Zertifikatsprüfung (anders als der lokale Client, der ein eigenes Zertifikat nutzt).
fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(Duration::from_secs(30)).build())
}

fn get(a: &ureq::Agent, url: &str) -> Result<ureq::Response, String> {
    a.get(url)
        .set("User-Agent", &format!("lolstats/{VERSION}"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())
}

/// Liest das neueste Release aus der GitHub-API.
pub fn check() -> Result<UpdateInfo, String> {
    let a = agent()?;
    let v: serde_json::Value = get(&a, &format!("https://api.github.com/repos/{REPO}/releases/latest"))?.into_json().map_err(|e| e.to_string())?;
    from_release(&v)
}

pub fn from_release(v: &serde_json::Value) -> Result<UpdateInfo, String> {
    let latest = v["tag_name"].as_str().ok_or("Release ohne Versionsnummer")?.trim_start_matches('v').to_string();
    let asset = |name: &str| {
        v["assets"].as_array().into_iter().flatten().find(|x| x["name"].as_str() == Some(name)).and_then(|x| x["browser_download_url"].as_str()).unwrap_or_default().to_string()
    };
    Ok(UpdateInfo {
        checked: crate::model::now_ms(),
        latest,
        exe_url: asset(ASSET),
        sha_url: asset(ASSET_SHA),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        error: String::new(),
    })
}

/// Im Dienst: höchstens einmal am Tag prüfen und das Ergebnis für das Fenster ablegen.
/// Liefert die neue Version, wenn gerade ein Update gefunden wurde.
pub fn check_if_due(store: &Store) -> Option<String> {
    let mut info: UpdateInfo = store.read(UPDATE).unwrap_or_default();
    let now = crate::model::now_ms();
    if !info.due(now) {
        return None;
    }
    let before = info.available().then(|| info.latest.clone());
    match check() {
        Ok(new) => info = new,
        Err(e) => {
            info.checked = now;
            info.error = format!("Update-Prüfung fehlgeschlagen: {e}");
        }
    }
    store.write(UPDATE, &info);
    (info.available() && before.as_deref() != Some(info.latest.as_str())).then(|| info.latest.clone())
}

/// Lädt die neue exe, prüft die Prüfsumme und tauscht sie gegen die laufende aus.
/// Die laufende Datei darf unter Windows umbenannt werden; sie wird beim nächsten Start gelöscht.
pub fn apply(info: &UpdateInfo) -> Result<(), String> {
    if !info.available() {
        return Err("kein Update verfügbar".into());
    }
    let a = agent()?;
    let expected = parse_sha(&get(&a, &info.sha_url)?.into_string().map_err(|e| e.to_string())?).ok_or("Prüfsummen-Datei unlesbar")?;
    let mut bytes = Vec::new();
    get(&a, &info.exe_url)?.into_reader().take(MAX_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let got = sha256_hex(&bytes);
    if got != expected {
        return Err(format!("Prüfsumme stimmt nicht (erwartet {expected}, erhalten {got}) – Update abgebrochen"));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new.exe");
    let old = exe.with_extension("old.exe");
    std::fs::write(&new, &bytes).map_err(|e| format!("Schreiben fehlgeschlagen: {e}"))?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("Umbenennen fehlgeschlagen: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        // zurückrollen, damit lolstats startfähig bleibt
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("Austausch fehlgeschlagen: {e}"));
    }
    Ok(())
}

/// Reste eines früheren Updates entfernen (beim Start).
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
        let _ = std::fs::remove_file(exe.with_extension("new.exe"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(newer("v1.0.0", "0.9.0"));
        assert!(newer("0.9.10", "0.9.9"));
        assert!(!newer("0.9.0", "0.9.0"));
        assert!(!newer("0.8.5", "0.9.0"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("", "0.9.0"));
    }

    #[test]
    fn sha_and_release_parsing() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let line = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD  lolstats.exe";
        assert_eq!(parse_sha(line).as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(parse_sha("kaputt").is_none());
        let v: serde_json::Value = serde_json::json!({
            "tag_name": "v0.9.1", "body": "Neu: ...", "html_url": "https://github.com/x/y/releases/tag/v0.9.1",
            "assets": [
                {"name": "lolstats.exe", "browser_download_url": "https://example/lolstats.exe"},
                {"name": "lolstats.exe.sha256", "browser_download_url": "https://example/lolstats.exe.sha256"}
            ]
        });
        let u = from_release(&v).unwrap();
        assert_eq!(u.latest, "0.9.1");
        assert!(u.exe_url.ends_with("lolstats.exe") && u.sha_url.ends_with(".sha256"));
        // fehlende Prüfsumme -> kein installierbares Update
        let mut no_sha = u.clone();
        no_sha.sha_url.clear();
        assert!(!no_sha.available() || !newer(&no_sha.latest, VERSION));
    }
}
