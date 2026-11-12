//! Interface graphique (egui / eframe).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, PopupCloseBehavior};

use crate::favorites::{self, Favorites};
use crate::fs_nav::{self, Entry, Location};
use crate::places::{self, Drive, DriveKind};
use crate::preview::{self, PreviewContent, TextPreview};
use crate::view::{self, COMMON_EXTENSIONS, ExtensionFilter, SortKey, SortOrder};

/// Part de l'écran occupée par la fenêtre au démarrage.
const WINDOW_SCREEN_RATIO: f32 = 0.6;
/// Largeur du volet d'aperçu agrandi, en part de la largeur disponible.
const PREVIEW_EXPANDED_RATIO: f32 = 0.65;
/// Largeur de la colonne « Taille ».
const SIZE_COLUMN_WIDTH: f32 = 110.0;
/// Nombre maximal de threads qui calculent les tailles de dossiers.
const MAX_SIZE_WORKERS: usize = 4;

/// Résultat renvoyé à l'interface par un thread de travail.
enum Event {
    Preview(PathBuf, Result<PreviewContent, String>),
    DriveSize(PathBuf, Option<u64>),
    /// Taille d'un dossier ; `generation` identifie le listing concerné.
    FolderSize {
        generation: u64,
        path: PathBuf,
        size: u64,
    },
}

enum PreviewState {
    None,
    Loading(PathBuf),
    Image {
        path: PathBuf,
        texture: egui::TextureHandle,
        original: (u32, u32),
    },
    Text {
        path: PathBuf,
        preview: TextPreview,
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
    ctx: egui::Context,
    location: Location,
    address: String,
    error: Option<String>,
    drives: Vec<Drive>,
    /// Taille totale des lecteurs : absente tant que le calcul est en cours,
    /// `None` si le lecteur n'est pas prêt.
    drive_sizes: HashMap<PathBuf, Option<u64>>,
    favorites: Favorites,
    /// Contenu brut du dossier courant.
    entries: Vec<Entry>,
    /// Contenu affiché : filtré par extension puis trié.
    visible: Vec<Entry>,
    sort_key: SortKey,
    sort_order: SortOrder,
    filter: ExtensionFilter,
    show_preview: bool,
    /// Volet d'aperçu agrandi (flèche « < » / « > »).
    preview_expanded: bool,
    /// Option : ne pas calculer la taille des dossiers.
    skip_folder_sizes: bool,
    /// Identifie le listing courant ; l'incrémenter annule les calculs en cours.
    folder_generation: Arc<AtomicU64>,
    window_sized: bool,
    selected: Option<Entry>,
    preview: PreviewState,
    event_tx: Sender<Event>,
    event_rx: Receiver<Event>,
}

impl ExpliaApp {
    pub fn new(ctx: egui::Context) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let location = Location::ThisPc;
        let mut app = Self {
            ctx,
            address: location.display(),
            location,
            error: None,
            drives: places::detect_drives(),
            drive_sizes: HashMap::new(),
            favorites: Favorites::load(favorites::default_file()),
            entries: Vec::new(),
            visible: Vec::new(),
            sort_key: SortKey::Name,
            sort_order: SortOrder::Ascending,
            filter: ExtensionFilter::default(),
            show_preview: true,
            preview_expanded: false,
            skip_folder_sizes: false,
            folder_generation: Arc::new(AtomicU64::new(0)),
            window_sized: false,
            selected: None,
            preview: PreviewState::None,
            event_tx,
            event_rx,
        };
        app.start_drive_sizes();
        app
    }

    /// Calcule la taille des lecteurs dans un thread (un lecteur réseau peut être lent).
    fn start_drive_sizes(&mut self) {
        self.drive_sizes.clear();
        let roots: Vec<PathBuf> = self.drives.iter().map(|d| d.path.clone()).collect();
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for root in roots {
                let size = places::drive_total_size(&root);
                let _ = tx.send(Event::DriveSize(root, size));
                ctx.request_repaint();
            }
        });
    }

    /// Lance le calcul de la taille des sous-dossiers du dossier affiché.
    /// Tout calcul précédent est annulé.
    fn start_folder_sizes(&mut self) {
        let generation = self.folder_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.skip_folder_sizes {
            return;
        }
        let folders: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|e| e.is_dir && e.size.is_none())
            .map(|e| e.path.clone())
            .collect();
        if folders.is_empty() {
            return;
        }
        let queue = Arc::new(Mutex::new(folders));
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(MAX_SIZE_WORKERS);
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let current = Arc::clone(&self.folder_generation);
            let tx = self.event_tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let cancelled = || current.load(Ordering::Relaxed) != generation;
                loop {
                    let Some(path) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                        return;
                    };
                    let Some(size) = fs_nav::dir_size(&path, &cancelled) else {
                        return;
                    };
                    if tx
                        .send(Event::FolderSize {
                            generation,
                            path,
                            size,
                        })
                        .is_err()
                    {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Fenêtre à 60 % de l'écran, centrée ; effectué une seule fois, dès que la taille
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
                // Annule un éventuel calcul de tailles de dossiers.
                self.folder_generation.fetch_add(1, Ordering::SeqCst);
                self.start_drive_sizes();
            }
            Location::Dir(path) => match fs_nav::list_dir(path) {
                Ok(entries) => {
                    self.entries = entries;
                    self.start_folder_sizes();
                }
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
        if !entry.is_dir && preview::is_previewable(&entry.path) {
            // Le décodage se fait hors du thread d'interface.
            let path = entry.path.clone();
            self.preview = PreviewState::Loading(path.clone());
            let tx = self.event_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = preview::load_preview(&path);
                let _ = tx.send(Event::Preview(path, result));
                ctx.request_repaint();
            });
        }
        self.selected = Some(entry);
    }

    /// Traite les résultats des threads de travail.
    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut sizes_changed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Preview(path, result) => self.on_preview(ctx, path, result),
                Event::DriveSize(path, size) => {
                    self.drive_sizes.insert(path, size);
                }
                Event::FolderSize {
                    generation,
                    path,
                    size,
                } => {
                    // Ignore les résultats d'un listing déjà quitté.
                    if generation != self.folder_generation.load(Ordering::SeqCst) {
                        continue;
                    }
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.path == path) {
                        entry.size = Some(size);
                        sizes_changed = true;
                    }
                }
            }
        }
        if sizes_changed {
            self.recompute_visible();
        }
    }

    fn on_preview(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        result: Result<PreviewContent, String>,
    ) {
        {
            // Ignore les résultats d'une sélection déjà abandonnée.
            if !matches!(&self.preview, PreviewState::Loading(p) if *p == path) {
                return;
            }
            self.preview = match result {
                Ok(PreviewContent::Image(img)) => {
                    let color = egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba);
                    let texture = ctx.load_texture(
                        path.to_string_lossy(),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    PreviewState::Image {
                        path,
                        texture,
                        original: img.original,
                    }
                }
                Ok(PreviewContent::Text(preview)) => PreviewState::Text { path, preview },
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

    /// Seconde ligne : Trier | Afficher | Extension | Option.
    fn options_bar(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut sizes_option_changed = false;
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
            ui.separator();
            menu(ui, "Option", |ui| {
                sizes_option_changed |= ui
                    .checkbox(
                        &mut self.skip_folder_sizes,
                        "Pas de calcul de taille de dossier",
                    )
                    .changed();
            });
        });
        if sizes_option_changed {
            if self.skip_folder_sizes {
                // Annule les calculs en cours ; les tailles déjà connues restent affichées.
                self.folder_generation.fetch_add(1, Ordering::SeqCst);
            } else {
                self.start_folder_sizes();
            }
        }
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
        table_header(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for drive in &self.drives {
                    let name = format!(
                        "{}  {}   —   {}",
                        drive_icon(drive.kind),
                        drive.label(),
                        drive.kind_name()
                    );
                    let size = match self.drive_sizes.get(&drive.path) {
                        None => "…".to_string(),
                        Some(None) => "—".to_string(),
                        Some(Some(bytes)) => fs_nav::format_size(*bytes),
                    };
                    if table_row(ui, false, &name, &size).clicked() {
                        action = Some(Action::Go(Location::Dir(drive.path.clone())));
                    }
                }
            });
        action
    }

    /// Texte de la colonne « Taille » d'une entrée de dossier.
    fn entry_size_text(&self, entry: &Entry) -> String {
        match entry.size {
            Some(bytes) => fs_nav::format_size(bytes),
            None if entry.is_dir && !self.skip_folder_sizes => "Calcul…".to_string(),
            None => String::new(),
        }
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
        table_header(ui);
        let row_height = table_row_height(ui) + ui.spacing().item_spacing.y;
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
                    let response = table_row(
                        ui,
                        is_selected,
                        &format!("{icon} {}", entry.name),
                        &self.entry_size_text(entry),
                    );
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

    /// Volet d'aperçu. Renvoie `true` si l'utilisateur a cliqué sur la flèche
    /// « < » / « > » pour agrandir ou réduire le volet.
    fn preview_panel(&self, ui: &mut egui::Ui) -> bool {
        let mut toggle = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (arrow, hint) = if self.preview_expanded {
                (">", "Réduire le volet d'aperçu")
            } else {
                ("<", "Agrandir le volet d'aperçu")
            };
            toggle = ui.button(arrow).on_hover_text(hint).clicked();
            ui.heading("Aperçu");
        });
        ui.separator();
        let Some(selected) = &self.selected else {
            ui.weak("Sélectionnez un élément.");
            return toggle;
        };
        // La taille d'un dossier peut arriver après la sélection : on relit l'entrée à jour.
        let entry = self
            .entries
            .iter()
            .find(|e| e.path == selected.path)
            .unwrap_or(selected);
        ui.label(egui::RichText::new(&entry.name).strong());
        if entry.is_dir {
            ui.label("Dossier");
            let size = self.entry_size_text(entry);
            if !size.is_empty() {
                ui.label(format!("Taille : {size}"));
            }
            return toggle;
        }
        ui.label(preview::kind_description(&entry.path));
        ui.label(format!("Taille : {}", self.entry_size_text(entry)));
        if !preview::is_previewable(&entry.path) {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("📄").size(64.0));
            ui.weak("Pas d'aperçu pour ce type de fichier.");
            return toggle;
        }
        ui.add_space(8.0);
        match &self.preview {
            PreviewState::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement de l'aperçu…");
                });
            }
            PreviewState::Image {
                path,
                texture,
                original,
            } if *path == entry.path => {
                ui.label(format!("Dimensions : {} × {} px", original.0, original.1));
                ui.add_space(4.0);
                egui::ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let tex_size = texture.size_vec2();
                        let scale = (ui.available_width() / tex_size.x).min(1.0);
                        ui.image(egui::load::SizedTexture::new(
                            texture.id(),
                            tex_size * scale,
                        ));
                    });
            }
            PreviewState::Text { path, preview } if *path == entry.path => {
                if preview.truncated {
                    ui.weak(format!(
                        "Début du fichier ({} Ko affichés)",
                        preview::MAX_TEXT_BYTES / 1024
                    ));
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&preview.content).monospace())
                                .wrap(),
                        );
                    });
            }
            PreviewState::Failed { path, message } if *path == entry.path => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Aperçu impossible : {message}"),
                );
            }
            _ => {}
        }
        toggle
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

fn table_row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y
}

/// En-tête du tableau : « Nom » à gauche, « Taille » alignée à droite.
fn table_header(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::hover(),
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    let pad = ui.spacing().button_padding.x;
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.left() + pad, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Nom",
        font.clone(),
        color,
    );
    painter.text(
        egui::pos2(rect.right() - pad, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "Taille",
        font,
        color,
    );
    ui.separator();
}

/// Ligne du tableau : nom à gauche, taille alignée à droite. Renvoie la réponse au clic.
fn table_row(ui: &mut egui::Ui, selected: bool, name: &str, size: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let corner = visuals.widgets.inactive.corner_radius;
        let (background, text_color) = if selected {
            (
                Some(visuals.selection.bg_fill),
                visuals.selection.stroke.color,
            )
        } else if response.hovered() {
            (
                Some(visuals.widgets.hovered.weak_bg_fill),
                visuals.text_color(),
            )
        } else {
            (None, visuals.text_color())
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let pad = ui.spacing().button_padding.x;
        let painter = ui.painter();
        if let Some(fill) = background {
            painter.rect_filled(rect, corner, fill);
        }
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + pad, rect.top()),
            egui::pos2(rect.right() - SIZE_COLUMN_WIDTH - pad, rect.bottom()),
        );
        painter.with_clip_rect(name_rect).text(
            egui::pos2(name_rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            font.clone(),
            text_color,
        );
        painter.text(
            egui::pos2(rect.right() - pad, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            size,
            font,
            text_color,
        );
    }
    response
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
        self.poll_events(&ctx);
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
            // Deux identifiants distincts : chaque mode garde sa propre largeur.
            let (id, width) = if self.preview_expanded {
                (
                    "preview_wide",
                    ui.available_width() * PREVIEW_EXPANDED_RATIO,
                )
            } else {
                ("preview", 300.0)
            };
            let toggle = egui::Panel::right(id)
                .default_size(width)
                .resizable(true)
                .show(ui, |ui| self.preview_panel(ui))
                .inner;
            if toggle {
                self.preview_expanded = !self.preview_expanded;
            }
        }
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.central(ui).or(action.take());
        });

        if let Some(action) = action {
            self.apply(&ctx, action);
        }
    }
}
