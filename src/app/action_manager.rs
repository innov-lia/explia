//! Gestionnaire central des actions.
//!
//! - `Command` : les commandes proposées à l'utilisateur (menus, boutons, raccourcis), chacune
//!   définie une seule fois avec son libellé, son raccourci et l'action qu'elle déclenche.
//! - `Action` : ce que l'interface demande pendant une frame (gestes traduits, commandes).
//! - `ExpliaApp::apply` : exécute une action ; les opérations sur les fichiers passent par
//!   les modules de stockage, jamais par l'interface.

use super::*;
use event_manager::Shortcut;

/// Commande de l'utilisateur sur un élément (ou la sélection dont il fait partie).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    /// Dossier : l'ouvrir. Fichier : l'ouvrir, le lire (audio, vidéo) ou l'exécuter.
    Open,
    /// Fichier lisible : l'ouvrir avec l'application par défaut de Windows.
    OpenExternally,
    /// Exécutable : le lancer en tant qu'administrateur (UAC).
    RunAsAdmin,
    OpenInNewTab,
    /// Ouvre le dossier qui contient l'élément et l'y sélectionne.
    ShowInFolder,
    /// Place les éléments dans le presse-papiers de Windows (copie au collage).
    Copy,
    /// Place les éléments dans le presse-papiers de Windows (déplacement au collage).
    Cut,
    /// Colle le presse-papiers dans le dossier.
    Paste,
    /// Crée un dossier (« Nouveau dossier ») dans le dossier visé.
    CreateFolder,
    /// Fenêtre « Propriétés » de Windows.
    ShowProperties,
    CopyPath,
    Rename,
    /// Supprime (corbeille, après confirmation).
    Delete,
    AddFavorite,
    RemoveFavorite,
    AddApplication,
    RemoveApplication,
    PowerShell,
    PowerShellAdmin,
}

impl Command {
    /// Libellé du menu pour `target` ; `count` : nombre d'éléments visés.
    pub(super) fn label(self, target: &Path, is_dir: bool, count: usize) -> String {
        let text = match self {
            Command::Open if is_dir => "Ouvrir",
            Command::Open if launch::is_executable(target) => "Exécuter",
            Command::Open if is_playable(target) => "Lire",
            Command::Open => "Ouvrir",
            Command::OpenExternally => "Ouvrir avec le lecteur externe",
            Command::RunAsAdmin if launch::is_executable(target) => {
                "Exécuter en tant qu'administrateur"
            }
            Command::RunAsAdmin => "Lancer en tant qu'administrateur",
            Command::OpenInNewTab => "🗂 Ouvrir dans un nouvel onglet",
            Command::ShowInFolder => "Afficher dans son dossier",
            Command::Copy if count > 1 => {
                return format!("📋 Copier ({count} éléments)  ({})", Shortcut::Copy.label());
            }
            Command::Copy => "📋 Copier",
            Command::Cut if count > 1 => {
                return format!("✂ Couper ({count} éléments)  ({})", Shortcut::Cut.label());
            }
            Command::Cut => "✂ Couper",
            Command::Paste => "📥 Coller dans ce dossier",
            Command::CreateFolder => "📁 Nouveau dossier",
            Command::ShowProperties => "Propriétés",
            Command::CopyPath => "Copier le chemin",
            Command::Rename => "Renommer",
            Command::Delete => "🗑 Supprimer",
            Command::AddFavorite => "Ajouter aux favoris",
            Command::RemoveFavorite => "Retirer des favoris",
            Command::AddApplication => "Ajouter aux applications",
            Command::RemoveApplication => "Retirer des applications",
            Command::PowerShell => ">_ PowerShell ici",
            Command::PowerShellAdmin => ">_ PowerShell ici (administrateur)",
        };
        match self.shortcut() {
            Some(shortcut) => format!("{text}  ({shortcut})"),
            None => text.to_string(),
        }
    }

    /// Raccourci clavier affiché à côté du libellé (déclaré dans `event_manager`).
    pub(super) fn shortcut(self) -> Option<&'static str> {
        let shortcut = match self {
            Command::Copy => Shortcut::Copy,
            Command::Cut => Shortcut::Cut,
            Command::Rename => Shortcut::Rename,
            Command::ShowProperties => Shortcut::Properties,
            Command::Delete => Shortcut::Delete,
            _ => return None,
        };
        Some(shortcut.label())
    }

    /// Bulle d'aide éventuelle.
    pub(super) fn hint(self) -> Option<&'static str> {
        match self {
            Command::Copy => {
                Some("À coller dans l'Explorateur, sur le Bureau ou ailleurs (Ctrl+V)")
            }
            Command::Cut => {
                Some("À coller dans un autre dossier (Ctrl+V) : les éléments y sont déplacés")
            }
            Command::CreateFolder => Some("Crée un dossier vide, puis le renomme (Ctrl+Maj+N)"),
            _ => None,
        }
    }

    /// Action déclenchée sur `target` ; `targets` : tous les éléments visés (multi-sélection).
    pub(super) fn action(self, target: &Entry, targets: Vec<Entry>) -> Action {
        let paths = || targets.iter().map(|e| e.path.clone()).collect();
        let path = target.path.clone();
        match self {
            Command::Open if target.is_dir => Action::Go(Location::Dir(path)),
            Command::Open => open_action(&path),
            Command::OpenExternally => Action::Open(path),
            Command::RunAsAdmin => Action::OpenAsAdmin(path),
            Command::OpenInNewTab => Action::NewTab(path),
            Command::ShowInFolder => match path.parent() {
                Some(dir) => Action::GoSelect {
                    dir: dir.to_path_buf(),
                    file: path,
                },
                None => Action::Go(Location::Dir(path)),
            },
            Command::Copy => Action::CopyFiles(paths()),
            Command::Cut => Action::CutFiles(paths()),
            Command::CreateFolder => Action::CreateFolder { parent: path },
            Command::ShowProperties => Action::ShowProperties(path),
            Command::Paste => Action::Paste { dest: path },
            Command::CopyPath => Action::CopyPath(path),
            Command::Rename => Action::StartRename(target.clone()),
            Command::Delete => Action::AskDeleteEntries(targets),
            Command::AddFavorite => Action::AddFavorite(path),
            Command::RemoveFavorite => Action::RemoveFavorite(path),
            Command::AddApplication => Action::AddApplication(path),
            Command::RemoveApplication => Action::RemoveApplication(path),
            Command::PowerShell => Action::OpenPowerShell {
                dir: path,
                admin: false,
            },
            Command::PowerShellAdmin => Action::OpenPowerShell {
                dir: path,
                admin: true,
            },
        }
    }
}

/// Action demandée par l'interface pendant une frame, puis exécutée par `ExpliaApp::apply`.
/// Les gestes (clics, touches) sont traduits en actions ; les commandes de l'utilisateur
/// (`Command`) aussi.
pub(super) enum Action {
    Go(Location),
    /// Clic sur un dossier de la barre latérale : l'ouvre, et les flèches ↑ / ↓ parcourent
    /// ensuite la liste de l'onglet.
    SidebarGo(PathBuf),
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
    /// Sélectionne ces éléments, puis demande confirmation avant de les supprimer (menu).
    AskDeleteEntries(Vec<Entry>),
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
    /// Lance la recherche des doublons, quelle que soit leur taille (selon l'option
    /// « Récursive »).
    FindDuplicates,
    /// Lance la liste des fichiers plus gros que le seuil en Mo (selon l'option « Récursive »).
    FindLargeFiles,
    /// Sélectionne les copies : tout sauf le premier fichier de chaque groupe de doublons.
    SelectDuplicateCopies,
    /// Ferme les résultats et revient au contenu du dossier.
    CloseResults,
    /// Passe la vidéo en plein écran, ou en sort.
    ToggleVideoFullscreen,
    /// Copie le chemin complet dans le presse-papiers (texte).
    CopyPath(PathBuf),
    /// Copie des fichiers ou dossiers dans le presse-papiers de Windows (à coller ailleurs).
    CopyFiles(Vec<PathBuf>),
    /// Coupe des fichiers ou dossiers : le prochain collage les déplacera.
    CutFiles(Vec<PathBuf>),
    /// Crée un dossier dans `parent` (affiché au besoin), puis le passe en renommage.
    CreateFolder {
        parent: PathBuf,
    },
    /// Ouvre la fenêtre « Propriétés » de Windows.
    ShowProperties(PathBuf),
    /// Colle dans `dest` les fichiers et dossiers du presse-papiers : copie, ou déplacement
    /// s'ils ont été coupés (jamais d'écrasement).
    Paste {
        dest: PathBuf,
    },
    /// Copie des éléments déposés dans la fenêtre vers `dest` (sans écrasement).
    CopyInto {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// Glisse des éléments vers une autre application (Bureau, Explorateur), en copie.
    DragOut(Vec<PathBuf>),
    /// Ouvre un dossier dans un nouvel onglet.
    NewTab(PathBuf),
    /// Affiche l'onglet donné.
    SwitchTab(usize),
    /// Ferme l'onglet donné.
    CloseTab(usize),
    /// Ouvre PowerShell dans un dossier, éventuellement en administrateur (UAC).
    OpenPowerShell {
        dir: PathBuf,
        admin: bool,
    },
    /// Demande la suppression d'un dossier qui n'est pas une ligne de la liste (le dossier
    /// affiché lui-même, ou un dossier parent d'un résultat).
    AskDeleteFolder(PathBuf),
    /// Clic sur un en-tête de colonne : trie dessus, ou inverse l'ordre s'il est déjà actif.
    SortBy(SortKey),
    /// Affiche ou masque les dossiers et fichiers cachés.
    ToggleHidden,
    /// Ctrl+clic (ou Maj+clic) dans une colonne qui n'est pas la dernière (vue en colonnes) :
    /// cette colonne devient le dossier affiché, l'élément qui y était en surbrillance est
    /// sélectionné, puis `entry` est ajouté (ou la plage jusqu'à `entry` si `range`).
    ColumnMultiSelect {
        dir: PathBuf,
        anchor: Option<PathBuf>,
        entry: Entry,
        range: bool,
    },
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

impl ExpliaApp {
    /// Exécute une action : seul point d'entrée des changements d'état demandés par
    /// l'interface.
    pub(super) fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Go(location) => self.navigate(location),
            Action::SidebarGo(path) => {
                let index = self.sidebar_items().iter().position(|p| *p == path);
                self.open_location(Location::Dir(path), AfterListing::None, index);
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
            Action::Open(path) => {
                let result = self
                    .storage
                    .open(&ItemId::path(&path))
                    .map_err(|e| e.to_string());
                self.report_launch(result);
            }
            Action::Play(path) => self.start_player(path),
            Action::StopPlayer => self.stop_player(),
            Action::OpenAsAdmin(path) => {
                let result = self
                    .storage
                    .open_as_admin(&ItemId::path(&path))
                    .map_err(|e| e.to_string());
                self.report_launch(result);
            }
            Action::EditAddress => {
                self.nav.address_editing = true;
                self.nav.focus_address = true;
            }
            Action::LongPress(entry) => {
                self.long_press_done = true;
                // Un clic maintenu sur l'élément sélectionné démarre la multi-sélection avec lui.
                if self.selection.multi.is_empty()
                    && self
                        .selection
                        .single
                        .as_ref()
                        .is_some_and(|s| s.path == entry.path)
                {
                    self.selection.single = None;
                    self.preview.state = PreviewState::None;
                }
                self.toggle_multi(entry);
                // Après un clic maintenu, chaque clic ajoute ou retire une ligne.
                self.selection.toggle_mode = !self.selection.multi.is_empty();
            }
            Action::ToggleMulti(entry) => self.toggle_multi(entry),
            Action::SelectRange(entry) => self.select_range(entry),
            Action::ClearMulti => self.clear_multi(),
            Action::AskDelete => {
                self.confirm_delete = !self.delete_targets().is_empty();
                self.delete_asked_at = Some(ctx.input(|i| i.time));
            }
            Action::AskDeleteEntries(entries) => {
                if entries != self.delete_targets() {
                    self.clear_multi();
                    self.selection.single = None;
                    self.preview.state = PreviewState::None;
                    match <[Entry; 1]>::try_from(entries) {
                        Ok([entry]) => self.selection.select_one(entry),
                        Err(entries) => {
                            self.selection.anchor = entries.first().map(|e| e.path.clone());
                            self.selection.multi = entries;
                        }
                    }
                }
                self.confirm_delete = !self.delete_targets().is_empty();
                self.delete_asked_at = Some(ctx.input(|i| i.time));
            }
            Action::CancelDelete => self.cancel_delete(),
            Action::CopyPath(path) => {
                let text = path.display().to_string();
                ctx.copy_text(text.clone());
                self.error = None;
                self.notice = Some(format!("Chemin copié : {text}"));
            }
            Action::CopyFiles(paths) => match self.clipboard.copy(&paths) {
                Ok(()) => {
                    self.error = None;
                    self.notice = Some(format!(
                        "{} élément(s) copié(s) : collez-les dans l'Explorateur ou sur le Bureau (Ctrl+V).",
                        paths.len()
                    ));
                }
                Err(message) => self.error = Some(message),
            },
            Action::CutFiles(paths) => match self.clipboard.cut(&paths) {
                Ok(()) => {
                    self.error = None;
                    self.notice = Some(format!(
                        "{} élément(s) coupé(s) : collez-les dans un autre dossier (Ctrl+V) pour les y déplacer.",
                        paths.len()
                    ));
                }
                Err(message) => self.error = Some(message),
            },
            Action::Paste { dest } => match self.clipboard.content() {
                Ok(None) => {
                    self.notice = Some("Rien à coller : copiez d'abord des fichiers.".to_string());
                }
                Ok(Some(content)) => self.start_transfer(content.paths, dest, content.mode),
                Err(message) => self.error = Some(message),
            },
            Action::CopyInto { sources, dest } => {
                self.start_transfer(sources, dest, ClipMode::Copy);
            }
            Action::CreateFolder { parent } => self.create_folder(parent),
            Action::ShowProperties(path) => {
                if let Err(err) = self.storage.show_properties(&ItemId::path(&path)) {
                    self.error = Some(err.to_string());
                }
            }
            Action::DragOut(paths) => {
                self.drag_out_done = true;
                let result = dragdrop::drag_files_out(&paths);
                self.drag_out_ended_at = Some(ctx.input(|i| i.time));
                match result {
                    Ok(true) => {
                        self.error = None;
                        self.notice =
                            Some(format!("{} élément(s) déposé(s) (copie).", paths.len()));
                    }
                    Ok(false) => {}
                    Err(message) => self.error = Some(message),
                }
            }
            Action::NewTab(path) => {
                if self.nav.tabs.is_empty()
                    && let Location::Dir(current) = &self.nav.location
                {
                    self.nav.tabs.push(current.clone());
                    self.nav.active_tab = 0;
                }
                self.nav.tabs.push(path.clone());
                self.nav.active_tab = self.nav.tabs.len() - 1;
                self.navigate(Location::Dir(path));
            }
            Action::SwitchTab(index) => {
                if let Some(path) = self.nav.tabs.get(index).cloned() {
                    self.nav.active_tab = index;
                    self.navigate(Location::Dir(path));
                }
            }
            Action::CloseTab(index) => self.close_tab(index),
            Action::OpenPowerShell { dir, admin } => {
                self.report_launch(launch::open_powershell(&dir, admin));
            }
            Action::ToggleVideoFullscreen => {
                self.set_video_fullscreen(ctx, !self.player.fullscreen)
            }
            Action::FindDuplicates => self.start_scan(ResultKind::Duplicates),
            Action::FindLargeFiles => self.start_scan(ResultKind::LargeFiles),
            Action::SelectDuplicateCopies => {
                // On garde le premier fichier de chaque groupe ; les autres sont sélectionnés.
                let mut seen = HashSet::new();
                let copies: Vec<Entry> = self
                    .visible
                    .iter()
                    .filter(|e| {
                        self.scan
                            .duplicate_groups
                            .get(&e.path)
                            .is_some_and(|group| !seen.insert(*group))
                    })
                    .cloned()
                    .collect();
                if !copies.is_empty() {
                    self.clear_multi();
                    self.selection.single = None;
                    self.preview.state = PreviewState::None;
                    self.selection.anchor = copies.first().map(|e| e.path.clone());
                    self.selection.multi = copies;
                }
            }
            Action::CloseResults => {
                let was_search = self
                    .scan
                    .results
                    .as_ref()
                    .is_some_and(|r| r.kind == ResultKind::Search);
                self.close_results();
                if was_search {
                    self.scan.search.clear();
                }
                self.clear_multi();
                self.selection.single = None;
                self.preview.state = PreviewState::None;
                self.recompute_visible();
            }
            Action::AskDeleteFolder(path) => {
                if let Some(entry) = fs_nav::entry_from_path(&path) {
                    self.clear_multi();
                    self.selection.single = Some(entry);
                    self.confirm_delete = true;
                    self.delete_asked_at = Some(ctx.input(|i| i.time));
                }
            }
            Action::ConfirmDelete => self.delete_selection(),
            Action::EnterFolder(path) => {
                self.open_location(Location::Dir(path), AfterListing::SelectFirst, None);
            }
            Action::SetViewMode(mode) => {
                self.view_mode = mode;
                self.nav.scroll_to_index = self.cursor_index();
            }
            Action::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.invalidate_columns();
                self.recompute_visible();
            }
            Action::GoSelect { dir, file } => {
                let then = AfterListing::Select(ItemId::path(file));
                self.open_location(Location::Dir(dir), then, None);
            }
            Action::ColumnMultiSelect {
                dir,
                anchor,
                entry,
                range,
            } => {
                let then = AfterListing::Extend {
                    anchor: anchor.map(ItemId::path),
                    entry: Box::new(entry),
                    range,
                };
                self.open_location(Location::Dir(dir), then, None);
            }
            Action::StartRename(entry) => self.start_rename(&entry),
            Action::CommitRename { path, new_name } => self.commit_rename(path, &new_name),
            Action::CancelRename => self.renaming = None,
        }
    }
}
