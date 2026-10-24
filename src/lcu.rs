//! Zugriff auf die lokale League-Client-API (LCU) und die Live-Client-API (Port 2999).
//! Alles läuft über 127.0.0.1 – es wird nichts im Spiel gelesen oder verändert.

use crate::model::{arr, js};
use crate::stats::RunePage;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn agent(timeout_s: u64) -> ureq::Agent {
    // Der Client nutzt ein selbstsigniertes Riot-Zertifikat auf 127.0.0.1.
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .expect("TLS-Setup");
    ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout(Duration::from_secs(timeout_s))
        .build()
}

#[derive(Clone)]
pub struct Lcu {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

impl Lcu {
    /// Verbindet nur, wenn der Client-Prozess läuft (verhindert veraltete lockfiles).
    pub fn connect(client_dir: &str) -> Option<Lcu> {
        let found = find_process_dir(&["LeagueClientUx.exe", "LeagueClient.exe"])?;
        let mut dirs = Vec::new();
        if !client_dir.is_empty() {
            dirs.push(PathBuf::from(client_dir));
        }
        dirs.push(found);
        for d in dirs {
            if let Ok(t) = std::fs::read_to_string(d.join("lockfile")) {
                if let Some((port, pw)) = parse_lockfile(&t) {
                    return Some(Lcu {
                        agent: agent(8),
                        base: format!("https://127.0.0.1:{port}"),
                        auth: format!("Basic {}", b64(format!("riot:{pw}").as_bytes())),
                    });
                }
            }
        }
        None
    }

    pub fn get(&self, path: &str) -> Result<Value, String> {
        self.send("GET", path, None)
    }

    pub fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let req = self
            .agent
            .request(method, &format!("{}{}", self.base, path))
            .set("Authorization", &self.auth)
            .set("Accept", "application/json");
        let resp = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        }
        .map_err(|e| e.to_string())?;
        let txt = resp.into_string().map_err(|e| e.to_string())?;
        if txt.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&txt).map_err(|e| e.to_string())
    }
}

/// Live-Client-Data-API des laufenden Spiels (nur lesend, öffentlich dokumentiert von Riot).
pub fn live_get(path: &str) -> Result<Value, String> {
    let resp = agent(2)
        .get(&format!("https://127.0.0.1:2999{path}"))
        .call()
        .map_err(|e| e.to_string())?;
    resp.into_json().map_err(|e| e.to_string())
}

/// Legt eine Runenseite "lolstats ..." an. Ersetzt nur alte "lolstats"-Seiten, nie eigene.
pub fn import_runes(l: &Lcu, page: &RunePage, name: &str) -> Result<(), String> {
    let mut ids: Vec<i32> = page.perks.to_vec();
    let shards = if page.shards.iter().all(|&x| x > 0) {
        page.shards
    } else {
        [5008, 5008, 5011]
    };
    ids.extend(shards);
    let name: String = name.chars().take(25).collect();
    let body = serde_json::json!({
        "name": name,
        "primaryStyleId": page.styles[0],
        "subStyleId": page.styles[1],
        "selectedPerkIds": ids,
        "current": true,
    });
    if let Ok(pages) = l.get("/lol-perks/v1/pages") {
        for p in arr(&pages) {
            if js(p, "name").starts_with("lolstats") && p["isDeletable"].as_bool().unwrap_or(false) {
                let _ = l.send("DELETE", &format!("/lol-perks/v1/pages/{}", p["id"]), None);
            }
        }
    }
    // Eigene Seiten werden nie angefasst: ist kein Platz frei, wird abgebrochen.
    l.send("POST", "/lol-perks/v1/pages", Some(&body))
        .map(|_| ())
        .map_err(|_| "kein freier Runenseiten-Platz (eigene Seiten werden nicht überschrieben)".into())
}

/// lockfile: "LeagueClient:<pid>:<port>:<passwort>:https"
pub fn parse_lockfile(t: &str) -> Option<(u16, String)> {
    let parts: Vec<&str> = t.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    Some((parts[2].parse().ok()?, parts[3].to_string()))
}

pub fn b64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Sucht einen laufenden Prozess per Toolhelp-Snapshot und liefert seinen Ordner.
pub fn find_process_dir(names: &[&str]) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut pid = 0;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..len]);
            if names.iter().any(|n| n.eq_ignore_ascii_case(&exe)) {
                pid = e.th32ProcessID;
                break;
            }
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if !ok {
            return None;
        }
        PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]))
            .parent()
            .map(|p| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(b64(b"riot:abc"), "cmlvdDphYmM=");
        assert_eq!(b64(b"ab"), "YWI=");
        assert_eq!(b64(b"abc"), "YWJj");
    }

    #[test]
    fn lockfile() {
        assert_eq!(
            parse_lockfile("LeagueClient:1234:54321:s3cr3t:https\n"),
            Some((54321, "s3cr3t".to_string()))
        );
        assert_eq!(parse_lockfile("kaputt"), None);
    }

    #[test]
    fn percent() {
        assert_eq!(pct("Faker#KR1"), "Faker%23KR1");
        assert_eq!(pct("a b"), "a%20b");
    }
}
