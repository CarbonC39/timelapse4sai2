#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[path = "../gui/mod.rs"]
mod gui;

fn main() -> eframe::Result {
    eframe::run_native(
        gui::APP_NAME,
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title(gui::APP_NAME)
                .with_decorations(true)
                .with_inner_size([560.0, 720.0])
                .with_min_inner_size([440.0, 480.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(gui::app::App::new(cc)))),
    )
}
