//! Interface graphique (egui / eframe).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, PopupCloseBehavior};

use crate::dates;
use crate::favorites::Favorites;
use crate::fileops;
use crate::fs_nav::{self, Entry, Location};
use crate::launch;
use crate::media::{self, MediaInfo, MediaKind};
use crate::places::{self, Drive, DriveInfo, DriveKind};
use crate::player::{self, Player, PlayerState};
use crate::preview::{self, ArchivePreview, ImagePreview, PreviewContent, TextPreview};
use crate::settings::Settings;
use crate::trash;
use crate::view::{self, Columns, Side, SortKey, SortOrder, ViewMode};

/// Part de l'écran occupée par la fenêtre à la première ouverture.
pub const WINDOW_SCREEN_RATIO: f32 = 0.8;
/// Largeur du volet d'aperçu agrandi, en part de la largeur disponible.
const PREVIEW_EXPANDED_RATIO: f32 = 0.65;
/// Message affiché quand on tente de mélanger dossiers et fichiers.
const MIXED_TYPES_MESSAGE: &str =
    "Sélection d'un seul type à la fois : dossiers ou fichiers, pas les deux.";
/// Durée du clic maintenu qui démarre la multi-sélection, en secondes.
const LONG_PRESS_SECONDS: f64 = 0.5;
/// Largeur des boutons « Renommer / Annuler » et « Supprimer / Confirmer » du volet Actions :
/// identique pour que « Confirmer » apparaisse exactement à la place de « Supprimer ».
const EDIT_BUTTON_WIDTH: f32 = 120.0;
/// Délai (en secondes) pendant lequel « Confirmer » ignore les clics après « Supprimer » :
/// un double-clic accidentel ne doit pas valider la suppression.
const CONFIRM_ARMING_SECONDS: f64 = 0.6;
/// Déplacement maximal du pointeur, en points, pendant un clic maintenu.
const LONG_PRESS_TOLERANCE: f32 = 12.0;
/// Nombre maximal de threads qui calculent les tailles de dossiers ou lisent des métadonnées.
const MAX_SIZE_WORKERS: usize = 4;
/// Côté d'une tuile de la grille de miniatures, en points (hors espacement).
const GALLERY_TILE: f32 = 150.0;
/// Hauteur du libellé sous la miniature.
const GALLERY_LABEL_HEIGHT: f32 = 22.0;
/// Côté maximal des miniatures décodées pour la grille, en pixels.
const GALLERY_THUMB_PIXELS: u32 = 256;
/// Nombre maximal de miniatures décodées en même temps.
const MAX_THUMBS_IN_FLIGHT: usize = 6;
/// Largeurs minimale et maximale d'une colonne de la navigation « style Mac ».
const COLUMN_VIEW_MIN_WIDTH: f32 = 150.0;
const COLUMN_VIEW_MAX_WIDTH: f32 = 340.0;
/// Largeur des onglets (« intercalaires ») du classeur de la barre latérale.
const SIDEBAR_TAB_WIDTH: f32 = 26.0;
/// Largeur d'ouverture de la barre latérale, en points.
const SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
/// Hauteur de la barre d'occupation d'un lecteur.
const DRIVE_BAR_HEIGHT: f32 = 16.0;
/// Largeurs des deux colonnes à gauche de la grille d'images : lecteurs et navigation.
/// Largeur de la barre de recherche, en haut de la zone centrale.
const SEARCH_WIDTH: f32 = 300.0;
/// Libellés des boutons de la barre du volet de détail (mesurés pour sa largeur minimale).
const OPTION_COLUMNS_LABEL: &str = "▥ Colonnes";
const OPTION_IMAGES_LABEL: &str = "🖼 Images";
const OPTION_VIDEOS_LABEL: &str = "🎬 Vidéos";
const GALLERY_DRIVES_WIDTH: f32 = 170.0;
const GALLERY_NAV_WIDTH: f32 = 210.0;
/// Agrandissement du texte et des boutons des barres du haut, en points.
const TOOLBAR_TEXT_BOOST: f32 = 2.0;

/// Onglets (« intercalaires ») de la barre latérale : le classeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarTab {
    Pc,
    Home,
    Favorites,
    Apps,
    Network,
}

impl SidebarTab {
    const ALL: [SidebarTab; 5] = [
        SidebarTab::Pc,
        SidebarTab::Home,
        SidebarTab::Favorites,
        SidebarTab::Apps,
        SidebarTab::Network,
    ];

    fn title(self) -> &'static str {
        match self {
            SidebarTab::Pc => "Ce PC",
            SidebarTab::Home => "Home",
            SidebarTab::Favorites => "Favoris",
            SidebarTab::Apps => "Applications",
            SidebarTab::Network => "Réseau",
        }
    }
}

/// Colonnes optionnelles du tableau.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Dimensions,
    Duration,
    Created,
    Modified,
    Size,
}

impl Column {
    fn title(self) -> &'static str {
        match self {
            Column::Dimensions => "Dimensions",
            Column::Duration => "Durée",
            Column::Created => "Créé le",
            Column::Modified => "Modifié le",
            Column::Size => "Taille",
        }
    }

    /// Critère de tri d'un clic sur l'en-tête (`None` : dimensions et durée, pas triables).
    fn sort_key(self) -> Option<SortKey> {
        match self {
            Column::Created => Some(SortKey::Created),
            Column::Modified => Some(SortKey::Modified),
            Column::Size => Some(SortKey::Size),
            Column::Dimensions | Column::Duration => None,
        }
    }

    fn width(self) -> f32 {
        match self {
            Column::Dimensions => 105.0,
            Column::Duration => 75.0,
            Column::Created | Column::Modified => 125.0,
            Column::Size => 95.0,
        }
    }
}

/// Résultat renvoyé à l'interface par un thread de travail.
enum Event {
    Preview(PathBuf, Result<PreviewContent, String>),
    /// Capacité et occupation d'un lecteur (`None` si le lecteur n'est pas prêt).
    DriveInfo(PathBuf, Option<DriveInfo>),
    /// Taille d'un dossier ; `generation` identifie le listing concerné.
    FolderSize {
        generation: u64,
        path: PathBuf,
        size: u64,
    },
    /// Métadonnées d'un fichier multimédia ; `generation` identifie le listing concerné.
    Media {
        generation: u64,
        path: PathBuf,
        info: MediaInfo,
    },
    /// Miniature décodée pour la grille d'images (`None` si le décodage a échoué).
    Thumb(PathBuf, Option<ImagePreview>),
    /// Lecteur audio / vidéo prêt (ou échec), après le chargement de VLC.
    PlayerReady(PathBuf, Result<Player, String>),
}

/// Lecture intégrée en cours dans le volet d'aperçu.
enum PlayerSlot {
    None,
    /// VLC se charge et ouvre le fichier (en arrière-plan).
    Loading(PathBuf),
    Ready(Box<ActivePlayer>),
}

struct ActivePlayer {
    path: PathBuf,
    player: Player,
    texture: Option<egui::TextureHandle>,
    /// Version de la dernière image affichée.
    frame_version: u64,
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
    /// Change d'onglet dans le classeur de la barre latérale.
    SidebarTab(SidebarTab),
    Select(Entry),
    AddFavorite(PathBuf),
    RemoveFavorite(PathBuf),
    /// Ajoute un exécutable à l'onglet « Applications ».
    AddApplication(PathBuf),
    RemoveApplication(PathBuf),
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
    /// Entre dans un dossier au clavier (flèche droite) : son premier élément est sélectionné.
    EnterFolder(PathBuf),
    /// Lit un fichier audio ou vidéo dans le volet d'aperçu (VLC), sinon avec l'application
    /// par défaut de Windows.
    Play(PathBuf),
    /// Arrête la lecture intégrée.
    StopPlayer,
    /// Change la façon d'afficher le dossier (liste, colonnes, grille d'images).
    SetViewMode(ViewMode),
    /// Clic sur un en-tête de colonne : trie dessus, ou inverse l'ordre s'il est déjà actif.
    SortBy(SortKey),
    /// Affiche ou masque les dossiers et fichiers cachés.
    ToggleHidden,
    /// Ouvre un dossier et y sélectionne un fichier (vue en colonnes).
    GoSelect {
        dir: PathBuf,
        file: PathBuf,
    },
    /// Passe une ligne en mode renommage (F2, menu contextuel, bouton « Renommer »).
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
    /// Capacité et occupation des lecteurs : absente tant que le calcul est en cours,
    /// `None` si le lecteur n'est pas prêt.
    drive_infos: HashMap<PathBuf, Option<DriveInfo>>,
    /// Option : barre d'occupation et pourcentage des lecteurs.
    show_drive_usage: bool,
    /// Disposition : côté de la barre latérale, de ses onglets et du volet de détail.
    sidebar_side: Side,
    tabs_side: Side,
    preview_side: Side,
    /// Onglet affiché dans la barre latérale.
    sidebar_tab: SidebarTab,
    /// Le clavier pilote la liste des lecteurs : ↑ / ↓ changent de lecteur, → revient au dossier.
    drives_focus: bool,
    /// Lecteur choisi au clavier (index dans `drives`).
    drive_cursor: Option<usize>,
    /// Dossier de l'utilisateur courant (onglet « Home »).
    home: Option<PathBuf>,
    /// Sous-dossiers du dossier de l'utilisateur.
    home_dirs: Vec<Entry>,
    favorites: Favorites,
    /// Applications de l'onglet « Applications » (ajoutées par l'utilisateur).
    applications: Favorites,
    /// ExPLIA lui-même, toujours en tête de l'onglet « Applications ».
    own_exe: Option<PathBuf>,
    /// Contenu brut du dossier courant.
    entries: Vec<Entry>,
    /// Contenu affiché : filtré par extension puis trié.
    visible: Vec<Entry>,
    sort_key: SortKey,
    sort_order: SortOrder,
    /// Recherche (barre au-dessus de la liste) : motifs de noms, voir `view::matches_query`.
    search: String,
    /// Donne le focus à la barre de recherche à la prochaine image (Ctrl+F).
    focus_search: bool,
    show_preview: bool,
    /// Volet d'aperçu agrandi (flèche « < » / « > »).
    preview_expanded: bool,
    /// Option : ne pas calculer la taille des dossiers.
    skip_folder_sizes: bool,
    /// Identifie le listing courant ; l'incrémenter annule les calculs en cours.
    folder_generation: Arc<AtomicU64>,
    /// Façon d'afficher le dossier : liste, colonnes (style Mac) ou grille d'images.
    view_mode: ViewMode,
    /// Afficher les dossiers et fichiers cachés.
    show_hidden: bool,
    /// Colonnes affichées dans le tableau.
    columns: Columns,
    /// Métadonnées multimédia lues en arrière-plan pour le listing courant.
    media: HashMap<PathBuf, MediaInfo>,
    /// Identifie le listing pour lequel les métadonnées sont lues ; l'incrémenter annule.
    media_generation: Arc<AtomicU64>,
    /// Listings des dossiers parents, pour la vue en colonnes.
    column_cache: HashMap<PathBuf, Vec<Entry>>,
    /// Largeur de chaque colonne parente de la vue en colonnes.
    column_widths: HashMap<PathBuf, f32>,
    /// Largeur de la dernière colonne (dossier courant) : dossier, nombre d'éléments, largeur.
    current_column_width: Option<(PathBuf, usize, f32)>,
    /// Après un changement de dossier : fichier à sélectionner (vue en colonnes).
    select_path_after_nav: Option<PathBuf>,
    /// Miniatures de la grille d'images.
    thumbs: HashMap<PathBuf, egui::TextureHandle>,
    /// Miniatures en cours de décodage ou déjà en échec (inutile de les redemander).
    thumbs_requested: HashSet<PathBuf>,
    thumbs_in_flight: usize,
    /// Nombre de tuiles par ligne dans la grille (pour les flèches haut / bas).
    gallery_columns: usize,
    /// Lecture audio / vidéo intégrée (libVLC).
    player: PlayerSlot,
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
    /// Instant (secondes) où « Supprimer » a été demandé : sert à armer « Confirmer ».
    delete_asked_at: Option<f64>,
    /// Dernière ligne atteinte (clic ou flèches) : point de départ des flèches.
    cursor: Option<PathBuf>,
    /// Ligne à rendre visible au prochain dessin de la liste (navigation au clavier).
    scroll_to_index: Option<usize>,
    /// Dernier état de défilement de la liste : (décalage, hauteur visible), en points.
    last_scroll: (f32, f32),
    /// Après l'entrée dans un dossier au clavier (flèche droite) : sélectionne son premier élément.
    select_first_after_nav: bool,
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
            drives: Vec::new(),
            drive_infos: HashMap::new(),
            show_drive_usage: settings.show_drive_usage,
            sidebar_side: settings.sidebar_side,
            tabs_side: settings.tabs_side,
            preview_side: settings.preview_side,
            sidebar_tab: SidebarTab::Pc,
            drives_focus: false,
            drive_cursor: None,
            home: places::user_home().filter(|home| home.is_dir()),
            home_dirs: Vec::new(),
            favorites: Favorites::from_paths(settings.favorites.clone()),
            applications: Favorites::from_paths(settings.applications.clone()),
            own_exe: std::env::current_exe().ok(),
            entries: Vec::new(),
            visible: Vec::new(),
            sort_key: settings.sort_key,
            sort_order: settings.sort_order,
            search: String::new(),
            focus_search: false,
            show_preview: settings.show_preview,
            preview_expanded: settings.preview_expanded,
            skip_folder_sizes: settings.skip_folder_sizes,
            view_mode: settings.view_mode,
            show_hidden: settings.show_hidden,
            columns: settings.columns,
            media: HashMap::new(),
            media_generation: Arc::new(AtomicU64::new(0)),
            column_cache: HashMap::new(),
            column_widths: HashMap::new(),
            current_column_width: None,
            select_path_after_nav: None,
            thumbs: HashMap::new(),
            thumbs_requested: HashSet::new(),
            thumbs_in_flight: 0,
            gallery_columns: 1,
            player: PlayerSlot::None,
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
            delete_asked_at: None,
            cursor: None,
            scroll_to_index: None,
            last_scroll: (0.0, 0.0),
            select_first_after_nav: false,
            preview: PreviewState::None,
            event_tx,
            event_rx,
        };
        app.refresh_drives();
        app.refresh_home_dirs();
        // Au lancement : le dossier de l'utilisateur courant (à défaut, le premier lecteur).
        // La vue du centre n'affiche jamais la liste des lecteurs : elle est dans la barre latérale.
        let start = match &app.home {
            Some(home) => Some(home.clone()),
            None => app.drives.first().map(|drive| drive.path.clone()),
        };
        app.sidebar_tab = if app.home.is_some() {
            SidebarTab::Home
        } else {
            SidebarTab::Pc
        };
        if let Some(start) = start {
            app.navigate(Location::Dir(start));
        }
        app
    }

    /// Relit les lecteurs et calcule leur capacité dans un thread (un lecteur réseau peut
    /// être lent). Les valeurs déjà connues restent affichées jusqu'à leur mise à jour.
    fn refresh_drives(&mut self) {
        self.drives = places::detect_drives();
        let roots: Vec<PathBuf> = self.drives.iter().map(|d| d.path.clone()).collect();
        self.drive_infos.retain(|path, _| roots.contains(path));
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for root in roots {
                let info = places::drive_info(&root);
                let _ = tx.send(Event::DriveInfo(root, info));
                ctx.request_repaint();
            }
        });
    }

    /// Relit les sous-dossiers du dossier de l'utilisateur (onglet « Home »).
    fn refresh_home_dirs(&mut self) {
        self.home_dirs = self
            .home
            .as_deref()
            .and_then(|home| fs_nav::list_dir(home).ok())
            .map(|entries| entries.into_iter().filter(|e| e.is_dir).collect())
            .unwrap_or_default();
    }

    /// Affiche un onglet du classeur ; son contenu est relu (lecteurs, dossiers).
    fn set_sidebar_tab(&mut self, tab: SidebarTab) {
        self.sidebar_tab = tab;
        self.drives_focus = false;
        self.drive_cursor = None;
        match tab {
            SidebarTab::Pc | SidebarTab::Network => self.refresh_drives(),
            SidebarTab::Home => self.refresh_home_dirs(),
            SidebarTab::Favorites | SidebarTab::Apps => {}
        }
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

    /// Oublie les listings et les largeurs de la vue en colonnes (tri, filtre, cachés ou
    /// contenu modifiés).
    fn invalidate_columns(&mut self) {
        self.column_cache.clear();
        self.column_widths.clear();
        self.current_column_width = None;
    }

    /// Applique le filtre d'extension, le masquage des éléments cachés et le tri à une liste.
    /// `with_search` : applique aussi la recherche (dossier affiché, pas les colonnes parentes).
    fn filtered_and_sorted(&self, entries: &[Entry], with_search: bool) -> Vec<Entry> {
        let mut list: Vec<Entry> = entries
            .iter()
            .filter(|entry| {
                (self.show_hidden || !entry.hidden)
                    && (!with_search || view::matches_query(&entry.name, &self.search))
            })
            .cloned()
            .collect();
        view::sort_entries(&mut list, self.sort_key, self.sort_order);
        list
    }

    fn recompute_visible(&mut self) {
        self.visible = self.filtered_and_sorted(&self.entries, true);
    }

    /// Famille multimédia d'un fichier si une colonne affichée a besoin de la lire.
    fn media_needed(&self, entry: &Entry) -> bool {
        if entry.is_dir {
            return false;
        }
        match media::kind_of(&entry.path) {
            Some(MediaKind::Image) => self.columns.dimensions,
            Some(MediaKind::Video) => self.columns.resolution || self.columns.duration,
            Some(MediaKind::Audio) => self.columns.duration,
            None => false,
        }
    }

    /// Lit en arrière-plan les métadonnées multimédia des fichiers du dossier affiché
    /// (dimensions, durée, résolution) pour les colonnes demandées. Annule la lecture précédente.
    /// `keep` : conserve les informations déjà lues (changement de colonnes, pas de dossier).
    fn start_media(&mut self, keep: bool) {
        let generation = self.media_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if !keep {
            self.media.clear();
        }
        if !self.columns.needs_media_info() {
            return;
        }
        let files: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|e| self.media_needed(e) && !self.media.contains_key(&e.path))
            .map(|e| e.path.clone())
            .rev() // `pop` prend par la fin : les premiers fichiers de la liste d'abord
            .collect();
        if files.is_empty() {
            return;
        }
        let queue = Arc::new(Mutex::new(files));
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(MAX_SIZE_WORKERS);
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let current = Arc::clone(&self.media_generation);
            let tx = self.event_tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                loop {
                    if current.load(Ordering::Relaxed) != generation {
                        return;
                    }
                    let Some(path) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                        return;
                    };
                    let info = media::read_info(&path);
                    let event = Event::Media {
                        generation,
                        path,
                        info,
                    };
                    if tx.send(event).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Demande le décodage des miniatures manquantes (grille d'images), quelques-unes à la fois.
    fn request_thumbs(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if self.thumbs_in_flight >= MAX_THUMBS_IN_FLIGHT {
                break;
            }
            if self.thumbs.contains_key(path) || !self.thumbs_requested.insert(path.clone()) {
                continue;
            }
            self.thumbs_in_flight += 1;
            let path = path.clone();
            let tx = self.event_tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let thumb = preview::load_thumbnail(&path, GALLERY_THUMB_PIXELS).ok();
                let _ = tx.send(Event::Thumb(path, thumb));
                ctx.request_repaint();
            });
        }
    }

    /// Change d'emplacement. En cas d'erreur, on reste sur l'emplacement actuel.
    fn navigate(&mut self, target: Location) {
        // La vue du centre ne revient jamais sur les lecteurs : « Ce PC » ouvre l'onglet
        // des lecteurs dans la barre latérale, et le dossier affiché ne change pas.
        let path = match &target {
            Location::ThisPc => {
                self.set_sidebar_tab(SidebarTab::Pc);
                // Les flèches ↑ / ↓ changent alors de lecteur.
                self.drives_focus = true;
                self.address = self.location.display();
                return;
            }
            Location::Dir(path) => path.clone(),
        };
        // Une lecture en cours ne suit pas dans un autre dossier.
        self.stop_player();
        let select_first = std::mem::take(&mut self.select_first_after_nav);
        let select_path = self.select_path_after_nav.take();
        // Dossier quitté : si on remonte d'un niveau, on le resélectionne (comme l'Explorateur).
        let came_from = match &self.location {
            Location::Dir(path) => Some(path.clone()),
            Location::ThisPc => None,
        };
        match fs_nav::list_dir(&path) {
            Ok(entries) => {
                self.entries = entries;
                self.start_folder_sizes();
                self.start_media(false);
            }
            Err(err) => {
                self.error = Some(format!(
                    "Impossible d'ouvrir « {} » : {err}",
                    path.display()
                ));
                self.address = self.location.display();
                return;
            }
        }
        self.error = None;
        self.notice = None;
        self.drives_focus = false;
        self.drive_cursor = None;
        // La recherche vaut pour le dossier affiché : un nouveau dossier repart sans filtre.
        self.search.clear();
        self.address = target.display();
        self.location = target;
        self.selected = None;
        self.clear_multi();
        self.anchor = None;
        self.cursor = None;
        self.scroll_to_index = None;
        self.renaming = None;
        self.preview = PreviewState::None;
        // Les miniatures de la grille appartiennent au dossier quitté.
        self.thumbs.clear();
        self.thumbs_requested.clear();
        self.recompute_visible();
        // Remontée d'un niveau : le dossier quitté redevient la ligne courante.
        if let Some(previous) = came_from
            && let Location::Dir(now) = &self.location
            && previous.parent() == Some(now.as_path())
            && let Some(index) = self.visible.iter().position(|e| e.path == previous)
        {
            self.cursor = Some(previous.clone());
            self.anchor = Some(previous);
            self.selected = Some(self.visible[index].clone());
            self.scroll_to_index = Some(index);
        }
        // Entrée dans un dossier au clavier : on sélectionne son premier élément.
        if select_first && let Some(first) = self.visible.first().cloned() {
            let ctx = self.ctx.clone();
            self.select(&ctx, first);
            self.scroll_to_index = Some(0);
        }
        // Clic sur un fichier d'une colonne précédente (vue en colonnes) : on le sélectionne.
        if let Some(path) = select_path
            && let Some(index) = self.visible.iter().position(|e| e.path == path)
        {
            let entry = self.visible[index].clone();
            let ctx = self.ctx.clone();
            self.select(&ctx, entry);
            self.scroll_to_index = Some(index);
        }
    }

    /// Passe au lecteur précédent (`delta` < 0) ou suivant (`delta` > 0) et l'ouvre ; le clavier
    /// continue de piloter la liste des lecteurs.
    fn step_drive(&mut self, delta: i32) {
        let current = self.drive_cursor.or(match &self.location {
            Location::Dir(path) => self.drives.iter().position(|d| path.starts_with(&d.path)),
            Location::ThisPc => None,
        });
        let Some(next) = view::step_index(current, self.drives.len(), delta) else {
            return;
        };
        let root = self.drives[next].path.clone();
        self.navigate(Location::Dir(root));
        self.drives_focus = true;
        // Même si le lecteur n'est pas prêt (erreur), le curseur avance : on peut passer au suivant.
        self.drive_cursor = Some(next);
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
        let list: Vec<Entry> = self.nav_list().into_iter().cloned().collect();
        match view::select_range(&list, &anchor, &entry.path) {
            Ok(range) => {
                self.confirm_delete = false;
                self.selected = None;
                self.preview = PreviewState::None;
                self.error = None;
                self.anchor = Some(anchor);
                self.cursor = Some(entry.path.clone());
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
                // Les listings mis en cache pour la vue en colonnes ne sont plus à jour.
                self.invalidate_columns();
                self.start_folder_sizes();
                self.start_media(true);
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
        self.drives_focus = false;
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
        self.cursor = Some(entry.path.clone());
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
        // Un fichier en cours de lecture est verrouillé : on libère VLC avant de supprimer.
        self.stop_player();
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
        self.stop_player();
        match fileops::rename_in_place(&path, new_name) {
            Ok(new_path) => {
                self.renaming = None;
                self.error = None;
                if self.favorites.contains(&path) {
                    self.favorites.remove(&path);
                    self.favorites.add(new_path.clone());
                }
                if self.applications.contains(&path) {
                    self.applications.remove(&path);
                    self.applications.add(new_path.clone());
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

    /// Arrête la lecture intégrée (libère le fichier et la mémoire).
    fn stop_player(&mut self) {
        self.player = PlayerSlot::None;
    }

    /// Lance la lecture dans le volet d'aperçu. VLC se charge en arrière-plan ; sans VLC, le
    /// fichier est ouvert avec l'application par défaut de Windows et un message l'explique.
    fn start_player(&mut self, path: PathBuf) {
        self.stop_player();
        if !player::is_available() {
            self.error = Some(player::VLC_MISSING_MESSAGE.to_string());
            self.report_launch_keep_error(launch::open(&path));
            return;
        }
        self.error = None;
        self.player = PlayerSlot::Loading(path.clone());
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = Player::open(&path);
            let _ = tx.send(Event::PlayerReady(path, result));
            ctx.request_repaint();
        });
    }

    /// Traite la fin du chargement de VLC : le lecteur est affiché s'il est toujours attendu.
    fn on_player_ready(&mut self, path: PathBuf, result: Result<Player, String>) {
        let still_wanted = matches!(&self.player, PlayerSlot::Loading(p) if *p == path);
        match result {
            Ok(player) if still_wanted => {
                self.player = PlayerSlot::Ready(Box::new(ActivePlayer {
                    path,
                    player,
                    texture: None,
                    frame_version: 0,
                }));
            }
            // Sélection abandonnée entre-temps : le lecteur est simplement libéré.
            Ok(_) => {}
            Err(message) if still_wanted => {
                self.player = PlayerSlot::None;
                self.error = Some(format!(
                    "{message} Ouverture avec l'application par défaut."
                ));
                self.report_launch_keep_error(launch::open(&path));
            }
            Err(_) => {}
        }
    }

    /// Met à jour la texture de l'image vidéo et demande les images suivantes pendant la lecture.
    fn update_player(&mut self, ctx: &egui::Context) {
        let PlayerSlot::Ready(active) = &mut self.player else {
            return;
        };
        if let Some(frame) = active.player.frame_newer_than(active.frame_version) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
            match &mut active.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    active.texture =
                        Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR));
                }
            }
            active.frame_version = frame.version;
        }
        // Pendant la lecture, l'interface se redessine environ 30 fois par seconde.
        if matches!(
            active.player.state(),
            PlayerState::Playing | PlayerState::Loading
        ) {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// Comme `report_launch`, sans effacer un message déjà affiché en cas de succès.
    fn report_launch_keep_error(&mut self, result: Result<(), String>) {
        if let Err(message) = result {
            self.error = Some(message);
        }
    }

    fn select(&mut self, ctx: &egui::Context, entry: Entry) {
        self.drives_focus = false;
        // Choisir un autre élément arrête la lecture en cours.
        if !matches!(&self.player, PlayerSlot::None) {
            let same = match &self.player {
                PlayerSlot::Loading(path) => *path == entry.path,
                PlayerSlot::Ready(active) => active.path == entry.path,
                PlayerSlot::None => false,
            };
            if !same {
                self.stop_player();
            }
        }
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
        self.cursor = Some(entry.path.clone());
        self.selected = Some(entry);
    }

    /// Traite les résultats des threads de travail.
    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut sizes_changed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Preview(path, result) => self.on_preview(ctx, path, result),
                Event::DriveInfo(path, info) => {
                    self.drive_infos.insert(path, info);
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
                Event::Media {
                    generation,
                    path,
                    info,
                } => {
                    if generation == self.media_generation.load(Ordering::SeqCst) {
                        self.media.insert(path, info);
                    }
                }
                Event::PlayerReady(path, result) => self.on_player_ready(path, result),
                Event::Thumb(path, thumb) => {
                    self.thumbs_in_flight = self.thumbs_in_flight.saturating_sub(1);
                    // Une miniature en échec reste dans `thumbs_requested` : on ne la redemande pas.
                    if let Some(img) = thumb
                        && self.thumbs_requested.contains(&path)
                    {
                        let color = egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba);
                        let texture = ctx.load_texture(
                            format!("thumb:{}", path.to_string_lossy()),
                            color,
                            egui::TextureOptions::LINEAR,
                        );
                        self.thumbs.insert(path, texture);
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
            Action::Go(location) => {
                let drive_root = match &location {
                    Location::Dir(path) => self.drives.iter().position(|d| d.path == *path),
                    Location::ThisPc => None,
                };
                self.navigate(location);
                // Un clic sur un lecteur donne aussi la main aux flèches ↑ / ↓.
                if drive_root.is_some()
                    && matches!(self.sidebar_tab, SidebarTab::Pc | SidebarTab::Network)
                {
                    self.drives_focus = true;
                    self.drive_cursor = drive_root;
                }
            }
            Action::SidebarTab(tab) => self.set_sidebar_tab(tab),
            Action::SortBy(key) => {
                if self.sort_key == key {
                    self.sort_order = match self.sort_order {
                        SortOrder::Ascending => SortOrder::Descending,
                        SortOrder::Descending => SortOrder::Ascending,
                    };
                } else {
                    self.sort_key = key;
                    // Nom : de A à Z ; taille et dates : les plus grands / récents d'abord.
                    self.sort_order = match key {
                        SortKey::Name | SortKey::Type => SortOrder::Ascending,
                        SortKey::Size | SortKey::Modified | SortKey::Created => {
                            SortOrder::Descending
                        }
                    };
                }
                self.recompute_visible();
                // Les colonnes de la vue « style Mac » dépendent du tri.
                self.invalidate_columns();
            }
            Action::Select(entry) => self.select(ctx, entry),
            Action::AddFavorite(path) => {
                self.favorites.add(path);
            }
            Action::RemoveFavorite(path) => self.favorites.remove(&path),
            Action::AddApplication(path) => {
                self.applications.add(path);
            }
            Action::RemoveApplication(path) => self.applications.remove(&path),
            Action::Open(path) => self.report_launch(launch::open(&path)),
            Action::Play(path) => self.start_player(path),
            Action::StopPlayer => self.stop_player(),
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
            Action::AskDelete => {
                self.confirm_delete = !self.delete_targets().is_empty();
                self.delete_asked_at = Some(ctx.input(|i| i.time));
            }
            Action::CancelDelete => self.confirm_delete = false,
            Action::ConfirmDelete => self.delete_selection(),
            Action::EnterFolder(path) => {
                self.select_first_after_nav = true;
                self.navigate(Location::Dir(path));
            }
            Action::SetViewMode(mode) => {
                self.view_mode = mode;
                self.scroll_to_index = self.cursor_index();
            }
            Action::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.invalidate_columns();
                self.recompute_visible();
            }
            Action::GoSelect { dir, file } => {
                self.select_path_after_nav = Some(file);
                self.navigate(Location::Dir(dir));
            }
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
            show_drive_usage: self.show_drive_usage,
            view_mode: self.view_mode,
            show_hidden: self.show_hidden,
            columns: self.columns,
            favorites: self.favorites.paths().to_vec(),
            applications: self.applications.paths().to_vec(),
            sidebar_side: self.sidebar_side,
            tabs_side: self.tabs_side,
            preview_side: self.preview_side,
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

    /// Haut de la zone centrale : fil d'Ariane (ou saisie du chemin) et barre de recherche.
    fn location_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            enlarge_toolbar(ui);
            let spacing = ui.spacing().item_spacing.x;
            let search_width = SEARCH_WIDTH.min(ui.available_width() * 0.4);
            let address_width = (ui.available_width() - search_width - spacing).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(address_width, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if self.address_editing {
                        self.address_edit(ui);
                    } else {
                        let crumb_action = self.breadcrumb_bar(ui);
                        action = crumb_action.or(action.take());
                    }
                },
            );
            self.search_box(ui, search_width);
        });
        action
    }

    /// Barre de recherche : filtre le dossier affiché par nom (voir `view::matches_query`).
    /// Ctrl+F lui donne le focus ; Échap l'efface.
    fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .desired_width(width)
                .hint_text("🔍 Rechercher : nom, *.exe, a*.txt…"),
        );
        if std::mem::take(&mut self.focus_search) {
            edit.request_focus();
        }
        if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.search.clear();
            edit.surrender_focus();
            self.recompute_visible();
        } else if edit.changed() {
            self.recompute_visible();
        }
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

    /// Fil d'Ariane : un bouton par niveau du chemin, qui s'illumine au survol ; un clic sur
    /// l'espace libre passe en mode saisie.
    fn breadcrumb_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let crumbs = match &self.location {
            Location::ThisPc => Vec::new(),
            // Dans le dossier de l'utilisateur : « Home > Videos » plutôt que C: > Users > nom > Videos.
            Location::Dir(path) => fs_nav::breadcrumbs_with_home(path, self.home.as_deref()),
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
                // Survol : le mot prend la couleur des liens et un fond apparaît, comme un bouton.
                let link = ui.visuals().hyperlink_color;
                let hovered = &mut ui.visuals_mut().widgets.hovered;
                hovered.weak_bg_fill = link.gamma_multiply(0.22);
                hovered.bg_fill = hovered.weak_bg_fill;
                hovered.bg_stroke = egui::Stroke::new(1.0, link);
                hovered.fg_stroke.color = link;
                egui::ScrollArea::horizontal()
                    .id_salt("breadcrumb")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .stick_to_right(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let this_pc = crumb_button(ui, "Ce PC");
                            if this_pc.clicked() {
                                action = Some(Action::Go(Location::ThisPc));
                            }
                            for (label, path) in &crumbs {
                                ui.weak(">");
                                let button = crumb_button(ui, label);
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
    fn options_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let mut sizes_option_changed = false;
        let mut columns_changed = false;
        let mut new_view_mode = None;
        let mut toggle_hidden = false;
        let mut hidden_shown = self.show_hidden;
        ui.horizontal(|ui| {
            if ui.button("🖥 Ce PC").clicked() {
                action = Some(Action::Go(Location::ThisPc));
            }
            ui.separator();
            menu(ui, "Afficher", |ui| {
                ui.weak("Colonnes");
                for (flag, label) in [
                    (&mut self.columns.size, "Taille"),
                    (&mut self.columns.modified, "Date de modification"),
                    (&mut self.columns.created, "Date de création"),
                    (&mut self.columns.dimensions, "Images : dimensions"),
                    (&mut self.columns.resolution, "Vidéos : résolution"),
                    (&mut self.columns.duration, "Vidéos et sons : durée"),
                ] {
                    columns_changed |= ui.checkbox(flag, label).changed();
                }
                ui.separator();
                ui.checkbox(&mut self.show_preview, "Volet d'aperçu");
                ui.separator();
                ui.weak("Affichage");
                for (mode, label) in [
                    (ViewMode::List, "Liste"),
                    (ViewMode::Columns, "Colonnes (style Mac)"),
                    (ViewMode::Gallery, "Grille d'images"),
                    (ViewMode::Videos, "Grille de vidéos"),
                ] {
                    if ui.radio(self.view_mode == mode, label).clicked() {
                        new_view_mode = Some(mode);
                    }
                }
                if ui
                    .checkbox(&mut hidden_shown, "Dossiers et fichiers cachés")
                    .changed()
                {
                    toggle_hidden = true;
                }
                ui.separator();
                ui.weak("Disposition (côté de la fenêtre)");
                for (label, side) in [
                    ("Barre latérale", &mut self.sidebar_side),
                    ("Onglets", &mut self.tabs_side),
                    ("Volet de détail", &mut self.preview_side),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.radio_value(side, Side::Left, "Gauche");
                        ui.radio_value(side, Side::Right, "Droite");
                    });
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
                ui.checkbox(
                    &mut self.show_drive_usage,
                    "Occupation des disques (barre et %)",
                );
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
        if columns_changed {
            // Lit (ou arrête de lire) les métadonnées multimédia selon les colonnes cochées.
            self.start_media(true);
        }
        if let Some(mode) = new_view_mode {
            let ctx = ui.ctx().clone();
            self.apply(&ctx, Action::SetViewMode(mode));
        }
        if toggle_hidden {
            let ctx = ui.ctx().clone();
            self.apply(&ctx, Action::ToggleHidden);
        }
        action
    }

    /// Position, dans la liste affichée, de la ligne courante (curseur ou sélection).
    fn cursor_index(&self) -> Option<usize> {
        self.cursor
            .as_ref()
            .or(self.selected.as_ref().map(|s| &s.path))
            .and_then(|path| self.nav_list().iter().position(|e| e.path == *path))
    }

    /// Le dossier affiché est dans le dossier de l'utilisateur (« Home »).
    fn in_home(&self) -> bool {
        match (&self.location, &self.home) {
            (Location::Dir(path), Some(home)) => path.starts_with(home),
            _ => false,
        }
    }

    /// Les colonnes « lecteurs » et « navigation » à gauche de la grille ne s'affichent que hors
    /// de Home : dans Home, le chemin et la barre latérale (onglet Home) suffisent.
    fn grid_side_columns(&self) -> bool {
        !self.in_home()
    }

    /// Le fichier figure dans la grille : les images en mode « Images », les vidéos en mode
    /// « Vidéos ». Hors grille, tout est affiché (les dossiers sont dans la colonne de navigation).
    fn in_grid(&self, entry: &Entry) -> bool {
        // Sans colonne de navigation (dossier de Home), les dossiers sont des tuiles de la grille.
        let folder = entry.is_dir && !self.grid_side_columns();
        match self.view_mode {
            ViewMode::Gallery => {
                folder || (!entry.is_dir && preview::is_supported_image(&entry.path))
            }
            ViewMode::Videos => folder || (!entry.is_dir && preview::is_video(&entry.path)),
            ViewMode::List | ViewMode::Columns => true,
        }
    }

    /// Éléments parcourus au clavier et par plage : ceux de la grille, ou tout le dossier.
    fn nav_list(&self) -> Vec<&Entry> {
        self.visible.iter().filter(|e| self.in_grid(e)).collect()
    }

    /// Panneau de la barre latérale, à gauche ou à droite selon la disposition.
    fn sidebar_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let panel = match self.sidebar_side {
            Side::Left => egui::Panel::left("sidebar-left"),
            Side::Right => egui::Panel::right("sidebar-right"),
        };
        panel
            .default_size(SIDEBAR_DEFAULT_WIDTH)
            .resizable(true)
            .show(ui, |ui| {
                action = self.sidebar(ui);
            });
        action
    }

    /// Volet de détail (aperçu, actions), à gauche ou à droite selon la disposition. Pendant
    /// la multi-sélection, il devient un volet d'actions, même masqué.
    fn detail_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.show_preview && self.multi.is_empty() {
            return None;
        }
        // Jamais plus étroit que la barre « < Colonnes Images Vidéos » sur une ligne.
        let min_width = Self::detail_min_width(ui);
        // Deux identifiants distincts : chaque mode garde sa propre largeur.
        let width = if self.preview_expanded {
            ui.available_width() * PREVIEW_EXPANDED_RATIO
        } else {
            300.0
        }
        .max(min_width);
        let panel = match (self.preview_side, self.preview_expanded) {
            (Side::Left, false) => egui::Panel::left("preview-left"),
            (Side::Left, true) => egui::Panel::left("preview-wide-left"),
            (Side::Right, false) => egui::Panel::right("preview-right"),
            (Side::Right, true) => egui::Panel::right("preview-wide-right"),
        };
        let (toggle, action) = panel
            .default_size(width)
            .min_size(min_width)
            .resizable(true)
            .show(ui, |ui| self.preview_panel(ui))
            .inner;
        if toggle {
            self.preview_expanded = !self.preview_expanded;
        }
        action
    }

    /// Barre latérale en « classeur » : la liste des dossiers de l'onglet actif à gauche,
    /// les onglets (intercalaires : Ce PC, Home, Favoris, Réseau) sur le bord droit.
    fn sidebar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let height = ui.available_height();
        let list_width =
            (ui.available_width() - SIDEBAR_TAB_WIDTH - ui.spacing().item_spacing.x).max(60.0);
        ui.horizontal_top(|ui| {
            if self.tabs_side == Side::Left
                && let Some(tab) = self.sidebar_tabs(ui)
            {
                action = Some(Action::SidebarTab(tab));
            }
            ui.vertical(|ui| {
                ui.set_width(list_width);
                ui.set_height(height);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-list")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        action = self.sidebar_list(ui).or(action.take());
                    });
            });
            if self.tabs_side == Side::Right
                && let Some(tab) = self.sidebar_tabs(ui)
            {
                action = Some(Action::SidebarTab(tab));
            }
        });
        action
    }

    /// Intercalaires du classeur : texte vertical, sur le bord gauche ou droit de la liste.
    fn sidebar_tabs(&self, ui: &mut egui::Ui) -> Option<SidebarTab> {
        let mut clicked = None;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(4.0);
            for tab in SidebarTab::ALL {
                let selected = self.sidebar_tab == tab;
                let font = egui::TextStyle::Body.resolve(ui.style());
                let color = if selected {
                    ui.visuals().selection.stroke.color
                } else {
                    ui.visuals().text_color()
                };
                let galley = ui
                    .painter()
                    .layout_no_wrap(tab.title().to_string(), font, color);
                let text = galley.size();
                let size = egui::vec2(SIDEBAR_TAB_WIDTH, text.x + 20.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let visuals = ui.visuals();
                let fill = if selected {
                    visuals.selection.bg_fill
                } else if response.hovered() {
                    visuals.widgets.hovered.weak_bg_fill
                } else {
                    visuals.widgets.inactive.weak_bg_fill
                };
                // Coins arrondis du côté extérieur ; texte tourné d'un quart de tour (de haut en
                // bas à droite, de bas en haut à gauche), centré dans l'onglet.
                let on_left = self.tabs_side == Side::Left;
                let (corners, origin, angle) = if on_left {
                    (
                        egui::CornerRadius {
                            nw: 8,
                            sw: 8,
                            ne: 0,
                            se: 0,
                        },
                        egui::pos2(
                            rect.center().x - text.y / 2.0,
                            rect.center().y + text.x / 2.0,
                        ),
                        -std::f32::consts::FRAC_PI_2,
                    )
                } else {
                    (
                        egui::CornerRadius {
                            nw: 0,
                            sw: 0,
                            ne: 8,
                            se: 8,
                        },
                        egui::pos2(
                            rect.center().x + text.y / 2.0,
                            rect.center().y - text.x / 2.0,
                        ),
                        std::f32::consts::FRAC_PI_2,
                    )
                };
                ui.painter().rect_filled(rect, corners, fill);
                ui.painter()
                    .add(egui::epaint::TextShape::new(origin, galley, color).with_angle(angle));
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = Some(tab);
                }
            }
        });
        clicked
    }

    /// Contenu de l'onglet actif.
    fn sidebar_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        // Un nom trop long est tronqué, jamais coupé sur deux lignes.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        match self.sidebar_tab {
            SidebarTab::Pc => self.drives_list(ui, false),
            SidebarTab::Network => self.drives_list(ui, true),
            SidebarTab::Home => self.home_list(ui),
            SidebarTab::Favorites => self.favorites_list(ui),
            SidebarTab::Apps => self.applications_list(ui),
        }
    }

    /// Lecteurs, avec leur occupation si l'option est cochée. `network_only` : onglet « Réseau »
    /// (lecteurs réseau connectés seulement).
    fn drives_list(&self, ui: &mut egui::Ui, network_only: bool) -> Option<Action> {
        let mut action = None;
        ui.heading(if network_only { "Réseau" } else { "Ce PC" });
        let mut any = false;
        for (index, drive) in self.drives.iter().enumerate() {
            if network_only && drive.kind != DriveKind::Network {
                continue;
            }
            any = true;
            let info = self.drive_infos.get(&drive.path).and_then(Option::as_ref);
            let here =
                matches!(&self.location, Location::Dir(path) if path.starts_with(&drive.path));
            // Pilotée au clavier : le lecteur choisi par les flèches est en surbrillance.
            let highlighted = match self.drive_cursor {
                Some(cursor) if self.drives_focus => cursor == index,
                _ => here,
            };
            if drive_entry(ui, drive, info, highlighted, self.show_drive_usage).clicked() {
                action = Some(Action::Go(Location::Dir(drive.path.clone())));
            }
        }
        if !any {
            ui.weak(if network_only {
                "Aucun lecteur réseau connecté."
            } else {
                "Aucun lecteur détecté."
            });
        }
        action
    }

    /// Dossier de l'utilisateur et ses sous-dossiers.
    fn home_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let Some(home) = &self.home else {
            ui.weak("Dossier de l'utilisateur introuvable.");
            return None;
        };
        ui.heading("Home");
        let here = self.location == Location::Dir(home.clone());
        if ui
            .selectable_label(here, format!("🏠 {}", favorite_name(home)))
            .on_hover_text(home.display().to_string())
            .clicked()
        {
            action = Some(Action::Go(Location::Dir(home.clone())));
        }
        for entry in &self.home_dirs {
            if entry.hidden && !self.show_hidden {
                continue;
            }
            let here = self.location == Location::Dir(entry.path.clone());
            if ui
                .selectable_label(here, format!("📁 {}", entry.name))
                .on_hover_text(entry.path.display().to_string())
                .clicked()
            {
                action = Some(Action::Go(Location::Dir(entry.path.clone())));
            }
        }
        action
    }

    /// Onglet « Applications » : ExPLIA, puis les exécutables ajoutés par l'utilisateur
    /// (clic droit sur un `.exe` > « Ajouter aux applications »). Un clic lance l'application.
    fn applications_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.heading("Applications");
        if self.applications.paths().is_empty() {
            ui.weak("Clic droit sur un exécutable (.exe, .bat…) pour l'ajouter.");
        }
        let own = self.own_exe.iter().map(|path| (path, false));
        let added = self.applications.paths().iter().map(|path| (path, true));
        for (path, removable) in own.chain(added) {
            let name = if removable {
                path.file_stem().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            } else {
                "ExPLIA".to_string()
            };
            let response = ui
                .selectable_label(false, format!("🚀 {name}"))
                .on_hover_text(path.display().to_string())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                action = Some(Action::Open(path.clone()));
            }
            response.context_menu(|ui| {
                if ui.button("Lancer").clicked() {
                    action = Some(Action::Open(path.clone()));
                    ui.close();
                }
                if ui.button("Lancer en tant qu'administrateur").clicked() {
                    action = Some(Action::OpenAsAdmin(path.clone()));
                    ui.close();
                }
                if let Some(dir) = path.parent()
                    && ui.button("Afficher dans son dossier").clicked()
                {
                    action = Some(Action::GoSelect {
                        dir: dir.to_path_buf(),
                        file: path.clone(),
                    });
                    ui.close();
                }
                if removable && ui.button("Retirer des applications").clicked() {
                    action = Some(Action::RemoveApplication(path.clone()));
                    ui.close();
                }
            });
        }
        action
    }

    fn favorites_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.heading("Favoris");
        if self.favorites.paths().is_empty() {
            ui.weak("Clic droit sur un dossier pour l'ajouter.");
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
        action
    }

    fn central(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        match (&self.location, self.view_mode) {
            (Location::ThisPc, _) => {
                ui.label("Choisissez un dossier ou un lecteur dans la barre latérale.");
                None
            }
            (Location::Dir(_), ViewMode::List) => self.listing_view(ui),
            (Location::Dir(_), ViewMode::Columns) => self.columns_view(ui),
            (Location::Dir(_), ViewMode::Gallery | ViewMode::Videos) => self.gallery_layout(ui),
        }
    }

    /// Navigation « style Mac » : une colonne par niveau du chemin (lecteurs, puis chaque
    /// dossier traversé, puis le dossier courant). Un clic sur un dossier l'ouvre dans la
    /// colonne suivante ; un clic sur un fichier le sélectionne.
    fn columns_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let Location::Dir(current) = self.location.clone() else {
            return None;
        };
        let crumbs = fs_nav::breadcrumbs(&current);
        // Listings des dossiers traversés (le dernier est le dossier courant : `self.visible`),
        // avec la largeur qui convient à leurs noms.
        for (_, path) in crumbs.iter().take(crumbs.len().saturating_sub(1)) {
            if !self.column_cache.contains_key(path)
                && let Ok(list) = fs_nav::list_dir(path)
            {
                let processed = self.filtered_and_sorted(&list, false);
                self.column_widths
                    .insert(path.clone(), column_width_for(ui, &processed));
                self.column_cache.insert(path.clone(), processed);
            }
        }
        let current_width = match &self.current_column_width {
            Some((path, len, width)) if *path == current && *len == self.visible.len() => *width,
            _ => {
                let width = column_width_for(ui, &self.visible);
                self.current_column_width = Some((current.clone(), self.visible.len(), width));
                width
            }
        };
        // La dernière colonne porte aussi les informations de la liste (taille, dates…).
        let info_columns = self.active_columns();
        let info_width: f32 = info_columns.iter().map(|column| column.width()).sum();
        let current_width = current_width + info_width;
        // Largeur de chaque colonne : adaptée à son contenu ; si la place le permet, la dernière
        // colonne occupe le reste de la largeur disponible.
        let mut widths: Vec<f32> = crumbs
            .iter()
            .enumerate()
            .map(|(level, (_, path))| {
                if level + 1 == crumbs.len() {
                    current_width
                } else {
                    self.column_widths
                        .get(path)
                        .copied()
                        .unwrap_or(COLUMN_VIEW_MIN_WIDTH)
                }
            })
            .collect();
        let separator_gap = ui.spacing().item_spacing.x * 2.0 + 1.0;
        let used =
            widths.iter().sum::<f32>() + separator_gap * widths.len().saturating_sub(1) as f32;
        if let Some(last) = widths.last_mut() {
            *last += (ui.available_width() - used - ui.spacing().item_spacing.x).max(0.0);
        }
        let mut action = None;
        let height = ui.available_height();
        let row_height = table_row_height(ui);
        let row_pitch = row_height + ui.spacing().item_spacing.y;
        let scroll_target = self.scroll_to_index.take();
        if scroll_target.is_some() {
            ui.ctx().request_repaint();
        }
        let last_scroll = self.last_scroll;
        let mut new_scroll = last_scroll;
        // Renommage en cours : sorti de `self` le temps de la frame (dernière colonne seulement).
        let mut rename = self.renaming.take();
        egui::ScrollArea::horizontal()
            .id_salt("columns-view")
            .auto_shrink([false; 2])
            .stick_to_right(true)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (level, (name, dir)) in crumbs.iter().enumerate() {
                        let is_last = level + 1 == crumbs.len();
                        let next_dir = crumbs.get(level + 1).map(|(_, path)| path);
                        let entries: &[Entry] = if is_last {
                            &self.visible
                        } else {
                            self.column_cache.get(dir).map_or(&[], Vec::as_slice)
                        };
                        ui.vertical(|ui| {
                            ui.set_width(widths[level]);
                            ui.set_height(height);
                            ui.add(egui::Label::new(egui::RichText::new(name).weak()).truncate());
                            if is_last {
                                if let Some(key) = table_header(
                                    ui,
                                    &info_columns,
                                    (self.sort_key, self.sort_order),
                                ) {
                                    action = Some(Action::SortBy(key));
                                }
                            } else {
                                // Même hauteur que l'en-tête de la dernière colonne : les lignes
                                // des colonnes restent alignées.
                                ui.allocate_exact_size(
                                    egui::vec2(1.0, row_height),
                                    egui::Sense::hover(),
                                );
                                ui.separator();
                            }
                            let mut area = egui::ScrollArea::vertical()
                                .id_salt(("column", level))
                                .auto_shrink([false; 2]);
                            if is_last && let Some(index) = scroll_target {
                                area = area.vertical_scroll_offset(view::scroll_offset_for(
                                    index,
                                    row_pitch,
                                    last_scroll.0,
                                    last_scroll.1,
                                ));
                            }
                            let output =
                                area.show_rows(ui, row_height, entries.len(), |ui, range| {
                                    for entry in &entries[range] {
                                        if is_last
                                            && let Some(state) = rename.as_mut()
                                            && state.path == entry.path
                                        {
                                            if let Some(rename_action) =
                                                rename_row(ui, state, info_width)
                                            {
                                                action = Some(rename_action);
                                            }
                                            continue;
                                        }
                                        let highlighted = if is_last {
                                            self.selected
                                                .as_ref()
                                                .is_some_and(|s| s.path == entry.path)
                                                || self.multi.iter().any(|m| m.path == entry.path)
                                        } else {
                                            next_dir.is_some_and(|next| *next == entry.path)
                                        };
                                        let icon = if entry.is_dir { "📁" } else { "📄" };
                                        let cells: Vec<String> = if is_last {
                                            info_columns
                                                .iter()
                                                .map(|column| self.entry_cell(entry, *column))
                                                .collect()
                                        } else {
                                            Vec::new()
                                        };
                                        let row_columns: &[Column] =
                                            if is_last { &info_columns } else { &[] };
                                        let response = table_row(
                                            ui,
                                            highlighted,
                                            entry.hidden,
                                            &format!("{icon} {}", entry.name),
                                            row_columns,
                                            &cells,
                                        );
                                        let (ctrl, shift) =
                                            ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                                        if is_last && response.clicked() && ctrl {
                                            // Ctrl+clic : ajoute ou retire l'élément de la sélection
                                            // multiple (dossier ou fichier de la dernière colonne).
                                            action = Some(Action::ToggleMulti(entry.clone()));
                                        } else if is_last && response.clicked() && shift {
                                            action = Some(Action::SelectRange(entry.clone()));
                                        } else if response.double_clicked() && !entry.is_dir {
                                            action = Some(open_action(&entry.path));
                                        } else if response.clicked() {
                                            action = Some(if entry.is_dir {
                                                Action::Go(Location::Dir(entry.path.clone()))
                                            } else if is_last {
                                                Action::Select(entry.clone())
                                            } else {
                                                Action::GoSelect {
                                                    dir: dir.clone(),
                                                    file: entry.path.clone(),
                                                }
                                            });
                                        }
                                        // Mêmes fonctions que la liste : renommer (dernière
                                        // colonne), favoris, ouvrir / lire / exécuter.
                                        response.context_menu(|ui| {
                                            if let Some(menu_action) =
                                                self.entry_menu(ui, entry, is_last)
                                            {
                                                action = Some(menu_action);
                                            }
                                        });
                                    }
                                });
                            if is_last {
                                new_scroll =
                                    (output.state.clamped_offset().y, output.inner_rect.height());
                            }
                        });
                        if !is_last {
                            ui.separator();
                        }
                    }
                });
            });
        self.last_scroll = new_scroll;
        self.renaming = rename;
        action
    }

    /// Mode « grille » : hors de Home, trois zones côte à côte — les lecteurs, une colonne de
    /// navigation (`..` et sous-dossiers), puis la grille. Dans Home, la grille seule (avec les
    /// dossiers en tuiles pour pouvoir y entrer).
    fn gallery_layout(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.grid_side_columns() {
            return self.gallery_view(ui);
        }
        let mut action = None;
        let height = ui.available_height();
        let gap = ui.spacing().item_spacing.x * 2.0 + 1.0;
        // Sur une fenêtre étroite, la grille garde au moins une tuile de large.
        let side = (ui.available_width() - GALLERY_TILE - 2.0 * gap).max(0.0);
        let drives_width = GALLERY_DRIVES_WIDTH.min(side * 0.45);
        let nav_width = GALLERY_NAV_WIDTH.min(side - drives_width).max(0.0);
        let grid_width = (ui.available_width() - drives_width - nav_width - 2.0 * gap).max(60.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(drives_width);
                ui.set_height(height);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                egui::ScrollArea::vertical()
                    .id_salt("gallery-drives")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        action = self.drives_list(ui, false).or(action.take());
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(nav_width);
                ui.set_height(height);
                action = self.nav_column(ui).or(action.take());
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(grid_width);
                ui.set_height(height);
                action = self.gallery_view(ui).or(action.take());
            });
        });
        action
    }

    /// Colonne de navigation de la grille d'images : `..` puis les sous-dossiers du dossier.
    fn nav_column(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        // Le dossier de la barre d'adresse, en tête de la colonne.
        if let Location::Dir(path) = &self.location {
            ui.add(egui::Label::new(egui::RichText::new(favorite_name(path)).weak()).truncate());
        }
        let parent = self.location.parent();
        let folders: Vec<&Entry> = self.visible.iter().filter(|e| e.is_dir).collect();
        let shift = usize::from(parent.is_some());
        let row_height = table_row_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("gallery-nav")
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, folders.len() + shift, |ui, range| {
                for row in range {
                    if row < shift {
                        if let Some(parent) = &parent
                            && parent_row(ui).clicked()
                        {
                            action = Some(Action::Go(parent.clone()));
                        }
                        continue;
                    }
                    let entry = folders[row - shift];
                    let selected = self.selected.as_ref().is_some_and(|s| s.path == entry.path);
                    let response = table_row(
                        ui,
                        selected,
                        entry.hidden,
                        &format!("📁 {}", entry.name),
                        &[],
                        &[],
                    );
                    if response.clicked() {
                        action = Some(Action::Go(Location::Dir(entry.path.clone())));
                    }
                    response.context_menu(|ui| {
                        if let Some(menu_action) = self.entry_menu(ui, entry, false) {
                            action = Some(menu_action);
                        }
                    });
                }
            });
        action
    }

    /// Grille de miniatures : visionnage rapide des images du dossier. Les dossiers et les autres
    /// fichiers apparaissent comme des tuiles avec une icône.
    fn gallery_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        // Éléments de la grille (indices dans `visible`) : les images ou les vidéos du dossier.
        let items: Vec<usize> = (0..self.visible.len())
            .filter(|&index| self.in_grid(&self.visible[index]))
            .collect();
        if items.is_empty() {
            ui.label(if self.view_mode == ViewMode::Videos {
                "Aucune vidéo dans ce dossier."
            } else {
                "Aucune image dans ce dossier."
            });
            return None;
        }
        let spacing = ui.spacing().item_spacing;
        let tile_height = GALLERY_TILE + GALLERY_LABEL_HEIGHT;
        let columns = (((ui.available_width() + spacing.x) / (GALLERY_TILE + spacing.x)).floor()
            as usize)
            .max(1);
        self.gallery_columns = columns;
        let rows = items.len().div_ceil(columns);
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            );
        if let Some(index) = self.scroll_to_index.take() {
            ui.ctx().request_repaint();
            let (offset, view_height) = self.last_scroll;
            area = area.vertical_scroll_offset(view::scroll_offset_for(
                index / columns,
                tile_height + spacing.y,
                offset,
                view_height,
            ));
        }
        let mut action = None;
        let mut wanted: Vec<PathBuf> = Vec::new();
        let output = area.show_rows(ui, tile_height, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let Some(&index) = items.get(row * columns + column) else {
                            break;
                        };
                        let entry = &self.visible[index];
                        let selected = self.selected.as_ref().is_some_and(|s| s.path == entry.path)
                            || self.multi.iter().any(|m| m.path == entry.path);
                        let is_image = !entry.is_dir && preview::is_supported_image(&entry.path);
                        let thumb = is_image.then(|| self.thumbs.get(&entry.path)).flatten();
                        if is_image && thumb.is_none() {
                            wanted.push(entry.path.clone());
                        }
                        let response = gallery_tile(ui, entry, selected, thumb);
                        let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                        if response.clicked() && ctrl {
                            // Ctrl+clic : ajoute ou retire la tuile de la sélection multiple.
                            action = Some(Action::ToggleMulti(entry.clone()));
                        } else if response.clicked() && shift {
                            // Maj+clic : sélectionne la plage depuis la dernière tuile choisie.
                            action = Some(Action::SelectRange(entry.clone()));
                        } else if response.double_clicked() {
                            action = Some(if entry.is_dir {
                                Action::Go(Location::Dir(entry.path.clone()))
                            } else {
                                open_action(&entry.path)
                            });
                        } else if response.clicked() {
                            action = Some(Action::Select(entry.clone()));
                        }
                    }
                });
            }
        });
        self.last_scroll = (output.state.clamped_offset().y, output.inner_rect.height());
        self.request_thumbs(&wanted);
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

    /// Colonnes affichées dans le tableau, de gauche à droite (après « Nom »).
    fn active_columns(&self) -> Vec<Column> {
        let mut columns = Vec::new();
        if self.columns.dimensions || self.columns.resolution {
            columns.push(Column::Dimensions);
        }
        if self.columns.duration {
            columns.push(Column::Duration);
        }
        if self.columns.created {
            columns.push(Column::Created);
        }
        if self.columns.modified {
            columns.push(Column::Modified);
        }
        if self.columns.size {
            columns.push(Column::Size);
        }
        columns
    }

    /// Texte d'une cellule du tableau pour une entrée.
    fn entry_cell(&self, entry: &Entry, column: Column) -> String {
        let date = |time: Option<std::time::SystemTime>| time.map(dates::format_local);
        match column {
            Column::Size => self.entry_size_text(entry),
            Column::Modified => date(entry.modified).unwrap_or_default(),
            Column::Created => date(entry.created).unwrap_or_default(),
            Column::Dimensions | Column::Duration => {
                if !self.media_needed(entry) {
                    return String::new();
                }
                let kind = media::kind_of(&entry.path);
                // Cette colonne vaut pour les images (dimensions) ou les vidéos (résolution),
                // et pour la durée des vidéos et des sons, selon ce qui est coché.
                let wanted = match (column, kind) {
                    (Column::Dimensions, Some(MediaKind::Image)) => self.columns.dimensions,
                    (Column::Dimensions, Some(MediaKind::Video)) => self.columns.resolution,
                    (Column::Duration, Some(MediaKind::Video | MediaKind::Audio)) => {
                        self.columns.duration
                    }
                    _ => false,
                };
                if !wanted {
                    return String::new();
                }
                match self.media.get(&entry.path) {
                    None => "…".to_string(),
                    Some(info) => match column {
                        Column::Dimensions => info
                            .dimensions
                            .map_or("—".to_string(), |(w, h)| format!("{w} × {h}")),
                        _ => info
                            .duration
                            .map_or("—".to_string(), dates::format_duration),
                    },
                }
            }
        }
    }

    fn listing_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        // Première ligne : « .. » revient au dossier précédent (le parent).
        let parent = self.location.parent();
        let shift = usize::from(parent.is_some());
        let empty_message = if self.entries.is_empty() {
            Some("Ce dossier est vide.")
        } else if self.visible.is_empty() {
            Some("Aucun élément ne correspond à la recherche.")
        } else {
            None
        };
        let columns = self.active_columns();
        if let Some(message) = empty_message {
            if let Some(parent) = parent
                && parent_row(ui).clicked()
            {
                action = Some(Action::Go(parent));
            }
            ui.label(message);
            return action;
        }
        if let Some(key) = table_header(ui, &columns, (self.sort_key, self.sort_order)) {
            action = Some(Action::SortBy(key));
        }
        // `show_rows` veut la hauteur d'une ligne SANS l'espacement (il l'ajoute lui-même) ;
        // le pas réel entre deux lignes sert au calcul du défilement.
        let row_height = table_row_height(ui);
        let row_pitch = row_height + ui.spacing().item_spacing.y;
        // État modifié pendant le dessin de la liste : sorti de `self` le temps de la frame.
        let mut rename = self.renaming.take();
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            // Un clic maintenu ne doit pas faire défiler la liste.
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            );
        // Navigation au clavier : la ligne atteinte doit rester visible.
        if let Some(index) = self.scroll_to_index.take() {
            // Une seconde image est nécessaire : les lignes affichées dépendent du décalage appliqué.
            ui.ctx().request_repaint();
            let (offset, view_height) = self.last_scroll;
            area = area.vertical_scroll_offset(view::scroll_offset_for(
                index + shift,
                row_pitch,
                offset,
                view_height,
            ));
        }
        let rows = self.visible.len() + shift;
        let output = area.show_rows(ui, row_height, rows, |ui, range| {
            for row in range {
                if row < shift {
                    if let Some(parent) = &parent
                        && parent_row(ui).clicked()
                    {
                        action = Some(Action::Go(parent.clone()));
                    }
                    continue;
                }
                let entry = &self.visible[row - shift];
                // Ligne en cours de renommage : champ de saisie à la place du libellé.
                if let Some(state) = rename.as_mut()
                    && state.path == entry.path
                {
                    if let Some(rename_action) = rename_row(ui, state, Column::Size.width()) {
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
                let cells: Vec<String> = columns
                    .iter()
                    .map(|column| self.entry_cell(entry, *column))
                    .collect();
                let response = table_row(
                    ui,
                    is_selected,
                    entry.hidden,
                    &format!("{icon} {}", entry.name),
                    &columns,
                    &cells,
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
                    // Dossier : on entre ; audio ou vidéo : on la lit ; autre fichier : on l'ouvre
                    // avec son application.
                    action = Some(if entry.is_dir {
                        Action::Go(Location::Dir(entry.path.clone()))
                    } else {
                        open_action(&entry.path)
                    });
                } else if response.clicked() {
                    action = Some(if multi_mode {
                        Action::ToggleMulti(entry.clone())
                    } else {
                        Action::Select(entry.clone())
                    });
                }
                response.context_menu(|ui| {
                    if let Some(menu_action) = self.entry_menu(ui, entry, true) {
                        action = Some(menu_action);
                    }
                });
            }
        });
        self.last_scroll = (output.state.clamped_offset().y, output.inner_rect.height());
        self.renaming = rename;
        action
    }

    /// Menu contextuel d'une ligne (liste, colonnes, colonne de navigation) : renommer
    /// (si `allow_rename`), favoris pour un dossier, ouvrir / lire / exécuter pour un fichier.
    fn entry_menu(&self, ui: &mut egui::Ui, entry: &Entry, allow_rename: bool) -> Option<Action> {
        let mut action = None;
        if allow_rename {
            if ui.button("Renommer  (F2)").clicked() {
                action = Some(Action::StartRename(entry.clone()));
                ui.close();
            }
            ui.separator();
        }
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
            let playable = is_playable(&entry.path);
            let label = if launch::is_executable(&entry.path) {
                "Exécuter"
            } else if playable {
                "Lire"
            } else {
                "Ouvrir"
            };
            if ui.button(label).clicked() {
                action = Some(open_action(&entry.path));
                ui.close();
            }
            if launch::is_executable(&entry.path) {
                if self.applications.contains(&entry.path) {
                    if ui.button("Retirer des applications").clicked() {
                        action = Some(Action::RemoveApplication(entry.path.clone()));
                        ui.close();
                    }
                } else if ui.button("Ajouter aux applications").clicked() {
                    action = Some(Action::AddApplication(entry.path.clone()));
                    ui.close();
                }
            }
            if playable && ui.button("Ouvrir avec le lecteur externe").clicked() {
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
        action
    }

    /// Navigation aux flèches :
    /// - haut / bas : ligne précédente / suivante (avec Maj : étend la sélection) ;
    /// - gauche : dossier parent (le dossier quitté est resélectionné) ;
    /// - droite : entre dans le dossier sélectionné (son premier élément est sélectionné).
    ///
    fn arrow_key_action(&mut self, ctx: &egui::Context) -> Option<Action> {
        let (up, down, left, right, shift, backspace, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.shift,
                i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Enter),
            )
        });
        let gallery = self.view_mode.is_grid();
        if !(up || down || left || right || backspace || (enter && gallery)) {
            return None;
        }
        let vertical = if down { 1 } else { -1 };
        // La liste des lecteurs a la main (flèche gauche au plus haut, ou clic sur un lecteur) :
        // ↑ / ↓ changent de lecteur, → rend la main au dossier.
        if self.drives_focus {
            if up || down {
                self.step_drive(vertical);
            } else if right || (enter && gallery) {
                self.drives_focus = false;
                self.drive_cursor = None;
                self.scroll_to_index = Some(0);
                return self.visible.first().cloned().map(Action::Select);
            }
            return None;
        }
        match &self.location {
            Location::ThisPc => None,
            Location::Dir(_) => {
                // Retour arrière : dossier parent, dans tous les modes d'affichage.
                if backspace {
                    return self.location.parent().map(Action::Go);
                }
                // Grille d'images : les quatre flèches déplacent la sélection dans la grille
                // et Entrée ouvre l'élément (dossier ou fichier).
                if gallery {
                    let columns = i32::try_from(self.gallery_columns.max(1)).unwrap_or(1);
                    let delta = match (left, right, up, down) {
                        (true, ..) => -1,
                        (_, true, ..) => 1,
                        (_, _, true, _) => -columns,
                        (_, _, _, true) => columns,
                        _ => 0,
                    };
                    if delta != 0 {
                        let list = self.nav_list();
                        let next = view::step_index(self.cursor_index(), list.len(), delta)?;
                        let entry = list[next].clone();
                        self.scroll_to_index = Some(next);
                        return Some(if shift {
                            Action::SelectRange(entry)
                        } else {
                            Action::Select(entry)
                        });
                    }
                    return match &self.selected {
                        Some(entry) if enter && entry.is_dir => {
                            Some(Action::EnterFolder(entry.path.clone()))
                        }
                        Some(entry) if enter => Some(open_action(&entry.path)),
                        _ => None,
                    };
                }
                if up || down {
                    let current = self.cursor_index();
                    let next = view::step_index(current, self.visible.len(), vertical)?;
                    let entry = self.visible[next].clone();
                    self.scroll_to_index = Some(next);
                    return Some(if shift {
                        Action::SelectRange(entry)
                    } else {
                        Action::Select(entry)
                    });
                }
                if left {
                    return self.location.parent().map(Action::Go);
                }
                match &self.selected {
                    Some(entry) if right && entry.is_dir && self.multi.is_empty() => {
                        Some(Action::EnterFolder(entry.path.clone()))
                    }
                    _ => None,
                }
            }
        }
    }

    /// Deux boutons côte à côte, de même largeur :
    /// - emplacement 1 : « Renommer » (ou « Désélectionner »), qui devient « Annuler »
    ///   pendant la confirmation ;
    /// - emplacement 2 : « Supprimer », qui devient « Confirmer » **à la même place** :
    ///   on clique deux fois au même endroit pour supprimer (vers la corbeille).
    ///
    /// `rename` : l'élément à renommer (un seul à la fois) ; `deselect` : propose
    /// « Désélectionner » à la place de « Renommer » (multi-sélection).
    fn edit_controls(
        &self,
        ui: &mut egui::Ui,
        what: &str,
        rename: Option<&Entry>,
        deselect: bool,
    ) -> Option<Action> {
        let mut action = None;
        let size = egui::vec2(EDIT_BUTTON_WIDTH, ui.spacing().interact_size.y);
        let now = ui.input(|i| i.time);
        let armed = self
            .delete_asked_at
            .is_none_or(|asked| now - asked > CONFIRM_ARMING_SECONDS);
        if self.confirm_delete && !armed {
            // Réveille l'interface pour que le bouton soit prêt à la fin du délai.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.horizontal(|ui| {
            // Emplacement 1.
            if self.confirm_delete {
                if ui.add_sized(size, egui::Button::new("Annuler")).clicked() {
                    action = Some(Action::CancelDelete);
                }
            } else if let Some(entry) = rename {
                if ui
                    .add_sized(size, egui::Button::new("✏ Renommer"))
                    .on_hover_text("Raccourci : F2")
                    .clicked()
                {
                    action = Some(Action::StartRename(entry.clone()));
                }
            } else if deselect
                && ui
                    .add_sized(size, egui::Button::new("Désélectionner"))
                    .clicked()
            {
                action = Some(Action::ClearMulti);
            }
            // Emplacement 2.
            if self.confirm_delete {
                let confirm =
                    egui::Button::new(egui::RichText::new("Confirmer").color(egui::Color32::WHITE))
                        .fill(egui::Color32::from_rgb(180, 40, 40));
                if ui.add_sized(size, confirm).clicked() && armed {
                    action = Some(Action::ConfirmDelete);
                }
            } else if ui
                .add_sized(size, egui::Button::new("🗑 Supprimer"))
                .on_hover_text("Raccourci : Suppr")
                .clicked()
            {
                action = Some(Action::AskDelete);
            }
        });
        // Ligne d'information sous les boutons : n'en change pas la position.
        if self.confirm_delete {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Envoyer {what} à la corbeille ?  (Entrée : confirmer, Échap : annuler)"),
            );
        } else {
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
        let action = self.edit_controls(ui, &format!("{count} {kind}"), None, true);
        ui.add_space(6.0);
        ui.separator();
        let row_height = table_row_height(ui);
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
                    size_row(ui, false, &format!("{icon} {}", item.name), &size);
                }
            });
        action
    }

    /// Volet d'aperçu. Renvoie `true` si l'utilisateur a cliqué sur la flèche
    /// « < » / « > » pour agrandir ou réduire le volet, et l'action éventuelle
    /// demandée (ouvrir le fichier).
    fn preview_panel(&self, ui: &mut egui::Ui) -> (bool, Option<Action>) {
        let (toggle, option_action) = self.option_bar(ui);
        let content_action = self.preview_content(ui);
        (toggle, option_action.or(content_action))
    }

    /// Largeur minimale du volet de détail : la flèche et les boutons Colonnes, Images et Vidéos
    /// tiennent sur une seule ligne.
    fn detail_min_width(ui: &egui::Ui) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let padding = ui.spacing().button_padding.x * 2.0;
        let gap = ui.spacing().item_spacing.x;
        let labels = [
            "<",
            OPTION_COLUMNS_LABEL,
            OPTION_IMAGES_LABEL,
            OPTION_VIDEOS_LABEL,
        ];
        let buttons: f32 = labels
            .iter()
            .map(|label| {
                let galley = ui.painter().layout_no_wrap(
                    (*label).to_string(),
                    font.clone(),
                    egui::Color32::WHITE,
                );
                galley.size().x + padding
            })
            .sum();
        // Marges du panneau, poignée de redimensionnement, barre de défilement éventuelle.
        buttons + gap * (labels.len() - 1) as f32 + 36.0
    }

    /// Barre du haut du volet de détail : flèche d'agrandissement, navigation en colonnes
    /// (style Mac), visionnage d'images et de vidéos (grilles). Les éléments cachés se règlent dans « Afficher ».
    fn option_bar(&self, ui: &mut egui::Ui) -> (bool, Option<Action>) {
        let mut action = None;
        let mut toggle = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Flèche pour agrandir / réduire le volet : elle pointe vers le centre de la fenêtre
            // quand le volet peut grandir, vers son bord quand il peut rétrécir.
            let grows_left = self.preview_side == Side::Right;
            let (arrow, hint) = if self.preview_expanded {
                (
                    if grows_left { ">" } else { "<" },
                    "Réduire le volet de détail",
                )
            } else {
                (
                    if grows_left { "<" } else { ">" },
                    "Agrandir le volet de détail",
                )
            };
            toggle = ui.button(arrow).on_hover_text(hint).clicked();
            for (mode, label, hint) in [
                (
                    ViewMode::Columns,
                    OPTION_COLUMNS_LABEL,
                    "Navigation en colonnes, comme sur Mac : un niveau du chemin par colonne",
                ),
                (
                    ViewMode::Gallery,
                    OPTION_IMAGES_LABEL,
                    "Visionnage d'images : les images du dossier en grille de miniatures",
                ),
                (
                    ViewMode::Videos,
                    OPTION_VIDEOS_LABEL,
                    "Visionnage de vidéos : les vidéos du dossier en grille",
                ),
            ] {
                let active = self.view_mode == mode;
                if ui
                    .add(egui::Button::new(label).selected(active))
                    .on_hover_text(hint)
                    .clicked()
                {
                    // Un second clic revient à la liste.
                    action = Some(Action::SetViewMode(if active {
                        ViewMode::List
                    } else {
                        mode
                    }));
                }
            }
        });
        ui.separator();
        (toggle, action)
    }

    /// Zone du lecteur intégré. Renvoie l'action éventuelle (arrêt) et `true` si le lecteur
    /// (ou son chargement) est affiché pour ce fichier.
    fn player_area(&self, ui: &mut egui::Ui, entry: &Entry) -> (Option<Action>, bool) {
        match &self.player {
            PlayerSlot::Loading(path) if *path == entry.path => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement de VLC…");
                });
                (None, true)
            }
            PlayerSlot::Ready(active) if active.path == entry.path => {
                (Self::player_controls(ui, active), true)
            }
            _ => (None, false),
        }
    }

    /// Image de la vidéo et commandes : lecture / pause, arrêt, position, volume.
    fn player_controls(ui: &mut egui::Ui, active: &ActivePlayer) -> Option<Action> {
        let mut action = None;
        let player = &active.player;
        match &active.texture {
            Some(texture) => {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(4.0);
                ui.image(egui::load::SizedTexture::new(texture.id(), size * scale));
            }
            None => {
                // Son (pas d'image) ou image pas encore décodée.
                ui.label(egui::RichText::new("🎵").size(48.0));
            }
        }
        let state = player.state();
        let length = player.length();
        ui.horizontal(|ui| {
            let label = match state {
                PlayerState::Playing => "⏸",
                PlayerState::Ended => "⟲",
                _ => "▶",
            };
            if ui
                .button(label)
                .on_hover_text("Lecture / pause (Espace)")
                .clicked()
            {
                player.toggle_pause();
            }
            if ui.button("⏹").on_hover_text("Arrêter").clicked() {
                action = Some(Action::StopPlayer);
            }
            let total = length.map_or("--:--".to_string(), dates::format_duration);
            ui.label(format!(
                "{} / {total}",
                dates::format_duration(player.time())
            ));
            if state == PlayerState::Error {
                ui.colored_label(ui.visuals().error_fg_color, "Erreur de lecture");
            }
        });
        // Les curseurs occupent toute la largeur du volet.
        ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(60.0);
        if let Some(length) = length {
            let total = length.as_secs_f32().max(0.1);
            let mut position = player.time().as_secs_f32().min(total);
            let slider = egui::Slider::new(&mut position, 0.0..=total).show_value(false);
            if ui.add(slider).changed() {
                player.set_time(std::time::Duration::from_secs_f32(position));
            }
        }
        ui.horizontal(|ui| {
            ui.label("🔊");
            ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(40.0);
            let mut volume = player.volume();
            let slider = egui::Slider::new(&mut volume, 0..=100).show_value(false);
            if ui.add(slider).changed() {
                player.set_volume(volume);
            }
        });
        action
    }

    /// Contenu du volet : en-tête, aperçu ou actions.
    fn preview_content(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        if !self.multi.is_empty() {
            return self.actions_content(ui);
        }
        let Some(selected) = &self.selected else {
            ui.weak("Sélectionnez un élément.");
            return None;
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
            return self.edit_controls(ui, &what, Some(entry), false);
        }
        ui.label(preview::kind_description(&entry.path));
        ui.label(format!("Taille : {}", self.entry_size_text(entry)));
        ui.add_space(4.0);
        let is_video = preview::is_video(&entry.path);
        let playable = is_playable(&entry.path);
        let is_executable = launch::is_executable(&entry.path);
        ui.horizontal_wrapped(|ui| {
            if playable {
                // Lecture dans le volet (VLC) ; sans VLC, ouverture avec l'application par défaut.
                if ui
                    .button("▶ Lire")
                    .on_hover_text("Lit le fichier ici (nécessite VLC 64 bits installé)")
                    .clicked()
                {
                    action = Some(Action::Play(entry.path.clone()));
                }
                if ui
                    .button("↗ Lecteur externe")
                    .on_hover_text("Ouvre le fichier avec l'application par défaut de Windows")
                    .clicked()
                {
                    action = Some(Action::Open(entry.path.clone()));
                }
            } else {
                let label = if is_executable {
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
        if let Some(edit_action) = self.edit_controls(ui, &what, Some(entry), false) {
            action = Some(edit_action);
        }
        ui.add_space(4.0);
        // Lecteur audio / vidéo intégré : affiché dès qu'une lecture est lancée sur ce fichier.
        if playable {
            let (player_action, showing) = self.player_area(ui, entry);
            if player_action.is_some() {
                action = player_action;
            }
            if showing {
                return action;
            }
        }
        if !preview::is_previewable(&entry.path) {
            ui.add_space(12.0);
            let icon = if is_video {
                "🎬"
            } else if playable {
                "🎵"
            } else {
                "📄"
            };
            ui.label(egui::RichText::new(icon).size(64.0));
            if playable && !player::is_available() {
                ui.weak(
                    "Lecture intégrée : installez VLC (64 bits, videolan.org). \
                     Sans VLC, « Lire » ouvre le lecteur par défaut.",
                );
            } else if playable {
                ui.weak("Cliquez sur « Lire » pour l'écouter ou la regarder ici.");
            } else {
                ui.weak("Pas d'aperçu pour ce type de fichier.");
            }
            return action;
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
                let row_height = table_row_height(ui);
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
                            size_row(ui, false, &format!("{icon} {}", item.name), &size);
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
        action
    }

    fn status_text(&self) -> String {
        match &self.location {
            Location::ThisPc => "Aucun dossier ouvert".to_string(),
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

/// Ligne « .. » en tête de liste : un clic revient au dossier précédent (le parent).
fn parent_row(ui: &mut egui::Ui) -> egui::Response {
    table_row(ui, false, false, "⬆ ..", &[], &[])
        .on_hover_text("Dossier parent (Retour arrière)")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Mot du fil d'Ariane : sans cadre au repos, bouton coloré au survol (voir `breadcrumb_bar`).
fn crumb_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).frame_when_inactive(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Agrandit le texte et les boutons d'une barre du haut (style local à `ui`).
fn enlarge_toolbar(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size += TOOLBAR_TEXT_BOOST;
        }
    }
    style.spacing.interact_size.y = 30.0;
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
}

/// Bouton de menu qui reste ouvert tant qu'on ne clique pas à l'extérieur.
fn menu(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    MenuButton::new(title)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, add_contents)
        .0
}

/// Ligne en cours de renommage : Entrée valide, Échap ou un clic ailleurs annule.
fn rename_row(ui: &mut egui::Ui, state: &mut RenameState, reserved: f32) -> Option<Action> {
    let id = egui::Id::unique("rename-edit");
    let width = (ui.available_width() - reserved).max(120.0);
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

/// En-tête du tableau : « Nom » à gauche, puis les colonnes demandées alignées à droite
/// (`columns` dans l'ordre d'affichage, de gauche à droite). Un clic sur un titre renvoie le
/// critère de tri correspondant ; le tri actif (`sort`) est marqué d'une flèche.
fn table_header(
    ui: &mut egui::Ui,
    columns: &[Column],
    sort: (SortKey, SortOrder),
) -> Option<SortKey> {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::hover(),
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pad = ui.spacing().button_padding.x;
    let mut clicked = None;
    // Zones des titres : « Nom » prend la place restante, chaque colonne sa largeur.
    let mut cells: Vec<(&str, Option<SortKey>, egui::Rect, egui::Align2)> = Vec::new();
    let mut right = rect.right() - pad;
    for column in columns.iter().rev() {
        let cell = egui::Rect::from_min_max(
            egui::pos2(right - column.width() + pad, rect.top()),
            egui::pos2(right + pad, rect.bottom()),
        );
        cells.push((
            column.title(),
            column.sort_key(),
            cell,
            egui::Align2::RIGHT_CENTER,
        ));
        right -= column.width();
    }
    let name_cell = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((right + pad).max(rect.left() + pad), rect.bottom()),
    );
    cells.push((
        "Nom",
        Some(SortKey::Name),
        name_cell,
        egui::Align2::LEFT_CENTER,
    ));
    for (title, key, cell, align) in cells {
        let response = key.map(|_| {
            ui.interact(
                cell,
                ui.scope_id().with(("header", title)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
        });
        let hovered = response.as_ref().is_some_and(egui::Response::hovered);
        let active = key == Some(sort.0);
        let color = if hovered {
            ui.visuals().hyperlink_color
        } else {
            ui.visuals().strong_text_color()
        };
        let arrow = match (active, sort.1) {
            (false, _) => "",
            (true, SortOrder::Ascending) => " ▲",
            (true, SortOrder::Descending) => " ▼",
        };
        let anchor = match align {
            egui::Align2::LEFT_CENTER => egui::pos2(cell.left() + pad, cell.center().y),
            _ => egui::pos2(cell.right() - pad, cell.center().y),
        };
        ui.painter().text(
            anchor,
            align,
            format!("{title}{arrow}"),
            font.clone(),
            color,
        );
        if response.is_some_and(|r| r.clicked()) {
            clicked = key;
        }
    }
    ui.separator();
    clicked
}

/// Ligne du tableau : nom à gauche, puis une cellule par colonne, alignées à droite.
/// `dimmed` : élément caché (texte atténué). Renvoie la réponse au clic.
fn table_row(
    ui: &mut egui::Ui,
    selected: bool,
    dimmed: bool,
    name: &str,
    columns: &[Column],
    cells: &[String],
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let corner = visuals.widgets.inactive.corner_radius;
        let normal_text = if dimmed {
            visuals.weak_text_color()
        } else {
            visuals.text_color()
        };
        let (background, text_color) = if selected {
            (
                Some(visuals.selection.bg_fill),
                visuals.selection.stroke.color,
            )
        } else if response.hovered() {
            (Some(visuals.widgets.hovered.weak_bg_fill), normal_text)
        } else {
            (None, normal_text)
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let pad = ui.spacing().button_padding.x;
        let painter = ui.painter();
        if let Some(fill) = background {
            painter.rect_filled(rect, corner, fill);
        }
        let mut right = rect.right() - pad;
        for (column, cell) in columns.iter().zip(cells).rev() {
            painter.text(
                egui::pos2(right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                cell,
                font.clone(),
                text_color,
            );
            right -= column.width();
        }
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + pad, rect.top()),
            egui::pos2((right - pad).max(rect.left() + pad), rect.bottom()),
        );
        painter.with_clip_rect(name_rect).text(
            egui::pos2(name_rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            font,
            text_color,
        );
    }
    response
}

/// Largeur d'une colonne (vue « style Mac ») adaptée aux noms les plus longs de la liste.
fn column_width_for(ui: &egui::Ui, entries: &[Entry]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    // Mesurer tous les noms serait coûteux dans un gros dossier : les plus longs suffisent.
    let mut longest: Vec<&Entry> = entries.iter().collect();
    longest.sort_by_key(|entry| std::cmp::Reverse(entry.name.chars().count()));
    let widest = longest
        .iter()
        .take(40)
        .map(|entry| {
            ui.painter()
                .layout_no_wrap(
                    format!("📁 {}", entry.name),
                    font.clone(),
                    egui::Color32::WHITE,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max);
    // Marge pour le remplissage des lignes et la barre de défilement.
    (widest + 44.0).clamp(COLUMN_VIEW_MIN_WIDTH, COLUMN_VIEW_MAX_WIDTH)
}

/// Tuile de la grille d'images : miniature (ou icône) et nom dessous.
fn gallery_tile(
    ui: &mut egui::Ui,
    entry: &Entry,
    selected: bool,
    thumb: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(GALLERY_TILE, GALLERY_TILE + GALLERY_LABEL_HEIGHT),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }
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
    } else if entry.hidden {
        (None, visuals.weak_text_color())
    } else {
        (None, visuals.text_color())
    };
    let painter = ui.painter();
    if let Some(fill) = background {
        painter.rect_filled(rect, corner, fill);
    }
    let picture = egui::Rect::from_min_size(
        rect.min + egui::vec2(6.0, 6.0),
        egui::vec2(GALLERY_TILE - 12.0, GALLERY_TILE - 12.0),
    );
    match thumb {
        Some(texture) => {
            // Miniature ajustée à la zone, proportions conservées et centrée.
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y);
            let fitted = egui::Rect::from_center_size(picture.center(), size * scale);
            painter.image(
                texture.id(),
                fitted,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
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
            painter.text(
                picture.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(54.0),
                text_color,
            );
        }
    }
    let label = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.bottom() - GALLERY_LABEL_HEIGHT),
        egui::pos2(rect.right() - 4.0, rect.bottom()),
    );
    painter.with_clip_rect(label).text(
        label.center(),
        egui::Align2::CENTER_CENTER,
        &entry.name,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    response
}

/// Ligne simple : nom et taille (listes de lecteurs, de sélection, d'archive).
fn size_row(ui: &mut egui::Ui, selected: bool, name: &str, size: &str) -> egui::Response {
    table_row(
        ui,
        selected,
        false,
        name,
        &[Column::Size],
        &[size.to_string()],
    )
}

/// Fichier audio ou vidéo que le lecteur intégré sait lire.
fn is_playable(path: &Path) -> bool {
    matches!(
        media::kind_of(path),
        Some(MediaKind::Video | MediaKind::Audio)
    )
}

/// Action d'« ouverture » d'un fichier (double-clic, Entrée) : lecture intégrée pour l'audio
/// et la vidéo, application associée pour le reste.
fn open_action(path: &Path) -> Action {
    if is_playable(path) {
        Action::Play(path.to_path_buf())
    } else {
        Action::Open(path.to_path_buf())
    }
}

fn favorite_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Couleur de la barre d'occupation : verte, orange à partir de 75 %, rouge à partir de 90 %.
fn usage_color(percent: u8) -> egui::Color32 {
    match percent {
        90.. => egui::Color32::from_rgb(190, 35, 45),
        75..=89 => egui::Color32::from_rgb(190, 125, 0),
        _ => egui::Color32::from_rgb(30, 140, 55),
    }
}

/// Texte de l'infobulle d'un lecteur : type, nom, format, capacité, occupation.
fn drive_tooltip(drive: &Drive, info: Option<&DriveInfo>) -> String {
    let mut text = format!("{}  —  {}", drive.label(), drive.kind_name());
    let Some(info) = info else {
        return text;
    };
    if !info.name.is_empty() {
        text += &format!("\nNom : {}", info.name);
    }
    if !info.file_system.is_empty() {
        text += &format!("\nFormat : {}", info.file_system);
    }
    text += &format!(
        "\nTotal : {}\nUtilisé : {} ({} %)\nLibre : {}",
        fs_nav::format_size(info.total),
        fs_nav::format_size(info.used()),
        info.used_percent(),
        fs_nav::format_size(info.free)
    );
    text
}

/// Ligne d'un lecteur dans la barre latérale : icône, lettre et nom du volume ; dessous, si
/// l'option est cochée, la barre d'occupation avec le pourcentage à l'intérieur.
fn drive_entry(
    ui: &mut egui::Ui,
    drive: &Drive,
    info: Option<&DriveInfo>,
    selected: bool,
    show_usage: bool,
) -> egui::Response {
    let line = table_row_height(ui);
    let usage = info.filter(|info| show_usage && info.total > 0);
    let height = if usage.is_some() {
        line + DRIVE_BAR_HEIGHT + 4.0
    } else {
        line
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
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
        let corner = visuals.widgets.inactive.corner_radius;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let pad = ui.spacing().button_padding.x;
        let mut title = format!("{} {}", drive_icon(drive.kind), drive.label());
        if let Some(info) = info
            && !info.name.is_empty()
        {
            title += &format!("  {}", info.name);
        }
        let painter = ui.painter();
        if let Some(fill) = background {
            painter.rect_filled(rect, corner, fill);
        }
        let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), line));
        painter.with_clip_rect(title_rect).text(
            egui::pos2(rect.left() + pad, title_rect.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            font.clone(),
            text_color,
        );
        if let Some(info) = usage {
            let percent = info.used_percent();
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + pad, rect.top() + line),
                egui::vec2((rect.width() - 2.0 * pad).max(20.0), DRIVE_BAR_HEIGHT),
            );
            painter.rect_filled(bar, 3.0, egui::Color32::from_gray(60));
            let mut filled = bar;
            filled.set_width(bar.width() * f32::from(percent) / 100.0);
            painter.rect_filled(filled, 3.0, usage_color(percent));
            painter.text(
                bar.center(),
                egui::Align2::CENTER_CENTER,
                format!("{percent} %"),
                egui::FontId::proportional(font.size - 1.5),
                egui::Color32::WHITE,
            );
        }
    }
    response.on_hover_text(drive_tooltip(drive, info))
}

fn drive_icon(kind: DriveKind) -> &'static str {
    match kind {
        // Symboles présents dans la police d'egui (le symbole « disque dur » n'y est pas).
        DriveKind::Fixed => "💽",
        DriveKind::Removable => "🔌",
        DriveKind::Network => "🌐",
        DriveKind::Optical => "📀",
        DriveKind::Other => "💽",
    }
}

impl eframe::App for ExpliaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.size_window_once(&ctx);
        self.poll_events(&ctx);
        self.update_player(&ctx);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            self.long_press_done = false;
        }
        // Échap annule la confirmation de suppression, sinon quitte la multi-sélection
        // (sauf pendant la saisie d'un chemin).
        if !self.address_editing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.confirm_delete {
                self.confirm_delete = false;
            } else if !self.multi.is_empty() {
                self.clear_multi();
            }
        }
        let mut action = None;
        // Ctrl+F : va dans la barre de recherche.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.focus_search = true;
        }

        // Raccourcis (jamais pendant une saisie) : F2 renomme ; Suppr demande la suppression,
        // puis Entrée la confirme (Échap l'annule).
        if !ctx.egui_wants_keyboard_input() && self.renaming.is_none() && !self.address_editing {
            let (f2, delete, enter, space) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::F2),
                    i.key_pressed(egui::Key::Delete),
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Space),
                )
            });
            // Espace : lecture / pause du lecteur intégré.
            if space && let PlayerSlot::Ready(active) = &self.player {
                active.player.toggle_pause();
            }
            let single = match self.multi.len() {
                0 => self.selected.as_ref(),
                1 => self.multi.first(),
                _ => None,
            };
            if enter && self.confirm_delete {
                action = Some(Action::ConfirmDelete);
            } else if f2 && let Some(entry) = single {
                action = Some(Action::StartRename(entry.clone()));
            } else if delete && !self.delete_targets().is_empty() {
                action = Some(Action::AskDelete);
            } else if !self.confirm_delete {
                action = self.arrow_key_action(&ctx);
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            enlarge_toolbar(ui);
            ui.add_space(4.0);
            action = self.options_bar(ui).or(action.take());
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
        // Les blocs se placent à gauche ou à droite (menu Afficher > Disposition) ; du même
        // côté, la barre latérale est la plus proche du bord de la fenêtre.
        action = self.sidebar_panel(ui).or(action.take());
        action = self.detail_panel(ui).or(action.take());
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.location_bar(ui).or(action.take());
            ui.add_space(4.0);
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
        self.stop_player();
        let settings = self.current_settings();
        self.write_settings(settings);
    }
}
