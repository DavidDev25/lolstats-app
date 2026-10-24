// Release-Build ohne Konsolenfenster; Debug-Build behält die Konsole für Ausgaben.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod analysis;
mod daemon;
mod gui;
mod icons;
mod lab;
mod lcu;
mod model;
mod queue;
mod stats;
mod store;
mod theme;
mod tray;
mod update;

use model::*;
use store::{Store, Table};

fn single_instance() -> bool {
    first_instance("Local\\lolstats-daemon")
}

/// true, wenn noch kein Prozess den benannten Mutex hält (Handle bleibt bis Prozessende offen).
fn first_instance(name: &str) -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = tray::wide(name);
    let h = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    h != 0 && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS
}

/// Fenster läuft schon: nach vorn holen statt ein zweites zu öffnen.
fn focus_existing_gui() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let title = tray::wide(WINDOW_TITLE);
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if h != 0 {
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }
}

fn status(store: &Store) {
    let me: Option<Me> = store.read(ME);
    let snaps = store.snaps();
    println!("Version: {} ({EDITION})", env!("CARGO_PKG_VERSION"));
    println!("Daten:   {}", store.dir.display());
    println!("Account: {}", me.as_ref().map(|m| m.name.as_str()).unwrap_or("-"));
    println!("Rang:    {}", snaps.last().map(|s| s.rank.label()).unwrap_or_else(|| "-".into()));
    if let Some(me) = store.read::<Me>(ME) {
        let gs = analysis::my_games(&store.matches(), &me.puuid, &store.timelines());
        let elo = store.snaps().last().and_then(|s| s.rank.value());
        let c = analysis::coaching(&gs, &analysis::elo_gpm(&store.matches(), &me.puuid, elo));
        let wr = |x: (usize, usize)| format!("{}/{}", x.1, x.0);
        println!("Coaching: {} eigene SR-Spiele", gs.len());
        let names: Names = store.read(NAMES).unwrap_or_default();
        let list = |v: &[(i32, f64, usize)]| v.iter().map(|(k, a, n)| format!("{} {a:.1} ({n})", names.champ(*k))).collect::<Vec<_>>().join(", ");
        println!(
            "  Fruehe Tode: Ø {} | Champions: {} | Gegner: {}",
            c.early_avg.map(|v| format!("{v:.1}")).unwrap_or_else(|| "-".into()),
            list(&c.early_by_champ),
            list(&c.early_by_opp)
        );
        let sw = analysis::session_wr(&gs);
        println!("  Session-Spielnummer 1./2./3./4.+: {} | {} | {} | {}", wr(sw[0]), wr(sw[1]), wr(sw[2]), wr(sw[3]));
        for l in &c.lines {
            println!("  {:<32} mit {:>7} | ohne {:>7} | {}", l.label, wr(l.with), wr(l.without), l.gap().map(|g| format!("{g:+.0} Pkt.")).unwrap_or_else(|| "-".into()));
        }
        println!(
            "  CS-Vorsprung {:+.1} @10 -> {:+.1} @15 ({} Spiele, schrumpft in {}) | Midgame-Tode wenig/viel Vision {:.1}/{:.1} ({} Spiele)",
            c.lead10,
            c.lead15,
            c.lead_games,
            c.lead_shrinks.map(|s| format!("{:.0} %", s * 100.0)).unwrap_or_else(|| "-".into()),
            c.mid_deaths_low_vision,
            c.mid_deaths_high_vision,
            c.vision_games
        );
    }
    println!(
        "Matches: {} ({} Lobbys, {} LP-Stände, {} Timelines)",
        store.count(Table::Matches),
        store.count(Table::Lobbies),
        store.count(Table::Lp),
        store.count(Table::Timelines)
    );
}

/// Vom Fenster gestartet: andere lolstats-Prozesse beenden (damit die exe frei wird), Update einspielen,
/// Dienst und Fenster neu starten – bei einem Fehler die bisherige Version.
fn apply_update(store: &Store) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut info: update::UpdateInfo = store.read(update::UPDATE).unwrap_or_default();
    let me = std::process::id();
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "lolstats.exe", "/FI", &format!("PID ne {me}")])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(800));
    match update::apply(&info) {
        Ok(()) => store.log(&format!("Update auf {} installiert", info.latest)),
        Err(e) => {
            store.log(&format!("Update fehlgeschlagen: {e}"));
            info.error = e;
            store.write(update::UPDATE, &info);
        }
    }
    // Windows meldet den ursprünglichen Startpfad – dort liegt jetzt die neue (bei Fehler die alte) Datei
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(&exe).spawn();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let _ = std::process::Command::new(&exe).arg("gui").spawn();
    }
}

fn main() {
    let store = Store::open();
    match std::env::args().nth(1).as_deref() {
        Some("gui") => {
            if first_instance("Local\\lolstats-gui") {
                gui::run()
            } else {
                focus_existing_gui()
            }
        }
        Some("status") => status(&store),
        Some("shots") => gui::run_shots(std::env::args().nth(2).map(Into::into).unwrap_or_else(|| store.dir.join("screenshots")), std::env::args().nth(3)),
        Some("sync") => println!("{}", daemon::sync_once(&store)),
        Some("apply-update") => apply_update(&store),
        _ => {
            if !single_instance() {
                return;
            }
            update::cleanup();
            store.log(&format!("lolstats {} gestartet", env!("CARGO_PKG_VERSION")));
            std::thread::spawn(move || daemon::run(store));
            tray::run_loop();
        }
    }
}
