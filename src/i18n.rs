//! Sprache der Oberfläche (Deutsch/Englisch). Texte stehen im Code direkt nebeneinander:
//! `tr!("Deutsch", "English")` für feste Texte, `trf!("… {x} …", "… {x} …", args)` statt `format!`.
//! Texte, die als Daten aus anderen Modulen kommen (Kennzahl-Namen, Urteile), übersetzt `tx`.

use std::sync::atomic::AtomicBool;
#[cfg(not(test))]
use std::sync::atomic::Ordering;

static EN: AtomicBool = AtomicBool::new(false);

#[cfg(not(test))]
pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

#[cfg(not(test))]
pub fn set_en(v: bool) {
    EN.store(v, Ordering::Relaxed);
}

// Tests laufen parallel: Sprache pro Thread, damit ein Test die anderen nicht umschaltet
#[cfg(test)]
thread_local!(static EN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
#[cfg(test)]
pub fn en() -> bool {
    let _ = &EN;
    EN_TEST.with(|c| c.get())
}
#[cfg(test)]
pub fn set_en(v: bool) {
    EN_TEST.with(|c| c.set(v));
}

/// Sprache aus der Einstellung: "de", "en" oder leer = wie Windows (Deutsch nur bei deutschem Windows).
pub fn apply(lang: &str) {
    set_en(match lang {
        "de" => false,
        "en" => true,
        _ => !system_is_german(),
    });
}

fn system_is_german() -> bool {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    // primäre Sprach-ID 0x07 = Deutsch
    (unsafe { GetUserDefaultUILanguage() } & 0x3ff) == 0x07
}

/// Fester Text in beiden Sprachen.
#[macro_export]
macro_rules! tr {
    ($de:literal, $en:literal $(,)?) => {
        if $crate::i18n::en() { $en } else { $de }
    };
}

/// Wie `format!`, mit deutscher und englischer Vorlage (gleiche Platzhalter).
#[macro_export]
macro_rules! trf {
    ($de:literal, $en:literal $($rest:tt)*) => {
        if $crate::i18n::en() { format!($en $($rest)*) } else { format!($de $($rest)*) }
    };
}

/// Konstante in beiden Sprachen: `const X: [&str; 2] = ["Deutsch", "English"]`.
pub fn l(x: [&'static str; 2]) -> &'static str {
    x[en() as usize]
}

/// Übersetzt Texte, die als Daten ankommen (z. B. Kennzahl-Namen aus analysis.rs); unbekannte bleiben.
pub fn tx(s: &str) -> &str {
    if !en() {
        return s;
    }
    DYN.iter().find(|(de, _)| *de == s).map_or(s, |(_, en)| en)
}

/// Deutsch -> Englisch für Texte aus anderen Modulen
static DYN: &[(&str, &str)] = &[
    // Urteile (queue.rs; im Fenster werden die deutschen Werte für die Farbe verglichen)
    ("zu wenige Spiele", "not enough games"),
    ("deutlicher Unterschied", "clear difference"),
    ("schwacher Hinweis", "weak hint"),
    ("kein Unterschied (Zufall)", "no difference (chance)"),
    // Spielstil-Kennzahlen (analysis::STYLE_LABELS)
    ("Kill-Beteiligung", "Kill participation"),
    ("Anteil am Team-Schaden", "Share of team damage"),
    ("Anteil am Team-Gold", "Share of team gold"),
    ("CS pro Minute", "CS per minute"),
    ("Vision pro Minute", "Vision per minute"),
    ("Anteil erlittener Schaden", "Share of damage taken"),
    ("Objective-Schaden pro Minute", "Objective damage per minute"),
    ("CC-Sekunden pro Minute", "CC seconds per minute"),
    ("Tode pro 10 Minuten", "Deaths per 10 minutes"),
    // Spielvergleich nach dem Match (analysis::match_summary)
    ("Tode", "Deaths"),
    ("Schaden/min", "Damage/min"),
    ("Sicht/min", "Vision/min"),
    ("CS nach 10 min", "CS at 10 min"),
    // Version
    ("öffentlich", "public"),
];

#[cfg(test)]
mod tests {
    #[test]
    fn switch_language_and_format() {
        super::set_en(false);
        let n = 3;
        assert_eq!(tr!("Sieg", "Victory"), "Sieg");
        assert_eq!(trf!("{n} Spiele", "{n} games"), "3 Spiele");
        assert_eq!(trf!("{} von {}", "{} of {}", 1, 2), "1 von 2");
        super::set_en(true);
        assert_eq!(tr!("Sieg", "Victory"), "Victory");
        assert_eq!(trf!("{n} Spiele", "{n} games"), "3 games");
        assert_eq!(super::l(["a", "b"]), "b");
        super::set_en(false);
    }
}
