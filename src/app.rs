//! Interface graphique (egui / eframe).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, PopupCloseBehavior};

use crate::favorites::{self, Favorites};
use crate::fs_nav::{self, Entry, Location};
use crate::places::{self, Drive, DriveKind, Place};
use crate::preview::{self, ImagePreview};
use crate::view::{self, COMMON_EXTENSIONS, ExtensionFilter, SortKey, SortOrder};

/// Part de l'écran occupée par la fenêtre au démarrage.
const WINDOW_SCREEN_RATIO: f32 = 0.8;

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
    AddFavorite(PathBuf),
    RemoveFavorite(PathBuf),
}

pub struct ExpliaApp {
    location: Location,
    address: String,
    error: Option<String>,
    drives: Vec<Drive>,
    places: Vec<Place>,
    favorites: Favorites,
    /// Contenu brut du dossier courant.
    entries: Vec<Entry>,
    /// Contenu affiché : filtré par extension puis trié.
    visible: Vec<Entry>,
    sort_key: SortKey,
    sort_order: SortOrder,
    filter: ExtensionFilter,
    show_preview: bool,
    window_sized: bool,
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
            favorites: Favorites::load(favorites::default_file()),
            entries: Vec::new(),
            visible: Vec::new(),
            sort_key: SortKey::Name,
            sort_order: SortOrder::Ascending,
            filter: ExtensionFilter::default(),
            show_preview: true,
            window_sized: false,
            selected: None,
            preview: PreviewState::None,
            preview_tx,
            preview_rx,
        }
    }

    /// Fenêtre à 80 % de l'écran, centrée ; effectué une seule fois, dès que la taille
    /// de l'écran est connue.
    fn size_window_once(&mut self, ctx: &egui::Context) {
        if self.window_sized {
            return;
        }
        let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
            return;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            monitor * WINDOW_SCREEN_RATIO,
        ));
        if let Some(center) = egui::ViewportCommand::center_on_screen(ctx) {
            ctx.send_viewport_cmd(center);
        }
        self.window_sized = true;
    }

    fn recompute_visible(&mut self) {
        let mut visible: Vec<Entry> = self
            .entries
            .iter()
            .filter(|entry| self.filter.matches(entry))
            .cloned()
            .collect();
        view::sort_entries(&mut visible, self.sort_key, self.sort_order);
        self.visible = visible;
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
        self.recompute_visible();
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

    fn save_favorites(&mut self) {
        if let Err(err) = self.favorites.save() {
            self.error = Some(format!("Impossible d'enregistrer les favoris : {err}"));
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Go(location) => self.navigate(location),
            Action::Select(entry) => self.select(ctx, entry),
            Action::AddFavorite(path) => {
                if self.favorites.add(path) {
                    self.save_favorites();
                }
            }
            Action::RemoveFavorite(path) => {
                self.favorites.remove(&path);
                self.save_favorites();
            }
        }
    }

    /// Première ligne : dossier parent, Ce PC, barre d'adresse.
    fn address_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.address)
                    .desired_width(f32::INFINITY)
                    .hint_text("Saisir un chemin Windows, par exemple C:\\Users"),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.submit_address();
            }
        });
        action
    }

    /// Seconde ligne : Trier | Afficher | Extension.
    fn options_bar(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal(|ui| {
            menu(ui, "Trier", |ui| {
                for (key, label) in [
                    (SortKey::Name, "Nom"),
                    (SortKey::Size, "Taille"),
                    (SortKey::Type, "Type"),
                ] {
                    changed |= ui.radio_value(&mut self.sort_key, key, label).changed();
                }
                ui.separator();
                for (order, label) in [
                    (SortOrder::Ascending, "Croissant"),
                    (SortOrder::Descending, "Décroissant"),
                ] {
                    changed |= ui.radio_value(&mut self.sort_order, order, label).changed();
                }
            });
            ui.separator();
            menu(ui, "Afficher", |ui| {
                ui.checkbox(&mut self.show_preview, "Volet d'aperçu");
            });
            ui.separator();
            let title = if self.filter.is_all() {
                "Extension".to_string()
            } else {
                format!("Extension ({})", self.filter.count())
            };
            menu(ui, &title, |ui| {
                let mut all = self.filter.is_all();
                if ui.checkbox(&mut all, "Tout").clicked() {
                    self.filter.clear();
                    changed = true;
                }
                ui.separator();
                for (index, (label, _)) in COMMON_EXTENSIONS.iter().enumerate() {
                    let mut on = self.filter.is_selected(index);
                    if ui.checkbox(&mut on, *label).changed() {
                        self.filter.set(index, on);
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.recompute_visible();
        }
    }

    fn sidebar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.add_space(4.0);
        ui.heading("Favoris");
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
        for path in self.favorites.paths() {
            let here = self.location == Location::Dir(path.clone());
            let response = ui
                .selectable_label(here, format!("⭐ {}", favorite_name(path)))
                .on_hover_text(path.display().to_string());
            if response.clicked() {
                action = Some(Action::Go(Location::Dir(path.clone())));
            }
            response.context_menu(|ui| {
                if ui.button("Retirer des favoris").clicked() {
                    action = Some(Action::RemoveFavorite(path.clone()));
                    ui.close();
                }
            });
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
        if self.visible.is_empty() {
            ui.label("Aucun élément ne correspond au filtre d'extension.");
            return None;
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, self.visible.len(), |ui, range| {
                for entry in &self.visible[range] {
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
                    if entry.is_dir {
                        response.context_menu(|ui| {
                            if self.favorites.contains(&entry.path) {
                                if ui.button("Retirer des favoris").clicked() {
                                    action = Some(Action::RemoveFavorite(entry.path.clone()));
                                    ui.close();
                                }
                            } else if ui.button("Ajouter aux favoris").clicked() {
                                action = Some(Action::AddFavorite(entry.path.clone()));
                                ui.close();
                            }
                        });
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

    fn status_text(&self) -> String {
        match &self.location {
            Location::ThisPc => format!("{} lecteur(s)", self.drives.len()),
            Location::Dir(_) if self.visible.len() == self.entries.len() => {
                format!("{} élément(s)", self.entries.len())
            }
            Location::Dir(_) => format!(
                "{} élément(s) affiché(s) sur {}",
                self.visible.len(),
                self.entries.len()
            ),
        }
    }
}

/// Bouton de menu qui reste ouvert tant qu'on ne clique pas à l'extérieur.
fn menu(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    MenuButton::new(title)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, add_contents);
}

fn favorite_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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
        self.size_window_once(&ctx);
        self.poll_preview(&ctx);
        let mut action = None;

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            action = self.address_bar(ui).or(action.take());
            ui.add_space(2.0);
            self.options_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            } else {
                ui.label(self.status_text());
            }
        });
        egui::Panel::left("sidebar")
            .default_size(200.0)
            .resizable(true)
            .show(ui, |ui| {
                action = self.sidebar(ui).or(action.take());
            });
        if self.show_preview {
            egui::Panel::right("preview")
                .default_size(300.0)
                .resizable(true)
                .show(ui, |ui| self.preview_panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.central(ui).or(action.take());
        });

        if let Some(action) = action {
            self.apply(&ctx, action);
        }
    }
}
