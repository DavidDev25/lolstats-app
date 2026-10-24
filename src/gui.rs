//! Fenster (eigener Prozess, nur bei Bedarf). Liest nur die Dateien des Hintergrunddienstes.

use crate::analysis::{self, Best, Comp, EloPicks, CompStats, DeathStats, Farm, GameStat, Matchups, Pool, Rec, Records, Style, STYLE_LABELS};
use crate::i18n::tx;
use crate::icons::{self, Icons, Kind};
use crate::lab::{self, Lab, LabRow};
use crate::lcu::{self, Lcu};
use crate::model::*;
use crate::stats::{self, Agg, Build, ChampLine, RunePage};
use crate::store::{Store, Table, DB};
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, RichText, Sense, Stroke};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

const WIN: Color32 = Color32::from_rgb(80, 180, 120);
const LOSS: Color32 = Color32::from_rgb(215, 85, 85);
const GOLD: Color32 = theme::GOLD;
const BLUE: Color32 = Color32::from_rgb(90, 140, 230);
/// ±1 Tier auf der LP-Skala für den Elo-Filter der Builds
const ELO_SPAN: i32 = 400;

pub fn run() {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([1000.0, 700.0])
        .with_min_inner_size([640.0, 420.0]);
    if let Some(p) = Store::open().read::<WinPos>(WINDOW) {
        viewport = viewport
            .with_position([p.x, p.y])
            .with_inner_size([p.w.max(640.0), p.h.max(420.0)])
            .with_maximized(p.maximized);
    }
    let opts = eframe::NativeOptions { viewport, ..Default::default() };
    let _ = eframe::run_native("lolstats", opts, Box::new(|cc| {
        theme::apply(&cc.egui_ctx);
        Ok(Box::new(App::new(&cc.egui_ctx)))
    }));
}

/// Screenshot-Modus: jede Seite von oben bis unten als PNG in `dir` (Aufruf: `lolstats.exe shots <Ordner>`).
pub fn run_shots(dir: std::path::PathBuf, lang: Option<String>) {
    let _ = std::fs::create_dir_all(&dir);
    let viewport = egui::ViewportBuilder::default()
        .with_title("lolstats (Screenshots)")
        .with_inner_size([1200.0, 900.0]);
    let opts = eframe::NativeOptions { viewport, ..Default::default() };
    let _ = eframe::run_native("lolstats-shots", opts, Box::new(move |cc| {
        theme::apply(&cc.egui_ctx);
        let mut app = App::new(&cc.egui_ctx);
        app.cfg.expert = true; // nur im Speicher: alle Seiten inkl. Labor zeigen
        if lang.is_some() {
            // nur im Speicher; alles neu berechnen, damit auch berechnete Texte in dieser Sprache entstehen
            app.force_lang = lang;
            app.force_reload();
        }
        app.shot = Some(Shot { dir, tabs: SHOT_TABS.to_vec(), wait: 40, ..Default::default() });
        // die ersten Daten wurden schon in App::new geladen -> sofort anonymisieren
        app.anonymize();
        Ok(Box::new(app))
    }));
}

const SHOT_TABS: [(Tab, &str); 7] = [
    (Tab::Live, "1_live"),
    (Tab::Me, "2_ich"),
    (Tab::Builds, "3_builds"),
    (Tab::Lab, "4_labor"),
    (Tab::Perf, "5_leistung"),
    (Tab::Help, "6_tutorial"),
    (Tab::Settings, "7_einstellungen"),
];
/// höchstens so viele Bilder pro Seite
const SHOT_MAX: usize = 25;

#[derive(Default)]
struct Shot {
    dir: std::path::PathBuf,
    tabs: Vec<(Tab, &'static str)>,
    idx: usize,
    /// aktuelle Scrollposition (Punkte) und Bildnummer auf der Seite
    offset: f32,
    n: usize,
    /// Frames warten (Bilder laden, Layout), Screenshot angefordert, Prüfsumme des letzten Bilds
    wait: u32,
    pending: bool,
    last: u64,
}

#[derive(PartialEq, Clone, Copy, Default)]
enum Tab {
    #[default]
    Live,
    Me,
    Builds,
    Lab,
    Perf,
    Help,
    Settings,
}

struct App {
    icons: Icons,
    store: Store,
    tab: Tab,
    cfg: Config,
    names: Names,
    me: Option<Me>,
    matches: Vec<Match>,
    lobbies: Vec<Lobby>,
    snaps: Vec<LpSnap>,
    state: LiveState,
    mtimes: HashMap<&'static str, Option<SystemTime>>,
    db_ver: HashMap<Table, i64>,
    last_check: Instant,
    my_games: Vec<usize>,
    champ_list: Vec<(i32, Agg)>,
    my_champs: Vec<ChampLine>,
    search: String,
    sel_champ: i32,
    build: Option<Build>,
    msg: String,
    lab: Lab,
    /// 0 = alle, 1 = Siege, 2 = Niederlagen
    lab_filter: u8,
    lab_only_lobby: bool,
    lab_msg: String,
    timelines: Vec<Timeline>,
    comps: CompStats,
    games: Vec<GameStat>,
    deaths: DeathStats,
    pool: Pool,
    /// Builds nur aus Matches nahe der eigenen Elo
    build_near_elo: bool,
    records: Records,
    farm: Farm,
    /// Champion -> (Ø CS/min in deiner Elo, Spiele) für den CS-Tacho
    elo_csm: HashMap<i32, (f64, usize)>,
    /// Champion -> (Ø Gold/min in deiner Elo, Spiele) fürs Coaching
    elo_gpm: HashMap<i32, (f64, usize)>,
    /// Screenshot-Modus (`lolstats.exe shots`) und dort vorgegebene Sprache
    shot: Option<Shot>,
    force_lang: Option<String>,
    /// Stand der Update-Prüfung (öffentliche Version) und Änderungszeit von update.json
    update: crate::update::UpdateInfo,
    update_mtime: Option<SystemTime>,
    /// zuletzt berechnete Matchup-Runen: ((Champion, Gegner, Matches), Ergebnis)
    matchup_runes: std::cell::RefCell<Option<((i32, i32, usize), stats::MatchupRunes)>>,
    /// Rückmeldung des Matchup-Runen-Imports
    matchup_msg: std::cell::RefCell<String>,
    style: Option<Style>,
    /// beste Picks für die eigene Elo auf der Hauptposition (Tab "Ich")
    elo_picks: EloPicks,
    queue: crate::queue::QueueReport,
    lane_freq: analysis::LaneFreq,
    /// Build für den eigenen Champion im Live-Tab
    live_build: Option<Build>,
    /// (Lobby-Zeitpunkt, Champion), für den schon automatisch auf "Live" gewechselt wurde
    auto_pick: (i64, i32),
    matchups: Matchups,
    season: Season,
    /// Suchfeld "Beste Picks gegen ..." im Leistungs-Tab
    pick_search: String,
    pick_enemy: i32,
    win: Option<WinPos>,
    win_changed: Option<Instant>,
    reloaded: Option<Instant>,
    /// asiatische Ersatzschriften: gebraucht (aus den angezeigten Namen) bzw. schon geladen
    fonts_need: theme::ExtraFonts,
    fonts_loaded: theme::ExtraFonts,
}


fn tier_color(tier: &str) -> Color32 {
    match tier {
        "IRON" => Color32::from_rgb(130, 120, 115),
        "BRONZE" => Color32::from_rgb(170, 110, 75),
        "SILVER" => Color32::from_rgb(170, 180, 190),
        "GOLD" => GOLD,
        "PLATINUM" => Color32::from_rgb(70, 170, 160),
        "EMERALD" => Color32::from_rgb(40, 180, 100),
        "DIAMOND" => Color32::from_rgb(110, 140, 230),
        "MASTER" => Color32::from_rgb(170, 90, 200),
        "GRANDMASTER" => Color32::from_rgb(210, 70, 70),
        "CHALLENGER" => Color32::from_rgb(240, 200, 90),
        _ => Color32::GRAY,
    }
}

fn wr_text(wins: i32, games: i32, text: String) -> RichText {
    let wr = if games > 0 { wins as f64 / games as f64 } else { 0.5 };
    let t = RichText::new(text);
    if games < 3 {
        t
    } else if wr >= 0.55 {
        t.color(WIN)
    } else if wr < 0.45 {
        t.color(LOSS)
    } else {
        t
    }
}

fn pct(a: &Agg) -> String {
    format!("{:.0} %", a.wr() * 100.0)
}

impl App {
    /// Senkrechter Scrollbereich einer Seite; im Screenshot-Modus an der vorgegebenen Position.
    fn scroll(&self) -> egui::ScrollArea {
        let s = egui::ScrollArea::vertical();
        match &self.shot {
            Some(st) => s.vertical_scroll_offset(st.offset),
            None => s,
        }
    }

    /// Für Screenshots: eigener Name -> "Du#EUW", alle anderen -> "Spieler 1..9".
    fn anonymize(&mut self) {
        if let Some(me) = self.me.as_mut() {
            me.name = tr!("Du#EUW", "You#EUW").into();
        }
        if let Some(lb) = self.state.lobby.as_mut() {
            let mut n = 0;
            for p in lb.players.iter_mut() {
                if p.me {
                    p.name = tr!("Du#EUW", "You#EUW").into();
                } else if !p.name.is_empty() {
                    n += 1;
                    p.name = trf!("Spieler {n}", "Player {n}");
                }
            }
        }
    }

    /// Screenshot-Modus: Seite einstellen, Bild anfordern, speichern, weiterscrollen, nächste Seite.
    fn shot_step(&mut self, ctx: &egui::Context) {
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let page_h = ctx.screen_rect().height();
        let Some(st) = self.shot.as_mut() else { return };
        ctx.request_repaint();
        if let Some(img) = img {
            st.pending = false;
            st.wait = 4;
            // gleiches Bild wie zuvor = Seitenende erreicht (egui begrenzt die Scrollposition)
            let mut h: u64 = 1469598103934665603;
            for p in &img.pixels {
                h = (h ^ u32::from_le_bytes(p.to_array()) as u64).wrapping_mul(1099511628211);
            }
            let name = st.tabs[st.idx].1;
            // hinter dem Seitenende: Inhaltsbereich (rechts der Seitenleiste, unter dem Kopf) fast nur Hintergrund
            let [w, hgt] = img.size;
            // Hintergrundfarbe aus der Ecke unten rechts (Bildschirmfarbe kann vom Theme-Wert leicht abweichen)
            let bg_px = img.pixels[(hgt - 3) * w + (w - 3)];
            let (mut bg, mut all) = (0usize, 0usize);
            for y in (80..hgt).step_by(4) {
                for x in (220..w).step_by(4) {
                    all += 1;
                    bg += (img.pixels[y * w + x] == bg_px) as usize;
                }
            }
            let blank = all > 0 && bg * 1000 >= all * 995;
            if h != st.last && st.n < SHOT_MAX && !blank {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Some(rgba) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes) {
                    let _ = rgba.save(st.dir.join(format!("{name}_{:02}.png", st.n + 1)));
                }
                st.n += 1;
                st.last = h;
                st.offset += page_h * 0.8;
            } else {
                st.idx += 1;
                st.offset = 0.0;
                st.n = 0;
                st.last = 0;
                st.wait = 8;
                if st.idx >= st.tabs.len() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        } else if !st.pending {
            if st.wait > 0 {
                st.wait -= 1;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                st.pending = true;
            }
        }
        self.tab = st.tabs[st.idx].0;
    }

    fn new(ctx: &egui::Context) -> Self {
        let store = Store::open();
        let mut a = App {
            icons: Icons::new(ctx, store.dir.clone()),
            store,
            tab: Tab::Live,
            cfg: Config::default(),
            names: Names::default(),
            me: None,
            matches: Vec::new(),
            lobbies: Vec::new(),
            snaps: Vec::new(),
            state: LiveState::default(),
            mtimes: HashMap::new(),
            db_ver: HashMap::new(),
            last_check: Instant::now(),
            my_games: Vec::new(),
            champ_list: Vec::new(),
            my_champs: Vec::new(),
            search: String::new(),
            sel_champ: 0,
            build: None,
            msg: String::new(),
            lab: Lab::default(),
            lab_filter: 0,
            lab_only_lobby: false,
            lab_msg: String::new(),
            elo_csm: HashMap::new(),
            elo_gpm: HashMap::new(),
            shot: None,
            force_lang: None,
            update: Default::default(),
            update_mtime: None,
            matchup_runes: Default::default(),
            matchup_msg: Default::default(),
            timelines: Vec::new(),
            comps: CompStats::default(),
            games: Vec::new(),
            deaths: DeathStats::default(),
            pool: Pool::default(),
            build_near_elo: false,
            records: Records::default(),
            farm: Farm::default(),
            style: None,
            elo_picks: EloPicks::default(),
            queue: Default::default(),
            lane_freq: analysis::LaneFreq::new(),
            live_build: None,
            auto_pick: (0, 0),
            matchups: Matchups::new(),
            season: Season::default(),
            pick_search: String::new(),
            pick_enemy: 0,
            win: None,
            win_changed: None,
            reloaded: None,
            fonts_need: theme::ExtraFonts::default(),
            fonts_loaded: theme::ExtraFonts::default(),
        };
        a.win = a.store.read(WINDOW);
        a.refresh();
        if a.state.lobby.is_none() && a.state.result.is_none() {
            a.tab = Tab::Me;
        }
        // Erster Start: Tutorial zeigen (einmalig)
        if !a.cfg.tutorial_seen {
            a.tab = Tab::Help;
            a.cfg.tutorial_seen = true;
            a.store.write(CONFIG, &a.cfg);
        }
        // Für Screenshots: Start-Tab und Champion per Umgebungsvariable
        if let Ok(t) = std::env::var("LOLSTATS_TAB") {
            a.tab = match t.as_str() {
                "ich" => Tab::Me,
                "builds" => Tab::Builds,
                "labor" => Tab::Lab,
                "leistung" => Tab::Perf,
                "settings" => Tab::Settings,
                _ => Tab::Live,
            };
        }
        if let Some(c) = std::env::var("LOLSTATS_CHAMP").ok().and_then(|c| c.parse().ok()) {
            a.select_champ(c);
        }
        a
    }

    /// Lädt nur neu, was sich geändert hat: JSON-Dateien per Änderungszeit, Tabellen per Version.
    fn refresh(&mut self) {
        {
            let m = self.store.mtime(crate::update::UPDATE);
            if m != self.update_mtime {
                self.update_mtime = m;
                self.update = self.store.read(crate::update::UPDATE).unwrap_or_default();
            }
        }
        let mut files = Vec::new();
        for f in [STATE, NAMES, ME, CONFIG, SEASON] {
            let m = self.store.mtime(f);
            if self.mtimes.get(f) != Some(&m) {
                self.mtimes.insert(f, m);
                files.push(f);
            }
        }
        let mut tables = Vec::new();
        for t in [Table::Matches, Table::Lobbies, Table::Lp, Table::Timelines] {
            let v = self.store.version(t);
            if self.db_ver.get(&t) != Some(&v) {
                self.db_ver.insert(t, v);
                tables.push(t);
            }
        }
        if files.is_empty() && tables.is_empty() {
            return;
        }
        for f in &files {
            match *f {
                STATE => self.state = analysis::without_off_modes(self.store.read(STATE).unwrap_or_default()),
                NAMES => {
                    self.names = self.store.read(NAMES).unwrap_or_default();
                    self.icons.aliases = self.names.alias.clone();
                }
                ME => self.me = self.store.read(ME),
                CONFIG => {
                    self.cfg = self.store.read(CONFIG).unwrap_or_default();
                    if let Some(l) = &self.force_lang {
                        self.cfg.lang = l.clone();
                    }
                    crate::i18n::apply(&self.cfg.lang);
                }
                SEASON => self.season = self.store.read(SEASON).unwrap_or_default(),
                _ => {}
            }
        }
        // Screenshot-Modus: keine echten Riot-IDs auf den Bildern
        if self.shot.is_some() {
            self.anonymize();
        }
        for t in &tables {
            match t {
                Table::Matches => {
                    self.matches = self.store.matches();
                    self.matches.reverse(); // neueste zuerst
                }
                Table::Lobbies => self.lobbies = self.store.lobbies(),
                Table::Lp => self.snaps = self.store.snaps(),
                Table::Timelines => self.timelines = self.store.timelines(),
            }
        }
        let puuid = self.me.as_ref().map(|m| m.puuid.clone()).unwrap_or_default();
        // Teure Neuberechnungen nur bei neuen Daten: Im Spiel ändert sich alle 15 s nur state.json (CS-Tacho),
        // und das Fenster läuft mit niedriger Priorität – sonst hinkt die Anzeige unter Spiel-Last Minuten hinterher
        let data = !tables.is_empty() || files.contains(&ME);
        if data {
            self.my_games = (0..self.matches.len()).filter(|&i| self.matches[i].part(&puuid).is_some()).collect();
        }
        // Namen, die das Fenster anzeigt (Lobby + eigene Spiele): brauchen sie asiatische Schriften?
        if files.contains(&STATE) || tables.contains(&Table::Matches) {
            let lobby = self.state.lobby.iter().flat_map(|lb| lb.players.iter().map(|p| p.name.as_str()));
            let own = self.my_games.iter().flat_map(|&i| self.matches[i].ps.iter().map(|p| p.name.as_str()));
            let need = lobby.chain(own).fold(theme::ExtraFonts::default(), |e, n| e.union(theme::ExtraFonts::needed_for(n)));
            self.fonts_need = self.fonts_need.union(need);
        }
        if data {
            self.champ_list = stats::champ_list(&self.matches);
            self.my_champs = stats::my_champs(&self.matches, &puuid);
        }
        if tables.contains(&Table::Matches) || files.contains(&NAMES) {
            self.comps = analysis::comp_stats(&self.matches, &self.names.info);
        }
        if tables.contains(&Table::Matches) || tables.contains(&Table::Timelines) || files.contains(&ME) {
            self.games = analysis::my_games(&self.matches, &puuid, &self.timelines);
            self.deaths = analysis::death_stats(&self.games);
            self.pool = analysis::pool(&self.games);
            self.farm = analysis::farm(&self.games);
        }
        if tables.contains(&Table::Matches) || files.contains(&ME) || tables.contains(&Table::Lp) {
            let elo = self.snaps.last().and_then(|s| s.rank.value());
            self.style = analysis::style(&self.matches, &puuid, elo);
            self.elo_csm = analysis::elo_csm(&self.matches, &puuid, elo);
            self.elo_gpm = analysis::elo_gpm(&self.matches, &puuid, elo);
            self.elo_picks = match &self.style {
                Some(st) => analysis::elo_picks(&self.matches, &self.games, &puuid, elo, st.position),
                None => EloPicks::default(),
            };
        }
        if tables.contains(&Table::Matches) {
            self.matchups = analysis::matchups(&self.matches);
        }
        if tables.contains(&Table::Matches) || tables.contains(&Table::Lp) || files.contains(&SEASON) || files.contains(&ME) {
            self.records = analysis::records(&self.games, &self.snaps, self.season.clone());
        }
        if tables.contains(&Table::Lobbies) || tables.contains(&Table::Lp) {
            self.lab = lab::build(&self.lobbies, &self.snaps);
        }
        if tables.contains(&Table::Matches) {
            self.lane_freq = self.store.lane_freq();
        }
        if tables.contains(&Table::Matches) || tables.contains(&Table::Lobbies) {
            self.queue = crate::queue::queue_report(&self.lobbies, &self.matches, &puuid, &self.lane_freq);
        }
        // Lobbys ohne Lanes (z. B. von einer älteren Version erfasst): hier schätzen
        if files.contains(&STATE) || tables.contains(&Table::Matches) {
            if let Some(lb) = self.state.lobby.as_mut() {
                if lb.players.iter().all(|p| p.lane.is_empty()) {
                    analysis::fill_lanes(&mut lb.players, &self.lane_freq, lb.queue);
                }
            }
        }
        // Champ Select: Build für den eigenen Champion vorbereiten und nach dem Lock einmal auf "Live" wechseln
        if files.contains(&STATE) || tables.contains(&Table::Matches) {
            let mine = self.state.lobby.as_ref().and_then(|lb| lb.players.iter().find(|p| p.me).map(|p| (lb.ts, lb.champ_select, p.champ, p.locked)));
            if let Some((key, cs, champ, locked)) = mine {
                if champ > 0 && (tables.contains(&Table::Matches) || self.live_build.as_ref().map_or(true, |b| b.champ != champ)) {
                    self.live_build = Some(self.build_of(champ));
                }
                if cs && locked && champ > 0 && self.auto_pick != (key, champ) {
                    self.auto_pick = (key, champ);
                    self.tab = Tab::Live;
                }
            }
        }
        if self.sel_champ > 0 && tables.contains(&Table::Matches) {
            self.select_champ(self.sel_champ);
        }
    }

    /// Alles neu einlesen und den Hintergrunddienst um einen sofortigen Abgleich bitten.
    fn force_reload(&mut self) {
        self.store.write(REFRESH, &now_ms());
        self.mtimes.clear();
        self.db_ver.clear();
        self.refresh();
        self.reloaded = Some(Instant::now());
    }

    /// Merkt sich Position/Größe; gespeichert wird erst, wenn das Fenster 1 s still steht.
    fn track_window(&mut self, ctx: &egui::Context) {
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.minimized == Some(true) {
            return;
        }
        if let (Some(o), Some(inner)) = (vp.outer_rect, vp.inner_rect) {
            if o.min.x < -20000.0 {
                return;
            }
            let p = if vp.maximized == Some(true) {
                WinPos { maximized: true, ..self.win.unwrap_or_default() }
            } else {
                WinPos { x: o.min.x, y: o.min.y, w: inner.width(), h: inner.height(), maximized: false }
            };
            if self.win != Some(p) {
                self.win = Some(p);
                self.win_changed = Some(Instant::now());
            }
        }
        if let (Some(t), Some(p)) = (self.win_changed, self.win) {
            if t.elapsed() >= Duration::from_secs(1) {
                self.store.write(WINDOW, &p);
                self.win_changed = None;
            }
        }
    }

    fn select_champ(&mut self, c: i32) {
        self.sel_champ = c;
        self.msg.clear();
        self.build = Some(self.build_of(c));
    }

    /// Build eines Champions aus allen Matches bzw. (Schalter im Builds-Tab) nur nahe der eigenen Elo.
    fn build_of(&self, c: i32) -> Build {
        let names = &self.names;
        if self.build_near_elo {
            let mine = self.snaps.last().and_then(|s| s.rank.value());
            let near: Vec<Match> = self.matches.iter().filter(|m| analysis::near_elo(m, mine, ELO_SPAN)).cloned().collect();
            stats::build_for(&near, c, &|id| names.completed(id), &|id| names.is_boots(id))
        } else {
            stats::build_for(&self.matches, c, &|id| names.completed(id), &|id| names.is_boots(id))
        }
    }

    /// Live-Tab komplett scrollbar (Ergebnis, Vergleich, Lane-Gegner, Build und Teams).
    /// Sanfter Pausen-Hinweis nach 2+ Niederlagen in Folge in derselben Session, belegt mit eigenen Zahlen.
    /// Liefert true, wenn etwas angezeigt wurde.
    fn ui_tilt(&self, ui: &mut egui::Ui) -> bool {
        let Some(t) = analysis::tilt(&self.games, now_ms()) else { return false };
        if t.streak < analysis::TILT_STREAK {
            // ohne Serie: Hinweis nur bei langer Session, wenn deine Daten einen Einbruch ab dem 4. Spiel zeigen
            let Some((late, early, n)) = analysis::long_session(&self.games, &t) else { return false };
            theme::section(ui, |ui| {
                help_header(ui, &trf!("Schon {} Spiele heute – Zeit für eine Pause?", "Already {} games today – time for a break?", t.session.0), crate::i18n::l(HELP_TILT));
                ui.label(
                    RichText::new(trf!(
                        "Ab dem 4. Spiel einer Session gewinnst du {:.0} % ({n} Spiele), in den ersten drei {:.0} %.", "From the 4th game of a session you win {:.0} % ({n} games), in the first three {:.0} %.",
                        late * 100.0,
                        early * 100.0
                    ))
                    .color(LOSS),
                );
                ui.label(RichText::new(trf!("Heute {} Spiele, {} Siege.", "Today {} games, {} wins.", t.session.0, t.session.1)).color(theme::MUTED));
            });
            return true;
        }
        let wr = |(g, w): (usize, usize)| w as f64 * 100.0 / g.max(1) as f64;
        theme::section(ui, |ui| {
            help_header(ui, &trf!("{} Niederlagen in Folge – kurze Pause?", "{} losses in a row – short break?", t.streak), crate::i18n::l(HELP_TILT));
            if t.after.0 >= analysis::TILT_MIN_GAMES {
                let (a, o) = (wr(t.after), wr(t.other));
                let col = if a + 5.0 < o { LOSS } else { theme::TEXT };
                ui.label(
                    RichText::new(trf!(
                        "Nach 2+ Niederlagen in Folge gewinnst du {a:.0} % ({} von {} Spielen), sonst {o:.0} %.", "After 2+ losses in a row you win {a:.0} % ({} of {} games), otherwise {o:.0} %.",
                        t.after.1, t.after.0
                    ))
                    .color(col),
                );
            } else {
                ui.label(
                    RichText::new(trf!(
                        "Ob du nach Serien schlechter spielst, zeigt sich ab {} solchen Spielen (bisher {}).", "Whether you play worse after streaks shows from {} such games (so far {}).",
                        analysis::TILT_MIN_GAMES,
                        t.after.0
                    ))
                    .color(theme::MUTED),
                );
            }
            if let (Some(s), Some(a)) = (t.early_streak, t.early_all) {
                if s > a + 0.5 {
                    ui.label(trf!("Tode vor Minute 10 in dieser Serie: Ø {s:.1} – dein Schnitt {a:.1}.", "Deaths before minute 10 in this streak: avg {s:.1} – your average {a:.1}."));
                }
            }
            ui.label(
                RichText::new(trf!("Heute {} Spiele, {} Siege. 10–15 Minuten Pause kosten nichts.", "Today {} games, {} wins. A 10–15 minute break costs nothing.", t.session.0, t.session.1))
                    .color(theme::MUTED),
            );
        });
        true
    }

    /// CS-Tacho im laufenden Spiel: CS jetzt gegen deinen Verlauf, den Elo-Schnitt und den Lane-Gegner.
    fn ui_cs_tacho(&self, ui: &mut egui::Ui, lb: &Lobby) {
        let Some(c) = self.state.cs.as_ref().filter(|c| c.game_id == lb.game_id && c.time >= 60.0) else { return };
        let mins = c.time / 60.0;
        let diff_label = |d: f64| RichText::new(format!("{d:+.0}")).strong().color(if d >= 0.0 { WIN } else { LOSS });
        theme::section(ui, |ui| {
            help_header(ui, tr!("CS-Tacho", "CS tracker"), crate::i18n::l(HELP_CS_TACHO));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} CS", c.cs)).size(22.0).strong());
                ui.label(RichText::new(trf!("bei {}:{:02} · {:.1} CS/min", "at {}:{:02} · {:.1} CS/min", c.time as i64 / 60, c.time as i64 % 60, c.cs as f64 / mins)).color(theme::MUTED));
                // Alter der Daten: zeigt, ob der Dienst oder die Anzeige hängt
                // (im Screenshot-Modus ausgeblendet, sonst ändert sich das Bild jede Sekunde)
                if self.shot.is_none() {
                    let age = (now_ms() - c.ts).max(0) / 1000;
                    ui.label(RichText::new(trf!("Stand vor {age} s", "updated {age} s ago")).small().color(if age > 45 { GOLD } else { theme::MUTED }));
                }
            });
            if let Some(curve) = analysis::cs_curve(&self.games, c.champ) {
                let exp = curve.expected(mins);
                ui.horizontal_wrapped(|ui| {
                    ui.label(diff_label(c.cs as f64 - exp));
                    let src = if curve.champ_only { trf!("auf {}", "on {}", self.names.champ(c.champ)) } else { tr!("über alle Champions", "across all champions").into() };
                    ui.label(trf!("gegen deinen Schnitt zu dieser Zeit ({exp:.0} CS, {} Spiele {src})", "vs your average at this time ({exp:.0} CS, {} games {src})", curve.games));
                });
            }
            if let Some(&(csm, n)) = self.elo_csm.get(&c.champ) {
                // Spielschnitt über die ganze Dauer enthält die ersten ~75 s ohne Vasallen:
                // auf die Farm-Zeit umrechnen (bei ~28 min Spieldauer Faktor ~1,05)
                let exp = csm * (mins - 1.25).max(0.0) * 1.05;
                ui.horizontal_wrapped(|ui| {
                    ui.label(diff_label(c.cs as f64 - exp));
                    ui.label(trf!("gegen {} in deiner Elo: Ø {csm:.1} CS/min ({n} Spiele)", "vs {} in your elo: avg {csm:.1} CS/min ({n} games)", self.names.champ(c.champ)));
                });
            }
            if let Some(o) = c.opp_cs {
                ui.horizontal_wrapped(|ui| {
                    ui.label(diff_label((c.cs - o) as f64));
                    ui.label(trf!("gegen deinen Lane-Gegner {} ({o} CS)", "vs your lane opponent {} ({o} CS)", self.names.champ(c.opp_champ)));
                });
            }
        });
        ui.add_space(8.0);
    }

    fn ui_live(&self, ui: &mut egui::Ui) {
        self.scroll().id_salt("live_scroll").show(ui, |ui| self.ui_live_body(ui));
    }

    fn ui_live_body(&self, ui: &mut egui::Ui) {
        theme::page_header(ui, "Live", tr!("Lobby, Pick-Empfehlung und Ergebnis des letzten Spiels.", "Lobby, pick suggestions and the result of your last game."));
        if let Some(r) = &self.state.result {
            let (txt, col) = match r.win {
                Some(true) => (tr!("Sieg", "Victory"), WIN),
                Some(false) => (tr!("Niederlage", "Defeat"), LOSS),
                None => (tr!("Spiel beendet", "Game over"), Color32::GRAY),
            };
            theme::section(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(txt).font(FontId::new(24.0, theme::bold())).color(col));
                    match (r.lp_delta, r.win) {
                        (Some(0), Some(false)) | (None, Some(false)) if r.queue == 420 => {
                            ui.label(RichText::new("±0 LP").size(22.0).strong());
                            ui.label(RichText::new(tr!("(Riot hat keinen LP-Abzug gemeldet)", "(Riot reported no LP loss)")).weak());
                        }
                        (Some(d), _) => {
                            ui.label(RichText::new(format!("{d:+} LP")).size(22.0).strong());
                        }
                        _ => {}
                    }
                    ui.label(
                        RichText::new(format!("{} · {} · {}", self.names.champ(r.champ), queue_name(r.queue), ago(r.ts)))
                            .weak(),
                    );
                });
                ui.label(RichText::new(trf!("Rang: {}", "Rank: {}", r.rank)).size(16.0));
                self.ui_match_summary(ui, r.game_id);
            });
            ui.add_space(8.0);
        }
        if self.ui_tilt(ui) {
            ui.add_space(8.0);
        }
        if !self.state.note.is_empty() {
            ui.label(RichText::new(&self.state.note).italics());
            ui.add_space(6.0);
        }
        let Some(lb) = &self.state.lobby else {
            ui.label(tr!("Noch keine Lobby. Sobald du in Champ Select oder im Ladebildschirm bist, erscheinen die Spieler hier automatisch.", "No lobby yet. As soon as you are in champ select or the loading screen, the players appear here automatically."));
            return;
        };
        let avg = lb.avg_others().map(|v| trf!(" · Lobby-Schnitt: {}", " · Lobby average: {}", div_label(v))).unwrap_or_default();
        ui.label(
            RichText::new(format!(
                "{} · {}{} · {}",
                queue_name(lb.queue),
                if lb.champ_select { "Champ Select" } else { tr!("Spiel", "Game") },
                avg,
                ago(lb.ts)
            ))
            .strong(),
        );
        if !lb.champ_select {
            self.ui_cs_tacho(ui, lb);
        }
        if lb.champ_select {
            let pos = lb.players.iter().find(|p| p.me).map(|p| position_from_select(&p.position)).unwrap_or("");
            let enemies: Vec<i32> = lb.players.iter().filter(|p| p.team == 200 && p.champ > 0).map(|p| p.champ).collect();
            theme::section(ui, |ui| {
                let vs = if enemies.is_empty() {
                    tr!("noch keine Gegner-Picks", "no enemy picks yet").to_string()
                } else {
                    enemies.iter().map(|&e| self.names.champ(e)).collect::<Vec<_>>().join(", ")
                };
                let pos_txt = if pos.is_empty() { "unbekannt".to_string() } else { position_label(pos).to_string() };
                ui.label(RichText::new(tr!("Pick-Empfehlung", "Pick suggestions")).strong());
                ui.label(RichText::new(trf!("Position: {pos_txt} · gegen: {vs}", "Position: {pos_txt} · vs: {vs}")).weak());
                let recs = analysis::recommend(&self.games, &self.matchups, &enemies, pos);
                rec_grid(ui, "live_recs", &recs, &self.names);
                for &e in &enemies {
                    let a = analysis::wr_vs(&self.games, e);
                    if a.games > 0 {
                        ui.label(wr_text(a.wins, a.games, trf!("Deine Bilanz gegen {}: {} Spiele, {} WR", "Your record vs {}: {} games, {} WR", self.names.champ(e), a.games, pct(&a))));
                    }
                }
            });
            ui.add_space(6.0);
        }
        ui.scope(|ui| {
            // Build für den eigenen Champion (ab Pick bzw. Hover in Champ Select, auch im Spiel)
            let my_champ = lb.players.iter().find(|p| p.me).map(|p| p.champ).unwrap_or(0);
            let build = self.live_build.as_ref().filter(|b| b.champ == my_champ && my_champ > 0);
            // Breites Fenster: Lane-Gegner und Build nebeneinander statt jeweils über die volle Breite
            match build {
                Some(b) if ui.available_width() >= WIDE_TWO_COL && lane_opponent(lb).is_some() => {
                    ui.columns(2, |cols| {
                        self.ui_lane_opponent(&mut cols[0], lb);
                        cols[1].add_space(6.0);
                        theme::section(&mut cols[1], |ui| self.live_build_card(ui, b));
                    });
                }
                _ => {
                    self.ui_lane_opponent(ui, lb);
                    if let Some(b) = build {
                        ui.add_space(6.0);
                        theme::section(ui, |ui| self.live_build_card(ui, b));
                    }
                }
            }
            // Teams nebeneinander, wenn beide Tabellen ganz hineinpassen (Experten-Ansicht hat mehr Spalten)
            let side_by_side = ui.available_width() >= if self.cfg.expert { WIDE_TEAMS_EXPERT } else { WIDE_TWO_COL };
            let team_block = |ui: &mut egui::Ui, team: i32| {
                let mut ps: Vec<&LivePlayer> = lb.players.iter().filter(|p| p.team == team).collect();
                // nach Lane sortieren (Top, Jungle, Mid, ADC, Support), unbekannte ans Ende
                ps.sort_by_key(|p| analysis::LANES.iter().position(|&l| l == p.lane).unwrap_or(9));
                if ps.is_empty() {
                    return;
                }
                ui.add_space(8.0);
                let (t, c) = if team == 100 {
                    (tr!("Blaues Team", "Blue team"), Color32::from_rgb(90, 140, 230))
                } else {
                    (tr!("Rotes Team", "Red team"), Color32::from_rgb(220, 90, 90))
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t).color(c).strong());
                    let comp = Comp::of(ps.iter().map(|p| p.champ), &self.names.info);
                    if comp.known > 0 && self.cfg.expert {
                        ui.label(RichText::new(format!("{} · {}", comp.label(), comp.profile().label())).weak());
                    }
                });
                wide(ui, ("lobby_scroll", team), |ui| egui::Grid::new(("lobby", team)).striped(true).min_col_width(56.0).show(ui, |ui| {
                    let ex = self.cfg.expert;
                    let head: &[&str] =
                        if ex { &["Champion", tr!("Spieler", "Player"), "Solo/Duo", tr!("Saison", "Season"), tr!("Letzte 20", "Last 20"), tr!("Auf Champ", "On champ"), "KDA"] } else { &["Champion", tr!("Spieler", "Player"), tr!("Rang", "Rank"), tr!("Auf Champ", "On champ")] };
                    for h in head {
                        ui.label(RichText::new(*h).weak());
                    }
                    ui.end_row();
                    for p in ps {
                        let (g, w) = p.recent;
                        ui.horizontal(|ui| {
                            lane_badge(ui, &p.lane, p.lane_guess);
                            icons::show(ui, &self.icons, Kind::Champ, p.champ, 22.0);
                            ui.label(self.names.champ(p.champ));
                            if p.champ > 0 && analysis::is_offmeta(&self.lane_freq, p.champ, &p.lane) {
                                ui.label(RichText::new("Offmeta").small().color(GOLD)).on_hover_text(trf!(
                                    "{} wird in den gesammelten Matches nur selten als {} gespielt (unter {:.0} %)", "{} is rarely played as {} in the collected matches (under {:.0} %)",
                                    self.names.champ(p.champ),
                                    position_label(&p.lane),
                                    analysis::OFFMETA_SHARE * 100.0
                                ));
                            }
                        });
                        let mut n = RichText::new(if p.name.is_empty() { "?" } else { p.name.as_str() });
                        if p.me {
                            n = n.strong().color(GOLD);
                        }
                        // Einfacher Modus: Details beim Drüberfahren über den Namen
                        let details = trf!(
                            "Saison: {}\nLetzte 20: {}\nKDA: {}", "Season: {}\nLast 20: {}\nKDA: {}",
                            p.rank.as_ref().map(|r| if r.losses > 0 { trf!("{}S {}N", "{}W {}L", r.wins, r.losses) } else { trf!("{} Siege", "{} wins", r.wins) }).unwrap_or_else(|| "-".into()),
                            if g > 0 { trf!("{w}S {}N", "{w}W {}L", g - w) } else { "-".into() },
                            if g > 0 { format!("{:.2}", p.kda) } else { "-".into() }
                        );
                        let resp = ui.label(n);
                        if !ex {
                            resp.on_hover_text(details);
                        }
                        match &p.rank {
                            Some(r) if r.value().is_some() => {
                                ui.label(RichText::new(r.label()).color(tier_color(&r.tier)));
                                // Für fremde Spieler liefert der Client keine Niederlagen -> keine Winrate
                                if ex && r.losses > 0 {
                                    let wr = r.wins as f64 * 100.0 / r.games().max(1) as f64;
                                    ui.label(wr_text(r.wins, r.games(), format!("{wr:.0} % ({})", r.games())));
                                } else if ex {
                                    ui.label(RichText::new(trf!("{} Siege", "{} wins", r.wins)).color(theme::MUTED));
                                }
                            }
                            _ => {
                                ui.label(RichText::new("Unranked").weak());
                                if ex {
                                    ui.label("-");
                                }
                            }
                        }
                        if ex {
                            ui.label(if g > 0 { wr_text(w, g, trf!("{w}S {}N", "{w}W {}L", g - w)) } else { RichText::new("-") });
                        }
                        match p.champs.iter().find(|c| c.0 == p.champ && p.champ > 0) {
                            Some(&(_, cg, cw)) => {
                                ui.label(wr_text(cw, cg, trf!("{cg} Sp. · {:.0} %", "{cg} g · {:.0} %", cw as f64 * 100.0 / cg as f64)))
                            }
                            None => ui.label("-"),
                        };
                        if ex {
                            ui.label(if g > 0 { format!("{:.2}", p.kda) } else { "-".into() });
                        }
                        ui.end_row();
                    }
                }));
            };
            if side_by_side {
                ui.columns(2, |cols| {
                    team_block(&mut cols[0], 100);
                    team_block(&mut cols[1], 200);
                });
            } else {
                team_block(ui, 100);
                team_block(ui, 200);
            }
        });
    }

    /// Direkter Gegner auf der eigenen Lane: Rang, Form, Erfahrung auf dem Champion, Bilanzen.
    fn ui_lane_opponent(&self, ui: &mut egui::Ui, lb: &Lobby) {
        let Some((me, opp)) = lane_opponent(lb) else { return };
        ui.add_space(6.0);
        theme::section(ui, |ui| {
            let guess = if opp.lane_guess { tr!(" (Lane geschätzt)", " (lane estimated)") } else { "" };
            sub_title(ui, &trf!("Dein Lane-Gegner · {}{guess}", "Your lane opponent · {}{guess}", position_label(position_from_select(&me.lane))));
            if opp.champ <= 0 {
                ui.label(RichText::new(tr!("Noch nicht gepickt.", "Not picked yet.")).color(theme::MUTED));
                return;
            }
            let cname = self.names.champ(opp.champ);
            ui.horizontal(|ui| {
                icons::show(ui, &self.icons, Kind::Champ, opp.champ, 40.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&cname).font(FontId::new(17.0, theme::bold())).color(Color32::WHITE));
                    let rank = match &opp.rank {
                        Some(r) if r.value().is_some() => RichText::new(r.label()).color(tier_color(&r.tier)),
                        _ => RichText::new("Unranked").color(theme::MUTED),
                    };
                    ui.horizontal(|ui| {
                        if !opp.name.is_empty() {
                            ui.label(RichText::new(&opp.name).color(theme::MUTED));
                        }
                        ui.label(rank);
                    });
                });
            });
            let mut facts: Vec<RichText> = Vec::new();
            let (g, w) = opp.recent;
            if g > 0 {
                facts.push(wr_text(w, g, trf!("Form: {w}S {}N in den letzten {g}", "Form: {w}W {}L in the last {g}", g - w)));
            }
            match opp.champs.iter().find(|c| c.0 == opp.champ) {
                Some(&(_, cg, cw)) => facts.push(wr_text(cw, cg, trf!("auf {cname}: {cg} Spiele, {:.0} %", "on {cname}: {cg} games, {:.0} %", cw as f64 * 100.0 / cg as f64))),
                None if g > 0 => facts.push(RichText::new(trf!("{cname} in den letzten {g} Spielen nicht gespielt", "{cname} not played in the last {g} games")).color(GOLD)),
                None => {}
            }
            let vs = analysis::wr_vs(&self.games, opp.champ);
            if vs.games > 0 {
                facts.push(wr_text(vs.wins, vs.games, trf!("deine Bilanz gegen {cname}: {} Spiele, {}", "your record vs {cname}: {} games, {}", vs.games, pct(&vs))));
            }
            // frühe Tode gegen genau diesen Gegner im Vergleich zu deinem Schnitt
            let early = |g: &GameStat| g.timeline.as_ref().map(|t| t.deaths.iter().filter(|&&d| d < 600).count() as f64);
            let mine: Vec<f64> = self.games.iter().filter(|g| g.opp == Some(opp.champ)).filter_map(early).collect();
            let all: Vec<f64> = self.games.iter().filter_map(early).collect();
            if mine.len() >= 2 && !all.is_empty() {
                let (m, a) = (mine.iter().sum::<f64>() / mine.len() as f64, all.iter().sum::<f64>() / all.len() as f64);
                if m >= a + 0.3 {
                    facts.push(
                        RichText::new(trf!("Gegen {cname} stirbst du vor Minute 10 öfter: Ø {m:.1} ({} Spiele), sonst Ø {a:.1} – Level 1–6 vorsichtig traden", "You die before minute 10 more often vs {cname}: avg {m:.1} ({} games), otherwise avg {a:.1} – trade carefully at levels 1–6", mine.len()))
                            .color(LOSS),
                    );
                }
            }
            if me.champ > 0 {
                if let Some(a) = self.matchups.get(&(me.champ, opp.champ)).filter(|a| a.games > 0) {
                    facts.push(wr_text(
                        a.wins,
                        a.games,
                        trf!("{} gegen {cname} (alle gesammelten Matches): {} Spiele, {}", "{} vs {cname} (all collected matches): {} games, {}", self.names.champ(me.champ), a.games, pct(a)),
                    ));
                }
            }
            for f in facts {
                ui.label(f);
            }
            if me.champ > 0 {
                self.ui_matchup_runes(ui, me.champ, opp.champ);
            }
        });
    }

    /// Runen, die mit deinem Champion gegen genau diesen Gegner deutlich häufiger gewählt werden.
    fn ui_matchup_runes(&self, ui: &mut egui::Ui, champ: i32, opp: i32) {
        let key = (champ, opp, self.matches.len());
        let mut cache = self.matchup_runes.borrow_mut();
        if cache.as_ref().map(|c| c.0) != Some(key) {
            *cache = Some((key, stats::matchup_runes(&self.matches, champ, opp)));
        }
        let r = &cache.as_ref().unwrap().1;
        let cname = self.names.champ(opp);
        if r.vs.games < stats::MATCHUP_RUNES_MIN {
            if r.vs.games > 0 {
                ui.label(RichText::new(trf!("Runen gegen {cname}: erst {} Spiele gesammelt (ab {})", "Runes vs {cname}: only {} games collected so far (from {})", r.vs.games, stats::MATCHUP_RUNES_MIN)).small().color(theme::MUTED));
            }
            return;
        }
        if r.notable.is_empty() {
            ui.label(RichText::new(trf!("Runen gegen {cname} ({} Spiele): wie gegen alle anderen – Standard-Runen passen.", "Runes vs {cname} ({} games): same as against everyone else – standard runes fit.", r.vs.games)).small().color(theme::MUTED));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(trf!("Gegen {cname} öfter gewählt:", "Picked more often vs {cname}:")).color(GOLD)).on_hover_text(crate::i18n::l(HELP_MATCHUP_RUNES));
                for (k, vs, other, agg) in &r.notable {
                    chip(ui, &self.names.perk(*k)).on_hover_text(trf!(
                        "{:.0} % gegen {cname} ({} Spiele, {} WR) · sonst {:.0} %", "{:.0} % vs {cname} ({} games, {} WR) · otherwise {:.0} %",
                        vs * 100.0,
                        agg.games,
                        pct(agg),
                        other * 100.0
                    ));
                }
            });
        }
        // Import nur im Champ Select (im Spiel lassen sich die Runen nicht mehr ändern), nur per Klick
        let champ_select = self.state.lobby.as_ref().is_some_and(|l| l.champ_select);
        if let (true, Some((page, agg))) = (champ_select, &r.page) {
            ui.horizontal_wrapped(|ui| {
                let tip = trf!("Meistgespielte komplette Runenseite gegen {cname}: {} Spiele, {} WR", "Most played full rune page vs {cname}: {} games, {} WR", agg.games, pct(agg));
                if ui.button(trf!("Matchup-Runen gegen {cname} importieren", "Import matchup runes vs {cname}")).on_hover_text(tip).clicked() {
                    let name = format!("lolstats vs {cname}");
                    *self.matchup_msg.borrow_mut() = match Lcu::connect(&self.cfg.client_dir) {
                        Some(l) => match lcu::import_runes(&l, page, &name) {
                            Ok(()) => trf!("✓ Runenseite „{name}“ im Client angelegt", "✓ Rune page “{name}” created in the client"),
                            Err(e) => trf!("Import fehlgeschlagen: {e}", "Import failed: {e}"),
                        },
                        None => tr!("League-Client nicht gefunden", "League client not found").into(),
                    };
                }
                let msg = self.matchup_msg.borrow();
                if !msg.is_empty() {
                    ui.label(RichText::new(msg.as_str()).small().color(theme::MUTED));
                }
            });
        }
    }

    /// Nach dem Spiel: eigene Werte gegen den Schnitt der 20 Spiele davor (grün besser, rot schlechter).
    fn ui_match_summary(&self, ui: &mut egui::Ui, game_id: u64) {
        ui.add_space(6.0);
        let Some((n, lines)) = analysis::match_summary(&self.games, game_id) else {
            ui.label(RichText::new(tr!("Spieldetails werden nach dem Spiel aus der Match-History nachgeladen …", "Game details are loaded from the match history after the game …")).small().color(theme::MUTED));
            return;
        };
        ui.label(RichText::new(trf!("Dein Spiel im Vergleich zum Schnitt deiner letzten {n} Spiele", "Your game compared with the average of your last {n} games")).small().color(theme::MUTED));
        ui.horizontal_wrapped(|ui| {
            for l in &lines {
                // Farbe = besser/schlechter, Pfeil = höher/niedriger (bei Toden ist niedriger besser)
                let (col, arrow) = match l.verdict() {
                    0 => (theme::TEXT, "="),
                    v => (if v > 0 { WIN } else { LOSS }, if l.value > l.avg { "▲" } else { "▼" }),
                };
                let d = l.decimals;
                theme::mini_stat(ui, tx(l.label), &format!("{arrow} {:.*}", d, l.value), col, &format!("Ø {:.*}", d, l.avg));
            }
        });
    }

    /// Kompakter Build für den eigenen Champion im Live-Tab: Runen, Zauber, Items mit Bildern.
    fn live_build_card(&self, ui: &mut egui::Ui, b: &Build) {
        ui.horizontal(|ui| {
            icons::show(ui, &self.icons, Kind::Champ, b.champ, 36.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(trf!("Build für {}", "Build for {}", self.names.champ(b.champ))).font(FontId::new(16.0, theme::bold())).color(Color32::WHITE));
                let scope = if self.build_near_elo { tr!(" · nahe deiner Elo", " · near your elo") } else { "" };
                ui.label(
                    RichText::new(trf!("{}{scope} · {} Spiele · {} Winrate", "{}{scope} · {} games · {} win rate", b.patch, b.total.games, pct(&b.total)))
                        .small()
                        .color(theme::MUTED),
                );
            });
        });
        if b.total.games == 0 {
            ui.label(RichText::new(tr!("Noch keine Spiele mit diesem Champion in der Datenbank.", "No games with this champion in the database yet.")).color(theme::MUTED));
            return;
        }
        ui.add_space(4.0);
        if let Some((pg, agg)) = &b.page {
            for (i, (style, perks)) in [(pg.styles[0], &pg.perks[..4]), (pg.styles[1], &pg.perks[4..])].into_iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    let col = if i == 0 { GOLD } else { theme::ACCENT };
                    ui.label(RichText::new(self.names.perk(style)).font(FontId::new(13.0, theme::bold())).color(col));
                    for &p in perks {
                        chip(ui, &self.names.perk(p));
                    }
                });
            }
            ui.label(RichText::new(trf!("Runen aus {} Spielen · {} WR", "Runes from {} games · {} WR", agg.games, pct(agg))).small().color(theme::MUTED));
        }
        if let Some((sp, agg)) = &b.spells {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr!("Zauber", "Spells")).small().color(theme::MUTED));
                chip(ui, &self.names.spell(sp[0]));
                chip(ui, &self.names.spell(sp[1]));
                ui.label(RichText::new(pct(agg)).small().color(theme::MUTED));
            });
        }
        if !b.items.is_empty() || b.boots.is_some() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                // Schuhe zuerst (gold umrandet), sie verteilen sich auf viele Sorten und fielen sonst aus der Top-Liste
                if let Some((id, agg)) = &b.boots {
                    let rate = agg.games as f64 * 100.0 / b.total.games.max(1) as f64;
                    let r = icons::show(ui, &self.icons, Kind::Item, *id, 34.0)
                        .on_hover_text(trf!("Schuhe: {} · {rate:.0} % Pickrate · {} WR", "Boots: {} · {rate:.0} % pick rate · {} WR", self.names.item(*id), pct(agg)));
                    ui.painter().rect_stroke(r.rect.expand(1.0), 4.0, Stroke::new(1.5_f32, GOLD));
                    ui.add_space(6.0);
                }
                for (id, agg) in b.items.iter().take(6) {
                    let rate = agg.games as f64 * 100.0 / b.total.games.max(1) as f64;
                    icons::show(ui, &self.icons, Kind::Item, *id, 34.0)
                        .on_hover_text(trf!("{} · {rate:.0} % Pickrate · {} WR", "{} · {rate:.0} % pick rate · {} WR", self.names.item(*id), pct(agg)));
                }
            });
            ui.label(RichText::new(tr!("Schuhe (gold) + häufigste fertige Items (Maus drüber für Details) · alles dazu im Tab Builds", "Boots (gold) + most common finished items (hover for details) · more in the Builds tab")).small().color(theme::MUTED));
        }
    }



    /// Loser-Queue-Test: Mitspieler gegen Gegner, nach Siegesserie gegen sonst.
    fn ui_queue_test(&self, ui: &mut egui::Ui) {
        help_header(ui, tr!("Loser-Queue-Test", "Loser queue test"), crate::i18n::l(HELP_QUEUE));
        let r = &self.queue;
        let after = r.rows.iter().filter(|x| x.streak >= 2).count();
        ui.label(
            RichText::new(trf!(
                "{} Solo/Duo-Spiele mit allen 10 Spielern, davon {} nach 2+ Siegen in Folge · Urteil ab {} je Gruppe", "{} Solo/Duo games with all 10 players, {} of them after 2+ wins in a row · verdict from {} per group",
                r.rows.len(),
                after,
                crate::queue::QUEUE_MIN
            ))
            .color(theme::MUTED),
        );
        ui.horizontal(|ui| {
            if ui.button(tr!("Loser-Queue-Protokoll als CSV", "Loser queue log as CSV")).clicked() {
                let path = self.store.p("loser_queue.csv");
                let _ = std::fs::write(&path, crate::queue::queue_csv(r));
                let _ = std::process::Command::new("explorer").arg("/select,").arg(&path).spawn();
            }
            ui.label(RichText::new(tr!("jedes Spiel mit Troll-Quote, Rang, Form, KDA beider Teams", "every game with troll rate, rank, form, KDA of both teams")).small().color(theme::MUTED));
        });
        wide(ui, "queue_scroll", |ui| {
            egui::Grid::new("queue").striped(true).show(ui, |ui| {
                for h in [tr!("Kennzahl", "Metric"), tr!("nach Serie", "after streak"), tr!("sonst", "otherwise"), tr!("Spiele", "Games"), "p", tr!("Urteil", "Verdict")] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for l in &r.lines {
                    let pct_like = l.label.contains("Quote") || l.label.contains("Winrate") || l.label.contains("rate");
                    let fmt = |v: f64| if pct_like { format!("{:+.1} %", v * 100.0) } else { format!("{v:+.2}") };
                    ui.label(l.label);
                    ui.label(fmt(l.after_streak));
                    ui.label(fmt(l.other));
                    ui.label(format!("{} / {}", l.n_streak, l.n_other));
                    ui.label(format!("{:.3}", l.p));
                    let col = match l.verdict() {
                        "deutlicher Unterschied" => LOSS,
                        "schwacher Hinweis" => GOLD,
                        "zu wenige Spiele" => theme::MUTED,
                        _ => theme::TEXT,
                    };
                    ui.label(RichText::new(tx(l.verdict())).color(col));
                    ui.end_row();
                }
            });
        });
        // Siegchance mit/ohne Offmeta-Pick im eigenen Team
        let [(wg, ww), (og, ow)] = r.offmeta_wr;
        let wr = |w: usize, g: usize| if g > 0 { format!("{:.0} % ({w}/{g})", w as f64 * 100.0 / g as f64) } else { "-".into() };
        ui.label(trf!("Deine Siegchance mit Offmeta-Pick im Team: {} · ohne: {}", "Your win rate with an off-meta pick on your team: {} · without: {}", wr(ww, wg), wr(ow, og)));
    }





    fn ui_labor(&mut self, ui: &mut egui::Ui) {
        self.scroll().show(ui, |ui| {
            let lab = &self.lab;
            {
                theme::page_header(
                    ui,
                    tr!("Matchmaking-Labor", "Matchmaking lab"),
                    tr!("Jedes Solo/Duo-Spiel mit laufendem lolstats wird hier protokolliert: Lobby-Stärke, \
                     Team-Vergleich, Serien und LP-Verlauf. Alles zur Nachbereitung, nichts davon greift ins Spiel ein.", "Every Solo/Duo game with lolstats running is logged here: lobby strength, team comparison, streaks and LP history. Everything is for review – none of it interferes with the game."),
                );
            }
            ui.add_space(6.0);

            help_header(ui, tr!("Kennzahlen", "Key figures"), crate::i18n::l(HELP_KPI));
            theme::section(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    kpi(ui, tr!("Spiele", "Games"), lab.rows.len().to_string());
                    kpi(ui, tr!("mit Lobby", "with lobby"), lab.records.len().to_string());
                    kpi(ui, tr!("Winrate", "Win rate"), if lab.total.games > 0 { pct(&lab.total) } else { "-".into() });
                    kpi(ui, tr!("Ø Sieg", "Avg win"), lab.avg_gain.map(|g| format!("+{g:.1} LP")).unwrap_or_else(|| "-".into()));
                    kpi(ui, tr!("Ø Niederlage", "Avg loss"), lab.avg_loss.map(|l| format!("-{l:.1} LP")).unwrap_or_else(|| "-".into()));
                });
            });
            ui.add_space(8.0);

            ui.add_space(8.0);

            // Loser-Queue-Test: vergleicht Mitspieler und Gegner
            theme::section(ui, |ui| self.ui_queue_test(ui));
            ui.add_space(8.0);
            help_header(ui, tr!("LP gegen Lobby-Ø minus Rang", "LP vs lobby avg minus rank"), crate::i18n::l(HELP_SCATTER));
            ui.label(RichText::new(tr!("Jeder Punkt = ein Spiel. Rechts = Lobby stärker als dein Rang, links = schwächer.", "Each dot = one game. Right = lobby stronger than your rank, left = weaker.")).weak());
            scatter(ui, lab);
            ui.add_space(8.0);
            help_header(ui, tr!("LP pro Spiel", "LP per game"), crate::i18n::l(HELP_DELTAS));
            delta_chart(ui, &lab.rows);
            ui.add_space(10.0);

            columns_or_stack(ui, 2, 330.0, |ui, i| {
                if i == 1 {
                    help_header(ui, tr!("Team-Ø gegen Gegner-Ø", "Team avg vs enemy avg"), crate::i18n::l(HELP_TEAMS));
                    egui::Grid::new("teams").striped(true).show(ui, |ui| {
                        for h in ["Lobby", tr!("Spiele", "Games"), "WR"] {
                            ui.label(RichText::new(h).weak());
                        }
                        ui.end_row();
                        for (label, a) in &lab.teams {
                            ui.label(*label);
                            ui.label(a.games.to_string());
                            ui.label(if a.games > 0 { wr_text(a.wins, a.games, pct(a)) } else { RichText::new("-") });
                            ui.end_row();
                        }
                    });
                    return;
                }
                help_header(ui, tr!("Serien", "Streaks"), crate::i18n::l(HELP_STREAKS));
                egui::Grid::new("streaks").striped(true).show(ui, |ui| {
                    for h in [tr!("Vor dem Spiel", "Before the game"), tr!("Spiele", "Games"), "WR", tr!("Ø Sieg", "Avg win"), tr!("Ø Niederl.", "Avg loss")] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for s in &lab.streaks {
                        ui.label(lab::streak_label(s.streak));
                        ui.label(s.all.games.to_string());
                        ui.label(if s.all.games > 0 { wr_text(s.all.wins, s.all.games, pct(&s.all)) } else { RichText::new("-") });
                        ui.label(s.gain.map(|g| format!("+{g:.1}")).unwrap_or_else(|| "-".into()));
                        ui.label(s.loss.map(|l| format!("-{l:.1}")).unwrap_or_else(|| "-".into()));
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.0);

            let cs = &self.comps;
            help_header(ui, tr!("Team-Kompositionen (alle gesammelten Matches)", "Team compositions (all collected matches)"), crate::i18n::l(HELP_COMPS));
            if cs.matches == 0 {
                ui.label(
                    RichText::new(tr!("Noch keine Daten. Die Spielstil-Werte der Champions lädt lolstats beim nächsten Client-Start bzw. per \"Aktualisieren\".", "No data yet. lolstats loads the champions' playstyle values on the next client start or via \"Refresh\"."))
                        .weak(),
                );
            } else {
                ui.label(
                    RichText::new(trf!(
                        "{} Matches · jede Zeile aus Sicht eines Teams (jedes Match zählt für Sieger und Verlierer)", "{} matches · each row from one team's point of view (every match counts for winner and loser)",
                        cs.matches
                    ))
                    .weak(),
                );
                let rows: [(&str, Vec<(&str, Agg)>); 3] = [
                    (tr!("Schadensprofil", "Damage profile"), cs.profile.iter().map(|(p, a)| (p.label(), *a)).collect()),
                    (tr!("CC gegenüber Gegner", "CC vs enemy"), cs.cc.clone()),
                    (tr!("Haltbarkeit gegenüber Gegner", "Durability vs enemy"), cs.durability.clone()),
                ];
                columns_or_stack(ui, 3, 230.0, |ui, i| {
                    let (title, lines) = &rows[i];
                    ui.label(RichText::new(*title).weak());
                    egui::Grid::new(("comp", i)).striped(true).show(ui, |ui| {
                        for (label, a) in lines {
                            ui.label(*label);
                            ui.label(trf!("{} T.", "{} t.", a.games));
                            ui.label(if a.games > 0 { wr_text(a.wins, a.games, pct(a)) } else { RichText::new("-") });
                            ui.end_row();
                        }
                    });
                });
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                help_header(ui, tr!("Protokoll", "Log"), crate::i18n::l(HELP_LOG));
                ui.selectable_value(&mut self.lab_filter, 0, tr!("Alle", "All"));
                ui.selectable_value(&mut self.lab_filter, 1, tr!("Siege", "Wins"));
                ui.selectable_value(&mut self.lab_filter, 2, tr!("Niederlagen", "Losses"));
                ui.checkbox(&mut self.lab_only_lobby, tr!("nur mit Lobby", "only with lobby"));
                if ui.button(tr!("Als CSV exportieren", "Export as CSV")).clicked() {
                    let path = self.store.p("labor.csv");
                    self.lab_msg = match std::fs::write(&path, lab::csv(&self.lab)) {
                        Ok(()) => trf!("Gespeichert: {}", "Saved: {}", path.display()),
                        Err(e) => trf!("Fehler: {e}", "Error: {e}"),
                    };
                }
            });
            if !self.lab_msg.is_empty() {
                ui.label(RichText::new(&self.lab_msg).italics());
            }
            let (filter, only_lobby) = (self.lab_filter, self.lab_only_lobby);
            let rows: Vec<&LabRow> = self
                .lab
                .rows
                .iter()
                .rev()
                .filter(|r| match filter {
                    1 => r.win,
                    2 => !r.win,
                    _ => true,
                })
                .filter(|r| !only_lobby || r.lobby.is_some())
                .collect();
            if rows.is_empty() {
                ui.label(RichText::new(tr!("Noch keine Spiele protokolliert.", "No games logged yet.")).weak());
                return;
            }
            wide(ui, "protokoll_scroll", |ui| egui::Grid::new("protokoll").striped(true).show(ui, |ui| {
                let mut head = vec!["", "LP"];
                head.extend([tr!("Rang vorher", "Rank before"), tr!("Rang nachher", "Rank after"), tr!("Lobby-Ø", "Lobby avg"), tr!("Team-Ø", "Team avg"), tr!("Gegner-Ø", "Enemy avg"), tr!("Serie", "Streak")]);
                head.push("vor");
                for h in head {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                let opt = |v: Option<i32>| v.map(div_label).unwrap_or_else(|| "-".into());
                for r in rows {
                    let (t, c) = if r.shield {
                        ("N*", Color32::GRAY)
                    } else if r.win {
                        (tr!("S", "W"), WIN)
                    } else {
                        (tr!("N", "L"), LOSS)
                    };
                    ui.label(RichText::new(t).strong().color(c));
                    ui.label(format!("{:+}", r.delta));
                    ui.label(value_label(r.rank_before));
                    ui.label(value_label(r.rank_after));
                    ui.label(opt(r.lobby));
                    ui.label(opt(r.team));
                    ui.label(opt(r.enemy));
                    ui.label(if r.streak == 0 { "-".into() } else { format!("{:+}", r.streak) });
                    ui.label(RichText::new(ago(r.ts)).weak());
                    ui.end_row();
                }
            }));
            ui.label(RichText::new(tr!("N* = Niederlage ohne LP-Verlust (Abstiegs- oder Verlustschutz, zählt nicht als Messpunkt).", "N* = loss without LP loss (demotion or loss protection, not counted as a data point).")).small().weak());
        });
    }

    fn ui_perf(&mut self, ui: &mut egui::Ui) {
        self.scroll().show(ui, |ui| {
            let gs = &self.games;
            let ex = self.cfg.expert;
            theme::page_header(ui, tr!("Leistung", "Performance"), tr!("Deine Spiele auf Summoner's Rift: Rekorde, Farmen, Spielstil und Matchups.", "Your Summoner's Rift games: records, farming, playstyle and matchups."));
            if gs.is_empty() {
                ui.label(RichText::new(tr!("Noch keine eigenen Spiele auf Summoner's Rift in der Datenbank.", "No own Summoner's Rift games in the database yet.")).weak());
                return;
            }
            if ex {
                ui.label(
                    RichText::new(trf!(
                        "{} eigene Spiele auf Summoner's Rift · {} davon mit Timeline (wird im Hintergrund nachgeladen)", "{} own Summoner's Rift games · {} of them with timeline (loaded in the background)",
                        gs.len(),
                        self.deaths.games
                    ))
                    .weak(),
                );
                ui.add_space(6.0);
            }

            let recent = &gs[gs.len().saturating_sub(20)..];
            let avg = |f: &dyn Fn(&GameStat) -> f64| recent.iter().map(f).sum::<f64>() / recent.len() as f64;
            let d = &self.deaths;
            theme::section(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    kpi(ui, tr!("Winrate (letzte 20)", "Win rate (last 20)"), format!("{:.0} %", avg(&|g| if g.win { 100.0 } else { 0.0 })));
                    kpi(ui, tr!("Ø KDA", "Avg KDA"), format!("{:.2}", avg(&|g| g.kda())));
                    kpi(ui, tr!("Ø CS/min", "Avg CS/min"), format!("{:.1}", avg(&|g| g.csm())));
                    kpi(ui, tr!("Ø CS nach 10 min", "Avg CS at 10 min"), d.cs10.map(|v| format!("{v:.0}")).unwrap_or_else(|| "-".into()));
                    kpi(ui, tr!("Ø Gold nach 10 min", "Avg gold at 10 min"), d.gold10.map(|v| format!("{v:.0}")).unwrap_or_else(|| "-".into()));
                });
            });
            ui.add_space(8.0);

            let tips: Vec<String> = gs
                .iter()
                .map(|g| format!("{} {} · {}", if g.win { tr!("Sieg", "Victory") } else { tr!("Niederlage", "Defeat") }, self.names.champ(g.champ), ago(g.ts)))
                .collect();
            let series = |f: &dyn Fn(&GameStat) -> f64| analysis::rolling(&gs.iter().map(f).collect::<Vec<_>>(), 10);
            theme::section(ui, |ui| {
                sub_title(ui, tr!("Winrate-Verlauf", "Win rate history"));
                ui.label(RichText::new(tr!("gleitender Schnitt über 10 Spiele", "rolling average over 10 games")).small().color(theme::MUTED));
                line_chart(ui, &series(&|g| if g.win { 100.0 } else { 0.0 }), WIN, &|v| format!("{v:.0} %"), &tips);
                if ex {
                    ui.label(RichText::new(tr!("CS pro Minute", "CS per minute")).weak());
                    line_chart(ui, &series(&|g| g.csm()), GOLD, &|v| format!("{v:.1}"), &tips);
                    ui.label(RichText::new("KDA").weak());
                    line_chart(ui, &series(&|g| g.kda()), BLUE, &|v| format!("{v:.2}"), &tips);
                }
            });
            ui.add_space(10.0);

            if !ex && d.games > 0 {
                theme::section(ui, |ui| {
                    sub_title(ui, tr!("Tode", "Deaths"));
                    let first = d.first_death.map(|f| trf!("erster Tod im Schnitt nach {}:{:02} min · ", "first death on average at {}:{:02} min ·", f as i32 / 60, f as i32 % 60)).unwrap_or_default();
                    ui.label(trf!("{first}in {:.0} % deiner Spiele kein Tod vor Minute 10", "{first}no death before minute 10 in {:.0} % of your games", d.clean_10 * 100.0));
                });
                ui.add_space(10.0);
            }
            if ex {
                theme::section(ui, |ui| {
                    ui.label(RichText::new(tr!("Death-Timings", "Death timings")).strong());
                    if d.games == 0 {
                        ui.label(RichText::new(tr!("Noch keine Timelines. lolstats lädt sie außerhalb von Spielen nach (eine pro 2 s).", "No timelines yet. lolstats loads them outside of games (one every 2 s).")).weak());
                        return;
                    }
                    ui.label(trf!(
                        "Ø Tode pro Spiel: Early (bis 14 min) {:.1} · Mid (14–25 min) {:.1} · Late (ab 25 min) {:.1}", "Avg deaths per game: early (until 14 min) {:.1} · mid (14–25 min) {:.1} · late (from 25 min) {:.1}",
                        d.early, d.mid, d.late
                    ));
                    if let Some(f) = d.first_death {
                        ui.label(trf!("Ø erster Tod nach {}:{:02} min", "Avg first death at {}:{:02} min", f as i32 / 60, f as i32 % 60));
                    }
                    ui.label(trf!("Ohne Tod vor Minute 10: {:.0} % deiner Spiele", "No death before minute 10: {:.0} % of your games", d.clean_10 * 100.0));
                    ui.label(RichText::new(trf!("Tode nach Spielminute ({} Spiele)", "Deaths by game minute ({} games)", d.games)).weak());
                    death_histogram(ui, gs);
                });
                ui.add_space(10.0);

                theme::section(ui, |ui| self.ui_records(ui));
                ui.add_space(10.0);
            }
            theme::section(ui, |ui| self.ui_coaching(ui));
            ui.add_space(10.0);
            if ex {
                theme::section(ui, |ui| self.ui_farm(ui));
                ui.add_space(10.0);
                theme::section(ui, |ui| self.ui_style(ui));
                ui.add_space(10.0);
            }

            let p = &self.pool;
            theme::section(ui, |ui| {
                sub_title(ui, &trf!("Champion-Pool (letzte {} Spiele)", "Champion pool (last {} games)", p.games));
                ui.label(
                    RichText::new(trf!("{} verschiedene Champions · die Top 3 machen {:.0} % deiner Spiele aus", "{} different champions · the top 3 make up {:.0} % of your games", p.distinct, p.top3 * 100.0))
                        .color(theme::MUTED),
                );
                if p.games < POOL_MIN_GAMES {
                    ui.label(RichText::new(trf!("Noch zu wenige Daten (< {POOL_MIN_GAMES} Spiele) für eine Pool-Bewertung.", "Not enough data yet (< {POOL_MIN_GAMES} games) to rate your pool.")).color(theme::MUTED));
                } else if p.top3 < POOL_FOCUS {
                    ui.label(RichText::new(trf!(
                        "Pool zu breit für einen effizienten Climb: Deine Top 3 machen unter {:.0} % aus. Weniger Champions = mehr Matchup-Erfahrung.", "Pool too wide for an efficient climb: your top 3 make up less than {:.0} %. Fewer champions = more matchup experience.",
                        POOL_FOCUS * 100.0
                    ))
                    .color(GOLD));
                }
                wide(ui, "pool_scroll", |ui| {
                    egui::Grid::new("pool").striped(true).show(ui, |ui| {
                        let head: &[&str] = if ex { &["Champion", tr!("Spiele", "Games"), tr!("Anteil", "Share"), "WR", "KDA", "CS/min"] } else { &["Champion", tr!("Spiele", "Games"), "WR"] };
                        for h in head {
                            ui.label(RichText::new(*h).weak());
                        }
                        ui.end_row();
                        for l in p.lines.iter().take(if ex { usize::MAX } else { 5 }) {
                            ui.horizontal(|ui| {
                                icons::show(ui, &self.icons, Kind::Champ, l.champ, 20.0);
                                ui.label(self.names.champ(l.champ));
                            });
                            ui.label(l.agg.games.to_string());
                            if ex {
                                ui.label(format!("{:.0} %", l.share * 100.0));
                            }
                            ui.label(wr_text(l.agg.wins, l.agg.games, pct(&l.agg)));
                            if ex {
                                ui.label(format!("{:.2}", l.kda));
                                ui.label(format!("{:.1}", l.csm));
                            }
                            ui.end_row();
                        }
                    });
                });
            });
            ui.add_space(10.0);

            // Beste Picks gegen einen bestimmten Gegner-Champion
            sub_title(ui, tr!("Beste Picks gegen …", "Best picks vs …"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.pick_search).hint_text(tr!("Gegner-Champion", "Enemy champion")).desired_width(180.0));
                let q = self.pick_search.trim().to_lowercase();
                if !q.is_empty() {
                    let mut hits: Vec<(i32, String)> = self
                        .names
                        .champs
                        .iter()
                        .filter(|(_, n)| n.to_lowercase().contains(&q))
                        .map(|(id, n)| (*id, n.clone()))
                        .collect();
                    hits.sort_by(|a, b| a.1.cmp(&b.1));
                    for (id, n) in hits.into_iter().take(6) {
                        if ui.selectable_label(self.pick_enemy == id, n).clicked() {
                            self.pick_enemy = id;
                        }
                    }
                }
            });
            if self.pick_enemy > 0 {
                let enemy = self.pick_enemy;
                let vs = analysis::wr_vs(&self.games, enemy);
                ui.label(trf!(
                    "Deine Bilanz gegen {}: {}", "Your record vs {}: {}",
                    self.names.champ(enemy),
                    if vs.games > 0 { trf!("{} Spiele, {} WR", "{} games, {} WR", vs.games, pct(&vs)) } else { tr!("noch keine Spiele", "no games yet").into() }
                ));
                let recs = analysis::recommend(&self.games, &self.matchups, &[enemy], "");
                rec_grid(ui, "pick_recs", &recs, &self.names);
            }
        });
    }

    /// Coaching: welche einfachen Merkmale gehen bei dir mit Siegen einher.
    fn ui_coaching(&self, ui: &mut egui::Ui) {
        let c = analysis::coaching(&self.games, &self.elo_gpm);
        help_header(ui, tr!("Coaching: Was entscheidet deine Spiele?", "Coaching: what decides your games?"), crate::i18n::l(HELP_COACH));
        let wr = |x: (usize, usize)| analysis::CoachLine::wr(x).map(|w| format!("{:.0} % ({}/{})", w * 100.0, x.1, x.0)).unwrap_or_else(|| "-".into());
        wide(ui, "coach_scroll", |ui| {
            egui::Grid::new("coach").striped(true).show(ui, |ui| {
                for h in [tr!("Wenn …", "If …"), tr!("Siegchance", "Win rate"), tr!("sonst", "otherwise"), tr!("Unterschied", "Difference")] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for l in &c.lines {
                    ui.label(l.label);
                    ui.label(wr(l.with));
                    ui.label(wr(l.without));
                    match l.gap() {
                        Some(g) => ui.label(RichText::new(trf!("{g:+.0} Pkt.", "{g:+.0} pts")).strong().color(if g >= 10.0 { WIN } else if g <= -10.0 { LOSS } else { theme::TEXT })),
                        None => ui.label(RichText::new(trf!("ab {} Spielen je Seite", "from {} games per side", analysis::COACH_MIN)).color(theme::MUTED)),
                    };
                    ui.end_row();
                }
            });
        });
        // größter belegter Hebel als Klartext
        if let Some((l, g)) = c.lines.iter().filter_map(|l| Some((l, l.gap()?))).max_by(|a, b| a.1.total_cmp(&b.1)).filter(|x| x.1 >= 10.0) {
            ui.label(RichText::new(trf!("Größter Hebel bisher: {} – damit gewinnst du {g:.0} Prozentpunkte öfter.", "Biggest lever so far: {} – with it you win {g:.0} percentage points more often.", l.label)).color(GOLD));
        }
        ui.add_space(4.0);
        if c.lead_games > 0 {
            let shrink = c.lead_shrinks.map(|s| trf!(" · In {:.0} % der Spiele mit Vorsprung bei 10 min schrumpft er bis Minute 15.", " · In {:.0} % of games with a lead at 10 min it shrinks by minute 15.", s * 100.0)).unwrap_or_default();
            ui.label(trf!(
                "CS gegen deinen Lane-Gegner: Ø {:+.1} bei 10 min → {:+.1} bei 15 min ({} Spiele){shrink}", "CS vs your lane opponent: avg {:+.1} at 10 min → {:+.1} at 15 min ({} games){shrink}",
                c.lead10, c.lead15, c.lead_games
            ));
        }
        if let Some(avg) = c.early_avg {
            let list = |v: &[(i32, f64, usize)]| v.iter().map(|(k, a, n)| trf!("{} Ø {a:.1} ({n})", "{} avg {a:.1} ({n})", self.names.champ(*k))).collect::<Vec<_>>().join(", ");
            let mut txt = trf!("Tode vor Minute 10: Ø {avg:.1} pro Spiel", "Deaths before minute 10: avg {avg:.1} per game");
            if !c.early_by_champ.is_empty() {
                txt += &trf!(" · am meisten mit {}", " · most with {}", list(&c.early_by_champ));
            }
            if !c.early_by_opp.is_empty() {
                txt += &trf!(" · gegen {}", " · vs {}", list(&c.early_by_opp));
            }
            ui.label(txt).on_hover_text(trf!(
                "Nur Spiele mit Timeline; Champions und Lane-Gegner ab {} Spielen, die drei höchsten Werte. In Klammern: Spiele.", "Only games with timeline; champions and lane opponents from {} games, the three highest values. In brackets: games.",
                analysis::EARLY_MIN_GAMES
            ));
        }
        let sw = analysis::session_wr(&self.games);
        let parts: Vec<String> = sw
            .iter()
            .zip(["1.", "2.", "3.", "4.+"])
            .filter(|(x, _)| x.0 > 0)
            .map(|(x, n)| trf!("{n} Spiel {}", "{n} game {}", wr(*x)))
            .collect();
        if !parts.is_empty() {
            ui.label(trf!("Siegchance nach Spielnummer in einer Session (Pause unter 2 h): {}", "Win rate by game number in a session (break under 2 h): {}", parts.join(" · ")))
                .on_hover_text(tr!("Fällt die Siegchance ab dem 3. oder 4. Spiel deutlich, lohnt es sich, Sessions kürzer zu halten.", "If your win rate drops clearly from the 3rd or 4th game, keep sessions shorter."));
        }
        if c.vision_games >= analysis::COACH_MIN {
            let (lo, hi) = (c.mid_deaths_low_vision, c.mid_deaths_high_vision);
            let note = if lo > hi + 0.3 { tr!(" – mehr Vision, weniger Midgame-Tode", " – more vision, fewer midgame deaths") } else { tr!(" – kein klarer Zusammenhang", " – no clear link") };
            ui.label(trf!(
                "Tode im Midgame (14–25 min): Ø {lo:.1} in Spielen mit wenig Vision, Ø {hi:.1} mit viel Vision ({} Spiele){note}", "Midgame deaths (14–25 min): avg {lo:.1} in games with little vision, avg {hi:.1} with a lot of vision ({} games){note}",
                c.vision_games
            ));
        } else {
            ui.label(
                RichText::new(trf!("Vision gegen Midgame-Tode: ab {} Spielen mit Timeline (bisher {}).", "Vision vs midgame deaths: from {} games with timeline (so far {}).", analysis::COACH_MIN, c.vision_games))
                    .color(theme::MUTED),
            );
        }
    }

    fn ui_records(&self, ui: &mut egui::Ui) {
        let r = &self.records;
        ui.label(RichText::new(tr!("Rekorde & Allzeit-Hochs", "Records & all-time highs")).strong());
        let s = &r.season;
        let rank = |v: Option<i32>| v.map(div_label).unwrap_or_else(|| "-".into());
        ui.label(trf!(
            "Peak diese Saison: {} · Vorsaison: Peak {}, Ende {}", "Peak this season: {} · last season: peak {}, end {}",
            rank(s.high),
            rank(s.prev_high),
            rank(s.prev_end)
        ));
        if let Some((v, ts)) = r.peak {
            ui.label(trf!("Höchster Stand seit lolstats läuft: {} ({})", "Highest rank since lolstats is running: {} ({})", value_label(v), ago(ts)));
        }
        ui.label(trf!("Längste Siegesserie: {} · längste Niederlagenserie: {}", "Longest win streak: {} · longest loss streak: {}", r.win_streak, r.loss_streak));
        let line = |ui: &mut egui::Ui, label: &str, b: &Option<Best>, fmt: &dyn Fn(f64) -> String| {
            ui.label(label);
            match b {
                Some(b) => {
                    ui.label(RichText::new(fmt(b.value)).strong());
                    ui.label(self.names.champ(b.champ));
                    ui.label(RichText::new(ago(b.ts)).weak());
                }
                None => {
                    ui.label("-");
                    ui.label("");
                    ui.label("");
                }
            }
            ui.end_row();
        };
        egui::Grid::new("records").striped(true).show(ui, |ui| {
            line(ui, tr!("Meiste Kills", "Most kills"), &r.kills, &|v| format!("{v:.0}"));
            line(ui, tr!("Beste KDA", "Best KDA"), &r.kda, &|v| format!("{v:.2}"));
            line(ui, tr!("Meiste CS", "Most CS"), &r.cs, &|v| format!("{v:.0}"));
            line(ui, tr!("Beste CS/min (ab 15 min)", "Best CS/min (from 15 min)"), &r.csm, &|v| format!("{v:.1}"));
            line(ui, tr!("Meister Schaden", "Most damage"), &r.dmg, &|v| format!("{v:.0}"));
            line(ui, tr!("Höchster Vision Score", "Highest vision score"), &r.vision, &|v| format!("{v:.0}"));
            line(ui, tr!("Längste Killing-Spree", "Longest killing spree"), &r.spree, &|v| format!("{v:.0}"));
        });
        ui.label(trf!(
            "Multi-Kills: {} Double · {} Triple · {} Quadra · {} Penta · First Bloods: {}", "Multi-kills: {} double · {} triple · {} quadra · {} penta · first bloods: {}",
            r.doubles, r.triples, r.quadras, r.pentas, r.first_bloods
        ));
        ui.label(
            RichText::new(tr!("Multi-Kills, Vision und Spree zählen ab Datenversion 2 (ältere Spiele werden im Hintergrund nachgeladen).", "Multi-kills, vision and sprees count from data version 2 (older games are reloaded in the background)."))
                .small()
                .weak(),
        );
    }

    fn ui_farm(&self, ui: &mut egui::Ui) {
        let f = &self.farm;
        ui.label(RichText::new(trf!("Farmen im Detail (letzte {} Spiele)", "Farming in detail (last {} games)", f.games)).strong());
        let o = |v: Option<f64>, d: usize| v.map(|x| format!("{x:.d$}")).unwrap_or_else(|| "-".into());
        ui.label(trf!("CS pro Minute: Lane {} · Jungle {}", "CS per minute: lane {} · jungle {}", o(f.lane_csm, 1), o(f.jungle_csm, 1)));
        ui.label(trf!("Ø CS nach 10 / 15 / 20 min: {} / {} / {}", "Avg CS at 10 / 15 / 20 min: {} / {} / {}", o(f.cs10, 0), o(f.cs15, 0), o(f.cs20, 0)));
        let signed = |v: Option<f64>| v.map(|x| format!("{x:+.1}")).unwrap_or_else(|| "-".into());
        let txt = trf!(
            "CS-Differenz zum direkten Lane-Gegner: {} nach 10 min · {} nach 15 min ({} Spiele)", "CS difference to your direct lane opponent: {} at 10 min · {} at 15 min ({} games)",
            signed(f.diff10),
            signed(f.diff15),
            f.diff_games
        );
        let col = match f.diff10 {
            Some(d) if d >= 5.0 => WIN,
            Some(d) if d <= -5.0 => LOSS,
            _ => ui.visuals().text_color(),
        };
        ui.label(RichText::new(txt).color(col));
    }

    fn ui_style(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr!("Spielstil-Profil", "Playstyle profile")).strong());
        let Some(s) = &self.style else {
            ui.label(RichText::new(tr!("Noch zu wenige vollständige Spiele (Datenversion 2). Wird im Hintergrund nachgeladen.", "Not enough complete games yet (data version 2). Loading in the background.")).weak());
            return;
        };
        ui.label(
            RichText::new(trf!(
                "Position {} · {} deiner Spiele · Vergleich mit {} Spielern {}", "Position {} · {} of your games · compared with {} players {}",
                position_label(s.position),
                s.games,
                s.peers,
                if s.near_elo { tr!("deiner Elo (±1 Tier)", "in your elo (±1 tier)") } else { tr!("aller Elos (für deine Elo noch zu wenige)", "of all elos (not enough yet for your elo)") }
            ))
            .weak(),
        );
        egui::Grid::new("style").striped(true).show(ui, |ui| {
            for h in ["", tr!("Du", "You"), tr!("Vergleich", "Comparison"), tr!("Unterschied", "Difference")] {
                ui.label(RichText::new(h).weak());
            }
            ui.end_row();
            for (i, (label, is_pct, higher_better)) in STYLE_LABELS.iter().enumerate() {
                let fmt = |v: f64| if *is_pct { format!("{:.0} %", v * 100.0) } else { format!("{v:.2}") };
                ui.label(tx(label));
                ui.label(fmt(s.mine[i]));
                match s.peer {
                    Some(p) if p[i].abs() > 1e-9 => {
                        ui.label(fmt(p[i]));
                        let rel = (s.mine[i] - p[i]) / p[i].abs();
                        let good = (rel > 0.0) == *higher_better;
                        let col = if rel.abs() < 0.05 { ui.visuals().text_color() } else if good { WIN } else { LOSS };
                        ui.label(RichText::new(format!("{:+.0} %", rel * 100.0)).color(col));
                    }
                    _ => {
                        ui.label("-");
                        ui.label("-");
                    }
                }
                ui.end_row();
            }
        });
    }

    fn ui_lp_chart(&self, ui: &mut egui::Ui) {
        let all: Vec<i32> = self.snaps.iter().filter_map(|s| s.rank.value()).collect();
        let vals = &all[all.len().saturating_sub(80)..];
        if vals.len() < 2 {
            ui.label(RichText::new(tr!("Noch zu wenig Datenpunkte für den LP-Verlauf.", "Not enough data points for the LP history yet.")).weak());
            return;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 150.0), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
        let lo = vals.iter().min().unwrap() - 20;
        let hi = vals.iter().max().unwrap() + 20;
        let span = (hi - lo).max(1) as f32;
        let y = |v: i32| rect.bottom() - 8.0 - (v - lo) as f32 / span * (rect.height() - 16.0);
        let mut d = (lo / 100 + 1) * 100;
        while d < hi {
            let yy = y(d);
            p.line_segment([Pos2::new(rect.left(), yy), Pos2::new(rect.right(), yy)], Stroke::new(1.0_f32, Color32::from_gray(70)));
            p.text(Pos2::new(rect.left() + 4.0, yy - 2.0), Align2::LEFT_BOTTOM, div_label(d), FontId::proportional(10.0), Color32::from_gray(150));
            d += 100;
        }
        let n = vals.len();
        let pts: Vec<Pos2> = vals
            .iter()
            .enumerate()
            .map(|(i, &v)| Pos2::new(rect.left() + 8.0 + i as f32 / (n - 1) as f32 * (rect.width() - 16.0), y(v)))
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, GOLD)));
    }

    fn ui_me(&self, ui: &mut egui::Ui) {
        let Some(me) = &self.me else {
            ui.label(tr!("Noch keine Daten. Starte den League-Client, dann liest lolstats dein Profil automatisch ein.", "No data yet. Start the League client and lolstats reads your profile automatically."));
            return;
        };
        self.scroll().show(ui, |ui| {
            let ex = self.cfg.expert;
            theme::page_header(ui, &me.name, tr!("Dein Profil", "Your profile"));
            // Kernaussagen als Kacheln
            ui.horizontal_wrapped(|ui| {
                if let Some(s) = self.snaps.last() {
                    let sub = if s.rank.games() > 0 {
                        trf!("{}S {}N · {:.0} % Winrate", "{}W {}L · {:.0} % win rate", s.rank.wins, s.rank.losses, s.rank.wins as f64 * 100.0 / s.rank.games() as f64)
                    } else {
                        tr!("noch keine Spiele", "no games yet").into()
                    };
                    theme::tile(ui, "Solo/Duo", &s.rank.label(), tier_color(&s.rank.tier), &sub);
                }
            });
            ui.add_space(8.0);
            if self.ui_tilt(ui) {
                ui.add_space(8.0);
            }
            theme::section(ui, |ui| self.ui_elo_pick(ui));
            ui.add_space(8.0);
            theme::section(ui, |ui| {
                sub_title(ui, tr!("LP-Verlauf", "LP history"));
                self.ui_lp_chart(ui);
                if ex {
                    ui.label(RichText::new(tr!("Alle Messpunkte, Diagramme und Auswertungen: Tab \"Labor\".", "All data points, charts and analyses: \"Lab\" tab.")).small().color(theme::MUTED));
                }
            });
            ui.add_space(10.0);
            let (n_champs, n_games) = if ex { (25, 25) } else { (5, 10) };
            let champs = |ui: &mut egui::Ui, icons: &Icons| {
                ui.label(RichText::new(if ex { tr!("Deine Champions (Summoner's Rift)", "Your champions (Summoner's Rift)") } else { tr!("Deine Top-Champions", "Your top champions") }).font(FontId::new(15.0, theme::bold())).color(Color32::WHITE));
                egui::Grid::new("mychamps").striped(true).show(ui, |ui| {
                    let head: &[&str] = if ex { &["Champion", tr!("Spiele", "Games"), "WR", "KDA", "CS/min"] } else { &["Champion", tr!("Spiele", "Games"), "WR"] };
                    for h in head {
                        ui.label(RichText::new(*h).weak());
                    }
                    ui.end_row();
                    for c in self.my_champs.iter().take(n_champs) {
                        ui.horizontal(|ui| {
                            icons::show(ui, icons, Kind::Champ, c.champ, 20.0);
                            ui.label(self.names.champ(c.champ));
                        });
                        ui.label(c.agg.games.to_string());
                        ui.label(wr_text(c.agg.wins, c.agg.games, pct(&c.agg)));
                        if ex {
                            ui.label(format!("{:.2}", c.kda()));
                            ui.label(format!("{:.1}", c.csm()));
                        }
                        ui.end_row();
                    }
                });
            };
            let recent = |ui: &mut egui::Ui, icons: &Icons| {
                ui.label(RichText::new(if ex { tr!("Letzte Spiele", "Recent games") } else { tr!("Letzte 10 Spiele", "Last 10 games") }).font(FontId::new(15.0, theme::bold())).color(Color32::WHITE));
                egui::Grid::new("recent").striped(true).show(ui, |ui| {
                    for &i in self.my_games.iter().take(n_games) {
                        let m = &self.matches[i];
                        let Some(p) = m.part(&me.puuid) else { continue };
                        let (t, c) = if m.remake() {
                            ("R", Color32::GRAY)
                        } else if p.win {
                            (tr!("S", "W"), WIN)
                        } else {
                            (tr!("N", "L"), LOSS)
                        };
                        ui.label(RichText::new(t).strong().color(c));
                        ui.horizontal(|ui| {
                            icons::show(ui, icons, Kind::Champ, p.champ, 20.0);
                            ui.label(self.names.champ(p.champ));
                        });
                        if ex {
                            ui.label(RichText::new(queue_name(m.queue)).weak());
                        }
                        ui.label(format!("{}/{}/{}", p.k, p.d, p.a));
                        if ex {
                            ui.label(format!("{:.1} CS/min", p.cs as f64 * 60.0 / m.dur.max(1) as f64));
                        }
                        ui.label(RichText::new(ago(m.ts)).weak());
                        ui.end_row();
                    }
                });
            };
            // Breit: beide Tabellen teilen sich die Breite gleichmäßig; schmal: untereinander.
            columns_or_stack(ui, 2, 440.0, |ui, i| {
                theme::section(ui, |ui| {
                    if i == 0 {
                        champs(ui, &self.icons);
                    } else {
                        wide(ui, "recent_scroll", |ui| recent(ui, &self.icons));
                    }
                });
            });
        });
    }


    /// Bester Pick für die eigene Elo auf der Hauptposition, darunter bis zu 4 Alternativen.
    fn ui_elo_pick(&self, ui: &mut egui::Ui) {
        let ep = &self.elo_picks;
        let pos = if ep.position.is_empty() { tr!("deiner Position", "your position").to_string() } else { position_label(ep.position).to_string() };
        let help = tr!("Aus allen gesammelten Matches nahe deiner Elo: geschätzte Winrate der anderen Spieler mit dem Champion \
                    auf deiner Hauptposition (60 %) plus deine eigene Winrate darauf (40 %).\n\
                    Elo-Fenster: ±1 Tier, bei weniger als 50 Spielen auf der Position ±2 Tiers, danach alle Elos.\n\
                    Bei wenigen Spielen im Fenster wird die Winrate zur Winrate des Champions in den anderen Elos gezogen, \
                    damit z. B. 4 Siege aus 5 Spielen nicht als 80 % gelten (markiert als \"wenig Daten\").\n\
                    Kandidaten: mind. 5 Spiele im Fenster, oder mind. 20 über alle Elos, oder mind. 3 eigene Spiele. \
                    Als Pick zählt, wer geschätzt mind. 50 % gewinnt oder bei dir selbst klar stark ist.", "From all collected matches near your elo: estimated win rate of other players with the champion on your main position (60 %) plus your own win rate on it (40 %).\nElo window: ±1 tier, with fewer than 50 games on the position ±2 tiers, then all elos.\nWith few games in the window the win rate is pulled towards the champion's win rate in other elos, so that e.g. 4 wins from 5 games don't count as 80 % (marked as \"little data\").\nCandidates: at least 5 games in the window, or at least 20 across all elos, or at least 3 own games. A pick counts if it is estimated to win at least 50 % or is clearly strong for you.");
        help_header(ui, &trf!("Bester Pick für deine Elo · {pos}", "Best pick for your elo · {pos}"), help);
        let Some(best) = ep.picks.first() else {
            ui.label(
                RichText::new(trf!(
                    "In deiner Elo gibt es auf {pos} noch keinen Champion mit genug Spielen und positiver Winrate \
                     ({} Matches gesammelt). lolstats sammelt automatisch weiter über deine Lobbys.", "In your elo there is no champion on {pos} with enough games and a positive win rate yet ({} matches collected). lolstats keeps collecting through your lobbies.",
                    ep.matches
                ))
                .color(theme::MUTED),
            );
            if let Some((c, a)) = &ep.own_best {
                ui.add_space(6.0);
                ui.label(RichText::new(tr!("Bis dahin aus deinen eigenen Spielen:", "Until then from your own games:")).small().color(theme::MUTED));
                ui.horizontal(|ui| {
                    icons::show(ui, &self.icons, Kind::Champ, *c, 40.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(self.names.champ(*c)).font(FontId::new(18.0, theme::bold())).color(Color32::WHITE));
                        ui.label(wr_text(a.wins, a.games, trf!("du: {} Winrate in {} Spielen auf {pos}", "you: {} win rate in {} games on {pos}", pct(a), a.games)));
                    });
                });
            }
            return;
        };
        let scope = match ep.span {
            Some(s) if s <= 400 => tr!("nahe deiner Elo (±1 Tier)", "near your elo (±1 tier)"),
            Some(_) => tr!("nahe deiner Elo (±2 Tiers, für ±1 noch zu wenige)", "near your elo (±2 tiers, not enough for ±1 yet)"),
            None => tr!("aus allen Elos (für deine Elo noch zu wenige)", "from all elos (not enough yet for your elo)"),
        };
        let mine = |a: &Agg| if a.games > 0 { trf!("du: {} Sp. · {}", "you: {} g · {}", a.games, pct(a)) } else { tr!("von dir noch nicht gespielt", "not played by you yet").into() };
        ui.horizontal(|ui| {
            icons::show(ui, &self.icons, Kind::Champ, best.champ, 48.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(self.names.champ(best.champ)).font(FontId::new(20.0, theme::bold())).color(Color32::WHITE));
                ui.label(wr_text(
                    (best.wr * 100.0).round() as i32,
                    100,
                    trf!("~{:.0} % Winrate {scope} ({} Spiele, gemessen {})", "~{:.0} % win rate {scope} ({} games, measured {})", best.wr * 100.0, best.elo.games, pct(&best.elo)),
                ));
                if best.thin() {
                    let out = Agg { games: best.all.games - best.elo.games, wins: best.all.wins - best.elo.wins };
                    let txt = if out.games > 0 {
                        trf!("wenig Daten: ergänzt um {} Spiele aus anderen Elos ({})", "little data: supplemented with {} games from other elos ({})", out.games, pct(&out))
                    } else {
                        tr!("wenig Daten: noch wenige Spiele, Schätzung zur Mitte gezogen", "little data: few games so far, estimate pulled towards the middle").to_string()
                    };
                    ui.label(RichText::new(txt).small().color(GOLD));
                }
                ui.label(RichText::new(mine(&best.mine)).small().color(theme::MUTED));
            });
        });
        if ep.picks.len() > 1 {
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("Alternativen", "Alternatives")).small().color(theme::MUTED));
            ui.horizontal_wrapped(|ui| {
                for p in &ep.picks[1..] {
                    ui.horizontal(|ui| {
                        icons::show(ui, &self.icons, Kind::Champ, p.champ, 22.0);
                        ui.label(self.names.champ(p.champ));
                        let thin = if p.thin() { tr!(" · wenig Daten", " · little data") } else { "" };
                        ui.label(wr_text((p.wr * 100.0).round() as i32, 100, format!("~{:.0} %", p.wr * 100.0))).on_hover_text(trf!(
                            "{} Spiele {scope}, gemessen {}{thin} · {}", "{} games {scope}, measured {}{thin} · {}",
                            p.elo.games,
                            pct(&p.elo),
                            mine(&p.mine)
                        ));
                    });
                    ui.add_space(10.0);
                }
            });
        }
    }

    fn ui_builds(&mut self, ui: &mut egui::Ui) {
        let list_frame = egui::Frame { inner_margin: egui::Margin { right: 12.0, ..Default::default() }, ..Default::default() };
        // Champion-Liste wächst bei breitem Fenster etwas mit (Namen werden nicht abgeschnitten)
        let list_w = (ui.available_width() * 0.2).clamp(190.0, 260.0);
        egui::SidePanel::left("champlist").resizable(false).exact_width(list_w).frame(list_frame).show_inside(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr!("🔍 Champion suchen", "🔍 Search champion")).desired_width(f32::INFINITY));
            let tagged = self.matches.iter().filter(|m| m.elo.is_some()).count();
            let mut clicked = None;
            if ui
                .checkbox(&mut self.build_near_elo, tr!("nur nahe meiner Elo", "only near my elo"))
                .on_hover_text(trf!(
                    "Nur Matches, die über Spieler innerhalb von ±1 Tier um deinen Rang gesammelt wurden. Bisher {tagged} von {} Matches mit Elo.", "Only matches collected through players within ±1 tier of your rank. So far {tagged} of {} matches with elo.",
                    self.matches.len()
                ))
                .changed()
                && self.sel_champ > 0
            {
                clicked = Some(self.sel_champ);
            }
            let q = self.search.to_lowercase();
            let shown: Vec<(&(i32, Agg), String)> = self
                .champ_list
                .iter()
                .map(|e| (e, self.names.champ(e.0)))
                .filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q))
                .collect();
            // Nur sichtbare Zeilen zeichnen (und deren Bilder laden)
            egui::ScrollArea::vertical().show_rows(ui, 32.0, shown.len(), |ui, range| {
                for ((c, agg), name) in &shown[range] {
                    if champ_row(ui, &self.icons, *c, name, agg, self.sel_champ == *c).clicked() {
                        clicked = Some(*c);
                    }
                }
            });
            if let Some(c) = clicked {
                self.select_champ(c);
            }
        });
        // Noch nichts gewählt: mit deinem meistgespielten Champion starten statt mit einer leeren Seite
        if self.build.is_none() && self.sel_champ == 0 {
            if let Some(c) = self.my_champs.first().map(|c| c.champ) {
                self.select_champ(c);
            }
        }
        let Some(b) = &self.build else {
            ui.add_space(44.0); // nicht unter den Aktualisieren-Knopf
            ui.label(trf!(
                "Champion links auswählen. Datenbasis: {} Spiele, gesammelt aus deinen Matches und denen deiner Lobby-Mitspieler.", "Select a champion on the left. Data: {} games, collected from your matches and those of your lobby mates.",
                self.matches.len()
            ));
            return;
        };
        let mut import: Option<RunePage> = None;
        let rune_lines = b.page.as_ref().map(|(pg, _)| {
            [(pg.styles[0], &pg.perks[..4]), (pg.styles[1], &pg.perks[4..])].map(|(st, ps)| (self.names.perk(st), ps.iter().map(|&x| self.names.perk(x)).collect::<Vec<_>>()))
        });
        let (names, icons, msg) = (&self.names, &self.icons, &self.msg);
        self.scroll().show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, icons, Kind::Champ, b.champ, 52.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(names.champ(b.champ)).font(FontId::new(24.0, theme::bold())).color(Color32::WHITE));
                    let scope = if self.build_near_elo { tr!(" · nahe deiner Elo", " · near your elo") } else { "" };
                    ui.label(RichText::new(trf!("{}{scope} · {} Spiele", "{}{scope} · {} games", b.patch, b.total.games)).color(theme::MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr!("Winrate", "Win rate")).small().color(theme::MUTED));
                        ui.label(wr_text(b.total.wins, b.total.games, pct(&b.total)).font(FontId::new(20.0, theme::bold())));
                    });
                });
            });
            ui.add_space(10.0);
            theme::section(ui, |ui| {
                sub_title(ui, tr!("Runen", "Runes"));
                match (&b.page, &rune_lines) {
                    (Some((pg, agg)), Some(lines)) => {
                        for (i, (style, perks)) in lines.iter().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                let col = if i == 0 { theme::GOLD } else { theme::ACCENT };
                                ui.label(RichText::new(style).font(FontId::new(14.0, theme::bold())).color(col));
                                for p in perks {
                                    chip(ui, p);
                                }
                            });
                        }
                        if pg.shards.iter().all(|&s| s > 0) {
                            let s: Vec<String> = pg.shards.iter().map(|&x| names.perk(x)).collect();
                            ui.label(RichText::new(s.join(" · ")).small().color(theme::MUTED));
                        }
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr!("In den Client importieren", "Import into client")).clicked() {
                                import = Some(pg.clone());
                            }
                            ui.label(RichText::new(trf!("{} Spiele · {} WR", "{} games · {} WR", agg.games, pct(agg))).color(theme::MUTED));
                        });
                    }
                    _ => {
                        ui.label(RichText::new(tr!("Keine Runendaten.", "No rune data.")).color(theme::MUTED));
                    }
                }
                if !msg.is_empty() {
                    ui.label(RichText::new(msg).italics());
                }
            });
            if let Some((sp, agg)) = &b.spells {
                ui.add_space(8.0);
                theme::section(ui, |ui| {
                    sub_title(ui, tr!("Beschwörerzauber", "Summoner spells"));
                    ui.horizontal_wrapped(|ui| {
                        chip(ui, &names.spell(sp[0]));
                        chip(ui, &names.spell(sp[1]));
                        ui.label(RichText::new(trf!("{} Spiele · {}", "{} games · {}", agg.games, pct(agg))).color(theme::MUTED));
                    });
                });
            }
            ui.add_space(8.0);
            theme::section(ui, |ui| {
                if let Some((id, agg)) = &b.boots {
                    sub_title(ui, tr!("Schuhe", "Boots"));
                    item_row(ui, icons, *id, &names.item(*id), agg.games as f32 / b.total.games.max(1) as f32, agg);
                    ui.add_space(6.0);
                }
                sub_title(ui, tr!("Häufigste fertige Items", "Most common finished items"));
                if b.items.is_empty() {
                    ui.label(RichText::new(tr!("Noch keine Item-Daten.", "No item data yet.")).color(theme::MUTED));
                }
                for (id, agg) in &b.items {
                    let rate = agg.games as f32 / b.total.games.max(1) as f32;
                    item_row(ui, icons, *id, &names.item(*id), rate, agg);
                }
            });
        });
        if let Some(pg) = import {
            let name = format!("lolstats {}", self.names.champ(self.sel_champ));
            self.msg = match Lcu::connect(&self.cfg.client_dir) {
                Some(l) => match lcu::import_runes(&l, &pg, &name) {
                    Ok(()) => tr!("Runenseite angelegt.", "Rune page created.").into(),
                    Err(e) => trf!("Fehler: {e}", "Error: {e}"),
                },
                None => tr!("League-Client läuft nicht.", "League client is not running.").into(),
            };
        }
    }

    /// Version und Updates (öffentliche Version): Prüfung über GitHub Releases, Installation per Knopf.
    fn ui_update(&self, ui: &mut egui::Ui) {
        use crate::update;
        let u = &self.update;
        sub_title(ui, tr!("Version & Updates", "Version & updates"));
        ui.label(trf!("Installiert: {}", "Installed: {}", update::VERSION));
        if u.available() {
            ui.label(RichText::new(trf!("Neue Version {} verfügbar", "New version {} available", u.latest)).strong().color(GOLD));
            if !u.notes.is_empty() {
                let notes: String = u.notes.lines().take(8).collect::<Vec<_>>().join("\n");
                ui.label(RichText::new(notes).color(theme::MUTED));
            }
            ui.horizontal(|ui| {
                if ui.button(tr!("Jetzt installieren", "Install now")).on_hover_text(tr!("Lädt die neue Version, prüft die Prüfsumme und startet lolstats neu (ca. 10 s).", "Downloads the new version, checks checksum and signature and restarts lolstats (about 10 s).")).clicked() {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("apply-update").spawn();
                    }
                }
                if !u.page.is_empty() && ui.button(tr!("Was ist neu?", "What's new?")).clicked() {
                    let _ = std::process::Command::new("explorer").arg(&u.page).spawn();
                }
            });
        } else if u.checked > 0 {
            ui.label(RichText::new(trf!("Du hast die neueste Version · zuletzt geprüft {}", "You have the latest version · last checked {}", ago(u.checked))).color(theme::MUTED));
        } else {
            ui.label(RichText::new(tr!("Noch nicht geprüft – lolstats sieht einmal am Tag nach.", "Not checked yet – lolstats checks once a day.")).color(theme::MUTED));
        }
        if !u.error.is_empty() {
            ui.label(RichText::new(&u.error).color(LOSS));
        }
        if ui.button(tr!("Jetzt nach Updates suchen", "Check for updates now")).clicked() {
            // im Hintergrund, damit das Fenster nicht hängt; das Ergebnis kommt über update.json
            let store = Store::open();
            std::thread::spawn(move || {
                let mut info: update::UpdateInfo = store.read(update::UPDATE).unwrap_or_default();
                match update::check() {
                    Ok(new) => info = new,
                    Err(e) => {
                        info.checked = now_ms();
                        info.error = trf!("Update-Prüfung fehlgeschlagen: {e}", "Update check failed: {e}");
                    }
                }
                store.write(update::UPDATE, &info);
            });
        }
        ui.label(RichText::new(trf!("Updates kommen ausschließlich von github.com/{} und werden per SHA-256 geprüft.", "Updates come only from github.com/{} and are checked via SHA-256 and signature.", update::REPO)).small().color(theme::MUTED));
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        self.scroll().id_salt("settings_scroll").show(ui, |ui| self.ui_settings_body(ui));
    }

    fn ui_settings_body(&mut self, ui: &mut egui::Ui) {
        theme::page_header(ui, tr!("Einstellungen", "Settings"), "");
        let mut changed = false;
        let mut lang_changed = false;
        theme::section(ui, |ui| {
            sub_title(ui, tr!("Ansicht", "View"));
            // Sprache: Wechsel lädt alles neu, damit auch berechnete Texte in der neuen Sprache entstehen
            ui.horizontal(|ui| {
                ui.label(tr!("Sprache", "Language"));
                let before = self.cfg.lang.clone();
                egui::ComboBox::from_id_salt("lang")
                    .selected_text(match self.cfg.lang.as_str() {
                        "de" => "Deutsch",
                        "en" => "English",
                        _ => tr!("Automatisch (Windows)", "Automatic (Windows)"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cfg.lang, String::new(), tr!("Automatisch (Windows)", "Automatic (Windows)"));
                        ui.selectable_value(&mut self.cfg.lang, "de".to_string(), "Deutsch");
                        ui.selectable_value(&mut self.cfg.lang, "en".to_string(), "English");
                    });
                if self.cfg.lang != before {
                    crate::i18n::apply(&self.cfg.lang);
                    lang_changed = true;
                }
            });
            ui.add_space(4.0);
            changed |= lang_changed;
            changed |= ui
                .checkbox(&mut self.cfg.expert, tr!("Expertenmodus", "Expert mode"))
                .on_hover_text(tr!("Zeigt das Labor, alle Tabellen und Diagramme. Aus: nur die Kernaussagen.", "Shows the lab, all tables and charts. Off: only the key points."))
                .changed();
            ui.label(RichText::new(tr!("Zeigt zusätzlich das Labor und alle Tabellen und Diagramme.", "Also shows the lab and all tables and charts.")).small().color(theme::MUTED));
            ui.add_space(8.0);
            sub_title(ui, tr!("Verhalten", "Behavior"));
            changed |= ui
                .checkbox(&mut self.cfg.crawl, tr!("Matches der Lobby-Mitspieler sammeln (für Builds, nie während eines Spiels)", "Collect matches of lobby players (for builds, never during a game)"))
                .changed();
            changed |= ui.checkbox(&mut self.cfg.auto_open_champselect, tr!("Fenster bei Champ Select automatisch öffnen", "Open the window automatically in champ select")).changed();
            ui.add_space(4.0);
            egui::Grid::new("settings_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tr!("Maximal gespeicherte Matches", "Maximum stored matches"));
                changed |= ui.add(egui::DragValue::new(&mut self.cfg.max_matches).speed(100)).changed();
                ui.end_row();
                ui.label(tr!("Client-Ordner", "Client folder"));
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut self.cfg.client_dir).hint_text(tr!("leer = automatisch", "empty = automatic")).desired_width(260.0))
                    .changed();
                ui.end_row();
            });
        });
        if changed {
            self.cfg.max_matches = self.cfg.max_matches.clamp(500, 500_000);
            self.store.write(CONFIG, &self.cfg);
        }
        if lang_changed {
            self.force_reload();
        }
        ui.add_space(8.0);
        theme::section(ui, |ui| {
            sub_title(ui, tr!("Daten", "Data"));
            ui.horizontal_wrapped(|ui| {
                kpi(ui, "Matches", self.matches.len().to_string());
                kpi(ui, tr!("eigene", "own"), self.my_games.len().to_string());
                kpi(ui, tr!("Lobbys", "Lobbies"), self.lobbies.len().to_string());
                kpi(ui, tr!("LP-Stände", "LP snapshots"), self.snaps.len().to_string());
                kpi(ui, tr!("Größe", "Size"), format!("{:.1} MB", self.store.db_size() as f64 / 1e6));
            });
            ui.add_space(4.0);
            let c = &self.state.crawl;
            let phase = self.state.phase.as_str();
            let (txt, col) = if !self.cfg.crawl {
                (tr!("Scraper: aus (siehe Verhalten)", "Scraper: off (see Behavior)").to_string(), theme::MUTED)
            } else if phase == "Client aus" || phase.is_empty() {
                (tr!("Scraper: pausiert – der League-Client ist geschlossen. Er sammelt nur bei offenem Client.", "Scraper: paused – the League client is closed. It only collects while the client is open.").to_string(), GOLD)
            } else if matches!(phase, "ChampSelect" | "GameStart" | "InProgress" | "Reconnect" | "WaitingForStats" | "PreEndOfGame") {
                (tr!("Scraper: pausiert während Champ Select und Spiel", "Scraper: paused during champ select and games").to_string(), theme::MUTED)
            } else if c.players == 0 && c.games == 0 {
                (trf!("Scraper: wartet auf neue Spieler · {} Matches seit Start", "Scraper: waiting for new players · {} matches since start", c.crawled), theme::MUTED)
            } else {
                (trf!("Scraper: läuft · {} Matches seit Start · Warteschlange {} Spieler, {} Spiele", "Scraper: running · {} matches since start · queue {} players, {} games", c.crawled, c.players, c.games), WIN)
            };
            ui.label(RichText::new(txt).color(col));
            if c.own_left > 0 {
                ui.label(RichText::new(trf!("Eigene Spiele: noch {} nachzuladen (Timelines, Zusatzwerte, älterer Verlauf)", "Own games: {} left to load (timelines, extra values, older history)", c.own_left)).color(theme::MUTED));
            }
            ui.label(RichText::new(format!("{DB} in {}", self.store.dir.display())).small().color(theme::MUTED));
            ui.horizontal(|ui| {
                if ui.button(tr!("Datenordner öffnen", "Open data folder")).clicked() {
                    let _ = std::process::Command::new("explorer").arg(&self.store.dir).spawn();
                }
                if ui.button(tr!("Tutorial öffnen", "Open tutorial")).on_hover_text(tr!("Was lolstats kann, wo du was findest und wie du die Zahlen liest", "What lolstats can do, where to find things and how to read the numbers")).clicked() {
                    self.tab = Tab::Help;
                }
            });
        });
        {
            ui.add_space(8.0);
            theme::section(ui, |ui| self.ui_update(ui));
            ui.add_space(8.0);
            theme::section(ui, |ui| {
                sub_title(ui, tr!("lolstats unterstützen", "Support lolstats"));
                ui.label(RichText::new(tr!("lolstats ist kostenlos und werbefrei. Wenn es dir hilft, freue ich mich über einen Kaffee auf Ko-fi.", "lolstats is free and ad-free. If it helps you, I'd appreciate a coffee on Ko-fi.")).color(theme::MUTED));
                if ui.button(tr!("☕ Einen Kaffee spendieren (Ko-fi)", "☕ Buy me a coffee (Ko-fi)")).on_hover_text(SUPPORT_URL).clicked() {
                    let _ = std::process::Command::new("explorer").arg(SUPPORT_URL).spawn();
                }
            });
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(trf!(
                "Version {} ({}) · Autostart: Rechtsklick auf das Tray-Icon -> \"Mit Windows starten\".", "Version {} ({}) · Autostart: right-click the tray icon -> \"Start with Windows\".",
                env!("CARGO_PKG_VERSION"),
                tx(EDITION)
            ))
            .small()
            .color(theme::MUTED),
        );
        ui.label(
            RichText::new("lolstats isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all associated properties are trademarks or registered trademarks of Riot Games, Inc.")
                .small()
                .color(theme::MUTED),
        );
    }
}

/// Unterstützer-Link (Buy Me a Coffee o. ä.); leer = Abschnitt ausgeblendet.
const SUPPORT_URL: &str = "https://ko-fi.com/daviddev25";

/// Kurzes Lane-Kürzel vor dem Champion; geschätzte Lanes mit "?" und blasser.
fn lane_badge(ui: &mut egui::Ui, lane: &str, guess: bool) {
    let short = match lane {
        "TOP" => "TOP",
        "JUNGLE" => "JGL",
        "MID" => "MID",
        "ADC" => "ADC",
        "SUPPORT" => "SUP",
        _ => "",
    };
    let text = if short.is_empty() { "–".to_string() } else if guess { format!("{short}?") } else { short.to_string() };
    let col = if short.is_empty() || guess { theme::MUTED } else { theme::ACCENT };
    let g = ui.painter().layout_no_wrap(text, FontId::new(11.0, theme::bold()), col);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(34.0, 18.0), Sense::hover());
    ui.painter().rect_filled(r, 4.0, theme::INSET);
    ui.painter().galley(r.center() - g.size() / 2.0, g, col);
    if !short.is_empty() {
        resp.on_hover_text(if guess {
            tr!("Lane geschätzt: aus der Champion-Statistik aller gesammelten Matches (und Smite = Jungle)", "Lane estimated: from the champion statistics of all collected matches (and Smite = jungle)")
        } else {
            tr!("Lane vom Client", "Lane from the client")
        });
    }
}


/// Ab dieser Inhaltsbreite stehen im Live-Tab zwei Karten bzw. die beiden Teams nebeneinander.
const WIDE_TWO_COL: f32 = 1000.0;
/// Experten-Ansicht: Team-Tabellen haben 7 Spalten und brauchen mehr Platz.
const WIDE_TEAMS_EXPERT: f32 = 1300.0;

/// Tutorial (TUTORIAL.md / TUTORIAL.en.md), von build.rs auf die jeweilige Version gefiltert.
const TUTORIAL: [&str; 2] = [include_str!(concat!(env!("OUT_DIR"), "/tutorial.md")), include_str!(concat!(env!("OUT_DIR"), "/tutorial.en.md"))];

/// Pool-Bewertung erst ab so vielen Spielen; "zu breit", wenn die Top 3 weniger als diesen Anteil ausmachen.
const POOL_MIN_GAMES: usize = 10;
const POOL_FOCUS: f64 = 0.6;

/// Eigener Spieler und sein direkter Gegner auf derselben Lane (nur wenn die Lane bekannt ist).
fn lane_opponent(lb: &Lobby) -> Option<(&LivePlayer, &LivePlayer)> {
    let me = lb.players.iter().find(|p| p.me)?;
    if me.lane.is_empty() {
        return None;
    }
    let opp = lb.players.iter().find(|p| p.team != me.team && p.lane == me.lane)?;
    Some((me, opp))
}

/// Nebeneinander, wenn jede Spalte mindestens `min_w` breit wird, sonst untereinander.
fn columns_or_stack(ui: &mut egui::Ui, n: usize, min_w: f32, mut f: impl FnMut(&mut egui::Ui, usize)) {
    if ui.available_width() >= n as f32 * min_w {
        ui.columns(n, |cols| {
            for (i, c) in cols.iter_mut().enumerate() {
                f(c, i);
            }
        });
    } else {
        for i in 0..n {
            f(ui, i);
            ui.add_space(8.0);
        }
    }
}

/// Breite Tabelle: bei schmalem Fenster seitlich scrollbar statt überlappend.
fn wide<R>(ui: &mut egui::Ui, id: impl std::hash::Hash, add: impl FnOnce(&mut egui::Ui) -> R) {
    egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, true]).show(ui, add);
}

fn sub_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(FontId::new(15.0, theme::bold())).color(Color32::WHITE));
    ui.add_space(2.0);
}

/// Tutorial-Seite: TUTORIAL.md (in die exe eingebettet) mit einer kleinen Markdown-Darstellung.
fn ui_help(ui: &mut egui::Ui, scroll: egui::ScrollArea) {
    let width = ui.available_width().min(900.0);
    scroll.id_salt("help_scroll").show(ui, |ui| {
        ui.set_max_width(width);
        let mut table: Vec<Vec<String>> = Vec::new();
        let mut n_table = 0;
        let lines: Vec<&str> = crate::i18n::l(TUTORIAL).lines().collect();
        for (i, raw) in lines.iter().enumerate() {
            let line = raw.trim_end();
            // Versionsabschnitte filtert schon build.rs; übrige HTML-Kommentare nicht anzeigen
            if line.starts_with("<!--") {
                continue;
            }
            // Tabellenzeilen sammeln, beim Ende der Tabelle zeichnen
            if line.starts_with('|') {
                let cells: Vec<String> = line.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                if !cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':')) {
                    table.push(cells);
                }
                if lines.get(i + 1).is_some_and(|n| n.starts_with('|')) {
                    continue;
                }
                n_table += 1;
                md_table(ui, n_table, &std::mem::take(&mut table), width);
                continue;
            }
            if let Some(t) = line.strip_prefix("# ") {
                theme::page_header(ui, t, "");
            } else if let Some(t) = line.strip_prefix("## ") {
                ui.add_space(14.0);
                ui.label(RichText::new(t).font(FontId::new(19.0, theme::bold())).color(Color32::WHITE));
                ui.add_space(4.0);
            } else if let Some(t) = line.strip_prefix("### ") {
                ui.add_space(8.0);
                ui.label(RichText::new(t).font(FontId::new(15.5, theme::bold())).color(theme::ACCENT));
            } else if line == "---" {
                ui.add_space(6.0);
                ui.separator();
            } else if let Some(t) = line.strip_prefix("> ") {
                egui::Frame { fill: theme::INSET, inner_margin: egui::Margin::symmetric(12.0, 8.0), rounding: 6.0.into(), ..Default::default() }.show(ui, |ui| {
                    ui.set_min_width(width - 24.0);
                    md_text(ui, t, GOLD, width - 24.0);
                });
            } else if let Some(t) = line.trim_start().strip_prefix("- ") {
                let indent = (line.len() - line.trim_start().len()) as f32 * 6.0;
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0 + indent);
                    ui.label(RichText::new("•").color(theme::ACCENT));
                    md_text(ui, t, theme::TEXT, width - 26.0 - indent);
                });
            } else if line.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
                let (n, t) = line.split_once(". ").unwrap();
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{n}.")).color(theme::ACCENT));
                    md_text(ui, t, theme::TEXT, width - 34.0);
                });
            } else if line.is_empty() {
                ui.add_space(4.0);
            } else {
                md_text(ui, line, theme::TEXT, width);
            }
        }
        ui.add_space(20.0);
    });
}

/// Fließtext mit **fett** und `Code`, umbrechend auf `width` (in Scrollbereichen ist die verfügbare Breite unbegrenzt).
fn md_text(ui: &mut egui::Ui, text: &str, color: Color32, width: f32) {
    md_label(ui, md_job(text, color, width.max(120.0)));
}

/// Fertig gesetzter Text: egui übernimmt die Umbruchbreite so (ein LayoutJob würde auf die
/// verfügbare Breite neu umbrochen, die in Scrollbereichen mit Tabellen unbegrenzt ist).
fn md_label(ui: &mut egui::Ui, job: egui::text::LayoutJob) {
    let galley = ui.fonts(|f| f.layout_job(job));
    ui.label(galley);
}

fn md_job(text: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for (i, part) in text.split("**").enumerate() {
        let bold = i % 2 == 1;
        for (j, seg) in part.split('`').enumerate() {
            let code = j % 2 == 1;
            // *kursiv* nur außerhalb von Code
            let pieces: Vec<(bool, &str)> = if code { vec![(false, seg)] } else { seg.split('*').enumerate().map(|(k, s)| (k % 2 == 1, s)).collect() };
            for (italic, s) in pieces {
                if s.is_empty() {
                    continue;
                }
                let fmt = TextFormat {
                    font_id: if code {
                        FontId::monospace(12.5)
                    } else if bold {
                        FontId::new(14.0, theme::bold())
                    } else {
                        FontId::proportional(14.0)
                    },
                    color: if code { theme::ACCENT } else if bold { Color32::WHITE } else { color },
                    italics: italic,
                    ..Default::default()
                };
                job.append(s, 0.0, fmt);
            }
        }
    }
    job
}

/// Markdown-Tabelle: erste Zeile als Kopf.
fn md_table(ui: &mut egui::Ui, id: usize, rows: &[Vec<String>], width: f32) {
    let Some((head, body)) = rows.split_first() else { return };
    ui.add_space(4.0);
    // Spaltenbreiten: erste Spalte schmaler, Rest teilt sich die Breite, Zellen brechen um
    let n = head.len().max(1) as f32;
    let total = width - 14.0 * n;
    let first = (total / n).min(200.0);
    let rest = if n > 1.0 { ((total - first) / (n - 1.0)).max(110.0) } else { total };
    let col_w = |i: usize| if i == 0 { first } else { rest };
    egui::Grid::new(("help_table", id)).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
        for (i, h) in head.iter().enumerate() {
            md_label(ui, md_job(h, theme::MUTED, col_w(i)));
        }
        ui.end_row();
        for r in body {
            for (i, c) in r.iter().enumerate() {
                md_label(ui, md_job(c, theme::TEXT, col_w(i)));
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
}

/// Kleines abgerundetes Kästchen für Runen- und Zaubernamen (feste Größe, bricht als Ganzes um).
fn chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.5), theme::TEXT);
    let (r, resp) = ui.allocate_exact_size(g.size() + egui::vec2(16.0, 6.0), Sense::hover());
    ui.painter().rect_filled(r, 6.0, theme::INSET);
    ui.painter().galley(r.min + egui::vec2(8.0, 3.0), g, theme::TEXT);
    resp
}

/// Eintrag der Champion-Liste: Porträt, Name, Spielanzahl.
fn champ_row(ui: &mut egui::Ui, icons: &Icons, champ: i32, name: &str, agg: &Agg, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(56, 189, 178, 32));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(8));
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(5.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    icons::show(&mut child, icons, Kind::Champ, champ, 26.0);
    child.label(RichText::new(name).color(if selected { Color32::WHITE } else { theme::TEXT }));
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(agg.games.to_string()).small().color(theme::MUTED));
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Item-Zeile über die volle Breite: Bild, Name, Winrate rechts, darunter Pickrate als Balken.
fn item_row(ui: &mut egui::Ui, icons: &Icons, id: i32, name: &str, rate: f32, agg: &Agg) {
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::hover());
    let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(row.min + egui::vec2(0.0, 3.0), egui::vec2(34.0, 34.0))));
    icons::show(&mut icon_ui, icons, Kind::Item, id, 34.0);
    let p = ui.painter_at(row);
    let x = row.left() + 46.0;
    let wr = p.layout_no_wrap(format!("{} WR", pct(agg)), FontId::new(13.0, theme::bold()), wr_color(agg.wins, agg.games));
    let wr_w = wr.size().x;
    p.galley(Pos2::new(row.right() - wr_w, row.top() + 2.0), wr, Color32::WHITE);
    let name_w = (row.right() - wr_w - 10.0 - x).max(20.0);
    // Zu lange Namen mit "…" kürzen, bis sie neben die Winrate passen
    let mut chars: Vec<char> = name.chars().collect();
    let mut ng = p.layout_no_wrap(name.to_string(), FontId::proportional(14.0), Color32::WHITE);
    while ng.size().x > name_w && chars.len() > 3 {
        chars.pop();
        ng = p.layout_no_wrap(format!("{}…", chars.iter().collect::<String>().trim_end()), FontId::proportional(14.0), Color32::WHITE);
    }
    p.galley(Pos2::new(x, row.top() + 1.0), ng, Color32::WHITE);
    let info = p.layout_no_wrap(trf!("{:.0} % · {} Sp.", "{:.0} % · {} g", rate * 100.0, agg.games), FontId::proportional(11.5), theme::MUTED);
    let bar_w = (row.right() - x - info.size().x - 10.0).max(30.0);
    let bar = Rect::from_min_size(Pos2::new(x, row.top() + 27.0), egui::vec2(bar_w, 6.0));
    p.rect_filled(bar, 3.0, theme::INSET);
    p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar_w * rate.clamp(0.0, 1.0), 6.0)), 3.0, theme::ACCENT);
    p.galley(Pos2::new(row.right() - info.size().x, bar.center().y - info.size().y / 2.0), info, theme::MUTED);
    ui.add_space(4.0);
}

fn wr_color(wins: i32, games: i32) -> Color32 {
    let wr = if games > 0 { wins as f64 / games as f64 } else { 0.5 };
    if games < 3 || (0.45..0.55).contains(&wr) {
        theme::TEXT
    } else if wr >= 0.55 {
        WIN
    } else {
        LOSS
    }
}

/// Kennzahl als Block fester Größe, damit sie in umbrechenden Zeilen als Ganzes umbricht.
fn kpi(ui: &mut egui::Ui, label: &str, value: String) -> egui::Response {
    let top = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.5), theme::MUTED);
    let val = ui.painter().layout_no_wrap(value, FontId::new(19.0, theme::bold()), Color32::WHITE);
    let size = egui::vec2(top.size().x.max(val.size().x) + 18.0, top.size().y + val.size().y + 4.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().galley(r.min, top.clone(), theme::MUTED);
    ui.painter().galley(r.min + egui::vec2(0.0, top.size().y + 4.0), val, Color32::WHITE);
    resp
}

/// Zeichenfläche mit Wertebereich; Ränder links für die Achsenbeschriftung.
struct Chart {
    rect: Rect,
    painter: Painter,
    hover: Option<Pos2>,
    x: (f32, f32),
    y: (f32, f32),
}

impl Chart {
    fn new(ui: &mut egui::Ui, height: f32, x: (f32, f32), y: (f32, f32)) -> Chart {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
        Chart { rect, painter, hover: resp.hover_pos(), x, y }
    }
    fn pos(&self, x: f32, y: f32) -> Pos2 {
        let fx = (x - self.x.0) / (self.x.1 - self.x.0).max(1e-6);
        let fy = (y - self.y.0) / (self.y.1 - self.y.0).max(1e-6);
        Pos2::new(
            self.rect.left() + 44.0 + fx * (self.rect.width() - 54.0),
            self.rect.bottom() - 18.0 - fy * (self.rect.height() - 28.0),
        )
    }
    fn hline(&self, y: f32, label: String, strong: bool) {
        let p = self.pos(self.x.0, y);
        let col = Color32::from_gray(if strong { 110 } else { 60 });
        self.painter.line_segment([Pos2::new(self.rect.left() + 40.0, p.y), Pos2::new(self.rect.right() - 6.0, p.y)], Stroke::new(1.0_f32, col));
        self.painter.text(Pos2::new(self.rect.left() + 4.0, p.y), Align2::LEFT_CENTER, label, FontId::proportional(10.0), Color32::from_gray(150));
    }
    fn note(&self, text: &str) {
        self.painter.text(self.rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), Color32::from_gray(140));
    }
    /// Kleines Info-Kästchen beim nächstgelegenen Punkt unter der Maus.
    fn tooltip(&self, pts: &[(Pos2, String)]) {
        let Some(h) = self.hover else { return };
        let Some((p, text)) = pts
            .iter()
            .filter(|(p, _)| p.distance(h) < 12.0)
            .min_by(|a, b| a.0.distance(h).total_cmp(&b.0.distance(h)))
        else {
            return;
        };
        let galley = self.painter.layout_no_wrap(text.clone(), FontId::proportional(12.0), Color32::WHITE);
        let mut at = *p + egui::vec2(10.0, -10.0 - galley.size().y);
        at.x = at.x.min(self.rect.right() - galley.size().x - 8.0);
        let bg = Rect::from_min_size(at - egui::vec2(4.0, 2.0), galley.size() + egui::vec2(8.0, 4.0));
        self.painter.rect_filled(bg, 3.0, Color32::from_black_alpha(220));
        self.painter.galley(at, galley, Color32::WHITE);
    }
}

fn scatter(ui: &mut egui::Ui, lab: &Lab) {
    let recs = &lab.records;
    let xs: Vec<f32> = recs.iter().map(|r| (r.lobby - r.rank) as f32).collect();
    let span = xs.iter().fold(150.0f32, |m, x| m.max(x.abs() + 30.0));
    let ymax = recs.iter().fold(35.0f32, |m, r| m.max(r.delta.abs() as f32 + 5.0));
    let c = Chart::new(ui, 220.0, (-span, span), (-ymax, ymax));
    if recs.is_empty() {
        c.note(tr!("Noch keine Messpunkte mit Lobby", "No data points with lobby yet"));
        return;
    }
    let mut y = -((ymax / 10.0).floor() * 10.0);
    while y <= ymax {
        c.hline(y, format!("{y:+.0}"), y == 0.0);
        y += 10.0;
    }
    let mid = c.pos(0.0, 0.0);
    c.painter.line_segment([Pos2::new(mid.x, c.rect.top() + 4.0), Pos2::new(mid.x, c.rect.bottom() - 18.0)], Stroke::new(1.0_f32, Color32::from_gray(110)));
    c.painter.text(Pos2::new(c.rect.right() - 6.0, c.rect.bottom() - 4.0), Align2::RIGHT_BOTTOM, tr!("Lobby stärker ->", "Lobby stronger ->"), FontId::proportional(10.0), Color32::from_gray(150));
    c.painter.text(Pos2::new(c.rect.left() + 48.0, c.rect.bottom() - 4.0), Align2::LEFT_BOTTOM, tr!("<- Lobby schwächer", "<- Lobby weaker"), FontId::proportional(10.0), Color32::from_gray(150));
    // Modellgeraden: Sieg = a + b·x, Niederlage = -(a - b·x)
    let mut tips = Vec::new();
    for (r, &x) in recs.iter().zip(&xs) {
        let p = c.pos(x, r.delta as f32);
        c.painter.circle_filled(p, 4.0, if r.win { WIN } else { LOSS });
        tips.push((p, trf!("{} {:+} LP · Lobby {} · Rang {} · {}", "{} {:+} LP · lobby {} · rank {} · {}", if r.win { tr!("Sieg", "Victory") } else { tr!("Niederlage", "Defeat") }, r.delta, div_label(r.lobby), value_label(r.rank), ago(r.ts))));
    }
    c.tooltip(&tips);
}

fn delta_chart(ui: &mut egui::Ui, rows: &[LabRow]) {
    let rows = &rows[rows.len().saturating_sub(60)..];
    let ymax = rows.iter().fold(35.0f32, |m, r| m.max(r.delta.abs() as f32 + 5.0));
    let n = rows.len().max(1) as f32;
    let c = Chart::new(ui, 170.0, (0.0, n), (-ymax, ymax));
    if rows.is_empty() {
        c.note(tr!("Noch keine Spiele", "No games yet"));
        return;
    }
    for y in [-30.0f32, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0] {
        if y.abs() <= ymax {
            c.hline(y, format!("{y:+.0}"), y == 0.0);
        }
    }
    let w = ((c.rect.width() - 54.0) / n * 0.7).clamp(2.0, 14.0);
    let mut tips = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let x = i as f32 + 0.5;
        let col = if r.shield { Color32::GRAY } else if r.win { WIN } else { LOSS };
        let (top, bottom) = (c.pos(x, r.delta.max(0) as f32), c.pos(x, r.delta.min(0) as f32));
        let bar = Rect::from_min_max(Pos2::new(top.x - w / 2.0, top.y), Pos2::new(top.x + w / 2.0, bottom.y.max(top.y + 2.0)));
        c.painter.rect_filled(bar, 1.0, col);
        let pred = "";
        tips.push((c.pos(x, r.delta as f32), format!("{:+} LP{pred} · {} · {}", r.delta, value_label(r.rank_after), ago(r.ts))));
    }
    c.tooltip(&tips);
}


fn rec_grid(ui: &mut egui::Ui, id: &str, recs: &[Rec], names: &Names) {
    if recs.is_empty() {
        ui.label(RichText::new(tr!("Noch zu wenige Daten: mindestens 3 eigene Spiele pro Champion.", "Not enough data yet: at least 3 own games per champion.")).weak());
        return;
    }
    egui::Grid::new(id).striped(true).show(ui, |ui| {
        for h in ["Champion", "Score", tr!("Deine WR", "Your WR"), "KDA", tr!("vs. Gegner (alle Matches)", "vs enemies (all matches)"), tr!("Du vs. Gegner", "You vs enemies"), tr!("auf Position", "on position")] {
            ui.label(RichText::new(h).weak());
        }
        ui.end_row();
        let agg = |a: &Agg| if a.games > 0 { wr_text(a.wins, a.games, format!("{} ({})", pct(a), a.games)) } else { RichText::new("-") };
        for (i, r) in recs.iter().enumerate() {
            let name = RichText::new(names.champ(r.champ));
            ui.label(if i == 0 { name.strong().color(GOLD) } else { name });
            ui.label(format!("{:.0}", r.score * 100.0));
            ui.label(agg(&r.mine));
            ui.label(format!("{:.2}", r.kda));
            ui.label(agg(&r.vs_db));
            ui.label(agg(&r.vs_me));
            ui.label(if r.on_pos > 0 { trf!("{} Sp.", "{} g", r.on_pos) } else { "-".into() });
            ui.end_row();
        }
    });
    ui.label(
        RichText::new(
            tr!("Score: 50 % deine Stärke auf dem Champion, 35 % Matchup gegen die Gegner aus allen gesammelten Matches, \
             15 % deine eigenen Spiele gegen diese Gegner. Kleine Stichproben zählen Richtung 50 %.", "Score: 50 % your strength on the champion, 35 % matchup against the enemies from all collected matches, 15 % your own games against these enemies. Small samples count towards 50 %."),
        )
        .small()
        .weak(),
    );
}



/// Abschnitts-Überschrift mit "(?)", das beim Drüberfahren erklärt, wie man den Abschnitt liest.
fn help_header(ui: &mut egui::Ui, title: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(FontId::new(15.0, theme::bold())).color(Color32::WHITE)).on_hover_text(help);
        theme::help_icon(ui).on_hover_text(help);
    });
}







const HELP_QUEUE: [&str; 2] = ["Prüft die 'Loser Queue'-Vermutung: Bekommst du nach einer Siegesserie schlechtere Mitspieler als die Gegner?\n\
Pro Spiel: Mitspieler (ohne dich) minus Gegner – Troll/AFK-Quote (AFK = unter halbem Team-Gold, Troll = 10+ Tode bei KDA unter 1), \
Rang-Schnitt, Form (Winrate der letzten 20), KDA im Spiel.\n\
Verglichen wird 'nach 2+ Siegen in Folge' mit 'sonst'. Gäbe es eine Loser Queue, wären die Mitspieler nach Serien \
messbar schlechter als die Gegner (negativer Rang/Form/KDA-Wert, positive Troll-Quote).\n\
p = Zufallstest mit 2000 zufälligen Gruppeneinteilungen. Urteil ab 15 Spielen je Gruppe.", "Tests the 'loser queue' theory: do you get worse teammates than the enemies after a win streak?\nPer game: teammates (without you) minus enemies – troll/AFK rate (AFK = under half the team's gold, troll = 10+ deaths with KDA under 1), average rank, form (win rate of the last 20), KDA in the game.\nCompared: 'after 2+ wins in a row' vs 'otherwise'. If there was a loser queue, teammates after streaks would be measurably worse than the enemies (negative rank/form/KDA value, positive troll rate).\np = random test with 2000 random group assignments. Verdict from 15 games per group."];

const HELP_MATCHUP_RUNES: [&str; 2] = ["Aus allen gesammelten Matches: Spieler deines Champions gegen genau diesen Lane-Gegner. \
Gezeigt werden Runen, die dabei mindestens 15 Prozentpunkte häufiger gewählt werden als gegen alle anderen Gegner \
(ab 8 Spielen im Matchup, je Rune mindestens 3). Maus über eine Rune: Anteil und Bilanz.", "From all collected matches: players of your champion against exactly this lane opponent. Shown are runes picked at least 15 percentage points more often than against all other opponents (from 8 games in the matchup, at least 3 per rune). Hover a rune: share and record."];

const HELP_COACH: [&str; 2] = ["Nur deine eigenen Spiele auf Summoner's Rift. Für jedes Merkmal: Siegchance in Spielen mit dem Merkmal gegen Spiele ohne.\n\
Unterschied erst ab 10 Spielen je Seite. Grün ab +10 Prozentpunkten. Das zeigt Zusammenhänge, keine Ursachen – \
ein CS-Vorsprung kann auch Folge eines schwachen Gegners sein.\n\
CS-Vorsprung: dein CS minus der deines direkten Lane-Gegners (Timeline, inkl. Jungle-Monster).\n\
Vision: Spiele ab 20 min, geteilt am Median deiner Vision pro Minute; Midgame = Minute 14 bis 25.", "Only your own Summoner's Rift games. For each trait: win rate in games with the trait vs games without.\nDifference only from 10 games per side. Green from +10 percentage points. This shows correlations, not causes – a CS lead can also be the result of a weak opponent.\nCS lead: your CS minus your direct lane opponent's (timeline, incl. jungle monsters).\nVision: games from 20 min, split at the median of your vision per minute; midgame = minute 14 to 25."];

const HELP_CS_TACHO: [&str; 2] = ["Dein CS im laufenden Spiel, alle 15 Sekunden aus der Live-Client-API.\n\
Achtung: Riot meldet CS dort nur in 10er-Schritten (für alle Spieler) – der Wert springt deshalb nur etwa alle 1–2 Minuten. \
'Stand vor X s' zeigt, wann zuletzt abgefragt wurde.\n\
Dein Schnitt: Ø CS bei 10, 15 und 20 Minuten aus deinen Timelines, dazwischen linear – ab 3 Spielen nur auf diesem Champion.\n\
Elo: Ø CS/min dieses Champions über ganze Spiele in deiner Elo (±400 LP, sonst alle Elos), umgerechnet auf die Zeit \
seit den ersten Vasallen (~1:15). Grob – die Farm-Rate ändert sich im Spielverlauf.\n\
Lane-Gegner: CS deines direkten Gegners (gleiche Lane).", "Your CS in the running game, every 15 seconds from the live client API.\nNote: Riot reports CS there only in steps of 10 (for all players) – so the value only changes every 1–2 minutes. 'Updated X s ago' shows when it was last queried.\nYour average: avg CS at 10, 15 and 20 minutes from your timelines, linear in between – from 3 games only on this champion.\nElo: avg CS/min of this champion over whole games in your elo (±400 LP, otherwise all elos), converted to the time since the first minions (~1:15). Rough – the farm rate changes during the game.\nLane opponent: CS of your direct opponent (same lane)."];

const HELP_TILT: [&str; 2] = ["Erscheint nach 2+ Niederlagen in Folge, wenn das letzte Spiel weniger als 2 Stunden her ist (gleiche Session).\n\
Verglichen wird deine Siegchance in Spielen direkt nach 2+ Niederlagen in derselben Session mit allen anderen Spielen \
(Summoner's Rift, alle Modi). Rot, wenn du danach mehr als 5 Prozentpunkte seltener gewinnst.\n\
Frühe Tode: Tode vor Minute 10 in der aktuellen Serie gegen deinen Schnitt (nur Spiele mit Timeline).\n\
Lange Session: Ohne Niederlagenserie erscheint der Hinweis ab dem 3. Spiel einer Session, aber nur, wenn du ab dem 4. Spiel \
belegbar seltener gewinnst (mindestens 10 Prozentpunkte, je Seite ab 10 Spielen).", "Appears after 2+ losses in a row if the last game was less than 2 hours ago (same session).\nCompares your win rate in games right after 2+ losses in the same session with all other games (Summoner's Rift, all modes). Red if you win more than 5 percentage points less often afterwards.\nEarly deaths: deaths before minute 10 in the current streak vs your average (only games with timeline).\nLong session: without a losing streak the hint appears from the 3rd game of a session, but only if you demonstrably win less often from the 4th game (at least 10 percentage points, from 10 games per side)."];



const HELP_KPI: [&str; 2] = ["Spiele: Solo/Duo-Spiele mit messbarer LP-Änderung.\n\
mit Lobby: davon mit erfassten Rängen der Mitspieler.\n\
Ø Sieg / Ø Niederlage: durchschnittlich gewonnene bzw. verlorene LP.", "Games: Solo/Duo games with measurable LP change.\nwith lobby: those with captured ranks of the other players.\nAvg win / Avg loss: average LP won or lost."];


const HELP_SCATTER: [&str; 2] = ["Jeder Punkt ist ein Spiel. Rechts = deine Lobby war stärker als dein Rang, links = schwächer.\n\
Grün oben = Siege (+LP), rot unten = Niederlagen (-LP).\n\
Steigen die Punkte nach rechts an, bekommst du in stärkeren Lobbys mehr LP. Liegen sie flach, hängt deine LP kaum von der Lobby ab.", "Each dot is a game. Right = your lobby was stronger than your rank, left = weaker.\nGreen on top = wins (+LP), red below = losses (-LP).\nIf the dots rise to the right, you get more LP in stronger lobbies. If they are flat, your LP hardly depends on the lobby."];

const HELP_DELTAS: [&str; 2] = ["Jeder Balken ist ein Spiel: grün = Sieg, rot = Niederlage, grau = 0 LP trotz Niederlage (Abstiegs- oder Verlustschutz).\n\
Werden die grünen Balken größer und die roten kleiner, bekommst du mehr LP pro Sieg als du pro Niederlage verlierst.", "Each bar is a game: green = win, red = loss, gray = 0 LP despite a loss (demotion or loss protection).\nIf the green bars get bigger and the red ones smaller, you get more LP per win than you lose per loss."];



const HELP_STREAKS: [&str; 2] = ["Winrate und LP abhängig davon, wie deine letzten Spiele liefen.\n\
Fällt die Winrate nach Niederlagenserien deutlich ab, kann Tilt eine Rolle spielen: dann lohnt eine Pause.\n\
Die LP-Spalten zeigen, ob Riot nach Serien andere LP vergibt.", "Win rate and LP depending on how your last games went.\nIf the win rate drops clearly after losing streaks, tilt may play a role: then a break is worth it.\nThe LP columns show whether Riot gives different LP after streaks."];

const HELP_TEAMS: [&str; 2] = ["Deine Winrate, je nachdem ob dein Team (ohne dich) im Schnitt höher, gleich oder niedriger gerankt war als der Gegner.\n\
Große Unterschiede zeigen, wie stark das Matchmaking deine Ergebnisse beeinflusst.", "Your win rate depending on whether your team (without you) was ranked higher, equal or lower than the enemy on average.\nBig differences show how strongly matchmaking influences your results."];

const HELP_COMPS: [&str; 2] = ["Über alle gesammelten Matches: Winrate von Teams nach Schadensprofil, CC und Haltbarkeit im Vergleich zum Gegner.\n\
Werte deutlich über 50 % zeigen, welche Zusammensetzungen in deiner Elo funktionieren.\n\
Jedes Match zählt für beide Teams, deshalb spiegeln sich 'mehr' und 'weniger' (z. B. 55 % und 45 %).", "Across all collected matches: win rate of teams by damage profile, CC and durability compared to the enemy.\nValues clearly above 50 % show which compositions work in your elo.\nEach match counts for both teams, so 'more' and 'less' mirror each other (e.g. 55 % and 45 %)."];

const HELP_LOG: [&str; 2] = ["Jede Zeile ist ein Solo/Duo-Spiel, neueste oben.\n\
Lobby-/Team-/Gegner-Ø: durchschnittlicher Rang der anderen Spieler.\n\
Serie: Siege (+) bzw. Niederlagen (-) direkt vor dem Spiel.\n\
N* = Niederlage ohne LP-Verlust (Abstiegs- oder Verlustschutz).", "Each row is a Solo/Duo game, newest on top.\nLobby/team/enemy avg: average rank of the other players.\nStreak: wins (+) or losses (-) right before the game.\nN* = loss without LP loss (demotion or loss protection)."];

/// Einfacher Linienverlauf mit Min/Mitte/Max-Linien und Tooltip pro Punkt.
fn line_chart(ui: &mut egui::Ui, vals: &[f64], col: Color32, fmt: &dyn Fn(f64) -> String, tips: &[String]) {
    if vals.len() < 2 {
        ui.label(RichText::new(tr!("Braucht mindestens 2 Spiele.", "Needs at least 2 games.")).weak());
        return;
    }
    let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let pad = ((hi - lo) * 0.1).max(0.5);
    let c = Chart::new(ui, 120.0, (0.0, (vals.len() - 1) as f32), ((lo - pad) as f32, (hi + pad) as f32));
    for y in [lo, (lo + hi) / 2.0, hi] {
        c.hline(y as f32, fmt(y), false);
    }
    let pts: Vec<Pos2> = vals.iter().enumerate().map(|(i, &v)| c.pos(i as f32, v as f32)).collect();
    c.painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0_f32, col)));
    let tt: Vec<(Pos2, String)> = pts
        .into_iter()
        .zip(vals)
        .zip(tips)
        .map(|((p, v), t)| (p, format!("{} · {t}", fmt(*v))))
        .collect();
    c.tooltip(&tt);
}

/// Tode aller Spiele mit Timeline, gezählt in 5-Minuten-Abschnitten.
fn death_histogram(ui: &mut egui::Ui, gs: &[GameStat]) {
    const BUCKETS: usize = 8;
    let mut counts = [0u32; BUCKETS];
    for t in gs.iter().filter_map(|g| g.timeline.as_ref()) {
        for &d in &t.deaths {
            counts[((d / 300) as usize).min(BUCKETS - 1)] += 1;
        }
    }
    let max = counts.iter().copied().max().unwrap_or(0).max(1);
    let c = Chart::new(ui, 130.0, (0.0, BUCKETS as f32), (0.0, max as f32));
    c.hline(max as f32, max.to_string(), false);
    let w = (c.rect.width() - 54.0) / BUCKETS as f32 * 0.7;
    let mut tips = Vec::new();
    for (i, &n) in counts.iter().enumerate() {
        let x = i as f32 + 0.5;
        let (top, bottom) = (c.pos(x, n as f32), c.pos(x, 0.0));
        let col = if i * 5 < 14 { BLUE } else if i * 5 < 25 { GOLD } else { LOSS };
        c.painter.rect_filled(Rect::from_min_max(Pos2::new(top.x - w / 2.0, top.y), Pos2::new(top.x + w / 2.0, bottom.y)), 1.0, col);
        let label = if i == BUCKETS - 1 { format!("{}+", i * 5) } else { format!("{}–{}", i * 5, i * 5 + 5) };
        c.painter.text(Pos2::new(bottom.x, c.rect.bottom() - 3.0), Align2::CENTER_BOTTOM, &label, FontId::proportional(10.0), Color32::from_gray(150));
        tips.push((top, trf!("Minute {label}: {n} Tode", "Minute {label}: {n} deaths")));
    }
    c.tooltip(&tips);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_check.elapsed() >= Duration::from_secs(2) {
            self.last_check = Instant::now();
            self.refresh();
        }
        // Ersatzschrift erst laden, wenn ein angezeigter Name sie braucht (spart bis zu ~21 MB)
        if self.fonts_need != self.fonts_loaded {
            theme::set_extra_fonts(ctx, self.fonts_need);
            self.fonts_loaded = self.fonts_need;
        }
        if self.shot.is_some() {
            self.shot_step(ctx);
        } else {
            self.track_window(ctx);
        }
        self.icons.poll(ctx);
        // Nur alle 2 s neu zeichnen (Dateien prüfen), sonst nur bei Eingaben.
        ctx.request_repaint_after(Duration::from_secs(2));
        let mut reload = false;
        let compact = ctx.screen_rect().width() < 960.0;
        if self.tab == Tab::Lab && !self.cfg.expert {
            self.tab = Tab::Me;
        }
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(if compact { 58.0 } else { 190.0 })
            .frame(egui::Frame { fill: theme::SIDEBAR, inner_margin: egui::Margin::symmetric(10.0, 16.0), ..Default::default() })
            .show(ctx, |ui| {
                if compact {
                    ui.vertical_centered(|ui| ui.label(RichText::new("ls").font(FontId::new(20.0, theme::bold())).color(GOLD)));
                } else {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new("lolstats").font(FontId::new(20.0, theme::bold())).color(Color32::WHITE));
                    });
                }
                ui.add_space(18.0);
                for (tab, icon, label) in [
                    (Tab::Live, "📡", "Live"),
                    (Tab::Me, "👤", tr!("Ich", "Me")),
                    (Tab::Builds, "🛠", "Builds"),
                    (Tab::Lab, "🔬", tr!("Labor", "Lab")),
                    (Tab::Perf, "📈", tr!("Leistung", "Performance")),
                    (Tab::Help, "📖", "Tutorial"),
                    (Tab::Settings, "⚙", tr!("Einstellungen", "Settings")),
                ] {
                    if tab == Tab::Lab && !self.cfg.expert {
                        continue;
                    }
                    if theme::nav_item(ui, self.tab == tab, icon, label, compact).clicked() {
                        self.tab = tab;
                    }
                    ui.add_space(2.0);
                }
                let update_ready = self.update.available();
                let mut open_settings = false;
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if update_ready {
                        let txt = if compact { "⬆" } else { tr!("⬆ Update verfügbar", "⬆ Update available") };
                        if ui.add(egui::Button::new(RichText::new(txt).color(GOLD)).frame(false)).on_hover_text(tr!("Neue lolstats-Version – in den Einstellungen installieren", "New lolstats version – install it in the settings")).clicked() {
                            open_settings = true;
                        }
                        ui.add_space(6.0);
                    }
                    let (dot, phase) = match self.state.phase.as_str() {
                        "" | "Client aus" => (theme::MUTED, tr!("Client aus", "Client off")),
                        "None" => (WIN, tr!("Client bereit", "Client ready")),
                        "Matchmaking" | "ReadyCheck" => (theme::ACCENT, tr!("Suche läuft", "In queue")),
                        "ChampSelect" => (GOLD, "Champ Select"),
                        "GameStart" | "InProgress" => (GOLD, tr!("Im Spiel", "In game")),
                        "Reconnect" => (LOSS, "Reconnect"),
                        "WaitingForStats" | "PreEndOfGame" | "EndOfGame" => (theme::ACCENT, tr!("Spielende", "Game end")),
                        p => (WIN, p),
                    };
                    if compact {
                        theme::status_dot(ui, dot, phase);
                    } else {
                        theme::status_pill(ui, dot, phase);
                    }
                });
                if open_settings {
                    self.tab = Tab::Settings;
                }
            });
        let central = egui::Frame { fill: theme::BG, inner_margin: if compact { egui::Margin::symmetric(14.0, 14.0) } else { egui::Margin::symmetric(22.0, 18.0) }, ..Default::default() };
        egui::CentralPanel::default().frame(central).show(ctx, |ui| match self.tab {
            Tab::Live => self.ui_live(ui),
            Tab::Me => self.ui_me(ui),
            Tab::Builds => self.ui_builds(ui),
            Tab::Lab => self.ui_labor(ui),
            Tab::Perf => self.ui_perf(ui),
            Tab::Help => ui_help(ui, self.scroll()),
            Tab::Settings => self.ui_settings(ui),
        });
        // Aktualisieren oben rechts auf jeder Seite (auch per F5), z. B. nach einem Match
        egui::Area::new(egui::Id::new("refresh"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_TOP, egui::vec2(-16.0, 14.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let done = self.reloaded.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                    if done {
                        ui.label(RichText::new(tr!("✓ aktualisiert", "✓ refreshed")).small().color(WIN));
                    }
                    let btn = egui::Button::new(RichText::new(tr!("⟳  Aktualisieren", "⟳  Refresh")).font(FontId::new(13.0, theme::bold())))
                        .fill(Color32::from_rgba_unmultiplied(56, 189, 178, 40))
                        .stroke(Stroke::new(1.0_f32, theme::ACCENT));
                    let resp = ui.add(btn).on_hover_text(
                        tr!("Lädt alle Daten neu und lässt lolstats sofort beim Client nachfragen: neue eigene Matches, LP-Stand, Namen (F5)", "Reloads all data and makes lolstats query the client immediately: new own matches, LP, names (F5)"),
                    );
                    reload |= resp.clicked() || ctx.input(|i| i.key_pressed(egui::Key::F5));
                });
            });
        if reload {
            self.force_reload();
            // "✓ aktualisiert" nach 4 s wieder ausblenden; neue Daten vom Dienst kommen über die normale 2-s-Prüfung
            ctx.request_repaint_after(Duration::from_secs(4));
        }
    }
}



