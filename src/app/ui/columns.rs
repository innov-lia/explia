//! Vue en colonnes « style Mac ».

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    /// Navigation « style Mac » : une colonne par niveau du chemin (lecteurs, puis chaque
    /// dossier traversé, puis le dossier courant). Un clic sur un dossier l'ouvre dans la
    /// colonne suivante ; un clic sur un fichier le sélectionne.
    pub(in crate::app) fn columns_view(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let Location::Dir(current) = self.nav.location.clone() else {
            return None;
        };
        // Dans Home, les colonnes commencent sous le dossier de l'utilisateur (la barre latérale
        // montre déjà son contenu) : pas de colonnes C:, Users, nom.
        let crumbs = fs_nav::column_levels(&current, self.sidebar.home.as_deref());
        // Listings des dossiers traversés (le dernier est le dossier courant : `self.visible`),
        // lus en arrière-plan : une colonne reste vide le temps de sa lecture.
        for (_, path) in crumbs.iter().take(crumbs.len().saturating_sub(1)) {
            self.request_column(path);
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
        let scroll_target = self.nav.scroll_to_index.take();
        if scroll_target.is_some() {
            ui.ctx().request_repaint();
        }
        let last_scroll = self.nav.last_scroll;
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
                            self.nav.column_cache.get(dir).map_or(&[], Vec::as_slice)
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
        self.nav.last_scroll = new_scroll;
        self.renaming = rename;
        action
    }
}
