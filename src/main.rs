// Pas de console en release sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clipboard;
mod clipboard_manager;
mod copy;
mod dates;
mod dragdrop;
mod droptarget;
mod favorites;
mod fileops;
mod fs_nav;
mod launch;
mod media;
mod places;
mod player;
mod preview;
mod scan;
mod selection_manager;
mod settings;
mod storage_manager;
mod trash;
mod view;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use settings::Settings;

/// Fenêtre de Windows (HWND) créée par eframe.
fn window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

fn main() -> eframe::Result {
    places::suppress_drive_error_dialogs();

    let settings_file = settings::default_file();
    let settings = settings_file
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("ExPLIA")
        .with_min_inner_size([900.0, 450.0])
        // Une taille mémorisée sur un autre écran ne doit pas dépasser l'écran actuel.
        .with_clamp_size_to_monitor_size(true)
        // ExPLIA enregistre sa propre cible de dépôt (`droptarget`) : elle sait sur quel
        // dossier les fichiers sont lâchés, et copier ou déplacer comme l'Explorateur.
        .with_drag_and_drop(false);
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
            let mut app = app::ExpliaApp::new(cc.egui_ctx.clone(), settings, settings_file);
            app.attach_window(window_handle(cc));
            Ok(Box::new(app))
        }),
    )
}
