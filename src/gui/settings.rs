use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once, OnceLock},
};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use serde::{Deserialize, Serialize};

use crate::gui::i18n::{Key, Language, Translator};
use crate::gui::numeric_input::numeric_input;

static SYSTEM_FONTS: OnceLock<Mutex<Option<Arc<Vec<SystemFont>>>>> = OnceLock::new();
pub const MAX_RECENT_SESSIONS: usize = 5;
const MAX_CJK_FALLBACKS: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub layout_version: u32,
    pub theme: Theme,
    /// `None` uses egui's built in font stack.
    pub font: Option<String>,
    /// Global base size in points. All egui text styles are derived from it.
    pub font_size: f32,
    /// Defaults keep settings written by older GUI builds readable.
    #[serde(default = "super::locale::system_language")]
    pub language: Language,
    #[serde(default)]
    pub recent_sessions: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            layout_version: 2,
            theme: Theme::System,
            font: None,
            font_size: 14.5,
            language: super::locale::system_language(),
            recent_sessions: Vec::new(),
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

    pub fn i18n(&self) -> Translator {
        Translator::new(self.language)
    }

    /// Promote a successfully opened or started session to the front of the
    /// recent list, removing duplicates and keeping only a small bounded set.
    pub fn remember_session(&mut self, path: PathBuf) {
        self.recent_sessions.retain(|recent| recent != &path);
        self.recent_sessions.insert(0, path);
        self.recent_sessions.truncate(MAX_RECENT_SESSIONS);
    }

    pub fn forget_session(&mut self, path: &Path) {
        self.recent_sessions.retain(|recent| recent != path);
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
                (TextStyle::Heading, 19.0 / 14.5, FontFamily::Proportional),
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
        let i18n = self.i18n();
        ui.menu_button(i18n.tr(Key::Appearance), |ui| {
            for (theme, key) in [
                (Theme::System, Key::System),
                (Theme::Light, Key::Light),
                (Theme::Dark, Key::Dark),
            ] {
                if ui.selectable_label(self.theme == theme, i18n.tr(key)).clicked() {
                    self.theme = theme;
                    ui.close();
                }
            }
        });
        if ui.button(i18n.tr(Key::Fonts)).clicked() {
            font_window.open = true;
            ui.close();
        }
        ui.menu_button(i18n.tr(Key::Language), |ui| {
            for (language, key) in [
                (Language::English, Key::English),
                (Language::Japanese, Key::Japanese),
                (Language::SimplifiedChinese, Key::SimplifiedChinese),
            ] {
                if ui
                    .selectable_label(self.language == language, i18n.tr(key))
                    .clicked()
                {
                    self.language = language;
                    ui.close();
                }
            }
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
        let i18n = settings.i18n();
        let mut open = self.open;
        egui::Window::new(i18n.tr(Key::Fonts))
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut settings.font_size, 10.0..=28.0)
                            .text(i18n.tr(Key::FontSize))
                            .show_value(false),
                    );
                    numeric_input(
                        ui,
                        "font_size_numeric_input",
                        &mut settings.font_size,
                        10.0..=28.0,
                        i18n.tr(Key::PointSuffix),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(i18n.tr(Key::Font));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(i18n.tr(Key::SearchFonts)),
                    );
                });
                ui.separator();
                let query = self.search.trim().to_lowercase();
                match system_fonts() {
                    None => {
                        ui.label(i18n.tr(Key::FontListLoading));
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
                            .selectable_label(settings.font.is_none(), i18n.tr(Key::DefaultFont))
                            .clicked()
                            && settings.font.take().is_some()
                        {
                            settings.apply_font(ctx);
                            self.font_initialized = true;
                        }
                        if matching.is_empty() {
                            ui.label(i18n.tr(Key::FontSearchNoResults));
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
    families: Vec<String>,
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
                        families: face
                            .families
                            .iter()
                            .map(|(family, _)| family.clone())
                            .collect(),
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
                .unwrap() = Some(shared.clone());
            // Read the small fallback set on the scan thread so the UI thread
            // only ever attaches already-loaded fallback data.
            let _ = cjk_fallbacks(&shared);
        });
    })
}

fn load_font(face: &SystemFont) -> Option<Arc<FontData>> {
    let bytes = std::fs::read(&face.path).ok()?;
    let mut data = FontData::from_owned(bytes);
    data.index = face.index;
    Some(Arc::new(data))
}

/// Preferred CJK families in priority order. The lists are intentionally
/// short: a couple of common UI fonts cover Latin, Simplified Chinese, and
/// Japanese while keeping the fallback stack bounded.
fn preferred_cjk_families() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &[
            "Microsoft YaHei UI",
            "Microsoft YaHei",
            "Yu Gothic UI",
            "Yu Gothic",
            "Meiryo",
            "MS Gothic",
        ]
    }
    #[cfg(not(target_os = "windows"))]
    {
        &[
            "Noto Sans CJK SC",
            "Noto Sans CJK JP",
            "Noto Sans SC",
            "Noto Sans JP",
            "WenQuanYi Micro Hei",
            "WenQuanYi Zen Hei",
            "Droid Sans Fallback",
        ]
    }
}

/// A small bounded CJK fallback set is shared across font changes. Each
/// preferred family is read at most once, and only the first few matches are
/// kept so egui's fallback chain stays predictable.
fn cjk_fallbacks(fonts: &[SystemFont]) -> &'static Vec<(String, Arc<FontData>)> {
    static FALLBACKS: OnceLock<Vec<(String, Arc<FontData>)>> = OnceLock::new();
    FALLBACKS.get_or_init(|| {
        let mut out = Vec::new();
        let mut seen_paths = HashSet::new();
        for preferred in preferred_cjk_families() {
            if out.len() >= MAX_CJK_FALLBACKS {
                break;
            }
            let face = fonts.iter().find(|face| {
                face.families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(preferred))
            });
            let Some(face) = face else {
                continue;
            };
            if !seen_paths.insert(face.path.clone()) {
                continue;
            }
            if let Some(data) = load_font(face) {
                out.push((format!("system-cjk-{}", out.len()), data));
            }
        }
        out
    })
}

fn append_cjk_fallbacks(defs: &mut FontDefinitions, fonts: &[SystemFont]) {
    for (name, data) in cjk_fallbacks(fonts) {
        defs.font_data.insert(name.clone(), data.clone());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let stack = defs.families.entry(family).or_default();
            if !stack.contains(name) {
                stack.push(name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_sessions_are_newest_first_and_deduplicated() {
        let mut settings = Settings::default();
        for path in ["/sessions/one", "/sessions/two", "/sessions/one"] {
            settings.remember_session(PathBuf::from(path));
        }
        assert_eq!(
            settings.recent_sessions,
            vec![
                PathBuf::from("/sessions/one"),
                PathBuf::from("/sessions/two")
            ]
        );
    }

    #[test]
    fn recent_sessions_are_capped_at_five() {
        let mut settings = Settings::default();
        for index in 0..8 {
            settings.remember_session(PathBuf::from(format!("/sessions/{index}")));
        }
        assert_eq!(settings.recent_sessions.len(), MAX_RECENT_SESSIONS);
        assert_eq!(
            settings.recent_sessions.first(),
            Some(&PathBuf::from("/sessions/7"))
        );
        assert_eq!(
            settings.recent_sessions.last(),
            Some(&PathBuf::from("/sessions/3"))
        );
    }

    #[test]
    fn forgetting_a_missing_session_removes_only_that_entry() {
        let mut settings = Settings::default();
        settings.remember_session(PathBuf::from("/sessions/missing"));
        settings.remember_session(PathBuf::from("/sessions/present"));
        settings.forget_session(Path::new("/sessions/missing"));
        assert_eq!(
            settings.recent_sessions,
            vec![PathBuf::from("/sessions/present")]
        );
    }

    #[test]
    fn settings_without_language_or_recent_sessions_still_load() {
        let json = r#"{
            "layout_version": 2,
            "theme": "Dark",
            "font": null,
            "font_size": 16.0
        }"#;
        let settings: Settings = serde_json::from_str(json).expect("old settings should load");
        assert_eq!(settings.language, super::super::locale::system_language());
        assert!(settings.recent_sessions.is_empty());
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.font_size, 16.0);
    }

    #[test]
    fn first_launch_uses_system_language_and_saved_choices_take_precedence() {
        assert_eq!(
            Settings::load(None).language,
            super::super::locale::system_language()
        );
        for language in Language::ALL {
            let settings = Settings {
                language,
                ..Settings::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            let restored: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.language, language);
        }
    }

    #[test]
    fn language_and_recent_sessions_survive_serialization() {
        let mut settings = Settings::default();
        settings.language = Language::Japanese;
        settings.remember_session(PathBuf::from("/sessions/illustration"));
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.language, Language::Japanese);
        assert_eq!(
            restored.recent_sessions,
            vec![PathBuf::from("/sessions/illustration")]
        );
    }
}
