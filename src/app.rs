//! Interface graphique (egui / eframe).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, PopupCloseBehavior};

use crate::favorites::Favorites;
use crate::fileops;
use crate::fs_nav::{self, Entry, Location};
use crate::launch;
use crate::places::{self, Drive, DriveKind};
use crate::preview::{self, ArchivePreview, PreviewContent, TextPreview};
use crate::settings::Settings;
use crate::trash;
use crate::view::{self, Category, ExtensionFilter, SortKey, SortOrder};

/// Part de l'écran occupée par la fenêtre à la première ouverture.
pub const WINDOW_SCREEN_RATIO: f32 = 0.8;
/// Largeur du volet d'aperçu agrandi, en part de la largeur disponible.
const PREVIEW_EXPANDED_RATIO: f32 = 0.65;
/// Message affiché quand on tente de mélanger dossiers et fichiers.
const MIXED_TYPES_MESSAGE: &str =
    "Sélection d'un seul type à la fois : dossiers ou fichiers, pas les deux.";
/// Durée du clic maintenu qui démarre la multi-sélection, en secondes.
const LONG_PRESS_SECONDS: f64 = 0.5;
/// Un second clic sur l'élément sélectionné démarre le renommage s'il survient entre ces
/// deux délais (en secondes) : plus tôt, c'est un double-clic ; plus tard, un nouveau clic.
const RENAME_CLICK_MIN_SECONDS: f64 = 0.45;
const RENAME_CLICK_MAX_SECONDS: f64 = 3.0;
/// Déplacement maximal du pointeur, en points, pendant un clic maintenu.
const LONG_PRESS_TOLERANCE: f32 = 12.0;
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
    Archive {
        path: PathBuf,
        preview: ArchivePreview,
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
    /// Ouvre un fichier avec son application associée (ou lance un exécutable).
    Open(PathBuf),
    /// Lance un exécutable en tant qu'administrateur (fenêtre UAC).
    OpenAsAdmin(PathBuf),
    /// Passe la barre d'adresse en mode saisie.
    EditAddress,
    /// Clic maintenu sur une ligne : démarre (ou complète) la multi-sélection.
    LongPress(Entry),
    /// Clic sur une ligne pendant la multi-sélection : l'ajoute ou la retire.
    ToggleMulti(Entry),
    /// Maj+clic : sélectionne tout entre le dernier élément choisi et celui-ci.
    SelectRange(Entry),
    /// Quitte la multi-sélection.
    ClearMulti,
    /// Demande confirmation avant de supprimer la sélection.
    AskDelete,
    CancelDelete,
    /// Envoie la sélection à la corbeille.
    ConfirmDelete,
    /// Passe une ligne en mode renommage (F2, clic lent, bouton « Renommer »).
    StartRename(Entry),
    CommitRename {
        path: PathBuf,
        new_name: String,
    },
    CancelRename,
}

/// Renommage en cours, directement dans la liste.
struct RenameState {
    path: PathBuf,
    text: String,
    /// Longueur (en caractères) du texte sélectionné au départ : le nom sans l'extension.
    initial_selection: usize,
    /// Nombre d'images déjà affichées : le focus est demandé à la première,
    /// la sélection du texte appliquée à la suivante.
    frames: u32,
}

pub struct ExpliaApp {
    ctx: egui::Context,
    /// Fichier de réglages ; `None` si le dossier de configuration est introuvable.
    settings_file: Option<PathBuf>,
    /// Dernier état écrit dans le fichier (hors taille de fenêtre, voir `save_settings_if_changed`).
    saved_settings: Settings,
    /// Taille intérieure actuelle de la fenêtre, en points.
    window_size: Option<(f32, f32)>,
    location: Location,
    address: String,
    /// Barre d'adresse en mode saisie (sinon : fil d'Ariane cliquable).
    address_editing: bool,
    focus_address: bool,
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
    /// Multi-sélection : un seul type à la fois (tous dossiers, ou tous fichiers).
    multi: Vec<Entry>,
    /// Dernier élément choisi : point de départ d'une sélection Maj+clic.
    anchor: Option<PathBuf>,
    /// Attente de confirmation de la suppression de la multi-sélection.
    confirm_delete: bool,
    /// Multi-sélection démarrée par un clic maintenu : un simple clic ajoute ou retire
    /// une ligne. Sinon (Ctrl/Maj), un simple clic remplace la sélection, comme l'Explorateur.
    multi_toggle_mode: bool,
    /// Un clic maintenu vient d'être traité : le relâchement ne doit pas compter comme un clic.
    long_press_done: bool,
    /// Message d'information (non erroné) affiché dans la barre d'état.
    notice: Option<String>,
    /// Renommage en cours (une seule ligne à la fois).
    renaming: Option<RenameState>,
    /// Dernier clic sur une ligne (chemin, instant) : un second clic lent sur la ligne
    /// déjà sélectionnée démarre le renommage, comme dans l'Explorateur.
    last_row_click: Option<(PathBuf, f64)>,
    preview: PreviewState,
    event_tx: Sender<Event>,
    event_rx: Receiver<Event>,
}

impl ExpliaApp {
    pub fn new(ctx: egui::Context, settings: Settings, settings_file: Option<PathBuf>) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let location = Location::ThisPc;
        let mut app = Self {
            ctx,
            settings_file,
            window_size: settings.window_size,
            address_editing: false,
            focus_address: false,
            address: location.display(),
            location,
            error: None,
            drives: places::detect_drives(),
            drive_sizes: HashMap::new(),
            favorites: Favorites::from_paths(settings.favorites.clone()),
            entries: Vec::new(),
            visible: Vec::new(),
            sort_key: settings.sort_key,
            sort_order: settings.sort_order,
            filter: ExtensionFilter::new(
                settings.categories.clone(),
                settings.custom_extensions.clone(),
            ),
            show_preview: settings.show_preview,
            preview_expanded: settings.preview_expanded,
            skip_folder_sizes: settings.skip_folder_sizes,
            folder_generation: Arc::new(AtomicU64::new(0)),
            // Une taille mémorisée est appliquée dès la création de la fenêtre (voir `main`).
            window_sized: settings.window_size.is_some(),
            saved_settings: settings,
            selected: None,
            multi: Vec::new(),
            anchor: None,
            confirm_delete: false,
            multi_toggle_mode: false,
            long_press_done: false,
            notice: None,
            renaming: None,
            last_row_click: None,
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

    /// Première ouverture (aucune taille mémorisée) : fenêtre à 80 % de l'écran, centrée.
    /// Effectué une seule fois, dès que la taille de l'écran est connue.
    fn size_window_once(&mut self, ctx: &egui::Context) {
        if self.window_sized {
            return;
        }
        let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
            return;
        };
        let size = monitor * WINDOW_SCREEN_RATIO;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        // Centrage calculé avec la nouvelle taille (`center_on_screen` utiliserait l'ancienne),
        // légèrement remonté pour laisser la place à la barre de titre et à la barre des tâches.
        let margin = (monitor - size) / 2.0;
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            margin.x,
            (margin.y - 24.0).max(0.0),
        )));
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
        self.notice = None;
        self.address = target.display();
        self.location = target;
        self.selected = None;
        self.clear_multi();
        self.anchor = None;
        self.renaming = None;
        self.last_row_click = None;
        self.preview = PreviewState::None;
        self.recompute_visible();
    }

    /// Maj+clic : sélectionne la plage entre l'ancre (dernier élément choisi) et `entry`.
    fn select_range(&mut self, entry: Entry) {
        let anchor = self
            .anchor
            .clone()
            .or_else(|| self.selected.as_ref().map(|s| s.path.clone()));
        let Some(anchor) = anchor else {
            // Rien de choisi avant : Maj+clic démarre simplement la sélection.
            self.toggle_multi(entry);
            return;
        };
        match view::select_range(&self.visible, &anchor, &entry.path) {
            Ok(range) => {
                self.confirm_delete = false;
                self.selected = None;
                self.preview = PreviewState::None;
                self.error = None;
                self.anchor = Some(anchor);
                self.multi = range;
            }
            Err(view::RangeError::MixedTypes) => {
                self.error = Some(MIXED_TYPES_MESSAGE.to_string());
            }
            Err(view::RangeError::NotFound) => self.toggle_multi(entry),
        }
    }

    /// Relit le dossier affiché (après une suppression, par exemple).
    fn refresh_listing(&mut self) {
        let Location::Dir(path) = &self.location else {
            return;
        };
        match fs_nav::list_dir(path) {
            Ok(entries) => {
                self.entries = entries;
                self.start_folder_sizes();
                self.recompute_visible();
            }
            Err(err) => {
                self.error = Some(format!("Impossible de relire le dossier : {err}"));
            }
        }
    }

    /// Ajoute ou retire une ligne de la multi-sélection. Un seul type à la fois :
    /// on ne mélange pas dossiers et fichiers.
    fn toggle_multi(&mut self, entry: Entry) {
        self.confirm_delete = false;
        // Comme dans l'Explorateur : le premier Ctrl+clic garde l'élément déjà sélectionné.
        if self.multi.is_empty()
            && let Some(current) = self.selected.clone()
        {
            if current.path == entry.path {
                // Ctrl+clic sur l'unique élément sélectionné : on le désélectionne.
                self.selected = None;
                self.preview = PreviewState::None;
                return;
            }
            if current.is_dir != entry.is_dir {
                self.error = Some(MIXED_TYPES_MESSAGE.to_string());
                return;
            }
            self.multi.push(current);
        }
        self.selected = None;
        self.preview = PreviewState::None;
        if let Some(position) = self.multi.iter().position(|e| e.path == entry.path) {
            self.multi.remove(position);
            if self.multi.is_empty() {
                self.multi_toggle_mode = false;
            }
            return;
        }
        if let Some(first) = self.multi.first()
            && first.is_dir != entry.is_dir
        {
            self.error = Some(MIXED_TYPES_MESSAGE.to_string());
            return;
        }
        self.error = None;
        self.anchor = Some(entry.path.clone());
        self.multi.push(entry);
    }

    /// Quitte la multi-sélection.
    fn clear_multi(&mut self) {
        self.multi.clear();
        self.multi_toggle_mode = false;
        self.confirm_delete = false;
    }

    /// Éléments visés par « Supprimer » : la multi-sélection, sinon l'élément sélectionné.
    fn delete_targets(&self) -> Vec<Entry> {
        if self.multi.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.multi.clone()
        }
    }

    fn delete_selection(&mut self) {
        let paths: Vec<PathBuf> = self.delete_targets().into_iter().map(|e| e.path).collect();
        let count = paths.len();
        self.confirm_delete = false;
        match trash::move_to_recycle_bin(&paths) {
            Ok(()) => {
                self.error = None;
                self.notice = Some(format!("{count} élément(s) envoyé(s) à la corbeille."));
            }
            Err(message) => {
                self.error = Some(message);
                self.notice = None;
            }
        }
        // Dans tous les cas, on relit le dossier : certains éléments ont pu être supprimés.
        self.clear_multi();
        self.selected = None;
        self.preview = PreviewState::None;
        self.refresh_listing();
    }

    /// Démarre le renommage de `entry` directement dans la liste.
    fn start_rename(&mut self, entry: &Entry) {
        if !matches!(self.location, Location::Dir(_)) {
            return;
        }
        self.error = None;
        self.confirm_delete = false;
        self.renaming = Some(RenameState {
            path: entry.path.clone(),
            text: entry.name.clone(),
            initial_selection: fileops::stem_len(&entry.name, entry.is_dir),
            frames: 0,
        });
    }

    /// Applique le renommage ; en cas d'erreur, la saisie reste ouverte pour corriger.
    fn commit_rename(&mut self, ctx: &egui::Context, path: PathBuf, new_name: &str) {
        match fileops::rename_in_place(&path, new_name) {
            Ok(new_path) => {
                self.renaming = None;
                self.error = None;
                if self.favorites.contains(&path) {
                    self.favorites.remove(&path);
                    self.favorites.add(new_path.clone());
                }
                self.refresh_listing();
                if let Some(entry) = self.entries.iter().find(|e| e.path == new_path).cloned() {
                    self.select(ctx, entry);
                }
                self.notice = Some(format!("Renommé en « {new_name} »."));
            }
            Err(message) => self.error = Some(message),
        }
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
        // Un clic simple remplace toute sélection multiple.
        self.clear_multi();
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
        self.anchor = Some(entry.path.clone());
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
                Ok(PreviewContent::Archive(preview)) => PreviewState::Archive { path, preview },
                Err(message) => PreviewState::Failed { path, message },
            };
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Go(location) => self.navigate(location),
            Action::Select(entry) => self.select(ctx, entry),
            Action::AddFavorite(path) => {
                self.favorites.add(path);
            }
            Action::RemoveFavorite(path) => self.favorites.remove(&path),
            Action::Open(path) => self.report_launch(launch::open(&path)),
            Action::OpenAsAdmin(path) => self.report_launch(launch::open_as_admin(&path)),
            Action::EditAddress => {
                self.address_editing = true;
                self.focus_address = true;
            }
            Action::LongPress(entry) => {
                self.long_press_done = true;
                // Un clic maintenu sur l'élément sélectionné démarre la multi-sélection avec lui.
                if self.multi.is_empty()
                    && self.selected.as_ref().is_some_and(|s| s.path == entry.path)
                {
                    self.selected = None;
                    self.preview = PreviewState::None;
                }
                self.toggle_multi(entry);
                // Après un clic maintenu, chaque clic ajoute ou retire une ligne.
                self.multi_toggle_mode = !self.multi.is_empty();
            }
            Action::ToggleMulti(entry) => self.toggle_multi(entry),
            Action::SelectRange(entry) => self.select_range(entry),
            Action::ClearMulti => self.clear_multi(),
            Action::AskDelete => self.confirm_delete = !self.delete_targets().is_empty(),
            Action::CancelDelete => self.confirm_delete = false,
            Action::ConfirmDelete => self.delete_selection(),
            Action::StartRename(entry) => self.start_rename(&entry),
            Action::CommitRename { path, new_name } => self.commit_rename(ctx, path, &new_name),
            Action::CancelRename => self.renaming = None,
        }
    }

    fn report_launch(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }

    /// Réglages courants, tels qu'ils seront écrits dans le fichier.
    fn current_settings(&self) -> Settings {
        Settings {
            window_size: self.window_size,
            sort_key: self.sort_key,
            sort_order: self.sort_order,
            show_preview: self.show_preview,
            preview_expanded: self.preview_expanded,
            skip_folder_sizes: self.skip_folder_sizes,
            categories: self.filter.categories().to_vec(),
            custom_extensions: self.filter.custom().to_string(),
            favorites: self.favorites.paths().to_vec(),
        }
    }

    fn write_settings(&mut self, settings: Settings) {
        let Some(file) = &self.settings_file else {
            return;
        };
        if let Err(err) = settings.save(file) {
            self.error = Some(format!("Impossible d'enregistrer les réglages : {err}"));
        }
        self.saved_settings = settings;
    }

    /// Enregistre les réglages dès qu'un d'eux change. La taille de la fenêtre est
    /// ignorée ici (elle varie en continu pendant un redimensionnement) : elle est
    /// écrite à la fermeture, et avec les autres réglages quand ceux-ci changent.
    fn save_settings_if_changed(&mut self) {
        let current = self.current_settings();
        let mut without_window = current.clone();
        without_window.window_size = self.saved_settings.window_size;
        if without_window != self.saved_settings {
            self.write_settings(current);
        }
    }

    /// Suit la taille de la fenêtre (sauf si elle est maximisée ou en plein écran).
    fn track_window_size(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let viewport = input.viewport();
            let resizable_state =
                viewport.maximized != Some(true) && viewport.fullscreen != Some(true);
            if let (Some(rect), true) = (viewport.inner_rect, resizable_state) {
                self.window_size = Some((rect.width(), rect.height()));
            }
        });
    }

    /// Première ligne : dossier parent, Ce PC, fil d'Ariane (ou saisie du chemin).
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
            if self.address_editing {
                self.address_edit(ui);
            } else {
                let crumb_action = self.breadcrumb_bar(ui);
                action = crumb_action.or(action.take());
            }
        });
        action
    }

    /// Mode saisie : Entrée valide, Échap ou clic ailleurs annule.
    fn address_edit(&mut self, ui: &mut egui::Ui) {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.address)
                .desired_width(f32::INFINITY)
                .hint_text("Saisir un chemin Windows, par exemple C:\\Users"),
        );
        if self.focus_address {
            edit.request_focus();
            self.focus_address = false;
            return;
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if edit.lost_focus() {
            if enter {
                self.submit_address();
            } else if escape || !edit.has_focus() {
                self.address = self.location.display();
            }
            self.address_editing = false;
        }
    }

    /// Fil d'Ariane : un bouton par niveau du chemin ; un clic sur l'espace libre
    /// (ou sur le crayon) passe en mode saisie.
    fn breadcrumb_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let crumbs = match &self.location {
            Location::ThisPc => Vec::new(),
            Location::Dir(path) => fs_nav::breadcrumbs(path),
        };
        let height = ui.spacing().interact_size.y;
        let frame = egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(4, 0));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(height);
            ui.horizontal(|ui| {
                if ui
                    .small_button("✏")
                    .on_hover_text("Modifier le chemin")
                    .clicked()
                {
                    action = Some(Action::EditAddress);
                }
                egui::ScrollArea::horizontal()
                    .id_salt("breadcrumb")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .stick_to_right(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let this_pc = ui.add(egui::Button::new("Ce PC").frame(false));
                            if this_pc.clicked() {
                                action = Some(Action::Go(Location::ThisPc));
                            }
                            for (label, path) in &crumbs {
                                ui.weak(">");
                                let button = ui.add(egui::Button::new(label).frame(false));
                                if button.clicked() {
                                    action = Some(Action::Go(Location::Dir(path.clone())));
                                }
                            }
                            // Espace libre : un clic passe en mode saisie.
                            let rest = ui.allocate_response(
                                egui::vec2(ui.available_width().max(24.0), height),
                                egui::Sense::click(),
                            );
                            if rest.clicked() {
                                action = Some(Action::EditAddress);
                            }
                        });
                    });
            });
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
                ui.horizontal(|ui| {
                    ui.label("Custom");
                    let edit = ui.add(
                        egui::TextEdit::singleline(self.filter.custom_mut())
                            .desired_width(110.0)
                            .hint_text("ex. mkv, pdf"),
                    );
                    changed |= edit.changed();
                });
                ui.separator();
                for category in Category::ALL {
                    let mut on = self.filter.has_category(category);
                    if ui.checkbox(&mut on, category.label()).changed() {
                        self.filter.set_category(category, on);
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

    fn central(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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

    fn listing_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
        // État modifié pendant le dessin de la liste : sorti de `self` le temps de la frame.
        let mut rename = self.renaming.take();
        let mut last_click = self.last_row_click.take();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            // Un clic maintenu ne doit pas faire défiler la liste.
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .show_rows(ui, row_height, self.visible.len(), |ui, range| {
                for entry in &self.visible[range] {
                    // Ligne en cours de renommage : champ de saisie à la place du libellé.
                    if let Some(state) = rename.as_mut()
                        && state.path == entry.path
                    {
                        if let Some(rename_action) = rename_row(ui, state) {
                            action = Some(rename_action);
                        }
                        continue;
                    }
                    let is_selected = self.selected.as_ref().is_some_and(|s| s.path == entry.path)
                        || self.multi.iter().any(|m| m.path == entry.path);
                    let icon = if entry.is_dir {
                        "📁"
                    } else if preview::is_supported_image(&entry.path) {
                        "🖼"
                    } else if preview::is_video(&entry.path) {
                        "🎬"
                    } else if preview::is_zip(&entry.path) {
                        "🗜"
                    } else {
                        "📄"
                    };
                    let response = table_row(
                        ui,
                        is_selected,
                        &format!("{icon} {}", entry.name),
                        &self.entry_size_text(entry),
                    );
                    // Après un clic maintenu : chaque clic ajoute ou retire une ligne.
                    // Avec Ctrl/Maj : un clic simple remplace la sélection (comportement habituel).
                    let multi_mode = self.multi_toggle_mode && !self.multi.is_empty();
                    // Clic maintenu : démarre la multi-sélection (ou ajoute la ligne).
                    // La détection se fait sur la position du pointeur, avec une tolérance
                    // pour les mouvements de la main, sans dépendre de l'état du widget.
                    if !self.long_press_done {
                        let held = ui.input(|i| {
                            if !i.pointer.primary_down() {
                                return None;
                            }
                            let origin = i.pointer.press_origin()?;
                            let position = i.pointer.latest_pos()?;
                            let started = i.pointer.press_start_time()?;
                            let on_this_row = response.rect.contains(origin)
                                && origin.distance(position) <= LONG_PRESS_TOLERANCE;
                            on_this_row.then_some(i.time - started)
                        });
                        match held {
                            Some(seconds) if seconds >= LONG_PRESS_SECONDS => {
                                action = Some(Action::LongPress(entry.clone()));
                            }
                            Some(_) => {
                                // Réveille l'interface pour détecter la fin du délai.
                                ui.ctx()
                                    .request_repaint_after(std::time::Duration::from_millis(100));
                            }
                            None => {}
                        }
                    }
                    // Appui long sur écran tactile.
                    if response.long_touched() {
                        action = Some(Action::LongPress(entry.clone()));
                    }
                    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                    if response.clicked() && ctrl {
                        // Ctrl+clic : ajoute ou retire la ligne de la sélection multiple.
                        action = Some(Action::ToggleMulti(entry.clone()));
                    } else if response.clicked() && shift {
                        // Maj+clic : sélectionne toute la plage depuis le dernier élément choisi.
                        action = Some(Action::SelectRange(entry.clone()));
                    } else if self.long_press_done {
                        // Le relâchement qui suit un clic maintenu n'est pas un clic.
                    } else if response.double_clicked() && !multi_mode {
                        // Dossier : on entre ; fichier : on l'ouvre avec son application.
                        action = Some(if entry.is_dir {
                            Action::Go(Location::Dir(entry.path.clone()))
                        } else {
                            Action::Open(entry.path.clone())
                        });
                    } else if response.clicked() {
                        let now = ui.input(|i| i.time);
                        // Second clic « lent » sur l'élément déjà sélectionné : renommage.
                        let slow_second_click = !multi_mode
                            && self.multi.is_empty()
                            && self.selected.as_ref().is_some_and(|s| s.path == entry.path)
                            && last_click.as_ref().is_some_and(|(path, time)| {
                                *path == entry.path
                                    && (RENAME_CLICK_MIN_SECONDS..RENAME_CLICK_MAX_SECONDS)
                                        .contains(&(now - time))
                            });
                        if slow_second_click {
                            last_click = None;
                            action = Some(Action::StartRename(entry.clone()));
                        } else {
                            last_click = Some((entry.path.clone(), now));
                            action = Some(if multi_mode {
                                Action::ToggleMulti(entry.clone())
                            } else {
                                Action::Select(entry.clone())
                            });
                        }
                    }
                    response.context_menu(|ui| {
                        if ui.button("Renommer  (F2)").clicked() {
                            action = Some(Action::StartRename(entry.clone()));
                            ui.close();
                        }
                        ui.separator();
                        if entry.is_dir {
                            if self.favorites.contains(&entry.path) {
                                if ui.button("Retirer des favoris").clicked() {
                                    action = Some(Action::RemoveFavorite(entry.path.clone()));
                                    ui.close();
                                }
                            } else if ui.button("Ajouter aux favoris").clicked() {
                                action = Some(Action::AddFavorite(entry.path.clone()));
                                ui.close();
                            }
                        } else {
                            let label = if launch::is_executable(&entry.path) {
                                "Exécuter"
                            } else {
                                "Ouvrir"
                            };
                            if ui.button(label).clicked() {
                                action = Some(Action::Open(entry.path.clone()));
                                ui.close();
                            }
                            if launch::is_executable(&entry.path)
                                && ui.button("Exécuter en tant qu'administrateur").clicked()
                            {
                                action = Some(Action::OpenAsAdmin(entry.path.clone()));
                                ui.close();
                            }
                        }
                    });
                }
            });
        self.renaming = rename;
        self.last_row_click = last_click;
        action
    }

    /// Boutons « Renommer » et « Supprimer ». « Supprimer » demande d'abord confirmation,
    /// puis envoie à la corbeille. `rename` : l'élément à renommer (un seul à la fois).
    fn edit_controls(
        &self,
        ui: &mut egui::Ui,
        what: &str,
        rename: Option<&Entry>,
    ) -> Option<Action> {
        let mut action = None;
        if self.confirm_delete {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Envoyer {what} à la corbeille ?"),
            );
            ui.horizontal(|ui| {
                let confirm = egui::Button::new(
                    egui::RichText::new("Confirmer la suppression").color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(180, 40, 40));
                if ui.add(confirm).clicked() {
                    action = Some(Action::ConfirmDelete);
                }
                if ui.button("Annuler").clicked() {
                    action = Some(Action::CancelDelete);
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if let Some(entry) = rename
                    && ui
                        .button("✏ Renommer")
                        .on_hover_text("Raccourci : F2")
                        .clicked()
                {
                    action = Some(Action::StartRename(entry.clone()));
                }
                if ui
                    .button("🗑 Supprimer")
                    .on_hover_text("Raccourci : Suppr")
                    .clicked()
                {
                    action = Some(Action::AskDelete);
                }
            });
            ui.weak("Suppression : vers la corbeille de Windows.");
        }
        action
    }

    /// Contenu du volet pendant la multi-sélection : résumé et actions (supprimer).
    fn actions_content(&self, ui: &mut egui::Ui) -> Option<Action> {
        let count = self.multi.len();
        let kind = if self.multi.first().is_some_and(|e| e.is_dir) {
            "dossier(s)"
        } else {
            "fichier(s)"
        };
        ui.label(egui::RichText::new(format!("{count} {kind} sélectionné(s)")).strong());
        ui.weak("Clic maintenu ou clic sur une ligne pour ajouter ou retirer.");
        ui.add_space(6.0);
        let mut action = self.edit_controls(ui, &format!("{count} {kind}"), None);
        if !self.confirm_delete && ui.button("Désélectionner").clicked() {
            action = Some(Action::ClearMulti);
        }
        ui.add_space(6.0);
        ui.separator();
        let row_height = table_row_height(ui) + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, count, |ui, range| {
                for item in &self.multi[range] {
                    let icon = if item.is_dir { "📁" } else { "📄" };
                    // Taille à jour (celle d'un dossier peut arriver après la sélection).
                    let fresh = self
                        .entries
                        .iter()
                        .find(|e| e.path == item.path)
                        .unwrap_or(item);
                    let size = self.entry_size_text(fresh);
                    table_row(ui, false, &format!("{icon} {}", item.name), &size);
                }
            });
        action
    }

    /// Volet d'aperçu. Renvoie `true` si l'utilisateur a cliqué sur la flèche
    /// « < » / « > » pour agrandir ou réduire le volet, et l'action éventuelle
    /// demandée (ouvrir le fichier).
    fn preview_panel(&self, ui: &mut egui::Ui) -> (bool, Option<Action>) {
        let mut toggle = false;
        let mut action = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (arrow, hint) = if self.preview_expanded {
                (">", "Réduire le volet d'aperçu")
            } else {
                ("<", "Agrandir le volet d'aperçu")
            };
            toggle = ui.button(arrow).on_hover_text(hint).clicked();
            // Un dossier (ou une multi-sélection) n'a pas d'aperçu : le volet propose des actions.
            let actions_mode =
                !self.multi.is_empty() || self.selected.as_ref().is_some_and(|s| s.is_dir);
            ui.heading(if actions_mode { "Actions" } else { "Aperçu" });
        });
        ui.separator();
        if !self.multi.is_empty() {
            return (toggle, self.actions_content(ui));
        }
        let Some(selected) = &self.selected else {
            ui.weak("Sélectionnez un élément.");
            return (toggle, None);
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
            ui.add_space(6.0);
            let what = format!("le dossier « {} »", entry.name);
            return (toggle, self.edit_controls(ui, &what, Some(entry)));
        }
        ui.label(preview::kind_description(&entry.path));
        ui.label(format!("Taille : {}", self.entry_size_text(entry)));
        ui.add_space(4.0);
        let is_video = preview::is_video(&entry.path);
        let is_executable = launch::is_executable(&entry.path);
        ui.horizontal_wrapped(|ui| {
            let label = if is_video {
                "▶ Lire"
            } else if is_executable {
                "▶ Exécuter"
            } else {
                "Ouvrir"
            };
            if ui
                .button(label)
                .on_hover_text("Ouvre le fichier avec l'application par défaut de Windows")
                .clicked()
            {
                action = Some(Action::Open(entry.path.clone()));
            }
            if is_executable
                && ui
                    .button("🛡 Exécuter en administrateur")
                    .on_hover_text("Windows demandera une confirmation (UAC)")
                    .clicked()
            {
                action = Some(Action::OpenAsAdmin(entry.path.clone()));
            }
        });
        let what = format!("le fichier « {} »", entry.name);
        if let Some(edit_action) = self.edit_controls(ui, &what, Some(entry)) {
            action = Some(edit_action);
        }
        ui.add_space(4.0);
        if !preview::is_previewable(&entry.path) {
            ui.add_space(12.0);
            let icon = if is_video { "🎬" } else { "📄" };
            ui.label(egui::RichText::new(icon).size(64.0));
            ui.weak("Pas d'aperçu pour ce type de fichier.");
            return (toggle, action);
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
            PreviewState::Archive { path, preview } if *path == entry.path => {
                let shown = preview.entries.len();
                if preview.total > shown {
                    ui.weak(format!("{shown} premiers éléments sur {}", preview.total));
                } else {
                    ui.weak(format!("{} élément(s)", preview.total));
                }
                ui.add_space(4.0);
                let row_height = table_row_height(ui) + ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show_rows(ui, row_height, shown, |ui, range| {
                        for item in &preview.entries[range] {
                            let icon = if item.is_dir { "📁" } else { "📄" };
                            let size = if item.is_dir {
                                String::new()
                            } else {
                                fs_nav::format_size(item.size)
                            };
                            table_row(ui, false, &format!("{icon} {}", item.name), &size);
                        }
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
        (toggle, action)
    }

    fn status_text(&self) -> String {
        match &self.location {
            Location::ThisPc => format!("{} lecteur(s)", self.drives.len()),
            Location::Dir(_) => {
                let count = if self.visible.len() == self.entries.len() {
                    format!("{} élément(s)", self.entries.len())
                } else {
                    format!(
                        "{} élément(s) affiché(s) sur {}",
                        self.visible.len(),
                        self.entries.len()
                    )
                };
                format!("{count}   |   Sélection multiple : clic maintenu, Ctrl+clic ou Maj+clic")
            }
        }
    }
}

/// Bouton de menu qui reste ouvert tant qu'on ne clique pas à l'extérieur.
fn menu(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    MenuButton::new(title)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, add_contents);
}

/// Ligne en cours de renommage : Entrée valide, Échap ou un clic ailleurs annule.
fn rename_row(ui: &mut egui::Ui, state: &mut RenameState) -> Option<Action> {
    let id = egui::Id::unique("rename-edit");
    let width = (ui.available_width() - SIZE_COLUMN_WIDTH).max(120.0);
    let edit = ui.add_sized(
        [width, table_row_height(ui)],
        egui::TextEdit::singleline(&mut state.text).id(id),
    );
    if state.frames == 0 {
        edit.request_focus();
    } else if state.frames == 1 {
        // Sélectionne le nom sans l'extension, comme l'Explorateur.
        if let Some(mut edit_state) = egui::TextEdit::load_state(ui.ctx(), id) {
            edit_state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(state.initial_selection),
                )));
            edit_state.store(ui.ctx(), id);
        }
    }
    state.frames += 1;
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if edit.lost_focus() && enter {
        return Some(Action::CommitRename {
            path: state.path.clone(),
            new_name: state.text.clone(),
        });
    }
    if escape || (edit.lost_focus() && state.frames > 2) {
        return Some(Action::CancelRename);
    }
    None
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
        if ctx.input(|i| i.pointer.primary_pressed()) {
            self.long_press_done = false;
        }
        // Échap quitte la multi-sélection (sauf pendant la saisie d'un chemin).
        if !self.address_editing
            && !self.multi.is_empty()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.clear_multi();
        }
        let mut action = None;

        // Raccourcis : F2 renomme, Suppr demande la suppression (jamais pendant une saisie).
        if !ctx.egui_wants_keyboard_input() && self.renaming.is_none() && !self.address_editing {
            let (f2, delete) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::F2),
                    i.key_pressed(egui::Key::Delete),
                )
            });
            let single = match self.multi.len() {
                0 => self.selected.as_ref(),
                1 => self.multi.first(),
                _ => None,
            };
            if f2 && let Some(entry) = single {
                action = Some(Action::StartRename(entry.clone()));
            } else if delete && !self.delete_targets().is_empty() {
                action = Some(Action::AskDelete);
            }
        }

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
            } else if let Some(notice) = &self.notice {
                ui.label(notice);
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
        // Pendant la multi-sélection, le volet devient un volet d'actions, même masqué.
        if self.show_preview || !self.multi.is_empty() {
            // Deux identifiants distincts : chaque mode garde sa propre largeur.
            let (id, width) = if self.preview_expanded {
                (
                    "preview_wide",
                    ui.available_width() * PREVIEW_EXPANDED_RATIO,
                )
            } else {
                ("preview", 300.0)
            };
            let (toggle, preview_action) = egui::Panel::right(id)
                .default_size(width)
                .resizable(true)
                .show(ui, |ui| self.preview_panel(ui))
                .inner;
            if toggle {
                self.preview_expanded = !self.preview_expanded;
            }
            action = preview_action.or(action.take());
        }
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.central(ui).or(action.take());
        });

        if let Some(action) = action {
            self.apply(&ctx, action);
        }
        self.track_window_size(&ctx);
        self.save_settings_if_changed();
    }

    /// À la fermeture : mémorise aussi la taille de la fenêtre.
    fn on_exit(&mut self) {
        let settings = self.current_settings();
        self.write_settings(settings);
    }
}
