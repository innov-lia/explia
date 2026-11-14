//! Interface graphique (egui / eframe).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{self, PopupCloseBehavior};

use crate::clipboard;
use crate::clipboard_manager::{ClipMode, ClipboardManager};
use crate::dates;
use crate::dragdrop::{self, DragOutcome};
use crate::droptarget;
use crate::favorites::Favorites;
use crate::fileops;
use crate::fs_nav::{self, Entry, Location};
use crate::launch;
use crate::media::{self, MediaInfo, MediaKind};
use crate::places::{self, Drive, DriveInfo, DriveKind};
use crate::player::{self, Player, PlayerState};
use crate::preview::{self, ArchivePreview, ImagePreview, PreviewContent, TextPreview};
use crate::scan;
use crate::selection_manager::{MIXED_TYPES_MESSAGE, Selection, SelectionError, Toggle};
use crate::settings::Settings;
use crate::storage_manager::{ItemId, StorageError, StorageManager, TransferReport};
use crate::view::{self, Columns, Side, SortKey, SortOrder, ViewMode};

mod action_manager;
mod context_menu_manager;
mod event_manager;
mod split;
mod state;
mod ui;

use action_manager::{Action, Command};
use event_manager::ItemView;
use split::SplitView;
use state::{Navigation, Playback, PreviewData, ScanState, SidebarState};

/// Part de l'écran occupée par la fenêtre à la première ouverture.
pub const WINDOW_SCREEN_RATIO: f32 = 0.8;
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
/// Déplacement (en points) au-delà duquel un appui sur une ligne devient un glisser vers
/// l'extérieur (plus que la tolérance du clic maintenu).
const DRAG_OUT_DISTANCE: f32 = 16.0;
/// Après un glisser lancé depuis ExPLIA, durée (s) pendant laquelle un dépôt dans sa propre
/// fenêtre est ignoré : relâcher le bouton dans ExPLIA ne crée pas de copie par mégarde.
const OWN_DROP_GUARD_SECONDS: f64 = 1.0;
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
/// Nom de référence qui fixe la largeur de toutes les colonnes de la navigation « style Mac » :
/// les noms plus longs sont coupés par « … ».
const COLUMN_VIEW_REFERENCE_NAME: &str = "MP3Studio YouTube Downloa";
/// Largeur des onglets (« intercalaires ») du classeur de la barre latérale.
const SIDEBAR_TAB_WIDTH: f32 = 26.0;
/// Largeur d'ouverture de la barre latérale, en points.
const SIDEBAR_DEFAULT_WIDTH: f32 = 250.0;
/// Hauteur de la barre d'occupation d'un lecteur.
const DRIVE_BAR_HEIGHT: f32 = 16.0;
/// Largeurs des deux colonnes à gauche de la grille d'images : lecteurs et navigation.
/// Largeur de la barre de recherche, en haut de la zone centrale.
const SEARCH_WIDTH: f32 = 300.0;
/// Libellé du filtre « Sélection », à droite de la barre de recherche (mesuré pour sa place).
const SELECTION_MENU_LABEL: &str = "🔽 Sélection";
/// Libellés des boutons de la barre du volet de détail (mesurés pour sa largeur minimale).
const OPTION_SPLIT_LABEL: &str = "↔ Fractionné";
const OPTION_COLUMNS_LABEL: &str = "▥ Colonnes";
const OPTION_IMAGES_LABEL: &str = "🖼 Images";
const OPTION_VIDEOS_LABEL: &str = "🎬 Vidéos";
const GALLERY_DRIVES_WIDTH: f32 = 170.0;
const GALLERY_NAV_WIDTH: f32 = 210.0;
/// Agrandissement du texte et des boutons des barres du haut, en points.
const TOOLBAR_TEXT_BOOST: f32 = 2.0;

/// Nature des résultats affichés à la place du contenu du dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResultKind {
    /// Recherche récursive par nom.
    Search,
    /// Fichiers de même contenu.
    Duplicates,
    /// Fichiers plus gros que le seuil, du plus gros au plus petit.
    LargeFiles,
}

/// Résultats d'un parcours (recherche récursive, doublons, gros fichiers), affichés dans la
/// liste à la place du contenu du dossier jusqu'à leur fermeture ou un changement de dossier.
struct Results {
    kind: ResultKind,
    /// Parcours en cours (les résultats arrivent au fur et à mesure).
    running: bool,
    /// Recherche limitée à `scan::SEARCH_LIMIT` résultats.
    truncated: bool,
    /// Avancement du parcours (doublons).
    progress: Option<String>,
}

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

/// Glisser venu d'une autre application, au-dessus de la fenêtre.
struct ExternalHover {
    paths: Vec<PathBuf>,
    /// Position du pointeur, en points.
    position: egui::Pos2,
    ctrl: bool,
    shift: bool,
}

/// Lecture de dossier en cours (`request_listing`).
struct PendingListing {
    generation: u64,
    path: PathBuf,
    kind: ListingKind,
    /// Suite à donner si la lecture réussit.
    then: AfterListing,
}

/// Pourquoi un dossier est lu.
enum ListingKind {
    /// Ouverture : le dossier devient l'emplacement affiché. `sidebar` : la barre latérale
    /// garde la main sur cet élément.
    Open { sidebar: Option<usize> },
    /// Relecture du dossier affiché.
    Refresh,
}

/// Suite d'une lecture de dossier réussie.
pub(super) enum AfterListing {
    None,
    /// Sélectionne le premier élément (entrée dans un dossier au clavier).
    SelectFirst,
    /// Sélectionne cet élément (fichier d'une colonne, élément copié ou renommé).
    Select(ItemId),
    /// Sélectionne cet élément et le passe en renommage (nouveau dossier).
    Rename(ItemId),
    /// Affiche ce message (il survit au changement de dossier).
    Notice(String),
    /// Vue en colonnes : part de `anchor` et ajoute `entry` (ou la plage jusqu'à lui).
    Extend {
        anchor: Option<ItemId>,
        entry: Box<Entry>,
        range: bool,
    },
}

/// Résultat renvoyé à l'interface par un thread de travail.
enum Event {
    /// Contenu d'un dossier lu par `request_listing`.
    Listing {
        generation: u64,
        result: Result<Vec<Entry>, StorageError>,
    },
    /// Contenu d'un dossier parent, pour la vue en colonnes.
    ColumnListing {
        generation: u64,
        path: PathBuf,
        entries: Vec<Entry>,
    },
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
    /// Lot de résultats d'un parcours ; `generation` identifie le parcours concerné.
    ScanBatch {
        generation: u64,
        entries: Vec<Entry>,
    },
    /// Groupes de doublons trouvés (un groupe = des fichiers de même contenu).
    ScanGroups {
        generation: u64,
        groups: Vec<Vec<Entry>>,
    },
    /// Avancement d'un parcours.
    ScanProgress {
        generation: u64,
        text: String,
    },
    /// Fin d'une copie (Coller, dépôt) ou d'un déplacement (Couper, Coller) vers `dest`.
    CopyDone {
        dest: PathBuf,
        report: TransferReport,
        mode: ClipMode,
    },
    /// Fin d'un parcours ; `truncated` : résultats limités.
    ScanDone {
        generation: u64,
        truncated: bool,
    },
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
    /// Emplacement affiché, barre d'adresse, onglets, lecture du dossier et position dans la liste.
    nav: Navigation,
    /// Barre latérale : lecteurs, dossier de l'utilisateur, onglet actif et main du clavier.
    sidebar: SidebarState,
    /// Recherche et parcours (recherche récursive, dossiers vides, doublons, gros fichiers) et leurs résultats.
    scan: ScanState,
    /// Lecture audio et vidéo intégrée (libVLC).
    player: Playback,
    /// Aperçu de l'élément sélectionné, miniatures de la grille et métadonnées multimédia du dossier.
    preview: PreviewData,
    error: Option<String>,
    /// Option : barre d'occupation et pourcentage des lecteurs.
    show_drive_usage: bool,
    /// Disposition : côté des onglets de la barre latérale et du volet de détail.
    tabs_side: Side,
    preview_side: Side,
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
    show_preview: bool,
    /// Volet d'aperçu agrandi (flèche « < » / « > »).
    /// Volet de détail au centre et navigation dans le volet latéral (triangle d'échange).
    detail_central: bool,
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
    /// Nombre de tuiles par ligne dans la grille (pour les flèches haut / bas).
    gallery_columns: usize,
    window_sized: bool,
    /// Élément sélectionné, multi-sélection et point d'ancrage (voir `selection_manager`).
    selection: Selection,
    /// Sources de stockage : toutes les opérations sur les fichiers passent par elles.
    storage: StorageManager,
    /// Copier / Couper : références placées dans le presse-papiers de Windows.
    clipboard: ClipboardManager,
    /// Affichage fractionné (deux volets), s'il est ouvert.
    split: Option<SplitView>,
    /// Éléments en cours de glisser dans la fenêtre (vers un dossier, l'autre volet…).
    drag: Option<Vec<PathBuf>>,
    /// Dossier sous le pointeur qui recevra le dépôt, repéré pendant le dessin de l'image.
    drop_target: std::cell::RefCell<Option<PathBuf>>,
    /// Zones de dépôt dessinées pendant l'image en cours.
    drop_zones: std::cell::RefCell<Vec<(egui::Rect, PathBuf)>>,
    /// Zones de la dernière image, lues par la cible de dépôt de Windows.
    shared_zones: Arc<Mutex<event_manager::DropZones>>,
    /// Survol et dépôts venus d'une autre application (cible de dépôt de Windows).
    external_drop: Arc<Mutex<droptarget::DropEvents>>,
    /// Glisser venu d'une autre application au-dessus de la fenêtre : éléments et position.
    external_hover: Option<ExternalHover>,
    /// Fenêtre de Windows d'ExPLIA, une fois connue.
    hwnd: Option<isize>,
    /// Relecture du dossier prévue à cet instant (après un glisser vers l'Explorateur).
    refresh_at: Option<f64>,
    /// Dernier numéro de lecture de dossier attribué (tous volets confondus).
    listing_serial: u64,
    /// Nombre de volets secondaires créés (distingue leurs compteurs).
    pane_serial: u64,
    /// Attente de confirmation de la suppression de la multi-sélection.
    confirm_delete: bool,
    /// Un clic maintenu vient d'être traité : le relâchement ne doit pas compter comme un clic.
    long_press_done: bool,
    /// Un glisser vers l'extérieur a déjà été lancé pour l'appui en cours.
    drag_out_done: bool,
    /// Instant (s) de la fin du dernier glisser lancé depuis ExPLIA (garde contre l'auto-dépôt).
    drag_out_ended_at: Option<f64>,
    /// Ctrl+V était enfoncé à l'image précédente (détection de l'appui).
    paste_keys_were_down: bool,
    /// Une copie est en cours en arrière-plan.
    copying: bool,
    /// Message d'information (non erroné) affiché dans la barre d'état.
    notice: Option<String>,
    /// Renommage en cours (une seule ligne à la fois).
    renaming: Option<RenameState>,
    /// Instant (secondes) où « Supprimer » a été demandé : sert à armer « Confirmer ».
    delete_asked_at: Option<f64>,
    event_tx: Sender<Event>,
    event_rx: Receiver<Event>,
}

impl ExpliaApp {
    pub fn new(ctx: egui::Context, settings: Settings, settings_file: Option<PathBuf>) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let mut app = Self {
            ctx,
            nav: Navigation::new(),
            sidebar: SidebarState::new(),
            scan: ScanState::new(&settings),
            player: Playback::default(),
            preview: PreviewData::default(),
            settings_file,
            window_size: settings.window_size,
            error: None,
            show_drive_usage: settings.show_drive_usage,
            tabs_side: settings.tabs_side,
            preview_side: settings.preview_side,
            favorites: Favorites::from_paths(settings.favorites.clone()),
            applications: Favorites::from_paths(settings.applications.clone()),
            own_exe: std::env::current_exe().ok(),
            entries: Vec::new(),
            visible: Vec::new(),
            sort_key: settings.sort_key,
            sort_order: settings.sort_order,
            show_preview: settings.show_preview,
            detail_central: settings.detail_central,
            skip_folder_sizes: settings.skip_folder_sizes,
            view_mode: settings.view_mode,
            show_hidden: settings.show_hidden,
            columns: settings.columns,
            gallery_columns: 1,
            folder_generation: Arc::new(AtomicU64::new(0)),
            // Une taille mémorisée est appliquée dès la création de la fenêtre (voir `main`).
            window_sized: settings.window_size.is_some(),
            saved_settings: settings,
            selection: Selection::default(),
            storage: StorageManager::default(),
            clipboard: ClipboardManager::default(),
            split: None,
            drag: None,
            drop_target: std::cell::RefCell::new(None),
            drop_zones: std::cell::RefCell::new(Vec::new()),
            shared_zones: Arc::new(Mutex::new(event_manager::DropZones::default())),
            external_drop: Arc::new(Mutex::new(droptarget::DropEvents::default())),
            external_hover: None,
            hwnd: None,
            refresh_at: None,
            listing_serial: 0,
            pane_serial: 0,
            confirm_delete: false,
            long_press_done: false,
            drag_out_done: false,
            drag_out_ended_at: None,
            paste_keys_were_down: false,
            copying: false,
            notice: None,
            renaming: None,
            delete_asked_at: None,
            event_tx,
            event_rx,
        };
        app.refresh_drives();
        app.refresh_home_dirs();
        // Au lancement : le dossier de l'utilisateur courant (à défaut, le premier lecteur).
        // La vue du centre n'affiche jamais la liste des lecteurs : elle est dans la barre latérale.
        let start = match &app.sidebar.home {
            Some(home) => Some(home.clone()),
            None => app.sidebar.drives.first().map(|drive| drive.path.clone()),
        };
        app.sidebar.tab = if app.sidebar.home.is_some() {
            SidebarTab::Home
        } else {
            SidebarTab::Pc
        };
        if let Some(start) = start {
            app.navigate(Location::Dir(start.clone()));
            if app.saved_settings.split_view {
                app.open_split(Some(start));
            }
        }
        app
    }

    /// Relit les lecteurs et calcule leur capacité dans un thread (un lecteur réseau peut
    /// être lent). Les valeurs déjà connues restent affichées jusqu'à leur mise à jour.
    fn refresh_drives(&mut self) {
        self.sidebar.drives = places::detect_drives();
        let roots: Vec<PathBuf> = self.sidebar.drives.iter().map(|d| d.path.clone()).collect();
        self.sidebar
            .drive_infos
            .retain(|path, _| roots.contains(path));
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
        self.sidebar.home_dirs = self
            .sidebar
            .home
            .as_deref()
            .and_then(|home| self.storage.list(&ItemId::path(home)).ok())
            .map(|entries| entries.into_iter().filter(|e| e.is_dir).collect())
            .unwrap_or_default();
    }

    /// Affiche un onglet du classeur ; son contenu est relu (lecteurs, dossiers).
    fn set_sidebar_tab(&mut self, tab: SidebarTab) {
        self.sidebar.tab = tab;
        self.sidebar.focus = false;
        self.sidebar.cursor = None;
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

    /// Oublie les listings de la vue en colonnes (tri, recherche, cachés ou contenu modifiés).
    fn invalidate_columns(&mut self) {
        self.nav.column_cache.clear();
        self.nav.column_requests.clear();
        self.nav.column_generation += 1;
    }

    /// Demande en arrière-plan la lecture d'un dossier parent pour la vue en colonnes.
    fn request_column(&mut self, path: &Path) {
        if self.nav.column_cache.contains_key(path)
            || !self.nav.column_requests.insert(path.to_path_buf())
        {
            return;
        }
        let generation = self.nav.column_generation;
        let path = path.to_path_buf();
        let storage = self.storage.clone();
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // Un dossier illisible donne une colonne vide.
            let entries = storage.list(&ItemId::path(&path)).unwrap_or_default();
            let _ = tx.send(Event::ColumnListing {
                generation,
                path,
                entries,
            });
            ctx.request_repaint();
        });
    }

    /// Applique le filtre d'extension, le masquage des éléments cachés et le tri à une liste.
    /// `with_search` : applique aussi la recherche (dossier affiché, pas les colonnes parentes).
    fn filtered_and_sorted(&self, entries: &[Entry], with_search: bool) -> Vec<Entry> {
        let mut list: Vec<Entry> = entries
            .iter()
            .filter(|entry| {
                (self.show_hidden || !entry.hidden)
                    && (!with_search || view::matches_query(&entry.name, &self.scan.search))
            })
            .cloned()
            .collect();
        view::sort_entries(&mut list, self.sort_key, self.sort_order);
        list
    }

    fn recompute_visible(&mut self) {
        if let Some(results) = &self.scan.results {
            let mut list: Vec<Entry> = self
                .scan
                .result_entries
                .iter()
                // La recherche de la barre filtre aussi les autres résultats (hors recherche récursive).
                .filter(|e| {
                    results.kind == ResultKind::Search
                        || view::matches_query(&e.name, &self.scan.search)
                })
                .cloned()
                .collect();
            // Les doublons restent groupés et les gros fichiers classés du plus gros au plus petit ;
            // le reste suit le tri choisi.
            if !matches!(
                results.kind,
                ResultKind::Duplicates | ResultKind::LargeFiles
            ) {
                view::sort_entries(&mut list, self.sort_key, self.sort_order);
            }
            self.visible = list;
            return;
        }
        self.visible = self.filtered_and_sorted(&self.entries, true);
    }

    /// Copie (ou déplace, `ClipMode::Cut`) des éléments dans `dest`, en arrière-plan, par le
    /// stockage (jamais d'écrasement).
    fn start_transfer(&mut self, sources: Vec<PathBuf>, dest: PathBuf, mode: ClipMode) {
        if sources.is_empty() {
            return;
        }
        if self.copying {
            self.error = Some("Une copie est déjà en cours.".to_string());
            return;
        }
        // Un fichier en cours de lecture est verrouillé : on libère VLC avant de le déplacer.
        if mode == ClipMode::Cut {
            self.stop_player();
        }
        self.copying = true;
        self.error = None;
        let what = match mode {
            ClipMode::Copy => "Copie",
            ClipMode::Cut => "Déplacement",
        };
        self.notice = Some(format!(
            "{what} de {} élément(s) vers « {} »…",
            sources.len(),
            dest.display()
        ));
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        let storage = self.storage.clone();
        std::thread::spawn(move || {
            let sources: Vec<ItemId> = sources.iter().map(ItemId::from).collect();
            let target = ItemId::path(&dest);
            let report = match mode {
                ClipMode::Copy => storage.copy_into(&sources, &target),
                ClipMode::Cut => storage.move_into(&sources, &target),
            };
            let _ = tx.send(Event::CopyDone { dest, report, mode });
            ctx.request_repaint();
        });
    }

    /// Fin d'une copie ou d'un déplacement : bilan dans la barre d'état, dossiers relus (dans
    /// les deux volets de l'affichage fractionné).
    fn on_copy_done(&mut self, dest: &Path, report: TransferReport, mode: ClipMode) {
        self.copying = false;
        if mode == ClipMode::Cut && !report.arrived.is_empty() && report.errors.is_empty() {
            self.clipboard.moved();
        }
        let first = report.arrived.first().cloned();
        self.refresh_after_transfer(dest, first.clone(), mode);
        if self.split.is_some() {
            self.with_other(|app| app.refresh_after_transfer(dest, first, mode));
        }
        let done = format!(
            "{} élément(s) {}",
            report.arrived.len(),
            match mode {
                ClipMode::Copy => "copié(s)",
                ClipMode::Cut => "déplacé(s)",
            }
        );
        if report.errors.is_empty() {
            self.error = None;
            self.notice = Some(format!("{done} dans « {} ».", dest.display()));
        } else {
            let first = &report.errors[0];
            let more = report.errors.len() - 1;
            self.error = Some(if more > 0 {
                format!("{done} ; {first} (et {more} autre(s) problème(s))")
            } else {
                format!("{done} ; {first}")
            });
        }
    }

    /// Relit le dossier affiché s'il a reçu les éléments (le premier arrivé est alors
    /// sélectionné) ou si des éléments en sont peut-être partis (déplacement).
    fn refresh_after_transfer(&mut self, dest: &Path, first: Option<ItemId>, mode: ClipMode) {
        if self.nav.location == Location::Dir(dest.to_path_buf()) {
            let then = first.map_or(AfterListing::None, AfterListing::Select);
            self.refresh_listing(then);
        } else if mode == ClipMode::Cut {
            self.refresh_listing(AfterListing::None);
        }
    }

    /// Ferme un onglet ; s'il était affiché, on passe à son voisin.
    fn close_tab(&mut self, index: usize) {
        if index >= self.nav.tabs.len() {
            return;
        }
        let was_active = index == self.nav.active_tab;
        self.nav.tabs.remove(index);
        if index < self.nav.active_tab {
            self.nav.active_tab -= 1;
        }
        self.nav.active_tab = self
            .nav
            .active_tab
            .min(self.nav.tabs.len().saturating_sub(1));
        let next = self.nav.tabs.get(self.nav.active_tab).cloned();
        if self.nav.tabs.len() <= 1 {
            // Un seul dossier ouvert : plus de barre d'onglets.
            self.nav.tabs.clear();
            self.nav.active_tab = 0;
        }
        if was_active && let Some(path) = next {
            self.navigate(Location::Dir(path));
        }
    }

    /// Annule le parcours en cours et ferme les résultats.
    fn close_results(&mut self) {
        self.scan.generation.fetch_add(1, Ordering::SeqCst);
        self.scan.results = None;
        self.scan.result_entries.clear();
        self.scan.duplicate_groups.clear();
    }

    /// Lance un parcours en arrière-plan sur le dossier affiché ; les résultats remplacent le
    /// contenu du dossier dans la liste. Un parcours précédent est annulé.
    fn start_scan(&mut self, kind: ResultKind) {
        let Location::Dir(root) = self.nav.location.clone() else {
            return;
        };
        let generation = self.scan.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.scan.results = Some(Results {
            kind,
            running: true,
            truncated: false,
            progress: None,
        });
        self.scan.result_entries.clear();
        self.scan.duplicate_groups.clear();
        self.clear_multi();
        self.selection.single = None;
        self.preview.state = PreviewState::None;
        self.error = None;
        self.notice = None;
        self.recompute_visible();
        let query = self.scan.search.clone();
        let recursive = self.scan.recursive;
        let show_hidden = self.show_hidden;
        let large_min_bytes = u64::from(self.scan.large_min_mb) * 1024 * 1024;
        let current = Arc::clone(&self.scan.generation);
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let cancelled = || current.load(Ordering::Relaxed) != generation;
            let send = |event: Event| {
                let _ = tx.send(event);
                ctx.request_repaint();
            };
            let mut truncated = false;
            match kind {
                ResultKind::Search => {
                    truncated = scan::search(
                        &root,
                        &query,
                        show_hidden,
                        scan::SEARCH_LIMIT,
                        &cancelled,
                        &mut |entries| {
                            send(Event::ScanBatch {
                                generation,
                                entries,
                            })
                        },
                    );
                }
                ResultKind::Duplicates => {
                    let found = scan::find_duplicates(
                        &root,
                        recursive,
                        // Toutes les tailles (les fichiers vides ne sont jamais des doublons).
                        0,
                        &cancelled,
                        &mut |count| {
                            send(Event::ScanProgress {
                                generation,
                                text: format!("{count} fichier(s) examiné(s)…"),
                            });
                        },
                    );
                    let groups: Vec<Vec<Entry>> = found
                        .iter()
                        .map(|group| {
                            group
                                .files
                                .iter()
                                .filter_map(|path| fs_nav::entry_from_path(path))
                                .collect()
                        })
                        .collect();
                    send(Event::ScanGroups { generation, groups });
                }
                ResultKind::LargeFiles => {
                    let found = scan::find_large_files(
                        &root,
                        recursive,
                        large_min_bytes,
                        &cancelled,
                        &mut |count| {
                            send(Event::ScanProgress {
                                generation,
                                text: format!("{count} fichier(s) trouvé(s)…"),
                            });
                        },
                    );
                    let entries: Vec<Entry> = found
                        .iter()
                        .filter_map(|(path, _)| fs_nav::entry_from_path(path))
                        .collect();
                    send(Event::ScanBatch {
                        generation,
                        entries,
                    });
                }
            }
            send(Event::ScanDone {
                generation,
                truncated,
            });
        });
    }

    /// Le texte de la barre de recherche a changé : recherche récursive en arrière-plan, ou
    /// filtre immédiat du dossier affiché.
    fn on_search_changed(&mut self) {
        let searching = self.scan.recursive
            && !self.scan.search.trim().is_empty()
            && matches!(self.nav.location, Location::Dir(_));
        if searching {
            self.start_scan(ResultKind::Search);
            return;
        }
        if self
            .scan
            .results
            .as_ref()
            .is_some_and(|r| r.kind == ResultKind::Search)
        {
            self.close_results();
            self.clear_multi();
            self.selection.single = None;
        }
        self.recompute_visible();
    }

    /// L'option « Récursive » a changé : les résultats affichés sont recalculés.
    fn on_recursive_changed(&mut self) {
        if let Some(kind) = self.scan.results.as_ref().map(|r| r.kind) {
            if kind == ResultKind::Search {
                self.on_search_changed();
            } else {
                self.start_scan(kind);
            }
        } else if self.scan.recursive && !self.scan.search.trim().is_empty() {
            self.on_search_changed();
        }
    }

    /// Annule la demande de suppression. Si elle visait un dossier qui n'est pas une ligne de la
    /// liste (dossier affiché, dossier parent d'un résultat), la sélection provisoire est retirée.
    fn cancel_delete(&mut self) {
        self.confirm_delete = false;
        let provisional = self
            .selection
            .single
            .as_ref()
            .is_some_and(|selected| !self.visible.iter().any(|e| e.path == selected.path));
        if provisional {
            self.selection.single = None;
            self.preview.state = PreviewState::None;
        }
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
        let generation = self.preview.media_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if !keep {
            self.preview.media.clear();
        }
        if !self.columns.needs_media_info() {
            return;
        }
        let files: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|e| self.media_needed(e) && !self.preview.media.contains_key(&e.path))
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
            let current = Arc::clone(&self.preview.media_generation);
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
            if self.preview.thumbs_in_flight >= MAX_THUMBS_IN_FLIGHT {
                break;
            }
            if self.preview.thumbs.contains_key(path)
                || !self.preview.thumbs_requested.insert(path.clone())
            {
                continue;
            }
            self.preview.thumbs_in_flight += 1;
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

    /// Change d'emplacement. Le dossier est lu en arrière-plan (un partage réseau peut être
    /// lent) ; il devient l'emplacement affiché quand sa lecture réussit. En cas d'erreur, on
    /// reste sur l'emplacement actuel.
    fn navigate(&mut self, target: Location) {
        self.open_location(target, AfterListing::None, None);
    }

    /// `navigate`, puis `then` une fois le dossier lu. `sidebar` : la barre latérale garde la
    /// main sur cet élément (flèches ↑ / ↓ dans ses listes).
    fn open_location(&mut self, target: Location, then: AfterListing, sidebar: Option<usize>) {
        // La vue du centre ne revient jamais sur les lecteurs : « Ce PC » ouvre l'onglet
        // des lecteurs dans la barre latérale, et le dossier affiché ne change pas.
        let path = match target {
            Location::ThisPc => {
                self.set_sidebar_tab(SidebarTab::Pc);
                // Les flèches ↑ / ↓ changent alors de lecteur.
                self.sidebar.focus = true;
                self.nav.address = self.nav.location.display();
                return;
            }
            Location::Dir(path) => path,
        };
        // Une lecture en cours ne suit pas dans un autre dossier.
        self.stop_player();
        if let Some(index) = sidebar {
            self.sidebar.focus = true;
            self.sidebar.cursor = Some(index);
        }
        self.notice = Some(format!("Ouverture de « {} »…", path.display()));
        self.request_listing(path, ListingKind::Open { sidebar }, then);
    }

    /// Lit `path` en arrière-plan ; le résultat arrive par `Event::Listing`. Une demande
    /// plus récente remplace la précédente (son résultat sera ignoré).
    fn request_listing(&mut self, path: PathBuf, kind: ListingKind, then: AfterListing) {
        // Numéro unique pour toute l'application : chaque volet retrouve ses résultats.
        self.listing_serial += 1;
        let generation = self.listing_serial;
        self.nav.pending_listing = Some(PendingListing {
            generation,
            path: path.clone(),
            kind,
            then,
        });
        let storage = self.storage.clone();
        let tx = self.event_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = storage.list(&ItemId::path(&path));
            let _ = tx.send(Event::Listing { generation, result });
            ctx.request_repaint();
        });
    }

    /// Fin d'une lecture de dossier demandée par `request_listing`.
    fn on_listing(&mut self, generation: u64, result: Result<Vec<Entry>, StorageError>) {
        let Some(pending) = self
            .nav
            .pending_listing
            .take_if(|pending| pending.generation == generation)
        else {
            return;
        };
        match (pending.kind, result) {
            (ListingKind::Open { sidebar }, Ok(entries)) => {
                self.finish_open(pending.path, entries, sidebar);
                self.after_listing(pending.then);
            }
            (ListingKind::Open { .. }, Err(err)) => {
                self.notice = None;
                self.error = Some(format!(
                    "Impossible d'ouvrir « {} » : {err}",
                    pending.path.display()
                ));
                self.nav.address = self.nav.location.display();
            }
            (ListingKind::Refresh, Ok(entries)) => {
                // Le dossier a pu changer entre-temps : seule la relecture du dossier affiché compte.
                if self.nav.location == Location::Dir(pending.path) {
                    self.entries = entries;
                    // Les listings mis en cache pour la vue en colonnes ne sont plus à jour.
                    self.invalidate_columns();
                    self.start_folder_sizes();
                    self.start_media(true);
                    self.recompute_visible();
                    self.after_listing(pending.then);
                }
            }
            (ListingKind::Refresh, Err(err)) => {
                self.error = Some(format!("Impossible de relire le dossier : {err}"));
            }
        }
    }

    /// Le dossier `path` est lu : il devient l'emplacement affiché.
    fn finish_open(&mut self, path: PathBuf, entries: Vec<Entry>, sidebar: Option<usize>) {
        // Dossier quitté : si on remonte d'un niveau, on le resélectionne (comme l'Explorateur).
        let came_from = match &self.nav.location {
            Location::Dir(path) => Some(path.clone()),
            Location::ThisPc => None,
        };
        self.entries = entries;
        self.start_folder_sizes();
        self.start_media(false);
        self.error = None;
        self.notice = None;
        if sidebar.is_none() {
            self.sidebar.focus = false;
            self.sidebar.cursor = None;
        }
        // La recherche et les résultats valent pour le dossier affiché : un nouveau dossier
        // repart sans filtre.
        self.close_results();
        self.scan.search.clear();
        if let Some(tab) = self.nav.tabs.get_mut(self.nav.active_tab) {
            tab.clone_from(&path);
        }
        let target = Location::Dir(path);
        self.nav.address = target.display();
        self.nav.location = target;
        self.selection.single = None;
        self.clear_multi();
        self.selection.anchor = None;
        self.nav.cursor = None;
        self.nav.scroll_to_index = None;
        self.renaming = None;
        self.preview.state = PreviewState::None;
        // Les miniatures de la grille et les colonnes appartiennent au dossier quitté.
        self.preview.thumbs.clear();
        self.preview.thumbs_requested.clear();
        self.recompute_visible();
        // Remontée d'un niveau : le dossier quitté redevient la ligne courante.
        if let Some(previous) = came_from
            && let Location::Dir(now) = &self.nav.location
            && previous.parent() == Some(now.as_path())
            && let Some(index) = self.visible.iter().position(|e| e.path == previous)
        {
            self.nav.cursor = Some(previous.clone());
            self.selection.anchor = Some(previous);
            self.selection.single = Some(self.visible[index].clone());
            self.nav.scroll_to_index = Some(index);
        }
    }

    /// Suite d'une lecture de dossier réussie.
    fn after_listing(&mut self, then: AfterListing) {
        let ctx = self.ctx.clone();
        let position = |app: &Self, id: &ItemId| app.visible.iter().position(|e| e.id == *id);
        match then {
            AfterListing::None => {}
            AfterListing::Notice(notice) => self.notice = Some(notice),
            AfterListing::SelectFirst => {
                if let Some(first) = self.visible.first().cloned() {
                    self.select(&ctx, first);
                    self.nav.scroll_to_index = Some(0);
                }
            }
            AfterListing::Select(id) | AfterListing::Rename(id)
                if position(self, &id).is_none() => {}
            AfterListing::Select(id) => {
                if let Some(index) = position(self, &id) {
                    let entry = self.visible[index].clone();
                    self.select(&ctx, entry);
                    self.nav.scroll_to_index = Some(index);
                }
            }
            AfterListing::Rename(id) => {
                if let Some(index) = position(self, &id) {
                    let entry = self.visible[index].clone();
                    self.select(&ctx, entry.clone());
                    self.nav.scroll_to_index = Some(index);
                    self.start_rename(&entry);
                }
            }
            AfterListing::Extend {
                anchor,
                entry,
                range,
            } => {
                // L'élément en surbrillance de la colonne devient le point de départ.
                if let Some(anchor) = anchor
                    && let Some(index) = position(self, &anchor)
                {
                    let first = self.visible[index].clone();
                    self.select(&ctx, first);
                }
                let target = self
                    .visible
                    .iter()
                    .find(|e| e.id == entry.id)
                    .cloned()
                    .unwrap_or(*entry);
                if range {
                    self.select_range(target);
                } else {
                    self.toggle_multi(target);
                }
            }
        }
    }

    /// Dossiers de la liste de l'onglet actif, dans l'ordre affiché (navigation aux flèches).
    fn sidebar_items(&self) -> Vec<PathBuf> {
        match self.sidebar.tab {
            SidebarTab::Pc => self.sidebar.drives.iter().map(|d| d.path.clone()).collect(),
            SidebarTab::Network => self
                .sidebar
                .drives
                .iter()
                .filter(|d| d.kind == DriveKind::Network)
                .map(|d| d.path.clone())
                .collect(),
            SidebarTab::Home => self
                .sidebar
                .home
                .iter()
                .cloned()
                .chain(
                    self.sidebar
                        .home_dirs
                        .iter()
                        .filter(|e| self.show_hidden || !e.hidden)
                        .map(|e| e.path.clone()),
                )
                .collect(),
            SidebarTab::Favorites => self.favorites.paths().to_vec(),
            SidebarTab::Apps => Vec::new(),
        }
    }

    /// Dossier choisi aux flèches dans la barre latérale, quand elle a la main.
    fn sidebar_cursor_path(&self) -> Option<PathBuf> {
        if !self.sidebar.focus {
            return None;
        }
        self.sidebar
            .cursor
            .and_then(|index| self.sidebar_items().get(index).cloned())
    }

    /// Passe à l'élément précédent (`delta` < 0) ou suivant (`delta` > 0) de la liste de la barre
    /// latérale (lecteurs, dossiers de Home, favoris) et l'ouvre ; le clavier garde la main.
    fn step_sidebar(&mut self, delta: i32) {
        let items = self.sidebar_items();
        let current = self.sidebar.cursor.or(match &self.nav.location {
            Location::Dir(path) => items.iter().position(|p| p == path).or_else(|| {
                // Lecteurs : le dossier affiché peut être n'importe où sur le lecteur.
                matches!(self.sidebar.tab, SidebarTab::Pc | SidebarTab::Network)
                    .then(|| items.iter().position(|root| path.starts_with(root)))
                    .flatten()
            }),
            Location::ThisPc => None,
        });
        let Some(next) = view::step_index(current, items.len(), delta) else {
            return;
        };
        // Même si le dossier n'est pas accessible (erreur), le curseur avance : on peut passer
        // au suivant.
        self.open_location(
            Location::Dir(items[next].clone()),
            AfterListing::None,
            Some(next),
        );
    }

    /// Maj+clic : sélectionne la plage entre l'ancre (dernier élément choisi) et `entry`.
    fn select_range(&mut self, entry: Entry) {
        let list: Vec<Entry> = self.nav_list().into_iter().cloned().collect();
        match self.selection.select_range(&list, &entry.path) {
            Ok(()) => {
                self.confirm_delete = false;
                self.preview.state = PreviewState::None;
                self.error = None;
                self.nav.cursor = Some(entry.path.clone());
            }
            Err(SelectionError::MixedTypes) => {
                self.error = Some(MIXED_TYPES_MESSAGE.to_string());
            }
            // Rien de choisi avant, ou ancre hors de la liste : Maj+clic démarre la sélection.
            Err(SelectionError::NotFound) => self.toggle_multi(entry),
        }
    }

    /// Relit le dossier affiché (après une suppression, par exemple), en arrière-plan, puis
    /// `then`. Si un autre dossier est en cours d'ouverture, sa lecture suffit.
    fn refresh_listing(&mut self, then: AfterListing) {
        // Parmi les résultats, on retire ce qui n'existe plus (supprimé, renommé).
        self.scan.result_entries.retain(|e| e.path.exists());
        if self
            .nav
            .pending_listing
            .as_ref()
            .is_some_and(|p| matches!(p.kind, ListingKind::Open { .. }))
        {
            return;
        }
        let Location::Dir(path) = self.nav.location.clone() else {
            return;
        };
        self.request_listing(path, ListingKind::Refresh, then);
    }

    /// Ajoute ou retire une ligne de la multi-sélection. Un seul type à la fois :
    /// on ne mélange pas dossiers et fichiers.
    fn toggle_multi(&mut self, entry: Entry) {
        self.sidebar.focus = false;
        self.confirm_delete = false;
        let path = entry.path.clone();
        match self.selection.toggle(entry) {
            Ok(Toggle::Added) => {
                self.preview.state = PreviewState::None;
                self.error = None;
                self.nav.cursor = Some(path);
            }
            Ok(Toggle::Removed | Toggle::Deselected) => self.preview.state = PreviewState::None,
            Err(_) => {
                if self.selection.single.is_none() {
                    self.preview.state = PreviewState::None;
                }
                self.error = Some(MIXED_TYPES_MESSAGE.to_string());
            }
        }
    }

    /// Quitte la multi-sélection.
    fn clear_multi(&mut self) {
        self.selection.clear_multi();
        self.confirm_delete = false;
    }

    /// Éléments visés par « Supprimer » : la multi-sélection, sinon l'élément sélectionné.
    fn delete_targets(&self) -> Vec<Entry> {
        self.selection.targets()
    }

    fn delete_selection(&mut self) {
        // Un fichier en cours de lecture est verrouillé : on libère VLC avant de supprimer.
        self.stop_player();
        let ids = self.selection.target_ids();
        let count = ids.len();
        self.confirm_delete = false;
        match self.storage.delete(&ids).map_err(|e| e.to_string()) {
            Ok(()) => {
                self.error = None;
                self.notice = Some(format!("{count} élément(s) envoyé(s) à la corbeille."));
                // Le dossier affiché a lui-même été supprimé : on remonte au dossier parent.
                if let Location::Dir(current) = self.nav.location.clone()
                    && !current.exists()
                    && let Some(parent) = current.parent()
                {
                    let notice = self.notice.take().unwrap_or_default();
                    self.open_location(
                        Location::Dir(parent.to_path_buf()),
                        AfterListing::Notice(notice),
                        None,
                    );
                    return;
                }
            }
            Err(message) => {
                self.error = Some(message);
                self.notice = None;
            }
        }
        // Dans tous les cas, on relit le dossier : certains éléments ont pu être supprimés.
        self.clear_multi();
        self.selection.single = None;
        self.preview.state = PreviewState::None;
        self.refresh_listing(AfterListing::None);
    }

    /// Crée un dossier dans `parent` (qui devient le dossier affiché), le sélectionne et le
    /// passe en renommage, comme l'Explorateur.
    fn create_folder(&mut self, parent: PathBuf) {
        match self.storage.create_folder(&ItemId::path(&parent)) {
            Ok(id) => {
                let name = id.as_path().map(favorite_name).unwrap_or_default();
                let then = AfterListing::Rename(id);
                if self.nav.location == Location::Dir(parent.clone()) {
                    if self.scan.results.is_some() {
                        self.close_results();
                    }
                    self.refresh_listing(then);
                } else {
                    self.open_location(Location::Dir(parent), then, None);
                }
                self.error = None;
                self.notice = Some(format!("Dossier créé : « {name} »."));
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    /// Démarre le renommage de `entry` directement dans la liste.
    fn start_rename(&mut self, entry: &Entry) {
        // Seul un élément de la liste affichée se renomme (pas le dossier courant lui-même).
        if !matches!(self.nav.location, Location::Dir(_))
            || !self.visible.iter().any(|e| e.path == entry.path)
        {
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
    fn commit_rename(&mut self, path: PathBuf, new_name: &str) {
        self.stop_player();
        let id = self
            .visible
            .iter()
            .find(|e| e.path == path)
            .map_or_else(|| ItemId::path(&path), |e| e.id.clone());
        match self
            .storage
            .rename(&id, new_name)
            .map_err(|e| e.to_string())
        {
            Ok(new_id) => {
                let new_path = new_id.as_path().map(Path::to_path_buf).unwrap_or_default();
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
                // L'élément renommé reste sélectionné une fois le dossier relu.
                self.refresh_listing(AfterListing::Select(new_id));
                self.notice = Some(format!("Renommé en « {new_name} »."));
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn submit_address(&mut self) {
        let Some(target) = fs_nav::parse_address(&self.nav.address) else {
            self.nav.address = self.nav.location.display();
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

    /// Entre en plein écran (fenêtre en plein écran, vidéo seule) ou en sort.
    fn set_video_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        if self.player.fullscreen != on {
            self.player.fullscreen = on;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        }
    }

    /// Arrête la lecture intégrée (libère le fichier et la mémoire).
    fn stop_player(&mut self) {
        self.player.slot = PlayerSlot::None;
    }

    /// Lance la lecture dans le volet d'aperçu. VLC se charge en arrière-plan ; sans VLC, le
    /// fichier est ouvert avec l'application par défaut de Windows et un message l'explique.
    fn start_player(&mut self, path: PathBuf) {
        self.stop_player();
        if !player::is_available() {
            self.error = Some(player::VLC_MISSING_MESSAGE.to_string());
            self.report_launch_keep_error(
                self.storage
                    .open(&ItemId::path(&path))
                    .map_err(|e| e.to_string()),
            );
            return;
        }
        self.error = None;
        self.player.slot = PlayerSlot::Loading(path.clone());
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
        let still_wanted = matches!(&self.player.slot, PlayerSlot::Loading(p) if *p == path);
        match result {
            Ok(player) if still_wanted => {
                self.player.slot = PlayerSlot::Ready(Box::new(ActivePlayer {
                    path,
                    player,
                    texture: None,
                    frame_version: 0,
                }));
            }
            // Sélection abandonnée entre-temps : le lecteur est simplement libéré.
            Ok(_) => {}
            Err(message) if still_wanted => {
                self.player.slot = PlayerSlot::None;
                self.error = Some(format!(
                    "{message} Ouverture avec l'application par défaut."
                ));
                self.report_launch_keep_error(
                    self.storage
                        .open(&ItemId::path(&path))
                        .map_err(|e| e.to_string()),
                );
            }
            Err(_) => {}
        }
    }

    /// Met à jour la texture de l'image vidéo et demande les images suivantes pendant la lecture.
    fn update_player(&mut self, ctx: &egui::Context) {
        let PlayerSlot::Ready(active) = &mut self.player.slot else {
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
        self.sidebar.focus = false;
        // Choisir un autre élément arrête la lecture en cours.
        if !matches!(&self.player.slot, PlayerSlot::None) {
            let same = match &self.player.slot {
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
        self.preview.state = PreviewState::None;
        if !entry.is_dir && preview::is_previewable(&entry.path) {
            // Le décodage se fait hors du thread d'interface.
            let path = entry.path.clone();
            self.preview.state = PreviewState::Loading(path.clone());
            let tx = self.event_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = preview::load_preview(&path);
                let _ = tx.send(Event::Preview(path, result));
                ctx.request_repaint();
            });
        }
        self.nav.cursor = Some(entry.path.clone());
        self.selection.select_one(entry);
    }

    /// Traite les résultats des threads de travail ; ceux de l'autre volet (affichage
    /// fractionné) lui sont livrés.
    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            if self.event_for_other(&event) {
                self.with_other(|app| {
                    if app.handle_event(ctx, event) {
                        app.recompute_visible();
                    }
                });
            } else {
                changed |= self.handle_event(ctx, event);
            }
        }
        if changed {
            self.recompute_visible();
        }
    }

    /// Applique un résultat de fond ; `true` si la liste affichée doit être recalculée.
    fn handle_event(&mut self, ctx: &egui::Context, event: Event) -> bool {
        let mut changed = false;
        match event {
            Event::Listing { generation, result } => self.on_listing(generation, result),
            Event::ColumnListing {
                generation,
                path,
                entries,
            } => {
                if generation == self.nav.column_generation {
                    self.nav.column_requests.remove(&path);
                    let processed = self.filtered_and_sorted(&entries, false);
                    self.nav.column_cache.insert(path, processed);
                }
            }
            Event::Preview(path, result) => self.on_preview(ctx, path, result),
            Event::DriveInfo(path, info) => {
                self.sidebar.drive_infos.insert(path, info);
            }
            Event::FolderSize {
                generation,
                path,
                size,
            } => {
                // Ignore les résultats d'un listing déjà quitté.
                if generation != self.folder_generation.load(Ordering::SeqCst) {
                    return false;
                }
                if let Some(entry) = self.entries.iter_mut().find(|e| e.path == path) {
                    entry.size = Some(size);
                    changed = true;
                }
            }
            Event::Media {
                generation,
                path,
                info,
            } => {
                if generation == self.preview.media_generation.load(Ordering::SeqCst) {
                    self.preview.media.insert(path, info);
                }
            }
            Event::PlayerReady(path, result) => self.on_player_ready(path, result),
            Event::ScanBatch {
                generation,
                entries,
            } => {
                if generation == self.scan.generation.load(Ordering::SeqCst)
                    && self.scan.results.is_some()
                {
                    self.scan.result_entries.extend(entries);
                    changed = true;
                }
            }
            Event::ScanGroups { generation, groups } => {
                if generation == self.scan.generation.load(Ordering::SeqCst)
                    && self.scan.results.is_some()
                {
                    self.scan.duplicate_groups.clear();
                    self.scan.result_entries.clear();
                    for (index, group) in groups.into_iter().enumerate() {
                        for entry in group {
                            self.scan
                                .duplicate_groups
                                .insert(entry.path.clone(), index + 1);
                            self.scan.result_entries.push(entry);
                        }
                    }
                    changed = true;
                }
            }
            Event::CopyDone { dest, report, mode } => self.on_copy_done(&dest, report, mode),
            Event::ScanProgress { generation, text } => {
                if generation == self.scan.generation.load(Ordering::SeqCst)
                    && let Some(results) = &mut self.scan.results
                {
                    results.progress = Some(text);
                }
            }
            Event::ScanDone {
                generation,
                truncated,
            } => {
                if generation == self.scan.generation.load(Ordering::SeqCst)
                    && let Some(results) = &mut self.scan.results
                {
                    results.running = false;
                    results.truncated = truncated;
                    results.progress = None;
                    changed = true;
                }
            }
            Event::Thumb(path, thumb) => {
                self.preview.thumbs_in_flight = self.preview.thumbs_in_flight.saturating_sub(1);
                // Une miniature en échec reste dans `thumbs_requested` : on ne la redemande pas.
                if let Some(img) = thumb
                    && self.preview.thumbs_requested.contains(&path)
                {
                    let color = egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba);
                    let texture = ctx.load_texture(
                        format!("thumb:{}", path.to_string_lossy()),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    self.preview.thumbs.insert(path, texture);
                }
            }
        }
        changed
    }

    fn on_preview(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        result: Result<PreviewContent, String>,
    ) {
        {
            // Ignore les résultats d'une sélection déjà abandonnée.
            if !matches!(&self.preview.state, PreviewState::Loading(p) if *p == path) {
                return;
            }
            self.preview.state = match result {
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
            detail_central: self.detail_central,
            split_view: self.split.is_some(),
            skip_folder_sizes: self.skip_folder_sizes,
            show_drive_usage: self.show_drive_usage,
            recursive: self.scan.recursive,
            large_min_mb: self.scan.large_min_mb,
            view_mode: self.view_mode,
            show_hidden: self.show_hidden,
            columns: self.columns,
            favorites: self.favorites.paths().to_vec(),
            applications: self.applications.paths().to_vec(),
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

    /// Position, dans la liste affichée, de la ligne courante (curseur ou sélection).
    fn cursor_index(&self) -> Option<usize> {
        self.nav
            .cursor
            .as_ref()
            .or(self.selection.single.as_ref().map(|s| &s.path))
            .and_then(|path| self.nav_list().iter().position(|e| e.path == *path))
    }

    /// Le dossier affiché est dans le dossier de l'utilisateur (« Home »).
    fn in_home(&self) -> bool {
        match (&self.nav.location, &self.sidebar.home) {
            (Location::Dir(path), Some(home)) => path.starts_with(home),
            _ => false,
        }
    }

    /// Les colonnes « lecteurs » et « navigation » à gauche de la grille ne s'affichent que hors
    /// de Home : dans Home, le chemin et la barre latérale (onglet Home) suffisent.
    fn grid_side_columns(&self) -> bool {
        !self.in_home()
    }

    /// Le fichier figure dans la grille : seulement les images en mode « Images », seulement
    /// les vidéos en mode « Vidéos », jamais de dossier (ils sont dans la colonne de navigation
    /// ou, dans Home, dans la barre latérale). Hors grille, tout est affiché.
    fn in_grid(&self, entry: &Entry) -> bool {
        match self.view_mode {
            ViewMode::Gallery => !entry.is_dir && preview::is_supported_image(&entry.path),
            ViewMode::Videos => !entry.is_dir && preview::is_video(&entry.path),
            ViewMode::List | ViewMode::Columns => true,
        }
    }

    /// Éléments parcourus au clavier et par plage : ceux de la grille, ou tout le dossier.
    fn nav_list(&self) -> Vec<&Entry> {
        self.visible.iter().filter(|e| self.in_grid(e)).collect()
    }

    /// Le volet de détail est affiché (option « Volet d'aperçu », ou multi-sélection en cours).
    fn detail_shown(&self) -> bool {
        // La demande de suppression s'affiche dans le détail : il apparaît pour elle.
        self.show_preview || !self.selection.multi.is_empty() || self.confirm_delete
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Dossier de test jetable, dans `target/`.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-app")
                .join(name);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, rel: &str, content: &[u8]) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        fn dir(&self, rel: &str) {
            std::fs::create_dir_all(self.0.join(rel)).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Application sans fenêtre, positionnée sur `tree` (aucun réglage n'est écrit).
    fn app_in(tree: &TempTree) -> (ExpliaApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = ExpliaApp::new(ctx.clone(), Settings::default(), None);
        app.navigate(Location::Dir(tree.0.clone()));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
        (app, ctx)
    }

    /// Traite les événements jusqu'à la fin des lectures de dossiers et des copies en cours.
    fn settle(app: &mut ExpliaApp, ctx: &egui::Context) {
        let start = Instant::now();
        loop {
            app.poll_events(ctx);
            if !app.listing_pending() && !app.copying {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "lecture trop longue"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Traite les événements jusqu'à la fin du parcours en cours.
    fn wait_scan(app: &mut ExpliaApp, ctx: &egui::Context) {
        let start = Instant::now();
        loop {
            app.poll_events(ctx);
            if app.scan.results.as_ref().is_some_and(|r| !r.running) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "parcours trop long"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn names(app: &ExpliaApp) -> Vec<String> {
        let mut names: Vec<String> = app.visible.iter().map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn duplicate_scan_groups_copies_and_selects_all_but_the_first() {
        let tree = TempTree::new("dups");
        // Petits fichiers : les doublons sont cherchés quelle que soit leur taille.
        tree.file("a.bin", b"meme contenu");
        tree.file("sub/b.bin", b"meme contenu");
        tree.file("sub/c.bin", b"meme contenu");
        tree.file("autre.bin", b"autre contenu");
        // Les fichiers vides ne sont pas des doublons.
        tree.file("vide1.txt", b"");
        tree.file("vide2.txt", b"");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::FindDuplicates);
        settle(&mut app, &ctx);
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["a.bin", "b.bin", "c.bin"]);
        assert_eq!(app.scan.duplicate_groups.len(), 3);
        app.apply(&ctx, Action::SelectDuplicateCopies);
        settle(&mut app, &ctx);
        assert_eq!(app.selection.multi.len(), 2);
        // Le premier fichier du groupe (ordre des chemins) est conservé.
        assert!(app.selection.multi.iter().all(|e| e.name != "a.bin"));
    }

    #[test]
    fn large_file_scan_lists_files_from_the_biggest_and_keeps_that_order() {
        let tree = TempTree::new("large");
        tree.file("moyen.bin", &vec![1u8; 1_500_000]);
        tree.file("sub/gros.bin", &vec![1u8; 2_500_000]);
        tree.file("petit.bin", &vec![1u8; 500_000]);
        let (mut app, ctx) = app_in(&tree);
        app.scan.large_min_mb = 1;
        app.apply(&ctx, Action::FindLargeFiles);
        settle(&mut app, &ctx);
        wait_scan(&mut app, &ctx);
        let order: Vec<String> = app.visible.iter().map(|e| e.name.clone()).collect();
        assert_eq!(order, ["gros.bin", "moyen.bin"]);
        // Le tri par nom ne défait pas le classement par taille.
        app.sort_key = SortKey::Name;
        app.sort_order = SortOrder::Ascending;
        app.recompute_visible();
        assert_eq!(app.visible[0].name, "gros.bin");
        // Rien n'est sélectionné d'office (ce ne sont pas forcément des fichiers à supprimer).
        assert!(app.selection.multi.is_empty());
        // Sans récursivité : seulement les fichiers du dossier affiché.
        app.scan.recursive = false;
        app.on_recursive_changed();
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["moyen.bin"]);
    }

    #[test]
    fn recursive_search_goes_into_subfolders_and_plain_search_does_not() {
        let tree = TempTree::new("search");
        tree.file("racine.exe", b"x");
        tree.file("sous/dossier/outil.exe", b"x");
        tree.file("sous/note.txt", b"x");
        let (mut app, ctx) = app_in(&tree);
        app.scan.search = "*.exe".to_string();
        app.scan.recursive = true;
        app.on_search_changed();
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["outil.exe", "racine.exe"]);
        // Effacer la recherche ferme les résultats.
        app.scan.search.clear();
        app.on_search_changed();
        assert!(app.scan.results.is_none());
        // Non récursive : simple filtre du dossier affiché.
        app.scan.recursive = false;
        app.scan.search = "*.exe".to_string();
        app.on_search_changed();
        assert!(app.scan.results.is_none());
        assert_eq!(names(&app), ["racine.exe"]);
    }

    #[test]
    fn changing_folder_cancels_and_closes_results() {
        let tree = TempTree::new("navigate");
        tree.dir("sous");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::FindDuplicates);
        settle(&mut app, &ctx);
        assert!(app.scan.results.is_some());
        app.navigate(Location::Dir(tree.0.join("sous")));
        settle(&mut app, &ctx);
        assert!(app.scan.results.is_none());
        assert!(app.scan.result_entries.is_empty());
    }

    #[test]
    fn deleting_the_current_folder_needs_a_confirmation_that_can_be_cancelled() {
        let tree = TempTree::new("current");
        tree.dir("vide");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::AskDeleteFolder(tree.0.clone()));
        settle(&mut app, &ctx);
        assert!(app.confirm_delete);
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.path.clone()),
            Some(tree.0.clone())
        );
        assert_eq!(app.delete_targets().len(), 1);
        // Annuler retire la sélection provisoire du dossier affiché.
        app.apply(&ctx, Action::CancelDelete);
        settle(&mut app, &ctx);
        assert!(!app.confirm_delete);
        assert!(app.selection.single.is_none());
        // On ne renomme pas le dossier affiché depuis sa propre liste.
        let current = fs_nav::entry_from_path(&tree.0).unwrap();
        app.start_rename(&current);
        assert!(app.renaming.is_none());
    }

    #[test]
    fn a_parent_folder_of_a_result_can_be_targeted_then_cancelled() {
        let tree = TempTree::new("parent-of-result");
        let big = vec![9u8; 1_200_000];
        tree.file("a/b/copie1.bin", &big);
        tree.file("c/copie2.bin", &big);
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::FindDuplicates);
        settle(&mut app, &ctx);
        wait_scan(&mut app, &ctx);
        assert_eq!(app.visible.len(), 2);
        let first = app.visible[0].clone();
        app.apply(&ctx, Action::Select(first.clone()));
        settle(&mut app, &ctx);
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.path.clone()),
            Some(first.path.clone())
        );
        // Le dossier parent du résultat devient la cible provisoire de la suppression.
        let parent = first.path.parent().unwrap().to_path_buf();
        app.apply(&ctx, Action::AskDeleteFolder(parent.clone()));
        settle(&mut app, &ctx);
        assert!(app.confirm_delete);
        assert_eq!(app.delete_targets()[0].path, parent);
        app.apply(&ctx, Action::CancelDelete);
        settle(&mut app, &ctx);
        assert!(!app.confirm_delete && app.selection.single.is_none());
        // Un résultat sélectionné normalement reste sélectionné après un Échap sans demande.
        app.apply(&ctx, Action::Select(first.clone()));
        settle(&mut app, &ctx);
        app.cancel_delete();
        assert!(app.selection.single.is_some());
        // « Ouvrir » : on arrive dans le dossier du fichier, résultats fermés, fichier choisi.
        app.apply(
            &ctx,
            Action::GoSelect {
                dir: parent.clone(),
                file: first.path.clone(),
            },
        );
        settle(&mut app, &ctx);
        assert!(app.scan.results.is_none());
        assert_eq!(app.nav.location, Location::Dir(parent));
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.name.clone()),
            Some(first.name)
        );
    }

    #[test]
    fn full_screen_toggles_on_and_off() {
        let tree = TempTree::new("fullscreen");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::ToggleVideoFullscreen);
        settle(&mut app, &ctx);
        assert!(app.player.fullscreen);
        app.apply(&ctx, Action::ToggleVideoFullscreen);
        settle(&mut app, &ctx);
        assert!(!app.player.fullscreen);
    }

    #[test]
    fn dropped_items_are_copied_into_the_current_folder_without_overwriting() {
        let tree = TempTree::new("drop");
        tree.file("ailleurs/photo.jpg", b"img");
        tree.file("ici/photo.jpg", b"ancienne");
        let here = tree.0.join("ici");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(here.clone()));
        settle(&mut app, &ctx);
        app.apply(
            &ctx,
            Action::CopyInto {
                sources: vec![tree.0.join("ailleurs/photo.jpg")],
                dest: here.clone(),
            },
        );
        settle(&mut app, &ctx);
        let start = Instant::now();
        while app.copying {
            app.poll_events(&ctx);
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(here.join("photo.jpg")).unwrap(), b"ancienne");
        assert_eq!(
            std::fs::read(here.join("photo - Copie.jpg")).unwrap(),
            b"img"
        );
        // Le dossier est relu et la copie sélectionnée.
        assert_eq!(names(&app), ["photo - Copie.jpg", "photo.jpg"]);
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.name.clone()),
            Some("photo - Copie.jpg".to_string())
        );
    }

    #[test]
    fn cut_items_are_moved_without_overwriting_and_leave_the_folder() {
        let tree = TempTree::new("cut");
        tree.file("ici/a.txt", b"nouveau");
        tree.file("ici/garder.txt", b"g");
        tree.file("dest/a.txt", b"ancien");
        let here = tree.0.join("ici");
        let dest = tree.0.join("dest");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(here.clone()));
        settle(&mut app, &ctx);
        app.start_transfer(vec![here.join("a.txt")], dest.clone(), ClipMode::Cut);
        settle(&mut app, &ctx);
        // Le dossier affiché est relu : l'élément déplacé n'y est plus.
        assert_eq!(names(&app), ["garder.txt"]);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"ancien");
        assert_eq!(
            std::fs::read(dest.join("a - Copie.txt")).unwrap(),
            b"nouveau"
        );
        assert!(app.notice.as_deref().unwrap_or("").contains("déplacé"));
    }

    #[test]
    fn a_new_folder_is_created_selected_and_renamed_in_place() {
        let tree = TempTree::new("new-folder");
        tree.dir("existant");
        let (mut app, ctx) = app_in(&tree);
        app.apply(
            &ctx,
            Action::CreateFolder {
                parent: tree.0.clone(),
            },
        );
        settle(&mut app, &ctx);
        assert!(tree.0.join("Nouveau dossier").is_dir());
        assert_eq!(names(&app), ["Nouveau dossier", "existant"]);
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.name.clone()),
            Some("Nouveau dossier".to_string())
        );
        assert_eq!(
            app.renaming.as_ref().map(|r| r.path.clone()),
            Some(tree.0.join("Nouveau dossier"))
        );
        // Dans un sous-dossier (menu « Nouveau dossier » d'un dossier) : on y entre d'abord.
        app.renaming = None;
        app.apply(
            &ctx,
            Action::CreateFolder {
                parent: tree.0.join("existant"),
            },
        );
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("existant")));
        assert!(tree.0.join("existant/Nouveau dossier").is_dir());
    }

    #[test]
    fn folders_are_read_in_the_background_and_only_the_last_request_counts() {
        let tree = TempTree::new("async-open");
        tree.dir("a");
        tree.dir("b");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(tree.0.join("a")));
        // L'emplacement ne change qu'une fois le dossier lu.
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
        assert!(app.listing_pending());
        // Une demande plus récente l'emporte sur la précédente.
        app.navigate(Location::Dir(tree.0.join("b")));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
        // Dossier illisible : on reste où l'on est, avec un message.
        app.navigate(Location::Dir(tree.0.join("absent")));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
        assert!(
            app.error
                .as_deref()
                .is_some_and(|e| e.starts_with("Impossible d'ouvrir"))
        );
    }

    /// Dossier affiché par l'autre volet.
    fn other_location(app: &mut ExpliaApp) -> Location {
        app.with_other(|other| other.nav.location.clone())
    }

    #[test]
    fn the_split_view_shows_two_independent_panes() {
        let tree = TempTree::new("split");
        tree.file("a/x.txt", b"x");
        tree.dir("b");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::ToggleSplit);
        settle(&mut app, &ctx);
        // L'autre volet s'ouvre sur le même dossier.
        assert_eq!(other_location(&mut app), Location::Dir(tree.0.clone()));
        // Chaque volet navigue de son côté ; les lectures de fond retrouvent leur volet.
        app.navigate(Location::Dir(tree.0.join("a")));
        app.with_other(|other| other.navigate(Location::Dir(tree.0.join("b"))));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("a")));
        assert_eq!(other_location(&mut app), Location::Dir(tree.0.join("b")));
        // Tab (ou un clic) donne la main à l'autre volet ; la sélection du volet quitté
        // disparaît : une seule sélection dans la fenêtre.
        let first = app.visible[0].clone();
        app.apply(&ctx, Action::Select(first));
        assert!(app.selection.single.is_some());
        app.apply(&ctx, Action::SwitchPane);
        assert!(app.with_other(|other| other.selection.single.is_none()));
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
        assert_eq!(other_location(&mut app), Location::Dir(tree.0.join("a")));
        // Colonnes, images et vidéos ferment l'affichage fractionné.
        app.apply(&ctx, Action::SetViewMode(ViewMode::Columns));
        assert!(app.split.is_none());
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
    }

    #[test]
    fn closing_a_pane_keeps_the_other_one() {
        let tree = TempTree::new("split-close");
        tree.dir("a");
        tree.dir("b");
        let (mut app, ctx) = app_in(&tree);
        let open = |app: &mut ExpliaApp| {
            app.apply(&ctx, Action::ToggleSplit);
            app.navigate(Location::Dir(tree.0.join("a")));
            app.with_other(|other| other.navigate(Location::Dir(tree.0.join("b"))));
            settle(app, &ctx);
        };
        // La croix du volet actif : l'autre volet reste, seul.
        open(&mut app);
        app.apply(&ctx, Action::ClosePane { active: true });
        assert!(app.split.is_none());
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
        // La croix de l'autre volet : le volet actif reste.
        open(&mut app);
        app.apply(&ctx, Action::ClosePane { active: false });
        assert!(app.split.is_none());
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("a")));
    }

    #[test]
    fn f5_copies_and_f6_moves_to_the_other_pane() {
        let tree = TempTree::new("split-transfer");
        tree.file("gauche/photo.jpg", b"img");
        tree.file("gauche/doc.txt", b"doc");
        tree.dir("droite");
        let left = tree.0.join("gauche");
        let right = tree.0.join("droite");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(left.clone()));
        settle(&mut app, &ctx);
        app.apply(&ctx, Action::ToggleSplit);
        app.with_other(|other| other.navigate(Location::Dir(right.clone())));
        settle(&mut app, &ctx);
        // Copie de la sélection (F5) : l'autre volet est relu et montre l'élément arrivé.
        let photo = app.visible.iter().find(|e| e.name == "photo.jpg").cloned();
        app.apply(&ctx, Action::Select(photo.unwrap()));
        app.apply(
            &ctx,
            Action::TransferToOther {
                mode: ClipMode::Copy,
            },
        );
        settle(&mut app, &ctx);
        assert!(left.join("photo.jpg").is_file() && right.join("photo.jpg").is_file());
        let other_names = app.with_other(|other| names(other));
        assert_eq!(other_names, ["photo.jpg"]);
        // Déplacement (F6, ou glisser avec Maj) : l'élément quitte ce volet.
        app.apply(
            &ctx,
            Action::TransferEntries {
                sources: vec![left.join("doc.txt")],
                mode: ClipMode::Cut,
            },
        );
        settle(&mut app, &ctx);
        assert!(!left.join("doc.txt").exists() && right.join("doc.txt").is_file());
        assert_eq!(names(&app), ["photo.jpg"]);
        let other_names = app.with_other(|other| names(other));
        assert_eq!(other_names, ["doc.txt", "photo.jpg"]);
    }

    /// Fichier déposé dans la fenêtre, pour les tests.
    #[derive(Debug)]
    struct TestDrop(PathBuf);

    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    /// Une image avec ces entrées brutes ; renvoie l'action décidée par `input_events`.
    fn frame_input(
        app: &mut ExpliaApp,
        ctx: &egui::Context,
        raw: egui::RawInput,
    ) -> Option<Action> {
        ctx.begin_pass(raw);
        let action = app.input_events(ctx);
        // Pas de rendu dans les tests : les textures préparées sont simplement oubliées.
        ctx.end_pass().textures_delta.clear();
        action
    }

    #[test]
    fn files_dropped_from_explorer_are_not_lost() {
        let tree = TempTree::new("drop-input");
        let source = tree.0.join("ailleurs").join("a.txt");
        tree.file("ailleurs/a.txt", b"a");
        tree.dir("ici");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(tree.0.join("ici")));
        settle(&mut app, &ctx);
        let raw = egui::RawInput {
            dropped_files: vec![std::sync::Arc::new(TestDrop(source))],
            ..Default::default()
        };
        // Autrefois, la gestion des flèches effaçait l'action (Coller, dépôt…) déjà décidée.
        let action = frame_input(&mut app, &ctx, raw);
        assert!(matches!(action, Some(Action::CopyInto { .. })));
    }

    #[test]
    fn dropping_dragged_items_on_a_folder_moves_them_there() {
        let tree = TempTree::new("drag-drop");
        tree.file("photo.jpg", b"img");
        tree.dir("Archives");
        let (mut app, ctx) = app_in(&tree);
        let photo = tree.0.join("photo.jpg");
        let archives = tree.0.join("Archives");
        app.apply(&ctx, Action::DragStart(vec![photo.clone()]));
        // Le dossier sous le pointeur, repéré pendant le dessin, devient la cible.
        assert!(app.offer_drop_target(&archives));
        let position = egui::pos2(20.0, 20.0);
        let raw = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let action = frame_input(&mut app, &ctx, raw);
        assert!(matches!(
            &action,
            Some(Action::TransferInto { dest, mode: ClipMode::Cut, .. }) if *dest == archives
        ));
        app.apply(&ctx, action.unwrap());
        settle(&mut app, &ctx);
        assert!(archives.join("photo.jpg").is_file() && !photo.exists());
        assert_eq!(names(&app), ["Archives"]);
        assert!(app.drag.is_none());
    }

    #[test]
    fn pressing_and_moving_on_a_row_starts_a_drag() {
        let tree = TempTree::new("row-drag");
        tree.file("photo.jpg", b"img");
        let (app, ctx) = app_in(&tree);
        let entry = app.visible[0].clone();
        let start = egui::pos2(30.0, 10.0);
        // Image 1 : appui sur la ligne ; image 2 : le pointeur a bougé de 40 points.
        let frames = [
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(start + egui::vec2(40.0, 20.0))],
        ];
        let mut action = None;
        for events in frames {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(raw, |ui| {
                let response = ui::table_row(ui, false, false, "📄 photo.jpg", &[], &[]);
                if let Some(gesture) = app.item_events(ui, &response, &entry, ItemView::List) {
                    action = Some(gesture);
                }
            });
            // Pas de rendu dans les tests : les textures préparées sont simplement oubliées.
            let mut output = output;
            output.textures_delta.clear();
        }
        // La ligne se prend à la souris : le glisser commence (sans défiler la liste).
        assert!(matches!(&action, Some(Action::DragStart(paths)) if *paths == [entry.path]));
    }

    #[test]
    fn tabs_open_switch_and_close() {
        let tree = TempTree::new("tabs");
        tree.dir("a");
        tree.dir("b");
        let (mut app, ctx) = app_in(&tree);
        assert!(app.nav.tabs.is_empty());
        app.apply(&ctx, Action::NewTab(tree.0.join("a")));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.tabs, [tree.0.clone(), tree.0.join("a")]);
        assert_eq!(app.nav.active_tab, 1);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("a")));
        // Naviguer change le dossier de l'onglet affiché.
        app.navigate(Location::Dir(tree.0.join("b")));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.tabs[1], tree.0.join("b"));
        app.apply(&ctx, Action::SwitchTab(0));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
        // Fermer l'onglet affiché passe au voisin ; avec un seul dossier, plus d'onglets.
        app.apply(&ctx, Action::CloseTab(0));
        settle(&mut app, &ctx);
        assert!(app.nav.tabs.is_empty());
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("b")));
        // Trois onglets : fermer un onglet non affiché garde le dossier affiché.
        app.apply(&ctx, Action::NewTab(tree.0.join("a")));
        settle(&mut app, &ctx);
        app.apply(&ctx, Action::NewTab(tree.0.clone()));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.tabs.len(), 3);
        app.apply(&ctx, Action::CloseTab(0));
        settle(&mut app, &ctx);
        assert_eq!(app.nav.tabs.len(), 2);
        assert_eq!(app.nav.active_tab, 1);
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
    }

    #[test]
    fn arrows_walk_through_the_home_list_after_a_click_in_it() {
        let tree = TempTree::new("home-arrows");
        tree.dir("Apple");
        tree.dir("build");
        tree.dir("Contacts");
        let (mut app, ctx) = app_in(&tree);
        app.sidebar.home = Some(tree.0.clone());
        app.set_sidebar_tab(SidebarTab::Home);
        assert_eq!(
            app.sidebar_items(),
            [
                tree.0.clone(),
                tree.0.join("Apple"),
                tree.0.join("build"),
                tree.0.join("Contacts")
            ]
        );
        app.apply(&ctx, Action::SidebarGo(tree.0.join("build")));
        settle(&mut app, &ctx);
        assert!(app.sidebar.focus);
        app.step_sidebar(1);
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("Contacts")));
        app.step_sidebar(-1);
        settle(&mut app, &ctx);
        app.step_sidebar(-1);
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.join("Apple")));
        // Le titre (le dossier de l'utilisateur) est le premier élément.
        app.step_sidebar(-1);
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
        assert_eq!(app.sidebar_cursor_path(), Some(tree.0.clone()));
        // Un clic dans la zone centrale ne donne pas la main à la barre latérale.
        app.apply(&ctx, Action::Go(Location::Dir(tree.0.join("Apple"))));
        settle(&mut app, &ctx);
        assert!(!app.sidebar.focus);
    }

    #[test]
    fn ctrl_click_in_a_previous_column_starts_from_its_highlighted_folder() {
        let tree = TempTree::new("column-multi");
        tree.file("grec/doc.txt", b"x");
        tree.dir("aa");
        tree.dir("models");
        let (mut app, ctx) = app_in(&tree);
        // « grec » est ouvert : sa colonne parente le montre en surbrillance.
        app.navigate(Location::Dir(tree.0.join("grec")));
        settle(&mut app, &ctx);
        let models = fs_nav::entry_from_path(&tree.0.join("models")).unwrap();
        app.apply(
            &ctx,
            Action::ColumnMultiSelect {
                dir: tree.0.clone(),
                anchor: Some(tree.0.join("grec")),
                entry: models,
                range: false,
            },
        );
        settle(&mut app, &ctx);
        assert_eq!(app.nav.location, Location::Dir(tree.0.clone()));
        let mut selected: Vec<String> =
            app.selection.multi.iter().map(|e| e.name.clone()).collect();
        selected.sort();
        assert_eq!(selected, ["grec", "models"]);
        // Maj+clic : la plage depuis l'élément en surbrillance.
        app.navigate(Location::Dir(tree.0.join("grec")));
        settle(&mut app, &ctx);
        let aa = fs_nav::entry_from_path(&tree.0.join("aa")).unwrap();
        app.apply(
            &ctx,
            Action::ColumnMultiSelect {
                dir: tree.0.clone(),
                anchor: Some(tree.0.join("grec")),
                entry: aa,
                range: true,
            },
        );
        settle(&mut app, &ctx);
        let mut range: Vec<String> = app.selection.multi.iter().map(|e| e.name.clone()).collect();
        range.sort();
        assert_eq!(range, ["aa", "grec"]);
    }
}
