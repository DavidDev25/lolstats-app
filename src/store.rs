//! Lokale Ablage in %LOCALAPPDATA%\lolstats:
//! - lolstats.db (SQLite, WAL): Matches, Lobbys, LP-Verlauf
//! - kleine JSON-Dateien (Config, Live-Zustand, Namen, Fensterposition)
//!
//! Hintergrunddienst und Fenster sind getrennte Prozesse mit je eigener Verbindung.

use crate::analysis::{add_lane_counts, queue_has_lanes, LaneFreq, LANE_QUEUES};
use crate::model::*;
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const DB: &str = "lolstats.db";
/// Alte JSON-Lines-Dateien (bis Version 0.1), werden beim ersten Start übernommen.
const LEGACY: [(&str, Table); 3] = [
    ("matches.jsonl", Table::Matches),
    ("lobbies.jsonl", Table::Lobbies),
    ("lp.jsonl", Table::Lp),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Matches,
    Lobbies,
    Lp,
    Timelines,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Table::Matches => "matches",
            Table::Lobbies => "lobbies",
            Table::Lp => "lp",
            Table::Timelines => "timelines",
        }
    }
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS matches (
    id    INTEGER PRIMARY KEY,
    ts    INTEGER NOT NULL,
    queue INTEGER NOT NULL,
    patch TEXT    NOT NULL,
    data  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS matches_ts ON matches(ts);
CREATE TABLE IF NOT EXISTS lobbies (
    game_id INTEGER NOT NULL,
    ts      INTEGER NOT NULL,
    queue   INTEGER NOT NULL,
    data    TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS lp (
    ts     INTEGER NOT NULL,
    tier   TEXT    NOT NULL,
    div    TEXT    NOT NULL,
    lp     INTEGER NOT NULL,
    wins   INTEGER NOT NULL,
    losses INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS timelines (
    match_id INTEGER PRIMARY KEY,
    data     TEXT    NOT NULL
);
-- Änderungszähler pro Tabelle, damit das Fenster auch ersetzte Einträge bemerkt
CREATE TABLE IF NOT EXISTS changes (
    t TEXT PRIMARY KEY,
    n INTEGER NOT NULL
);
-- Wie oft jeder Champion auf welcher Lane gespielt wurde (0..4 = TOP..SUPPORT), wird beim
-- Speichern jedes Matches mitgeführt, damit niemand dafür alle Matches laden muss
CREATE TABLE IF NOT EXISTS lane_counts (
    champ INTEGER NOT NULL,
    lane  INTEGER NOT NULL,
    n     INTEGER NOT NULL,
    PRIMARY KEY (champ, lane)
);
";

pub struct Store {
    pub dir: PathBuf,
    db: Connection,
}

impl Store {
    /// Datenordner, ohne die Datenbank zu öffnen.
    pub fn data_dir() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lolstats")
    }

    pub fn open() -> Store {
        Self::open_at(&Self::data_dir())
    }

    pub fn open_at(dir: &Path) -> Store {
        let _ = fs::create_dir_all(dir);
        let db = Connection::open(dir.join(DB)).expect("lolstats.db lässt sich nicht öffnen");
        let _ = db.busy_timeout(Duration::from_secs(3));
        db.execute_batch(SCHEMA).expect("Datenbank-Schema");
        let s = Store { dir: dir.to_path_buf(), db };
        s.migrate_legacy();
        // Version 1: Positionen erkennen jetzt CARRY/SUPPORT und Smite -> Lane-Zähler einmal neu aufbauen
        let v: i64 = s.db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
        if v < 1 {
            let _ = s.db.execute("DELETE FROM lane_counts", []);
            let _ = s.db.execute_batch("PRAGMA user_version = 1");
        }
        s.init_lane_counts();
        let (m, l) = s.purge_off_modes();
        if m + l > 0 {
            s.log(&format!("Sondermodi entfernt: {m} Matches, {l} Lobbys (ARAM, Queue 2400 usw.)"));
        }
        s
    }

    /// Baut `lane_counts` einmalig aus den vorhandenen Matches auf (Zeile für Zeile, ohne alles zu laden).
    fn init_lane_counts(&self) {
        let have: i64 = self.db.query_row("SELECT COUNT(*) FROM lane_counts", [], |r| r.get(0)).unwrap_or(0);
        if have > 0 || self.count(Table::Matches) == 0 {
            return;
        }
        let Ok(tx) = self.db.unchecked_transaction() else { return };
        let mut n = 0;
        if let Ok(mut st) = self.db.prepare("SELECT data FROM matches") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for m in rows.filter_map(|x| serde_json::from_str::<Match>(&x.ok()?).ok()) {
                    self.count_lanes(&m, 1);
                    n += 1;
                }
            }
        }
        if tx.commit().is_ok() {
            self.log(&format!("Lane-Zähler aus {n} Matches aufgebaut"));
        }
    }

    /// Lane-Zählung eines Matches hinzufügen (`sign` = 1) oder wieder abziehen (-1).
    fn count_lanes(&self, m: &Match, sign: i64) {
        let mut f = LaneFreq::new();
        add_lane_counts(&mut f, m);
        for (champ, lanes) in f {
            for (lane, n) in lanes.iter().enumerate().filter(|x| *x.1 > 0) {
                let _ = self.db.execute(
                    "INSERT INTO lane_counts (champ, lane, n) VALUES (?1, ?2, ?3)
                     ON CONFLICT(champ, lane) DO UPDATE SET n = MAX(0, n + ?3)",
                    params![champ, lane as i64, *n as i64 * sign],
                );
            }
        }
    }

    /// Champion -> Anzahl Spiele pro Lane, direkt aus der kleinen Zähltabelle.
    pub fn lane_freq(&self) -> LaneFreq {
        let mut f = LaneFreq::new();
        let Ok(mut st) = self.db.prepare("SELECT champ, lane, n FROM lane_counts") else { return f };
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))) {
            for (champ, lane, n) in rows.filter_map(Result::ok) {
                if let Some(slot) = f.entry(champ).or_default().get_mut(lane as usize) {
                    *slot = n.max(0) as u32;
                }
            }
        }
        f
    }

    pub fn p(&self, f: &str) -> PathBuf {
        self.dir.join(f)
    }

    // ---------- SQLite ----------

    fn bump(&self, t: Table) {
        let _ = self.db.execute(
            "INSERT INTO changes (t, n) VALUES (?1, 1) ON CONFLICT(t) DO UPDATE SET n = n + 1",
            params![t.name()],
        );
    }

    fn put_match(&self, m: &Match, verb: &str) {
        let Ok(data) = serde_json::to_string(m) else { return };
        // Match und Lane-Zähler gemeinsam schreiben (eigene Transaktion, außer es läuft schon eine)
        let tx = if self.db.is_autocommit() { self.db.unchecked_transaction().ok() } else { None };
        let replace = verb.contains("REPLACE");
        let old: Option<Match> = if replace {
            self.db
                .query_row("SELECT data FROM matches WHERE id = ?1", params![m.id as i64], |r| r.get::<_, String>(0))
                .ok()
                .and_then(|d| serde_json::from_str(&d).ok())
        } else {
            None
        };
        let n = self
            .db
            .execute(
                &format!("{verb} INTO matches (id, ts, queue, patch, data) VALUES (?1, ?2, ?3, ?4, ?5)"),
                params![m.id as i64, m.ts, m.queue, m.patch, data],
            )
            .unwrap_or(0);
        if n > 0 {
            if let Some(old) = &old {
                self.count_lanes(old, -1);
            }
            self.count_lanes(m, 1);
        }
        if let Some(tx) = tx {
            let _ = tx.commit();
        }
        if n > 0 {
            self.bump(Table::Matches);
        }
    }

    /// Speichert ein Match; Sondermodi (ARAM, Queue 2400 …) werden nicht gespeichert.
    pub fn add_match(&self, m: &Match) {
        if queue_has_lanes(m.queue) {
            self.put_match(m, "INSERT OR IGNORE");
        }
    }

    /// Ersetzt ein Match (z. B. nach Neu-Laden mit neuer Datenversion).
    pub fn replace_match(&self, m: &Match) {
        if queue_has_lanes(m.queue) {
            self.put_match(m, "INSERT OR REPLACE");
        }
    }

    /// Entfernt alle Sondermodus-Spiele (Matches, Lobbys, zugehörige Timelines). Liefert (Matches, Lobbys).
    /// Sondermodi zählen in keiner Auswertung; die Lane-Zählung enthält sie ohnehin nicht.
    pub fn purge_off_modes(&self) -> (usize, usize) {
        let list = LANE_QUEUES.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(",");
        let Ok(tx) = self.db.unchecked_transaction() else { return (0, 0) };
        let m = self.db.execute(&format!("DELETE FROM matches WHERE queue NOT IN ({list})"), []).unwrap_or(0);
        let l = self.db.execute(&format!("DELETE FROM lobbies WHERE queue NOT IN ({list})"), []).unwrap_or(0);
        let t = self.db.execute("DELETE FROM timelines WHERE match_id NOT IN (SELECT id FROM matches)", []).unwrap_or(0);
        if tx.commit().is_err() {
            return (0, 0);
        }
        if m > 0 {
            self.bump(Table::Matches);
        }
        if l > 0 {
            self.bump(Table::Lobbies);
        }
        if t > 0 {
            self.bump(Table::Timelines);
        }
        (m, l)
    }

    /// Speichert eine Lobby; dieselbe Spiel-ID wird nur einmal gespeichert (z. B. nach Neustart im Match).
    /// Lobbys aus Sondermodi werden nicht gespeichert.
    pub fn add_lobby(&self, l: &Lobby) {
        if !queue_has_lanes(l.queue) {
            return;
        }
        if let Ok(data) = serde_json::to_string(l) {
            let n = self
                .db
                .execute(
                    "INSERT INTO lobbies (game_id, ts, queue, data)
                     SELECT ?1, ?2, ?3, ?4
                     WHERE ?1 = 0 OR NOT EXISTS (SELECT 1 FROM lobbies WHERE game_id = ?1)",
                    params![l.game_id as i64, l.ts, l.queue, data],
                )
                .unwrap_or(0);
            if n > 0 {
                self.bump(Table::Lobbies);
            }
        }
    }

    /// Markiert die Lobby eines Spiels nachträglich als Remake.
    pub fn mark_remake(&self, game_id: u64) {
        let _ = self.db.execute(
            "UPDATE lobbies SET data = json_set(data, '$.remake', json('true')) WHERE game_id = ?1",
            params![game_id as i64],
        );
        self.bump(Table::Lobbies);
    }

    /// Ersetzt die Spieler einer erfassten Lobby (nachgetragene fehlende Spieler).
    pub fn set_lobby_players(&self, game_id: u64, ps: &[LivePlayer]) {
        let Ok(js) = serde_json::to_string(ps) else { return };
        let _ = self.db.execute(
            "UPDATE lobbies SET data = json_set(data, '$.players', json(?2)) WHERE game_id = ?1",
            params![game_id as i64, js],
        );
        self.bump(Table::Lobbies);
    }

    /// Hängt Riots LP-Meldung nachträglich an die Lobby eines Spiels.
    pub fn set_lp_note(&self, game_id: u64, n: &LpNote) {
        let Ok(js) = serde_json::to_string(n) else { return };
        let _ = self.db.execute(
            "UPDATE lobbies SET data = json_set(data, '$.lp_note', json(?2)) WHERE game_id = ?1",
            params![game_id as i64, js],
        );
        self.bump(Table::Lobbies);
    }

    pub fn add_snap(&self, s: &LpSnap) {
        let r = &s.rank;
        let _ = self.db.execute(
            "INSERT INTO lp (ts, tier, div, lp, wins, losses) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![s.ts, r.tier, r.div, r.lp, r.wins, r.losses],
        );
        self.bump(Table::Lp);
    }

    /// Speichert eine Timeline, aber nur zu einem gespeicherten Match (Sondermodi werden nicht gespeichert).
    pub fn add_timeline(&self, t: &Timeline) {
        let known = self
            .db
            .query_row("SELECT 1 FROM matches WHERE id = ?1", params![t.match_id as i64], |_| Ok(()))
            .is_ok();
        if !known {
            return;
        }
        if let Ok(data) = serde_json::to_string(t) {
            let _ = self.db.execute(
                "INSERT OR REPLACE INTO timelines (match_id, data) VALUES (?1, ?2)",
                params![t.match_id as i64, data],
            );
            self.bump(Table::Timelines);
        }
    }

    pub fn timelines(&self) -> Vec<Timeline> {
        self.json_rows("SELECT data FROM timelines")
    }

    fn json_rows<T: DeserializeOwned>(&self, sql: &str) -> Vec<T> {
        let Ok(mut st) = self.db.prepare(sql) else { return Vec::new() };
        let rows = st.query_map([], |r| r.get::<_, String>(0));
        match rows {
            Ok(it) => it.filter_map(|x| serde_json::from_str(&x.ok()?).ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Alle Matches, älteste zuerst.
    pub fn matches(&self) -> Vec<Match> {
        self.json_rows("SELECT data FROM matches ORDER BY ts")
    }

    /// Alle erfassten Lobbys in Aufnahme-Reihenfolge.
    pub fn lobbies(&self) -> Vec<Lobby> {
        self.json_rows("SELECT data FROM lobbies ORDER BY rowid")
    }

    /// LP-Verlauf in Aufnahme-Reihenfolge.
    pub fn snaps(&self) -> Vec<LpSnap> {
        let Ok(mut st) = self.db.prepare("SELECT ts, tier, div, lp, wins, losses FROM lp ORDER BY rowid") else {
            return Vec::new();
        };
        let rows = st.query_map([], |r| {
            Ok(LpSnap {
                ts: r.get(0)?,
                rank: Rank { tier: r.get(1)?, div: r.get(2)?, lp: r.get(3)?, wins: r.get(4)?, losses: r.get(5)? },
            })
        });
        match rows {
            Ok(it) => it.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn count(&self, t: Table) -> i64 {
        self.db
            .query_row(&format!("SELECT COUNT(*) FROM {}", t.name()), [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Ändert sich bei jedem Schreiben in die Tabelle (für die Neu-Lade-Erkennung im Fenster).
    /// Ein Zähler statt rowid/Anzahl, weil ersetzte Matches weder ID noch Anzahl ändern.
    pub fn version(&self, t: Table) -> i64 {
        self.db
            .query_row("SELECT n FROM changes WHERE t = ?1", params![t.name()], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn db_size(&self) -> u64 {
        ["", "-wal"]
            .iter()
            .filter_map(|s| fs::metadata(self.p(&format!("{DB}{s}"))).ok())
            .map(|m| m.len())
            .sum()
    }

    /// Übernimmt alte JSON-Lines-Dateien einmalig in die Datenbank und benennt sie in *.bak um.
    fn migrate_legacy(&self) {
        for (file, table) in LEGACY {
            let path = self.p(file);
            if !path.exists() {
                continue;
            }
            let Ok(tx) = self.db.unchecked_transaction() else { continue };
            let n = match table {
                Table::Matches => self.import::<Match>(&path, |m| self.add_match(m)),
                Table::Lobbies => self.import::<Lobby>(&path, |l| self.add_lobby(l)),
                Table::Lp => self.import::<LpSnap>(&path, |s| self.add_snap(s)),
                Table::Timelines => 0, // gab es als JSON-Lines nie
            };
            if tx.commit().is_ok() {
                let _ = fs::rename(&path, self.p(&format!("{file}.bak")));
                self.log(&format!("Migration: {n} Einträge aus {file} übernommen"));
            }
        }
    }

    fn import<T: DeserializeOwned>(&self, path: &Path, add: impl Fn(&T)) -> usize {
        let Ok(fh) = File::open(path) else { return 0 };
        let mut n = 0;
        for line in BufReader::new(fh).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<T>(&line) {
                add(&v);
                n += 1;
            }
        }
        n
    }

    // ---------- kleine JSON-Dateien ----------

    pub fn read<T: DeserializeOwned>(&self, f: &str) -> Option<T> {
        serde_json::from_slice(&fs::read(self.p(f)).ok()?).ok()
    }

    /// Schreibt atomar (tmp + rename), damit nie eine halbe Datei gelesen wird.
    pub fn write<T: Serialize>(&self, f: &str, v: &T) {
        if let Ok(data) = serde_json::to_vec(v) {
            let tmp = self.p(&format!("{f}.tmp"));
            if fs::write(&tmp, data).is_ok() {
                let _ = fs::rename(&tmp, self.p(f));
            }
        }
    }

    pub fn mtime(&self, f: &str) -> Option<SystemTime> {
        fs::metadata(self.p(f)).and_then(|m| m.modified()).ok()
    }

    pub fn log(&self, msg: &str) {
        let path = self.p("log.txt");
        if fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
            let _ = fs::rename(&path, self.p("log.old.txt"));
        }
        if let Ok(mut fh) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(fh, "[{}] {msg}", fmt_utc(now_ms()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lolstats-test-{tag}-{}", now_ms()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_and_dedupe() {
        let dir = temp_dir("db");
        let s = Store::open_at(&dir);
        let m = Match { id: 42, ts: 5, queue: 420, patch: "14.20".into(), ps: vec![Part { champ: 1, ..Default::default() }], ..Default::default() };
        s.add_match(&m);
        s.add_match(&m); // doppelt -> ignoriert
        s.add_lobby(&Lobby { game_id: 42, ts: 4, queue: 420, ..Default::default() });
        s.add_lobby(&Lobby { game_id: 42, ts: 9, queue: 420, ..Default::default() }); // doppelt -> ignoriert
        assert_eq!(s.count(Table::Lobbies), 1);
        assert!(!s.lobbies()[0].remake);
        s.mark_remake(42);
        assert!(s.lobbies()[0].remake);
        s.add_snap(&LpSnap { ts: 6, rank: Rank { tier: "GOLD".into(), div: "II".into(), lp: 10, wins: 3, losses: 2 } });
        assert_eq!(s.count(Table::Matches), 1);
        assert_eq!(s.matches()[0].ps[0].champ, 1);
        assert_eq!(s.lobbies()[0].game_id, 42);
        assert_eq!(s.snaps()[0].rank.label(), "Gold II 10 LP");
        let v = s.version(Table::Lp);
        s.add_snap(&LpSnap { ts: 7, ..Default::default() });
        assert!(s.version(Table::Lp) > v);
        // älteres Match mit kleinerer ID muss ebenfalls auffallen
        let v = s.version(Table::Matches);
        s.add_match(&Match { id: 3, ..Default::default() });
        assert!(s.version(Table::Matches) > v);
        // doppeltes Match ändert nichts, ersetztes Match schon
        let v = s.version(Table::Matches);
        s.add_match(&Match { id: 3, ..Default::default() });
        assert_eq!(s.version(Table::Matches), v);
        s.replace_match(&Match { id: 3, dur: 1234, ..Default::default() });
        assert!(s.version(Table::Matches) > v);
        assert_eq!(s.matches().iter().find(|m| m.id == 3).unwrap().dur, 1234);
        assert_eq!(s.count(Table::Matches), 2);
        let tl = |deaths: Vec<i32>| Timeline { match_id: 42, deaths, cs10: Some(70), gold10: Some(3500), ..Default::default() };
        s.add_timeline(&tl(vec![300, 900]));
        s.add_timeline(&tl(vec![300])); // ersetzt
        assert_eq!(s.timelines(), vec![tl(vec![300])]);
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }

    fn lane_match(id: u64, champs: [(i32, &str, &str); 3]) -> Match {
        let ps = champs
            .iter()
            .enumerate()
            .map(|(i, (champ, lane, role))| Part {
                puuid: format!("p{id}-{i}"),
                champ: *champ,
                team: 100,
                lane: lane.to_string(),
                role: role.to_string(),
                ..Default::default()
            })
            .collect();
        Match { id, ts: id as i64, queue: 420, dur: 1800, ps, ..Default::default() }
    }

    #[test]
    fn off_modes_are_never_stored_and_purged() {
        let dir = temp_dir("offmode");
        let s = Store::open_at(&dir);
        let sr = Match { id: 1, queue: 420, ..Default::default() };
        let aram = Match { id: 2, queue: 450, ..Default::default() };
        let event = Match { id: 3, queue: 2400, ..Default::default() };
        for m in [&sr, &aram, &event] {
            s.add_match(m);
        }
        s.replace_match(&event);
        s.add_lobby(&Lobby { game_id: 3, queue: 2400, ..Default::default() });
        s.add_timeline(&Timeline { match_id: 3, ..Default::default() }); // Match fehlt -> nicht gespeichert
        assert_eq!(s.matches().iter().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!((s.count(Table::Lobbies), s.count(Table::Timelines)), (0, 0));
        // Altbestand (z. B. von einer älteren Version) wird beim nächsten Öffnen entfernt
        s.db.execute("INSERT INTO matches (id, ts, queue, patch, data) VALUES (9, 0, 2400, '', '{\"id\":9,\"queue\":2400}')", []).unwrap();
        s.db.execute("INSERT INTO lobbies (game_id, ts, queue, data) VALUES (9, 0, 2400, '{\"game_id\":9,\"queue\":2400}')", []).unwrap();
        s.db.execute("INSERT INTO timelines (match_id, data) VALUES (9, '{\"match_id\":9}')", []).unwrap();
        drop(s);
        let s = Store::open_at(&dir);
        assert_eq!(s.count(Table::Matches), 1);
        assert_eq!((s.count(Table::Lobbies), s.count(Table::Timelines)), (0, 0));
        // Anzeige: Lobby/Ergebnis aus Sondermodi werden ausgeblendet, Summoner's Rift bleibt
        let st = LiveState {
            lobby: Some(Lobby { queue: 2400, ..Default::default() }),
            result: Some(PostGame { queue: 420, ..Default::default() }),
            ..Default::default()
        };
        let st = crate::analysis::without_off_modes(st);
        assert!(st.lobby.is_none() && st.result.is_some());
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lane_counts_match_full_recount() {
        let dir = temp_dir("lanes");
        let s = Store::open_at(&dir);
        let a = lane_match(1, [(10, "TOP", "SOLO"), (20, "JUNGLE", "NONE"), (30, "BOTTOM", "")]);
        let b = lane_match(2, [(10, "TOP", "SOLO"), (20, "MIDDLE", "SOLO"), (40, "BOTTOM", "DUO_SUPPORT")]);
        s.add_match(&a);
        s.add_match(&b);
        s.add_match(&a); // doppelt -> nicht doppelt gezählt
        // Neu-Laden mit Rollen-Angabe: alte Zählung raus, neue rein
        let a2 = lane_match(1, [(10, "TOP", "SOLO"), (20, "JUNGLE", "NONE"), (30, "BOTTOM", "DUO_CARRY")]);
        s.replace_match(&a2);
        let expect = crate::analysis::lane_freq(&[a2.clone(), b.clone()]);
        assert_eq!(s.lane_freq(), expect);
        assert_eq!(s.lane_freq()[&10][0], 2); // 2x Top
        assert_eq!(s.lane_freq()[&30][3], 1); // ADC erst nach dem Neu-Laden bekannt
        drop(s);
        // Tabelle leeren -> beim nächsten Öffnen einmalig aus den Matches neu aufgebaut
        let s = Store::open_at(&dir);
        s.db.execute("DELETE FROM lane_counts", []).unwrap();
        drop(s);
        let s = Store::open_at(&dir);
        assert_eq!(s.lane_freq(), expect);
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_jsonl_once() {
        let dir = temp_dir("mig");
        fs::create_dir_all(&dir).unwrap();
        let m = Match { id: 7, ts: 1, queue: 420, ..Default::default() };
        let snap = LpSnap { ts: 2, rank: Rank { tier: "IRON".into(), div: "I".into(), lp: 50, wins: 1, losses: 0 } };
        fs::write(dir.join("matches.jsonl"), format!("{}\nkaputte zeile\n", serde_json::to_string(&m).unwrap())).unwrap();
        fs::write(dir.join("lp.jsonl"), serde_json::to_string(&snap).unwrap() + "\n").unwrap();
        let s = Store::open_at(&dir);
        assert_eq!(s.count(Table::Matches), 1);
        assert_eq!(s.snaps()[0].rank.lp, 50);
        assert!(dir.join("matches.jsonl.bak").exists());
        assert!(!dir.join("matches.jsonl").exists());
        drop(s);
        // zweiter Start: nichts doppelt
        let s = Store::open_at(&dir);
        assert_eq!(s.count(Table::Matches), 1);
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
