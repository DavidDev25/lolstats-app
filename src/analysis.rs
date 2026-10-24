//! Matchmaking-Analytics: Team-Kompositionen, Elo-Filter und persönliche Leistung.

use crate::model::*;
use crate::stats::{Agg, SR_QUEUES};
use std::collections::HashMap;

// ---------- Team-Kompositionen ----------

/// Zusammensetzung eines Teams aus den Spielstil-Werten des Clients.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Comp {
    pub ad: u8,
    pub ap: u8,
    pub mixed: u8,
    pub cc: u32,
    pub durability: u32,
    pub damage: u32,
    pub mobility: u32,
    /// Champions mit bekannten Werten
    pub known: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    Balanced,
    AdHeavy,
    ApHeavy,
}

impl Profile {
    pub fn label(self) -> &'static str {
        match self {
            Profile::Balanced => tr!("ausgewogen (AD + AP)", "balanced (AD + AP)"),
            Profile::AdHeavy => tr!("einseitig AD (4+ AD)", "one-sided AD (4+ AD)"),
            Profile::ApHeavy => tr!("einseitig AP (4+ AP)", "one-sided AP (4+ AP)"),
        }
    }
}

impl Comp {
    pub fn of(champs: impl IntoIterator<Item = i32>, info: &HashMap<i32, ChampInfo>) -> Comp {
        let mut c = Comp::default();
        for id in champs {
            let Some(i) = info.get(&id) else { continue };
            c.known += 1;
            match i.dmg {
                DmgType::Ad => c.ad += 1,
                DmgType::Ap => c.ap += 1,
                DmgType::Mixed => c.mixed += 1,
            }
            c.cc += i.cc as u32;
            c.durability += i.durability as u32;
            c.damage += i.damage as u32;
            c.mobility += i.mobility as u32;
        }
        c
    }

    pub fn profile(&self) -> Profile {
        if self.ad >= 4 {
            Profile::AdHeavy
        } else if self.ap >= 4 {
            Profile::ApHeavy
        } else {
            Profile::Balanced
        }
    }

    pub fn label(&self) -> String {
        trf!(
            "{} AD · {} AP · {} gemischt · CC {} · Haltbarkeit {} · Mobilität {}", "{} AD · {} AP · {} mixed · CC {} · durability {} · mobility {}",
            self.ad, self.ap, self.mixed, self.cc, self.durability, self.mobility
        )
    }
}

/// Winrate-Auswertung über alle gesammelten Summoner's-Rift-Matches.
/// Jedes Match ergibt zwei Teams (Sieger und Verlierer), Werte sind also aus Sicht eines Teams.
#[derive(Clone, Debug, Default)]
pub struct CompStats {
    pub matches: usize,
    pub profile: Vec<(Profile, Agg)>,
    pub cc: Vec<(&'static str, Agg)>,
    pub durability: Vec<(&'static str, Agg)>,
}

fn bucket(d: i32) -> usize {
    if d >= 3 {
        0
    } else if d <= -3 {
        2
    } else {
        1
    }
}

pub fn comp_stats(ms: &[Match], info: &HashMap<i32, ChampInfo>) -> CompStats {
    let mut s = CompStats {
        profile: vec![
            (Profile::Balanced, Agg::default()),
            (Profile::AdHeavy, Agg::default()),
            (Profile::ApHeavy, Agg::default()),
        ],
        cc: vec![(tr!("3+ mehr CC als Gegner", "3+ more CC than enemy"), Agg::default()), (tr!("ähnlich (±2)", "similar (±2)"), Agg::default()), (tr!("3+ weniger CC", "3+ less CC"), Agg::default())],
        durability: vec![
            (tr!("3+ mehr Haltbarkeit", "3+ more durability"), Agg::default()),
            (tr!("ähnlich (±2)", "similar (±2)"), Agg::default()),
            (tr!("3+ weniger Haltbarkeit", "3+ less durability"), Agg::default()),
        ],
        ..Default::default()
    };
    if info.is_empty() {
        return s;
    }
    for m in ms.iter().filter(|m| SR_QUEUES.contains(&m.queue) && !m.remake() && m.ps.len() == 10) {
        let team = |t: i32| Comp::of(m.ps.iter().filter(|p| p.team == t).map(|p| p.champ), info);
        let (blue, red) = (team(100), team(200));
        if blue.known < 4 || red.known < 4 {
            continue;
        }
        let blue_win = m.ps.iter().find(|p| p.team == 100).is_some_and(|p| p.win);
        s.matches += 1;
        for (me, other, win) in [(blue, red, blue_win), (red, blue, !blue_win)] {
            let pi = s.profile.iter().position(|x| x.0 == me.profile()).unwrap_or(0);
            s.profile[pi].1.add(win);
            s.cc[bucket(me.cc as i32 - other.cc as i32)].1.add(win);
            s.durability[bucket(me.durability as i32 - other.durability as i32)].1.add(win);
        }
    }
    s
}

// ---------- Elo-Filter ----------

/// Match stammt aus einer Elo innerhalb von ±`span` (LP-Skala) um den eigenen Rang.
pub fn near_elo(m: &Match, mine: Option<i32>, span: i32) -> bool {
    matches!((m.elo, mine), (Some(e), Some(c)) if (e - c).abs() <= span)
}

// ---------- persönliche Leistung ----------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameStat {
    pub id: u64,
    pub ts: i64,
    pub queue: i32,
    pub win: bool,
    pub champ: i32,
    pub mins: f64,
    pub k: i32,
    pub d: i32,
    pub a: i32,
    pub cs: i32,
    pub timeline: Option<Timeline>,
    /// alle gespeicherten Werte des eigenen Teilnehmers
    pub part: Part,
    /// Champions des Gegnerteams
    pub enemies: Vec<i32>,
    /// Champion des direkten Lane-Gegners
    pub opp: Option<i32>,
}

impl GameStat {
    pub fn kda(&self) -> f64 {
        (self.k + self.a) as f64 / self.d.max(1) as f64
    }
    pub fn csm(&self) -> f64 {
        if self.mins > 0.0 {
            self.cs as f64 / self.mins
        } else {
            0.0
        }
    }
}

/// Eigene Summoner's-Rift-Spiele, älteste zuerst, mit Timeline falls vorhanden.
pub fn my_games(ms: &[Match], me: &str, tls: &[Timeline]) -> Vec<GameStat> {
    let tl: HashMap<u64, &Timeline> = tls.iter().map(|t| (t.match_id, t)).collect();
    let mut v: Vec<GameStat> = ms
        .iter()
        .filter(|m| SR_QUEUES.contains(&m.queue) && !m.remake())
        .filter_map(|m| {
            let p = m.part(me)?;
            Some(GameStat {
                id: m.id,
                ts: m.ts,
                queue: m.queue,
                win: p.win,
                champ: p.champ,
                mins: m.dur as f64 / 60.0,
                k: p.k,
                d: p.d,
                a: p.a,
                cs: p.cs,
                timeline: tl.get(&m.id).map(|t| (*t).clone()),
                part: p.clone(),
                enemies: m.ps.iter().filter(|q| q.team != p.team).map(|q| q.champ).collect(),
                opp: m.opponent(me).map(|o| o.champ),
            })
        })
        .collect();
    v.sort_by_key(|g| g.ts);
    v
}

/// Gleitender Schnitt über die letzten `n` Werte.
pub fn rolling(v: &[f64], n: usize) -> Vec<f64> {
    (0..v.len())
        .map(|i| {
            let w = &v[(i + 1).saturating_sub(n)..=i];
            w.iter().sum::<f64>() / w.len() as f64
        })
        .collect()
}

pub const EARLY_END: i32 = 14 * 60;
pub const MID_END: i32 = 25 * 60;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeathStats {
    /// Spiele mit Timeline
    pub games: usize,
    /// Ø Tode pro Spiel in Early (bis 14 min), Mid (14–25), Late (ab 25)
    pub early: f64,
    pub mid: f64,
    pub late: f64,
    /// Ø Zeitpunkt des ersten Todes (Sekunden), nur Spiele mit Tod
    pub first_death: Option<f64>,
    /// Anteil Spiele ohne Tod vor 10 Minuten
    pub clean_10: f64,
    pub cs10: Option<f64>,
    pub gold10: Option<f64>,
}

pub fn death_stats(gs: &[GameStat]) -> DeathStats {
    let tls: Vec<&Timeline> = gs.iter().filter_map(|g| g.timeline.as_ref()).collect();
    let n = tls.len();
    let mut s = DeathStats { games: n, ..Default::default() };
    if n == 0 {
        return s;
    }
    let (mut e, mut m, mut l, mut clean) = (0, 0, 0, 0);
    let mut firsts = Vec::new();
    for t in &tls {
        for &d in &t.deaths {
            if d < EARLY_END {
                e += 1;
            } else if d < MID_END {
                m += 1;
            } else {
                l += 1;
            }
        }
        if let Some(&f) = t.deaths.iter().min() {
            firsts.push(f as f64);
        }
        if t.deaths.iter().all(|&d| d >= 600) {
            clean += 1;
        }
    }
    let avg = |v: Vec<f64>| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    s.early = e as f64 / n as f64;
    s.mid = m as f64 / n as f64;
    s.late = l as f64 / n as f64;
    s.first_death = avg(firsts);
    s.clean_10 = clean as f64 / n as f64;
    s.cs10 = avg(tls.iter().filter_map(|t| t.cs10).map(|x| x as f64).collect());
    s.gold10 = avg(tls.iter().filter_map(|t| t.gold10).map(|x| x as f64).collect());
    s
}

// ---------- Coaching ----------
//
// Was entscheidet deine Spiele? Nur eigene Daten: Siegchance mit/ohne einfache Merkmale,
// Einbruch des CS-Vorsprungs von 10 auf 15 Minuten, Vision gegen Tode im Midgame (14–25 min).

/// Ab so vielen Spielen je Gruppe gibt es ein Urteil.
pub const COACH_MIN: usize = 10;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoachLine {
    pub label: &'static str,
    /// (Spiele, Siege) mit bzw. ohne das Merkmal
    pub with: (usize, usize),
    pub without: (usize, usize),
}

impl CoachLine {
    pub fn wr(x: (usize, usize)) -> Option<f64> {
        (x.0 > 0).then(|| x.1 as f64 / x.0 as f64)
    }
    /// Unterschied der Siegchance in Prozentpunkten, erst ab genug Spielen je Gruppe
    pub fn gap(&self) -> Option<f64> {
        if self.with.0 < COACH_MIN || self.without.0 < COACH_MIN {
            return None;
        }
        Some((Self::wr(self.with)? - Self::wr(self.without)?) * 100.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coaching {
    pub lines: Vec<CoachLine>,
    /// Spiele mit CS beider Lane-Gegner bei 10 und 15 min
    pub lead_games: usize,
    pub lead10: f64,
    pub lead15: f64,
    /// Anteil der Spiele mit Vorsprung bei 10 min, in denen er bis 15 min schrumpft
    pub lead_shrinks: Option<f64>,
    /// Spiele ab 20 min mit Vision und Timeline; Ø Midgame-Tode bei Vision unter/über deinem Median
    pub vision_games: usize,
    pub mid_deaths_low_vision: f64,
    pub mid_deaths_high_vision: f64,
    /// Ø Tode vor Minute 10 insgesamt und die höchsten Werte je eigenem Champion bzw. Lane-Gegner:
    /// (Champion, Ø frühe Tode, Spiele), ab `EARLY_MIN_GAMES` Spielen, höchste zuerst
    pub early_avg: Option<f64>,
    pub early_by_champ: Vec<(i32, f64, usize)>,
    pub early_by_opp: Vec<(i32, f64, usize)>,
}

/// Mindestspiele je Champion/Gegner für die Liste der frühen Tode.
pub const EARLY_MIN_GAMES: usize = 3;

/// Gruppiert Tode vor Minute 10 nach `key` und liefert die drei höchsten Schnitte.
fn early_by(gs: &[GameStat], key: &dyn Fn(&GameStat) -> Option<i32>) -> Vec<(i32, f64, usize)> {
    let mut h: HashMap<i32, (f64, usize)> = HashMap::new();
    for g in gs {
        let (Some(k), Some(t)) = (key(g), g.timeline.as_ref()) else { continue };
        let e = h.entry(k).or_default();
        e.0 += t.deaths.iter().filter(|&&d| d < 600).count() as f64;
        e.1 += 1;
    }
    let mut v: Vec<(i32, f64, usize)> = h.into_iter().filter(|x| x.1 .1 >= EARLY_MIN_GAMES).map(|(k, (s, n))| (k, s / n as f64, n)).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(3);
    v
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

/// `gpm` = Ø Gold/min je Champion in deiner Elo (aus `elo_gpm`).
pub fn coaching(gs: &[GameStat], gpm: &HashMap<i32, (f64, usize)>) -> Coaching {
    let vis = |g: &GameStat| (g.part.v >= PART_V && g.mins > 0.0).then(|| g.part.vision as f64 / g.mins);
    let vis_median = median(gs.iter().filter_map(vis).collect());
    let lead10 = |g: &GameStat| g.timeline.as_ref().and_then(|t| Some(t.cs10? - t.opp_cs10?));
    let lead15 = |g: &GameStat| g.timeline.as_ref().and_then(|t| Some(t.cs15? - t.opp_cs15?));
    let line = |label: &'static str, f: &dyn Fn(&GameStat) -> Option<bool>| {
        let mut l = CoachLine { label, ..Default::default() };
        for g in gs {
            if let Some(has) = f(g) {
                let b = if has { &mut l.with } else { &mut l.without };
                b.0 += 1;
                b.1 += g.win as usize;
            }
        }
        l
    };
    let lines = vec![
        line(tr!("kein Tod vor Minute 10", "no death before minute 10"), &|g| g.timeline.as_ref().map(|t| t.deaths.iter().all(|&d| d >= 600))),
        line(tr!("CS-Vorsprung bei 10 min", "CS lead at 10 min"), &|g| lead10(g).map(|d| d > 0)),
        line(tr!("CS-Vorsprung bei 15 min", "CS lead at 15 min"), &|g| lead15(g).map(|d| d > 0)),
        line(tr!("Vision/min über deinem Schnitt", "vision/min above your average"), &|g| Some(vis(g)? > vis_median?)),
        line(tr!("höchstens 5 Tode", "at most 5 deaths"), &|g| Some(g.d <= 5)),
        // Zusatzfelder erst ab Erweiterung 1 bekannt
        line(tr!("an First Blood beteiligt", "involved in first blood"), &|g| (g.part.ext >= PART_EXT).then(|| g.part.first_blood || g.part.fb_assist)),
        line(tr!("am ersten Turm beteiligt", "involved in first tower"), &|g| (g.part.ext >= PART_EXT).then_some(g.part.first_tower)),
        line(tr!("Gold/min über Elo-Schnitt des Champions", "gold/min above the champion's elo average"), &|g| {
            let (avg, _) = gpm.get(&g.champ)?;
            (g.mins > 0.0).then(|| g.part.gold as f64 / g.mins > *avg)
        }),
    ];
    let both: Vec<(i32, i32)> = gs.iter().filter_map(|g| Some((lead10(g)?, lead15(g)?))).collect();
    let n = both.len().max(1) as f64;
    let ahead: Vec<&(i32, i32)> = both.iter().filter(|x| x.0 > 0).collect();
    let mid_deaths = |g: &GameStat| g.timeline.as_ref().map(|t| t.deaths.iter().filter(|&&d| (EARLY_END..MID_END).contains(&d)).count() as f64);
    let vg: Vec<(f64, f64)> = gs.iter().filter(|g| g.mins >= 20.0).filter_map(|g| Some((vis(g)?, mid_deaths(g)?))).collect();
    let vm = median(vg.iter().map(|x| x.0).collect()).unwrap_or(0.0);
    let avg = |v: Vec<f64>| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    Coaching {
        lines,
        lead_games: both.len(),
        lead10: both.iter().map(|x| x.0 as f64).sum::<f64>() / n,
        lead15: both.iter().map(|x| x.1 as f64).sum::<f64>() / n,
        lead_shrinks: (!ahead.is_empty()).then(|| ahead.iter().filter(|x| x.1 < x.0).count() as f64 / ahead.len() as f64),
        vision_games: vg.len(),
        mid_deaths_low_vision: avg(vg.iter().filter(|x| x.0 <= vm).map(|x| x.1).collect()),
        mid_deaths_high_vision: avg(vg.iter().filter(|x| x.0 > vm).map(|x| x.1).collect()),
        early_avg: {
            let e: Vec<f64> = gs.iter().filter_map(|g| Some(g.timeline.as_ref()?.deaths.iter().filter(|&&d| d < 600).count() as f64)).collect();
            (!e.is_empty()).then(|| avg(e))
        },
        early_by_champ: early_by(gs, &|g| Some(g.champ)),
        early_by_opp: early_by(gs, &|g| g.opp),
    }
}

// ---------- CS-Tacho ----------
//
// Live-Vergleich: eigener CS jetzt gegen den eigenen Verlauf (Ø CS bei 10/15/20 min aus Timelines)
// und gegen den CS/min-Schnitt des Champions in der eigenen Elo.

/// Eigener CS-Verlauf: (Spiele, Ø CS bei 10, 15, 20 min, nur dieser Champion?)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CsCurve {
    pub games: usize,
    pub at: [f64; 3],
    pub champ_only: bool,
}

/// Ab so vielen Timelines auf dem Champion wird nur dieser verwendet, sonst alle Spiele.
pub const CS_CURVE_MIN: usize = 3;

pub fn cs_curve(gs: &[GameStat], champ: i32) -> Option<CsCurve> {
    let full = |g: &&GameStat| g.timeline.as_ref().is_some_and(|t| t.cs10.is_some() && t.cs15.is_some() && t.cs20.is_some());
    let on_champ: Vec<&GameStat> = gs.iter().filter(|g| g.champ == champ).filter(full).collect();
    let champ_only = on_champ.len() >= CS_CURVE_MIN;
    let pool: Vec<&GameStat> = if champ_only { on_champ } else { gs.iter().filter(full).collect() };
    if pool.is_empty() {
        return None;
    }
    let n = pool.len() as f64;
    let avg = |f: fn(&Timeline) -> Option<i32>| pool.iter().filter_map(|g| f(g.timeline.as_ref()?)).sum::<i32>() as f64 / n;
    Some(CsCurve { games: pool.len(), at: [avg(|t| t.cs10), avg(|t| t.cs15), avg(|t| t.cs20)], champ_only })
}

impl CsCurve {
    /// Erwarteter CS bei `mins` Spielminuten (linear zwischen 0, 10, 15, 20 min, danach fortgeschrieben).
    pub fn expected(&self, mins: f64) -> f64 {
        let pts = [(0.0, 0.0), (10.0, self.at[0]), (15.0, self.at[1]), (20.0, self.at[2])];
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if mins <= x1 {
                return y0 + (y1 - y0) * (mins - x0).max(0.0) / (x1 - x0);
            }
        }
        self.at[2] + (self.at[2] - self.at[1]) / 5.0 * (mins - 20.0)
    }
}

/// Ø CS/min pro Champion (Summoner's Rift, ohne dich) in deiner Elo (±400 LP, sonst alle Elos): Champion -> (CS/min, Spiele).
pub fn elo_csm(ms: &[Match], me: &str, mine_elo: Option<i32>) -> HashMap<i32, (f64, usize)> {
    elo_rate(ms, me, mine_elo, |p| p.cs)
}

/// Ø Gold/min pro Champion in deiner Elo (wie `elo_csm`).
pub fn elo_gpm(ms: &[Match], me: &str, mine_elo: Option<i32>) -> HashMap<i32, (f64, usize)> {
    elo_rate(ms, me, mine_elo, |p| p.gold)
}

/// Ø eines Werts pro Minute je Champion in deiner Elo (±400 LP; zu wenige Spiele -> alle Elos).
fn elo_rate(ms: &[Match], me: &str, mine_elo: Option<i32>, val: fn(&Part) -> i32) -> HashMap<i32, (f64, usize)> {
    let collect = |near: bool| {
        let mut h: HashMap<i32, (f64, usize)> = HashMap::new();
        for m in ms.iter().filter(|m| SR_QUEUES.contains(&m.queue) && !m.remake() && m.dur > 0) {
            if near && !near_elo(m, mine_elo, 400) {
                continue;
            }
            for p in m.ps.iter().filter(|p| p.puuid != me) {
                let e = h.entry(p.champ).or_default();
                e.0 += val(p) as f64 * 60.0 / m.dur as f64;
                e.1 += 1;
            }
        }
        h
    };
    let near = collect(true);
    let all = collect(false);
    all.into_iter()
        .map(|(c, v)| {
            let v = near.get(&c).copied().filter(|n| n.1 >= ELO_CSM_MIN).unwrap_or(v);
            (c, (v.0 / v.1 as f64, v.1))
        })
        .collect()
}

/// Mindestspiele eines Champions in der Elo, sonst wird der Schnitt über alle Elos genommen.
pub const ELO_CSM_MIN: usize = 10;

// ---------- Tilt-Hinweis ----------
//
// Nach 2+ Niederlagen in Folge in derselben Session (Pause unter 2 h) erscheint ein sanfter Hinweis.
// Belegt wird er mit den eigenen Daten: Siegchance nach solchen Serien gegen sonst und frühe Tode.

/// Spiele gehören zur selben Session, wenn dazwischen weniger als 2 h Pause liegen.
pub const SESSION_GAP_MS: i64 = 2 * 3600 * 1000;
/// Ab so vielen Niederlagen in Folge erscheint der Hinweis.
pub const TILT_STREAK: usize = 2;
/// Ab so vielen Spielen nach Serien wird die Siegchance verglichen.
pub const TILT_MIN_GAMES: usize = 10;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tilt {
    /// Niederlagen in Folge zuletzt (innerhalb der Session)
    pub streak: usize,
    /// (Spiele, Siege) der aktuellen Session
    pub session: (usize, usize),
    /// (Spiele, Siege) direkt nach 2+ Niederlagen in derselben Session, und alle übrigen Spiele
    pub after: (usize, usize),
    pub other: (usize, usize),
    /// Ø Tode vor Minute 10 in der Niederlagenserie und in allen Spielen (nur mit Timeline)
    pub early_streak: Option<f64>,
    pub early_all: Option<f64>,
}

fn game_end(g: &GameStat) -> i64 {
    g.ts + (g.mins * 60_000.0) as i64
}

/// Lange Session: Hast du heute schon 3+ Spiele gemacht und gewinnst du ab dem 4. Spiel belegbar
/// seltener (je Seite ab `COACH_MIN` Spielen, mindestens 10 Prozentpunkte)? -> (Siegchance 4.+, Siegchance 1.–3., Spiele 4.+)
pub fn long_session(gs: &[GameStat], t: &Tilt) -> Option<(f64, f64, usize)> {
    if t.session.0 < 3 {
        return None;
    }
    let sw = session_wr(gs);
    let early = (sw[0].0 + sw[1].0 + sw[2].0, sw[0].1 + sw[1].1 + sw[2].1);
    let late = sw[3];
    if early.0 < COACH_MIN || late.0 < COACH_MIN {
        return None;
    }
    let (we, wl) = (early.1 as f64 / early.0 as f64, late.1 as f64 / late.0 as f64);
    (wl + 0.10 <= we).then_some((wl, we, late.0))
}

/// Siegchance nach Spielnummer in der Session: (Spiele, Siege) für das 1., 2., 3. und 4.+ Spiel.
pub fn session_wr(gs: &[GameStat]) -> [(usize, usize); 4] {
    let mut out = [(0, 0); 4];
    let mut idx = 0;
    for (i, g) in gs.iter().enumerate() {
        idx = if i > 0 && g.ts - game_end(&gs[i - 1]) < SESSION_GAP_MS { idx + 1 } else { 0 };
        let b = &mut out[idx.min(3)];
        b.0 += 1;
        b.1 += g.win as usize;
    }
    out
}

/// Tilt-Lage nach dem letzten Spiel. `now` in ms; None, wenn das letzte Spiel länger als 2 h her ist.
pub fn tilt(gs: &[GameStat], now: i64) -> Option<Tilt> {
    let last = gs.last()?;
    if now - game_end(last) > SESSION_GAP_MS {
        return None;
    }
    // Serie vor jedem Spiel (gleiche Session) und Session-Grenzen
    let mut t = Tilt::default();
    let mut run = 0;
    for (i, g) in gs.iter().enumerate() {
        let same = i > 0 && g.ts - game_end(&gs[i - 1]) < SESSION_GAP_MS;
        if !same {
            run = 0;
            t.session = (0, 0);
        }
        let bucket = if run >= TILT_STREAK { &mut t.after } else { &mut t.other };
        bucket.0 += 1;
        bucket.1 += g.win as usize;
        t.session.0 += 1;
        t.session.1 += g.win as usize;
        run = if g.win { 0 } else { run + 1 };
    }
    t.streak = run;
    let early = |g: &GameStat| g.timeline.as_ref().map(|tl| tl.deaths.iter().filter(|&&d| d < 600).count() as f64);
    let avg = |v: Vec<f64>| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    t.early_streak = avg(gs[gs.len() - run..].iter().filter_map(early).collect());
    t.early_all = avg(gs.iter().filter_map(early).collect());
    Some(t)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolLine {
    pub champ: i32,
    pub agg: Agg,
    /// Anteil an den betrachteten Spielen
    pub share: f64,
    pub kda: f64,
    pub csm: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pool {
    /// betrachtete Spiele (die letzten 50)
    pub games: usize,
    pub distinct: usize,
    /// Anteil der 3 meistgespielten Champions
    pub top3: f64,
    pub lines: Vec<PoolLine>,
}

pub fn pool(gs: &[GameStat]) -> Pool {
    let recent = &gs[gs.len().saturating_sub(50)..];
    let mut map: HashMap<i32, (Agg, i32, i32, i32, i32, f64)> = HashMap::new();
    for g in recent {
        let e = map.entry(g.champ).or_default();
        e.0.add(g.win);
        e.1 += g.k;
        e.2 += g.d;
        e.3 += g.a;
        e.4 += g.cs;
        e.5 += g.mins;
    }
    let n = recent.len().max(1) as f64;
    let mut lines: Vec<PoolLine> = map
        .into_iter()
        .map(|(champ, (agg, k, d, a, cs, mins))| PoolLine {
            champ,
            agg,
            share: agg.games as f64 / n,
            kda: (k + a) as f64 / d.max(1) as f64,
            csm: if mins > 0.0 { cs as f64 / mins } else { 0.0 },
        })
        .collect();
    lines.sort_by(|a, b| b.agg.games.cmp(&a.agg.games).then(a.champ.cmp(&b.champ)));
    let top3 = lines.iter().take(3).map(|l| l.share).sum();
    Pool { games: recent.len(), distinct: lines.len(), top3, lines }
}

// ---------- Rekorde & Allzeit-Hochs ----------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Best {
    pub value: f64,
    pub champ: i32,
    pub ts: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Records {
    /// höchster Stand im eigenen LP-Verlauf (LP-Skala, Zeitpunkt)
    pub peak: Option<(i32, i64)>,
    pub season: Season,
    pub win_streak: usize,
    pub loss_streak: usize,
    pub kills: Option<Best>,
    pub cs: Option<Best>,
    pub csm: Option<Best>,
    pub dmg: Option<Best>,
    pub kda: Option<Best>,
    pub vision: Option<Best>,
    pub spree: Option<Best>,
    pub doubles: i32,
    pub triples: i32,
    pub quadras: i32,
    pub pentas: i32,
    pub first_bloods: i32,
}

fn best(gs: &[GameStat], f: impl Fn(&GameStat) -> Option<f64>) -> Option<Best> {
    gs.iter()
        .filter_map(|g| Some(Best { value: f(g)?, champ: g.champ, ts: g.ts }))
        .max_by(|a, b| a.value.total_cmp(&b.value))
}

pub fn records(gs: &[GameStat], snaps: &[LpSnap], season: Season) -> Records {
    let full = |g: &GameStat| g.part.v >= PART_V;
    let (mut win, mut loss, mut cw, mut cl) = (0, 0, 0, 0);
    for g in gs {
        if g.win {
            cw += 1;
            cl = 0;
        } else {
            cl += 1;
            cw = 0;
        }
        win = win.max(cw);
        loss = loss.max(cl);
    }
    let sum = |f: fn(&Part) -> i32| gs.iter().filter(|g| full(g)).map(|g| f(&g.part)).sum();
    Records {
        peak: snaps
            .iter()
            .filter_map(|s| Some((s.rank.value()?, s.ts)))
            .max_by_key(|x| x.0),
        season,
        win_streak: win,
        loss_streak: loss,
        kills: best(gs, |g| Some(g.k as f64)),
        cs: best(gs, |g| Some(g.cs as f64)),
        // CS/min erst ab 15 Minuten Spielzeit, sonst verzerren kurze Spiele
        csm: best(gs, |g| (g.mins >= 15.0).then(|| g.csm())),
        dmg: best(gs, |g| Some(g.part.dmg as f64)),
        kda: best(gs, |g| Some(g.kda())),
        vision: best(gs, |g| full(g).then_some(g.part.vision as f64)),
        spree: best(gs, |g| full(g).then_some(g.part.spree as f64)),
        doubles: sum(|p| p.doubles),
        triples: sum(|p| p.triples),
        quadras: sum(|p| p.quadras),
        pentas: sum(|p| p.pentas),
        first_bloods: sum(|p| p.first_blood as i32),
    }
}

// ---------- Farmen ----------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Farm {
    /// betrachtete Spiele (die letzten 50)
    pub games: usize,
    pub lane_csm: Option<f64>,
    pub jungle_csm: Option<f64>,
    pub cs10: Option<f64>,
    pub cs15: Option<f64>,
    pub cs20: Option<f64>,
    /// Ø CS-Differenz zum direkten Lane-Gegner (positiv = du hast mehr)
    pub diff10: Option<f64>,
    pub diff15: Option<f64>,
    pub diff_games: usize,
}

fn avg(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| s / n as f64)
}

pub fn farm(gs: &[GameStat]) -> Farm {
    let recent = &gs[gs.len().saturating_sub(50)..];
    let full: Vec<&GameStat> = recent.iter().filter(|g| g.part.v >= PART_V && g.mins > 0.0).collect();
    let tls: Vec<&Timeline> = recent.iter().filter_map(|g| g.timeline.as_ref()).collect();
    let diff = |a: fn(&Timeline) -> Option<i32>, b: fn(&Timeline) -> Option<i32>| {
        avg(tls.iter().filter_map(|t| Some((a(t)? - b(t)?) as f64)))
    };
    Farm {
        games: recent.len(),
        lane_csm: avg(full.iter().map(|g| g.part.lane_cs as f64 / g.mins)),
        jungle_csm: avg(full.iter().map(|g| g.part.jungle_cs as f64 / g.mins)),
        cs10: avg(tls.iter().filter_map(|t| t.cs10).map(|x| x as f64)),
        cs15: avg(tls.iter().filter_map(|t| t.cs15).map(|x| x as f64)),
        cs20: avg(tls.iter().filter_map(|t| t.cs20).map(|x| x as f64)),
        diff10: diff(|t| t.cs10, |t| t.opp_cs10),
        diff15: diff(|t| t.cs15, |t| t.opp_cs15),
        diff_games: tls.iter().filter(|t| t.cs10.is_some() && t.opp_cs10.is_some()).count(),
    }
}

// ---------- Spielstil ----------

pub const STYLE_LABELS: [(&str, bool, bool); 9] = [
    // (Name, Prozentwert, höher = besser)
    ("Kill-Beteiligung", true, true),
    ("Anteil am Team-Schaden", true, true),
    ("Anteil am Team-Gold", true, true),
    ("CS pro Minute", false, true),
    ("Vision pro Minute", false, true),
    ("Anteil erlittener Schaden", true, true),
    ("Objective-Schaden pro Minute", false, true),
    ("CC-Sekunden pro Minute", false, true),
    ("Tode pro 10 Minuten", false, false),
];

fn style_metrics(m: &Match, p: &Part) -> Option<[f64; 9]> {
    if p.v < PART_V || m.ps.len() != 10 || m.dur <= 0 {
        return None;
    }
    let mins = m.dur as f64 / 60.0;
    let share = |x: i32, total: i32| x as f64 / total.max(1) as f64;
    Some([
        share(p.k + p.a, m.team_sum(p.team, |q| q.k)),
        share(p.dmg, m.team_sum(p.team, |q| q.dmg)),
        share(p.gold, m.team_sum(p.team, |q| q.gold)),
        p.cs as f64 / mins,
        p.vision as f64 / mins,
        share(p.dmg_taken, m.team_sum(p.team, |q| q.dmg_taken)),
        p.dmg_obj as f64 / mins,
        p.cc_time as f64 / mins,
        p.d as f64 / mins * 10.0,
    ])
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// Hauptposition (am häufigsten gespielt)
    pub position: &'static str,
    pub games: usize,
    pub peers: usize,
    /// Vergleichsgruppe auf Elo-nahe Matches eingeschränkt (sonst alle)
    pub near_elo: bool,
    pub mine: [f64; 9],
    pub peer: Option<[f64; 9]>,
}

fn mean_rows(rows: &[[f64; 9]]) -> Option<[f64; 9]> {
    (!rows.is_empty()).then(|| std::array::from_fn(|i| rows.iter().map(|r| r[i]).sum::<f64>() / rows.len() as f64))
}

/// Eigenes Profil auf der Hauptposition (letzte 50 Spiele) gegen Spieler derselben Position,
/// bevorzugt aus Matches nahe der eigenen Elo (ab 30 Vergleichswerten).
pub fn style(ms: &[Match], me: &str, mine_elo: Option<i32>) -> Option<Style> {
    let sr = |m: &&Match| SR_QUEUES.contains(&m.queue) && !m.remake();
    let mut own: Vec<(&Match, &Part)> = ms.iter().filter(sr).filter_map(|m| Some((m, m.part(me)?))).collect();
    own.sort_by_key(|(m, _)| m.ts);
    let own = &own[own.len().saturating_sub(50)..];
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for (_, p) in own.iter().filter(|(_, p)| p.v >= PART_V) {
        if !p.position().is_empty() {
            *counts.entry(p.position()).or_default() += 1;
        }
    }
    let position = counts.into_iter().max_by_key(|x| x.1)?.0;
    let mine: Vec<[f64; 9]> = own
        .iter()
        .filter(|(_, p)| p.position() == position)
        .filter_map(|(m, p)| style_metrics(m, p))
        .collect();
    let peers_for = |near: bool| -> Vec<[f64; 9]> {
        ms.iter()
            .filter(sr)
            .filter(|m| !near || near_elo(m, mine_elo, 400))
            .flat_map(|m| m.ps.iter().filter(|p| p.puuid != me && p.position() == position).filter_map(move |p| style_metrics(m, p)))
            .collect()
    };
    let near = peers_for(true);
    let (peers, near_elo) = if near.len() >= 30 { (near, true) } else { (peers_for(false), false) };
    Some(Style {
        position,
        games: mine.len(),
        peers: peers.len(),
        near_elo,
        mine: mean_rows(&mine)?,
        peer: mean_rows(&peers),
    })
}

// ---------- Pick-Empfehlung ----------

/// (Champion, Gegner-Champion) -> Bilanz des Champions in Spielen gegen diesen Gegner.
pub type Matchups = HashMap<(i32, i32), Agg>;

pub fn matchups(ms: &[Match]) -> Matchups {
    let mut mu = Matchups::new();
    for m in ms.iter().filter(|m| SR_QUEUES.contains(&m.queue) && !m.remake() && m.ps.len() == 10) {
        for p in &m.ps {
            for q in m.ps.iter().filter(|q| q.team != p.team) {
                mu.entry((p.champ, q.champ)).or_default().add(p.win);
            }
        }
    }
    mu
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rec {
    pub champ: i32,
    /// deine Bilanz auf dem Champion (letzte 100 Spiele)
    pub mine: Agg,
    pub kda: f64,
    /// Champion gegen die gepickten Gegner, aus allen gesammelten Matches
    pub vs_db: Agg,
    /// du mit diesem Champion gegen die gepickten Gegner
    pub vs_me: Agg,
    /// deine Spiele mit dem Champion auf der gesuchten Position
    pub on_pos: i32,
    pub score: f64,
}

/// Winrate mit Vorab-Annahme 50 %, die mit `k` Spielen gewichtet wird (kleine Stichproben -> nahe 50 %).
pub fn shrink(a: &Agg, k: f64) -> f64 {
    (a.wins as f64 + k * 0.5) / (a.games as f64 + k)
}

/// Empfiehlt Champions aus dem eigenen Pool (mind. 3 Spiele in den letzten 100).
/// Score = 50 % eigene Stärke + 35 % Matchup aus der DB + 15 % eigene Ergebnisse gegen diese Gegner,
/// abgewertet, wenn der Champion nie auf der gesuchten Position gespielt wurde.
pub fn recommend(gs: &[GameStat], mu: &Matchups, enemies: &[i32], position: &str) -> Vec<Rec> {
    let recent = &gs[gs.len().saturating_sub(100)..];
    let mut by: HashMap<i32, (Agg, i32, i32, i32, i32, Agg)> = HashMap::new();
    for g in recent {
        let e = by.entry(g.champ).or_default();
        e.0.add(g.win);
        e.1 += g.k;
        e.2 += g.d;
        e.3 += g.a;
        if !position.is_empty() && g.part.position() == position {
            e.4 += 1;
        }
        if enemies.iter().any(|x| g.enemies.contains(x)) {
            e.5.add(g.win);
        }
    }
    let mut recs: Vec<Rec> = by
        .into_iter()
        .filter(|(_, e)| e.0.games >= 3)
        .map(|(champ, (mine, k, d, a, on_pos, vs_me))| {
            let mut vs_db = Agg::default();
            for &e in enemies.iter().filter(|&&e| e > 0) {
                if let Some(x) = mu.get(&(champ, e)) {
                    vs_db.games += x.games;
                    vs_db.wins += x.wins;
                }
            }
            let mut score = 0.5 * shrink(&mine, 5.0) + 0.35 * shrink(&vs_db, 8.0) + 0.15 * shrink(&vs_me, 3.0);
            if !position.is_empty() && on_pos == 0 {
                score *= 0.85;
            }
            Rec { champ, mine, kda: (k + a) as f64 / d.max(1) as f64, vs_db, vs_me, on_pos, score }
        })
        .collect();
    recs.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.champ.cmp(&b.champ)));
    recs.truncate(8);
    recs
}

// ---------- Nach-Match-Zusammenfassung ----------

/// Ein Wert des letzten Spiels gegen den Schnitt der Spiele davor.
#[derive(Clone, Debug, PartialEq)]
pub struct SumLine {
    pub label: &'static str,
    pub value: f64,
    pub avg: f64,
    /// true = höher ist besser (bei Toden false)
    pub higher_better: bool,
    pub decimals: usize,
}

impl SumLine {
    /// +1 besser als der Schnitt, -1 schlechter, 0 etwa gleich (innerhalb von 10 %)
    pub fn verdict(&self) -> i32 {
        let tol = (self.avg.abs() * 0.1).max(0.05);
        if (self.value - self.avg).abs() <= tol {
            0
        } else if (self.value > self.avg) == self.higher_better {
            1
        } else {
            -1
        }
    }
}

/// Spiel `game_id` gegen den Schnitt der bis zu 20 eigenen Spiele davor. None, solange das Spiel
/// (noch) nicht in der Match-History ist oder es keine Spiele davor gibt.
pub fn match_summary(gs: &[GameStat], game_id: u64) -> Option<(usize, Vec<SumLine>)> {
    let i = gs.iter().position(|g| g.id == game_id)?;
    let g = &gs[i];
    let base: Vec<&GameStat> = gs[..i].iter().rev().filter(|x| x.mins > 5.0).take(20).collect();
    if base.is_empty() || g.mins <= 0.0 {
        return None;
    }
    let avg = |f: &dyn Fn(&GameStat) -> f64| base.iter().map(|x| f(x)).sum::<f64>() / base.len() as f64;
    let per_min = |v: i32, x: &GameStat| v as f64 / x.mins.max(1.0);
    let mut lines = vec![
        SumLine { label: "KDA", value: g.kda(), avg: avg(&|x| x.kda()), higher_better: true, decimals: 2 },
        SumLine { label: "Kills", value: g.k as f64, avg: avg(&|x| x.k as f64), higher_better: true, decimals: 1 },
        SumLine { label: "Tode", value: g.d as f64, avg: avg(&|x| x.d as f64), higher_better: false, decimals: 1 },
        SumLine { label: "CS/min", value: g.csm(), avg: avg(&|x| x.csm()), higher_better: true, decimals: 1 },
        SumLine { label: "Gold/min", value: per_min(g.part.gold, g), avg: avg(&|x| per_min(x.part.gold, x)), higher_better: true, decimals: 0 },
        SumLine { label: "Schaden/min", value: per_min(g.part.dmg, g), avg: avg(&|x| per_min(x.part.dmg, x)), higher_better: true, decimals: 0 },
        SumLine { label: "Sicht/min", value: per_min(g.part.vision, g), avg: avg(&|x| per_min(x.part.vision, x)), higher_better: true, decimals: 2 },
    ];
    let cs10: Vec<f64> = base.iter().filter_map(|x| x.timeline.as_ref()?.cs10).map(|v| v as f64).collect();
    if let (Some(v), false) = (g.timeline.as_ref().and_then(|t| t.cs10), cs10.is_empty()) {
        let a = cs10.iter().sum::<f64>() / cs10.len() as f64;
        lines.push(SumLine { label: "CS nach 10 min", value: v as f64, avg: a, higher_better: true, decimals: 0 });
    }
    Some((base.len(), lines))
}

// ---------- Lane-Erkennung ----------

pub const LANES: [&str; 5] = ["TOP", "JUNGLE", "MID", "ADC", "SUPPORT"];

/// Champion -> wie oft er in den gesammelten Matches auf TOP/JUNGLE/MID/ADC/SUPPORT gespielt wurde.
pub type LaneFreq = HashMap<i32, [u32; 5]>;

/// Komplette Neuzählung; im Programm übernimmt das die Tabelle `lane_counts` (store.rs), hier nur als Referenz für Tests.
#[cfg(test)]
pub fn lane_freq(ms: &[Match]) -> LaneFreq {
    let mut f = LaneFreq::new();
    for m in ms {
        add_lane_counts(&mut f, m);
    }
    f
}

/// Ein Match in die Lane-Statistik aufnehmen (für den Cache im Hintergrunddienst).
pub fn add_lane_counts(f: &mut LaneFreq, m: &Match) {
    if !SR_QUEUES.contains(&m.queue) || m.remake() {
        return;
    }
    for p in &m.ps {
        if let Some(i) = LANES.iter().position(|&l| l == p.position()) {
            f.entry(p.champ).or_default()[i] += 1;
        }
    }
}

/// Mindestspiele eines Champions in der Lane-Statistik, bevor "Offmeta" beurteilt wird.
pub const OFFMETA_MIN_GAMES: u32 = 20;
/// Anteil an der Position, unter dem ein Pick als Offmeta gilt.
pub const OFFMETA_SHARE: f64 = 0.05;

/// Champion auf einer Position gespielt, auf der er in den gesammelten Matches kaum vorkommt
/// (z. B. Fizz als ADC). `lane` = "TOP"/"JUNGLE"/"MID"/"ADC"/"SUPPORT".
pub fn is_offmeta(freq: &LaneFreq, champ: i32, lane: &str) -> bool {
    let Some(i) = LANES.iter().position(|&l| l == lane) else { return false };
    let Some(c) = freq.get(&champ) else { return false };
    let total: u32 = c.iter().sum();
    total >= OFFMETA_MIN_GAMES && (c[i] as f64) < total as f64 * OFFMETA_SHARE
}

/// Queues mit festen Lanes: Summoner's Rift (Normal, Ranked, Quickplay, Swiftplay 480, Clash 700)
/// und 0 = unbekannt (z. B. Custom auf Summoner's Rift). Alles andere gilt als Sondermodus
/// (ARAM, Queue 2400 usw.) und wird von lolstats weder gespeichert noch angezeigt.
pub const LANE_QUEUES: [i32; 8] = [0, 400, 420, 430, 440, 490, 480, 700];

pub fn queue_has_lanes(queue: i32) -> bool {
    LANE_QUEUES.contains(&queue)
}

/// Lobby und Ergebnis aus Sondermodi (ARAM, Queue 2400 …) nicht anzeigen.
pub fn without_off_modes(mut s: LiveState) -> LiveState {
    if s.lobby.as_ref().is_some_and(|l| !queue_has_lanes(l.queue)) {
        s.lobby = None;
    }
    if s.result.as_ref().is_some_and(|r| !queue_has_lanes(r.queue)) {
        s.result = None;
    }
    s
}

/// Setzt `lane` für alle Spieler mit Champion: Position vom Client, wenn bekannt, sonst pro Team die
/// wahrscheinlichste Verteilung (alle Permutationen) aus Champion-Statistik und Smite (= Jungle).
/// In Modi ohne Lanes bleibt `lane` leer.
pub fn fill_lanes(players: &mut [LivePlayer], freq: &LaneFreq, queue: i32) {
    if !queue_has_lanes(queue) {
        for p in players.iter_mut() {
            p.lane.clear();
        }
        return;
    }
    for team in [100, 200] {
        let idx: Vec<usize> = (0..players.len()).filter(|&i| players[i].team == team).collect();
        let raw: Vec<Option<usize>> =
            idx.iter().map(|&i| LANES.iter().position(|&l| l == position_from_select(&players[i].position))).collect();
        // Doppelte Client-Positionen (z. B. Blind Pick) sind nicht verlässlich -> nur eindeutige übernehmen
        let known: Vec<Option<usize>> =
            raw.iter().map(|k| k.filter(|k| raw.iter().filter(|x| **x == Some(*k)).count() == 1)).collect();
        let mut best: Option<(f64, [usize; 5])> = None;
        let mut perm = [0usize, 1, 2, 3, 4];
        loop {
            let mut score = 0.0;
            let mut ok = true;
            for (j, &i) in idx.iter().enumerate().take(5) {
                let lane = perm[j];
                let p = &players[i];
                if let Some(k) = known[j] {
                    if k != lane {
                        ok = false;
                        break;
                    }
                    continue;
                }
                if p.champ <= 0 {
                    continue;
                }
                let c = freq.get(&p.champ).copied().unwrap_or_default();
                let total: u32 = c.iter().sum();
                score += ((c[lane] as f64 + 0.5) / (total as f64 + 2.5)).ln();
                match (p.smite, lane == 1) {
                    (Some(true), true) | (Some(false), false) => {}
                    (Some(_), _) => score -= 4.0,
                    (None, _) => {}
                }
            }
            if ok && best.map_or(true, |(b, _)| score > b) {
                best = Some((score, perm));
            }
            if !next_perm(&mut perm) {
                break;
            }
        }
        let Some((_, perm)) = best else { continue };
        for (j, &i) in idx.iter().enumerate().take(5) {
            let p = &mut players[i];
            let from_client = known[j].is_some();
            if p.champ <= 0 && !from_client {
                p.lane.clear();
                continue;
            }
            // Öffentliche Version: nur Positionen vom Client, keine geschätzten Lanes (noch nicht verlässlich genug)
            if !from_client {
                p.lane.clear();
                p.lane_guess = false;
                continue;
            }
            p.lane = LANES[perm[j]].to_string();
            p.lane_guess = !from_client;
        }
    }
}



/// Nächste Permutation in lexikografischer Reihenfolge; false, wenn es die letzte war.
fn next_perm(a: &mut [usize; 5]) -> bool {
    let Some(i) = (0..4).rev().find(|&i| a[i] < a[i + 1]) else { return false };
    let j = (i + 1..5).rev().find(|&j| a[j] > a[i]).unwrap();
    a.swap(i, j);
    a[i + 1..].reverse();
    true
}

/// Champion, der sich in deiner Elo auf deiner Hauptposition am meisten lohnt.
#[derive(Clone, Debug, PartialEq)]
pub struct EloPick {
    pub champ: i32,
    /// alle anderen Spieler nahe deiner Elo mit diesem Champion auf der Position
    pub elo: Agg,
    /// du selbst mit dem Champion auf der Position (letzte 100 Spiele)
    pub mine: Agg,
    /// alle Elos (Vorwissen, wenn in deiner Elo noch wenige Spiele sind)
    pub all: Agg,
    /// geschätzte Winrate in deiner Elo: Elo-Spiele, zur Winrate aller Elos gezogen
    pub wr: f64,
    pub score: f64,
}

impl EloPick {
    /// Weniger als 15 Spiele in der Elo: Schätzung stützt sich stark auf alle Elos.
    pub fn thin(&self) -> bool {
        self.elo.games < 15
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EloPicks {
    pub position: &'static str,
    /// false = für deine Elo noch zu wenige Matches, Werte aus allen Elos
    pub near_elo: bool,
    /// genutztes Elo-Fenster in LP (400 = ±1 Tier, 800 = ±2 Tiers), None = alle Elos
    pub span: Option<i32>,
    pub matches: usize,
    pub picks: Vec<EloPick>,
    /// Ersatz, solange es in der Elo noch keinen klaren Pick gibt: dein bester eigener Champion auf der Position
    pub own_best: Option<(i32, Agg)>,
}

/// Mindestspiele eines Champions im Elo-Fenster, damit er allein aus diesen Daten als Pick zählt.
const ELO_PICK_MIN: i32 = 5;
/// Alternativ: so viele Spiele über alle Elos (+ mind. 1 im Fenster) reichen als Kandidat.
const ALL_PICK_MIN: i32 = 20;
/// Eigene Spiele auf der Position, ab denen ein eigener Champion immer Kandidat ist.
const OWN_PICK_MIN: i32 = 3;
/// Spiele auf der Position, die ein Elo-Fenster mindestens braucht, bevor es genutzt wird.
const POS_MIN_GAMES: usize = 50;
/// Gewicht des Vorwissens aus allen Elos, gemessen in "Spielen"
const ELO_PRIOR_K: f64 = 15.0;

/// Beste Picks für die eigene Elo: Score = 60 % geschätzte Winrate im Elo-Fenster (ohne dich)
/// + 40 % deine eigene Winrate auf dem Champion. Die Elo-Winrate wird zur Winrate des Champions in
/// den übrigen Elos gezogen (diese wiederum zu 50 %), damit wenige Spiele nicht überbewertet werden.
///
/// Bei dünnen Daten (z. B. Mid) weicht die Funktion schrittweise aus: Elo-Fenster ±1 Tier, dann
/// ±2 Tiers, dann alle Elos – jeweils gemessen an den Spielen auf *dieser* Position.
/// Kandidaten: mind. 5 Spiele im Fenster, oder mind. 20 über alle Elos (+1 im Fenster), oder
/// mind. 3 eigene Spiele. Als Pick zählt, wer geschätzt ≥ 50 % gewinnt oder bei dir ≥ 55 % steht.
pub fn elo_picks(ms: &[Match], gs: &[GameStat], me: &str, mine_elo: Option<i32>, position: &'static str) -> EloPicks {
    let sr = |m: &&Match| SR_QUEUES.contains(&m.queue) && !m.remake();
    let count = |ms: &[&Match]| {
        let mut h: HashMap<i32, Agg> = HashMap::new();
        for m in ms {
            for p in m.ps.iter().filter(|p| p.puuid != me && p.position() == position) {
                h.entry(p.champ).or_default().add(p.win);
            }
        }
        h
    };
    let all_ms: Vec<&Match> = ms.iter().filter(sr).collect();
    let all = count(&all_ms);
    // kleinstes Elo-Fenster mit genug Spielen auf der Position
    let window = [400, 800].into_iter().find_map(|span| {
        let pool: Vec<&Match> = all_ms.iter().copied().filter(|m| near_elo(m, mine_elo, span)).collect();
        let h = count(&pool);
        let games: i32 = h.values().map(|a| a.games).sum();
        (games as usize >= POS_MIN_GAMES).then_some((span, pool, h))
    });
    let (span, pool_len, elo) = match window {
        Some((span, pool, h)) => (Some(span), pool.len(), h),
        None => (None, all_ms.len(), all.clone()),
    };
    let mut mine: HashMap<i32, Agg> = HashMap::new();
    for g in gs[gs.len().saturating_sub(100)..].iter().filter(|g| g.part.position() == position) {
        mine.entry(g.champ).or_default().add(g.win);
    }
    let mut champs: Vec<i32> = elo.keys().chain(mine.keys()).copied().collect();
    champs.sort_unstable();
    champs.dedup();
    let mut picks: Vec<EloPick> = champs
        .into_iter()
        .filter_map(|champ| {
            let a = elo.get(&champ).copied().unwrap_or_default();
            let m = mine.get(&champ).copied().unwrap_or_default();
            let al = all.get(&champ).copied().unwrap_or(a);
            let candidate = a.games >= ELO_PICK_MIN || (al.games >= ALL_PICK_MIN && a.games >= 1) || m.games >= OWN_PICK_MIN;
            if !candidate {
                return None;
            }
            // Vorwissen nur aus den Spielen außerhalb des Fensters, sonst zählen die Elo-Spiele doppelt
            let outside = if span.is_some() { Agg { games: al.games - a.games, wins: al.wins - a.wins } } else { Agg::default() };
            let prior = shrink(&outside, 10.0);
            let wr = (a.wins as f64 + ELO_PRIOR_K * prior) / (a.games as f64 + ELO_PRIOR_K);
            let own = shrink(&m, 5.0);
            // in deiner Elo voraussichtlich mind. die Hälfte gewonnen, oder bei dir selbst klar stark
            (wr >= 0.5 || own >= 0.55).then_some(EloPick { champ, elo: a, mine: m, all: al, wr, score: 0.6 * wr + 0.4 * own })
        })
        .collect();
    picks.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.champ.cmp(&b.champ)));
    picks.truncate(5);
    let own_best = mine
        .into_iter()
        .filter(|(_, a)| a.games >= 3)
        .max_by(|a, b| shrink(&a.1, 5.0).total_cmp(&shrink(&b.1, 5.0)).then(b.0.cmp(&a.0)));
    EloPicks { position, near_elo: span.is_some(), span, matches: pool_len, picks, own_best }
}

/// Deine Bilanz in Spielen, in denen `enemy` im Gegnerteam war.
pub fn wr_vs(gs: &[GameStat], enemy: i32) -> Agg {
    let mut a = Agg::default();
    for g in gs.iter().filter(|g| g.enemies.contains(&enemy)) {
        a.add(g.win);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coaching_splits_games_by_simple_traits() {
        // (Sieg, Tod-Zeitpunkte, CS-Vorsprung 10/15, Vision)
        let g = |win: bool, deaths: Vec<i32>, l10: i32, l15: i32, vision: i32| GameStat {
            win,
            mins: 30.0,
            d: deaths.len() as i32,
            part: Part { v: PART_V, vision, ..Default::default() },
            timeline: Some(Timeline {
                deaths,
                cs10: Some(80 + l10),
                opp_cs10: Some(80),
                cs15: Some(120 + l15),
                opp_cs15: Some(120),
                ..Default::default()
            }),
            ..Default::default()
        };
        let gs = vec![
            g(true, vec![900], 10, 5, 60),
            g(true, vec![], 5, 8, 50),
            g(false, vec![300, 1000, 1100], -5, -10, 10),
            g(false, vec![400, 1200], 4, -2, 20),
        ];
        let c = coaching(&gs, &HashMap::new());
        let clean = &c.lines[0];
        assert_eq!((clean.with, clean.without), ((2, 2), (2, 0)));
        assert_eq!(clean.gap(), None); // zu wenige Spiele
        assert_eq!(c.lead_games, 4);
        assert!((c.lead10 - 3.5).abs() < 1e-9);
        // Vorsprung bei 10 in 3 Spielen, geschrumpft in 2 davon
        assert!((c.lead_shrinks.unwrap() - 2.0 / 3.0).abs() < 1e-9);
        // wenig Vision (10, 20): 2 bzw. 1 Midgame-Tod; viel Vision: 1 bzw. 0
        assert_eq!(c.vision_games, 4);
        assert!((c.mid_deaths_low_vision - 1.5).abs() < 1e-9);
        assert!((c.mid_deaths_high_vision - 0.5).abs() < 1e-9);
        // frühe Tode: 0, 0, 1, 1 -> Ø 0,5; alle Spiele Champion 0, aber unter 3 Spielen je Gegner keine Gegner-Liste
        assert_eq!(c.early_avg, Some(0.5));
        assert_eq!(c.early_by_champ, vec![(0, 0.5, 4)]);
        assert!(c.early_by_opp.is_empty());
    }

    #[test]
    fn cs_curve_interpolates_own_history() {
        let g = |champ: i32, c: [i32; 3]| GameStat {
            champ,
            timeline: Some(Timeline { cs10: Some(c[0]), cs15: Some(c[1]), cs20: Some(c[2]), ..Default::default() }),
            ..Default::default()
        };
        let gs = vec![g(1, [60, 100, 140]), g(1, [80, 120, 160]), g(2, [40, 60, 80])];
        // nur 2 Spiele auf Champion 1 -> alle Spiele
        let c = cs_curve(&gs, 1).unwrap();
        assert!(!c.champ_only && c.games == 3);
        let gs = [gs.clone(), vec![g(1, [70, 110, 150])]].concat();
        let c = cs_curve(&gs, 1).unwrap();
        assert!(c.champ_only && c.games == 3);
        assert_eq!(c.at, [70.0, 110.0, 150.0]);
        assert!((c.expected(5.0) - 35.0).abs() < 1e-9);
        assert!((c.expected(12.5) - 90.0).abs() < 1e-9);
        assert!((c.expected(25.0) - 190.0).abs() < 1e-9);
        assert!(cs_curve(&[], 1).is_none());
    }

    #[test]
    fn tilt_counts_loss_streak_within_session() {
        const H: i64 = 3600 * 1000;
        let g = |h: i64, win: bool, early: usize| GameStat {
            ts: h * H,
            mins: 30.0,
            win,
            timeline: Some(Timeline { deaths: vec![300; early], ..Default::default() }),
            ..Default::default()
        };
        // Tag 1: N N S (S kommt nach 2 Niederlagen); Tag 2 (lange Pause): S N N
        let gs = vec![g(0, false, 1), g(1, false, 1), g(2, true, 0), g(30, true, 0), g(31, false, 3), g(32, false, 3)];
        let now = 32 * H + 40 * 60_000;
        let t = tilt(&gs, now).unwrap();
        assert_eq!(t.streak, 2);
        assert_eq!(t.session, (3, 1));
        assert_eq!(t.after, (1, 1)); // nur das Spiel nach N N am Tag 1
        assert_eq!(t.other, (5, 1));
        assert_eq!(t.early_streak, Some(3.0));
        assert!((t.early_all.unwrap() - 8.0 / 6.0).abs() < 1e-9);
        // letzte Spiel länger als 2 h her: kein Hinweis
        assert!(tilt(&gs, now + 3 * H).is_none());
        // Spielnummer in der Session: Tag 1 N N S, Tag 2 S N N
        assert_eq!(session_wr(&gs), [(2, 1), (2, 0), (2, 1), (0, 0)]);
        // lange Session: zu wenige Spiele ab dem 4. -> kein Hinweis
        assert!(long_session(&gs, &t).is_none());
        // 4 Sessions à 6 Spiele: Spiele 1–3 alle gewonnen, 4–6 alle verloren
        let mut many = Vec::new();
        for day in 0..4i64 {
            for k in 0..6i64 {
                many.push(g(day * 48 + k, k < 3, 0));
            }
        }
        let tl = Tilt { session: (3, 3), ..Default::default() };
        assert_eq!(long_session(&many, &tl), Some((0.0, 1.0, 12)));
        assert!(long_session(&many, &Tilt { session: (2, 2), ..Default::default() }).is_none());
    }

    fn info() -> HashMap<i32, ChampInfo> {
        let mut m = HashMap::new();
        for id in 1..=5 {
            m.insert(id, ChampInfo { dmg: DmgType::Ad, cc: 1, durability: 1, ..Default::default() });
        }
        for id in 6..=10 {
            m.insert(id, ChampInfo { dmg: if id % 2 == 0 { DmgType::Ap } else { DmgType::Ad }, cc: 2, durability: 2, ..Default::default() });
        }
        m
    }

    #[test]
    fn comp_profile_and_buckets() {
        let i = info();
        let c = Comp::of(1..=5, &i);
        assert_eq!((c.ad, c.ap, c.cc), (5, 0, 5));
        assert_eq!(c.profile(), Profile::AdHeavy);
        // Team 100: Champs 1-5 (AD-lastig, CC 5) verliert gegen Team 200: 6-10 (gemischt, CC 10)
        let ps = (1..=10)
            .map(|c| Part { champ: c, team: if c <= 5 { 100 } else { 200 }, win: c > 5, ..Default::default() })
            .collect();
        let m = Match { id: 1, queue: 420, dur: 1800, ps, ..Default::default() };
        let s = comp_stats(&[m], &i);
        assert_eq!(s.matches, 1);
        assert_eq!(s.profile[1].1, Agg { games: 1, wins: 0 }); // AD-lastig verloren
        assert_eq!(s.profile[0].1, Agg { games: 1, wins: 1 }); // ausgewogen gewonnen
        assert_eq!(s.cc[0].1, Agg { games: 1, wins: 1 }); // mehr CC gewonnen
        assert_eq!(s.cc[2].1, Agg { games: 1, wins: 0 });
    }

    #[test]
    fn elo_filter() {
        let m = Match { elo: Some(1000), ..Default::default() };
        assert!(near_elo(&m, Some(1300), 400));
        assert!(!near_elo(&m, Some(1500), 400));
        assert!(!near_elo(&Match::default(), Some(1000), 400));
    }

    #[test]
    fn deaths_pool_and_rolling() {
        let g = |id: u64, champ: i32, win: bool, deaths: Vec<i32>| GameStat {
            id,
            champ,
            win,
            mins: 30.0,
            k: 3,
            d: 2,
            a: 3,
            cs: 180,
            timeline: Some(Timeline { match_id: id, deaths, cs10: Some(60), gold10: Some(3000), ..Default::default() }),
            ..Default::default()
        };
        let gs = vec![g(1, 7, true, vec![300, 1000]), g(2, 7, false, vec![1600]), g(3, 9, true, vec![])];
        let d = death_stats(&gs);
        assert_eq!(d.games, 3);
        assert!((d.early - 1.0 / 3.0).abs() < 1e-9);
        assert!((d.mid - 1.0 / 3.0).abs() < 1e-9);
        assert!((d.late - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(d.first_death, Some((300.0 + 1600.0) / 2.0));
        assert!((d.clean_10 - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(d.cs10, Some(60.0));

        let p = pool(&gs);
        assert_eq!((p.games, p.distinct), (3, 2));
        assert_eq!(p.lines[0].champ, 7);
        assert!((p.top3 - 1.0).abs() < 1e-9);
        assert!((gs[0].csm() - 6.0).abs() < 1e-9);

        assert_eq!(rolling(&[1.0, 3.0, 5.0], 2), vec![1.0, 2.0, 4.0]);
    }

    #[test]
    fn timeline_parsing() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"frames":[
                {"timestamp":0,"participantFrames":{"3":{"minionsKilled":0,"jungleMinionsKilled":0,"totalGold":500}},"events":[]},
                {"timestamp":540000,"participantFrames":{},"events":[{"type":"CHAMPION_KILL","timestamp":420500,"victimId":3},
                                                                     {"type":"CHAMPION_KILL","timestamp":430000,"victimId":4}]},
                {"timestamp":600123,"participantFrames":{"3":{"minionsKilled":71,"jungleMinionsKilled":4,"totalGold":3600}},"events":[]}
            ]}"#,
        )
        .unwrap();
        let t = Timeline::from_lcu(9, 3, None, &v);
        assert_eq!(t.deaths, vec![420]);
        assert_eq!((t.cs10, t.gold10), (Some(75), Some(3600)));
        assert_eq!((t.cs15, t.opp_cs10), (None, None));
    }

    #[test]
    fn timeline_with_opponent_and_farm_diff() {
        let frame = |min: i64, me: i32, opp: i32| {
            format!(
                r#"{{"timestamp":{},"participantFrames":{{"1":{{"minionsKilled":{me},"jungleMinionsKilled":0,"totalGold":1}},
                   "6":{{"minionsKilled":{opp},"jungleMinionsKilled":0,"totalGold":1}}}},"events":[]}}"#,
                min * 60_000 + 500
            )
        };
        let json = format!(r#"{{"frames":[{},{},{}]}}"#, frame(10, 80, 70), frame(15, 120, 125), frame(20, 160, 150));
        let t = Timeline::from_lcu(1, 1, Some(6), &serde_json::from_str(&json).unwrap());
        assert_eq!((t.cs10, t.opp_cs10, t.cs15, t.opp_cs15, t.cs20), (Some(80), Some(70), Some(120), Some(125), Some(160)));
        let g = GameStat { timeline: Some(t), mins: 30.0, ..Default::default() };
        let f = farm(&[g]);
        assert_eq!((f.diff10, f.diff15, f.diff_games), (Some(10.0), Some(-5.0), 1));
        assert_eq!(f.cs20, Some(160.0));
    }

    fn full_part(puuid: &str, champ: i32, team: i32, win: bool, lane: &str, role: &str) -> Part {
        Part {
            puuid: puuid.into(),
            champ,
            team,
            win,
            lane: lane.into(),
            role: role.into(),
            v: PART_V,
            k: 5,
            d: 2,
            a: 5,
            cs: 200,
            dmg: 20000,
            gold: 12000,
            vision: 30,
            dmg_taken: 15000,
            ..Default::default()
        }
    }

    #[test]
    fn afk_teammate_detection() {
        // Werte aus dem echten Spiel 8022989218: Jax 4274 Gold, Support 6856 Gold
        let p = |puuid: &str, champ: i32, gold: i32, team: i32| Part { puuid: puuid.into(), champ, gold, team, ..Default::default() };
        let ps = vec![
            p("jax", 24, 4274, 100),
            p("me", 234, 10429, 100),
            p("a", 101, 10619, 100),
            p("b", 523, 11197, 100),
            p("sup", 40, 6856, 100),
            p("x", 1, 3000, 200), // Gegner zählt nicht
        ];
        let m = Match { ps, ..Default::default() };
        assert_eq!(m.afk_teammate("me").unwrap().champ, 24);
        // ohne den AFK-Spieler: der Support mit wenig Gold gilt nicht als AFK
        let m2 = Match { ps: m.ps.iter().filter(|x| x.puuid != "jax").cloned().collect(), ..Default::default() };
        assert!(m2.afk_teammate("me").is_none());
    }

    #[test]
    fn positions_and_opponent() {
        let ps = vec![
            full_part("me", 1, 100, true, "BOTTOM", "DUO_CARRY"),
            full_part("sup", 2, 100, true, "BOTTOM", "DUO_SUPPORT"),
            full_part("x", 3, 200, false, "BOTTOM", "DUO_CARRY"),
            full_part("y", 4, 200, false, "BOTTOM", "DUO_SUPPORT"),
        ];
        let m = Match { ps, ..Default::default() };
        assert_eq!(m.part("me").unwrap().position(), "ADC");
        assert_eq!(m.opponent("me").unwrap().champ, 3);
        assert_eq!(m.opponent("sup").unwrap().champ, 4);
        assert_eq!(position_from_select("utility"), "SUPPORT");
    }

    #[test]
    fn records_and_streaks() {
        let g = |win: bool, k: i32, ts: i64| GameStat {
            win,
            k,
            ts,
            champ: 7,
            mins: 30.0,
            cs: 240,
            part: Part { v: PART_V, pentas: if k > 10 { 1 } else { 0 }, vision: k, ..Default::default() },
            ..Default::default()
        };
        let gs = vec![g(true, 3, 1), g(true, 12, 2), g(true, 4, 3), g(false, 1, 4), g(false, 2, 5)];
        let snaps = vec![
            LpSnap { ts: 10, rank: Rank { tier: "SILVER".into(), div: "I".into(), lp: 90, ..Default::default() } },
            LpSnap { ts: 20, rank: Rank { tier: "SILVER".into(), div: "I".into(), lp: 40, ..Default::default() } },
        ];
        let r = records(&gs, &snaps, Season::default());
        assert_eq!((r.win_streak, r.loss_streak), (3, 2));
        assert_eq!(r.kills.unwrap(), Best { value: 12.0, champ: 7, ts: 2 });
        assert_eq!(r.pentas, 1);
        assert_eq!(r.peak, Some((800 + 300 + 90, 10)));
        assert!((r.csm.unwrap().value - 8.0).abs() < 1e-9);
    }

    #[test]
    fn style_against_peers() {
        let mut ms = Vec::new();
        for i in 0..3 {
            let mut ps = vec![full_part("me", 1, 100, true, "MIDDLE", "SOLO")];
            ps.extend((1..5).map(|j| full_part(&format!("a{j}"), 10 + j, 100, true, "TOP", "SOLO")));
            ps.push(Part { vision: 10, ..full_part(&format!("p{i}"), 20, 200, false, "MIDDLE", "SOLO") });
            ps.extend((1..5).map(|j| full_part(&format!("b{j}"), 30 + j, 200, false, "TOP", "SOLO")));
            ms.push(Match { id: i, ts: i as i64, queue: 420, dur: 1800, ps, ..Default::default() });
        }
        let s = style(&ms, "me", None).unwrap();
        assert_eq!((s.position, s.games, s.peers, s.near_elo), ("MID", 3, 3, false));
        // Kill-Beteiligung: (5+5) / (5 Spieler * 5 Kills) = 40 %
        assert!((s.mine[0] - 0.4).abs() < 1e-9);
        // Vision/min: 30/30 = 1.0 gegen 10/30
        assert!((s.mine[4] - 1.0).abs() < 1e-9);
        assert!((s.peer.unwrap()[4] - 10.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn recommendation_ranks_pool() {
        let g = |champ: i32, win: bool, enemies: Vec<i32>, pos: &str| GameStat {
            champ,
            win,
            enemies,
            d: 1,
            k: 3,
            part: Part { lane: pos.into(), role: "SOLO".into(), ..Default::default() },
            ..Default::default()
        };
        let mut gs = Vec::new();
        for i in 0..6 {
            gs.push(g(1, i < 5, vec![50], "MIDDLE")); // stark auf 1, mid
        }
        for i in 0..6 {
            gs.push(g(2, i < 2, vec![60], "MIDDLE")); // schwach auf 2
        }
        gs.push(g(3, true, vec![50], "MIDDLE")); // zu wenige Spiele
        let mut mu = Matchups::new();
        mu.insert((2, 50), Agg { games: 40, wins: 32 }); // 2 kontert 50 stark
        mu.insert((1, 50), Agg { games: 40, wins: 16 });
        let r = recommend(&gs, &mu, &[50], "MID");
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.champ != 3));
        let c1 = r.iter().find(|x| x.champ == 1).unwrap();
        assert_eq!((c1.mine.games, c1.vs_me.games, c1.on_pos), (6, 6, 6));
        assert_eq!(wr_vs(&gs, 50), Agg { games: 7, wins: 6 });
        assert!((shrink(&Agg::default(), 5.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn match_summary_against_previous_games() {
        let g = |id: u64, k: i32, d: i32, cs: i32| GameStat {
            id,
            ts: id as i64,
            mins: 30.0,
            k,
            d,
            a: 0,
            cs,
            part: Part { gold: 12000, dmg: 18000, vision: 30, ..Default::default() },
            ..Default::default()
        };
        let gs = vec![g(1, 4, 4, 180), g(2, 6, 6, 180), g(3, 10, 2, 240), g(4, 0, 9, 90)];
        // Spiel 3 gegen den Schnitt von 1 und 2
        let (n, l) = match_summary(&gs, 3).unwrap();
        assert_eq!(n, 2);
        let kda = &l[0];
        assert!((kda.value - 5.0).abs() < 1e-9 && (kda.avg - 1.0).abs() < 1e-9);
        assert_eq!(kda.verdict(), 1);
        let deaths = l.iter().find(|x| x.label == "Tode").unwrap();
        assert_eq!(deaths.verdict(), 1); // weniger Tode = besser
        let gold = l.iter().find(|x| x.label == "Gold/min").unwrap();
        assert_eq!(gold.verdict(), 0); // gleich
        assert!(l.iter().all(|x| x.label != "CS nach 10 min")); // keine Timelines
        // erstes Spiel hat keinen Vergleich, unbekanntes Spiel auch nicht
        assert!(match_summary(&gs, 1).is_none());
        assert!(match_summary(&gs, 99).is_none());
        assert_eq!(match_summary(&gs, 4).unwrap().1[2].verdict(), -1); // 9 Tode
    }

    #[test]
    fn lanes_only_from_client() {
        let mut f = LaneFreq::new();
        f.insert(1, [20, 0, 0, 0, 0]);
        let pl = |champ: i32, pos: &str| LivePlayer { champ, team: 100, position: pos.into(), ..Default::default() };
        // Client-Position wird übernommen, ohne Position bleibt die Lane leer (keine Schätzung)
        let mut ps = vec![pl(3, "utility"), pl(1, ""), pl(5, "middle")];
        fill_lanes(&mut ps, &f, 420);
        assert_eq!((ps[0].lane.as_str(), ps[0].lane_guess), ("SUPPORT", false));
        assert_eq!(ps[1].lane, "");
        assert_eq!(ps[2].lane, "MID");
        assert!(ps.iter().all(|p| !p.lane_guess));
    }


    #[test]
    fn elo_picks_prefer_strong_champs_in_my_elo() {
        let mid = |champ: i32, win: bool, puuid: &str| Part {
            champ,
            win,
            puuid: puuid.into(),
            lane: "MIDDLE".into(),
            role: "SOLO".into(),
            ..Default::default()
        };
        let mut ms = Vec::new();
        for i in 0..60u64 {
            // nahe meiner Elo: Champion 1 gewinnt 70 %, Champion 2 nur 30 %; ich selbst zähle nicht mit
            let ps = vec![mid(1, i % 10 < 7, "a"), mid(2, i % 10 < 3, "b"), mid(1, false, "me")];
            ms.push(Match { id: i, queue: 420, dur: 1800, elo: Some(1000), ps, ..Default::default() });
        }
        // weit weg von meiner Elo: Champion 2 gewinnt immer
        for i in 0..40u64 {
            ms.push(Match { id: 100 + i, queue: 420, dur: 1800, elo: Some(2500), ps: vec![mid(2, true, "c")], ..Default::default() });
        }
        let r = elo_picks(&ms, &[], "me", Some(1100), "MID");
        assert!(r.near_elo);
        assert_eq!(r.matches, 60);
        assert_eq!(r.picks[0].champ, 1);
        assert_eq!(r.picks[0].elo, Agg { games: 60, wins: 42 });
        // Champion 2 gewinnt in meiner Elo nur 30 % (trotz 100 % in hoher Elo) -> kein Pick
        assert_eq!(r.picks.len(), 1);
        assert!(!r.picks[0].thin());
        assert!(r.picks[0].wr > 0.6 && r.picks[0].wr < 0.7);
        // wenige Elo-Spiele: 4 Siege aus 5 werden zur Winrate aller Elos gezogen
        // (in anderen Elos gewinnt Champion 3 nur 10 von 20) -> geschätzt ~58 % statt 80 %
        let mut few: Vec<Match> = (0..60u64)
            .map(|i| {
                let ps = if i < 5 { vec![mid(3, i < 4, "d")] } else { vec![mid(1, true, "a")] };
                Match { id: 500 + i, queue: 420, dur: 1800, elo: Some(1000), ps, ..Default::default() }
            })
            .collect();
        for i in 0..20u64 {
            few.push(Match { id: 700 + i, queue: 420, dur: 1800, elo: Some(2500), ps: vec![mid(3, i < 10, "e")], ..Default::default() });
        }
        let r3 = elo_picks(&few, &[], "me", Some(1100), "MID");
        let p3 = r3.picks.iter().find(|p| p.champ == 3).unwrap();
        assert!(p3.thin() && p3.wr < 0.6 && p3.wr >= 0.5);
        assert_eq!(r.own_best, None);
        // Ersatz aus eigenen Spielen: bester eigener Champion auf der Position (ab 3 Spielen)
        let g = |champ: i32, win: bool| GameStat { champ, win, part: mid(champ, win, "me"), ..Default::default() };
        let gs = vec![g(7, true), g(7, true), g(7, false), g(8, false), g(8, false), g(8, true), g(9, true)];
        assert_eq!(elo_picks(&ms, &gs, "me", Some(1100), "MID").own_best, Some((7, Agg { games: 3, wins: 2 })));
        // andere Position: keine Picks
        assert!(elo_picks(&ms, &[], "me", Some(1100), "TOP").picks.is_empty());
        // ohne genug Matches in der Elo: alle Elos
        assert!(!elo_picks(&ms[..10], &[], "me", Some(1100), "MID").near_elo);
    }

    #[test]
    fn elo_picks_widen_window_on_thin_position() {
        let mid = |champ: i32, win: bool, puuid: &str| Part {
            champ,
            win,
            puuid: puuid.into(),
            lane: "MIDDLE".into(),
            role: "SOLO".into(),
            ..Default::default()
        };
        // ±1 Tier: nur 30 Mid-Spiele (unter 50), ±2 Tiers: zusätzlich 60 -> Fenster ±2 Tiers
        let mut ms: Vec<Match> = (0..30u64)
            .map(|i| Match { id: i, queue: 420, dur: 1800, elo: Some(1100), ps: vec![mid(1, i % 3 != 0, "a")], ..Default::default() })
            .collect();
        ms.extend((0..60u64).map(|i| Match { id: 100 + i, queue: 420, dur: 1800, elo: Some(1700), ps: vec![mid(1, i % 3 != 0, "b")], ..Default::default() }));
        let r = elo_picks(&ms, &[], "me", Some(1100), "MID");
        assert_eq!((r.span, r.near_elo, r.matches), (Some(800), true, 90));
        assert_eq!(r.picks[0].champ, 1);
        // eigener starker Champion zählt, auch ohne Spiele anderer in der Elo
        let g = |win: bool| GameStat { champ: 9, win, part: mid(9, win, "me"), ..Default::default() };
        let gs = vec![g(true), g(true), g(true), g(false)];
        let r = elo_picks(&ms, &gs, "me", Some(1100), "MID");
        let own = r.picks.iter().find(|p| p.champ == 9).expect("eigener Champion als Kandidat");
        assert!(own.thin() && own.mine == Agg { games: 4, wins: 3 });
        // Champion mit vielen Spielen über alle Elos, aber nur 2 im Fenster: Kandidat, als dünn markiert
        let mut ms2 = ms.clone();
        ms2.extend((0..2u64).map(|i| Match { id: 300 + i, queue: 420, dur: 1800, elo: Some(1100), ps: vec![mid(5, true, "c")], ..Default::default() }));
        ms2.extend((0..30u64).map(|i| Match { id: 400 + i, queue: 420, dur: 1800, elo: Some(2800), ps: vec![mid(5, i % 5 != 0, "d")], ..Default::default() }));
        let r = elo_picks(&ms2, &[], "me", Some(1100), "MID");
        let p5 = r.picks.iter().find(|p| p.champ == 5).expect("Champion 5 als Kandidat");
        assert!(p5.thin() && p5.elo.games == 2 && p5.wr > 0.5);
    }
}
