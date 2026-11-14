//! Grilles d'images et de vidéos, avec leurs colonnes de lecteurs et de navigation.

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    /// Mode « grille » : hors de Home, des zones côte à côte — les lecteurs (sauf si l'onglet « Ce PC »
    /// de la barre latérale les montre déjà), une colonne de navigation (`..` et sous-dossiers),
    /// puis la grille. Dans Home, la grille seule (on change de dossier par la barre latérale).
    /// La grille ne contient que des images, ou que des vidéos : jamais de dossier.
    pub(in crate::app) fn gallery_layout(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.grid_side_columns() {
            return self.gallery_view(ui);
        }
        let mut action = None;
        let height = ui.available_height();
        let gap = ui.spacing().item_spacing.x * 2.0 + 1.0;
        // Les lecteurs ne sont pas répétés quand l'onglet « Ce PC » de la barre latérale les montre.
        let show_drives = self.sidebar.tab != SidebarTab::Pc;
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
    pub(in crate::app) fn nav_column(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        // Le dossier de la barre d'adresse, en tête de la colonne.
        if let Location::Dir(path) = &self.nav.location {
            ui.add(egui::Label::new(egui::RichText::new(favorite_name(path)).weak()).truncate());
        }
        let parent = self.nav.location.parent();
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
                    self.drop_zone(ui, &response, &entry.path);
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
    pub(in crate::app) fn gallery_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
        if let Some(index) = self.nav.scroll_to_index.take() {
            ui.ctx().request_repaint();
            let (offset, view_height) = self.nav.last_scroll;
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
                        let thumb = is_image
                            .then(|| self.preview.thumbs.get(&entry.path))
                            .flatten();
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
        self.nav.last_scroll = (output.state.clamped_offset().y, output.inner_rect.height());
        self.request_thumbs(&wanted);
        action
    }
}
