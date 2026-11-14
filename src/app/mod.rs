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
use crate::copy::CopyReport;
use crate::dates;
use crate::dragdrop;
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
use crate::storage_manager::StorageManager;
use crate::view::{self, Columns, Side, SortKey, SortOrder, ViewMode};

mod action_manager;
mod context_menu_manager;
mod event_manager;

use action_manager::{Action, Command};
use event_manager::ItemView;

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
/// Place réservée à la case « Récursive », à gauche de la barre de recherche.
const RECURSIVE_WIDTH: f32 = 110.0;
/// Libellé du filtre « Sélection », à droite de la barre de recherche (mesuré pour sa place).
const SELECTION_MENU_LABEL: &str = "⏷ Sélection";
/// Libellés des boutons de la barre du volet de détail (mesurés pour sa largeur minimale).
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
    /// Dossiers sans aucun fichier.
    EmptyFolders,
    /// Fichiers de même contenu.
    Duplicates,
    /// Fichiers plus gros que le seuil, du plus gros au plus petit.
    LargeFiles,
}

/// Résultats d'un parcours (recherche récursive, dossiers vides, doublons), affichés dans la
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
        report: CopyReport,
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
    /// Disposition : côté des onglets de la barre latérale et du volet de détail.
    tabs_side: Side,
    preview_side: Side,
    /// Onglet affiché dans la barre latérale.
    sidebar_tab: SidebarTab,
    /// Le clavier pilote la liste des lecteurs : ↑ / ↓ changent de lecteur, → revient au dossier.
    sidebar_focus: bool,
    /// Lecteur choisi au clavier (index dans `drives`).
    sidebar_cursor: Option<usize>,
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
    /// Option « Récursive » : recherche, dossiers vides et doublons parcourent les sous-dossiers.
    recursive: bool,
    /// Taille minimale (Mo) des fichiers examinés pour les doublons.
    duplicate_min_mb: u32,
    /// Taille minimale (Mo) des fichiers listés par « Fichiers > N Mo ».
    large_min_mb: u32,
    /// Résultats affichés à la place du contenu du dossier.
    results: Option<Results>,
    /// Éléments trouvés par le parcours en cours ou terminé.
    result_entries: Vec<Entry>,
    /// Numéro de groupe (à partir de 1) de chaque fichier en doublon.
    duplicate_groups: HashMap<PathBuf, usize>,
    /// Identifie le parcours courant ; l'incrémenter l'annule.
    scan_generation: Arc<AtomicU64>,
    /// Recherche (barre au-dessus de la liste) : motifs de noms, voir `view::matches_query`.
    search: String,
    /// Donne le focus à la barre de recherche à la prochaine image (Ctrl+F).
    focus_search: bool,
    show_preview: bool,
    /// Volet d'aperçu agrandi (flèche « < » / « > »).
    /// Volet de détail au centre et navigation dans le volet latéral (triangle d'échange).
    detail_central: bool,
    /// Vidéo en plein écran (double-clic sur la vidéo du volet de détail).
    video_fullscreen: bool,
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
    /// Élément sélectionné, multi-sélection et point d'ancrage (voir `selection_manager`).
    selection: Selection,
    /// Sources de stockage : toutes les opérations sur les fichiers passent par elles.
    storage: StorageManager,
    /// Copier / Couper : références placées dans le presse-papiers de Windows.
    clipboard: ClipboardManager,
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
    /// Onglets : dossier de chaque onglet ; vide tant qu'un seul dossier est ouvert.
    tabs: Vec<PathBuf>,
    /// Onglet affiché.
    active_tab: usize,
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
            tabs_side: settings.tabs_side,
            preview_side: settings.preview_side,
            sidebar_tab: SidebarTab::Pc,
            sidebar_focus: false,
            sidebar_cursor: None,
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
            recursive: settings.recursive,
            duplicate_min_mb: settings.duplicate_min_mb,
            large_min_mb: settings.large_min_mb,
            results: None,
            result_entries: Vec::new(),
            duplicate_groups: HashMap::new(),
            scan_generation: Arc::new(AtomicU64::new(0)),
            focus_search: false,
            show_preview: settings.show_preview,
            detail_central: settings.detail_central,
            video_fullscreen: false,
            skip_folder_sizes: settings.skip_folder_sizes,
            view_mode: settings.view_mode,
            show_hidden: settings.show_hidden,
            columns: settings.columns,
            media: HashMap::new(),
            media_generation: Arc::new(AtomicU64::new(0)),
            column_cache: HashMap::new(),
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
            selection: Selection::default(),
            storage: StorageManager::default(),
            clipboard: ClipboardManager::default(),
            confirm_delete: false,
            long_press_done: false,
            drag_out_done: false,
            drag_out_ended_at: None,
            paste_keys_were_down: false,
            copying: false,
            tabs: Vec::new(),
            active_tab: 0,
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
            .and_then(|home| self.storage.list(home).ok())
            .map(|entries| entries.into_iter().filter(|e| e.is_dir).collect())
            .unwrap_or_default();
    }

    /// Affiche un onglet du classeur ; son contenu est relu (lecteurs, dossiers).
    fn set_sidebar_tab(&mut self, tab: SidebarTab) {
        self.sidebar_tab = tab;
        self.sidebar_focus = false;
        self.sidebar_cursor = None;
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
        self.column_cache.clear();
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
        if let Some(results) = &self.results {
            let mut list: Vec<Entry> = self
                .result_entries
                .iter()
                // La recherche de la barre filtre aussi les autres résultats (hors recherche récursive).
                .filter(|e| {
                    results.kind == ResultKind::Search || view::matches_query(&e.name, &self.search)
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
            let report = match mode {
                ClipMode::Copy => storage.copy_into(&sources, &dest),
                ClipMode::Cut => storage.move_into(&sources, &dest),
            };
            let _ = tx.send(Event::CopyDone { dest, report, mode });
            ctx.request_repaint();
        });
    }

    /// Fin d'une copie ou d'un déplacement : bilan dans la barre d'état, dossier relu.
    fn on_copy_done(&mut self, dest: &Path, report: CopyReport, mode: ClipMode) {
        self.copying = false;
        if mode == ClipMode::Cut && !report.copied.is_empty() && report.errors.is_empty() {
            self.clipboard.moved();
        }
        // Après un déplacement, des éléments ont quitté le dossier affiché.
        if mode == ClipMode::Cut || self.location == Location::Dir(dest.to_path_buf()) {
            self.refresh_listing();
        }
        if self.location == Location::Dir(dest.to_path_buf()) {
            // Les éléments copiés sont sélectionnés (un seul type à la fois).
            if let Some(first) = report.copied.first()
                && let Some(entry) = self.visible.iter().find(|e| e.path == *first).cloned()
            {
                let ctx = self.ctx.clone();
                self.select(&ctx, entry);
            }
        }
        let done = format!(
            "{} élément(s) {}",
            report.copied.len(),
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

    /// Ferme un onglet ; s'il était affiché, on passe à son voisin.
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let was_active = index == self.active_tab;
        self.tabs.remove(index);
        if index < self.active_tab {
            self.active_tab -= 1;
        }
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        let next = self.tabs.get(self.active_tab).cloned();
        if self.tabs.len() <= 1 {
            // Un seul dossier ouvert : plus de barre d'onglets.
            self.tabs.clear();
            self.active_tab = 0;
        }
        if was_active && let Some(path) = next {
            self.navigate(Location::Dir(path));
        }
    }

    /// Annule le parcours en cours et ferme les résultats.
    fn close_results(&mut self) {
        self.scan_generation.fetch_add(1, Ordering::SeqCst);
        self.results = None;
        self.result_entries.clear();
        self.duplicate_groups.clear();
    }

    /// Lance un parcours en arrière-plan sur le dossier affiché ; les résultats remplacent le
    /// contenu du dossier dans la liste. Un parcours précédent est annulé.
    fn start_scan(&mut self, kind: ResultKind) {
        let Location::Dir(root) = self.location.clone() else {
            return;
        };
        let generation = self.scan_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.results = Some(Results {
            kind,
            running: true,
            truncated: false,
            progress: None,
        });
        self.result_entries.clear();
        self.duplicate_groups.clear();
        self.clear_multi();
        self.selection.single = None;
        self.preview = PreviewState::None;
        self.error = None;
        self.notice = None;
        self.recompute_visible();
        let query = self.search.clone();
        let recursive = self.recursive;
        let show_hidden = self.show_hidden;
        let min_bytes = u64::from(self.duplicate_min_mb) * 1024 * 1024;
        let large_min_bytes = u64::from(self.large_min_mb) * 1024 * 1024;
        let current = Arc::clone(&self.scan_generation);
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
                ResultKind::EmptyFolders => {
                    let folders = scan::find_empty_folders(&root, recursive, &cancelled);
                    let entries: Vec<Entry> = folders
                        .iter()
                        .filter_map(|path| fs_nav::entry_from_path(path))
                        .collect();
                    send(Event::ScanBatch {
                        generation,
                        entries,
                    });
                }
                ResultKind::Duplicates => {
                    let found = scan::find_duplicates(
                        &root,
                        recursive,
                        min_bytes,
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
        let searching = self.recursive
            && !self.search.trim().is_empty()
            && matches!(self.location, Location::Dir(_));
        if searching {
            self.start_scan(ResultKind::Search);
            return;
        }
        if self
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
        if let Some(kind) = self.results.as_ref().map(|r| r.kind) {
            if kind == ResultKind::Search {
                self.on_search_changed();
            } else {
                self.start_scan(kind);
            }
        } else if self.recursive && !self.search.trim().is_empty() {
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
            self.preview = PreviewState::None;
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
                self.sidebar_focus = true;
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
        match self.storage.list(&path) {
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
        self.sidebar_focus = false;
        self.sidebar_cursor = None;
        // La recherche et les résultats valent pour le dossier affiché : un nouveau dossier
        // repart sans filtre.
        self.close_results();
        self.search.clear();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.clone_from(&path);
        }
        self.address = target.display();
        self.location = target;
        self.selection.single = None;
        self.clear_multi();
        self.selection.anchor = None;
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
            self.selection.anchor = Some(previous);
            self.selection.single = Some(self.visible[index].clone());
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

    /// Dossiers de la liste de l'onglet actif, dans l'ordre affiché (navigation aux flèches).
    fn sidebar_items(&self) -> Vec<PathBuf> {
        match self.sidebar_tab {
            SidebarTab::Pc => self.drives.iter().map(|d| d.path.clone()).collect(),
            SidebarTab::Network => self
                .drives
                .iter()
                .filter(|d| d.kind == DriveKind::Network)
                .map(|d| d.path.clone())
                .collect(),
            SidebarTab::Home => self
                .home
                .iter()
                .cloned()
                .chain(
                    self.home_dirs
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
        if !self.sidebar_focus {
            return None;
        }
        self.sidebar_cursor
            .and_then(|index| self.sidebar_items().get(index).cloned())
    }

    /// Passe à l'élément précédent (`delta` < 0) ou suivant (`delta` > 0) de la liste de la barre
    /// latérale (lecteurs, dossiers de Home, favoris) et l'ouvre ; le clavier garde la main.
    fn step_sidebar(&mut self, delta: i32) {
        let items = self.sidebar_items();
        let current = self.sidebar_cursor.or(match &self.location {
            Location::Dir(path) => items.iter().position(|p| p == path).or_else(|| {
                // Lecteurs : le dossier affiché peut être n'importe où sur le lecteur.
                matches!(self.sidebar_tab, SidebarTab::Pc | SidebarTab::Network)
                    .then(|| items.iter().position(|root| path.starts_with(root)))
                    .flatten()
            }),
            Location::ThisPc => None,
        });
        let Some(next) = view::step_index(current, items.len(), delta) else {
            return;
        };
        self.navigate(Location::Dir(items[next].clone()));
        self.sidebar_focus = true;
        // Même si le dossier n'est pas accessible (erreur), le curseur avance : on peut passer
        // au suivant.
        self.sidebar_cursor = Some(next);
    }

    /// Maj+clic : sélectionne la plage entre l'ancre (dernier élément choisi) et `entry`.
    fn select_range(&mut self, entry: Entry) {
        let list: Vec<Entry> = self.nav_list().into_iter().cloned().collect();
        match self.selection.select_range(&list, &entry.path) {
            Ok(()) => {
                self.confirm_delete = false;
                self.preview = PreviewState::None;
                self.error = None;
                self.cursor = Some(entry.path.clone());
            }
            Err(SelectionError::MixedTypes) => {
                self.error = Some(MIXED_TYPES_MESSAGE.to_string());
            }
            // Rien de choisi avant, ou ancre hors de la liste : Maj+clic démarre la sélection.
            Err(SelectionError::NotFound) => self.toggle_multi(entry),
        }
    }

    /// Relit le dossier affiché (après une suppression, par exemple).
    fn refresh_listing(&mut self) {
        // Parmi les résultats, on retire ce qui n'existe plus (supprimé, renommé).
        self.result_entries.retain(|e| e.path.exists());
        let Location::Dir(path) = &self.location else {
            return;
        };
        match self.storage.list(path) {
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
        self.sidebar_focus = false;
        self.confirm_delete = false;
        let path = entry.path.clone();
        match self.selection.toggle(entry) {
            Ok(Toggle::Added) => {
                self.preview = PreviewState::None;
                self.error = None;
                self.cursor = Some(path);
            }
            Ok(Toggle::Removed | Toggle::Deselected) => self.preview = PreviewState::None,
            Err(_) => {
                if self.selection.single.is_none() {
                    self.preview = PreviewState::None;
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
        let paths: Vec<PathBuf> = self.delete_targets().into_iter().map(|e| e.path).collect();
        let count = paths.len();
        self.confirm_delete = false;
        match self.storage.delete(&paths).map_err(|e| e.to_string()) {
            Ok(()) => {
                self.error = None;
                self.notice = Some(format!("{count} élément(s) envoyé(s) à la corbeille."));
                // Le dossier affiché a lui-même été supprimé : on remonte au dossier parent.
                if let Location::Dir(current) = self.location.clone()
                    && !current.exists()
                    && let Some(parent) = current.parent()
                {
                    let notice = self.notice.take();
                    self.navigate(Location::Dir(parent.to_path_buf()));
                    self.notice = notice;
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
        self.preview = PreviewState::None;
        self.refresh_listing();
    }

    /// Crée un dossier dans `parent` (qui devient le dossier affiché), le sélectionne et le
    /// passe en renommage, comme l'Explorateur.
    fn create_folder(&mut self, ctx: &egui::Context, parent: PathBuf) {
        if self.location != Location::Dir(parent.clone()) {
            self.navigate(Location::Dir(parent.clone()));
            if self.location != Location::Dir(parent.clone()) {
                return;
            }
        }
        if self.results.is_some() {
            self.close_results();
        }
        match self.storage.create_folder(&parent) {
            Ok(path) => {
                self.refresh_listing();
                self.error = None;
                self.notice = Some(format!("Dossier créé : « {} ».", favorite_name(&path)));
                if let Some(entry) = self.visible.iter().find(|e| e.path == path).cloned() {
                    self.select(ctx, entry.clone());
                    self.scroll_to_index = self.cursor_index();
                    self.start_rename(&entry);
                }
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    /// Démarre le renommage de `entry` directement dans la liste.
    fn start_rename(&mut self, entry: &Entry) {
        // Seul un élément de la liste affichée se renomme (pas le dossier courant lui-même).
        if !matches!(self.location, Location::Dir(_))
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
    fn commit_rename(&mut self, ctx: &egui::Context, path: PathBuf, new_name: &str) {
        self.stop_player();
        match self
            .storage
            .rename(&path, new_name)
            .map_err(|e| e.to_string())
        {
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

    /// Entre en plein écran (fenêtre en plein écran, vidéo seule) ou en sort.
    fn set_video_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        if self.video_fullscreen != on {
            self.video_fullscreen = on;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        }
    }

    /// Plein écran : la vidéo occupe toute la fenêtre, les commandes en bas. Double-clic ou
    /// Échap en sort. Renvoie `false` s'il n'y a plus de vidéo à montrer (plein écran quitté).
    fn fullscreen_video(&mut self, ui: &mut egui::Ui) -> bool {
        let ctx = ui.ctx().clone();
        let PlayerSlot::Ready(active) = &self.player else {
            self.set_video_fullscreen(&ctx, false);
            return false;
        };
        let Some(texture) = &active.texture else {
            self.set_video_fullscreen(&ctx, false);
            return false;
        };
        let mut action = None;
        let mut exit = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Panel::bottom("fullscreen-controls").show(ui, |ui| {
            ui.add_space(4.0);
            action = Self::player_bar(ui, active);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let size = texture.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y);
                let rect = egui::Rect::from_center_size(area.center(), size * scale);
                let response = ui.allocate_rect(area, egui::Sense::click());
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                if response
                    .on_hover_text("Double-clic ou Échap : quitter le plein écran")
                    .double_clicked()
                {
                    exit = true;
                }
            });
        if exit {
            self.set_video_fullscreen(&ctx, false);
        }
        if let Some(action) = action {
            if matches!(action, Action::StopPlayer) {
                self.set_video_fullscreen(&ctx, false);
            }
            self.apply(&ctx, action);
        }
        true
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
            self.report_launch_keep_error(self.storage.open(&path).map_err(|e| e.to_string()));
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
                self.report_launch_keep_error(self.storage.open(&path).map_err(|e| e.to_string()));
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
        self.sidebar_focus = false;
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
        self.cursor = Some(entry.path.clone());
        self.selection.select_one(entry);
    }

    /// Traite les résultats des threads de travail.
    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut sizes_changed = false;
        let mut results_changed = false;
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
                Event::ScanBatch {
                    generation,
                    entries,
                } => {
                    if generation == self.scan_generation.load(Ordering::SeqCst)
                        && self.results.is_some()
                    {
                        self.result_entries.extend(entries);
                        results_changed = true;
                    }
                }
                Event::ScanGroups { generation, groups } => {
                    if generation == self.scan_generation.load(Ordering::SeqCst)
                        && self.results.is_some()
                    {
                        self.duplicate_groups.clear();
                        self.result_entries.clear();
                        for (index, group) in groups.into_iter().enumerate() {
                            for entry in group {
                                self.duplicate_groups.insert(entry.path.clone(), index + 1);
                                self.result_entries.push(entry);
                            }
                        }
                        results_changed = true;
                    }
                }
                Event::CopyDone { dest, report, mode } => self.on_copy_done(&dest, report, mode),
                Event::ScanProgress { generation, text } => {
                    if generation == self.scan_generation.load(Ordering::SeqCst)
                        && let Some(results) = &mut self.results
                    {
                        results.progress = Some(text);
                    }
                }
                Event::ScanDone {
                    generation,
                    truncated,
                } => {
                    if generation == self.scan_generation.load(Ordering::SeqCst)
                        && let Some(results) = &mut self.results
                    {
                        results.running = false;
                        results.truncated = truncated;
                        results.progress = None;
                        // Dossiers vides : tous sont sélectionnés, il ne reste qu'à supprimer.
                        if results.kind == ResultKind::EmptyFolders {
                            self.recompute_visible();
                            self.selection.multi = self.visible.clone();
                            self.selection.anchor =
                                self.selection.multi.first().map(|e| e.path.clone());
                        }
                        results_changed = true;
                    }
                }
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
        if sizes_changed || results_changed {
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
            skip_folder_sizes: self.skip_folder_sizes,
            show_drive_usage: self.show_drive_usage,
            recursive: self.recursive,
            duplicate_min_mb: self.duplicate_min_mb,
            large_min_mb: self.large_min_mb,
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

    /// Pendant le survol de fichiers venus d'une autre application : voile et consigne.
    fn drop_hint(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering
            && !self.drag_out_done
            && let Location::Dir(current) = &self.location
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drop-hint"),
            ));
            let screen = ctx.content_rect();
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                format!("Déposer pour copier dans « {} »", favorite_name(current)),
                egui::FontId::proportional(22.0),
                egui::Color32::WHITE,
            );
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

    /// Bandeau des résultats d'un parcours : titre, avancement, actions et fermeture.
    fn results_banner(&self, ui: &mut egui::Ui) -> Option<Action> {
        let results = self.results.as_ref()?;
        let mut action = None;
        let count = self.visible.len();
        let title = match results.kind {
            ResultKind::Search => {
                format!("Recherche « {} » : {count} résultat(s)", self.search.trim())
            }
            ResultKind::EmptyFolders => format!("Dossiers vides : {count}"),
            ResultKind::Duplicates => {
                let groups = self.duplicate_groups.values().copied().max().unwrap_or(0);
                format!(
                    "Doublons > {} Mo : {groups} groupe(s), {count} fichier(s)",
                    self.duplicate_min_mb
                )
            }
            ResultKind::LargeFiles => {
                let total: u64 = self.visible.iter().filter_map(|e| e.size).sum();
                format!(
                    "Fichiers > {} Mo : {count} fichier(s), {} au total",
                    self.large_min_mb,
                    fs_nav::format_size(total)
                )
            }
        };
        let scope = if self.recursive {
            "sous-dossiers inclus"
        } else {
            "dossier courant seulement"
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong(title);
                ui.weak(format!("({scope})"));
                if results.running {
                    ui.spinner();
                    ui.weak(results.progress.as_deref().unwrap_or("Analyse en cours…"));
                }
                if results.truncated {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("limité à {} résultats", scan::SEARCH_LIMIT),
                    );
                }
                if results.kind == ResultKind::Duplicates
                    && !results.running
                    && count > 0
                    && ui
                        .button("Sélectionner les copies")
                        .on_hover_text("Sélectionne tout sauf le premier fichier de chaque groupe")
                        .clicked()
                {
                    action = Some(Action::SelectDuplicateCopies);
                }
                if ui.button("✕ Fermer").clicked() {
                    action = Some(Action::CloseResults);
                }
            });
        });
        action
    }

    /// Haut de la zone centrale : fil d'Ariane (ou saisie du chemin) et barre de recherche.
    fn location_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            enlarge_toolbar(ui);
            let spacing = ui.spacing().item_spacing.x;
            let search_width = SEARCH_WIDTH.min(ui.available_width() * 0.4);
            // La case « Récursive » prend sa place à gauche de la barre de recherche.
            let recursive_width = RECURSIVE_WIDTH;
            let selection_width = {
                let font = egui::TextStyle::Button.resolve(ui.style());
                let galley = ui.painter().layout_no_wrap(
                    SELECTION_MENU_LABEL.to_string(),
                    font,
                    egui::Color32::WHITE,
                );
                galley.size().x + ui.spacing().button_padding.x * 2.0
            };
            let address_width = (ui.available_width()
                - search_width
                - recursive_width
                - selection_width
                - 3.0 * spacing)
                .max(80.0);
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
            if ui
                .checkbox(&mut self.recursive, "Récursive")
                .on_hover_text(
                    "Recherche, dossiers vides et doublons parcourent aussi les sous-dossiers",
                )
                .changed()
            {
                self.on_recursive_changed();
            }
            self.search_box(ui, search_width);
            if let Some(chosen) = self.selection_menu(ui) {
                action = Some(chosen);
            }
        });
        action
    }

    /// Filtre « Sélection », à droite de la barre de recherche : dossiers vides, doublons (du plus
    /// gros au plus petit) et fichiers plus gros qu'un seuil, selon l'option « Récursive ».
    fn selection_menu(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let on_dir = matches!(self.location, Location::Dir(_));
        let scope = if self.recursive {
            "sous-dossiers compris"
        } else {
            "dossier courant seulement"
        };
        menu(ui, SELECTION_MENU_LABEL, |ui| {
            ui.weak(format!("Portée : {scope} (case Récursive)"));
            if ui
                .add_enabled(on_dir, egui::Button::new("📂 Dossiers vides"))
                .on_hover_text("Liste et sélectionne les dossiers sans aucun fichier")
                .clicked()
            {
                action = Some(Action::FindEmptyFolders);
                ui.close();
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(on_dir, egui::Button::new("📑 Doublons"))
                    .on_hover_text(
                        "Fichiers de même contenu (même empreinte), plus gros que la taille \
                         indiquée, classés du plus gros au plus petit",
                    )
                    .clicked()
                {
                    action = Some(Action::FindDuplicates);
                    ui.close();
                }
                ui.label(">");
                ui.add(
                    egui::DragValue::new(&mut self.duplicate_min_mb)
                        .range(1..=1_000_000)
                        .suffix(" Mo"),
                )
                .on_hover_text("Taille minimale des fichiers examinés (modifiable)");
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(on_dir, egui::Button::new("📦 Fichiers"))
                    .on_hover_text(
                        "Fichiers plus gros que la taille indiquée, du plus gros au plus petit",
                    )
                    .clicked()
                {
                    action = Some(Action::FindLargeFiles);
                    ui.close();
                }
                ui.label(">");
                ui.add(
                    egui::DragValue::new(&mut self.large_min_mb)
                        .range(1..=1_000_000)
                        .suffix(" Mo"),
                )
                .on_hover_text("Taille minimale des fichiers listés (modifiable)");
            });
        });
        action
    }

    /// Barre de recherche par nom (voir `view::matches_query`) : filtre le dossier affiché, ou, si
    /// « Récursive » est cochée, cherche aussi dans les sous-dossiers. Ctrl+F lui donne le focus ;
    /// Échap l'efface.
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
            self.on_search_changed();
        } else if edit.changed() {
            self.on_search_changed();
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
            // Onglets : après le menu Option, dès que plusieurs dossiers sont ouverts.
            if self.tabs.len() > 1 {
                ui.separator();
                for (index, path) in self.tabs.iter().enumerate() {
                    let mut name = favorite_name(path);
                    if name.chars().count() > 22 {
                        name = name.chars().take(21).collect::<String>() + "…";
                    }
                    let active = index == self.active_tab;
                    if ui
                        .add(egui::Button::new(format!("🗂 {name}")).selected(active))
                        .on_hover_text(path.display().to_string())
                        .clicked()
                        && !active
                    {
                        action = Some(Action::SwitchTab(index));
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Fermer l'onglet (Ctrl+W)")
                        .clicked()
                    {
                        action = Some(Action::CloseTab(index));
                    }
                }
            }
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
            .or(self.selection.single.as_ref().map(|s| &s.path))
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

    /// Panneau de la barre latérale, toujours à gauche.
    fn sidebar_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        egui::Panel::left("sidebar-left")
            .default_size(SIDEBAR_DEFAULT_WIDTH)
            .resizable(true)
            .show(ui, |ui| {
                action = self.sidebar(ui);
            });
        action
    }

    /// Le volet de détail est affiché (option « Volet d'aperçu », ou multi-sélection en cours).
    fn detail_shown(&self) -> bool {
        // La demande de suppression s'affiche dans le détail : il apparaît pour elle.
        self.show_preview || !self.selection.multi.is_empty() || self.confirm_delete
    }

    /// Volet latéral (à gauche ou à droite selon la disposition) : le volet de détail, ou la
    /// navigation quand le détail est au centre. Un petit triangle, en bas contre la zone
    /// centrale, échange les deux.
    fn side_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.detail_shown() {
            return None;
        }
        let mut action = None;
        let central = self.detail_central;
        let (min_width, width) = if central {
            (260.0, 420.0)
        } else {
            // Jamais plus étroit que la barre « Colonnes Images Vidéos » sur une ligne.
            let min = Self::detail_min_width(ui);
            (min, 300.0_f32.max(min))
        };
        let panel = match (self.preview_side, central) {
            (Side::Left, false) => egui::Panel::left("detail-left"),
            (Side::Right, false) => egui::Panel::right("detail-right"),
            (Side::Left, true) => egui::Panel::left("navigation-left"),
            (Side::Right, true) => egui::Panel::right("navigation-right"),
        };
        let response = panel
            .default_size(width)
            .min_size(min_width)
            .resizable(true)
            .show(ui, |ui| {
                action = if central {
                    self.navigation_area(ui)
                } else {
                    self.preview_panel(ui)
                };
            })
            .response;
        if self.swap_handle(ui.ctx(), response.rect) {
            self.detail_central = !self.detail_central;
        }
        action
    }

    /// Triangle d'échange, en bas du volet latéral, contre la zone centrale : il pointe vers le
    /// centre et y amène le contenu du volet. Renvoie `true` s'il est cliqué.
    fn swap_handle(&self, ctx: &egui::Context, panel: egui::Rect) -> bool {
        let on_right = self.preview_side == Side::Right;
        let (arrow, x) = if on_right {
            ("◀", panel.left() + 4.0)
        } else {
            ("▶", panel.right() - 26.0)
        };
        let hint = if self.detail_central {
            "Remettre la navigation au centre"
        } else {
            "Mettre le volet de détail au centre"
        };
        let mut clicked = false;
        egui::Area::new(egui::Id::unique("swap-panels"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, panel.bottom() - 26.0))
            .show(ctx, |ui| {
                clicked = ui.small_button(arrow).on_hover_text(hint).clicked();
            });
        clicked
    }

    /// Navigation dans les dossiers : barre d'adresse et recherche, résultats, contenu.
    fn navigation_area(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = self.location_bar(ui);
        action = self.results_banner(ui).or(action);
        ui.add_space(4.0);
        self.central(ui).or(action)
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
        let cursor = self.sidebar_cursor_path();
        let mut any = false;
        for drive in &self.drives {
            if network_only && drive.kind != DriveKind::Network {
                continue;
            }
            any = true;
            let info = self.drive_infos.get(&drive.path).and_then(Option::as_ref);
            let here =
                matches!(&self.location, Location::Dir(path) if path.starts_with(&drive.path));
            // Pilotée au clavier : le lecteur choisi par les flèches est en surbrillance.
            let highlighted = cursor.as_ref().map_or(here, |c| *c == drive.path);
            if drive_entry(ui, drive, info, highlighted, self.show_drive_usage).clicked() {
                action = Some(Action::SidebarGo(drive.path.clone()));
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
        // Le titre est le dossier de l'utilisateur lui-même : un clic l'ouvre.
        let cursor = self.sidebar_cursor_path();
        let marked = |path: &PathBuf| {
            cursor
                .as_ref()
                .map_or(self.location == Location::Dir(path.clone()), |c| c == path)
        };
        if ui
            .selectable_label(
                marked(home),
                egui::RichText::new(format!("🏠 {}", favorite_name(home))).heading(),
            )
            .on_hover_text(home.display().to_string())
            .clicked()
        {
            action = Some(Action::SidebarGo(home.clone()));
        }
        for entry in &self.home_dirs {
            if entry.hidden && !self.show_hidden {
                continue;
            }
            if ui
                .selectable_label(marked(&entry.path), format!("📁 {}", entry.name))
                .on_hover_text(entry.path.display().to_string())
                .clicked()
            {
                action = Some(Action::SidebarGo(entry.path.clone()));
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
                if let Some(chosen) = self.application_menu(ui, path, removable) {
                    action = Some(chosen);
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
        let cursor = self.sidebar_cursor_path();
        for path in self.favorites.paths() {
            let here = cursor
                .as_ref()
                .map_or(self.location == Location::Dir(path.clone()), |c| c == path);
            let response = ui
                .selectable_label(here, format!("⭐ {}", favorite_name(path)))
                .on_hover_text(path.display().to_string());
            if response.clicked() {
                action = Some(Action::SidebarGo(path.clone()));
            }
            response.context_menu(|ui| {
                if let Some(chosen) = self.favorite_menu(ui, path) {
                    action = Some(chosen);
                }
            });
        }
        action
    }

    fn central(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        // Les résultats d'un parcours (recherche récursive, dossiers vides, doublons) s'affichent
        // toujours en liste, quel que soit le mode d'affichage.
        if self.results.is_some() && matches!(self.location, Location::Dir(_)) {
            return self.listing_view(ui);
        }
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
        // Dans Home, les colonnes commencent sous le dossier de l'utilisateur (la barre latérale
        // montre déjà son contenu) : pas de colonnes C:, Users, nom.
        let crumbs = fs_nav::column_levels(&current, self.home.as_deref());
        // Listings des dossiers traversés (le dernier est le dossier courant : `self.visible`).
        for (_, path) in crumbs.iter().take(crumbs.len().saturating_sub(1)) {
            if !self.column_cache.contains_key(path)
                && let Ok(list) = self.storage.list(path)
            {
                let processed = self.filtered_and_sorted(&list, false);
                self.column_cache.insert(path.clone(), processed);
            }
        }
        // Toutes les colonnes ont la même largeur fixe ; la dernière porte aussi les informations
        // de la liste (taille, dates…) et, si la place le permet, occupe le reste de la largeur.
        let name_width = fixed_column_width(ui);
        let info_columns = self.active_columns();
        let info_width: f32 = info_columns.iter().map(|column| column.width()).sum();
        let mut widths: Vec<f32> = (0..crumbs.len())
            .map(|level| {
                if level + 1 == crumbs.len() {
                    name_width + info_width
                } else {
                    name_width
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
                    for (level, (_, dir)) in crumbs.iter().enumerate() {
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
                                            self.selection.contains(&entry.path)
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
                                            entry.hidden || self.clipboard.is_cut(&entry.path),
                                            &format!("{icon} {}", entry.name),
                                            row_columns,
                                            &cells,
                                        );
                                        let view = ItemView::Column {
                                            dir,
                                            highlighted: next_dir.map(PathBuf::as_path),
                                            is_last,
                                        };
                                        if let Some(gesture) =
                                            self.item_events(ui, &response, entry, view)
                                        {
                                            action = Some(gesture);
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

    /// Mode « grille » : hors de Home, des zones côte à côte — les lecteurs (sauf si l'onglet « Ce PC »
    /// de la barre latérale les montre déjà), une colonne de navigation (`..` et sous-dossiers),
    /// puis la grille. Dans Home, la grille seule (avec les
    /// dossiers en tuiles pour pouvoir y entrer).
    fn gallery_layout(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.grid_side_columns() {
            return self.gallery_view(ui);
        }
        let mut action = None;
        let height = ui.available_height();
        let gap = ui.spacing().item_spacing.x * 2.0 + 1.0;
        // Les lecteurs ne sont pas répétés quand l'onglet « Ce PC » de la barre latérale les montre.
        let show_drives = self.sidebar_tab != SidebarTab::Pc;
        let zones = if show_drives { 2.0 } else { 1.0 };
        // Sur une fenêtre étroite, la grille garde au moins une tuile de large.
        let side = (ui.available_width() - GALLERY_TILE - (zones * gap)).max(0.0);
        let drives_width = if show_drives {
            GALLERY_DRIVES_WIDTH.min(side * 0.45)
        } else {
            0.0
        };
        let nav_width = GALLERY_NAV_WIDTH.min(side - drives_width).max(0.0);
        let grid_width = (ui.available_width() - drives_width - nav_width - zones * gap).max(60.0);
        ui.horizontal_top(|ui| {
            if show_drives {
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
            }
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
                    let selected = self
                        .selection
                        .single
                        .as_ref()
                        .is_some_and(|s| s.path == entry.path);
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
                        let selected = self.selection.contains(&entry.path);
                        let is_image = !entry.is_dir && preview::is_supported_image(&entry.path);
                        let thumb = is_image.then(|| self.thumbs.get(&entry.path)).flatten();
                        if is_image && thumb.is_none() {
                            wanted.push(entry.path.clone());
                        }
                        let response = gallery_tile(ui, entry, selected, thumb);
                        if let Some(gesture) =
                            self.item_events(ui, &response, entry, ItemView::Grid)
                        {
                            action = Some(gesture);
                        }
                    }
                });
            }
        });
        self.last_scroll = (output.state.clamped_offset().y, output.inner_rect.height());
        self.request_thumbs(&wanted);
        action
    }

    /// Libellé d'une ligne de la liste. Parmi des résultats : numéro de groupe (doublons) et
    /// dossier d'origine, relatif au dossier affiché.
    fn row_label(&self, icon: &str, entry: &Entry) -> String {
        if self.results.is_none() {
            return format!("{icon} {}", entry.name);
        }
        let group = self
            .duplicate_groups
            .get(&entry.path)
            .map_or(String::new(), |group| format!("#{group}  "));
        let origin = match (&self.location, entry.path.parent()) {
            (Location::Dir(root), Some(parent)) => parent.strip_prefix(root).map_or_else(
                |_| parent.display().to_string(),
                |p| p.display().to_string(),
            ),
            (_, Some(parent)) => parent.display().to_string(),
            _ => String::new(),
        };
        if origin.is_empty() {
            format!("{group}{icon} {}", entry.name)
        } else {
            format!("{group}{icon} {}   —   {origin}", entry.name)
        }
    }

    /// Texte de la colonne « Taille » d'une entrée de dossier.
    fn entry_size_text(&self, entry: &Entry) -> String {
        match entry.size {
            Some(bytes) => fs_nav::format_size(bytes),
            None if entry.is_dir && !self.skip_folder_sizes && self.results.is_none() => {
                "Calcul…".to_string()
            }
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
        let parent = if self.results.is_some() {
            None
        } else {
            self.location.parent()
        };
        let shift = usize::from(parent.is_some());
        let empty_message = if let Some(results) = &self.results {
            if !self.visible.is_empty() {
                None
            } else if results.running {
                Some("Analyse en cours…")
            } else {
                Some("Aucun résultat.")
            }
        } else if self.entries.is_empty() {
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
                let is_selected = self.selection.contains(&entry.path);
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
                    entry.hidden || self.clipboard.is_cut(&entry.path),
                    &self.row_label(icon, entry),
                    &columns,
                    &cells,
                );
                if let Some(gesture) = self.item_events(ui, &response, entry, ItemView::List) {
                    action = Some(gesture);
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
        let recycle_bin = self
            .delete_targets()
            .first()
            .is_none_or(|e| self.storage.capabilities(&e.path).recycle_bin);
        if self.confirm_delete {
            let question = if recycle_bin {
                format!("Envoyer {what} à la corbeille ?")
            } else {
                format!("Pas de corbeille ici : supprimer {what} définitivement ?")
            };
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{question}  (Entrée : confirmer, Échap : annuler)"),
            );
        } else if recycle_bin {
            ui.weak("Suppression : vers la corbeille de Windows.");
        } else {
            ui.weak("Suppression définitive : pas de corbeille sur ce lecteur.");
        }
        action
    }

    /// Emplacement d'un résultat, dans le volet de détail : le dossier qui le contient, puis
    /// chaque dossier au-dessus jusqu'au dossier affiché (exclu). Chaque ligne permet d'**ouvrir**
    /// ce dossier (on y retrouve le fichier, on peut remonter) ou de le **supprimer**
    /// (corbeille, avec confirmation) : pratique pour éliminer un dossier entier de doublons.
    fn result_location(&self, ui: &mut egui::Ui, entry: &Entry) -> Option<Action> {
        let mut action = None;
        let Location::Dir(root) = &self.location else {
            return None;
        };
        let parent = entry.path.parent()?;
        ui.add_space(2.0);
        ui.weak("Emplacement");
        let mut levels = fs_nav::levels_below(root, &entry.path);
        if levels.is_empty() {
            // Directement dans le dossier affiché : on peut seulement l'ouvrir.
            levels.push(parent.to_path_buf());
        }
        for (index, level) in levels.iter().enumerate() {
            let label = level
                .strip_prefix(root)
                .ok()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| level.display().to_string(), |p| p.display().to_string());
            ui.horizontal(|ui| {
                let open = ui
                    .small_button("↗ Ouvrir")
                    .on_hover_text(format!("Aller dans {}", level.display()));
                if open.clicked() {
                    action = Some(if index == 0 {
                        Action::GoSelect {
                            dir: level.clone(),
                            file: entry.path.clone(),
                        }
                    } else {
                        Action::Go(Location::Dir(level.clone()))
                    });
                }
                // Le dossier affiché lui-même se supprime depuis son propre volet.
                if level != root
                    && ui
                        .small_button("🗑")
                        .on_hover_text("Supprimer ce dossier (corbeille, avec confirmation)")
                        .clicked()
                {
                    action = Some(Action::AskDeleteFolder(level.clone()));
                }
                ui.add(egui::Label::new(egui::RichText::new(label).weak()).truncate());
            });
        }
        ui.add_space(2.0);
        action
    }

    /// Outils d'un dossier, dans le volet de détail : PowerShell (normal ou administrateur) et
    /// copie du chemin.
    fn folder_tools(&self, ui: &mut egui::Ui, dir: &Path) -> Option<Action> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if powershell_button(ui, ">_ PowerShell")
                .on_hover_text("Ouvre PowerShell dans ce dossier")
                .clicked()
            {
                action = Some(Action::OpenPowerShell {
                    dir: dir.to_path_buf(),
                    admin: false,
                });
            }
            if powershell_button(ui, ">_ Admin")
                .on_hover_text("Ouvre PowerShell en tant qu'administrateur (fenêtre UAC)")
                .clicked()
            {
                action = Some(Action::OpenPowerShell {
                    dir: dir.to_path_buf(),
                    admin: true,
                });
            }
            if ui
                .button("📋 Chemin")
                .on_hover_text("Copie le chemin du dossier")
                .clicked()
            {
                action = Some(Action::CopyPath(dir.to_path_buf()));
            }
        });
        // Nouveau dossier, Coller ici, Propriétés : selon ce que la source permet.
        let caps = self.storage.capabilities(dir);
        let folder = Entry::folder_at(dir);
        let targets = std::slice::from_ref(&folder);
        ui.horizontal_wrapped(|ui| {
            for (allowed, command) in [
                (caps.create_folder, Command::CreateFolder),
                (caps.copy_into, Command::Paste),
                (caps.properties, Command::ShowProperties),
            ] {
                if allowed
                    && let Some(chosen) =
                        context_menu_manager::command_button(ui, command, &folder, targets)
                {
                    action = Some(chosen);
                }
            }
        });
        ui.add_space(4.0);
        action
    }

    /// Contenu du volet pendant la multi-sélection : résumé et actions (supprimer).
    fn actions_content(&self, ui: &mut egui::Ui) -> Option<Action> {
        let count = self.selection.multi.len();
        let kind = if self.selection.multi.first().is_some_and(|e| e.is_dir) {
            "dossier(s)"
        } else {
            "fichier(s)"
        };
        ui.label(egui::RichText::new(format!("{count} {kind} sélectionné(s)")).strong());
        ui.weak("Clic maintenu ou clic sur une ligne pour ajouter ou retirer.");
        ui.add_space(6.0);
        let mut copy_action = None;
        if let Some(first) = self.selection.multi.first() {
            let caps = self.storage.capabilities(&first.path);
            ui.horizontal_wrapped(|ui| {
                for (allowed, command) in [(true, Command::Copy), (caps.move_items, Command::Cut)] {
                    if allowed
                        && let Some(chosen) = context_menu_manager::command_button(
                            ui,
                            command,
                            first,
                            &self.selection.multi,
                        )
                    {
                        copy_action = Some(chosen);
                    }
                }
            });
        }
        let action = self
            .edit_controls(ui, &format!("{count} {kind}"), None, true)
            .or(copy_action);
        ui.add_space(6.0);
        ui.separator();
        let row_height = table_row_height(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, count, |ui, range| {
                for item in &self.selection.multi[range] {
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
    fn preview_panel(&self, ui: &mut egui::Ui) -> Option<Action> {
        let option_action = self.option_bar(ui);
        let content_action = self.preview_content(ui);
        option_action.or(content_action)
    }

    /// Largeur minimale du volet de détail : la flèche et les boutons Colonnes, Images et Vidéos
    /// tiennent sur une seule ligne.
    fn detail_min_width(ui: &egui::Ui) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let padding = ui.spacing().button_padding.x * 2.0;
        let gap = ui.spacing().item_spacing.x;
        let labels = [
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
    fn option_bar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
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
        action
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
        match &active.texture {
            Some(texture) => {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(4.0);
                let image =
                    egui::Image::new(egui::load::SizedTexture::new(texture.id(), size * scale))
                        .sense(egui::Sense::click());
                // Double-clic sur la vidéo : plein écran.
                if ui
                    .add(image)
                    .on_hover_text("Double-clic : plein écran")
                    .double_clicked()
                {
                    action = Some(Action::ToggleVideoFullscreen);
                }
            }
            None => {
                // Son (pas d'image) ou image pas encore décodée.
                ui.label(egui::RichText::new("🎵").size(48.0));
            }
        }
        Self::player_bar(ui, active).or(action)
    }

    /// Commandes du lecteur : lecture / pause, arrêt, position, volume.
    fn player_bar(ui: &mut egui::Ui, active: &ActivePlayer) -> Option<Action> {
        let mut action = None;
        let player = &active.player;
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
        if !self.selection.multi.is_empty() {
            return self.actions_content(ui);
        }
        let Some(selected) = &self.selection.single else {
            // Aucun élément choisi : le volet propose d'agir sur le dossier affiché lui-même.
            if let Location::Dir(path) = &self.location
                && self.results.is_none()
                && path.parent().is_some()
            {
                ui.label(egui::RichText::new(favorite_name(path)).strong());
                ui.label("Dossier courant");
                ui.add_space(6.0);
                let tools = self.folder_tools(ui, path);
                if ui
                    .button("🗑 Supprimer ce dossier")
                    .on_hover_text("Envoie le dossier affiché à la corbeille (après confirmation)")
                    .clicked()
                {
                    return Some(Action::AskDeleteFolder(path.clone()));
                }
                ui.weak("Suppression : vers la corbeille de Windows.");
                return tools;
            } else {
                ui.weak("Sélectionnez un élément.");
            }
            return None;
        };
        // La taille d'un dossier peut arriver après la sélection : on relit l'entrée à jour.
        let entry = self
            .entries
            .iter()
            .find(|e| e.path == selected.path)
            .unwrap_or(selected);
        ui.label(egui::RichText::new(&entry.name).strong());
        // Parmi des résultats (doublons, recherche) : où se trouve l'élément, avec de quoi aller
        // dans son dossier, remonter et supprimer un dossier parent.
        if self.results.is_some() {
            action = self.result_location(ui, entry);
        }
        if entry.is_dir {
            ui.label("Dossier");
            let size = self.entry_size_text(entry);
            if !size.is_empty() {
                ui.label(format!("Taille : {size}"));
            }
            ui.add_space(6.0);
            let tools = self.folder_tools(ui, &entry.path);
            let what = format!("le dossier « {} »", entry.name);
            return self
                .edit_controls(ui, &what, Some(entry), false)
                .or(action)
                .or(tools);
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
        if let Some(results) = &self.results {
            let what = match results.kind {
                ResultKind::Search => "résultat(s) de recherche",
                ResultKind::EmptyFolders => "dossier(s) vide(s)",
                ResultKind::Duplicates => "fichier(s) en doublon",
                ResultKind::LargeFiles => "gros fichier(s)",
            };
            return format!("{} {what}", self.visible.len());
        }
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

/// Bouton aux couleurs de PowerShell (fond bleu nuit, texte blanc).
fn powershell_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(1, 36, 86)),
    )
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
        // Un nom trop long est coupé par « … ».
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width());
        let galley = painter.layout_job(job);
        let top_left = egui::pos2(name_rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(top_left, galley, text_color);
    }
    response
}

/// Largeur fixe d'une colonne de la vue « style Mac » : celle du nom de référence
/// (`COLUMN_VIEW_REFERENCE_NAME`) suivi de « … », avec l'icône et une marge pour le remplissage
/// des lignes et la barre de défilement.
fn fixed_column_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let reference = ui.painter().layout_no_wrap(
        format!("📁 {COLUMN_VIEW_REFERENCE_NAME}…"),
        font,
        egui::Color32::WHITE,
    );
    reference.size().x + 44.0
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
        let mut action = self.input_events(&ctx);
        self.drop_hint(&ctx);

        // Vidéo en plein écran : elle seule occupe la fenêtre.
        if self.video_fullscreen && self.fullscreen_video(ui) {
            self.track_window_size(&ctx);
            self.save_settings_if_changed();
            return;
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
        // Barre latérale à gauche ; volet latéral (détail, ou navigation si le détail est au
        // centre) à gauche ou à droite selon la disposition ; le reste au centre.
        action = self.sidebar_panel(ui).or(action.take());
        action = self.side_panel(ui).or(action.take());
        let detail_in_center = self.detail_central && self.detail_shown();
        egui::CentralPanel::default().show(ui, |ui| {
            let central_action = if detail_in_center {
                self.preview_panel(ui)
            } else {
                self.navigation_area(ui)
            };
            action = central_action.or(action.take());
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
        assert_eq!(app.location, Location::Dir(tree.0.clone()));
        (app, ctx)
    }

    /// Traite les événements jusqu'à la fin du parcours en cours.
    fn wait_scan(app: &mut ExpliaApp, ctx: &egui::Context) {
        let start = Instant::now();
        loop {
            app.poll_events(ctx);
            if app.results.as_ref().is_some_and(|r| !r.running) {
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
    fn empty_folder_scan_lists_and_selects_them() {
        let tree = TempTree::new("empty");
        tree.dir("vide");
        tree.dir("b/c");
        tree.file("b/d/fichier.txt", b"x");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::FindEmptyFolders);
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["c", "vide"]);
        // Tous sont sélectionnés (multi-sélection de dossiers), prêts à être supprimés.
        assert_eq!(app.selection.multi.len(), 2);
        assert!(app.selection.multi.iter().all(|e| e.is_dir));
        // Sans récursivité : seulement les sous-dossiers directs.
        app.recursive = false;
        app.on_recursive_changed();
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["vide"]);
        // Fermer revient au contenu du dossier.
        app.apply(&ctx, Action::CloseResults);
        assert!(app.results.is_none());
        assert_eq!(names(&app), ["b", "vide"]);
    }

    #[test]
    fn duplicate_scan_groups_copies_and_selects_all_but_the_first() {
        let tree = TempTree::new("dups");
        let big = vec![3u8; 1_200_000];
        tree.file("a.bin", &big);
        tree.file("sub/b.bin", &big);
        tree.file("sub/c.bin", &big);
        tree.file("autre.bin", &vec![4u8; 1_200_000]);
        let (mut app, ctx) = app_in(&tree);
        app.duplicate_min_mb = 1;
        app.apply(&ctx, Action::FindDuplicates);
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["a.bin", "b.bin", "c.bin"]);
        assert_eq!(app.duplicate_groups.len(), 3);
        app.apply(&ctx, Action::SelectDuplicateCopies);
        assert_eq!(app.selection.multi.len(), 2);
        // Le premier fichier du groupe (ordre des chemins) est conservé.
        assert!(app.selection.multi.iter().all(|e| e.name != "a.bin"));
        // Seuil plus haut que les fichiers : aucun doublon.
        app.duplicate_min_mb = 2;
        app.apply(&ctx, Action::FindDuplicates);
        wait_scan(&mut app, &ctx);
        assert!(app.visible.is_empty());
    }

    #[test]
    fn large_file_scan_lists_files_from_the_biggest_and_keeps_that_order() {
        let tree = TempTree::new("large");
        tree.file("moyen.bin", &vec![1u8; 1_500_000]);
        tree.file("sub/gros.bin", &vec![1u8; 2_500_000]);
        tree.file("petit.bin", &vec![1u8; 500_000]);
        let (mut app, ctx) = app_in(&tree);
        app.large_min_mb = 1;
        app.apply(&ctx, Action::FindLargeFiles);
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
        app.recursive = false;
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
        app.search = "*.exe".to_string();
        app.recursive = true;
        app.on_search_changed();
        wait_scan(&mut app, &ctx);
        assert_eq!(names(&app), ["outil.exe", "racine.exe"]);
        // Effacer la recherche ferme les résultats.
        app.search.clear();
        app.on_search_changed();
        assert!(app.results.is_none());
        // Non récursive : simple filtre du dossier affiché.
        app.recursive = false;
        app.search = "*.exe".to_string();
        app.on_search_changed();
        assert!(app.results.is_none());
        assert_eq!(names(&app), ["racine.exe"]);
    }

    #[test]
    fn changing_folder_cancels_and_closes_results() {
        let tree = TempTree::new("navigate");
        tree.dir("sous");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::FindEmptyFolders);
        assert!(app.results.is_some());
        app.navigate(Location::Dir(tree.0.join("sous")));
        assert!(app.results.is_none());
        assert!(app.result_entries.is_empty());
    }

    #[test]
    fn deleting_the_current_folder_needs_a_confirmation_that_can_be_cancelled() {
        let tree = TempTree::new("current");
        tree.dir("vide");
        let (mut app, ctx) = app_in(&tree);
        app.apply(&ctx, Action::AskDeleteFolder(tree.0.clone()));
        assert!(app.confirm_delete);
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.path.clone()),
            Some(tree.0.clone())
        );
        assert_eq!(app.delete_targets().len(), 1);
        // Annuler retire la sélection provisoire du dossier affiché.
        app.apply(&ctx, Action::CancelDelete);
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
        app.duplicate_min_mb = 1;
        app.apply(&ctx, Action::FindDuplicates);
        wait_scan(&mut app, &ctx);
        assert_eq!(app.visible.len(), 2);
        let first = app.visible[0].clone();
        app.apply(&ctx, Action::Select(first.clone()));
        assert_eq!(
            app.selection.single.as_ref().map(|e| e.path.clone()),
            Some(first.path.clone())
        );
        // Le dossier parent du résultat devient la cible provisoire de la suppression.
        let parent = first.path.parent().unwrap().to_path_buf();
        app.apply(&ctx, Action::AskDeleteFolder(parent.clone()));
        assert!(app.confirm_delete);
        assert_eq!(app.delete_targets()[0].path, parent);
        app.apply(&ctx, Action::CancelDelete);
        assert!(!app.confirm_delete && app.selection.single.is_none());
        // Un résultat sélectionné normalement reste sélectionné après un Échap sans demande.
        app.apply(&ctx, Action::Select(first.clone()));
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
        assert!(app.results.is_none());
        assert_eq!(app.location, Location::Dir(parent));
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
        assert!(app.video_fullscreen);
        app.apply(&ctx, Action::ToggleVideoFullscreen);
        assert!(!app.video_fullscreen);
    }

    #[test]
    fn dropped_items_are_copied_into_the_current_folder_without_overwriting() {
        let tree = TempTree::new("drop");
        tree.file("ailleurs/photo.jpg", b"img");
        tree.file("ici/photo.jpg", b"ancienne");
        let here = tree.0.join("ici");
        let (mut app, ctx) = app_in(&tree);
        app.navigate(Location::Dir(here.clone()));
        app.apply(
            &ctx,
            Action::CopyInto {
                sources: vec![tree.0.join("ailleurs/photo.jpg")],
                dest: here.clone(),
            },
        );
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
        app.start_transfer(vec![here.join("a.txt")], dest.clone(), ClipMode::Cut);
        let start = Instant::now();
        while app.copying {
            app.poll_events(&ctx);
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
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
        assert_eq!(app.location, Location::Dir(tree.0.join("existant")));
        assert!(tree.0.join("existant/Nouveau dossier").is_dir());
    }

    #[test]
    fn tabs_open_switch_and_close() {
        let tree = TempTree::new("tabs");
        tree.dir("a");
        tree.dir("b");
        let (mut app, ctx) = app_in(&tree);
        assert!(app.tabs.is_empty());
        app.apply(&ctx, Action::NewTab(tree.0.join("a")));
        assert_eq!(app.tabs, [tree.0.clone(), tree.0.join("a")]);
        assert_eq!(app.active_tab, 1);
        assert_eq!(app.location, Location::Dir(tree.0.join("a")));
        // Naviguer change le dossier de l'onglet affiché.
        app.navigate(Location::Dir(tree.0.join("b")));
        assert_eq!(app.tabs[1], tree.0.join("b"));
        app.apply(&ctx, Action::SwitchTab(0));
        assert_eq!(app.location, Location::Dir(tree.0.clone()));
        // Fermer l'onglet affiché passe au voisin ; avec un seul dossier, plus d'onglets.
        app.apply(&ctx, Action::CloseTab(0));
        assert!(app.tabs.is_empty());
        assert_eq!(app.location, Location::Dir(tree.0.join("b")));
        // Trois onglets : fermer un onglet non affiché garde le dossier affiché.
        app.apply(&ctx, Action::NewTab(tree.0.join("a")));
        app.apply(&ctx, Action::NewTab(tree.0.clone()));
        assert_eq!(app.tabs.len(), 3);
        app.apply(&ctx, Action::CloseTab(0));
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert_eq!(app.location, Location::Dir(tree.0.clone()));
    }

    #[test]
    fn arrows_walk_through_the_home_list_after_a_click_in_it() {
        let tree = TempTree::new("home-arrows");
        tree.dir("Apple");
        tree.dir("build");
        tree.dir("Contacts");
        let (mut app, ctx) = app_in(&tree);
        app.home = Some(tree.0.clone());
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
        assert!(app.sidebar_focus);
        app.step_sidebar(1);
        assert_eq!(app.location, Location::Dir(tree.0.join("Contacts")));
        app.step_sidebar(-1);
        app.step_sidebar(-1);
        assert_eq!(app.location, Location::Dir(tree.0.join("Apple")));
        // Le titre (le dossier de l'utilisateur) est le premier élément.
        app.step_sidebar(-1);
        assert_eq!(app.location, Location::Dir(tree.0.clone()));
        assert_eq!(app.sidebar_cursor_path(), Some(tree.0.clone()));
        // Un clic dans la zone centrale ne donne pas la main à la barre latérale.
        app.apply(&ctx, Action::Go(Location::Dir(tree.0.join("Apple"))));
        assert!(!app.sidebar_focus);
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
        assert_eq!(app.location, Location::Dir(tree.0.clone()));
        let mut selected: Vec<String> =
            app.selection.multi.iter().map(|e| e.name.clone()).collect();
        selected.sort();
        assert_eq!(selected, ["grec", "models"]);
        // Maj+clic : la plage depuis l'élément en surbrillance.
        app.navigate(Location::Dir(tree.0.join("grec")));
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
        let mut range: Vec<String> = app.selection.multi.iter().map(|e| e.name.clone()).collect();
        range.sort();
        assert_eq!(range, ["aa", "grec"]);
    }
}
