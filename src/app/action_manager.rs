//! Gestionnaire central des actions.
//!
//! - `Command` : les commandes proposées à l'utilisateur (menus, boutons, raccourcis), chacune
//!   définie une seule fois avec son libellé, son raccourci et l'action qu'elle déclenche.
//! - `Action` : ce que l'interface demande pendant une frame (gestes traduits, commandes).
//! - `ExpliaApp::apply` : exécute une action ; les opérations sur les fichiers passent par
//!   les modules de stockage, jamais par l'interface.

use super::*;

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
    /// Place les éléments dans le presse-papiers de Windows.
    Copy,
    /// Colle le presse-papiers dans le dossier.
    Paste,
    CopyPath,
    Rename,
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
                return format!("📋 Copier ({count} éléments)  (Ctrl+C)");
            }
            Command::Copy => "📋 Copier",
            Command::Paste => "📥 Coller dans ce dossier",
            Command::CopyPath => "Copier le chemin",
            Command::Rename => "Renommer",
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
        match self {
            Command::Copy => Some("Ctrl+C"),
            Command::Rename => Some("F2"),
            _ => None,
        }
    }

    /// Bulle d'aide éventuelle.
    pub(super) fn hint(self) -> Option<&'static str> {
        match self {
            Command::Copy => {
                Some("À coller dans l'Explorateur, sur le Bureau ou ailleurs (Ctrl+V)")
            }
            _ => None,
        }
    }

    /// Action déclenchée sur `target` ; `targets` : tous les éléments visés (multi-sélection).
    pub(super) fn action(self, target: &Entry, targets: Vec<PathBuf>) -> Action {
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
            Command::Copy => Action::CopyFiles(targets),
            Command::Paste => Action::Paste { dest: path },
            Command::CopyPath => Action::CopyPath(path),
            Command::Rename => Action::StartRename(target.clone()),
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
    /// Lance la recherche des dossiers vides (selon l'option « Récursive »).
    FindEmptyFolders,
    /// Lance la recherche des doublons (selon l'option « Récursive » et le seuil en Mo).
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
    /// Colle dans `dest` les fichiers et dossiers du presse-papiers (copie, sans écrasement).
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
                self.navigate(Location::Dir(path));
                if index.is_some() {
                    self.sidebar_focus = true;
                    self.sidebar_cursor = index;
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
                if self.selection.multi.is_empty()
                    && self
                        .selection
                        .single
                        .as_ref()
                        .is_some_and(|s| s.path == entry.path)
                {
                    self.selection.single = None;
                    self.preview = PreviewState::None;
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
            Action::CancelDelete => self.cancel_delete(),
            Action::CopyPath(path) => {
                let text = path.display().to_string();
                ctx.copy_text(text.clone());
                self.error = None;
                self.notice = Some(format!("Chemin copié : {text}"));
            }
            Action::CopyFiles(paths) => match clipboard::copy_files(&paths) {
                Ok(()) => {
                    self.error = None;
                    self.notice = Some(format!(
                        "{} élément(s) copié(s) : collez-les dans l'Explorateur ou sur le Bureau (Ctrl+V).",
                        paths.len()
                    ));
                }
                Err(message) => self.error = Some(message),
            },
            Action::Paste { dest } => match clipboard::clipboard_files() {
                Ok(files) if files.is_empty() => {
                    self.notice = Some("Rien à coller : copiez d'abord des fichiers.".to_string());
                }
                Ok(files) => self.start_copy(files, dest),
                Err(message) => self.error = Some(message),
            },
            Action::CopyInto { sources, dest } => self.start_copy(sources, dest),
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
                if self.tabs.is_empty()
                    && let Location::Dir(current) = &self.location
                {
                    self.tabs.push(current.clone());
                    self.active_tab = 0;
                }
                self.tabs.push(path.clone());
                self.active_tab = self.tabs.len() - 1;
                self.navigate(Location::Dir(path));
            }
            Action::SwitchTab(index) => {
                if let Some(path) = self.tabs.get(index).cloned() {
                    self.active_tab = index;
                    self.navigate(Location::Dir(path));
                }
            }
            Action::CloseTab(index) => self.close_tab(index),
            Action::OpenPowerShell { dir, admin } => {
                self.report_launch(launch::open_powershell(&dir, admin));
            }
            Action::ToggleVideoFullscreen => self.set_video_fullscreen(ctx, !self.video_fullscreen),
            Action::FindEmptyFolders => self.start_scan(ResultKind::EmptyFolders),
            Action::FindDuplicates => self.start_scan(ResultKind::Duplicates),
            Action::FindLargeFiles => self.start_scan(ResultKind::LargeFiles),
            Action::SelectDuplicateCopies => {
                // On garde le premier fichier de chaque groupe ; les autres sont sélectionnés.
                let mut seen = HashSet::new();
                let copies: Vec<Entry> = self
                    .visible
                    .iter()
                    .filter(|e| {
                        self.duplicate_groups
                            .get(&e.path)
                            .is_some_and(|group| !seen.insert(*group))
                    })
                    .cloned()
                    .collect();
                if !copies.is_empty() {
                    self.clear_multi();
                    self.selection.single = None;
                    self.preview = PreviewState::None;
                    self.selection.anchor = copies.first().map(|e| e.path.clone());
                    self.selection.multi = copies;
                }
            }
            Action::CloseResults => {
                let was_search = self
                    .results
                    .as_ref()
                    .is_some_and(|r| r.kind == ResultKind::Search);
                self.close_results();
                if was_search {
                    self.search.clear();
                }
                self.clear_multi();
                self.selection.single = None;
                self.preview = PreviewState::None;
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
            Action::ColumnMultiSelect {
                dir,
                anchor,
                entry,
                range,
            } => {
                self.select_path_after_nav = anchor;
                self.navigate(Location::Dir(dir.clone()));
                if self.location == Location::Dir(dir) {
                    let target = self
                        .visible
                        .iter()
                        .find(|e| e.path == entry.path)
                        .cloned()
                        .unwrap_or(entry);
                    if range {
                        self.select_range(target);
                    } else {
                        self.toggle_multi(target);
                    }
                }
            }
            Action::StartRename(entry) => self.start_rename(&entry),
            Action::CommitRename { path, new_name } => self.commit_rename(ctx, path, &new_name),
            Action::CancelRename => self.renaming = None,
        }
    }
}
