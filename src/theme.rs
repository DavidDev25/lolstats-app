//! Aussehen des Fensters: Farben, Schrift (Segoe UI aus Windows, nichts eingebettet), Karten, Navigation.

use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Margin, Rounding, Sense, Shadow, Stroke, TextStyle,
};

pub const BG: Color32 = Color32::from_rgb(13, 16, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(17, 21, 30);
pub const CARD: Color32 = Color32::from_rgb(22, 27, 38);
pub const INSET: Color32 = Color32::from_rgb(16, 20, 28);
pub const BORDER: Color32 = Color32::from_rgb(36, 43, 58);
pub const TEXT: Color32 = Color32::from_rgb(214, 220, 230);
pub const MUTED: Color32 = Color32::from_rgb(128, 138, 156);
pub const ACCENT: Color32 = Color32::from_rgb(56, 189, 178);
pub const GOLD: Color32 = Color32::from_rgb(214, 170, 82);

const WIDGET: Color32 = Color32::from_rgb(30, 36, 50);
const WIDGET_HOVER: Color32 = Color32::from_rgb(40, 48, 66);

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Ersatzschriften für asiatische Spielernamen. Sie sind groß (MS Gothic 8,6 MB, Malgun Gothic 12,8 MB)
/// und werden deshalb erst geladen, wenn ein Name sie braucht.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtraFonts {
    /// Japanisch/Chinesisch (Kana, CJK-Ideogramme) -> MS Gothic
    pub cjk: bool,
    /// Koreanisch (Hangul) -> Malgun Gothic
    pub hangul: bool,
}

impl ExtraFonts {
    /// Welche Ersatzschriften ein Text braucht.
    pub fn needed_for(s: &str) -> ExtraFonts {
        let mut e = ExtraFonts::default();
        for c in s.chars() {
            match c as u32 {
                0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => e.hangul = true,
                0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xFF66..=0xFF9F => e.cjk = true,
                _ => {}
            }
        }
        e
    }
    pub fn union(self, o: ExtraFonts) -> ExtraFonts {
        ExtraFonts { cjk: self.cjk || o.cjk, hangul: self.hangul || o.hangul }
    }
}

/// Schriften neu setzen, z. B. wenn ein koreanischer Name aufgetaucht ist.
pub fn set_extra_fonts(ctx: &egui::Context, extra: ExtraFonts) {
    ctx.set_fonts(fonts(extra));
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts(ExtraFonts::default()));
    let mut s = (*ctx.style()).clone();
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Heading, FontId::new(21.0, bold())),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(12.0, 5.0);
    s.spacing.interact_size.y = 26.0;
    s.spacing.scroll = egui::style::ScrollStyle::floating();

    let r = Rounding::same(4.0);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    v.window_shadow = Shadow { offset: egui::vec2(0.0, 6.0), blur: 18.0, spread: 0.0, color: Color32::from_black_alpha(110) };
    v.popup_shadow = Shadow { offset: egui::vec2(0.0, 4.0), blur: 12.0, spread: 0.0, color: Color32::from_black_alpha(110) };
    v.extreme_bg_color = INSET;
    v.faint_bg_color = Color32::from_rgb(26, 32, 44);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(56, 189, 178, 60);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.rounding = r;
    for (st, fill) in [(&mut w.inactive, WIDGET), (&mut w.hovered, WIDGET_HOVER), (&mut w.active, WIDGET_HOVER), (&mut w.open, WIDGET)] {
        st.bg_fill = fill;
        st.weak_bg_fill = fill;
        st.rounding = r;
        st.expansion = 0.0;
    }
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(64, 76, 98));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    // strong() nutzt die Farbe von "active"
    w.active.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
    w.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    s.visuals = v;
    ctx.set_style(s);
}

/// Segoe UI (+ Semibold) aus dem Windows-Schriftordner; fehlen sie, bleiben die egui-Schriften.
/// Asiatische Ersatzschriften nur, wenn `extra` sie anfordert.
fn fonts(extra: ExtraFonts) -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    let fallback = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = fallback.clone();
    let mut semibold = fallback;
    if let Ok(b) = std::fs::read(dir.join("segoeui.ttf")) {
        f.font_data.insert("segoe".into(), FontData::from_owned(b));
        regular.insert(0, "segoe".into());
    }
    if let Some(b) = ["seguisb.ttf", "segoeuib.ttf"].iter().find_map(|n| std::fs::read(dir.join(n)).ok()) {
        f.font_data.insert("segoe_sb".into(), FontData::from_owned(b));
        semibold.insert(0, "segoe_sb".into());
    } else {
        semibold = regular.clone();
    }
    // Ersatz für japanische/chinesische Zeichen in Spielernamen (nur genutzt, wo Segoe UI kein Zeichen hat)
    if extra.cjk {
        if let Ok(b) = std::fs::read(dir.join("msgothic.ttc")) {
            f.font_data.insert("cjk".into(), FontData::from_owned(b));
            regular.push("cjk".into());
            semibold.push("cjk".into());
        }
    }
    // Ersatz für koreanische Zeichen (Hangul)
    if extra.hangul {
        if let Ok(b) = std::fs::read(dir.join("malgun.ttf")) {
            f.font_data.insert("hangul".into(), FontData::from_owned(b));
            regular.push("hangul".into());
            semibold.push("hangul".into());
        }
    }
    f.families.insert(FontFamily::Proportional, regular);
    f.families.insert(bold(), semibold);
    f
}

/// Karte für einen Abschnitt.
pub fn card() -> egui::Frame {
    egui::Frame {
        inner_margin: Margin::same(14.0),
        rounding: Rounding::same(10.0),
        fill: CARD,
        stroke: Stroke::new(1.0_f32, BORDER),
        ..Default::default()
    }
}

/// Seitenüberschrift mit Untertitel.
pub fn page_header(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(egui::RichText::new(title).font(FontId::new(24.0, bold())).color(Color32::WHITE));
    if !sub.is_empty() {
        ui.label(egui::RichText::new(sub).color(MUTED));
    }
    ui.add_space(10.0);
}

/// Eintrag der Seitenleiste: Symbol + Text, ausgewählt mit Akzentbalken links.
/// `compact`: nur das Symbol (schmale Fenster), Text als Tooltip.
pub fn nav_item(ui: &mut egui::Ui, selected: bool, icon: &str, label: &str, compact: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered() || selected);
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 7.0, Color32::from_rgba_unmultiplied(56, 189, 178, 28));
        p.rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 8.0), egui::vec2(3.0, rect.height() - 16.0)), 2.0, ACCENT);
    } else if hover > 0.0 {
        p.rect_filled(rect, 7.0, Color32::from_white_alpha((hover * 8.0) as u8));
    }
    let col = if selected { Color32::WHITE } else { lerp(MUTED, TEXT, hover) };
    let icon_col = if selected { ACCENT } else { col };
    if compact {
        p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(17.0), icon_col);
        return resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label);
    }
    p.text(rect.left_center() + egui::vec2(16.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), icon_col);
    let font = if selected { FontId::new(14.0, bold()) } else { FontId::proportional(14.0) };
    p.text(rect.left_center() + egui::vec2(42.0, 0.0), Align2::LEFT_CENTER, label, font, col);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Nur der farbige Punkt (schmale Seitenleiste), Text als Tooltip.
pub fn status_dot(ui: &mut egui::Ui, dot: Color32, text: &str) {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, dot);
    resp.on_hover_text(text);
}

/// Kleine Pille mit farbigem Punkt, z. B. für den Client-Status.
pub fn status_pill(ui: &mut egui::Ui, dot: Color32, text: &str) {
    egui::Frame {
        inner_margin: Margin::symmetric(10.0, 5.0),
        rounding: Rounding::same(20.0),
        fill: WIDGET,
        ..Default::default()
    }
    .show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(egui::RichText::new(text).small().color(TEXT));
        });
    });
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Karte über die volle verfügbare Breite.
pub fn section(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}

/// Kleines "?" im Kreis für Erklärungen beim Drüberfahren.
pub fn help_icon(ui: &mut egui::Ui) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
    let col = if resp.hovered() { ACCENT } else { MUTED };
    ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.2_f32, col));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, "?", FontId::new(10.5, bold()), col);
    resp
}

/// Große Kachel mit Kernaussage: Überschrift, Wert, Untertitel. Feste Größe, bricht als Ganzes um.
pub fn tile(ui: &mut egui::Ui, label: &str, value: &str, value_col: Color32, sub: &str) -> egui::Response {
    let p = ui.painter();
    let top = p.layout_no_wrap(label.to_string(), FontId::proportional(11.5), MUTED);
    let val = p.layout_no_wrap(value.to_string(), FontId::new(20.0, bold()), value_col);
    let low = p.layout_no_wrap(sub.to_string(), FontId::proportional(11.5), MUTED);
    let w = top.size().x.max(val.size().x).max(low.size().x).max(130.0) + 28.0;
    let h = 14.0 + top.size().y + 4.0 + val.size().y + 2.0 + low.size().y + 14.0;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect(r, 10.0, CARD, Stroke::new(1.0_f32, BORDER));
    let mut y = r.top() + 14.0;
    let x = r.left() + 14.0;
    let (th, vh) = (top.size().y, val.size().y);
    p.galley(egui::pos2(x, y), top, MUTED);
    y += th + 4.0;
    p.galley(egui::pos2(x, y), val, value_col);
    y += vh + 2.0;
    p.galley(egui::pos2(x, y), low, MUTED);
    resp
}

/// Kleine Kennzahl-Kachel (z. B. Nach-Match-Vergleich): Überschrift, Wert, Untertitel, kompakt.
pub fn mini_stat(ui: &mut egui::Ui, label: &str, value: &str, value_col: Color32, sub: &str) -> egui::Response {
    let p = ui.painter();
    let top = p.layout_no_wrap(label.to_string(), FontId::proportional(10.5), MUTED);
    let val = p.layout_no_wrap(value.to_string(), FontId::new(15.0, bold()), value_col);
    let low = p.layout_no_wrap(sub.to_string(), FontId::proportional(10.5), MUTED);
    let w = top.size().x.max(val.size().x).max(low.size().x).max(70.0) + 18.0;
    let h = 8.0 + top.size().y + 2.0 + val.size().y + 1.0 + low.size().y + 8.0;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 8.0, INSET);
    let x = r.left() + 9.0;
    let (th, vh) = (top.size().y, val.size().y);
    p.galley(egui::pos2(x, r.top() + 8.0), top, MUTED);
    p.galley(egui::pos2(x, r.top() + 10.0 + th), val, value_col);
    p.galley(egui::pos2(x, r.top() + 11.0 + th + vh), low, MUTED);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_asian_scripts_in_names() {
        assert_eq!(ExtraFonts::needed_for("Spieler#EUW"), ExtraFonts::default());
        assert_eq!(ExtraFonts::needed_for("Sörén Ægir"), ExtraFonts::default());
        assert_eq!(ExtraFonts::needed_for("페이커#KR1"), ExtraFonts { cjk: false, hangul: true });
        assert_eq!(ExtraFonts::needed_for("すし侍"), ExtraFonts { cjk: true, hangul: false });
        let both = ExtraFonts::needed_for("한").union(ExtraFonts::needed_for("中"));
        assert_eq!(both, ExtraFonts { cjk: true, hangul: true });
    }
}
