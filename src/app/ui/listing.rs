//! Zone centrale en liste détaillée (et résultats des parcours).

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    pub(in crate::app) fn central(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        // Les résultats d'un parcours (recherche récursive, dossiers vides, doublons) s'affichent
        // toujours en liste, quel que soit le mode d'affichage.
        if self.scan.results.is_some() && matches!(self.nav.location, Location::Dir(_)) {
            return self.listing_view(ui);
        }
        match (&self.nav.location, self.view_mode) {
            (Location::ThisPc, _) => {
                ui.label("Choisissez un dossier ou un lecteur dans la barre latérale.");
                None
            }
            (Location::Dir(_), ViewMode::List) => self.listing_view(ui),
            (Location::Dir(_), ViewMode::Columns) => self.columns_view(ui),
            (Location::Dir(_), ViewMode::Gallery | ViewMode::Videos) => self.gallery_layout(ui),
        }
    }

    /// Affichage fractionné : les deux volets côte à côte, chacun avec sa barre d'adresse et
    /// sa liste. Le volet qui a la main est encadré ; un clic dans l'autre lui donne la main.
    pub(in crate::app) fn split_area(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let active = self.split.as_ref()?.active;
        let mut action = None;
        let gap = ui.spacing().item_spacing.x * 2.0 + 1.0;
        let half = ((ui.available_width() - gap) / 2.0).max(120.0);
        let height = ui.available_height();
        let mut rects = [egui::Rect::NOTHING; 2];
        ui.horizontal_top(|ui| {
            for side in [Side::Left, Side::Right] {
                let is_active = side == active;
                let stroke = if is_active {
                    ui.visuals().selection.stroke
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                };
                let response = ui
                    .vertical(|ui| {
                        ui.set_width(half);
                        ui.set_height(height);
                        egui::Frame::group(ui.style())
                            .stroke(stroke)
                            .inner_margin(4.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_height(ui.available_height());
                                let pane_action = if is_active {
                                    self.pane_contents(ui)
                                } else {
                                    self.with_other(|app| app.pane_contents(ui))
                                };
                                if pane_action.is_some() {
                                    action = pane_action;
                                }
                            });
                    })
                    .response;
                rects[split::side_index(side)] = response.rect;
                if side == Side::Left {
                    ui.separator();
                }
            }
        });
        // Un clic (ou un clic droit) dans l'autre volet lui donne la main ; son action
        // éventuelle s'applique ensuite à lui.
        let other_rect = rects[split::side_index(split::opposite(active))];
        let pressed_in_other = ui.input(|i| {
            (i.pointer.primary_pressed() || i.pointer.secondary_pressed())
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| other_rect.contains(p))
        });
        if let Some(split) = self.split.as_mut() {
            split.rects = rects;
        }
        // L'autre volet, hors de ses dossiers, reçoit dans son dossier ce qu'on y dépose.
        if let Some(dir) = self.other_dir() {
            self.drop_zones.borrow_mut().push((other_rect, dir.clone()));
            let pointer = self.drag_pointer(ui.ctx());
            if self.drop_target.borrow().is_none()
                && pointer.is_some_and(|p| other_rect.contains(p))
            {
                self.offer_drop_target(&dir);
            }
        }
        if pressed_in_other {
            self.activate_other();
        }
        action
    }

    /// Contenu d'un volet : barre d'adresse, bandeau des résultats, liste.
    fn pane_contents(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = self.location_bar(ui);
        action = self.results_banner(ui).or(action);
        ui.add_space(4.0);
        if matches!(self.nav.location, Location::Dir(_)) {
            self.listing_view(ui).or(action)
        } else {
            ui.label("Choisissez un dossier ou un lecteur dans la barre latérale.");
            action
        }
    }

    /// Pendant un glisser : une étiquette suit le pointeur et dit ce que fera le dépôt ;
    /// l'autre volet s'illumine quand il est la cible.
    pub(in crate::app) fn drag_hint(&self, ctx: &egui::Context) {
        let Some(paths) = self.dragged_paths() else {
            return;
        };
        let Some(pointer) = self.drag_pointer(ctx) else {
            return;
        };
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drag-hint"),
        ));
        let target = self.drop_target.borrow().clone();
        if let (Some(split), Some(target)) = (&self.split, &target)
            && self.other_dir().as_ref() == Some(target)
        {
            let rect = split.rects[split::side_index(split::opposite(split.active))];
            if rect.contains(pointer) {
                let color = ctx
                    .global_style()
                    .visuals
                    .selection
                    .bg_fill
                    .gamma_multiply(0.25);
                painter.rect_filled(rect, 4.0, color);
            }
        }
        // Touches : celles d'ExPLIA, ou celles que Windows transmet pour un glisser venu
        // d'ailleurs.
        let (ctrl, shift) = match (&self.drag, &self.external_hover) {
            (None, Some(hover)) => (hover.ctrl, hover.shift),
            _ => ctx.input(|i| (i.modifiers.command, i.modifiers.shift)),
        };
        let text = match &target {
            Some(dest) => {
                let verb = match event_manager::drop_mode(paths, dest, ctrl, shift) {
                    ClipMode::Copy => "Copier",
                    ClipMode::Cut => "Déplacer",
                };
                format!(
                    "📄 {} élément(s) — {verb} vers « {} »  (Ctrl : copier, Maj : déplacer)",
                    paths.len(),
                    favorite_name(dest)
                )
            }
            None => format!(
                "📄 {} élément(s) — déposez sur un dossier, ou hors de la fenêtre",
                paths.len()
            ),
        };
        let galley =
            painter.layout_no_wrap(text, egui::FontId::proportional(14.0), egui::Color32::WHITE);
        let rect = egui::Rect::from_min_size(
            pointer + egui::vec2(14.0, 10.0),
            galley.size() + egui::vec2(12.0, 8.0),
        );
        painter.rect_filled(rect, 4.0, egui::Color32::from_black_alpha(200));
        painter.galley(
            rect.min + egui::vec2(6.0, 4.0),
            galley,
            egui::Color32::WHITE,
        );
        ctx.request_repaint();
    }

    pub(in crate::app) fn listing_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        // Première ligne : « .. » revient au dossier précédent (le parent).
        let parent = if self.scan.results.is_some() {
            None
        } else {
            self.nav.location.parent()
        };
        let shift = usize::from(parent.is_some());
        let empty_message = if let Some(results) = &self.scan.results {
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
        if let Some(index) = self.nav.scroll_to_index.take() {
            // Une seconde image est nécessaire : les lignes affichées dépendent du décalage appliqué.
            ui.ctx().request_repaint();
            let (offset, view_height) = self.nav.last_scroll;
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
                if entry.is_dir {
                    self.drop_zone(ui, &response, &entry.path);
                }
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
        self.nav.last_scroll = (output.state.clamped_offset().y, output.inner_rect.height());
        self.renaming = rename;
        action
    }

    /// Libellé d'une ligne de la liste. Parmi des résultats : numéro de groupe (doublons) et
    /// dossier d'origine, relatif au dossier affiché.
    pub(in crate::app) fn row_label(&self, icon: &str, entry: &Entry) -> String {
        if self.scan.results.is_none() {
            return format!("{icon} {}", entry.name);
        }
        let group = self
            .scan
            .duplicate_groups
            .get(&entry.path)
            .map_or(String::new(), |group| format!("#{group}  "));
        let origin = match (&self.nav.location, entry.path.parent()) {
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
    pub(in crate::app) fn entry_size_text(&self, entry: &Entry) -> String {
        match entry.size {
            Some(bytes) => fs_nav::format_size(bytes),
            None if entry.is_dir && !self.skip_folder_sizes && self.scan.results.is_none() => {
                "Calcul…".to_string()
            }
            None => String::new(),
        }
    }

    /// Colonnes affichées dans le tableau, de gauche à droite (après « Nom »).
    pub(in crate::app) fn active_columns(&self) -> Vec<Column> {
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
    pub(in crate::app) fn entry_cell(&self, entry: &Entry, column: Column) -> String {
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
                match self.preview.media.get(&entry.path) {
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
}
