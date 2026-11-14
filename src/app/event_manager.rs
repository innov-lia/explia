//! Gestionnaire central des événements.
//!
//! egui fournit les événements natifs (clics, double-clics, clics droits, appuis longs,
//! clavier, fichiers déposés) ; ce module les lit à un seul endroit et les traduit en
//! `Action`, sans rien exécuter lui-même :
//! - `Pointer` décrit ce qui s'est passé sur un élément (ligne ou tuile), indépendamment de
//!   la vue ; `translate` en déduit l'action selon la vue (règles testées sans interface) ;
//! - `Shortcut` déclare tous les raccourcis clavier ;
//! - `input_events` traite le clavier et la fenêtre (raccourcis, flèches, dépôt de fichiers).
//!
//! Le menu contextuel (clic droit) est confié à `context_menu_manager`.

use super::*;

/// Raccourcis clavier d'ExPLIA, déclarés une seule fois.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shortcut {
    Find,
    NewTab,
    CloseTab,
    Copy,
    Cut,
    Paste,
    Rename,
    Delete,
    NewFolder,
    Properties,
    /// Confirme la suppression demandée.
    Confirm,
    /// Annule la confirmation, ou quitte la multi-sélection.
    Cancel,
    /// Lecture / pause du lecteur intégré.
    PlayPause,
    /// Affichage fractionné : copie la sélection vers l'autre volet.
    CopyToOther,
    /// Affichage fractionné : déplace la sélection vers l'autre volet.
    MoveToOther,
    /// Affichage fractionné : donne la main à l'autre volet.
    SwitchPane,
}

impl Shortcut {
    /// Texte du raccourci, tel qu'affiché dans les menus et les bulles d'aide.
    pub(super) fn label(self) -> &'static str {
        match self {
            Shortcut::Find => "Ctrl+F",
            Shortcut::NewTab => "Ctrl+T",
            Shortcut::CloseTab => "Ctrl+W",
            Shortcut::Copy => "Ctrl+C",
            Shortcut::Cut => "Ctrl+X",
            Shortcut::Paste => "Ctrl+V",
            Shortcut::Rename => "F2",
            Shortcut::Delete => "Suppr",
            Shortcut::NewFolder => "Ctrl+Maj+N",
            Shortcut::Properties => "Alt+Entrée",
            Shortcut::Confirm => "Entrée",
            Shortcut::Cancel => "Échap",
            Shortcut::PlayPause => "Espace",
            Shortcut::CopyToOther => "F5",
            Shortcut::MoveToOther => "F6",
            Shortcut::SwitchPane => "Tab",
        }
    }

    /// Touche et modificateurs exigés. Copier, Couper et Coller sont lus autrement (voir
    /// `pressed`).
    fn binding(self) -> (egui::Key, egui::Modifiers) {
        let ctrl = egui::Modifiers::COMMAND;
        let none = egui::Modifiers::NONE;
        match self {
            Shortcut::Find => (egui::Key::F, ctrl),
            Shortcut::NewTab => (egui::Key::T, ctrl),
            Shortcut::CloseTab => (egui::Key::W, ctrl),
            Shortcut::Copy => (egui::Key::C, ctrl),
            Shortcut::Cut => (egui::Key::X, ctrl),
            Shortcut::Paste => (egui::Key::V, ctrl),
            Shortcut::Rename => (egui::Key::F2, none),
            Shortcut::Delete => (egui::Key::Delete, none),
            Shortcut::NewFolder => (egui::Key::N, ctrl | egui::Modifiers::SHIFT),
            Shortcut::Properties => (egui::Key::Enter, egui::Modifiers::ALT),
            Shortcut::Confirm => (egui::Key::Enter, none),
            Shortcut::Cancel => (egui::Key::Escape, none),
            Shortcut::PlayPause => (egui::Key::Space, none),
            Shortcut::CopyToOther => (egui::Key::F5, none),
            Shortcut::MoveToOther => (egui::Key::F6, none),
            Shortcut::SwitchPane => (egui::Key::Tab, none),
        }
    }

    /// Comme `pressed`, mais retire la touche des événements (egui ne la voit plus).
    fn consume(self, input: &mut egui::InputState) -> bool {
        let (key, modifiers) = self.binding();
        input.consume_key(modifiers, key)
    }

    /// Le raccourci vient d'être pressé. Ctrl+C arrive d'egui comme un événement « Copier ».
    /// Ctrl+V n'est pas signalé par egui quand le presse-papiers ne contient que des fichiers :
    /// il est lu sur le clavier de Windows (`clipboard::paste_keys_down`) et vaut tant qu'il
    /// est enfoncé ; l'appelant détecte l'appui.
    fn pressed(self, input: &egui::InputState) -> bool {
        match self {
            Shortcut::Copy => return input.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            Shortcut::Cut => return input.events.iter().any(|e| matches!(e, egui::Event::Cut)),
            Shortcut::Paste => return input.focused && clipboard::paste_keys_down(),
            _ => {}
        }
        let (key, modifiers) = self.binding();
        // Les modificateurs demandés doivent être enfoncés ; Entrée seule ne vaut pas Alt+Entrée.
        let alt_ok = modifiers.alt || !input.modifiers.alt || key != egui::Key::Enter;
        input.modifiers.contains(modifiers) && alt_ok && input.key_pressed(key)
    }
}

/// Ce qui s'est passé pendant la frame sur un élément (ligne, tuile, colonne).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Pointer {
    pub clicked: bool,
    pub double_clicked: bool,
    pub ctrl: bool,
    pub shift: bool,
    /// Bouton maintenu assez longtemps sans bouger (souris).
    pub long_press: bool,
    /// Appui long sur écran tactile.
    pub long_touch: bool,
    /// Appui puis déplacement : glisser vers une autre application.
    pub drag_out: bool,
}

/// Vue où se trouve l'élément : les mêmes gestes n'y ont pas tout à fait le même sens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ItemView<'a> {
    /// Liste détaillée (et résultats).
    List,
    /// Grilles d'images et de vidéos, colonne de navigation.
    Grid,
    /// Vue en colonnes : `dir` est le dossier de la colonne, `highlighted` l'élément qui y est
    /// en surbrillance, `is_last` vrai pour la colonne du dossier affiché.
    Column {
        dir: &'a Path,
        highlighted: Option<&'a Path>,
        is_last: bool,
    },
}

/// État de la sélection utile à la traduction des gestes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct GestureState {
    /// Multi-sélection démarrée par un clic maintenu : un clic ajoute ou retire l'élément.
    pub toggle_mode: bool,
    /// Un clic maintenu vient d'être traité : le relâchement n'est pas un clic.
    pub long_press_done: bool,
}

/// Ouvrir un élément : entrer dans un dossier, lire ou ouvrir un fichier.
fn open_entry(entry: &Entry) -> Action {
    if entry.is_dir {
        Action::Go(Location::Dir(entry.path.clone()))
    } else {
        open_action(&entry.path)
    }
}

/// Traduit les gestes sur `entry` en action. `drag_targets` : éléments emportés par un
/// glisser (la multi-sélection si `entry` en fait partie).
pub(super) fn translate(
    pointer: Pointer,
    entry: &Entry,
    view: ItemView,
    state: GestureState,
    drag_targets: impl FnOnce() -> Vec<PathBuf>,
) -> Option<Action> {
    let Pointer {
        clicked,
        double_clicked,
        ctrl,
        shift,
        long_press,
        long_touch,
        drag_out,
    } = pointer;
    match view {
        ItemView::List => {
            // Un clic l'emporte sur le glisser, qui l'emporte sur l'appui long.
            if clicked && ctrl {
                Some(Action::ToggleMulti(entry.clone()))
            } else if clicked && shift {
                Some(Action::SelectRange(entry.clone()))
            } else if state.long_press_done && (clicked || double_clicked) {
                // Le relâchement qui suit un clic maintenu n'est pas un clic.
                None
            } else if double_clicked && !state.toggle_mode {
                Some(open_entry(entry))
            } else if clicked {
                Some(if state.toggle_mode {
                    Action::ToggleMulti(entry.clone())
                } else {
                    Action::Select(entry.clone())
                })
            } else if drag_out {
                Some(Action::DragOut(drag_targets()))
            } else if long_touch || (long_press && !state.long_press_done) {
                Some(Action::LongPress(entry.clone()))
            } else {
                None
            }
        }
        ItemView::Grid => {
            if drag_out {
                Some(Action::DragOut(drag_targets()))
            } else if clicked && ctrl {
                Some(Action::ToggleMulti(entry.clone()))
            } else if clicked && shift {
                Some(Action::SelectRange(entry.clone()))
            } else if double_clicked {
                Some(open_entry(entry))
            } else if clicked {
                Some(Action::Select(entry.clone()))
            } else {
                None
            }
        }
        ItemView::Column {
            dir,
            highlighted,
            is_last,
        } => {
            if drag_out {
                Some(Action::DragOut(drag_targets()))
            } else if is_last && clicked && ctrl {
                Some(Action::ToggleMulti(entry.clone()))
            } else if is_last && clicked && shift {
                Some(Action::SelectRange(entry.clone()))
            } else if !is_last && clicked && (ctrl || shift) {
                // Colonne précédente : la multi-sélection part de l'élément en surbrillance.
                Some(Action::ColumnMultiSelect {
                    dir: dir.to_path_buf(),
                    anchor: highlighted.map(Path::to_path_buf),
                    entry: entry.clone(),
                    range: shift && !ctrl,
                })
            } else if double_clicked && !entry.is_dir {
                Some(open_action(&entry.path))
            } else if clicked {
                Some(if entry.is_dir {
                    Action::Go(Location::Dir(entry.path.clone()))
                } else if is_last {
                    Action::Select(entry.clone())
                } else {
                    Action::GoSelect {
                        dir: dir.to_path_buf(),
                        file: entry.path.clone(),
                    }
                })
            } else {
                None
            }
        }
    }
}

/// Zones de dépôt de la dernière image, partagées avec la cible de dépôt de Windows (qui
/// répond pendant le survol, hors des images d'egui).
#[derive(Debug, Default)]
pub(super) struct DropZones {
    /// Dossiers visibles (lignes, lecteurs, favoris, autre volet) et leur rectangle, en points.
    pub zones: Vec<(egui::Rect, PathBuf)>,
    /// Dossier affiché : reçoit ce qui est déposé ailleurs dans la fenêtre.
    pub fallback: Option<PathBuf>,
    pub pixels_per_point: f32,
}

/// Dossier qui recevrait des fichiers venus d'une autre application, et copie ou déplacement
/// (comme l'Explorateur, dans la limite de ce que la source autorise). `None` : refusé.
pub(super) fn resolve_drop(
    zones: &DropZones,
    request: &droptarget::DropRequest,
) -> Option<(PathBuf, droptarget::DropKind)> {
    let scale = zones.pixels_per_point.max(0.1);
    let position = egui::pos2(request.x / scale, request.y / scale);
    let paths = &request.paths;
    let dest = zones
        .zones
        .iter()
        .find(|(rect, dir)| rect.contains(position) && accepts_drop(paths, dir))
        .map(|(_, dir)| dir.clone())
        .or_else(|| {
            zones
                .fallback
                .clone()
                .filter(|dir| accepts_drop(paths, dir))
        })?;
    let wanted = drop_mode(paths, &dest, request.ctrl, request.shift);
    let kind = match wanted {
        ClipMode::Cut if request.can_move => droptarget::DropKind::Move,
        ClipMode::Copy if request.can_copy => droptarget::DropKind::Copy,
        _ if request.can_copy => droptarget::DropKind::Copy,
        _ if request.can_move => droptarget::DropKind::Move,
        _ => return None,
    };
    Some((dest, kind))
}

/// Un dépôt dans `dir` a un sens : ce n'est ni un élément glissé, ni l'un de ses
/// sous-dossiers, ni le dossier d'où viennent déjà tous les éléments.
pub(super) fn accepts_drop(sources: &[PathBuf], dir: &Path) -> bool {
    !sources.is_empty()
        && !sources.iter().any(|source| dir.starts_with(source))
        && !sources.iter().all(|source| source.parent() == Some(dir))
}

/// Copie ou déplacement, comme dans l'Explorateur : Ctrl copie, Maj déplace ; sans touche, on
/// déplace sur un même lecteur et on copie vers un autre lecteur.
pub(super) fn drop_mode(sources: &[PathBuf], dest: &Path, ctrl: bool, shift: bool) -> ClipMode {
    if ctrl {
        ClipMode::Copy
    } else if shift || sources.iter().all(|source| same_volume(source, dest)) {
        ClipMode::Cut
    } else {
        ClipMode::Copy
    }
}

/// Deux chemins sur le même lecteur (même lettre, ou même partage réseau).
fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |path: &Path| {
        path.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
    };
    root(a).is_some() && root(a) == root(b)
}

impl ExpliaApp {
    /// Lit les gestes egui sur un élément et les traduit en action (`translate`).
    pub(super) fn item_events(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        entry: &Entry,
        view: ItemView,
    ) -> Option<Action> {
        let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pointer = Pointer {
            clicked: response.clicked(),
            double_clicked: response.double_clicked(),
            ctrl,
            shift,
            // Le clic maintenu ne sert qu'à la liste (multi-sélection sans clavier).
            long_press: view == ItemView::List && self.long_press(ui, response),
            long_touch: view == ItemView::List && response.long_touched(),
            drag_out: self.drag_out(ui, response),
        };
        let state = GestureState {
            toggle_mode: self.selection.toggle_mode && !self.selection.multi.is_empty(),
            long_press_done: self.long_press_done,
        };
        let action = translate(pointer, entry, view, state, || {
            self.selection.paths_from(entry)
        });
        // Le glisser commence dans la fenêtre (vers un dossier, l'autre volet…) ; hors de la
        // fenêtre, il repart vers les autres applications (voir `drag_events`).
        match action {
            Some(Action::DragOut(paths)) => Some(Action::DragStart(paths)),
            other => other,
        }
    }

    /// Bouton maintenu sur l'élément depuis `LONG_PRESS_SECONDS`, sans bouger plus que la
    /// tolérance. La détection suit la position du pointeur, pas l'état du widget.
    fn long_press(&self, ui: &egui::Ui, response: &egui::Response) -> bool {
        if self.long_press_done {
            return false;
        }
        let held = ui.input(|i| {
            if !i.pointer.primary_down() {
                return None;
            }
            let origin = i.pointer.press_origin()?;
            let position = i.pointer.latest_pos()?;
            let started = i.pointer.press_start_time()?;
            let on_this_item =
                response.rect.contains(origin) && origin.distance(position) <= LONG_PRESS_TOLERANCE;
            on_this_item.then_some(i.time - started)
        });
        match held {
            Some(seconds) if seconds >= LONG_PRESS_SECONDS => true,
            Some(_) => {
                // Réveille l'interface pour détecter la fin du délai.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
                false
            }
            None => false,
        }
    }

    /// Appui maintenu sur l'élément et pointeur déplacé : début d'un glisser vers l'extérieur.
    fn drag_out(&self, ui: &egui::Ui, response: &egui::Response) -> bool {
        // L'appui a commencé sur cet élément (lignes et tuiles sentent le glisser).
        let pressed_here = response.is_pointer_button_down_on()
            || ui
                .input(|i| {
                    i.pointer
                        .press_origin()
                        .filter(|_| i.pointer.primary_down())
                })
                .is_some_and(|origin| response.rect.contains(origin));
        if self.drag_out_done || !pressed_here {
            return false;
        }
        ui.input(
            |i| match (i.pointer.press_origin(), i.pointer.latest_pos()) {
                (Some(origin), Some(position)) => origin.distance(position) > DRAG_OUT_DISTANCE,
                _ => false,
            },
        )
    }

    /// Événements du clavier et de la fenêtre pour cette frame : raccourcis, flèches,
    /// fichiers déposés. Renvoie l'action à exécuter.
    pub(super) fn input_events(&mut self, ctx: &egui::Context) -> Option<Action> {
        if ctx.input(|i| i.pointer.primary_pressed()) {
            self.long_press_done = false;
            self.drag_out_done = false;
        }
        let pressed = |shortcut: Shortcut| ctx.input(|i| shortcut.pressed(i));
        // Échap annule la confirmation de suppression, sinon quitte la multi-sélection
        // (sauf pendant la saisie d'un chemin).
        if !self.nav.address_editing && !self.player.fullscreen && pressed(Shortcut::Cancel) {
            if self.confirm_delete {
                self.cancel_delete();
            } else if !self.selection.multi.is_empty() {
                self.clear_multi();
            }
        }
        let mut action = None;
        if pressed(Shortcut::Find) {
            self.scan.focus_search = true;
        }
        let typing =
            ctx.egui_wants_keyboard_input() || self.nav.address_editing || self.renaming.is_some();
        if !typing && let Location::Dir(current) = &self.nav.location {
            if pressed(Shortcut::NewTab) {
                action = Some(Action::NewTab(current.clone()));
            } else if pressed(Shortcut::CloseTab) && !self.nav.tabs.is_empty() {
                action = Some(Action::CloseTab(self.nav.active_tab));
            }
        }
        // Coller : détecté à l'appui de Ctrl+V.
        let paste_down = pressed(Shortcut::Paste);
        if paste_down
            && !self.paste_keys_were_down
            && !typing
            && self.clipboard.has_files()
            && let Location::Dir(current) = &self.nav.location
        {
            action = Some(Action::Paste {
                dest: current.clone(),
            });
        }
        self.paste_keys_were_down = paste_down;
        if let Some(drop) = self.dropped_files(ctx) {
            action = Some(drop);
        }
        if let Some(drop) = self.external_drops(ctx) {
            action = Some(drop);
        }
        // Après un glisser vers l'Explorateur, il déplace parfois les éléments un peu plus tard.
        if self
            .refresh_at
            .is_some_and(|at| ctx.input(|i| i.time) >= at)
        {
            self.refresh_at = None;
            self.refresh_listing(AfterListing::None);
        }
        if let Some(drag) = self.drag_events(ctx) {
            action = Some(drag);
        }
        // Raccourcis sur la sélection (jamais pendant une saisie) : F2 renomme ; Suppr demande
        // la suppression, puis Entrée la confirme (Échap l'annule).
        if !ctx.egui_wants_keyboard_input() && self.renaming.is_none() && !self.nav.address_editing
        {
            if pressed(Shortcut::PlayPause)
                && let PlayerSlot::Ready(active) = &self.player.slot
            {
                active.player.toggle_pause();
            }
            let single = match self.selection.multi.len() {
                0 => self.selection.single.as_ref(),
                1 => self.selection.multi.first(),
                _ => None,
            };
            let targets = self.delete_targets();
            let paths = || targets.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
            let current_dir = match &self.nav.location {
                Location::Dir(dir) => Some(dir.clone()),
                Location::ThisPc => None,
            };
            if pressed(Shortcut::Properties) {
                action = single
                    .map(|e| e.path.clone())
                    .or(current_dir)
                    .map(Action::ShowProperties);
            } else if pressed(Shortcut::Confirm) && self.confirm_delete {
                action = Some(Action::ConfirmDelete);
            } else if pressed(Shortcut::NewFolder)
                && let Some(parent) = current_dir
            {
                action = Some(Action::CreateFolder { parent });
            } else if pressed(Shortcut::Rename)
                && let Some(entry) = single
            {
                action = Some(Action::StartRename(entry.clone()));
            } else if pressed(Shortcut::Copy) && !targets.is_empty() {
                action = Some(Action::CopyFiles(paths()));
            } else if pressed(Shortcut::Cut) && !targets.is_empty() {
                action = Some(Action::CutFiles(paths()));
            } else if self.split.is_some() && pressed(Shortcut::CopyToOther) {
                action = Some(Action::TransferToOther {
                    mode: ClipMode::Copy,
                });
            } else if self.split.is_some() && pressed(Shortcut::MoveToOther) {
                action = Some(Action::TransferToOther {
                    mode: ClipMode::Cut,
                });
            } else if self.split.is_some()
                // Tab est retiré des événements : egui ne s'en sert pas pour changer de champ.
                && ctx.input_mut(|i| Shortcut::SwitchPane.consume(i))
            {
                action = Some(Action::SwitchPane);
            } else if pressed(Shortcut::Delete) && !targets.is_empty() {
                action = Some(Action::AskDelete);
            } else if !self.confirm_delete
                // Les flèches ne remplacent pas une action déjà décidée (Coller, dépôt…).
                && let Some(arrow) = self.arrow_key_action(ctx)
            {
                action = Some(arrow);
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
        // La barre latérale a la main (clic sur un lecteur, un dossier de Home ou un favori, ou
        // flèche gauche au plus haut d'un lecteur) : ↑ / ↓ changent d'élément, → rend la main
        // au dossier.
        if self.sidebar.focus {
            if up || down {
                self.step_sidebar(vertical);
            } else if right || (enter && gallery) {
                self.sidebar.focus = false;
                self.sidebar.cursor = None;
                self.nav.scroll_to_index = Some(0);
                return self.visible.first().cloned().map(Action::Select);
            }
            return None;
        }
        match &self.nav.location {
            Location::ThisPc => None,
            Location::Dir(_) => {
                // Retour arrière : dossier parent, dans tous les modes d'affichage.
                if backspace {
                    return self.nav.location.parent().map(Action::Go);
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
                        self.nav.scroll_to_index = Some(next);
                        return Some(if shift {
                            Action::SelectRange(entry)
                        } else {
                            Action::Select(entry)
                        });
                    }
                    return match &self.selection.single {
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
                    self.nav.scroll_to_index = Some(next);
                    return Some(if shift {
                        Action::SelectRange(entry)
                    } else {
                        Action::Select(entry)
                    });
                }
                if left {
                    return self.nav.location.parent().map(Action::Go);
                }
                match &self.selection.single {
                    Some(entry) if right && entry.is_dir && self.selection.multi.is_empty() => {
                        Some(Action::EnterFolder(entry.path.clone()))
                    }
                    _ => None,
                }
            }
        }
    }

    /// Glisser-déposer dans la fenêtre, à chaque image : dépôt sur le dossier visé (copie ou
    /// déplacement, voir `drop_mode`), abandon ailleurs, ou glisser vers une autre application
    /// dès que le pointeur sort de la fenêtre.
    fn drag_events(&mut self, ctx: &egui::Context) -> Option<Action> {
        let paths = self.drag.clone()?;
        let (released, down, position, ctrl, shift) = ctx.input(|i| {
            (
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.latest_pos(),
                i.modifiers.command,
                i.modifiers.shift,
            )
        });
        // Pointeur hors de la fenêtre : egui le voit sortir, et Windows le confirme (souris
        // capturée, egui peut garder la dernière position intérieure).
        let inside_window = position.is_some_and(|p| ctx.content_rect().contains(p))
            && ctx.input(|i| i.pointer.hover_pos().is_some())
            && self
                .hwnd
                .and_then(droptarget::cursor_inside)
                .unwrap_or(true);
        if released {
            self.drag = None;
            let dest = self.drop_target.borrow_mut().take()?;
            let mode = drop_mode(&paths, &dest, ctrl, shift);
            return Some(Action::TransferInto {
                sources: paths,
                dest,
                mode,
            });
        }
        if down && !inside_window {
            // Hors de la fenêtre : le glisser continue vers le Bureau ou l'Explorateur.
            self.drag = None;
            return Some(Action::DragOut(paths));
        }
        if !down {
            self.drag = None;
        }
        None
    }

    /// `response` (une ligne, un lecteur, un favori…) reçoit ce qu'on dépose sur elle dans
    /// `dir`. Pendant un glisser (depuis ExPLIA ou une autre application), si le pointeur est
    /// dessus, elle est encadrée et devient la cible.
    pub(super) fn drop_zone(&self, ui: &egui::Ui, response: &egui::Response, dir: &Path) {
        self.drop_zones
            .borrow_mut()
            .push((response.rect, dir.to_path_buf()));
        let over = self
            .drag_pointer(ui.ctx())
            .is_some_and(|p| response.rect.contains(p));
        if over && self.offer_drop_target(dir) {
            ui.painter().rect_stroke(
                response.rect,
                2.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Éléments en cours de glisser au-dessus de la fenêtre : ceux d'ExPLIA, sinon ceux d'une
    /// autre application.
    pub(super) fn dragged_paths(&self) -> Option<&[PathBuf]> {
        self.drag.as_deref().or_else(|| {
            self.external_hover
                .as_ref()
                .map(|hover| hover.paths.as_slice())
        })
    }

    /// Position du pointeur pendant un glisser au-dessus de la fenêtre.
    pub(super) fn drag_pointer(&self, ctx: &egui::Context) -> Option<egui::Pos2> {
        if self.drag.is_some() {
            ctx.input(|i| i.pointer.latest_pos())
        } else {
            self.external_hover.as_ref().map(|hover| hover.position)
        }
    }

    /// Propose `dir` comme cible du glisser en cours ; refusé si c'est un élément glissé (ou un
    /// de ses sous-dossiers) ou le dossier d'où viennent tous les éléments.
    pub(super) fn offer_drop_target(&self, dir: &Path) -> bool {
        let Some(sources) = self.dragged_paths() else {
            return false;
        };
        if !accepts_drop(sources, dir) {
            return false;
        }
        *self.drop_target.borrow_mut() = Some(dir.to_path_buf());
        true
    }

    /// Glisser venu d'une autre application (Explorateur, Bureau…) : survol en cours, et dépôts
    /// reçus par la cible de dépôt de Windows, à copier ou déplacer dans le dossier visé.
    fn external_drops(&mut self, ctx: &egui::Context) -> Option<Action> {
        let (hover, dropped) = {
            let mut events = self
                .external_drop
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (events.hover.clone(), std::mem::take(&mut events.dropped))
        };
        let scale = ctx.pixels_per_point().max(0.1);
        self.external_hover = hover.map(|request| ExternalHover {
            position: egui::pos2(request.x / scale, request.y / scale),
            ctrl: request.ctrl,
            shift: request.shift,
            paths: request.paths,
        });
        let zones = self
            .shared_zones
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut action = None;
        for (request, kind) in dropped {
            if let Some((dest, _)) = resolve_drop(&zones, &request) {
                action = Some(Action::TransferInto {
                    sources: request.paths,
                    dest,
                    mode: match kind {
                        droptarget::DropKind::Copy => ClipMode::Copy,
                        droptarget::DropKind::Move => ClipMode::Cut,
                    },
                });
            }
        }
        action
    }

    /// Fin d'image : les zones de dépôt dessinées sont confiées à la cible de dépôt de Windows.
    pub(super) fn publish_drop_zones(&self, ctx: &egui::Context) {
        let zones = self.drop_zones.take();
        let fallback = match &self.nav.location {
            Location::Dir(dir) => Some(dir.clone()),
            Location::ThisPc => None,
        };
        *self
            .shared_zones
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = DropZones {
            zones,
            fallback,
            pixels_per_point: ctx.pixels_per_point(),
        };
    }

    /// Fenêtre de Windows d'ExPLIA : y enregistre la cible de dépôt (glisser depuis
    /// l'Explorateur, le Bureau…).
    pub fn attach_window(&mut self, hwnd: Option<isize>) {
        let Some(hwnd) = hwnd else {
            return;
        };
        self.hwnd = Some(hwnd);
        let zones = Arc::clone(&self.shared_zones);
        let ctx = self.ctx.clone();
        let handlers = droptarget::Handlers {
            decide: Box::new(move |request| {
                let zones = zones
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                resolve_drop(&zones, request).map(|(_, kind)| kind)
            }),
            events: Arc::clone(&self.external_drop),
            wake: Box::new(move || ctx.request_repaint()),
        };
        if let Err(message) = droptarget::register(hwnd, handlers) {
            self.error = Some(message);
        }
    }

    /// Éléments glissés depuis l'Explorateur (ou le Bureau) et déposés dans la fenêtre : ils
    /// seront copiés dans le dossier affiché. Un dépôt juste après un glisser lancé depuis
    /// ExPLIA est ignoré (bouton relâché dans sa propre fenêtre).
    fn dropped_files(&self, ctx: &egui::Context) -> Option<Action> {
        let (dropped, now) = ctx.input(|i| {
            (
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect::<Vec<PathBuf>>(),
                i.time,
            )
        });
        let own_drag = self
            .drag_out_ended_at
            .is_some_and(|ended| now - ended < OWN_DROP_GUARD_SECONDS);
        match &self.nav.location {
            Location::Dir(current) if !dropped.is_empty() && !own_drag => Some(Action::CopyInto {
                sources: dropped,
                dest: current.clone(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Entry {
        Entry::file_at(Path::new(name))
    }

    fn folder(name: &str) -> Entry {
        Entry::folder_at(Path::new(name))
    }

    fn click() -> Pointer {
        Pointer {
            clicked: true,
            ..Pointer::default()
        }
    }

    fn run(pointer: Pointer, entry: &Entry, view: ItemView, state: GestureState) -> Option<Action> {
        translate(pointer, entry, view, state, || vec![entry.path.clone()])
    }

    #[test]
    fn a_click_selects_and_a_double_click_opens() {
        let a = file("a.txt");
        let state = GestureState::default();
        assert!(matches!(
            run(click(), &a, ItemView::List, state),
            Some(Action::Select(_))
        ));
        let double = Pointer {
            double_clicked: true,
            ..Pointer::default()
        };
        assert!(matches!(
            run(double, &folder("d"), ItemView::Grid, state),
            Some(Action::Go(_))
        ));
        assert!(matches!(
            run(double, &a, ItemView::List, state),
            Some(Action::Open(_))
        ));
    }

    #[test]
    fn ctrl_and_shift_clicks_extend_the_selection_in_every_view() {
        let a = file("a.txt");
        let ctrl = Pointer {
            ctrl: true,
            ..click()
        };
        let shift = Pointer {
            shift: true,
            ..click()
        };
        let last_column = ItemView::Column {
            dir: Path::new("x"),
            highlighted: None,
            is_last: true,
        };
        for view in [ItemView::List, ItemView::Grid, last_column] {
            let state = GestureState::default();
            assert!(matches!(
                run(ctrl, &a, view, state),
                Some(Action::ToggleMulti(_))
            ));
            assert!(matches!(
                run(shift, &a, view, state),
                Some(Action::SelectRange(_))
            ));
        }
    }

    #[test]
    fn after_a_long_press_clicks_toggle_and_the_release_is_ignored() {
        let a = file("a.txt");
        let toggle = GestureState {
            toggle_mode: true,
            long_press_done: false,
        };
        assert!(matches!(
            run(click(), &a, ItemView::List, toggle),
            Some(Action::ToggleMulti(_))
        ));
        let released = GestureState {
            toggle_mode: true,
            long_press_done: true,
        };
        assert!(run(click(), &a, ItemView::List, released).is_none());
        let held = Pointer {
            long_press: true,
            ..Pointer::default()
        };
        assert!(matches!(
            run(held, &a, ItemView::List, GestureState::default()),
            Some(Action::LongPress(_))
        ));
        // Le clic maintenu n'a pas d'effet dans les grilles.
        assert!(run(held, &a, ItemView::Grid, GestureState::default()).is_none());
    }

    #[test]
    fn dragging_carries_the_targets() {
        let a = file("a.txt");
        let drag = Pointer {
            drag_out: true,
            ..Pointer::default()
        };
        match run(drag, &a, ItemView::Grid, GestureState::default()) {
            Some(Action::DragOut(paths)) => assert_eq!(paths, [PathBuf::from("a.txt")]),
            _ => panic!("glisser attendu"),
        }
    }

    #[test]
    fn a_click_in_a_previous_column_opens_or_selects_there() {
        let parent = Path::new(r"C:\a");
        let view = ItemView::Column {
            dir: parent,
            highlighted: Some(Path::new(r"C:\a\b")),
            is_last: false,
        };
        let state = GestureState::default();
        assert!(matches!(
            run(click(), &file(r"C:\a\f.txt"), view, state),
            Some(Action::GoSelect { .. })
        ));
        let ctrl = Pointer {
            ctrl: true,
            ..click()
        };
        match run(ctrl, &file(r"C:\a\f.txt"), view, state) {
            Some(Action::ColumnMultiSelect { anchor, range, .. }) => {
                assert_eq!(anchor, Some(PathBuf::from(r"C:\a\b")));
                assert!(!range);
            }
            _ => panic!("multi-sélection de colonne attendue"),
        }
    }

    #[test]
    fn dropping_moves_on_the_same_drive_and_copies_elsewhere() {
        let sources = [PathBuf::from(r"C:\Photos\a.jpg")];
        let same = Path::new(r"c:\Archives");
        let other = Path::new(r"D:\Sauvegarde");
        assert_eq!(drop_mode(&sources, same, false, false), ClipMode::Cut);
        assert_eq!(drop_mode(&sources, other, false, false), ClipMode::Copy);
        // Ctrl force la copie, Maj le déplacement.
        assert_eq!(drop_mode(&sources, same, true, false), ClipMode::Copy);
        assert_eq!(drop_mode(&sources, other, false, true), ClipMode::Cut);
    }

    #[test]
    fn a_folder_is_never_a_target_for_itself_or_where_items_already_are() {
        let sources = [PathBuf::from(r"C:\Projets\site")];
        assert!(!accepts_drop(&sources, Path::new(r"C:\Projets\site")));
        assert!(!accepts_drop(
            &sources,
            Path::new(r"C:\Projets\site\images")
        ));
        assert!(!accepts_drop(&sources, Path::new(r"C:\Projets")));
        assert!(accepts_drop(&sources, Path::new(r"C:\Archives")));
        assert!(!accepts_drop(&[], Path::new(r"C:\Archives")));
    }

    fn request(path: &str, x: f32, y: f32) -> droptarget::DropRequest {
        droptarget::DropRequest {
            paths: vec![PathBuf::from(path)],
            x,
            y,
            ctrl: false,
            shift: false,
            can_copy: true,
            can_move: true,
        }
    }

    #[test]
    fn files_from_explorer_land_on_the_folder_under_the_pointer() {
        let zones = DropZones {
            zones: vec![(
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 20.0)),
                PathBuf::from(r"C:\Docs\Archives"),
            )],
            fallback: Some(PathBuf::from(r"C:\Docs")),
            pixels_per_point: 2.0,
        };
        // (100, 20) pixels = (50, 10) points : sur la ligne « Archives », même lecteur.
        let (dest, kind) = resolve_drop(&zones, &request(r"C:\Bureau\a.txt", 100.0, 20.0)).unwrap();
        assert_eq!(dest, PathBuf::from(r"C:\Docs\Archives"));
        assert_eq!(kind, droptarget::DropKind::Move);
        // Ailleurs dans la fenêtre : le dossier affiché ; autre lecteur : copie.
        let (dest, kind) = resolve_drop(&zones, &request(r"D:\a.txt", 100.0, 300.0)).unwrap();
        assert_eq!(dest, PathBuf::from(r"C:\Docs"));
        assert_eq!(kind, droptarget::DropKind::Copy);
        // La source n'autorise que la copie : jamais de déplacement.
        let copy_only = droptarget::DropRequest {
            can_move: false,
            ..request(r"C:\Bureau\a.txt", 100.0, 20.0)
        };
        assert_eq!(
            resolve_drop(&zones, &copy_only).unwrap().1,
            droptarget::DropKind::Copy
        );
        // Des fichiers déjà dans le dossier affiché, lâchés à côté : rien à faire.
        assert!(resolve_drop(&zones, &request(r"C:\Docs\a.txt", 100.0, 300.0)).is_none());
    }

    #[test]
    fn every_shortcut_has_a_label() {
        for shortcut in [
            Shortcut::Find,
            Shortcut::NewTab,
            Shortcut::CloseTab,
            Shortcut::Copy,
            Shortcut::Cut,
            Shortcut::Paste,
            Shortcut::Rename,
            Shortcut::Delete,
            Shortcut::NewFolder,
            Shortcut::Properties,
            Shortcut::Confirm,
            Shortcut::Cancel,
            Shortcut::PlayPause,
            Shortcut::CopyToOther,
            Shortcut::MoveToOther,
            Shortcut::SwitchPane,
        ] {
            assert!(!shortcut.label().is_empty());
        }
    }
}
