//! Loser-Queue-Test (beide Versionen): Bekommst du nach einer Siegesserie schlechtere Mitspieler als die Gegner?
//!
//! Pro Solo/Duo-Spiel wird "Mitspieler minus Gegner" gemessen (Troll/AFK-Quote, Rang, Form, KDA, Offmeta)
//! und zwischen "nach 2+ Siegen in Folge" und "sonst" verglichen. Gäbe es eine Loser Queue, wären die
//! Mitspieler nach Serien messbar schlechter als die Gegner – bei Zufall gibt es keinen Unterschied.
//! Die Serie kommt direkt aus den Match-Ergebnissen (kein LP-Verlauf nötig, zählt auch ältere Spiele).

use crate::analysis::{is_offmeta, LaneFreq};
use crate::model::*;

/// Einfacher deterministischer Zufall (xorshift) für Permutationstests und Simulationen.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Mittelwertunterschied zweier Gruppen mit Permutationstest (2000 zufällige Gruppeneinteilungen).
/// Liefert (Mittel Gruppe true, Mittel Gruppe false, n true, n false, p).
pub fn group_test(vals: &[(bool, f64)]) -> (f64, f64, usize, usize, f64) {
    let mean = |g: bool, v: &[(bool, f64)]| {
        let x: Vec<f64> = v.iter().filter(|p| p.0 == g).map(|p| p.1).collect();
        (x.iter().sum::<f64>() / x.len().max(1) as f64, x.len())
    };
    let (ms, ns) = mean(true, vals);
    let (mo, no) = mean(false, vals);
    if ns == 0 || no == 0 {
        return (ms, mo, ns, no, 1.0);
    }
    let obs = (ms - mo).abs();
    let mut labels: Vec<bool> = vals.iter().map(|p| p.0).collect();
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    let mut hits = 0;
    const PERMS: usize = 2000;
    for _ in 0..PERMS {
        for i in (1..labels.len()).rev() {
            labels.swap(i, (rng.next() % (i as u64 + 1)) as usize);
        }
        let shuffled: Vec<(bool, f64)> = labels.iter().zip(vals).map(|(l, v)| (*l, v.1)).collect();
        if (mean(true, &shuffled).0 - mean(false, &shuffled).0).abs() >= obs - 1e-12 {
            hits += 1;
        }
    }
    (ms, mo, ns, no, (hits + 1) as f64 / (PERMS + 1) as f64)
}

/// Spieler war vermutlich AFK (weniger als halbes Team-Gold) oder hat getrollt (10+ Tode, KDA < 1).
fn is_troll(p: &Part, team: &[&Part]) -> bool {
    let others: Vec<i32> = team.iter().filter(|q| q.puuid != p.puuid).map(|q| q.gold).collect();
    let avg = others.iter().sum::<i32>() as f64 / others.len().max(1) as f64;
    let afk = avg > 0.0 && (p.gold as f64) < avg * 0.5;
    let kda = (p.k + p.a) as f64 / p.d.max(1) as f64;
    afk || (p.d >= 10 && kda < 1.0)
}

/// Kennzahlen eines Spiels, jeweils Mitspieler (ohne dich) und Gegner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueRow {
    pub ts: i64,
    pub game_id: u64,
    pub win: bool,
    /// Siege in Folge direkt vor dem Spiel
    pub streak: i32,
    /// Troll/AFK-Quote pro Spieler
    pub troll_mates: f64,
    pub troll_enemy: f64,
    /// Rang-Schnitt (LP-Skala), falls die Lobby erfasst wurde
    pub rank_mates: Option<f64>,
    pub rank_enemy: Option<f64>,
    /// Winrate der letzten 20 Spiele (0..1), falls erfasst
    pub form_mates: Option<f64>,
    pub form_enemy: Option<f64>,
    pub kda_mates: f64,
    pub kda_enemy: f64,
    /// Anteil Offmeta-Picks (Champion auf seltener Position, z. B. Fizz als ADC)
    pub offmeta_mates: f64,
    pub offmeta_enemy: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueLine {
    pub label: &'static str,
    /// Mittelwert nach Serie / sonst
    pub after_streak: f64,
    pub other: f64,
    pub n_streak: usize,
    pub n_other: usize,
    /// Zufallstest auf den Unterschied der beiden Gruppen
    pub p: f64,
}

/// Mindestspiele je Gruppe für ein Urteil bzw. ein belastbares Urteil.
pub const QUEUE_MIN: usize = 15;
pub const QUEUE_SOLID: usize = 30;

impl QueueLine {
    pub fn verdict(&self) -> &'static str {
        if self.n_streak < QUEUE_MIN || self.n_other < QUEUE_MIN {
            "zu wenige Spiele"
        } else if self.p < 0.01 && self.n_streak >= QUEUE_SOLID {
            "deutlicher Unterschied"
        } else if self.p < 0.05 {
            "schwacher Hinweis"
        } else {
            "kein Unterschied (Zufall)"
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueReport {
    pub rows: Vec<QueueRow>,
    pub lines: Vec<QueueLine>,
    /// (Spiele, Siege) mit Offmeta-Mitspieler bzw. ohne
    pub offmeta_wr: [(usize, usize); 2],
}

/// Offmeta-Quote eines Teams aus der erfassten Lobby (eigenes Team ohne dich bzw. Gegner),
/// nur wenn die Lobby für das Team Lanes kennt.
fn lobby_offmeta(lobby: Option<&Lobby>, mine: bool, freq: &LaneFreq) -> Option<f64> {
    let lb = lobby?;
    let my_side = lb.players.iter().find(|x| x.me)?.team;
    let ps: Vec<&LivePlayer> = lb.players.iter().filter(|x| !x.me && (x.team == my_side) == mine && x.champ > 0).collect();
    if ps.len() < 4 || ps.iter().any(|x| x.lane.is_empty()) {
        return None;
    }
    let n = ps.iter().filter(|x| is_offmeta(freq, x.champ, &x.lane)).count();
    Some(n as f64 / ps.len() as f64)
}

pub fn queue_report(lobbies: &[Lobby], ms: &[Match], me: &str, freq: &LaneFreq) -> QueueReport {
    let offmeta = |ps: &[&Part]| ps.iter().filter(|q| is_offmeta(freq, q.champ, q.position())).count() as f64 / ps.len().max(1) as f64;
    // eigene Solo/Duo-Spiele in zeitlicher Reihenfolge; Remakes zählen nicht
    let mut own: Vec<(&Match, &Part)> = ms.iter().filter(|m| m.queue == 420 && !m.remake()).filter_map(|m| Some((m, m.part(me)?))).collect();
    own.sort_by_key(|(m, _)| m.ts);
    let mut rows = Vec::new();
    let mut streak = 0;
    for &(m, p) in &own {
        if m.ps.len() == 10 {
            let mates: Vec<&Part> = m.ps.iter().filter(|q| q.team == p.team && q.puuid != me).collect();
            let enemy: Vec<&Part> = m.ps.iter().filter(|q| q.team != p.team).collect();
            let my_team: Vec<&Part> = m.ps.iter().filter(|q| q.team == p.team).collect();
            let rate = |ps: &[&Part], team: &[&Part]| ps.iter().filter(|q| is_troll(q, team)).count() as f64 / ps.len().max(1) as f64;
            let kda = |ps: &[&Part]| ps.iter().map(|q| (q.k + q.a) as f64 / q.d.max(1) as f64).sum::<f64>() / ps.len().max(1) as f64;
            let lobby = lobbies.iter().find(|l| l.game_id == m.id);
            let lobby_avg = |mine: bool, f: &dyn Fn(&LivePlayer) -> Option<f64>| {
                let lb = lobby?;
                let my_side = lb.players.iter().find(|x| x.me)?.team;
                let v: Vec<f64> = lb.players.iter().filter(|x| !x.me && (x.team == my_side) == mine).filter_map(f).collect();
                (v.len() >= 2).then(|| v.iter().sum::<f64>() / v.len() as f64)
            };
            let rank = |x: &LivePlayer| x.rank.as_ref()?.value().map(|v| v as f64);
            let form = |x: &LivePlayer| (x.recent.0 >= 5).then(|| x.recent.1 as f64 / x.recent.0 as f64);
            rows.push(QueueRow {
                ts: m.ts,
                game_id: m.id,
                win: p.win,
                streak,
                troll_mates: rate(&mates, &my_team),
                troll_enemy: rate(&enemy, &enemy),
                rank_mates: lobby_avg(true, &rank),
                rank_enemy: lobby_avg(false, &rank),
                form_mates: lobby_avg(true, &form),
                form_enemy: lobby_avg(false, &form),
                kda_mates: kda(&mates),
                kda_enemy: kda(&enemy),
                // Position bevorzugt aus der Lobby (Champ Select/Ladebildschirm, vom Client); die
                // Match-History rät sie nachträglich und lässt sie bei Offmeta-Picks oft leer
                offmeta_mates: lobby_offmeta(lobby, true, freq).unwrap_or_else(|| offmeta(&mates)),
                offmeta_enemy: lobby_offmeta(lobby, false, freq).unwrap_or_else(|| offmeta(&enemy)),
            });
        }
        streak = if p.win { streak + 1 } else { 0 };
    }
    let test = |label: &'static str, f: &dyn Fn(&QueueRow) -> Option<f64>| {
        let vals: Vec<(bool, f64)> = rows.iter().filter_map(|r| Some((r.streak >= 2, f(r)?))).collect();
        let (after_streak, other, n_streak, n_other, p) = group_test(&vals);
        QueueLine { label, after_streak, other, n_streak, n_other, p }
    };
    let lines = vec![
        test("Troll/AFK-Quote Mitspieler − Gegner", &|r| Some(r.troll_mates - r.troll_enemy)),
        test("Rang-Ø Mitspieler − Gegner (LP)", &|r| Some(r.rank_mates? - r.rank_enemy?)),
        test("Form Mitspieler − Gegner (Winrate)", &|r| Some(r.form_mates? - r.form_enemy?)),
        test("KDA Mitspieler − Gegner", &|r| Some(r.kda_mates - r.kda_enemy)),
        test("Offmeta-Quote Mitspieler − Gegner", &|r| Some(r.offmeta_mates - r.offmeta_enemy)),
        test("Troll/AFK-Quote deiner Mitspieler", &|r| Some(r.troll_mates)),
    ];
    // Siegchance mit/ohne Offmeta-Pick im eigenen Team
    let wr = |with: bool| {
        let g: Vec<&QueueRow> = rows.iter().filter(|r| (r.offmeta_mates > 0.0) == with).collect();
        (g.len(), g.iter().filter(|r| r.win).count())
    };
    QueueReport { offmeta_wr: [wr(true), wr(false)], rows, lines }
}

/// Loser-Queue-Test als CSV: Ergebnis je Kennzahl, dann jedes Spiel.
pub fn queue_csv(r: &QueueReport) -> String {
    let f = |v: f64| format!("{v:.3}").replace('.', ",");
    let o = |v: Option<f64>| v.map(f).unwrap_or_default();
    let mut out = String::from("# Loser-Queue-Test: Mitspieler minus Gegner, nach 2+ Siegen in Folge gegen sonst\n");
    out += "kennzahl;nach_serie;sonst;spiele_serie;spiele_sonst;p;urteil\n";
    for l in &r.lines {
        out += &format!("{};{};{};{};{};{};{}\n", l.label, f(l.after_streak), f(l.other), l.n_streak, l.n_other, f(l.p), l.verdict());
    }
    let [(wg, ww), (og, ow)] = r.offmeta_wr;
    out += &format!("# Siegchance mit Offmeta-Mitspieler: {ww}/{wg}, ohne: {ow}/{og}\n");
    out += "\nzeit_utc;game_id;ergebnis;siege_in_folge_davor;troll_mitspieler;troll_gegner;rang_mitspieler;rang_gegner;form_mitspieler;form_gegner;kda_mitspieler;kda_gegner;offmeta_mitspieler;offmeta_gegner\n";
    for row in &r.rows {
        out += &format!(
            "{};{};{};{};{};{};{};{};{};{};{};{};{};{}\n",
            fmt_utc(row.ts),
            row.game_id,
            if row.win { "S" } else { "N" },
            row.streak,
            f(row.troll_mates),
            f(row.troll_enemy),
            o(row.rank_mates),
            o(row.rank_enemy),
            o(row.form_mates),
            o(row.form_enemy),
            f(row.kda_mates),
            f(row.kda_enemy),
            f(row.offmeta_mates),
            f(row.offmeta_enemy)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offmeta_detection() {
        let mut f = LaneFreq::new();
        f.insert(105, [5, 2, 60, 1, 0]); // Fizz: fast nur Mid
        f.insert(99, [1, 0, 3, 0, 4]); // zu wenige Spiele für ein Urteil
        assert!(is_offmeta(&f, 105, "ADC")); // 1 von 68 Spielen
        assert!(!is_offmeta(&f, 105, "MID"));
        assert!(!is_offmeta(&f, 105, "TOP")); // 5 von 68 = 7 % -> noch normal
        assert!(!is_offmeta(&f, 99, "JUNGLE"));
        assert!(!is_offmeta(&f, 42, "ADC")); // unbekannter Champion
        assert!(!is_offmeta(&f, 105, "")); // Lane unbekannt
    }

    #[test]
    fn loser_queue_test_groups_and_trolls() {
        // Troll/AFK-Erkennung
        let p = |id: &str, k: i32, d: i32, a: i32, gold: i32| Part { puuid: id.into(), k, d, a, gold, ..Default::default() };
        let team = [p("a", 5, 3, 5, 10000), p("b", 0, 12, 2, 7000), p("c", 0, 2, 0, 3000), p("d", 4, 4, 4, 9000)];
        let refs: Vec<&Part> = team.iter().collect();
        assert!(!is_troll(&team[0], &refs));
        assert!(is_troll(&team[1], &refs)); // 12 Tode, KDA 0,17
        assert!(is_troll(&team[2], &refs)); // halbes Team-Gold -> AFK
        // Gruppentest: nach Serie deutlich schlechter -> kleiner p-Wert; gleich verteilt -> großer p-Wert
        let shifted: Vec<(bool, f64)> = (0..60).map(|i| (i % 2 == 0, if i % 2 == 0 { -0.3 } else { 0.0 } + (i % 5) as f64 * 0.01)).collect();
        let (ms, mo, ns, no, pv) = group_test(&shifted);
        assert!(ms < mo && ns == 30 && no == 30 && pv < 0.01);
        let same: Vec<(bool, f64)> = (0..60).map(|i| (i % 2 == 0, (i % 7) as f64 * 0.01)).collect();
        assert!(group_test(&same).4 > 0.05);
        let l = QueueLine { n_streak: 5, n_other: 40, p: 0.001, ..Default::default() };
        assert_eq!(l.verdict(), "zu wenige Spiele");
    }

    #[test]
    fn streak_comes_from_match_results() {
        // S S S N S als Solo/Duo-Matches mit 10 Spielern; Remake und andere Queue zählen nicht
        let game = |id: u64, win: bool, queue: i32, dur: i32| Match {
            id,
            ts: id as i64,
            queue,
            dur,
            ps: (0..10)
                .map(|i| Part { puuid: if i == 0 { "me".into() } else { format!("p{i}") }, team: if i < 5 { 100 } else { 200 }, win: (i < 5) == win, gold: 8000, ..Default::default() })
                .collect(),
            ..Default::default()
        };
        let ms = vec![game(1, true, 420, 1800), game(2, true, 420, 1800), game(3, false, 420, 120), game(4, true, 440, 1800), game(5, true, 420, 1800), game(6, false, 420, 1800), game(7, true, 420, 1800)];
        let r = queue_report(&[], &ms, "me", &LaneFreq::new());
        let st: Vec<i32> = r.rows.iter().map(|x| x.streak).collect();
        assert_eq!(st, vec![0, 1, 2, 3, 0]);
        assert!(queue_csv(&r).lines().count() > 7);
    }
}
