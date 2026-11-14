//! Sous-structures de l'état de l'application, regroupées par domaine.
//!
//! `ExpliaApp` garde les réglages, la liste affichée et les gestionnaires ; le reste est
//! rangé ici pour réduire la structure centrale. Champs visibles dans tout `app`.

use super::*;

/// Emplacement affiché, barre d'adresse, onglets, lecture du dossier et position dans la liste.
pub(in crate::app) struct Navigation {
    pub(in crate::app) location: Location,
    pub(in crate::app) address: String,
    /// Barre d'adresse en mode saisie (sinon : fil d'Ariane cliquable).
    pub(in crate::app) address_editing: bool,
    pub(in crate::app) focus_address: bool,
    /// Onglets : dossier de chaque onglet ; vide tant qu'un seul dossier est ouvert.
    pub(in crate::app) tabs: Vec<PathBuf>,
    /// Onglet affiché.
    pub(in crate::app) active_tab: usize,
    /// Lecture de dossier en cours (ouverture ou relecture).
    pub(in crate::app) pending_listing: Option<PendingListing>,
    /// Identifie la dernière lecture demandée.
    pub(in crate::app) listing_generation: u64,
    /// Dernière ligne atteinte (clic ou flèches) : point de départ des flèches.
    pub(in crate::app) cursor: Option<PathBuf>,
    /// Ligne à rendre visible au prochain dessin de la liste (navigation au clavier).
    pub(in crate::app) scroll_to_index: Option<usize>,
    /// Dernier état de défilement de la liste : (décalage, hauteur visible), en points.
    pub(in crate::app) last_scroll: (f32, f32),
    /// Listings des dossiers parents, pour la vue en colonnes.
    pub(in crate::app) column_cache: HashMap<PathBuf, Vec<Entry>>,
    /// Dossiers de la vue en colonnes en cours de lecture.
    pub(in crate::app) column_requests: HashSet<PathBuf>,
    /// Identifie les lectures de la vue en colonnes ; l'incrémenter rend les anciennes caduques.
    pub(in crate::app) column_generation: u64,
}

impl Navigation {
    pub(in crate::app) fn new() -> Self {
        Self {
            location: Location::ThisPc,
            address: Location::ThisPc.display(),
            address_editing: false,
            focus_address: false,
            tabs: Vec::new(),
            active_tab: 0,
            pending_listing: None,
            listing_generation: 0,
            cursor: None,
            scroll_to_index: None,
            last_scroll: (0.0, 0.0),
            column_cache: HashMap::new(),
            column_requests: HashSet::new(),
            column_generation: 0,
        }
    }
}

/// Barre latérale : lecteurs, dossier de l'utilisateur, onglet actif et main du clavier.
pub(in crate::app) struct SidebarState {
    pub(in crate::app) drives: Vec<Drive>,
    /// Capacité et occupation des lecteurs : absente tant que le calcul est en cours,
    /// `None` si le lecteur n'est pas prêt.
    pub(in crate::app) drive_infos: HashMap<PathBuf, Option<DriveInfo>>,
    /// Onglet affiché dans la barre latérale.
    pub(in crate::app) tab: SidebarTab,
    /// Le clavier pilote la liste des lecteurs : ↑ / ↓ changent de lecteur, → revient au dossier.
    pub(in crate::app) focus: bool,
    /// Lecteur choisi au clavier (index dans `drives`).
    pub(in crate::app) cursor: Option<usize>,
    /// Dossier de l'utilisateur courant (onglet « Home »).
    pub(in crate::app) home: Option<PathBuf>,
    /// Sous-dossiers du dossier de l'utilisateur.
    pub(in crate::app) home_dirs: Vec<Entry>,
}

impl SidebarState {
    pub(in crate::app) fn new() -> Self {
        Self {
            drives: Vec::new(),
            drive_infos: HashMap::new(),
            tab: SidebarTab::Pc,
            focus: false,
            cursor: None,
            home: places::user_home().filter(|home| home.is_dir()),
            home_dirs: Vec::new(),
        }
    }
}

/// Recherche et parcours (recherche récursive, dossiers vides, doublons, gros fichiers) et leurs résultats.
pub(in crate::app) struct ScanState {
    /// Résultats affichés à la place du contenu du dossier.
    pub(in crate::app) results: Option<Results>,
    /// Éléments trouvés par le parcours en cours ou terminé.
    pub(in crate::app) result_entries: Vec<Entry>,
    /// Numéro de groupe (à partir de 1) de chaque fichier en doublon.
    pub(in crate::app) duplicate_groups: HashMap<PathBuf, usize>,
    /// Identifie le parcours courant ; l'incrémenter l'annule.
    pub(in crate::app) generation: Arc<AtomicU64>,
    /// Recherche (barre au-dessus de la liste) : motifs de noms, voir `view::matches_query`.
    pub(in crate::app) search: String,
    /// Donne le focus à la barre de recherche à la prochaine image (Ctrl+F).
    pub(in crate::app) focus_search: bool,
    /// Option « Récursive » : recherche, doublons et gros fichiers parcourent les sous-dossiers.
    pub(in crate::app) recursive: bool,
    /// Taille minimale (Mo) des fichiers listés par « Fichiers > N Mo ».
    pub(in crate::app) large_min_mb: u32,
}

impl ScanState {
    pub(in crate::app) fn new(settings: &Settings) -> Self {
        Self {
            results: None,
            result_entries: Vec::new(),
            duplicate_groups: HashMap::new(),
            generation: Arc::new(AtomicU64::new(0)),
            search: String::new(),
            focus_search: false,
            recursive: settings.recursive,
            large_min_mb: settings.large_min_mb,
        }
    }
}

/// Lecture audio et vidéo intégrée (libVLC).
pub(in crate::app) struct Playback {
    /// Lecture audio / vidéo intégrée (libVLC).
    pub(in crate::app) slot: PlayerSlot,
    /// Vidéo en plein écran (double-clic sur la vidéo du volet de détail).
    pub(in crate::app) fullscreen: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            slot: PlayerSlot::None,
            fullscreen: false,
        }
    }
}

/// Aperçu de l'élément sélectionné, miniatures de la grille et métadonnées multimédia du dossier.
pub(in crate::app) struct PreviewData {
    pub(in crate::app) state: PreviewState,
    /// Miniatures de la grille d'images.
    pub(in crate::app) thumbs: HashMap<PathBuf, egui::TextureHandle>,
    /// Miniatures en cours de décodage ou déjà en échec (inutile de les redemander).
    pub(in crate::app) thumbs_requested: HashSet<PathBuf>,
    pub(in crate::app) thumbs_in_flight: usize,
    /// Métadonnées multimédia lues en arrière-plan pour le listing courant.
    pub(in crate::app) media: HashMap<PathBuf, MediaInfo>,
    /// Identifie le listing pour lequel les métadonnées sont lues ; l'incrémenter annule.
    pub(in crate::app) media_generation: Arc<AtomicU64>,
}

impl Default for PreviewData {
    fn default() -> Self {
        Self {
            state: PreviewState::None,
            thumbs: HashMap::new(),
            thumbs_requested: HashSet::new(),
            thumbs_in_flight: 0,
            media: HashMap::new(),
            media_generation: Arc::new(AtomicU64::new(0)),
        }
    }
}
