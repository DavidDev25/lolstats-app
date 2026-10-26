//! Datenmodell, Rang-Umrechnung und die lokale Dateiablage (%LOCALAPPDATA%\lolstats).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// Kleine JSON-Dateien im Datenordner; Matches, Lobbys und LP liegen in SQLite (store.rs).
pub const STATE: &str = "state.json";
pub const CONFIG: &str = "config.json";
pub const NAMES: &str = "names.json";
pub const ME: &str = "me.json";
pub const WINDOW: &str = "window.json";
/// Signaldatei: das Fenster bittet den Hintergrunddienst um einen sofortigen Abgleich.
pub const REFRESH: &str = "refresh.flag";
/// Laufendes Match, dessen Ergebnis noch aussteht (übersteht Neustarts).
pub const PENDING: &str = "pending.json";
pub const SEASON: &str = "season.json";

/// Letzte Fensterposition (logische Pixel), damit das Fenster an derselben Stelle wieder aufgeht.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(default)]
pub struct WinPos {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

// ---------- JSON-Helfer ----------

pub fn js(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn ji(v: &Value, k: &str) -> i32 {
    v.get(k).and_then(|x| x.as_i64()).unwrap_or(0) as i32
}

pub fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

/// Champ Select: hat die Zelle `cell` ihren Champion gelockt (abgeschlossene "pick"-Aktion)?
pub fn pick_locked(session: &Value, cell: i64) -> bool {
    arr(&session["actions"])
        .iter()
        .flat_map(|g| arr(g).iter())
        .any(|a| a["actorCellId"].as_i64() == Some(cell) && js(a, "type") == "pick" && a["completed"].as_bool() == Some(true))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn patch_of(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// "Name#TAG" aus den verschiedenen Feldvarianten der Client-APIs.
pub fn riot_name(p: &Value) -> String {
    for (g, t) in [("gameName", "tagLine"), ("riotIdGameName", "riotIdTagLine")] {
        let gn = js(p, g);
        if !gn.is_empty() {
            let tl = js(p, t);
            return if tl.is_empty() { gn } else { format!("{gn}#{tl}") };
        }
    }
    // versteckte Namen kommen als "#" bzw. leerer Name mit Tag – dann lieber gar keinen Namen
    let r = js(p, "riotId");
    if !r.trim_start_matches('#').is_empty() && !r.starts_with('#') {
        return r;
    }
    let s = js(p, "summonerName");
    if s.starts_with('#') { String::new() } else { s }
}

pub fn ago(ts_ms: i64) -> String {
    let s = ((now_ms() - ts_ms) / 1000).max(0);
    if s < 3600 {
        trf!("vor {} min", "{} min ago", s / 60)
    } else if s < 86400 {
        trf!("vor {} h", "{} h ago", s / 3600)
    } else {
        trf!("vor {} T", "{} d ago", s / 86400)
    }
}

/// UTC-Zeitstempel für das Log, ohne chrono.
pub fn fmt_utc(ms: i64) -> String {
    let secs = ms / 1000;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn queue_name(q: i32) -> String {
    match q {
        420 => "Solo/Duo".into(),
        440 => "Flex".into(),
        400 => "Normal Draft".into(),
        430 => "Blind".into(),
        490 => "Quickplay".into(),
        450 => "ARAM".into(),
        1700 => "Arena".into(),
        900 | 1900 => "URF".into(),
        0 => "Custom".into(),
        _ => format!("Queue {q}"),
    }
}

// ---------- Ränge ----------

pub const TIERS: [&str; 10] = [
    "IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "EMERALD", "DIAMOND", "MASTER", "GRANDMASTER",
    "CHALLENGER",
];
const DIVS: [&str; 4] = ["IV", "III", "II", "I"];

/// Rang auf eine lineare LP-Skala: Iron IV 0 LP = 0, jede Division 100, Master+ ab 2800.
pub fn rank_value(tier: &str, div: &str, lp: i32) -> Option<i32> {
    let t = TIERS.iter().position(|x| x.eq_ignore_ascii_case(tier))? as i32;
    if t >= 7 {
        return Some(2800 + lp.max(0));
    }
    let d = DIVS.iter().position(|x| *x == div).unwrap_or(0) as i32;
    Some(t * 400 + d * 100 + lp.clamp(0, 100))
}

pub fn tier_name(t: &str) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

pub fn div_label(v: i32) -> String {
    if v >= 2800 {
        return "Master+".into();
    }
    let v = v.max(0);
    format!("{} {}", tier_name(TIERS[(v / 400) as usize]), DIVS[((v % 400) / 100) as usize])
}

pub fn value_label(v: i32) -> String {
    if v >= 2800 {
        return format!("Master+ {} LP", v - 2800);
    }
    format!("{} {} LP", div_label(v), v.max(0) % 100)
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Rank {
    pub tier: String,
    pub div: String,
    pub lp: i32,
    pub wins: i32,
    pub losses: i32,
}

impl Rank {
    /// Solo/Duo-Eintrag aus /lol-ranked/v1/ranked-stats/{puuid}.
    pub fn from_lcu(v: &Value) -> Option<Rank> {
        let mut q = &v["queueMap"]["RANKED_SOLO_5x5"];
        if q.is_null() {
            q = arr(&v["queues"]).iter().find(|x| js(x, "queueType") == "RANKED_SOLO_5x5")?;
        }
        if q.is_null() {
            return None;
        }
        Some(Rank {
            tier: js(q, "tier"),
            div: js(q, "division"),
            lp: ji(q, "leaguePoints"),
            wins: ji(q, "wins"),
            losses: ji(q, "losses"),
        })
    }
    pub fn value(&self) -> Option<i32> {
        rank_value(&self.tier, &self.div, self.lp)
    }
    pub fn games(&self) -> i32 {
        self.wins + self.losses
    }
    pub fn label(&self) -> String {
        match self.value() {
            None => "Unranked".into(),
            Some(v) if v >= 2800 => format!("{} {} LP", tier_name(&self.tier), self.lp),
            Some(_) => format!("{} {} {} LP", tier_name(&self.tier), self.div, self.lp),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct LpSnap {
    pub ts: i64,
    pub rank: Rank,
}

// ---------- Matches ----------

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Part {
    pub puuid: String,
    pub name: String,
    pub champ: i32,
    pub team: i32,
    pub win: bool,
    pub k: i32,
    pub d: i32,
    pub a: i32,
    pub cs: i32,
    pub gold: i32,
    pub dmg: i32,
    pub items: [i32; 7],
    pub perks: [i32; 6],
    pub shards: [i32; 3],
    pub styles: [i32; 2],
    pub spells: [i32; 2],
    pub lane: String,
    /// participantId im Spiel (1..10), für die Timeline
    pub pid: i32,
    // --- ab Datenversion 2 ---
    /// 0/1 = alte Einträge ohne die folgenden Felder, 2 = vollständig
    pub v: u8,
    pub role: String,
    pub lane_cs: i32,
    pub jungle_cs: i32,
    pub vision: i32,
    pub wards: i32,
    pub wards_killed: i32,
    pub dmg_taken: i32,
    pub dmg_obj: i32,
    pub cc_time: i32,
    pub heal: i32,
    pub level: i32,
    pub first_blood: bool,
    pub doubles: i32,
    pub triples: i32,
    pub quadras: i32,
    pub pentas: i32,
    pub spree: i32,
    // --- ab Erweiterung 1 (ext >= 1) ---
    /// 1 = First-Blood-Assist und erster Turm sind erfasst (0 = ältere Einträge, Werte unbekannt)
    pub ext: u8,
    pub fb_assist: bool,
    /// am ersten Turm des Spiels beteiligt (Kill oder Assist)
    pub first_tower: bool,
}

/// Stand der Zusatzfelder (`Part::ext`); ältere eigene Spiele werden im Hintergrund nachgeladen.
pub const PART_EXT: u8 = 1;

/// Aktuelle Datenversion von `Part`.
pub const PART_V: u8 = 2;
/// Beschwörerzauber-ID von Smite
pub const SMITE: i32 = 11;

impl Part {
    /// Position aus Lane und Rolle der Match-History: TOP, JUNGLE, MID, ADC, SUPPORT ("" = unbekannt).
    /// Smite ist der sicherste Jungle-Beleg; Riot markiert ohne Smite oft fälschlich Jungle.
    /// Bot-Rollen kommen heute als CARRY/SUPPORT (früher DUO_CARRY/DUO_SUPPORT), Supports oft mit Lane NONE.
    pub fn position(&self) -> &'static str {
        if self.spells.contains(&SMITE) {
            return "JUNGLE";
        }
        match (self.lane.as_str(), self.role.as_str()) {
            (_, "SUPPORT" | "DUO_SUPPORT") => "SUPPORT",
            ("BOTTOM", "CARRY" | "DUO_CARRY") => "ADC",
            ("TOP", _) => "TOP",
            ("MIDDLE" | "MID", _) => "MID",
            _ => "",
        }
    }
}

pub fn position_label(p: &str) -> &'static str {
    match p {
        "TOP" | "top" => "Top",
        "JUNGLE" | "jungle" => "Jungle",
        "MID" | "middle" => "Mid",
        "ADC" | "bottom" => "ADC",
        "SUPPORT" | "utility" => "Support",
        _ => "?",
    }
}

/// Champ-Select-Position ("top", "middle", "utility", ...) auf die Match-Positionen abbilden.
/// Position aus Champ Select ("middle"), Spielsitzung ("MIDDLE") oder Live-API ("UTILITY") -> "TOP"/"JUNGLE"/"MID"/"ADC"/"SUPPORT".
pub fn position_from_select(p: &str) -> &'static str {
    match p {
        "top" | "TOP" => "TOP",
        "jungle" | "JUNGLE" => "JUNGLE",
        "middle" | "MIDDLE" | "MID" => "MID",
        "bottom" | "BOTTOM" | "ADC" => "ADC",
        "utility" | "UTILITY" | "SUPPORT" => "SUPPORT",
        _ => "",
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Match {
    pub id: u64,
    pub ts: i64,
    pub queue: i32,
    pub dur: i32,
    pub patch: String,
    pub ps: Vec<Part>,
    /// Elo (LP-Skala) des Spielers, über den das Match gesammelt wurde; None = unbekannt
    pub elo: Option<i32>,
}

impl Match {
    pub fn part(&self, puuid: &str) -> Option<&Part> {
        self.ps.iter().find(|p| p.puuid == puuid)
    }
    pub fn remake(&self) -> bool {
        self.dur > 0 && self.dur < 300
    }
    /// participantId eines Spielers; alte Einträge ohne pid sind in Reihenfolge 1..10 gespeichert.
    pub fn pid_of(&self, puuid: &str) -> Option<i32> {
        let i = self.ps.iter().position(|p| p.puuid == puuid)?;
        Some(if self.ps[i].pid > 0 { self.ps[i].pid } else { i as i32 + 1 })
    }
    /// Direkter Lane-Gegner (gleiche Position im anderen Team, eindeutig).
    pub fn opponent(&self, puuid: &str) -> Option<&Part> {
        let me = self.part(puuid)?;
        let pos = me.position();
        if pos.is_empty() {
            return None;
        }
        let mut it = self.ps.iter().filter(|p| p.team != me.team && p.position() == pos);
        let first = it.next()?;
        it.next().is_none().then_some(first)
    }
    /// Mitspieler, der vermutlich AFK war oder das Spiel verlassen hat:
    /// weniger als die Hälfte des Golds im Schnitt der anderen Teammitglieder.
    pub fn afk_teammate(&self, puuid: &str) -> Option<&Part> {
        let me = self.part(puuid)?;
        let team: Vec<&Part> = self.ps.iter().filter(|p| p.team == me.team && p.puuid != puuid).collect();
        team.iter()
            .copied()
            .filter(|p| {
                let others: Vec<i32> = team.iter().filter(|q| q.puuid != p.puuid).map(|q| q.gold).collect();
                let avg = others.iter().sum::<i32>() as f64 / others.len().max(1) as f64;
                avg > 0.0 && (p.gold as f64) < avg * 0.5
            })
            .min_by_key(|p| p.gold)
    }
    pub fn team_sum(&self, team: i32, f: impl Fn(&Part) -> i32) -> i32 {
        self.ps.iter().filter(|p| p.team == team).map(f).sum()
    }
}

/// Parst ein Spiel aus /lol-match-history (Detail = 10 Teilnehmer, Liste = nur der Spieler selbst).
pub fn parse_game(v: &Value) -> Option<Match> {
    let id = v.get("gameId")?.as_u64()?;
    let mut ids: HashMap<i64, (String, String)> = HashMap::new();
    for pi in arr(&v["participantIdentities"]) {
        let p = &pi["player"];
        ids.insert(
            pi["participantId"].as_i64().unwrap_or(0),
            (js(p, "puuid"), riot_name(p)),
        );
    }
    let mut ps = Vec::new();
    for p in arr(&v["participants"]) {
        let st = &p["stats"];
        let (puuid, name) = ids
            .get(&p["participantId"].as_i64().unwrap_or(0))
            .cloned()
            .unwrap_or_default();
        ps.push(Part {
            puuid,
            name,
            champ: ji(p, "championId"),
            team: ji(p, "teamId"),
            win: st["win"].as_bool().unwrap_or(false),
            k: ji(st, "kills"),
            d: ji(st, "deaths"),
            a: ji(st, "assists"),
            cs: ji(st, "totalMinionsKilled") + ji(st, "neutralMinionsKilled"),
            gold: ji(st, "goldEarned"),
            dmg: ji(st, "totalDamageDealtToChampions"),
            items: std::array::from_fn(|n| ji(st, &format!("item{n}"))),
            perks: std::array::from_fn(|n| ji(st, &format!("perk{n}"))),
            shards: std::array::from_fn(|n| ji(st, &format!("statPerk{n}"))),
            styles: [ji(st, "perkPrimaryStyle"), ji(st, "perkSubStyle")],
            spells: [ji(p, "spell1Id"), ji(p, "spell2Id")],
            lane: js(&p["timeline"], "lane"),
            pid: ji(p, "participantId"),
            v: PART_V,
            role: js(&p["timeline"], "role"),
            lane_cs: ji(st, "totalMinionsKilled"),
            jungle_cs: ji(st, "neutralMinionsKilled"),
            vision: ji(st, "visionScore"),
            wards: ji(st, "wardsPlaced"),
            wards_killed: ji(st, "wardsKilled"),
            dmg_taken: ji(st, "totalDamageTaken"),
            dmg_obj: ji(st, "damageDealtToObjectives"),
            cc_time: ji(st, "timeCCingOthers"),
            heal: ji(st, "totalHeal"),
            level: ji(st, "champLevel"),
            first_blood: st["firstBloodKill"].as_bool().unwrap_or(false),
            doubles: ji(st, "doubleKills"),
            triples: ji(st, "tripleKills"),
            quadras: ji(st, "quadraKills"),
            pentas: ji(st, "pentaKills"),
            spree: ji(st, "largestKillingSpree"),
            ext: PART_EXT,
            fb_assist: st["firstBloodAssist"].as_bool().unwrap_or(false),
            first_tower: st["firstTowerKill"].as_bool().unwrap_or(false) || st["firstTowerAssist"].as_bool().unwrap_or(false),
        });
    }
    Some(Match {
        id,
        ts: v["gameCreation"].as_i64().unwrap_or(0),
        queue: ji(v, "queueId"),
        dur: ji(v, "gameDuration"),
        patch: patch_of(&js(v, "gameVersion")),
        ps,
        elo: None,
    })
}

// ---------- Live-Lobby / Zustand ----------

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct LivePlayer {
    pub puuid: String,
    pub name: String,
    pub champ: i32,
    pub team: i32,
    pub me: bool,
    pub rank: Option<Rank>,
    /// (Spiele, Siege) der letzten 20
    pub recent: (i32, i32),
    /// (Champion, Spiele, Siege) innerhalb der letzten 20
    pub champs: Vec<(i32, i32, i32)>,
    pub kda: f32,
    /// zugewiesene Position aus Champ Select ("top", "jungle", "middle", "bottom", "utility")
    pub position: String,
    /// Lane ("TOP", "JUNGLE", "MID", "ADC", "SUPPORT"), vom Client oder geschätzt
    pub lane: String,
    /// true = Lane geschätzt (Champion-Statistik/Smite), nicht vom Client
    pub lane_guess: bool,
    /// Champ Select: eigener Pick ist gelockt (nicht nur gehovert)
    pub locked: bool,
    /// Champ Select: Zellen-ID; die Reihenfolge der Spieler ändert sich beim Tauschen, die Zelle nicht
    pub cell: Option<i64>,
    /// hat Smite (nur während der Erfassung bekannt)
    #[serde(skip)]
    pub smite: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Lobby {
    pub game_id: u64,
    pub ts: i64,
    pub queue: i32,
    pub champ_select: bool,
    pub players: Vec<LivePlayer>,
    /// Spiel wurde geremaked (unter 5 min) – zählt nicht als Matchmaking-Messpunkt
    pub remake: bool,
    /// Riots LP-Meldung nach dem Spiel (Bonus wie Autofill-Schutz, Verlustschutz)
    pub lp_note: Option<LpNote>,
    /// im Champ Select erkannt: zugewiesene Position war keine Wunschposition
    pub autofill: bool,
}

/// Niederlage auf genau 0 LP mit weniger Abzug als das gilt als gekappt (Abstiegsschutz);
/// 20 -> 0 ist dagegen eine volle Niederlage.
pub const CAPPED_LOSS: i32 = 15;

/// Aus /lol-ranked/v1/current-lp-change-notification: warum die LP vom Normalfall abweichen.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct LpNote {
    pub delta: i32,
    /// lpBonusAppliedReason, z. B. "AUTOFILL_AEGIS" (leer = kein Bonus)
    pub bonus: String,
    /// consolationLpUsed: LP, die Riot beim Verlust erlassen hat (AFK-Mitspieler)
    pub consolation: i32,
    /// du selbst warst AFK/Leaver
    pub was_afk: bool,
}

impl LpNote {
    pub fn from_lcu(v: &serde_json::Value) -> Option<(u64, LpNote)> {
        let gid = v["gameId"].as_u64()?;
        Some((
            gid,
            LpNote {
                delta: v["leaguePointsDelta"].as_i64().unwrap_or(0) as i32,
                bonus: v["lpBonusAppliedReason"].as_str().unwrap_or("").to_string(),
                consolation: v["consolationLpUsed"].as_i64().unwrap_or(0) as i32,
                was_afk: v["wasAfkOrLeaver"].as_bool().unwrap_or(false),
            },
        ))
    }

    /// LP sind durch einen Riot-Bonus/-Schutz verändert -> kein normales Spiel für LP-Auswertungen
    pub fn modified(&self) -> bool {
        !self.bonus.is_empty() || self.consolation > 0
    }
}

impl Lobby {
    /// Durchschnittlicher Solo-Rang der anderen Spieler (mind. 3 gerankte).
    pub fn avg_others(&self) -> Option<i32> {
        let v: Vec<i32> = self
            .players
            .iter()
            .filter(|p| !p.me)
            .filter_map(|p| p.rank.as_ref()?.value())
            .collect();
        if v.len() < 3 {
            return None;
        }
        Some(v.iter().sum::<i32>() / v.len() as i32)
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct PostGame {
    pub game_id: u64,
    pub ts: i64,
    pub queue: i32,
    pub champ: i32,
    pub win: Option<bool>,
    pub lp_delta: Option<i32>,
    pub rank: String,
    /// Rang vor dem Spiel (für "vorher -> nachher" in der Anzeige)
    pub rank_before: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct LiveState {
    pub phase: String,
    pub updated: i64,
    pub lobby: Option<Lobby>,
    pub result: Option<PostGame>,
    pub note: String,
    /// CS-Tacho im laufenden Spiel (alle ~15 s aus der Live-Client-API)
    pub cs: Option<LiveCs>,
    /// Stand von Scraper und Nachladen für die Anzeige
    pub crawl: CrawlStatus,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct CrawlStatus {
    /// seit dem Start des Dienstes gesammelte fremde Matches
    pub crawled: usize,
    /// wartende Spieler bzw. Spiele
    pub players: usize,
    pub games: usize,
    /// noch nachzuladende eigene Spiele
    pub own_left: usize,
    /// Zeitpunkt der letzten Aktualisierung (ms)
    pub ts: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct LiveCs {
    pub game_id: u64,
    /// Spielzeit in Sekunden
    pub time: f64,
    pub champ: i32,
    pub cs: i32,
    /// direkter Lane-Gegner, falls bekannt
    pub opp_champ: i32,
    pub opp_cs: Option<i32>,
    /// Zeitpunkt der Abfrage (ms)
    pub ts: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Me {
    pub puuid: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub crawl: bool,
    pub auto_open_champselect: bool,
    pub client_dir: String,
    pub crawl_per_player: usize,
    pub max_matches: usize,
    /// Expertenmodus: Labor, Formeln und alle Tabellen; sonst nur die Kernaussagen
    pub expert: bool,
    /// Tutorial wurde beim ersten Start schon gezeigt
    pub tutorial_seen: bool,
    /// Sprache der Oberfläche: "de", "en" oder leer = wie Windows
    pub lang: String,
    /// Felder, die diese Version nicht kennt, beim Speichern unverändert behalten
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

pub const WINDOW_TITLE: &str = "lolstats";
pub const EDITION: &str = "öffentlich";

impl Default for Config {
    fn default() -> Self {
        Config {
            crawl: true,
            auto_open_champselect: false,
            client_dir: String::new(),
            crawl_per_player: 10,
            max_matches: 20000,
            expert: false,
            tutorial_seen: false,
            lang: String::new(),
            other: Default::default(),
        }
    }
}

/// Namen aus den lokalen Spieldaten des Clients (/lol-game-data), gecacht in names.json.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Names {
    pub champs: HashMap<i32, String>,
    pub perks: HashMap<i32, String>,
    /// id -> (Name, fertiges Item)
    pub items: HashMap<i32, (String, bool)>,
    /// Item-IDs der aufgewerteten Schuhe (Kategorie "Boots", ab 900 Gold)
    pub boots: Vec<i32>,
    pub spells: HashMap<i32, String>,
    /// Champion-ID -> Data-Dragon-Name für Bilder (z. B. 62 -> "MonkeyKing")
    pub alias: HashMap<i32, String>,
    /// Spielstil-Werte pro Champion aus /lol-game-data (für Team-Kompositionen)
    pub info: HashMap<i32, ChampInfo>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChampInfo {
    /// "kPhysical" -> Ad, "kMagic" -> Ap, sonst Mixed
    pub dmg: DmgType,
    /// jeweils 0..3 laut Client
    pub damage: u8,
    pub durability: u8,
    pub cc: u8,
    pub mobility: u8,
    pub utility: u8,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DmgType {
    Ad,
    Ap,
    #[default]
    Mixed,
}

impl ChampInfo {
    pub fn from_lcu(v: &Value) -> ChampInfo {
        let ps = &v["playstyleInfo"];
        let u = |k: &str| ji(ps, k).clamp(0, 3) as u8;
        ChampInfo {
            dmg: match js(&v["tacticalInfo"], "damageType").as_str() {
                "kPhysical" => DmgType::Ad,
                "kMagic" => DmgType::Ap,
                _ => DmgType::Mixed,
            },
            damage: u("damage"),
            durability: u("durability"),
            cc: u("crowdControl"),
            mobility: u("mobility"),
            utility: u("utility"),
        }
    }
}

/// Auszug aus der Timeline eines eigenen Spiels.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Timeline {
    pub match_id: u64,
    /// Zeitpunkte der eigenen Tode in Sekunden
    pub deaths: Vec<i32>,
    pub cs10: Option<i32>,
    pub gold10: Option<i32>,
    // --- ab Version 2 ---
    pub v: u8,
    pub cs15: Option<i32>,
    pub cs20: Option<i32>,
    pub gold15: Option<i32>,
    /// CS des direkten Lane-Gegners
    pub opp_cs10: Option<i32>,
    pub opp_cs15: Option<i32>,
}

pub const TIMELINE_V: u8 = 2;

/// (CS, Gold) eines Teilnehmers in einem Timeline-Frame.
fn frame_cs(f: &Value, pid: i32) -> Option<(i32, i32)> {
    let pf = &f["participantFrames"][pid.to_string()];
    if pf.is_null() {
        return None;
    }
    Some((ji(pf, "minionsKilled") + ji(pf, "jungleMinionsKilled"), ji(pf, "totalGold")))
}

impl Timeline {
    /// Liest /lol-match-history/v1/game-timelines/{id} für den Teilnehmer `pid` (und optional seinen Lane-Gegner).
    pub fn from_lcu(match_id: u64, pid: i32, opp: Option<i32>, v: &Value) -> Timeline {
        let mut t = Timeline { match_id, v: TIMELINE_V, ..Default::default() };
        for f in arr(&v["frames"]) {
            let minute = f["timestamp"].as_i64().unwrap_or(0) / 60_000;
            let me = frame_cs(f, pid);
            let op = opp.and_then(|o| frame_cs(f, o)).map(|x| x.0);
            match minute {
                10 if t.cs10.is_none() && me.is_some() => {
                    t.cs10 = me.map(|x| x.0);
                    t.gold10 = me.map(|x| x.1);
                    t.opp_cs10 = op;
                }
                15 if t.cs15.is_none() && me.is_some() => {
                    t.cs15 = me.map(|x| x.0);
                    t.gold15 = me.map(|x| x.1);
                    t.opp_cs15 = op;
                }
                20 if t.cs20.is_none() && me.is_some() => t.cs20 = me.map(|x| x.0),
                _ => {}
            }
            for e in arr(&f["events"]) {
                if js(e, "type") == "CHAMPION_KILL" && ji(e, "victimId") == pid {
                    t.deaths.push((e["timestamp"].as_i64().unwrap_or(0) / 1000) as i32);
                }
            }
        }
        t
    }
}

/// Saison-Hochs aus /lol-ranked/v1/ranked-stats (Solo/Duo).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Season {
    pub high: Option<i32>,
    pub prev_high: Option<i32>,
    pub prev_end: Option<i32>,
}

impl Season {
    pub fn from_lcu(v: &Value) -> Season {
        let mut q = &v["queueMap"]["RANKED_SOLO_5x5"];
        if q.is_null() {
            q = arr(&v["queues"]).iter().find(|x| js(x, "queueType") == "RANKED_SOLO_5x5").unwrap_or(&Value::Null);
        }
        let r = |t: &str, d: &str| rank_value(&js(q, t), &js(q, d), 0);
        Season {
            high: r("highestTier", "highestDivision"),
            prev_high: r("previousSeasonHighestTier", "previousSeasonHighestDivision"),
            prev_end: r("previousSeasonEndTier", "previousSeasonEndDivision"),
        }
    }
}

impl Names {
    fn get(map: &HashMap<i32, String>, id: i32) -> String {
        if id <= 0 {
            return "-".into();
        }
        map.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))
    }
    pub fn champ(&self, id: i32) -> String {
        Self::get(&self.champs, id)
    }
    pub fn perk(&self, id: i32) -> String {
        Self::get(&self.perks, id)
    }
    pub fn spell(&self, id: i32) -> String {
        Self::get(&self.spells, id)
    }
    pub fn item(&self, id: i32) -> String {
        self.items.get(&id).map(|x| x.0.clone()).unwrap_or_else(|| format!("#{id}"))
    }
    pub fn completed(&self, id: i32) -> bool {
        self.items.get(&id).map(|x| x.1).unwrap_or(false)
    }
    pub fn is_boots(&self, id: i32) -> bool {
        self.boots.contains(&id)
    }
    pub fn champ_id(&self, name: &str) -> i32 {
        let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let n = norm(name);
        // Sondermodi führen Champions doppelt (z. B. Galio 3 und 60003): immer die echte, kleinste ID
        self.champs
            .iter()
            .filter(|(_, v)| norm(v) == n)
            .map(|(k, _)| *k)
            .min()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_roundtrip() {
        assert_eq!(rank_value("IRON", "IV", 0), Some(0));
        assert_eq!(rank_value("GOLD", "II", 54), Some(3 * 400 + 200 + 54));
        assert_eq!(rank_value("MASTER", "I", 120), Some(2920));
        assert_eq!(rank_value("", "", 0), None);
        assert_eq!(value_label(1454), "Gold II 54 LP");
        assert_eq!(value_label(2920), "Master+ 120 LP");
        assert_eq!(div_label(2399), "Emerald I");
    }

    #[test]
    fn rank_from_lcu() {
        let v: Value = serde_json::from_str(
            r#"{"queueMap":{"RANKED_SOLO_5x5":{"tier":"EMERALD","division":"III","leaguePoints":40,"wins":30,"losses":25}}}"#,
        )
        .unwrap();
        let r = Rank::from_lcu(&v).unwrap();
        assert_eq!(r.label(), "Emerald III 40 LP");
        assert_eq!(r.games(), 55);
        let un: Value = serde_json::from_str(
            r#"{"queueMap":{"RANKED_SOLO_5x5":{"tier":"","division":"NA","leaguePoints":0,"wins":0,"losses":0}}}"#,
        )
        .unwrap();
        assert_eq!(Rank::from_lcu(&un).unwrap().label(), "Unranked");
    }

    #[test]
    fn parses_game_detail() {
        let v: Value = serde_json::from_str(
            r#"{"gameId":7001,"gameCreation":1700000000000,"gameDuration":1800,"queueId":420,"gameVersion":"14.20.123.4567",
            "participantIdentities":[{"participantId":1,"player":{"puuid":"aaa","gameName":"Me","tagLine":"EUW"}},
                                     {"participantId":2,"player":{"puuid":"bbb","summonerName":"Old"}}],
            "participants":[{"participantId":1,"teamId":100,"championId":145,"spell1Id":4,"spell2Id":7,
                "stats":{"win":true,"kills":8,"deaths":2,"assists":6,"totalMinionsKilled":200,"neutralMinionsKilled":10,
                "item0":3031,"item1":6672,"perk0":8005,"perk1":9111,"perk2":9104,"perk3":8014,"perk4":8139,"perk5":8135,
                "perkPrimaryStyle":8000,"perkSubStyle":8100,"firstBloodAssist":true,"firstTowerAssist":true},"timeline":{"lane":"BOTTOM"}},
                {"participantId":2,"teamId":200,"championId":222,"stats":{"win":false,"firstBloodKill":true}}]}"#,
        )
        .unwrap();
        let m = parse_game(&v).unwrap();
        assert_eq!(m.id, 7001);
        assert_eq!(m.patch, "14.20");
        assert_eq!(m.ps.len(), 2);
        let me = m.part("aaa").unwrap();
        assert_eq!(me.name, "Me#EUW");
        assert_eq!(me.cs, 210);
        assert_eq!(me.items[1], 6672);
        assert_eq!(me.styles, [8000, 8100]);
        assert_eq!(me.lane, "BOTTOM");
        assert_eq!(m.part("bbb").unwrap().name, "Old");
        assert!(!m.remake());
        // Zusatzfelder: First-Blood-Assist und Turm-Assist zählen als Beteiligung
        assert_eq!(me.ext, PART_EXT);
        assert!(me.fb_assist && !me.first_blood && me.first_tower);
        let other = m.part("bbb").unwrap();
        assert!(other.first_blood && !other.fb_assist && !other.first_tower);
    }

    #[test]
    fn utc_format() {
        assert_eq!(fmt_utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(fmt_utc(1_700_000_000_000), "2023-11-14 22:13:20Z");
    }

    #[test]
    fn positions_from_current_match_history() {
        let p = |lane: &str, role: &str, smite: bool| Part {
            lane: lane.into(),
            role: role.into(),
            spells: if smite { [4, SMITE] } else { [4, 14] },
            ..Default::default()
        };
        // heutige Riot-Werte: CARRY/SUPPORT, Support oft mit Lane NONE
        assert_eq!(p("BOTTOM", "CARRY", false).position(), "ADC");
        assert_eq!(p("BOTTOM", "SUPPORT", false).position(), "SUPPORT");
        assert_eq!(p("NONE", "SUPPORT", false).position(), "SUPPORT");
        // ältere Werte bleiben gültig
        assert_eq!(p("BOTTOM", "DUO_CARRY", false).position(), "ADC");
        // Jungle nur mit Smite; Riots Jungle-Markierung ohne Smite ist unzuverlässig
        assert_eq!(p("JUNGLE", "NONE", true).position(), "JUNGLE");
        assert_eq!(p("JUNGLE", "NONE", false).position(), "");
        assert_eq!(p("MIDDLE", "SOLO", true).position(), "JUNGLE");
        assert_eq!(p("TOP", "SOLO", false).position(), "TOP");
    }

    #[test]
    fn lp_note_from_client() {
        // echte Meldung (gekürzt): Sieg mit Autofill-Schutz, +42 LP
        let v = serde_json::json!({"afkLpPenaltyAmount":0,"consolationLpUsed":0,"gameId":8024344091u64,"leaguePointsDelta":42,
            "lpBonusAppliedReason":"AUTOFILL_AEGIS","wasAfkOrLeaver":false,"queueType":"RANKED_SOLO_5x5"});
        let (gid, n) = LpNote::from_lcu(&v).unwrap();
        assert_eq!((gid, n.delta, n.bonus.as_str(), n.consolation), (8024344091, 42, "AUTOFILL_AEGIS", 0));
        assert!(n.modified());
        let plain = LpNote::from_lcu(&serde_json::json!({"gameId":1,"leaguePointsDelta":-20,"lpBonusAppliedReason":""})).unwrap().1;
        assert!(!plain.modified());
        assert!(LpNote::from_lcu(&serde_json::json!({})).is_none()); // 204 ohne Inhalt
    }

    #[test]
    fn champ_lookup() {
        let mut n = Names::default();
        n.champs.insert(145, "Kai'Sa".into());
        assert_eq!(n.champ_id("Kaisa"), 145);
        assert_eq!(n.champ_id("KAI'SA"), 145);
        assert_eq!(n.champ(145), "Kai'Sa");
        assert_eq!(n.champ(0), "-");
        // doppelter Name aus einem Sondermodus: echte ID gewinnt, unabhängig von der Reihenfolge
        n.champs.insert(60003, "Galio".into());
        n.champs.insert(3, "Galio".into());
        for _ in 0..20 {
            assert_eq!(n.champ_id("Galio"), 3);
        }
    }

    #[test]
    fn pick_lock_only_for_completed_own_pick() {
        let s: Value = serde_json::from_str(
            r#"{"actions":[[{"actorCellId":2,"type":"ban","completed":true}],
                           [{"actorCellId":2,"type":"pick","completed":false},{"actorCellId":3,"type":"pick","completed":true}]]}"#,
        )
        .unwrap();
        assert!(!pick_locked(&s, 2)); // nur gehovert
        assert!(pick_locked(&s, 3));
        let done: Value = serde_json::from_str(r#"{"actions":[[{"actorCellId":2,"type":"pick","completed":true}]]}"#).unwrap();
        assert!(pick_locked(&done, 2));
        assert!(!pick_locked(&Value::Null, 2));
    }
}
