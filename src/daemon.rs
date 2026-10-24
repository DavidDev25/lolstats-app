//! Hintergrund-Worker: pollt die Gameflow-Phase des Clients und reagiert darauf.
//! Client aus -> alle 10 s ein Prozess-Check, sonst alle 2 s (im Spiel 5 s) ein lokaler Request.

use crate::analysis::{self, LaneFreq};
use crate::lcu::{self, Lcu};
use crate::model::*;
use crate::stats;
use crate::store::{Store, Table};
use crate::tray;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

pub fn run(store: Store) {
    loop {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Worker::new(&store).run()));
        if r.is_err() {
            store.log("Worker abgestürzt, Neustart in 10 s");
        }
        sleep(Duration::from_secs(10));
    }
}

/// Einmal-Sync für die Kommandozeile (`lolstats sync`).
pub fn sync_once(store: &Store) -> String {
    let mut w = Worker::new(store);
    let Some(l) = w.ensure_lcu() else {
        return "League-Client läuft nicht.".into();
    };
    if !w.init_me(&l) {
        return "Client läuft, aber kein eingeloggter Account gefunden.".into();
    }
    format!(
        "OK: {} – {} Matches in der DB, Rang: {}",
        w.me.as_ref().map(|m| m.name.as_str()).unwrap_or("?"),
        w.match_count,
        w.last_snap.as_ref().map(|r| r.label()).unwrap_or_else(|| "?".into())
    )
}

/// Laufendes Spiel, dessen Ergebnis noch aussteht. Wird in pending.json gesichert,
/// damit ein Neustart (z. B. Update) mitten im Match das Ergebnis nicht verliert.
#[derive(Serialize, Deserialize)]
struct Pending {
    game_id: u64,
    queue: i32,
    champ: i32,
    pre: Option<Rank>,
    /// Zeitpunkt der Lobby-Erfassung (ms)
    #[serde(default)]
    ts: i64,
    #[serde(skip)]
    ended: Option<Instant>,
    #[serde(skip)]
    last_try: Option<Instant>,
}

/// Gesicherte Matches, die älter sind, gelten als verwaist (mehrere Spiele würden sich vermischen).
const PENDING_MAX_AGE_MS: i64 = 3 * 3600 * 1000;

struct Worker<'a> {
    store: &'a Store,
    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    lcu: Option<Lcu>,
    me: Option<Me>,
    names: Names,
    known: HashSet<u64>,
    match_count: usize,
    patch: String,
    patch_ts: i64,
    own_recent: VecDeque<Match>,
    /// (Match-ID, Elo des Spielers, über den es gefunden wurde)
    crawl_games: VecDeque<(u64, Option<i32>)>,
    /// eigene Spiele zum Nachladen: None = Match neu laden (alte Datenversion), Some = nur Timeline
    backfill_q: VecDeque<(Match, bool)>,
    crawl_players: VecDeque<String>,
    seen_players: HashSet<String>,
    phase: String,
    state: LiveState,
    cs_lobby: bool,
    /// Champion -> Lane-Häufigkeiten für die Lane-Erkennung; einmal aus der DB gezählt,
    /// danach bei jedem neuen Match ergänzt (kein erneutes Lesen aller Matches)
    lane_freq: Option<LaneFreq>,
    /// Stand der Match-Tabelle, zu dem `lane_freq` gelesen wurde
    lane_freq_ver: i64,
    lobby_game: u64,
    game_seen: Option<(u64, Instant)>,
    pending: Option<Pending>,
    last_snap: Option<Rank>,
    /// letzte Abfrage für den CS-Tacho
    last_cs: Option<Instant>,
    /// älterer eigener Verlauf: noch abzufragende Seiten (Start-Index) und gefundene, unbekannte Spiele
    own_pages: VecDeque<usize>,
    own_old: VecDeque<u64>,
    /// seit dem Start gecrawlte Matches (fürs Protokoll)
    crawled: usize,
    /// CS-Tacho im aktuellen Spiel: (Spiel, Abfragen, erfolgreiche)
    cs_stats: (u64, usize, usize),
    /// letzte Prüfung, ob die Update-Abfrage fällig ist
    last_update: Option<Instant>,
}

/// Unter so vielen wartenden Spielern füllen gecrawlte Spiele nahe deiner Elo die Warteschlange nach.
const CRAWL_REFILL: usize = 50;

/// Seitengröße beim Nachladen des eigenen Verlaufs.
const OWN_PAGE: usize = 50;

/// So viele eigene Spiele (neueste zuerst) werden beim Start auf Vollständigkeit geprüft und ggf. nachgeladen.
const BACKFILL_MAX: usize = 300;

/// Abstand der CS-Abfragen im Spiel.
const CS_EVERY: Duration = Duration::from_secs(15);

impl<'a> Worker<'a> {
    fn new(store: &'a Store) -> Self {
        let ms = store.matches();
        let me: Option<Me> = store.read(ME);
        let mut own: Vec<Match> = match &me {
            Some(me) => ms.iter().filter(|m| m.part(&me.puuid).is_some()).cloned().collect(),
            None => Vec::new(),
        };
        own.sort_by_key(|m| m.ts);
        // Bisherige eigene Spiele vervollständigen (neueste zuerst, max. BACKFILL_MAX):
        // alte Datenversion/fehlende Zusatzfelder -> Match neu laden, fehlende/alte Timeline -> nur Timeline
        let tl: HashMap<u64, u8> = store.timelines().iter().map(|t| (t.match_id, t.v)).collect();
        let my = me.as_ref().map(|m| m.puuid.as_str()).unwrap_or("");
        // (gespeichertes Match, true = neu laden; sonst nur Timeline)
        let backfill_q: VecDeque<(Match, bool)> = own
            .iter()
            .rev()
            .take(BACKFILL_MAX)
            .filter_map(|m| {
                if m.part(my).is_some_and(|p| p.v < PART_V || p.ext < PART_EXT) {
                    Some((m.clone(), true))
                } else if tl.get(&m.id).is_none_or(|&v| v < TIMELINE_V) {
                    Some((m.clone(), false))
                } else {
                    None
                }
            })
            .collect();
        if !backfill_q.is_empty() {
            store.log(&format!("{} eigene Spiele werden im Hintergrund vervollständigt", backfill_q.len()));
        }
        let own_recent: VecDeque<Match> = own.into_iter().rev().take(20).collect();
        let snaps = store.snaps();
        let pending: Option<Pending> = store.read::<Pending>(PENDING).filter(|p| now_ms() - p.ts < PENDING_MAX_AGE_MS);
        if let Some(p) = &pending {
            store.log(&format!("Laufendes Match {} nach Neustart übernommen", p.game_id));
        }
        Worker {
            store,
            cfg: Config::default(),
            cfg_mtime: None,
            lcu: None,
            me: None,
            names: store.read(NAMES).unwrap_or_default(),
            known: ms.iter().map(|m| m.id).collect(),
            match_count: ms.len(),
            patch: stats::latest_patch(&ms),
            patch_ts: ms.iter().map(|m| m.ts).max().unwrap_or(0),
            own_recent,
            crawl_games: VecDeque::new(),
            backfill_q,
            crawl_players: VecDeque::new(),
            seen_players: HashSet::new(),
            phase: String::new(),
            state: analysis::without_off_modes(store.read(STATE).unwrap_or_default()),
            cs_lobby: false,
            lane_freq: None,
            lane_freq_ver: -1,
            lobby_game: pending.as_ref().map_or(0, |p| p.game_id),
            game_seen: None,
            pending,
            last_snap: snaps.last().map(|s| s.rank.clone()),
            last_cs: None,
            // die ersten 50 holt init_me, danach Seite für Seite bis BACKFILL_MAX
            own_pages: (OWN_PAGE..BACKFILL_MAX).step_by(OWN_PAGE).collect(),
            own_old: VecDeque::new(),
            crawled: 0,
            cs_stats: (0, 0, 0),
            last_update: None,
        }
    }

    fn set_pending(&mut self, p: Option<Pending>) {
        match &p {
            Some(p) => self.store.write(PENDING, p),
            None => {
                let _ = std::fs::remove_file(self.store.p(PENDING));
            }
        }
        self.pending = p;
    }

    fn log(&self, m: &str) {
        self.store.log(m);
    }

    fn save_state(&mut self) {
        self.state.updated = now_ms();
        self.store.write(STATE, &self.state);
    }

    /// Öffentliche Version: alle 10 min nachsehen, ob die tägliche Update-Prüfung fällig ist (nie im Spiel).
    fn update_step(&mut self) {
        let busy = matches!(self.phase.as_str(), "ChampSelect" | "GameStart" | "InProgress" | "Reconnect");
        if busy || self.last_update.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) {
            return;
        }
        self.last_update = Some(Instant::now());
        if let Some(v) = crate::update::check_if_due(self.store) {
            self.log(&format!("Update verfügbar: {v} (installiert: {})", crate::update::VERSION));
            tray::balloon(tr!("lolstats-Update verfügbar", "lolstats update available"), &trf!("Version {v} – im Fenster unter Einstellungen installieren", "Version {v} – install it in the window under Settings"));
        }
    }

    /// Scraper-Stand für die Anzeige, höchstens alle 30 s geschrieben.
    fn update_crawl_status(&mut self) {
        let now = now_ms();
        if now - self.state.crawl.ts < 30_000 {
            return;
        }
        let s = CrawlStatus {
            crawled: self.crawled,
            players: self.crawl_players.len(),
            games: self.crawl_games.len(),
            own_left: self.backfill_q.len() + self.own_old.len(),
            ts: now,
        };
        let old = &self.state.crawl;
        if (s.crawled, s.players, s.games, s.own_left) != (old.crawled, old.players, old.games, old.own_left) {
            self.state.crawl = s;
            self.save_state();
        }
    }

    fn reload_cfg(&mut self) {
        let m = self.store.mtime(CONFIG);
        if m != self.cfg_mtime || m.is_none() {
            self.cfg = self.store.read(CONFIG).unwrap_or_default();
            if m.is_none() {
                self.store.write(CONFIG, &self.cfg);
            }
            self.cfg_mtime = self.store.mtime(CONFIG);
            // Sprache auch für Tray-Menü, Hinweise und Meldungen des Dienstes
            crate::i18n::apply(&self.cfg.lang);
        }
    }

    fn ensure_lcu(&mut self) -> Option<Lcu> {
        if self.lcu.is_none() {
            self.lcu = Lcu::connect(&self.cfg.client_dir);
            if self.lcu.is_some() {
                self.log("Client verbunden");
                self.me = None;
            }
        }
        self.lcu.clone()
    }

    fn set_phase(&mut self, p: &str) {
        if self.phase != p {
            self.phase = p.to_string();
            self.state.phase = p.to_string();
            self.save_state();
        }
    }

    fn run(&mut self) {
        loop {
            self.reload_cfg();
            self.update_step();
            let Some(l) = self.ensure_lcu() else {
                self.set_phase("Client aus");
                sleep(Duration::from_secs(10));
                continue;
            };
            if self.me.is_none() && !self.init_me(&l) {
                sleep(Duration::from_secs(5));
                continue;
            }
            let phase = match l.get("/lol-gameflow/v1/gameflow-phase") {
                Ok(v) => v.as_str().unwrap_or("None").to_string(),
                Err(e) => {
                    self.log(&format!("Client-Verbindung weg: {e}"));
                    self.lcu = None;
                    sleep(Duration::from_secs(3));
                    continue;
                }
            };
            if phase != self.phase {
                self.on_phase(&phase);
            }
            let ingame = matches!(phase.as_str(), "GameStart" | "InProgress" | "Reconnect");
            if self.store.p(REFRESH).exists() {
                let _ = std::fs::remove_file(self.store.p(REFRESH));
                let n = self.sync_own(&l, 20);
                if !ingame {
                    self.refresh_names(&l);
                }
                self.snapshot(&l);
                self.log(&format!("Manuell aktualisiert: {n} neue eigene Matches"));
            }
            if phase == "ChampSelect" {
                self.tick_champ_select(&l);
            } else if ingame {
                self.tick_game(&l);
                if self.last_cs.is_none_or(|t| t.elapsed() >= CS_EVERY) {
                    self.last_cs = Some(Instant::now());
                    self.tick_cs();
                }
            }
            let post = matches!(phase.as_str(), "WaitingForStats" | "PreEndOfGame");
            if self.pending.is_some() && !ingame && !post {
                self.try_finish(&l);
            }
            if !ingame && !post && phase != "ChampSelect" {
                self.backfill_step(&l);
                if self.cfg.crawl {
                    // im Leerlauf (kein Lobby/Queue) doppelt so schnell sammeln
                    let steps = if phase == "None" { 2 } else { 1 };
                    for _ in 0..steps {
                        self.crawl_step(&l);
                    }
                }
                self.update_crawl_status();
            }
            sleep(Duration::from_secs(if ingame { 5 } else { 2 }));
        }
    }

    fn on_phase(&mut self, phase: &str) {
        self.log(&format!("Phase {} -> {}", self.phase, phase));
        if phase == "ChampSelect" {
            self.cs_lobby = false;
            self.state.cs = None;
            self.state.note.clear();
            if self.cfg.auto_open_champselect {
                tray::request_open_gui();
            }
        }
        self.set_phase(phase);
    }

    fn init_me(&mut self, l: &Lcu) -> bool {
        let Ok(v) = l.get("/lol-summoner/v1/current-summoner") else {
            return false;
        };
        let puuid = js(&v, "puuid");
        if puuid.is_empty() {
            return false;
        }
        let mut name = riot_name(&v);
        if name.is_empty() {
            name = js(&v, "displayName");
        }
        let me = Me { puuid, name };
        self.log(&format!("Eingeloggt als {}", me.name));
        self.store.write(ME, &me);
        self.me = Some(me);
        self.refresh_names(l);
        let n = self.sync_own(l, 50);
        self.log(&format!("{n} neue eigene Matches"));
        if let Some(r) = self.snapshot(l) {
            tray::set_tip(&format!("lolstats · {}", r.label()));
        }
        true
    }

    fn refresh_names(&mut self, l: &Lcu) {
        let mut n = Names::default();
        if let Ok(v) = l.get("/lol-game-data/assets/v1/champion-summary.json") {
            for c in arr(&v) {
                if ji(c, "id") > 0 {
                    n.champs.insert(ji(c, "id"), js(c, "name"));
                    n.alias.insert(ji(c, "id"), js(c, "alias"));
                }
            }
        }
        if let Ok(v) = l.get("/lol-game-data/assets/v1/perks.json") {
            for p in arr(&v) {
                n.perks.insert(ji(p, "id"), js(p, "name"));
            }
        }
        if let Ok(v) = l.get("/lol-game-data/assets/v1/perkstyles.json") {
            for s in arr(&v["styles"]) {
                n.perks.insert(ji(s, "id"), js(s, "name"));
            }
        }
        if let Ok(v) = l.get("/lol-game-data/assets/v1/items.json") {
            for it in arr(&v) {
                // Fertiges Item = nicht weiter aufwertbar, ab 900 Gold. Schuhe der Stufe 2 lassen sich
                // zu Stufe 3 aufwerten, sind aber meist das Endprodukt -> Schuhe ab 900 Gold zählen immer.
                let boots = arr(&it["categories"]).iter().any(|c| c.as_str() == Some("Boots"));
                let done = ji(it, "priceTotal") >= 900 && (boots || arr(&it["to"]).is_empty());
                n.items.insert(ji(it, "id"), (js(it, "name"), done));
                if done && boots {
                    n.boots.push(ji(it, "id"));
                }
            }
        }
        if let Ok(v) = l.get("/lol-game-data/assets/v1/summoner-spells.json") {
            for s in arr(&v) {
                n.spells.insert(ji(s, "id"), js(s, "name"));
            }
        }
        // Spielstil-Werte pro Champion; bekannte aus dem Cache, nur neue Champions abfragen
        let ids: Vec<i32> = n.champs.keys().copied().collect();
        let mut fetched = 0;
        for id in ids {
            if let Some(i) = self.names.info.get(&id) {
                n.info.insert(id, *i);
            } else if let Ok(v) = l.get(&format!("/lol-game-data/assets/v1/champions/{id}.json")) {
                n.info.insert(id, ChampInfo::from_lcu(&v));
                fetched += 1;
            }
        }
        if fetched > 0 {
            self.log(&format!("Spielstil-Werte für {fetched} Champions geladen"));
        }
        if !n.champs.is_empty() {
            self.store.write(NAMES, &n);
            self.names = n;
        }
    }

    fn history(&self, l: &Lcu, puuid: &str, count: usize) -> Result<Value, String> {
        self.history_range(l, puuid, 0, count)
    }

    fn history_range(&self, l: &Lcu, puuid: &str, beg: usize, count: usize) -> Result<Value, String> {
        l.get(&format!(
            "/lol-match-history/v1/products/lol/{puuid}/matches?begIndex={beg}&endIndex={}",
            (beg + count).saturating_sub(1)
        ))
    }

    /// Älteren eigenen Verlauf nachladen: erst die Seite abfragen, dann ein Spiel pro Tick.
    /// Liefert true, wenn etwas zu tun war.
    fn own_history_step(&mut self, l: &Lcu) -> bool {
        if let Some(id) = self.own_old.pop_front() {
            let elo = self.last_snap.as_ref().and_then(|r| r.value());
            self.fetch_game(l, id, true, elo);
            if self.own_old.is_empty() && self.own_pages.is_empty() {
                self.log("Älterer eigener Verlauf vollständig geladen");
            }
            return true;
        }
        let Some(beg) = self.own_pages.pop_front() else { return false };
        let Some(me) = self.me.clone() else { return false };
        let Ok(v) = self.history_range(l, &me.puuid, beg, OWN_PAGE) else { return true };
        let games = arr(&v["games"]["games"]);
        let ids: Vec<u64> = games.iter().filter_map(|g| g["gameId"].as_u64()).filter(|id| !self.known.contains(id)).collect();
        self.log(&format!("Eigener Verlauf ab Spiel {beg}: {} Spiele, {} neu", games.len(), ids.len()));
        // Ende des Verlaufs erreicht oder nichts Neues (der Client liefert ältere Spiele dann nicht mehr): aufhören
        if games.len() < OWN_PAGE || ids.is_empty() {
            self.own_pages.clear();
        }
        self.own_old.extend(ids);
        true
    }

    /// Holt die eigenen letzten `n` Spiele und speichert unbekannte mit allen 10 Teilnehmern.
    fn sync_own(&mut self, l: &Lcu, n: usize) -> usize {
        let Some(me) = self.me.clone() else { return 0 };
        let Ok(v) = self.history(l, &me.puuid, n) else { return 0 };
        let elo = self.last_snap.as_ref().and_then(|r| r.value());
        let mut new = 0;
        for g in arr(&v["games"]["games"]) {
            if let Some(id) = g["gameId"].as_u64() {
                if !self.known.contains(&id) && self.fetch_game(l, id, true, elo) {
                    new += 1;
                }
            }
        }
        new
    }

    /// Lädt ein Match mit allen 10 Teilnehmern; `elo` = Elo des Spielers, über den es gefunden wurde.
    fn fetch_game(&mut self, l: &Lcu, id: u64, seed: bool, elo: Option<i32>) -> bool {
        let Some(mut m) = l.get(&format!("/lol-match-history/v1/games/{id}")).ok().and_then(|v| parse_game(&v)) else {
            return false;
        };
        if m.ps.len() < 2 {
            return false;
        }
        // Sondermodi (ARAM, Queue 2400 …) nicht speichern, aber als bekannt merken (kein erneutes Laden)
        if !analysis::queue_has_lanes(m.queue) {
            self.known.insert(id);
            return false;
        }
        m.elo = elo;
        self.store.add_match(&m); // zählt die Lanes in der Datenbank gleich mit
        self.known.insert(id);
        self.match_count += 1;
        if m.ts > self.patch_ts {
            self.patch_ts = m.ts;
            self.patch = m.patch.clone();
        }
        let my = self.me.as_ref().map(|x| x.puuid.clone()).unwrap_or_default();
        // Eigene Spiele liefern immer neue Spieler; gecrawlte nur, wenn die Warteschlange leerläuft und das
        // Spiel nahe deiner Elo lag (Schneeball, damit der Crawler nicht nach 300 Spielern stehen bleibt)
        let mine_elo = self.last_snap.as_ref().and_then(|r| r.value());
        let snowball = !seed && self.crawl_players.len() < CRAWL_REFILL && analysis::near_elo(&m, mine_elo, 400);
        if seed || snowball {
            for p in &m.ps {
                if p.puuid != my && !p.puuid.is_empty() && self.crawl_players.len() < 300 && self.seen_players.insert(p.puuid.clone()) {
                    self.crawl_players.push_back(p.puuid.clone());
                }
            }
        }
        if !seed {
            self.crawled += 1;
            if self.crawled % 50 == 0 {
                self.log(&format!(
                    "Crawler: {} Matches gesammelt (gesamt {} in der DB), {} Spieler und {} Spiele in der Warteschlange",
                    self.crawled,
                    self.match_count,
                    self.crawl_players.len(),
                    self.crawl_games.len()
                ));
            }
        }
        if m.part(&my).is_some() {
            self.fetch_timeline(l, &m);
            // nur neuere Spiele vorn einreihen (älterer Verlauf darf die letzten 20 nicht verdrängen)
            if self.own_recent.front().is_none_or(|f| m.ts >= f.ts) {
                self.own_recent.push_front(m);
                self.own_recent.truncate(20);
            }
        }
        true
    }

    /// Timeline eines eigenen Spiels inkl. CS des direkten Lane-Gegners.
    fn fetch_timeline(&mut self, l: &Lcu, m: &Match) {
        let my = self.me.as_ref().map(|x| x.puuid.clone()).unwrap_or_default();
        let Some(pid) = m.pid_of(&my) else { return };
        let opp = m.opponent(&my).and_then(|o| m.pid_of(&o.puuid));
        if let Ok(v) = l.get(&format!("/lol-match-history/v1/game-timelines/{}", m.id)) {
            if !arr(&v["frames"]).is_empty() {
                self.store.add_timeline(&Timeline::from_lcu(m.id, pid, opp, &v));
            }
        }
    }

    /// Ein eigenes Spiel pro Tick vervollständigen (nur außerhalb von Champ Select und Spiel).
    fn backfill_step(&mut self, l: &Lcu) {
        let Some((old, refetch)) = self.backfill_q.pop_front() else {
            self.own_history_step(l);
            return;
        };
        let m = if refetch {
            let Some(mut m) = l.get(&format!("/lol-match-history/v1/games/{}", old.id)).ok().and_then(|v| parse_game(&v)) else {
                return;
            };
            // Elo zum Zeitpunkt des Spiels bleibt erhalten (die Match-History kennt sie nicht)
            m.elo = old.elo;
            self.store.replace_match(&m);
            m
        } else {
            old
        };
        self.fetch_timeline(l, &m);
        if self.backfill_q.is_empty() {
            self.log("Eigene Spiele vollständig nachgeladen");
        }
    }

    /// Merkt sich gerankte Spiele (aktueller Patch) aus einer Match-History zum späteren Laden.
    fn queue_games(&mut self, v: &Value, elo: Option<i32>) {
        let mut added = 0;
        for g in arr(&v["games"]["games"]) {
            let Some(id) = g["gameId"].as_u64() else { continue };
            let q = ji(g, "queueId");
            let patch_ok = self.patch.is_empty() || patch_of(&js(g, "gameVersion")) == self.patch;
            if (q == 420 || q == 440)
                && patch_ok
                && !self.known.contains(&id)
                && !self.crawl_games.iter().any(|c| c.0 == id)
                && added < self.cfg.crawl_per_player
                && self.crawl_games.len() < 1000
            {
                self.crawl_games.push_back((id, elo));
                added += 1;
            }
        }
    }

    /// Ein Request pro Tick, nur außerhalb von Champ Select und Spiel.
    fn crawl_step(&mut self, l: &Lcu) {
        if self.match_count >= self.cfg.max_matches {
            return;
        }
        if let Some((id, elo)) = self.crawl_games.pop_front() {
            if !self.known.contains(&id) {
                self.fetch_game(l, id, false, elo);
            }
        } else if let Some(puuid) = self.crawl_players.pop_front() {
            let elo = self.fetch_rank(l, &puuid).and_then(|r| r.value());
            if let Ok(v) = self.history(l, &puuid, 20) {
                self.queue_games(&v, elo);
            }
        }
    }

    fn fetch_rank(&self, l: &Lcu, puuid: &str) -> Option<Rank> {
        Rank::from_lcu(&l.get(&format!("/lol-ranked/v1/ranked-stats/{puuid}")).ok()?)
    }

    /// Eigenen Rang holen und bei Änderung als LP-Snapshot speichern.
    fn snapshot(&mut self, l: &Lcu) -> Option<Rank> {
        let puuid = self.me.as_ref()?.puuid.clone();
        let v = l.get(&format!("/lol-ranked/v1/ranked-stats/{puuid}")).ok()?;
        let r = Rank::from_lcu(&v)?;
        let season = Season::from_lcu(&v);
        if self.store.read::<Season>(SEASON).as_ref() != Some(&season) {
            self.store.write(SEASON, &season);
        }
        if self.last_snap.as_ref() != Some(&r) {
            self.correct_late_lp(self.last_snap.clone(), &r);
            self.store.add_snap(&LpSnap { ts: now_ms(), rank: r.clone() });
            self.last_snap = Some(r.clone());
        }
        Some(r)
    }

    /// Kam die LP-Änderung erst nach dem gemeldeten Ergebnis (gleiche Spielanzahl, innerhalb von
    /// 30 min), wird das angezeigte Ergebnis nachträglich korrigiert.
    fn correct_late_lp(&mut self, prev: Option<Rank>, now: &Rank) {
        let (Some(prev), Some(res)) = (prev, self.state.result.as_mut()) else { return };
        let fresh = now_ms() - res.ts <= stats::LATE_LP_MS;
        if !fresh || res.queue != 420 || prev.games() != now.games() || !matches!(res.lp_delta, Some(0) | None) {
            return;
        }
        let (Some(a), Some(b)) = (prev.value(), now.value()) else { return };
        let d = b - a;
        if d == 0 || d.abs() > 60 {
            return;
        }
        res.lp_delta = Some(d);
        res.rank = now.label();
        let gid = res.game_id;
        self.log(&format!("Ergebnis {gid} nachträglich korrigiert: {d:+} LP (Riot hat verspätet abgerechnet)"));
        tray::balloon(tr!("LP nachträglich abgerechnet", "LP settled late"), &format!("{d:+} LP · {}", now.label()));
        self.save_state();
    }

    /// Rang + letzte 20 Spiele eines Mitspielers.
    fn fill_player(&mut self, l: &Lcu, p: &mut LivePlayer) {
        p.rank = self.fetch_rank(l, &p.puuid);
        let Ok(v) = self.history(l, &p.puuid, 20) else { return };
        let (mut g, mut w, mut k, mut d, mut a) = (0, 0, 0, 0, 0);
        let mut champs: HashMap<i32, (i32, i32)> = HashMap::new();
        for gm in arr(&v["games"]["games"]) {
            let Some(m) = parse_game(gm) else { continue };
            if m.remake() {
                continue;
            }
            let Some(x) = m.part(&p.puuid).or(m.ps.first()) else { continue };
            g += 1;
            w += x.win as i32;
            k += x.k;
            d += x.d;
            a += x.a;
            let c = champs.entry(x.champ).or_default();
            c.0 += 1;
            c.1 += x.win as i32;
            if p.name.is_empty() {
                p.name = x.name.clone();
            }
        }
        p.recent = (g, w);
        p.kda = if g > 0 { (k + a) as f32 / d.max(1) as f32 } else { 0.0 };
        p.champs = champs.into_iter().map(|(c, (g, w))| (c, g, w)).collect();
        if self.cfg.crawl {
            let elo = p.rank.as_ref().and_then(|r| r.value());
            self.queue_games(&v, elo);
        }
    }

    /// Lanes setzen (Client-Position, sonst Schätzung aus Champion-Statistik und Smite).
    fn fill_lanes(&mut self, players: &mut [LivePlayer], queue: i32) {
        // Die Datenbank führt die Zählung bei jedem gespeicherten Match mit; neu lesen nur, wenn sich
        // die Match-Tabelle geändert hat (kleine Tabelle, unabhängig von der Anzahl der Matches).
        let ver = self.store.version(Table::Matches);
        if self.lane_freq.is_none() || ver != self.lane_freq_ver {
            self.lane_freq = Some(self.store.lane_freq());
            self.lane_freq_ver = ver;
        }
        if let Some(f) = &self.lane_freq {
            analysis::fill_lanes(players, f, queue);
        }
    }

    /// Live-Check: Lane, Champion und Quelle pro Spieler ins Log (einmal pro Spiel, im Ladebildschirm).
    fn log_lanes(&self, players: &[LivePlayer]) {
        for team in [100, 200] {
            let mut ps: Vec<&LivePlayer> = players.iter().filter(|p| p.team == team).collect();
            ps.sort_by_key(|p| analysis::LANES.iter().position(|&l| l == p.lane).unwrap_or(9));
            let line: Vec<String> = ps
                .iter()
                .map(|p| {
                    let src = if !p.lane_guess {
                        "Client"
                    } else if p.smite == Some(true) && p.lane == "JUNGLE" {
                        "Smite"
                    } else {
                        "Statistik"
                    };
                    let lane = if p.lane.is_empty() { "?" } else { p.lane.as_str() };
                    format!("{lane} {} ({src})", self.names.champ(p.champ))
                })
                .collect();
            self.log(&format!("Lanes Team {team}: {}", line.join(", ")));
        }
    }

    /// Position und Smite aus der Live-Client-API (Ladebildschirm/Spiel), falls die Sitzung keine Position hat.
    fn live_lanes(&self, players: &mut [LivePlayer]) {
        let Ok(v) = lcu::live_get("/liveclientdata/playerlist") else { return };
        for lp in arr(&v) {
            let champ = self.names.champ_id(&js(lp, "championName"));
            let team = if js(lp, "team") == "CHAOS" { 200 } else { 100 };
            let Some(p) = players.iter_mut().find(|p| p.team == team && champ > 0 && p.champ == champ) else { continue };
            if p.position.is_empty() {
                p.position = js(lp, "position");
            }
            let spells = &lp["summonerSpells"];
            let raw = format!("{}{}", js(&spells["summonerSpellOne"], "rawDisplayName"), js(&spells["summonerSpellTwo"], "rawDisplayName"));
            if !raw.is_empty() {
                p.smite = Some(raw.contains("Smite"));
            }
        }
    }

    fn tick_champ_select(&mut self, l: &Lcu) {
        let Ok(s) = l.get("/lol-champ-select/v1/session") else { return };
        let cell = s["localPlayerCellId"].as_i64().unwrap_or(-1);
        let team = arr(&s["myTeam"]);
        // Gegner-Picks werden erst nach dem Lock sichtbar; Namen bleiben verborgen.
        let their = arr(&s["theirTeam"]);
        let champ_of = |p: &Value| {
            let c = ji(p, "championId");
            if c > 0 { c } else { ji(p, "championPickIntent") }
        };

        // Eigener Pick gelockt = abgeschlossene "pick"-Aktion der eigenen Zelle
        let locked = pick_locked(&s, cell);
        // Smite ist Zauber-ID 11; Gegner-Zauber sind in Champ Select verborgen (0)
        let smite_of = |p: &Value| {
            let (a, b) = (ji(p, "spell1Id"), ji(p, "spell2Id"));
            if a > 0 || b > 0 { Some(a == 11 || b == 11) } else { None }
        };
        if !self.cs_lobby {
            self.cs_lobby = true;
            let queue = l
                .get("/lol-gameflow/v1/session")
                .map(|v| ji(&v["gameData"]["queue"], "id"))
                .unwrap_or(0);
            if !analysis::queue_has_lanes(queue) {
                // Sondermodus: keine Lobby-Anzeige
                if self.state.lobby.take().is_some() {
                    self.save_state();
                }
                return;
            }
            let me = self.me.clone().unwrap_or_default();
            let mut players = Vec::new();
            for p in team {
                let is_me = p["cellId"].as_i64() == Some(cell);
                let mut lp = LivePlayer {
                    puuid: if is_me { me.puuid.clone() } else { js(p, "puuid") },
                    name: if is_me { me.name.clone() } else { String::new() },
                    champ: champ_of(p),
                    team: 100,
                    me: is_me,
                    position: js(p, "assignedPosition"),
                    smite: smite_of(p),
                    locked: is_me && locked,
                    cell: p["cellId"].as_i64(),
                    ..Default::default()
                };
                // Riot-Richtlinie: im Champ Select keine Mitspieler aufdecken – Rang/Verlauf der anderen
                // erst im Ladebildschirm, wenn Riot die Namen ohnehin zeigt
                let reveal = is_me;
                if lp.puuid.is_empty() || !reveal {
                    lp.name = if is_me { me.name.clone() } else { "(verborgen)".into() };
                    if !is_me {
                        lp.puuid.clear();
                    }
                } else {
                    self.fill_player(l, &mut lp);
                }
                players.push(lp);
            }
            for p in their {
                players.push(LivePlayer {
                    name: "(Gegner)".into(),
                    champ: ji(p, "championId"),
                    team: 200,
                    position: js(p, "assignedPosition"),
                    cell: p["cellId"].as_i64(),
                    ..Default::default()
                });
            }
            self.fill_lanes(&mut players, queue);
            self.state.lobby = Some(Lobby { game_id: 0, ts: now_ms(), queue, champ_select: true, players, ..Default::default() });
            self.save_state();
        } else {
            let mut changed = false;
            if let Some(lb) = self.state.lobby.as_mut() {
                // nach Zellen-ID zuordnen, nicht nach Reihenfolge (die ändert sich beim Tauschen)
                let mine = team.iter().map(|p| (p, champ_of(p), smite_of(p)));
                let enemies = their.iter().map(|p| (p, ji(p, "championId"), None));
                for (p, c, sm) in mine.chain(enemies) {
                    let Some(x) = lb.players.iter_mut().find(|x| x.cell.is_some() && x.cell == p["cellId"].as_i64()) else { continue };
                    let pos = js(p, "assignedPosition");
                    if x.champ != c || (sm.is_some() && x.smite != sm) || (!pos.is_empty() && x.position != pos) {
                        x.champ = c;
                        x.smite = sm.or(x.smite);
                        if !pos.is_empty() {
                            x.position = pos;
                        }
                        changed = true;
                    }
                }
                if let Some(p) = lb.players.iter_mut().find(|p| p.me) {
                    if p.locked != locked {
                        p.locked = locked;
                        changed = true;
                    }
                }
            }
            if changed {
                // neue Picks -> Lanes neu schätzen
                if let Some(mut lb) = self.state.lobby.take() {
                    self.fill_lanes(&mut lb.players, lb.queue);
                    self.state.lobby = Some(lb);
                }
                self.save_state();
            }
        }

    }


    /// CS-Tacho: eigener CS und der des Lane-Gegners aus der Live-Client-API (wie im Tab-Fenster sichtbar).
    fn tick_cs(&mut self) {
        // (Spiel, eigener Champion, Lane-Gegner) aus der erfassten Lobby
        let Some((gid, my_champ, opp)) = self.state.lobby.as_ref().and_then(|lb| {
            if lb.champ_select || lb.game_id == 0 {
                return None;
            }
            let me = lb.players.iter().find(|p| p.me && p.champ > 0)?;
            let opp = (!me.lane.is_empty())
                .then(|| lb.players.iter().find(|p| p.team != me.team && p.lane == me.lane && p.champ > 0))
                .flatten()
                .map_or(0, |p| p.champ);
            Some((lb.game_id, me.champ, opp))
        }) else {
            return;
        };
        if self.cs_stats.0 != gid {
            self.cs_stats = (gid, 0, 0);
        }
        self.cs_stats.1 += 1;
        let (Ok(list), Ok(stats)) = (lcu::live_get("/liveclientdata/playerlist"), lcu::live_get("/liveclientdata/gamestats")) else { return };
        let cs_of = |champ: i32| {
            arr(&list)
                .iter()
                .find(|lp| self.names.champ_id(&js(lp, "championName")) == champ)
                .and_then(|lp| lp["scores"]["creepScore"].as_i64())
                .map(|v| v as i32)
        };
        let Some(cs) = cs_of(my_champ) else { return };
        let live = LiveCs {
            game_id: gid,
            time: stats["gameTime"].as_f64().unwrap_or(0.0),
            champ: my_champ,
            cs,
            opp_champ: opp,
            opp_cs: (opp > 0).then(|| cs_of(opp)).flatten(),
            ts: now_ms(),
        };
        self.cs_stats.2 += 1;
        self.state.cs = Some(live);
        self.save_state();
    }

    /// Fehlende puuids über die Live-Client-API + Riot-ID-Suche auflösen.
    fn resolve_from_live(&mut self, l: &Lcu, players: &mut Vec<LivePlayer>) {
        let Ok(v) = lcu::live_get("/liveclientdata/playerlist") else { return };
        for lp in arr(&v) {
            let rid = riot_name(lp);
            let champ = self.names.champ_id(&js(lp, "championName"));
            let team = if js(lp, "team") == "CHAOS" { 200 } else { 100 };
            let idx = players.iter().position(|p| p.team == team && champ > 0 && p.champ == champ);
            let p = match idx {
                Some(i) => &mut players[i],
                None => {
                    players.push(LivePlayer { team, champ, ..Default::default() });
                    players.last_mut().unwrap()
                }
            };
            if p.name.is_empty() {
                p.name = rid.clone();
            }
            if p.puuid.is_empty() && !rid.is_empty() {
                if let Ok(s) = l.get(&format!("/lol-summoner/v1/summoners?name={}", lcu::pct(&rid))) {
                    p.puuid = js(&s, "puuid");
                }
            }
        }
    }

    /// Ladebildschirm/Spiel: einmal pro Spiel alle 10 Spieler einlesen.
    fn tick_game(&mut self, l: &Lcu) {
        let Ok(sess) = l.get("/lol-gameflow/v1/session") else { return };
        let gd = &sess["gameData"];
        let gid = gd["gameId"].as_u64().unwrap_or(0);
        if gid == 0 || gid == self.lobby_game {
            return;
        }
        let first_seen = match self.game_seen {
            Some((g, t)) if g == gid => t,
            _ => {
                self.game_seen = Some((gid, Instant::now()));
                Instant::now()
            }
        };
        let queue = ji(&gd["queue"], "id");
        // Sondermodus: nichts erfassen, nichts anzeigen, nicht abrechnen
        if !analysis::queue_has_lanes(queue) {
            self.lobby_game = gid;
            if self.state.lobby.take().is_some() {
                self.save_state();
            }
            self.log(&format!("Spiel {gid} ({}) ist ein Sondermodus – wird nicht erfasst", queue_name(queue)));
            return;
        }
        let mut players = Vec::new();
        for (team, key) in [(100, "teamOne"), (200, "teamTwo")] {
            for p in arr(&gd[key]) {
                players.push(LivePlayer {
                    puuid: js(p, "puuid"),
                    name: riot_name(p),
                    champ: ji(p, "championId"),
                    team,
                    position: js(p, "selectedPosition"),
                    ..Default::default()
                });
            }
        }
        if players.is_empty() || players.iter().any(|p| p.puuid.is_empty()) {
            self.resolve_from_live(l, &mut players);
        }
        let unresolved = players.iter().filter(|p| p.puuid.is_empty()).count();
        // 5v5: auf alle 10 warten (ein Spieler fehlt manchmal noch in den Sitzungsdaten, z. B. beim Laden)
        let incomplete = analysis::queue_has_lanes(queue) && players.len() < 10;
        if (players.len() < 2 || unresolved > 0 || incomplete) && first_seen.elapsed() < Duration::from_secs(90) {
            return; // nächster Tick, Live-API ist evtl. noch nicht bereit
        }
        let me = self.me.clone().unwrap_or_default();
        let pre = self.snapshot(l);
        let mut my_champ = 0;
        for p in players.iter_mut() {
            p.me = p.puuid == me.puuid;
            if p.me {
                my_champ = p.champ;
                if p.name.is_empty() {
                    p.name = me.name.clone();
                }
            }
            if !p.puuid.is_empty() {
                self.fill_player(l, p);
            }
        }
        self.live_lanes(&mut players);
        self.fill_lanes(&mut players, queue);
        if analysis::queue_has_lanes(queue) {
            self.log_lanes(&players);
        }
        let lobby = Lobby {
            game_id: gid,
            ts: now_ms(),
            queue,
            champ_select: false,
            players,
            remake: false,
        };
        let avg = lobby.avg_others().map(value_label).unwrap_or_else(|| "-".into());
        self.log(&format!("Lobby {gid} ({}) erfasst, Schnitt: {avg}", queue_name(queue)));
        self.store.add_lobby(&lobby);
        self.state.lobby = Some(lobby);
        self.save_state();
        self.lobby_game = gid;
        self.set_pending(Some(Pending { game_id: gid, queue, champ: my_champ, pre, ts: now_ms(), ended: None, last_try: None }));
    }


    /// Nach dem Spiel: auf LP-Update + Match-History warten, dann das Ergebnis melden.
    fn try_finish(&mut self, l: &Lcu) {
        // Sondermodus (z. B. von einer älteren Version erfasst): nicht abrechnen
        if self.pending.as_ref().is_some_and(|p| !analysis::queue_has_lanes(p.queue)) {
            let gid = self.pending.as_ref().map_or(0, |p| p.game_id);
            self.log(&format!("Spiel {gid} ist ein Sondermodus – wird nicht abgerechnet"));
            self.set_pending(None);
            return;
        }
        let Some(p) = self.pending.as_mut() else { return };
        if p.last_try.is_some_and(|t| t.elapsed() < Duration::from_secs(8)) {
            return;
        }
        p.last_try = Some(Instant::now());
        let ended = *p.ended.get_or_insert_with(Instant::now);
        let (gid, queue, champ, pre) = (p.game_id, p.queue, p.champ, p.pre.clone());

        self.sync_own(l, 5);
        let me = self.me.clone().unwrap_or_default();
        let my_match = self.own_recent.iter().find(|m| m.id == gid).cloned();
        let now = self.snapshot(l);
        let ranked = queue == 420;
        // Riot zählt das Spiel manchmal vor der LP-Änderung: gleiche LP nach einer Niederlage ist nur
        // mit Abstiegsschutz (0 LP vorher) echt, nach einem Sieg nie. Sonst auf den LP-Abzug warten.
        let lp_changed = matches!((&pre, &now), (Some(a), Some(b))
            if b.games() > a.games() && (a.value() != b.value() || (b.wins == a.wins && a.lp == 0)));
        // Remake: Riot zählt kein Spiel, auf eine LP-Änderung muss nicht gewartet werden.
        let remake = my_match.as_ref().is_some_and(|m| m.remake());
        let ready = my_match.is_some() && (!ranked || lp_changed || remake);
        if !ready && ended.elapsed() < Duration::from_secs(240) {
            return;
        }

        let win = my_match
            .as_ref()
            .and_then(|m| m.part(&me.puuid))
            .map(|p| p.win)
            .or_else(|| match (&pre, &now) {
                (Some(a), Some(b)) if lp_changed => Some(b.wins > a.wins),
                _ => None,
            });
        if remake {
            self.store.mark_remake(gid);
        }
        let delta = match (&pre, &now) {
            (Some(a), Some(b)) if lp_changed => Some(b.value().unwrap_or(0) - a.value().unwrap_or(0)),
            _ => None,
        };
        // Niederlage ohne LP-Abzug: häufig Riots Verlustschutz, weil ein Mitspieler AFK war
        if ranked && win == Some(false) && matches!(delta, None | Some(0)) {
            let afk = my_match.as_ref().and_then(|m| m.afk_teammate(&me.puuid));
            self.state.note = match afk {
                Some(p) => trf!(
                    "Verlustschutz: {} war vermutlich AFK ({} Gold, Level {}, {} CS) – deshalb kein LP-Abzug.", "Loss protection: {} was probably AFK ({} gold, level {}, {} CS) – that's why no LP were lost.",
                    self.names.champ(p.champ),
                    p.gold,
                    p.level,
                    p.cs
                ),
                None => tr!("Kein LP-Abzug trotz Niederlage (Riots Verlustschutz, Grund nicht erkennbar).", "No LP loss despite the defeat (Riot's loss protection, reason unknown).").into(),
            };
            self.log(&self.state.note.clone());
        }
        let rank = now.as_ref().map(|r| r.label()).unwrap_or_else(|| "Unranked".into());

        let title = if remake {
            "Remake"
        } else {
            match win {
                Some(true) => tr!("Sieg", "Victory"),
                Some(false) => tr!("Niederlage", "Defeat"),
                None => tr!("Spiel beendet", "Game over"),
            }
        };
        let mut body = String::new();
        if let Some(d) = delta {
            body += &format!("{d:+} LP");
            body += " · ";
        }
        body += &rank;
        tray::balloon(&format!("{title} · {}", queue_name(queue)), &body);
        tray::set_tip(&format!("lolstats · {rank}"));
        self.log(&format!("Ergebnis {gid}: {title}, Delta {delta:?}"));
        if self.cs_stats.0 == gid {
            let (_, n, ok) = self.cs_stats;
            self.log(&format!("CS-Tacho: {ok} von {n} Abfragen erfolgreich (Ziel: alle 15 s)"));
        }

        let champ = my_match
            .as_ref()
            .and_then(|m| m.part(&me.puuid))
            .map(|p| p.champ)
            .unwrap_or(champ);
        self.state.result = Some(PostGame {
            game_id: gid,
            ts: now_ms(),
            queue,
            champ,
            win,
            lp_delta: delta,
            rank,
            rank_before: pre.as_ref().map(|r| r.label()).unwrap_or_default(),
        });
        self.save_state();
        self.set_pending(None);
    }
}
