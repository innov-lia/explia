// Pas de console en release sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod favorites;
mod fs_nav;
mod places;
mod preview;
mod view;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ExPLIA")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ExPLIA",
        options,
        Box::new(|_cc| Ok(Box::new(app::ExpliaApp::new()))),
    )
}
