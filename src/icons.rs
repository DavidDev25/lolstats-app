//! Item- und Champion-Bilder: einmal von Data Dragon laden, unter icons\ speichern, nur bei Bedarf.
//! Ein einziger Hintergrund-Thread lädt, das Fenster blockiert nie.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

const DDRAGON: &str = "https://ddragon.leagueoflegends.com";

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Item,
    Champ,
}

type Key = (Kind, i32);

pub struct Icons {
    /// RefCell: Bilder werden beim Zeichnen nachgefordert, auch aus Funktionen mit nur lesendem Zugriff
    tex: RefCell<HashMap<Key, Option<TextureHandle>>>,
    req: Sender<(Key, Option<String>)>,
    /// Champion-ID -> Data-Dragon-Name (z. B. 62 -> "MonkeyKing")
    pub aliases: HashMap<i32, String>,
    done: Receiver<(Key, Option<ColorImage>)>,
}

impl Icons {
    pub fn new(ctx: &egui::Context, data_dir: PathBuf) -> Icons {
        let (req, rx) = channel();
        let (tx, done) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || worker(rx, tx, ctx, data_dir.join("icons")));
        Icons { tex: RefCell::new(HashMap::new()), req, aliases: HashMap::new(), done }
    }

    /// Fertig geladene Bilder als Texturen übernehmen (einmal pro Frame).
    pub fn poll(&self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.done.try_recv() {
            let t = img.map(|i| {
                let name = format!("icon{}-{}", key.0 as u8, key.1);
                ctx.load_texture(name, i, TextureOptions::LINEAR)
            });
            self.tex.borrow_mut().insert(key, t);
        }
    }

    pub fn get(&self, kind: Kind, id: i32) -> Option<TextureHandle> {
        let key = (kind, id);
        if id <= 0 {
            return None;
        }
        let mut tex = self.tex.borrow_mut();
        if !tex.contains_key(&key) {
            let alias = self.aliases.get(&id).cloned();
            if kind == Kind::Champ && alias.is_none() {
                return None;
            }
            // None = angefragt bzw. nicht verfügbar
            tex.insert(key, None);
            let _ = self.req.send((key, alias));
        }
        tex.get(&key).and_then(|t| t.clone())
    }
}

/// Bild in fester Größe, bis es geladen ist ein leeres Feld gleicher Größe (kein Springen im Layout).
pub fn show(ui: &mut egui::Ui, icons: &Icons, kind: Kind, id: i32, size: f32) -> egui::Response {
    let sz = egui::vec2(size, size);
    match icons.get(kind, id) {
        Some(t) => ui.add(egui::Image::new((t.id(), sz)).rounding(size * 0.15)),
        None => {
            let (r, resp) = ui.allocate_exact_size(sz, egui::Sense::hover());
            ui.painter().rect_filled(r, size * 0.15, crate::theme::INSET);
            resp
        }
    }
}

fn worker(rx: Receiver<(Key, Option<String>)>, tx: Sender<(Key, Option<ColorImage>)>, ctx: egui::Context, dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let agent = native_tls::TlsConnector::new().ok().map(|tls| {
        ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(10)).build()
    });
    let mut version: Option<String> = None;
    while let Ok((key, alias)) = rx.recv() {
        let file = dir.join(match key.0 {
            Kind::Item => format!("item_{}.png", key.1),
            Kind::Champ => format!("champ_{}.png", key.1),
        });
        let mut bytes = std::fs::read(&file).ok();
        if bytes.is_none() {
            if let Some(a) = &agent {
                if version.is_none() {
                    version = a
                        .get(&format!("{DDRAGON}/api/versions.json"))
                        .call()
                        .ok()
                        .and_then(|r| r.into_json::<Vec<String>>().ok())
                        .and_then(|v| v.into_iter().next());
                }
                let path = match key.0 {
                    Kind::Item => Some(format!("item/{}.png", key.1)),
                    Kind::Champ => alias.map(|k| format!("champion/{k}.png")),
                };
                if let (Some(v), Some(p)) = (&version, path) {
                    bytes = download(a, &format!("{DDRAGON}/cdn/{v}/img/{p}"));
                    if let Some(b) = &bytes {
                        let _ = std::fs::write(&file, b);
                    }
                }
            }
        }
        let img = bytes.and_then(|b| decode(&b));
        if tx.send((key, img)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn download(a: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    let r = a.get(url).call().ok()?;
    let mut buf = Vec::new();
    r.into_reader().take(1 << 20).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn decode(b: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory_with_format(b, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}
