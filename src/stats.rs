//! Auswertungen: Builds/Runen pro Champion, eigene Champion-Statistik, LP-Verlauf.

use crate::model::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::Hash;

pub const SR_QUEUES: [i32; 5] = [400, 420, 430, 440, 490];

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Agg {
    pub games: i32,
    pub wins: i32,
}

impl Agg {
    pub fn add(&mut self, win: bool) {
        self.games += 1;
        if win {
            self.wins += 1;
        }
    }
    pub fn wr(&self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            self.wins as f64 / self.games as f64
        }
    }
    /// Untere Wilson-Grenze: bestraft kleine Stichproben.
    pub fn score(&self) -> f64 {
        wilson(self.wins, self.games)
    }
}

pub fn wilson(wins: i32, games: i32) -> f64 {
    if games == 0 {
        return 0.0;
    }
    let (z, n) = (1.0f64, games as f64);
    let p = wins as f64 / n;
    (p + z * z / (2.0 * n) - z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt()) / (1.0 + z * z / n)
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RunePage {
    pub styles: [i32; 2],
    pub perks: [i32; 6],
    pub shards: [i32; 3],
}

pub struct Build {
    pub champ: i32,
    pub patch: String,
    pub total: Agg,
    pub page: Option<(RunePage, Agg)>,
    pub spells: Option<([i32; 2], Agg)>,
    /// fertige Items ohne Schuhe
    pub items: Vec<(i32, Agg)>,
    /// meistgespielte Schuhe (getrennt, weil sie sich auf viele Sorten verteilen)
    pub boots: Option<(i32, Agg)>,
}

fn sr(m: &Match) -> bool {
    SR_QUEUES.contains(&m.queue) && !m.remake()
}

fn sample<'a>(ms: &'a [Match], champ: i32, patch: Option<&str>) -> Vec<&'a Part> {
    ms.iter()
        .filter(|m| sr(m) && patch.map_or(true, |p| m.patch == p))
        .flat_map(|m| m.ps.iter())
        .filter(|p| p.champ == champ)
        .collect()
}

pub fn latest_patch(ms: &[Match]) -> String {
    ms.iter().max_by_key(|m| m.ts).map(|m| m.patch.clone()).unwrap_or_default()
}

fn most_played<K: Clone + Eq + Hash>(map: &HashMap<K, Agg>) -> Option<(K, Agg)> {
    map.iter()
        .max_by(|a, b| {
            a.1.games
                .cmp(&b.1.games)
                .then(a.1.score().partial_cmp(&b.1.score()).unwrap_or(Ordering::Equal))
        })
        .map(|(k, v)| (k.clone(), *v))
}

/// `completed` = fertiges Item, `boots` = Schuhe (werden gesondert ausgewertet).
pub fn build_for(ms: &[Match], champ: i32, completed: &dyn Fn(i32) -> bool, boots: &dyn Fn(i32) -> bool) -> Build {
    let patch = latest_patch(ms);
    let mut parts = sample(ms, champ, Some(&patch));
    let mut label = format!("Patch {patch}");
    if parts.len() < 15 {
        parts = sample(ms, champ, None);
        label = "alle Patches".into();
    }
    let mut total = Agg::default();
    let mut keystones: HashMap<i32, Agg> = HashMap::new();
    let mut spells: HashMap<[i32; 2], Agg> = HashMap::new();
    let mut items: HashMap<i32, Agg> = HashMap::new();
    let mut shoes: HashMap<i32, Agg> = HashMap::new();
    for p in &parts {
        total.add(p.win);
        if p.perks[0] != 0 {
            keystones.entry(p.perks[0]).or_default().add(p.win);
        }
        let mut sp = p.spells;
        sp.sort();
        if sp[0] != 0 {
            spells.entry(sp).or_default().add(p.win);
        }
        let mut seen = Vec::new();
        for &it in &p.items[..6] {
            if it != 0 && completed(it) && !seen.contains(&it) {
                seen.push(it);
                let map = if boots(it) { &mut shoes } else { &mut items };
                map.entry(it).or_default().add(p.win);
            }
        }
    }
    // Keystone nach Winrate (Wilson), dann die häufigste komplette Seite mit diesem Keystone.
    let page = keystones
        .iter()
        .max_by(|a, b| {
            a.1.score()
                .partial_cmp(&b.1.score())
                .unwrap_or(Ordering::Equal)
                .then(a.1.games.cmp(&b.1.games))
        })
        .and_then(|(&key, _)| {
            let mut pages: HashMap<RunePage, Agg> = HashMap::new();
            for p in parts.iter().filter(|p| p.perks[0] == key) {
                pages
                    .entry(RunePage { styles: p.styles, perks: p.perks, shards: p.shards })
                    .or_default()
                    .add(p.win);
            }
            most_played(&pages)
        });
    let mut items: Vec<(i32, Agg)> = items.into_iter().collect();
    items.sort_by(|a, b| b.1.games.cmp(&a.1.games).then(a.0.cmp(&b.0)));
    items.truncate(10);
    Build { champ, patch: label, total, page, spells: most_played(&spells), items, boots: most_played(&shoes) }
}

/// Runen gegen einen bestimmten Lane-Gegner im Vergleich zu allen Spielen des Champions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchupRunes {
    /// Spiele des Champions gegen genau diesen Lane-Gegner
    pub vs: Agg,
    /// alle Spiele des Champions
    pub all: usize,
    /// Runen, die gegen den Gegner deutlich häufiger gewählt werden: (Rune, Anteil gegen ihn, Anteil sonst, Bilanz damit)
    pub notable: Vec<(i32, f64, f64, Agg)>,
    /// meistgespielte komplette Runenseite gegen genau diesen Gegner (zum Importieren)
    pub page: Option<(RunePage, Agg)>,
}

/// Ab so vielen Spielen gegen den Gegner werden Runen verglichen.
pub const MATCHUP_RUNES_MIN: i32 = 8;
/// Mindestens so viel häufiger (Anteil) muss eine Rune gegen den Gegner sein.
pub const MATCHUP_RUNE_LIFT: f64 = 0.15;

pub fn matchup_runes(ms: &[Match], champ: i32, opp: i32) -> MatchupRunes {
    let mut r = MatchupRunes::default();
    let mut count_vs: HashMap<i32, Agg> = HashMap::new();
    let mut count_all: HashMap<i32, usize> = HashMap::new();
    let mut pages: HashMap<RunePage, Agg> = HashMap::new();
    for m in ms.iter().filter(|m| sr(m)) {
        for p in m.ps.iter().filter(|p| p.champ == champ && p.perks[0] != 0) {
            r.all += 1;
            for &k in &p.perks {
                *count_all.entry(k).or_default() += 1;
            }
            if m.opponent(&p.puuid).is_some_and(|o| o.champ == opp) {
                r.vs.add(p.win);
                for &k in &p.perks {
                    count_vs.entry(k).or_default().add(p.win);
                }
                if p.styles[0] != 0 && p.shards.iter().all(|&s| s != 0) {
                    pages.entry(RunePage { styles: p.styles, perks: p.perks, shards: p.shards }).or_default().add(p.win);
                }
            }
        }
    }
    if r.vs.games < MATCHUP_RUNES_MIN {
        return r;
    }
    // häufigste komplette Seite, aber nur, wenn sie wirklich mehrfach gespielt wurde
    r.page = most_played(&pages).filter(|(_, a)| a.games >= 3);
    let others = (r.all as i32 - r.vs.games).max(1) as f64;
    r.notable = count_vs
        .into_iter()
        .filter(|(_, a)| a.games >= 3)
        .map(|(k, a)| {
            let share_vs = a.games as f64 / r.vs.games as f64;
            let share_other = (count_all.get(&k).copied().unwrap_or(0) as f64 - a.games as f64) / others;
            (k, share_vs, share_other, a)
        })
        .filter(|x| x.1 - x.2 >= MATCHUP_RUNE_LIFT)
        .collect();
    r.notable.sort_by(|a, b| (b.1 - b.2).total_cmp(&(a.1 - a.2)).then(a.0.cmp(&b.0)));
    r.notable.truncate(3);
    r
}

/// Alle Champions in der DB mit Anzahl Spielen (für die Build-Liste).
pub fn champ_list(ms: &[Match]) -> Vec<(i32, Agg)> {
    let mut map: HashMap<i32, Agg> = HashMap::new();
    for m in ms.iter().filter(|m| sr(m)) {
        for p in &m.ps {
            map.entry(p.champ).or_default().add(p.win);
        }
    }
    let mut v: Vec<(i32, Agg)> = map.into_iter().filter(|(c, _)| *c > 0).collect();
    v.sort_by(|a, b| b.1.games.cmp(&a.1.games));
    v
}

#[derive(Clone, Default, Debug)]
pub struct ChampLine {
    pub champ: i32,
    pub agg: Agg,
    pub k: i32,
    pub d: i32,
    pub a: i32,
    pub cs: i32,
    pub mins: f64,
}

impl ChampLine {
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

pub fn my_champs(ms: &[Match], me: &str) -> Vec<ChampLine> {
    let mut map: HashMap<i32, ChampLine> = HashMap::new();
    for m in ms.iter().filter(|m| sr(m)) {
        if let Some(p) = m.part(me) {
            let l = map.entry(p.champ).or_insert_with(|| ChampLine { champ: p.champ, ..Default::default() });
            l.agg.add(p.win);
            l.k += p.k;
            l.d += p.d;
            l.a += p.a;
            l.cs += p.cs;
            l.mins += m.dur as f64 / 60.0;
        }
    }
    let mut v: Vec<ChampLine> = map.into_values().collect();
    v.sort_by(|a, b| b.agg.games.cmp(&a.agg.games));
    v
}

/// Ein Solo/Duo-Spiel mit erfasster Lobby und bekanntem LP-Stand vorher/nachher.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameRec {
    pub game_id: u64,
    pub ts: i64,
    pub win: bool,
    pub delta: i32,
    /// eigener Rang (LP-Skala) vor dem Spiel
    pub rank: i32,
    /// Schnitt der anderen 9
    pub lobby: i32,
    pub team: Option<i32>,
    pub enemy: Option<i32>,
}

pub fn team_avg(l: &Lobby, team: i32) -> Option<i32> {
    let v: Vec<i32> = l
        .players
        .iter()
        .filter(|p| !p.me && p.team == team)
        .filter_map(|p| p.rank.as_ref()?.value())
        .collect();
    (v.len() >= 2).then(|| v.iter().sum::<i32>() / v.len() as i32)
}

/// Ordnet jeder erfassten Solo/Duo-Lobby den LP-Stand davor und danach zu.
/// Spiele mit Abstiegsschutz (Niederlage ohne LP-Verlust) zählen nicht als Messpunkt.
pub fn game_records(lobbies: &[Lobby], snaps: &[LpSnap]) -> Vec<GameRec> {
    let solo: Vec<&Lobby> = lobbies
        .iter()
        .filter(|l| l.queue == 420 && !l.champ_select && l.game_id != 0 && !l.remake)
        .collect();
    let mut out = Vec::new();
    for (i, l) in solo.iter().enumerate() {
        let next_ts = solo.get(i + 1).map_or(i64::MAX, |n| n.ts);
        let Some(pi) = snaps.iter().rposition(|s| s.ts <= l.ts) else { continue };
        let pre = &snaps[pi];
        let Some(qi) = snaps
            .iter()
            .position(|s| s.ts > l.ts && s.ts < next_ts && s.rank.games() == pre.rank.games() + 1)
        else {
            continue;
        };
        let post = &snaps[settled(snaps, pi, qi)];
        let (Some(a), Some(b)) = (pre.rank.value(), post.rank.value()) else { continue };
        let win = post.rank.wins > pre.rank.wins;
        let delta = b - a;
        if delta.abs() > 60 || (!win && delta == 0) {
            continue;
        }
        let Some(lobby) = l.avg_others() else { continue };
        let my_team = l.players.iter().find(|p| p.me).map_or(0, |p| p.team);
        out.push(GameRec {
            game_id: l.game_id,
            ts: l.ts,
            win,
            delta,
            rank: a,
            lobby,
            team: team_avg(l, my_team),
            enemy: team_avg(l, if my_team == 200 { 100 } else { 200 }),
        });
    }
    out
}

/// LP-Änderungen zwischen aufeinanderfolgenden Snapshots mit genau einem Spiel Abstand.
/// Liefert (Zeit, Delta, Sieg).
#[cfg(test)]
pub fn lp_deltas(snaps: &[LpSnap]) -> Vec<(i64, i32, bool)> {
    (1..snaps.len())
        .filter_map(|i| {
            let (a, b) = (&snaps[i - 1].rank, &snaps[i].rank);
            if b.games() != a.games() + 1 {
                return None;
            }
            let b = &snaps[settled(snaps, i - 1, i)].rank;
            let d = b.value()? - a.value()?;
            (d.abs() <= 60).then_some((snaps[i].ts, d, b.wins > a.wins))
        })
        .collect()
}

/// Riot zählt Sieg/Niederlage manchmal ein paar Sekunden vor der LP-Änderung.
/// Hat der Snapshot nach dem Spiel (`post`) noch dieselben LP wie davor (`pre`), gehört ein
/// kurz darauf folgender Snapshot mit gleicher Spielanzahl zu diesem Spiel. Liefert dessen Index.
pub const LATE_LP_MS: i64 = 30 * 60 * 1000;

pub fn settled(snaps: &[LpSnap], pre: usize, post: usize) -> usize {
    let (a, b) = (&snaps[pre].rank, &snaps[post].rank);
    if a.value() != b.value() {
        return post;
    }
    match snaps.get(post + 1) {
        Some(n) if n.rank.games() == b.games() && n.ts - snaps[post].ts <= LATE_LP_MS => post + 1,
        _ => post,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(champ: i32, win: bool, keystone: i32, items: [i32; 7]) -> Part {
        Part {
            champ,
            win,
            perks: [keystone, 1, 2, 3, 4, 5],
            styles: [8000, 8100],
            spells: [7, 4],
            items,
            ..Default::default()
        }
    }

    fn game(id: u64, ps: Vec<Part>) -> Match {
        Match { id, ts: id as i64, queue: 420, dur: 1800, patch: "14.20".into(), ps, elo: None }
    }

    #[test]
    fn build_prefers_winning_keystone() {
        let mut ms = Vec::new();
        for i in 0..10 {
            ms.push(game(i, vec![part(145, i < 8, 8005, [3031, 1001, 0, 0, 0, 0, 3340])]));
        }
        for i in 10..14 {
            ms.push(game(i, vec![part(145, false, 8021, [3031, 0, 0, 0, 0, 0, 0])]));
        }
        let b = build_for(&ms, 145, &|id| id == 3031 || id == 1001, &|id| id == 1001);
        // 1001 zählt hier als Schuh: getrennt ausgewiesen, nicht in der Item-Liste
        assert_eq!(b.boots, Some((1001, Agg { games: 10, wins: 8 })));
        assert_eq!(b.total.games, 14);
        let (pg, agg) = b.page.unwrap();
        assert_eq!(pg.perks[0], 8005);
        assert_eq!(agg.games, 10);
        assert_eq!(b.items, vec![(3031, Agg { games: 14, wins: 8 })]);
        assert_eq!(b.spells.unwrap().0, [4, 7]);
        assert_eq!(b.patch, "alle Patches"); // < 15 Spiele auf dem Patch
    }

    #[test]
    fn matchup_runes_find_runes_picked_more_against_opponent() {
        let mid = |champ: i32, team: i32, puuid: &str, perks: [i32; 6], win: bool| Part {
            champ,
            team,
            win,
            puuid: puuid.into(),
            perks,
            styles: [8100, 8300],
            shards: [5008, 5008, 5001],
            lane: "MIDDLE".into(),
            ..Default::default()
        };
        let mut ms = Vec::new();
        // 10 Spiele Champion 1 gegen Champion 2: Rune 77 (Knochenpanzer) in 8 davon
        for i in 0..10u64 {
            let r = if i < 8 { 77 } else { 5 };
            ms.push(game(i, vec![mid(1, 100, &format!("a{i}"), [8010, 1, 2, r, 4, 6], i < 6), mid(2, 200, &format!("b{i}"), [8112, 0, 0, 0, 0, 0], i >= 6)]));
        }
        // 20 Spiele gegen andere: Rune 77 nur in 2
        for i in 10..30u64 {
            let r = if i < 12 { 77 } else { 5 };
            ms.push(game(i, vec![mid(1, 100, &format!("a{i}"), [8010, 1, 2, r, 4, 6], true), mid(3, 200, &format!("c{i}"), [8112, 0, 0, 0, 0, 0], false)]));
        }
        let r = matchup_runes(&ms, 1, 2);
        assert_eq!((r.vs, r.all), (Agg { games: 10, wins: 6 }, 30));
        assert_eq!(r.notable.len(), 1);
        let (k, vs, other, agg) = r.notable[0];
        assert_eq!(k, 77);
        assert!((vs - 0.8).abs() < 1e-9 && (other - 0.1).abs() < 1e-9);
        assert_eq!(agg, Agg { games: 8, wins: 6 });
        // Importseite: die meistgespielte komplette Seite gegen diesen Gegner (mit Rune 77)
        let (page, pagg) = r.page.as_ref().unwrap();
        assert_eq!((page.perks[3], pagg.games), (77, 8));
        // zu wenige Spiele gegen den Gegner: keine Aussage
        assert!(matchup_runes(&ms[..5], 1, 2).notable.is_empty());
    }

    #[test]
    fn wilson_penalizes_small_samples() {
        assert!(wilson(1, 1) < wilson(60, 100));
        assert_eq!(wilson(0, 0), 0.0);
    }

    fn snap(tier: &str, div: &str, lp: i32, w: i32, l: i32) -> LpSnap {
        LpSnap { ts: 0, rank: Rank { tier: tier.into(), div: div.into(), lp, wins: w, losses: l } }
    }

    #[test]
    fn lp_deltas_need_exactly_one_game() {
        let snaps = vec![
            snap("GOLD", "II", 80, 10, 10),
            snap("GOLD", "I", 5, 11, 10),  // +25 (Aufstieg)
            snap("GOLD", "II", 90, 11, 11), // -15
            snap("GOLD", "II", 90, 13, 11), // 2 Spiele -> ignoriert
        ];
        let d = lp_deltas(&snaps);
        assert_eq!(d.iter().map(|x| (x.1, x.2)).collect::<Vec<_>>(), vec![(25, true), (-15, false)]);
    }

    #[test]
    fn late_lp_update_belongs_to_the_game() {
        let at = |ts: i64, lp: i32, w: i32, l: i32| LpSnap { ts, ..snap("BRONZE", "II", lp, w, l) };
        // Niederlage gezählt, LP aber erst 2 Minuten später abgezogen
        let snaps = vec![at(0, 76, 167, 177), at(1000, 76, 167, 178), at(121_000, 56, 167, 178)];
        let d = lp_deltas(&snaps);
        assert_eq!(d.iter().map(|x| (x.1, x.2)).collect::<Vec<_>>(), vec![(-20, false)]);
        // echter Abstiegsschutz: 0 LP vorher, keine spätere Änderung -> bleibt 0
        let shield = vec![at(0, 0, 5, 5), at(1000, 0, 5, 6)];
        assert_eq!(lp_deltas(&shield)[0].1, 0);
        // spätere LP-Änderung nach mehr als 30 Minuten gehört nicht mehr dazu
        let late = vec![at(0, 76, 1, 1), at(1000, 76, 1, 2), at(1000 + LATE_LP_MS + 1, 56, 1, 2)];
        assert_eq!(lp_deltas(&late)[0].1, 0);
    }

    #[test]
    fn records_pair_lobby_with_lp() {
        let ranked = |tier: &str, team: i32| LivePlayer {
            team,
            rank: Some(Rank { tier: tier.into(), div: "IV".into(), ..Default::default() }),
            ..Default::default()
        };
        let lobby = Lobby {
            game_id: 7,
            ts: 100,
            queue: 420,
            players: vec![
                LivePlayer { me: true, team: 100, ..Default::default() },
                ranked("SILVER", 100),
                ranked("SILVER", 100),
                ranked("GOLD", 200),
                ranked("GOLD", 200),
            ],
            ..Default::default()
        };
        let mut pre = snap("SILVER", "II", 40, 10, 10);
        pre.ts = 90;
        let mut post = snap("SILVER", "II", 61, 11, 10);
        post.ts = 2000;
        let r = &game_records(&[lobby], &[pre, post])[0];
        assert_eq!((r.win, r.delta, r.rank), (true, 21, 1040));
        assert_eq!(r.lobby, 1000);
        assert_eq!((r.team, r.enemy), (Some(800), Some(1200)));
    }
}
