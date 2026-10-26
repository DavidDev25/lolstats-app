//! Labor: vollständiges Protokoll aller Solo/Duo-Spiele und Auswertungen darauf.

use crate::model::*;
use crate::stats::{self, Agg, GameRec};

/// Ein Solo/Duo-Spiel aus den LP-Snapshots, ergänzt um die Lobby (falls erfasst).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabRow {
    pub game_id: u64,
    /// Zeitpunkt des LP-Snapshots nach dem Spiel
    pub ts: i64,
    pub win: bool,
    pub delta: i32,
    /// Niederlage ohne LP-Verlust (Abstiegsschutz)
    pub shield: bool,
    /// Riot hat Bonus-LP gegeben (z. B. Autofill-Schutz) – kein normales Spiel für LP-Auswertungen
    pub bonus: bool,
    /// Autofill im Champ Select erkannt (Bonus war vorher absehbar)
    pub autofill: bool,
    pub rank_before: i32,
    pub rank_after: i32,
    /// LP in der Division vor dem Spiel (für gekappte Niederlagen)
    pub lp_before: i32,
    pub lobby: Option<i32>,
    pub team: Option<i32>,
    pub enemy: Option<i32>,
    /// Serie vor dem Spiel: +n Siege bzw. -n Niederlagen in Folge
    pub streak: i32,
}

impl LabRow {


}

#[derive(Clone, Debug, Default)]
pub struct StreakLine {
    pub streak: i32,
    pub all: Agg,
    pub gain: Option<f64>,
    pub loss: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Lab {
    pub rows: Vec<LabRow>,
    pub records: Vec<GameRec>,
    pub total: Agg,
    pub avg_gain: Option<f64>,
    pub avg_loss: Option<f64>,
    pub streaks: Vec<StreakLine>,
    /// (Beschriftung, Bilanz) für Team-Ø minus Gegner-Ø
    pub teams: Vec<(&'static str, Agg)>,
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}


pub fn build(lobbies: &[Lobby], snaps: &[LpSnap]) -> Lab {
    let solo: Vec<&Lobby> = lobbies
        .iter()
        .filter(|l| l.queue == 420 && !l.champ_select && l.game_id != 0 && !l.remake)
        .collect();
    let mut rows = Vec::new();
    for i in 1..snaps.len() {
        if snaps[i].rank.games() != snaps[i - 1].rank.games() + 1 {
            continue;
        }
        // verspätete LP-Änderung nach dem Spiel noch diesem Spiel zuordnen
        let j = stats::settled(snaps, i - 1, i);
        let (pre, post) = (&snaps[i - 1], &snaps[j]);
        let (Some(a), Some(b)) = (pre.rank.value(), post.rank.value()) else { continue };
        let delta = b - a;
        if delta.abs() > 60 {
            continue;
        }
        let win = post.rank.wins > pre.rank.wins;
        let lb = solo.iter().rev().find(|l| l.ts >= pre.ts && l.ts < snaps[i].ts);
        let my_team = lb.and_then(|l| l.players.iter().find(|p| p.me)).map_or(100, |p| p.team);
        rows.push(LabRow {
            game_id: lb.map_or(0, |l| l.game_id),
            ts: snaps[i].ts,
            win,
            delta,
            // 0 LP verloren, deutlich gekappt (auf 0 LP mit kleinem Abzug) oder abgestiegen (Riot setzt dann
            // auf 75 LP): Riots Regeln, keine normale Niederlage. 20 -> 0 ist dagegen eine volle Niederlage.
            shield: !win && (delta == 0 || (post.rank.lp == 0 && delta > -CAPPED_LOSS) || pre.rank.lp == 0),
            bonus: lb.and_then(|l| l.lp_note.as_ref()).is_some_and(|n| !n.bonus.is_empty()),
            autofill: lb.is_some_and(|l| l.autofill),
            rank_before: a,
            rank_after: b,
            lp_before: pre.rank.lp,
            lobby: lb.and_then(|l| l.avg_others()),
            team: lb.and_then(|l| stats::team_avg(l, my_team)),
            enemy: lb.and_then(|l| stats::team_avg(l, if my_team == 200 { 100 } else { 200 })),
            ..Default::default()
        });
    }

    let mut cur = 0;
    for r in rows.iter_mut() {
        r.streak = cur;
        cur = match (r.win, cur > 0, cur < 0) {
            (true, true, _) => cur + 1,
            (true, false, _) => 1,
            (false, _, true) => cur - 1,
            (false, _, false) => -1,
        };
    }


    let records = stats::game_records(lobbies, snaps);


    let mut total = Agg::default();
    let (mut gains, mut losses) = (Vec::new(), Vec::new());
    for r in &rows {
        total.add(r.win);
        if r.bonus {
            continue;
        }
        if r.win {
            gains.push(r.delta as f64);
        } else if !r.shield {
            losses.push(-r.delta as f64);
        }
    }

    let mut streaks = Vec::new();
    for s in -3i32..=3 {
        if s == 0 {
            continue;
        }
        let sel: Vec<&LabRow> = rows
            .iter()
            .filter(|r| if s.abs() == 3 { r.streak.signum() == s.signum() && r.streak.abs() >= 3 } else { r.streak == s })
            .collect();
        let mut line = StreakLine { streak: s, ..Default::default() };
        let mut g = Vec::new();
        let mut l = Vec::new();
        for r in &sel {
            line.all.add(r.win);
            if r.win {
                g.push(r.delta as f64);
            } else if !r.shield {
                l.push(-r.delta as f64);
            }
        }
        line.gain = mean(&g);
        line.loss = mean(&l);
        streaks.push(line);
    }

    let mut teams = vec![
        (tr!("Dein Team > 50 LP höher", "Your team > 50 LP higher"), Agg::default()),
        (tr!("ungefähr gleich (±50)", "roughly equal (±50)"), Agg::default()),
        (tr!("Gegner > 50 LP höher", "Enemy > 50 LP higher"), Agg::default()),
    ];
    for r in &rows {
        if let (Some(t), Some(e)) = (r.team, r.enemy) {
            let i = if t - e > 50 { 0 } else if e - t > 50 { 2 } else { 1 };
            teams[i].1.add(r.win);
        }
    }

    Lab {
        rows,
        records,
        total,
        avg_gain: mean(&gains),
        avg_loss: mean(&losses),
        streaks,
        teams,
    }
}

pub fn streak_label(s: i32) -> String {
    match s {
        3 => tr!("3+ Siege in Folge", "3+ wins in a row").into(),
        -3 => tr!("3+ Niederlagen in Folge", "3+ losses in a row").into(),
        1 => tr!("nach 1 Sieg", "after 1 win").into(),
        -1 => tr!("nach 1 Niederlage", "after 1 loss").into(),
        s if s > 0 => trf!("nach {s} Siegen", "after {s} wins"),
        s => trf!("nach {} Niederlagen", "after {} losses", -s),
    }
}

/// CSV mit ";" (deutsches Excel), alle Ränge zusätzlich als Text.
pub fn csv(lab: &Lab) -> String {
    let mut head = vec!["zeit_utc", "game_id", "ergebnis", "lp", "abstiegsschutz"];
    head.extend(["rang_vorher", "rang_vorher_text", "rang_nachher", "lobby_avg", "team_avg", "gegner_avg", "serie_vorher"]);
    let mut out = head.join(";") + "\n";

    let o = |v: Option<i32>| v.map(|x| x.to_string()).unwrap_or_default();
    for r in &lab.rows {
        let mut c = vec![
            fmt_utc(r.ts),
            if r.game_id == 0 { String::new() } else { r.game_id.to_string() },
            (if r.win { "S" } else { "N" }).to_string(),
            r.delta.to_string(),
            (if r.shield { "1" } else { "0" }).to_string(),
        ];
        c.extend([
            r.rank_before.to_string(),
            value_label(r.rank_before),
            r.rank_after.to_string(),
            o(r.lobby),
            o(r.team),
            o(r.enemy),
            r.streak.to_string(),
        ]);
        out += &(c.join(";") + "\n");
    }
    out
}





#[cfg(test)]
mod tests {
    use super::*;


    fn snap(ts: i64, lp: i32, w: i32, l: i32) -> LpSnap {
        LpSnap { ts, rank: Rank { tier: "SILVER".into(), div: "II".into(), lp, wins: w, losses: l } }
    }

    #[test]
    fn rows_streaks_and_errors() {
        let snaps = vec![
            snap(0, 50, 10, 10),
            snap(10, 72, 11, 10), // S +22
            snap(20, 95, 12, 10), // S +23
            snap(30, 75, 12, 11), // N -20
            snap(40, 75, 12, 12), // N 0 (Abstiegsschutz)
        ];
        let other = |tier: &str, team: i32| LivePlayer {
            team,
            rank: Some(Rank { tier: tier.into(), div: "IV".into(), ..Default::default() }),
            ..Default::default()
        };
        let lobby = Lobby {
            game_id: 5,
            ts: 15,
            queue: 420,
            players: vec![
                LivePlayer { me: true, team: 100, ..Default::default() },
                other("GOLD", 100),
                other("GOLD", 100),
                other("SILVER", 200),
                other("SILVER", 200),
            ],
            ..Default::default()
        };
        let lab = build(&[lobby], &snaps);
        assert_eq!(lab.rows.len(), 4);
        assert_eq!(lab.rows.iter().map(|r| r.streak).collect::<Vec<_>>(), vec![0, 1, 2, -1]);
        let r = &lab.rows[1];
        assert_eq!((r.game_id, r.delta), (5, 23));
        assert_eq!((r.team, r.enemy), (Some(1200), Some(800)));
        assert!(lab.rows[3].shield);
        assert_eq!((lab.total.games, lab.total.wins), (4, 2));
        assert_eq!(lab.avg_gain, Some(22.5));
        assert_eq!(lab.avg_loss, Some(20.0)); // Abstiegsschutz zählt nicht
        assert_eq!(lab.teams[0].1, Agg { games: 1, wins: 1 });
        let csv = csv(&lab);
        assert_eq!(csv.lines().count(), 5);
        {
            assert!(csv.lines().nth(2).unwrap().contains(";5;S;23;0;1072;"));
        }
    }

    #[test]
    fn streak_labels() {
        assert_eq!(streak_label(-3), tr!("3+ Niederlagen in Folge", "3+ losses in a row"));
        assert_eq!(streak_label(2), "nach 2 Siegen");
    }
}
