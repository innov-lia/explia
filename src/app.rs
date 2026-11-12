//! Interface graphique (egui / eframe).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

use crate::fs_nav::{self, Entry, Location};
use crate::places::{self, Drive, DriveKind, Place};
use crate::preview::{self, ImagePreview};

type PreviewResult = (PathBuf, Result<ImagePreview, String>);

enum PreviewState {
    None,
    Loading(PathBuf),
    Ready {
        path: PathBuf,
        texture: egui::TextureHandle,
        original: (u32, u32),
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

/// Action demandée par l'interface pendant une frame, appliquée ensuite.
enum Action {
    Go(Location),
    Select(Entry),
}

pub struct ExpliaApp {
    location: Location,
    address: String,
    error: Option<String>,
    drives: Vec<Drive>,
    places: Vec<Place>,
    entries: Vec<Entry>,
    selected: Option<Entry>,
    preview: PreviewState,
    preview_tx: Sender<PreviewResult>,
    preview_rx: Receiver<PreviewResult>,
}

impl ExpliaApp {
    pub fn new() -> Self {
        let (preview_tx, preview_rx) = mpsc::channel();
        let location = Location::ThisPc;
        Self {
            address: location.display(),
            location,
            error: None,
            drives: places::detect_drives(),
            places: places::quick_places(),
            entries: Vec::new(),
            selected: None,
            preview: PreviewState::None,
            preview_tx,
            preview_rx,
        }
    }

    /// Change d'emplacement. En cas d'erreur, on reste sur l'emplacement actuel.
    fn navigate(&mut self, target: Location) {
        match &target {
            Location::ThisPc => {
                self.drives = places::detect_drives();
                self.entries.clear();
            }
            Location::Dir(path) => match fs_nav::list_dir(path) {
                Ok(entries) => self.entries = entries,
                Err(err) => {
                    self.error = Some(format!(
                        "Impossible d'ouvrir « {} » : {err}",
                        path.display()
                    ));
                    self.address = self.location.display();
                    return;
                }
            },
        }
        self.error = None;
        self.address = target.display();
        self.location = target;
        self.selected = None;
        self.preview = PreviewState::None;
    }

    fn submit_address(&mut self) {
        let Some(target) = fs_nav::parse_address(&self.address) else {
            self.address = self.location.display();
            return;
        };
        if let Location::Dir(path) = &target
            && !path.is_dir()
        {
            self.error = Some(format!(
                "« {} » n'est pas un dossier accessible.",
                path.display()
            ));
            return;
        }
        self.navigate(target);
    }

    fn select(&mut self, ctx: &egui::Context, entry: Entry) {
        self.preview = PreviewState::None;
        if !entry.is_dir && preview::is_supported_image(&entry.path) {
            // Le décodage se fait hors du thread d'interface.
            let path = entry.path.clone();
            self.preview = PreviewState::Loading(path.clone());
            let tx = self.preview_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = preview::load_image_preview(&path);
                let _ = tx.send((path, result));
                ctx.request_repaint();
            });
        }
        self.selected = Some(entry);
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        while let Ok((path, result)) = self.preview_rx.try_recv() {
            // Ignore les résultats d'une sélection déjà abandonnée.
            if !matches!(&self.preview, PreviewState::Loading(p) if *p == path) {
                continue;
            }
            self.preview = match result {
                Ok(img) => {
                    let color = egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba);
                    let texture = ctx.load_texture(
                        path.to_string_lossy(),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    PreviewState::Ready {
                        path,
                        texture,
                        original: img.original,
                    }
                }
                Err(message) => PreviewState::Failed { path, message },
            };
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Go(location) => self.navigate(location),
            Action::Select(entry) => self.select(ctx, entry),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            let parent = self.location.parent();
            if ui
                .add_enabled(parent.is_some(), egui::Button::new("⬆ Dossier parent"))
                .clicked()
            {
                action = parent.map(Action::Go);
            }
            if ui.button("🖥 Ce PC").clicked() {
                action = Some(Action::Go(Location::ThisPc));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let go = ui.button("Aller").clicked();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.address)
                        .desired_width(f32::INFINITY)
                        .hint_text("Saisir un chemin Windows, par exemple C:\\Users"),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if go || enter {
                    self.submit_address();
                }
            });
        });
        action
    }

    fn sidebar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.add_space(4.0);
        ui.heading("Accès rapide");
        if ui
            .selectable_label(self.location == Location::ThisPc, "🖥 Ce PC")
            .clicked()
        {
            action = Some(Action::Go(Location::ThisPc));
        }
        for place in &self.places {
            let here = self.location == Location::Dir(place.path.clone());
            if ui
                .selectable_label(here, format!("📁 {}", place.name))
                .clicked()
            {
                action = Some(Action::Go(Location::Dir(place.path.clone())));
            }
        }
        ui.separator();
        ui.heading("Lecteurs");
        for drive in &self.drives {
            let here = self.location == Location::Dir(drive.path.clone());
            if ui
                .selectable_label(
                    here,
                    format!("{} {}", drive_icon(drive.kind), drive.label()),
                )
                .clicked()
            {
                action = Some(Action::Go(Location::Dir(drive.path.clone())));
            }
        }
        action
    }

    fn central(&self, ui: &mut egui::Ui) -> Option<Action> {
        match &self.location {
            Location::ThisPc => self.drives_view(ui),
            Location::Dir(_) => self.listing_view(ui),
        }
    }

    fn drives_view(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.heading("Périphériques et lecteurs");
        ui.add_space(6.0);
        if self.drives.is_empty() {
            ui.label("Aucun lecteur détecté.");
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for drive in &self.drives {
                    let label = format!(
                        "{}  {}   —   {}",
                        drive_icon(drive.kind),
                        drive.label(),
                        drive.kind_name()
                    );
                    if ui
                        .add_sized([ui.available_width(), 32.0], egui::Button::new(label))
                        .clicked()
                    {
                        action = Some(Action::Go(Location::Dir(drive.path.clone())));
                    }
                }
            });
        action
    }

    fn listing_view(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        if self.entries.is_empty() {
            ui.label("Ce dossier est vide.");
            return None;
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, self.entries.len(), |ui, range| {
                for entry in &self.entries[range] {
                    let is_selected = self.selected.as_ref().is_some_and(|s| s.path == entry.path);
                    let icon = if entry.is_dir {
                        "📁"
                    } else if preview::is_supported_image(&entry.path) {
                        "🖼"
                    } else {
                        "📄"
                    };
                    let response =
                        ui.selectable_label(is_selected, format!("{icon} {}", entry.name));
                    if response.double_clicked() && entry.is_dir {
                        action = Some(Action::Go(Location::Dir(entry.path.clone())));
                    } else if response.clicked() {
                        action = Some(Action::Select(entry.clone()));
                    }
                }
            });
        action
    }

    fn preview_panel(&self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("Aperçu");
        ui.separator();
        let Some(entry) = &self.selected else {
            ui.weak("Sélectionnez un élément.");
            return;
        };
        ui.label(egui::RichText::new(&entry.name).strong());
        if entry.is_dir {
            ui.label("Dossier");
            return;
        }
        ui.label(preview::kind_description(&entry.path));
        ui.label(format!("Taille : {}", fs_nav::format_size(entry.size)));
        if !preview::is_supported_image(&entry.path) {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("📄").size(64.0));
            return;
        }
        ui.add_space(8.0);
        match &self.preview {
            PreviewState::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement de l'aperçu…");
                });
            }
            PreviewState::Ready {
                path,
                texture,
                original,
            } if *path == entry.path => {
                ui.label(format!("Dimensions : {} × {} px", original.0, original.1));
                ui.add_space(4.0);
                let available = ui.available_width();
                let tex_size = texture.size_vec2();
                let scale = (available / tex_size.x).min(1.0);
                ui.image(egui::load::SizedTexture::new(
                    texture.id(),
                    tex_size * scale,
                ));
            }
            PreviewState::Failed { path, message } if *path == entry.path => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Aperçu impossible : {message}"),
                );
            }
            _ => {}
        }
    }
}

fn drive_icon(kind: DriveKind) -> &'static str {
    match kind {
        DriveKind::Fixed => "💾",
        DriveKind::Removable => "🔌",
        DriveKind::Network => "🌐",
        DriveKind::Optical => "💿",
        DriveKind::Other => "💾",
    }
}

impl eframe::App for ExpliaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_preview(&ctx);
        let mut action = None;

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            action = self.toolbar(ui).or(action.take());
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            } else if matches!(self.location, Location::Dir(_)) {
                ui.label(format!("{} élément(s)", self.entries.len()));
            } else {
                ui.label(format!("{} lecteur(s)", self.drives.len()));
            }
        });
        egui::Panel::left("sidebar")
            .default_size(200.0)
            .resizable(true)
            .show(ui, |ui| {
                action = self.sidebar(ui).or(action.take());
            });
        egui::Panel::right("preview")
            .default_size(300.0)
            .resizable(true)
            .show(ui, |ui| self.preview_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.central(ui).or(action.take());
        });

        if let Some(action) = action {
            self.apply(&ctx, action);
        }
    }
}
