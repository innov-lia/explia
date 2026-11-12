// Pas de console en release sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dates;
mod favorites;
mod fileops;
mod fs_nav;
mod launch;
mod media;
mod places;
mod preview;
mod settings;
mod trash;
mod view;

use settings::Settings;

fn main() -> eframe::Result {
    places::suppress_drive_error_dialogs();

    let settings_file = settings::default_file();
    let settings = settings_file
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("ExPLIA")
        .with_min_inner_size([640.0, 400.0])
        // Une taille mémorisée sur un autre écran ne doit pas dépasser l'écran actuel.
        .with_clamp_size_to_monitor_size(true);
    viewport = match settings.window_size {
        Some((width, height)) => viewport.with_inner_size([width, height]),
        // Première ouverture : la fenêtre est portée à 80 % de l'écran au premier affichage.
        None => viewport.with_inner_size([1100.0, 700.0]),
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "ExPLIA",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::ExpliaApp::new(
                cc.egui_ctx.clone(),
                settings,
                settings_file,
            )))
        }),
    )
}
