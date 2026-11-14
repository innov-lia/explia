//! Affichage fractionné : deux volets côte à côte, chacun avec son dossier, pour transférer
//! des éléments de l'un à l'autre (comme WinSCP ou un commandeur de fichiers).
//!
//! Le volet qui a la main occupe les champs habituels d'`ExpliaApp` (navigation, liste,
//! sélection, recherche, aperçu) : tout le reste de l'application (barre latérale, clavier,
//! menus, volet de détail) agit sur lui sans le savoir. L'autre volet attend dans
//! `SplitView::other` ; `with_other` l'échange le temps de le dessiner ou de lui livrer un
//! résultat de fond, `activate_other` lui donne la main.

use super::*;

/// Écart entre les générations de deux volets : leurs compteurs (tailles de dossiers, parcours,
/// métadonnées) ne se recoupent jamais, un résultat de fond retrouve toujours son volet.
const PANE_GENERATION_STRIDE: u64 = 1 << 40;

/// État d'un volet de l'affichage fractionné (celui qui n'a pas la main).
pub(super) struct Pane {
    nav: Navigation,
    entries: Vec<Entry>,
    visible: Vec<Entry>,
    selection: Selection,
    scan: ScanState,
    preview: PreviewData,
    folder_generation: Arc<AtomicU64>,
    renaming: Option<RenameState>,
    confirm_delete: bool,
    delete_asked_at: Option<f64>,
}

impl Pane {
    /// Volet vide ; `serial` distingue ses compteurs de ceux des autres volets.
    fn new(serial: u64, recursive: bool, large_min_mb: u32) -> Self {
        let base = serial * PANE_GENERATION_STRIDE;
        let mut scan = ScanState::with_options(recursive, large_min_mb);
        scan.generation = Arc::new(AtomicU64::new(base));
        let preview = PreviewData {
            media_generation: Arc::new(AtomicU64::new(base)),
            ..PreviewData::default()
        };
        Self {
            nav: Navigation::new(),
            entries: Vec::new(),
            visible: Vec::new(),
            selection: Selection::default(),
            scan,
            preview,
            folder_generation: Arc::new(AtomicU64::new(base)),
            renaming: None,
            confirm_delete: false,
            delete_asked_at: None,
        }
    }
}

/// Affichage fractionné en cours.
pub(super) struct SplitView {
    /// Le volet qui n'a pas la main.
    other: Box<Pane>,
    /// Côté du volet qui a la main.
    pub(super) active: Side,
    /// Position des volets (gauche, droite) à la dernière image.
    pub(super) rects: [egui::Rect; 2],
}

/// L'autre côté.
pub(super) fn opposite(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

/// Index d'un côté dans `SplitView::rects`.
pub(super) fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

impl ExpliaApp {
    /// Échange le volet qui a la main et l'autre (champs d'`ExpliaApp` ↔ `SplitView::other`).
    fn exchange_panes(&mut self) {
        let Some(split) = self.split.as_mut() else {
            return;
        };
        let other = &mut *split.other;
        std::mem::swap(&mut self.nav, &mut other.nav);
        std::mem::swap(&mut self.entries, &mut other.entries);
        std::mem::swap(&mut self.visible, &mut other.visible);
        std::mem::swap(&mut self.selection, &mut other.selection);
        std::mem::swap(&mut self.scan, &mut other.scan);
        std::mem::swap(&mut self.preview, &mut other.preview);
        std::mem::swap(&mut self.folder_generation, &mut other.folder_generation);
        std::mem::swap(&mut self.renaming, &mut other.renaming);
        std::mem::swap(&mut self.confirm_delete, &mut other.confirm_delete);
        std::mem::swap(&mut self.delete_asked_at, &mut other.delete_asked_at);
    }

    /// Exécute `f` sur l'autre volet (sans lui donner la main).
    pub(super) fn with_other<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.exchange_panes();
        let result = f(self);
        self.exchange_panes();
        result
    }

    /// Donne la main à l'autre volet.
    pub(super) fn activate_other(&mut self) {
        if self.split.is_none() {
            return;
        }
        // La lecture et les saisies en cours appartiennent au volet qui perd la main.
        self.stop_player();
        self.renaming = None;
        self.confirm_delete = false;
        self.nav.address_editing = false;
        self.exchange_panes();
        if let Some(split) = self.split.as_mut() {
            split.active = opposite(split.active);
        }
        self.error = None;
    }

    /// Ouvre ou ferme l'affichage fractionné. À l'ouverture, l'autre volet montre le même
    /// dossier ; à la fermeture, le volet qui a la main reste affiché.
    pub(super) fn toggle_split(&mut self) {
        if self.split.take().is_some() {
            return;
        }
        let start = match &self.nav.location {
            Location::Dir(path) => Some(path.clone()),
            Location::ThisPc => self.nav.pending_listing.as_ref().map(|p| p.path.clone()),
        };
        self.open_split(start);
    }

    /// Crée l'autre volet et lui fait lire `start`.
    pub(super) fn open_split(&mut self, start: Option<PathBuf>) {
        self.pane_serial += 1;
        let pane = Pane::new(
            self.pane_serial,
            self.scan.recursive,
            self.scan.large_min_mb,
        );
        self.split = Some(SplitView {
            other: Box::new(pane),
            active: Side::Left,
            rects: [egui::Rect::NOTHING; 2],
        });
        // Les volets sont des listes : les grilles et les colonnes n'y ont pas leur place.
        self.view_mode = ViewMode::List;
        if let Some(start) = start {
            self.with_other(|app| {
                app.request_listing(
                    start,
                    ListingKind::Open { sidebar: None },
                    AfterListing::None,
                )
            });
        }
    }

    /// Dossier affiché par l'autre volet.
    pub(super) fn other_dir(&self) -> Option<PathBuf> {
        match &self.split.as_ref()?.other.nav.location {
            Location::Dir(path) => Some(path.clone()),
            Location::ThisPc => None,
        }
    }

    /// Copie ou déplace `sources` vers le dossier de l'autre volet.
    pub(super) fn transfer_to_other(&mut self, sources: Vec<PathBuf>, mode: ClipMode) {
        match self.other_dir() {
            Some(dest) => self.start_transfer(sources, dest, mode),
            None => {
                self.error = Some("L'autre volet n'affiche aucun dossier.".to_string());
            }
        }
    }

    /// Un dossier est en cours de lecture, dans l'un ou l'autre volet.
    pub(super) fn listing_pending(&self) -> bool {
        self.nav.pending_listing.is_some()
            || self
                .split
                .as_ref()
                .is_some_and(|s| s.other.nav.pending_listing.is_some())
    }

    /// Le résultat de fond `event` concerne l'autre volet.
    pub(super) fn event_for_other(&self, event: &Event) -> bool {
        let Some(split) = &self.split else {
            return false;
        };
        let other = &split.other;
        let scan = |generation: u64| generation == other.scan.generation.load(Ordering::SeqCst);
        match event {
            Event::Listing { generation, .. } => other
                .nav
                .pending_listing
                .as_ref()
                .is_some_and(|p| p.generation == *generation),
            Event::FolderSize { generation, .. } => {
                *generation == other.folder_generation.load(Ordering::SeqCst)
            }
            Event::Media { generation, .. } => {
                *generation == other.preview.media_generation.load(Ordering::SeqCst)
            }
            Event::ScanBatch { generation, .. }
            | Event::ScanGroups { generation, .. }
            | Event::ScanProgress { generation, .. }
            | Event::ScanDone { generation, .. } => scan(*generation),
            Event::Preview(path, _) => {
                matches!(&other.preview.state, PreviewState::Loading(p) if p == path)
                    && !matches!(&self.preview.state, PreviewState::Loading(p) if p == path)
            }
            Event::Thumb(path, _) => {
                other.preview.thumbs_requested.contains(path)
                    && !self.preview.thumbs_requested.contains(path)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_are_opposite_and_indexed() {
        assert_eq!(opposite(Side::Left), Side::Right);
        assert_eq!(opposite(Side::Right), Side::Left);
        assert_eq!(side_index(Side::Left), 0);
        assert_eq!(side_index(Side::Right), 1);
    }

    #[test]
    fn panes_never_share_a_generation() {
        let first = Pane::new(1, true, 50);
        let second = Pane::new(2, true, 50);
        let start = |pane: &Pane| pane.folder_generation.load(Ordering::SeqCst);
        assert_ne!(start(&first), start(&second));
        assert_ne!(start(&first), 0);
    }
}
