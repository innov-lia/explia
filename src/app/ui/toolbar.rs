//! Barres du haut : barre de menus, fil d'Ariane et saisie du chemin, recherche, filtre « Sélection
//! », bandeau des résultats.

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    /// Bandeau des résultats d'un parcours : titre, avancement, actions et fermeture.
    pub(in crate::app) fn results_banner(&self, ui: &mut egui::Ui) -> Option<Action> {
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
    pub(in crate::app) fn location_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
    pub(in crate::app) fn selection_menu(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
    pub(in crate::app) fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
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
    pub(in crate::app) fn address_edit(&mut self, ui: &mut egui::Ui) {
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
    pub(in crate::app) fn breadcrumb_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
    pub(in crate::app) fn options_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
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
}
