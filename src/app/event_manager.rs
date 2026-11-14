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
        }
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
        translate(pointer, entry, view, state, || {
            self.selection.paths_from(entry)
        })
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
        if self.drag_out_done || !response.is_pointer_button_down_on() {
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
            } else if pressed(Shortcut::Delete) && !targets.is_empty() {
                action = Some(Action::AskDelete);
            } else if !self.confirm_delete {
                action = self.arrow_key_action(ctx);
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
        ] {
            assert!(!shortcut.label().is_empty());
        }
    }
}
