//! Volet de détail : aperçu, lecteur, informations, actions sur la sélection, barre d'état.

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    /// Deux boutons côte à côte, de même largeur :
    /// - emplacement 1 : « Renommer » (ou « Désélectionner »), qui devient « Annuler »
    ///   pendant la confirmation ;
    /// - emplacement 2 : « Supprimer », qui devient « Confirmer » **à la même place** :
    ///   on clique deux fois au même endroit pour supprimer (vers la corbeille).
    ///
    /// `rename` : l'élément à renommer (un seul à la fois) ; `deselect` : propose
    /// « Désélectionner » à la place de « Renommer » (multi-sélection).
    pub(in crate::app) fn edit_controls(
        &self,
        ui: &mut egui::Ui,
        what: &str,
        rename: Option<&Entry>,
        deselect: bool,
    ) -> Option<Action> {
        let mut action = None;
        let size = egui::vec2(EDIT_BUTTON_WIDTH, ui.spacing().interact_size.y);
        let now = ui.input(|i| i.time);
        let armed = self
            .delete_asked_at
            .is_none_or(|asked| now - asked > CONFIRM_ARMING_SECONDS);
        if self.confirm_delete && !armed {
            // Réveille l'interface pour que le bouton soit prêt à la fin du délai.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.horizontal(|ui| {
            // Emplacement 1.
            if self.confirm_delete {
                if ui.add_sized(size, egui::Button::new("Annuler")).clicked() {
                    action = Some(Action::CancelDelete);
                }
            } else if let Some(entry) = rename {
                if ui
                    .add_sized(size, egui::Button::new("✏ Renommer"))
                    .on_hover_text("Raccourci : F2")
                    .clicked()
                {
                    action = Some(Action::StartRename(entry.clone()));
                }
            } else if deselect
                && ui
                    .add_sized(size, egui::Button::new("Désélectionner"))
                    .clicked()
            {
                action = Some(Action::ClearMulti);
            }
            // Emplacement 2.
            if self.confirm_delete {
                let confirm =
                    egui::Button::new(egui::RichText::new("Confirmer").color(egui::Color32::WHITE))
                        .fill(egui::Color32::from_rgb(180, 40, 40));
                if ui.add_sized(size, confirm).clicked() && armed {
                    action = Some(Action::ConfirmDelete);
                }
            } else if ui
                .add_sized(size, egui::Button::new("🗑 Supprimer"))
                .on_hover_text("Raccourci : Suppr")
                .clicked()
            {
                action = Some(Action::AskDelete);
            }
        });
        // Ligne d'information sous les boutons : n'en change pas la position.
        let recycle_bin = self
            .delete_targets()
            .first()
            .is_none_or(|e| self.storage.capabilities(&e.id).recycle_bin);
        if self.confirm_delete {
            let question = if recycle_bin {
                format!("Envoyer {what} à la corbeille ?")
            } else {
                format!("Pas de corbeille ici : supprimer {what} définitivement ?")
            };
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{question}  (Entrée : confirmer, Échap : annuler)"),
            );
        } else if recycle_bin {
            ui.weak("Suppression : vers la corbeille de Windows.");
        } else {
            ui.weak("Suppression définitive : pas de corbeille sur ce lecteur.");
        }
        action
    }

    /// Emplacement d'un résultat, dans le volet de détail : le dossier qui le contient, puis
    /// chaque dossier au-dessus jusqu'au dossier affiché (exclu). Chaque ligne permet d'**ouvrir**
    /// ce dossier (on y retrouve le fichier, on peut remonter) ou de le **supprimer**
    /// (corbeille, avec confirmation) : pratique pour éliminer un dossier entier de doublons.
    pub(in crate::app) fn result_location(
        &self,
        ui: &mut egui::Ui,
        entry: &Entry,
    ) -> Option<Action> {
        let mut action = None;
        let Location::Dir(root) = &self.nav.location else {
            return None;
        };
        let parent = entry.path.parent()?;
        ui.add_space(2.0);
        ui.weak("Emplacement");
        let mut levels = fs_nav::levels_below(root, &entry.path);
        if levels.is_empty() {
            // Directement dans le dossier affiché : on peut seulement l'ouvrir.
            levels.push(parent.to_path_buf());
        }
        for (index, level) in levels.iter().enumerate() {
            let label = level
                .strip_prefix(root)
                .ok()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| level.display().to_string(), |p| p.display().to_string());
            ui.horizontal(|ui| {
                let open = ui
                    .small_button("↗ Ouvrir")
                    .on_hover_text(format!("Aller dans {}", level.display()));
                if open.clicked() {
                    action = Some(if index == 0 {
                        Action::GoSelect {
                            dir: level.clone(),
                            file: entry.path.clone(),
                        }
                    } else {
                        Action::Go(Location::Dir(level.clone()))
                    });
                }
                // Le dossier affiché lui-même se supprime depuis son propre volet.
                if level != root
                    && ui
                        .small_button("🗑")
                        .on_hover_text("Supprimer ce dossier (corbeille, avec confirmation)")
                        .clicked()
                {
                    action = Some(Action::AskDeleteFolder(level.clone()));
                }
                ui.add(egui::Label::new(egui::RichText::new(label).weak()).truncate());
            });
        }
        ui.add_space(2.0);
        action
    }

    /// Outils d'un dossier, dans le volet de détail : PowerShell (normal ou administrateur) et
    /// copie du chemin.
    pub(in crate::app) fn folder_tools(&self, ui: &mut egui::Ui, dir: &Path) -> Option<Action> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if powershell_button(ui, ">_ PowerShell")
                .on_hover_text("Ouvre PowerShell dans ce dossier")
                .clicked()
            {
                action = Some(Action::OpenPowerShell {
                    dir: dir.to_path_buf(),
                    admin: false,
                });
            }
            if powershell_button(ui, ">_ Admin")
                .on_hover_text("Ouvre PowerShell en tant qu'administrateur (fenêtre UAC)")
                .clicked()
            {
                action = Some(Action::OpenPowerShell {
                    dir: dir.to_path_buf(),
                    admin: true,
                });
            }
            if ui
                .button("📋 Chemin")
                .on_hover_text("Copie le chemin du dossier")
                .clicked()
            {
                action = Some(Action::CopyPath(dir.to_path_buf()));
            }
        });
        // Nouveau dossier, Coller ici, Propriétés : selon ce que la source permet.
        let caps = self.storage.capabilities(&ItemId::path(dir));
        let folder = Entry::folder_at(dir);
        let targets = std::slice::from_ref(&folder);
        ui.horizontal_wrapped(|ui| {
            for (allowed, command) in [
                (caps.create_folder, Command::CreateFolder),
                (caps.copy_into, Command::Paste),
                (caps.properties, Command::ShowProperties),
            ] {
                if allowed
                    && let Some(chosen) =
                        context_menu_manager::command_button(ui, command, &folder, targets)
                {
                    action = Some(chosen);
                }
            }
        });
        ui.add_space(4.0);
        action
    }

    /// Contenu du volet pendant la multi-sélection : résumé et actions (supprimer).
    pub(in crate::app) fn actions_content(&self, ui: &mut egui::Ui) -> Option<Action> {
        let count = self.selection.multi.len();
        let kind = if self.selection.multi.first().is_some_and(|e| e.is_dir) {
            "dossier(s)"
        } else {
            "fichier(s)"
        };
        ui.label(egui::RichText::new(format!("{count} {kind} sélectionné(s)")).strong());
        ui.weak("Clic maintenu ou clic sur une ligne pour ajouter ou retirer.");
        ui.add_space(6.0);
        let mut copy_action = None;
        if let Some(first) = self.selection.multi.first() {
            let caps = self.storage.capabilities(&first.id);
            ui.horizontal_wrapped(|ui| {
                for (allowed, command) in [(true, Command::Copy), (caps.move_items, Command::Cut)] {
                    if allowed
                        && let Some(chosen) = context_menu_manager::command_button(
                            ui,
                            command,
                            first,
                            &self.selection.multi,
                        )
                    {
                        copy_action = Some(chosen);
                    }
                }
            });
        }
        let action = self
            .edit_controls(ui, &format!("{count} {kind}"), None, true)
            .or(copy_action);
        ui.add_space(6.0);
        ui.separator();
        let row_height = table_row_height(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, count, |ui, range| {
                for item in &self.selection.multi[range] {
                    let icon = if item.is_dir { "📁" } else { "📄" };
                    // Taille à jour (celle d'un dossier peut arriver après la sélection).
                    let fresh = self
                        .entries
                        .iter()
                        .find(|e| e.path == item.path)
                        .unwrap_or(item);
                    let size = self.entry_size_text(fresh);
                    size_row(ui, false, &format!("{icon} {}", item.name), &size);
                }
            });
        action
    }

    /// Volet d'aperçu. Renvoie `true` si l'utilisateur a cliqué sur la flèche
    /// « < » / « > » pour agrandir ou réduire le volet, et l'action éventuelle
    /// demandée (ouvrir le fichier).
    pub(in crate::app) fn preview_panel(&self, ui: &mut egui::Ui) -> Option<Action> {
        let option_action = self.option_bar(ui);
        let content_action = self.preview_content(ui);
        option_action.or(content_action)
    }

    /// Largeur minimale du volet de détail : la flèche et les boutons Colonnes, Images et Vidéos
    /// tiennent sur une seule ligne.
    pub(in crate::app) fn detail_min_width(ui: &egui::Ui) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let padding = ui.spacing().button_padding.x * 2.0;
        let gap = ui.spacing().item_spacing.x;
        let labels = [
            OPTION_SPLIT_LABEL,
            OPTION_COLUMNS_LABEL,
            OPTION_IMAGES_LABEL,
            OPTION_VIDEOS_LABEL,
        ];
        let buttons: f32 = labels
            .iter()
            .map(|label| {
                let galley = ui.painter().layout_no_wrap(
                    (*label).to_string(),
                    font.clone(),
                    egui::Color32::WHITE,
                );
                galley.size().x + padding
            })
            .sum();
        // Marges du panneau, poignée de redimensionnement, barre de défilement éventuelle.
        buttons + gap * (labels.len() - 1) as f32 + 36.0
    }

    /// Barre du haut du volet de détail : flèche d'agrandissement, navigation en colonnes
    /// (style Mac), visionnage d'images et de vidéos (grilles). Les éléments cachés se règlent dans « Afficher ».
    pub(in crate::app) fn option_bar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Affichage fractionné : deux volets côte à côte, pour transférer de l'un à l'autre.
            if ui
                .add(egui::Button::new(OPTION_SPLIT_LABEL).selected(self.split.is_some()))
                .on_hover_text(
                    "Deux volets côte à côte : glissez d'un volet à l'autre pour copier (Maj : \
                     déplacer), F5 copie et F6 déplace vers l'autre volet, Tab change de volet",
                )
                .clicked()
            {
                action = Some(Action::ToggleSplit);
            }
            for (mode, label, hint) in [
                (
                    ViewMode::Columns,
                    OPTION_COLUMNS_LABEL,
                    "Navigation en colonnes, comme sur Mac : un niveau du chemin par colonne",
                ),
                (
                    ViewMode::Gallery,
                    OPTION_IMAGES_LABEL,
                    "Visionnage d'images : les images du dossier en grille de miniatures",
                ),
                (
                    ViewMode::Videos,
                    OPTION_VIDEOS_LABEL,
                    "Visionnage de vidéos : les vidéos du dossier en grille",
                ),
            ] {
                let active = self.split.is_none() && self.view_mode == mode;
                if ui
                    .add(egui::Button::new(label).selected(active))
                    .on_hover_text(hint)
                    .clicked()
                {
                    // Un second clic revient à la liste.
                    action = Some(Action::SetViewMode(if active {
                        ViewMode::List
                    } else {
                        mode
                    }));
                }
            }
        });
        ui.separator();
        action
    }

    /// Zone du lecteur intégré. Renvoie l'action éventuelle (arrêt) et `true` si le lecteur
    /// (ou son chargement) est affiché pour ce fichier.
    pub(in crate::app) fn player_area(
        &self,
        ui: &mut egui::Ui,
        entry: &Entry,
    ) -> (Option<Action>, bool) {
        match &self.player.slot {
            PlayerSlot::Loading(path) if *path == entry.path => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement de VLC…");
                });
                (None, true)
            }
            PlayerSlot::Ready(active) if active.path == entry.path => {
                (Self::player_controls(ui, active), true)
            }
            _ => (None, false),
        }
    }

    /// Image de la vidéo et commandes : lecture / pause, arrêt, position, volume.
    pub(in crate::app) fn player_controls(
        ui: &mut egui::Ui,
        active: &ActivePlayer,
    ) -> Option<Action> {
        let mut action = None;
        match &active.texture {
            Some(texture) => {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(4.0);
                let image =
                    egui::Image::new(egui::load::SizedTexture::new(texture.id(), size * scale))
                        .sense(egui::Sense::click());
                // Double-clic sur la vidéo : plein écran.
                if ui
                    .add(image)
                    .on_hover_text("Double-clic : plein écran")
                    .double_clicked()
                {
                    action = Some(Action::ToggleVideoFullscreen);
                }
            }
            None => {
                // Son (pas d'image) ou image pas encore décodée.
                ui.label(egui::RichText::new("🎵").size(48.0));
            }
        }
        Self::player_bar(ui, active).or(action)
    }

    /// Commandes du lecteur : lecture / pause, arrêt, position, volume.
    pub(in crate::app) fn player_bar(ui: &mut egui::Ui, active: &ActivePlayer) -> Option<Action> {
        let mut action = None;
        let player = &active.player;
        let state = player.state();
        let length = player.length();
        ui.horizontal(|ui| {
            let label = match state {
                PlayerState::Playing => "⏸",
                PlayerState::Ended => "⟲",
                _ => "▶",
            };
            if ui
                .button(label)
                .on_hover_text("Lecture / pause (Espace)")
                .clicked()
            {
                player.toggle_pause();
            }
            if ui.button("⏹").on_hover_text("Arrêter").clicked() {
                action = Some(Action::StopPlayer);
            }
            let total = length.map_or("--:--".to_string(), dates::format_duration);
            ui.label(format!(
                "{} / {total}",
                dates::format_duration(player.time())
            ));
            if state == PlayerState::Error {
                ui.colored_label(ui.visuals().error_fg_color, "Erreur de lecture");
            }
        });
        // Les curseurs occupent toute la largeur du volet.
        ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(60.0);
        if let Some(length) = length {
            let total = length.as_secs_f32().max(0.1);
            let mut position = player.time().as_secs_f32().min(total);
            let slider = egui::Slider::new(&mut position, 0.0..=total).show_value(false);
            if ui.add(slider).changed() {
                player.set_time(std::time::Duration::from_secs_f32(position));
            }
        }
        ui.horizontal(|ui| {
            ui.label("🔊");
            ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(40.0);
            let mut volume = player.volume();
            let slider = egui::Slider::new(&mut volume, 0..=100).show_value(false);
            if ui.add(slider).changed() {
                player.set_volume(volume);
            }
        });
        action
    }

    /// Contenu du volet : en-tête, aperçu ou actions.
    pub(in crate::app) fn preview_content(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        if !self.selection.multi.is_empty() {
            return self.actions_content(ui);
        }
        let Some(selected) = &self.selection.single else {
            // Aucun élément choisi : le volet propose d'agir sur le dossier affiché lui-même.
            if let Location::Dir(path) = &self.nav.location
                && self.scan.results.is_none()
                && path.parent().is_some()
            {
                ui.label(egui::RichText::new(favorite_name(path)).strong());
                ui.label("Dossier courant");
                ui.add_space(6.0);
                let tools = self.folder_tools(ui, path);
                if ui
                    .button("🗑 Supprimer ce dossier")
                    .on_hover_text("Envoie le dossier affiché à la corbeille (après confirmation)")
                    .clicked()
                {
                    return Some(Action::AskDeleteFolder(path.clone()));
                }
                ui.weak("Suppression : vers la corbeille de Windows.");
                return tools;
            } else {
                ui.weak("Sélectionnez un élément.");
            }
            return None;
        };
        // La taille d'un dossier peut arriver après la sélection : on relit l'entrée à jour.
        let entry = self
            .entries
            .iter()
            .find(|e| e.path == selected.path)
            .unwrap_or(selected);
        ui.label(egui::RichText::new(&entry.name).strong());
        // Parmi des résultats (doublons, recherche) : où se trouve l'élément, avec de quoi aller
        // dans son dossier, remonter et supprimer un dossier parent.
        if self.scan.results.is_some() {
            action = self.result_location(ui, entry);
        }
        if entry.is_dir {
            ui.label("Dossier");
            let size = self.entry_size_text(entry);
            if !size.is_empty() {
                ui.label(format!("Taille : {size}"));
            }
            ui.add_space(6.0);
            let tools = self.folder_tools(ui, &entry.path);
            let what = format!("le dossier « {} »", entry.name);
            return self
                .edit_controls(ui, &what, Some(entry), false)
                .or(action)
                .or(tools);
        }
        ui.label(preview::kind_description(&entry.path));
        ui.label(format!("Taille : {}", self.entry_size_text(entry)));
        ui.add_space(4.0);
        let is_video = preview::is_video(&entry.path);
        let playable = is_playable(&entry.path);
        let is_executable = launch::is_executable(&entry.path);
        ui.horizontal_wrapped(|ui| {
            if playable {
                // Lecture dans le volet (VLC) ; sans VLC, ouverture avec l'application par défaut.
                if ui
                    .button("▶ Lire")
                    .on_hover_text("Lit le fichier ici (nécessite VLC 64 bits installé)")
                    .clicked()
                {
                    action = Some(Action::Play(entry.path.clone()));
                }
                if ui
                    .button("↗ Lecteur externe")
                    .on_hover_text("Ouvre le fichier avec l'application par défaut de Windows")
                    .clicked()
                {
                    action = Some(Action::Open(entry.path.clone()));
                }
            } else {
                let label = if is_executable {
                    "▶ Exécuter"
                } else {
                    "Ouvrir"
                };
                if ui
                    .button(label)
                    .on_hover_text("Ouvre le fichier avec l'application par défaut de Windows")
                    .clicked()
                {
                    action = Some(Action::Open(entry.path.clone()));
                }
            }
            if is_executable
                && ui
                    .button("🛡 Exécuter en administrateur")
                    .on_hover_text("Windows demandera une confirmation (UAC)")
                    .clicked()
            {
                action = Some(Action::OpenAsAdmin(entry.path.clone()));
            }
        });
        let what = format!("le fichier « {} »", entry.name);
        if let Some(edit_action) = self.edit_controls(ui, &what, Some(entry), false) {
            action = Some(edit_action);
        }
        ui.add_space(4.0);
        // Lecteur audio / vidéo intégré : affiché dès qu'une lecture est lancée sur ce fichier.
        if playable {
            let (player_action, showing) = self.player_area(ui, entry);
            if player_action.is_some() {
                action = player_action;
            }
            if showing {
                return action;
            }
        }
        if !preview::is_previewable(&entry.path) {
            ui.add_space(12.0);
            let icon = if is_video {
                "🎬"
            } else if playable {
                "🎵"
            } else {
                "📄"
            };
            ui.label(egui::RichText::new(icon).size(64.0));
            if playable && !player::is_available() {
                ui.weak(
                    "Lecture intégrée : installez VLC (64 bits, videolan.org). \
                     Sans VLC, « Lire » ouvre le lecteur par défaut.",
                );
            } else if playable {
                ui.weak("Cliquez sur « Lire » pour l'écouter ou la regarder ici.");
            } else {
                ui.weak("Pas d'aperçu pour ce type de fichier.");
            }
            return action;
        }
        ui.add_space(8.0);
        match &self.preview.state {
            PreviewState::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement de l'aperçu…");
                });
            }
            PreviewState::Image {
                path,
                texture,
                original,
            } if *path == entry.path => {
                ui.label(format!("Dimensions : {} × {} px", original.0, original.1));
                ui.add_space(4.0);
                egui::ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let tex_size = texture.size_vec2();
                        let scale = (ui.available_width() / tex_size.x).min(1.0);
                        ui.image(egui::load::SizedTexture::new(
                            texture.id(),
                            tex_size * scale,
                        ));
                    });
            }
            PreviewState::Text { path, preview } if *path == entry.path => {
                if preview.truncated {
                    ui.weak(format!(
                        "Début du fichier ({} Ko affichés)",
                        preview::MAX_TEXT_BYTES / 1024
                    ));
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&preview.content).monospace())
                                .wrap(),
                        );
                    });
            }
            PreviewState::Archive { path, preview } if *path == entry.path => {
                let shown = preview.entries.len();
                if preview.total > shown {
                    ui.weak(format!("{shown} premiers éléments sur {}", preview.total));
                } else {
                    ui.weak(format!("{} élément(s)", preview.total));
                }
                ui.add_space(4.0);
                let row_height = table_row_height(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show_rows(ui, row_height, shown, |ui, range| {
                        for item in &preview.entries[range] {
                            let icon = if item.is_dir { "📁" } else { "📄" };
                            let size = if item.is_dir {
                                String::new()
                            } else {
                                fs_nav::format_size(item.size)
                            };
                            size_row(ui, false, &format!("{icon} {}", item.name), &size);
                        }
                    });
            }
            PreviewState::Failed { path, message } if *path == entry.path => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Aperçu impossible : {message}"),
                );
            }
            _ => {}
        }
        action
    }

    pub(in crate::app) fn status_text(&self) -> String {
        if let Some(results) = &self.scan.results {
            let what = match results.kind {
                ResultKind::Search => "résultat(s) de recherche",
                ResultKind::Duplicates => "fichier(s) en doublon",
                ResultKind::LargeFiles => "gros fichier(s)",
            };
            return format!("{} {what}", self.visible.len());
        }
        match &self.nav.location {
            Location::ThisPc => "Aucun dossier ouvert".to_string(),
            Location::Dir(_) => {
                let count = if self.visible.len() == self.entries.len() {
                    format!("{} élément(s)", self.entries.len())
                } else {
                    format!(
                        "{} élément(s) affiché(s) sur {}",
                        self.visible.len(),
                        self.entries.len()
                    )
                };
                format!("{count}   |   Sélection multiple : clic maintenu, Ctrl+clic ou Maj+clic")
            }
        }
    }

    /// Plein écran : la vidéo occupe toute la fenêtre, les commandes en bas. Double-clic ou
    /// Échap en sort. Renvoie `false` s'il n'y a plus de vidéo à montrer (plein écran quitté).
    pub(in crate::app) fn fullscreen_video(&mut self, ui: &mut egui::Ui) -> bool {
        let ctx = ui.ctx().clone();
        let PlayerSlot::Ready(active) = &self.player.slot else {
            self.set_video_fullscreen(&ctx, false);
            return false;
        };
        let Some(texture) = &active.texture else {
            self.set_video_fullscreen(&ctx, false);
            return false;
        };
        let mut action = None;
        let mut exit = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Panel::bottom("fullscreen-controls").show(ui, |ui| {
            ui.add_space(4.0);
            action = Self::player_bar(ui, active);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let size = texture.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y);
                let rect = egui::Rect::from_center_size(area.center(), size * scale);
                let response = ui.allocate_rect(area, egui::Sense::click());
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                if response
                    .on_hover_text("Double-clic ou Échap : quitter le plein écran")
                    .double_clicked()
                {
                    exit = true;
                }
            });
        if exit {
            self.set_video_fullscreen(&ctx, false);
        }
        if let Some(action) = action {
            if matches!(action, Action::StopPlayer) {
                self.set_video_fullscreen(&ctx, false);
            }
            self.apply(&ctx, action);
        }
        true
    }

    /// Pendant le survol de fichiers venus d'une autre application : voile et consigne.
    pub(in crate::app) fn drop_hint(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering
            && !self.drag_out_done
            && let Location::Dir(current) = &self.nav.location
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drop-hint"),
            ));
            let screen = ctx.content_rect();
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                format!("Déposer pour copier dans « {} »", favorite_name(current)),
                egui::FontId::proportional(22.0),
                egui::Color32::WHITE,
            );
        }
    }
}
