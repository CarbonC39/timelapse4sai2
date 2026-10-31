use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Once, OnceLock},
};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use serde::{Deserialize, Serialize};

use crate::gui::i18n::{Key, tr};

static SYSTEM_FONTS: OnceLock<Mutex<Option<Arc<Vec<SystemFont>>>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub theme: Theme,
    /// `None` uses egui's built in font stack.
    pub font: Option<String>,
    /// Global base size in points. All egui text styles are derived from it.
    pub font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font: None,
            font_size: 14.5,
        }
    }
}

impl Settings {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default()
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Applies theme and text sizes. Font data is handled separately so a theme or
    /// size change never reads font files or asks egui to rebuild its atlas.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
        let base = self.font_size.clamp(8.0, 40.0);
        ctx.all_styles_mut(|style| {
            style.visuals.override_text_color =
                Some(egui::Color32::from_gray(if style.visuals.dark_mode {
                    222
                } else {
                    35
                }));
            style.visuals.weak_text_color =
                Some(egui::Color32::from_gray(if style.visuals.dark_mode {
                    170
                } else {
                    95
                }));
            for (text_style, scale, family) in [
                (TextStyle::Small, 0.85, FontFamily::Proportional),
                (TextStyle::Body, 1.0, FontFamily::Proportional),
                (TextStyle::Button, 1.0, FontFamily::Proportional),
                (TextStyle::Heading, 20.0 / 14.5, FontFamily::Proportional),
                (TextStyle::Monospace, 1.0, FontFamily::Monospace),
            ] {
                style
                    .text_styles
                    .insert(text_style, FontId::new(base * scale, family));
            }
        });
    }

    /// Rebuilds the active font definitions. Call at startup and after a font
    /// selection changes; the selected font bytes are not cached globally.
    pub fn apply_font(&self, ctx: &egui::Context) {
        start_font_scan();
        let mut defs = FontDefinitions::default();
        let Some(fonts) = system_fonts() else {
            ctx.set_fonts(defs);
            return;
        };
        append_cjk_fallbacks(&mut defs, &fonts);
        if let Some(selected) = self.font.as_deref() {
            if let Some(face) = fonts.iter().find(|face| face.name == selected) {
                if let Some(data) = load_font(face) {
                    const SELECTED: &str = "settings-selected-font";
                    defs.font_data.insert(SELECTED.into(), data);
                    for family in [FontFamily::Proportional, FontFamily::Monospace] {
                        let stack = defs.families.entry(family).or_default();
                        stack.retain(|name| name != SELECTED);
                        stack.insert(0, SELECTED.into());
                    }
                }
            }
        }
        ctx.set_fonts(defs);
    }

    /// Draw the View menu controls. The font picker opens a separate searchable
    /// window, keeping the menu compact even on systems with many installed fonts.
    pub fn view_menu(&mut self, ui: &mut egui::Ui, font_window: &mut FontWindow) {
        ui.menu_button(tr(Key::Appearance), |ui| {
            for (theme, key) in [
                (Theme::System, Key::System),
                (Theme::Light, Key::Light),
                (Theme::Dark, Key::Dark),
            ] {
                if ui.selectable_label(self.theme == theme, tr(key)).clicked() {
                    self.theme = theme;
                    ui.close();
                }
            }
        });
        if ui.button(tr(Key::Fonts)).clicked() {
            font_window.open = true;
            ui.close();
        }
        ui.menu_button(tr(Key::Language), |ui| {
            let _ = ui.selectable_label(true, tr(Key::English));
        });
    }
}

#[derive(Default)]
pub struct FontWindow {
    pub open: bool,
    search: String,
    font_initialized: bool,
}

impl FontWindow {
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        start_font_scan();
        if !self.font_initialized && system_fonts().is_some() {
            settings.apply_font(ctx);
            self.font_initialized = true;
        }
        if !self.open {
            if system_fonts().is_none() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            return;
        }
        let previous_size = settings.font_size;
        let mut open = self.open;
        egui::Window::new(tr(Key::Fonts))
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut settings.font_size, 10.0..=28.0)
                            .text(tr(Key::FontSize))
                            .show_value(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut settings.font_size)
                            .range(10.0..=28.0)
                            .suffix(tr(Key::PointSuffix)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(tr(Key::Font));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(tr(Key::SearchFonts)),
                    );
                });
                ui.separator();
                let query = self.search.trim().to_lowercase();
                match system_fonts() {
                    None => {
                        ui.label(tr(Key::FontListLoading));
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    Some(fonts) => {
                        let matching = fonts
                            .iter()
                            .filter(|font| {
                                query.is_empty() || font.name.to_lowercase().contains(&query)
                            })
                            .collect::<Vec<_>>();
                        if ui
                            .selectable_label(settings.font.is_none(), tr(Key::DefaultFont))
                            .clicked()
                            && settings.font.take().is_some()
                        {
                            settings.apply_font(ctx);
                            self.font_initialized = true;
                        }
                        if matching.is_empty() {
                            ui.label(tr(Key::FontSearchNoResults));
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("font-list-rows")
                                .max_height(360.0)
                                .show_rows(
                                    ui,
                                    ui.text_style_height(&TextStyle::Body) + 8.0,
                                    matching.len(),
                                    |ui, rows| {
                                        for index in rows {
                                            let font = matching[index];
                                            if ui
                                                .selectable_label(
                                                    settings.font.as_deref()
                                                        == Some(font.name.as_str()),
                                                    &font.name,
                                                )
                                                .clicked()
                                                && settings.font.as_deref()
                                                    != Some(font.name.as_str())
                                            {
                                                settings.font = Some(font.name.clone());
                                                settings.apply_font(ctx);
                                                self.font_initialized = true;
                                            }
                                        }
                                    },
                                );
                        }
                    }
                }
            });
        self.open &= open;
        if settings.font_size != previous_size {
            settings.apply(ctx);
        }
    }
}

#[derive(Clone)]
struct SystemFont {
    name: String,
    path: PathBuf,
    index: u32,
}

/// Metadata only: the OS font database is retained, but font file contents are
/// read only when a selected font is activated.
fn system_fonts() -> Option<Arc<Vec<SystemFont>>> {
    let fonts = SYSTEM_FONTS.get_or_init(|| Mutex::new(None));
    fonts.lock().ok()?.clone()
}

fn start_font_scan() {
    static START: Once = Once::new();
    START.call_once(|| {
        std::thread::spawn(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let mut fonts = db
                .faces()
                .filter_map(|face| {
                    let fontdb::Source::File(path) = &face.source else {
                        return None;
                    };
                    face.families.first().map(|(name, _)| SystemFont {
                        name: name.clone(),
                        path: path.clone(),
                        index: face.index,
                    })
                })
                .collect::<Vec<_>>();
            fonts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            fonts.dedup_by(|a, b| a.name == b.name);
            let shared = Arc::new(fonts);
            *SYSTEM_FONTS
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap() = Some(shared);
        });
    })
}

fn load_font(face: &SystemFont) -> Option<Arc<FontData>> {
    let bytes = std::fs::read(&face.path).ok()?;
    let mut data = FontData::from_owned(bytes);
    data.index = face.index;
    Some(Arc::new(data))
}

/// A small bounded CJK fallback set is shared across font changes. The selected
/// font is never included in this cache, avoiding stale fallback selection.
fn append_cjk_fallbacks(defs: &mut FontDefinitions, fonts: &[SystemFont]) {
    static FALLBACKS: OnceLock<Vec<(String, Arc<FontData>)>> = OnceLock::new();
    let fallbacks = FALLBACKS.get_or_init(|| {
        let mut out = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();
        for face in fonts {
            if out.len() >= 2 {
                break;
            }
            let is_cjk = {
                let name = face.name.to_lowercase();
                [
                    "cjk",
                    "noto sans cjk",
                    "noto serif cjk",
                    "droid sans fallback",
                    "wqy",
                    "wenquanyi",
                    "japanese",
                    "gothic",
                    "ming",
                    "yahei",
                    "simsun",
                    "yu gothic",
                    "malgun",
                    "pingfang",
                ]
                .iter()
                .any(|needle| name.contains(needle))
            };
            if !is_cjk || !seen_paths.insert(face.path.clone()) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&face.path) {
                let mut data = FontData::from_owned(bytes);
                data.index = face.index;
                out.push((format!("system-cjk-{}", out.len()), Arc::new(data)));
            }
        }
        out
    });
    for (name, data) in fallbacks {
        defs.font_data.insert(name.clone(), data.clone());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let stack = defs.families.entry(family).or_default();
            if !stack.contains(name) {
                stack.push(name.clone());
            }
        }
    }
}
