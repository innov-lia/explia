//! Barre latérale (classeur : Ce PC, Home, Favoris, Applications, Réseau) et disposition des volets
//! latéraux.

use super::super::*;
use super::widgets::*;

impl ExpliaApp {
    /// Panneau de la barre latérale, toujours à gauche.
    pub(in crate::app) fn sidebar_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        egui::Panel::left("sidebar-left")
            .default_size(SIDEBAR_DEFAULT_WIDTH)
            .resizable(true)
            .show(ui, |ui| {
                action = self.sidebar(ui);
            });
        action
    }

    /// Volet latéral (à gauche ou à droite selon la disposition) : le volet de détail, ou la
    /// navigation quand le détail est au centre. Un petit triangle, en bas contre la zone
    /// centrale, échange les deux.
    pub(in crate::app) fn side_panel(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        if !self.detail_shown() {
            return None;
        }
        let mut action = None;
        let central = self.detail_central;
        let (min_width, width) = if central {
            (260.0, 420.0)
        } else {
            // Jamais plus étroit que la barre « Colonnes Images Vidéos » sur une ligne.
            let min = Self::detail_min_width(ui);
            (min, 300.0_f32.max(min))
        };
        let panel = match (self.preview_side, central) {
            (Side::Left, false) => egui::Panel::left("detail-left"),
            (Side::Right, false) => egui::Panel::right("detail-right"),
            (Side::Left, true) => egui::Panel::left("navigation-left"),
            (Side::Right, true) => egui::Panel::right("navigation-right"),
        };
        let response = panel
            .default_size(width)
            .min_size(min_width)
            .resizable(true)
            .show(ui, |ui| {
                action = if central {
                    self.navigation_area(ui)
                } else {
                    self.preview_panel(ui)
                };
            })
            .response;
        if self.swap_handle(ui.ctx(), response.rect) {
            self.detail_central = !self.detail_central;
        }
        action
    }

    /// Triangle d'échange, en bas du volet latéral, contre la zone centrale : il pointe vers le
    /// centre et y amène le contenu du volet. Renvoie `true` s'il est cliqué.
    pub(in crate::app) fn swap_handle(&self, ctx: &egui::Context, panel: egui::Rect) -> bool {
        let on_right = self.preview_side == Side::Right;
        let (arrow, x) = if on_right {
            ("◀", panel.left() + 4.0)
        } else {
            ("▶", panel.right() - 26.0)
        };
        let hint = if self.detail_central {
            "Remettre la navigation au centre"
        } else {
            "Mettre le volet de détail au centre"
        };
        let mut clicked = false;
        egui::Area::new(egui::Id::unique("swap-panels"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, panel.bottom() - 26.0))
            .show(ctx, |ui| {
                clicked = ui.small_button(arrow).on_hover_text(hint).clicked();
            });
        clicked
    }

    /// Navigation dans les dossiers : barre d'adresse et recherche, résultats, contenu.
    pub(in crate::app) fn navigation_area(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = self.location_bar(ui);
        action = self.results_banner(ui).or(action);
        ui.add_space(4.0);
        self.central(ui).or(action)
    }

    /// Barre latérale en « classeur » : la liste des dossiers de l'onglet actif à gauche,
    /// les onglets (intercalaires : Ce PC, Home, Favoris, Réseau) sur le bord droit.
    pub(in crate::app) fn sidebar(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let height = ui.available_height();
        let list_width =
            (ui.available_width() - SIDEBAR_TAB_WIDTH - ui.spacing().item_spacing.x).max(60.0);
        ui.horizontal_top(|ui| {
            if self.tabs_side == Side::Left
                && let Some(tab) = self.sidebar_tabs(ui)
            {
                action = Some(Action::SidebarTab(tab));
            }
            ui.vertical(|ui| {
                ui.set_width(list_width);
                ui.set_height(height);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-list")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        action = self.sidebar_list(ui).or(action.take());
                    });
            });
            if self.tabs_side == Side::Right
                && let Some(tab) = self.sidebar_tabs(ui)
            {
                action = Some(Action::SidebarTab(tab));
            }
        });
        action
    }

    /// Intercalaires du classeur : texte vertical, sur le bord gauche ou droit de la liste.
    pub(in crate::app) fn sidebar_tabs(&self, ui: &mut egui::Ui) -> Option<SidebarTab> {
        let mut clicked = None;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(4.0);
            for tab in SidebarTab::ALL {
                let selected = self.sidebar.tab == tab;
                let font = egui::TextStyle::Body.resolve(ui.style());
                let color = if selected {
                    ui.visuals().selection.stroke.color
                } else {
                    ui.visuals().text_color()
                };
                let galley = ui
                    .painter()
                    .layout_no_wrap(tab.title().to_string(), font, color);
                let text = galley.size();
                let size = egui::vec2(SIDEBAR_TAB_WIDTH, text.x + 20.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let visuals = ui.visuals();
                let fill = if selected {
                    visuals.selection.bg_fill
                } else if response.hovered() {
                    visuals.widgets.hovered.weak_bg_fill
                } else {
                    visuals.widgets.inactive.weak_bg_fill
                };
                // Coins arrondis du côté extérieur ; texte tourné d'un quart de tour (de haut en
                // bas à droite, de bas en haut à gauche), centré dans l'onglet.
                let on_left = self.tabs_side == Side::Left;
                let (corners, origin, angle) = if on_left {
                    (
                        egui::CornerRadius {
                            nw: 8,
                            sw: 8,
                            ne: 0,
                            se: 0,
                        },
                        egui::pos2(
                            rect.center().x - text.y / 2.0,
                            rect.center().y + text.x / 2.0,
                        ),
                        -std::f32::consts::FRAC_PI_2,
                    )
                } else {
                    (
                        egui::CornerRadius {
                            nw: 0,
                            sw: 0,
                            ne: 8,
                            se: 8,
                        },
                        egui::pos2(
                            rect.center().x + text.y / 2.0,
                            rect.center().y - text.x / 2.0,
                        ),
                        std::f32::consts::FRAC_PI_2,
                    )
                };
                ui.painter().rect_filled(rect, corners, fill);
                ui.painter()
                    .add(egui::epaint::TextShape::new(origin, galley, color).with_angle(angle));
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = Some(tab);
                }
            }
        });
        clicked
    }

    /// Contenu de l'onglet actif.
    pub(in crate::app) fn sidebar_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        // Un nom trop long est tronqué, jamais coupé sur deux lignes.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        match self.sidebar.tab {
            SidebarTab::Pc => self.drives_list(ui, false),
            SidebarTab::Network => self.drives_list(ui, true),
            SidebarTab::Home => self.home_list(ui),
            SidebarTab::Favorites => self.favorites_list(ui),
            SidebarTab::Apps => self.applications_list(ui),
        }
    }

    /// Lecteurs, avec leur occupation si l'option est cochée. `network_only` : onglet « Réseau »
    /// (lecteurs réseau connectés seulement).
    pub(in crate::app) fn drives_list(
        &self,
        ui: &mut egui::Ui,
        network_only: bool,
    ) -> Option<Action> {
        let mut action = None;
        ui.heading(if network_only { "Réseau" } else { "Ce PC" });
        let cursor = self.sidebar_cursor_path();
        let mut any = false;
        for drive in &self.sidebar.drives {
            if network_only && drive.kind != DriveKind::Network {
                continue;
            }
            any = true;
            let info = self
                .sidebar
                .drive_infos
                .get(&drive.path)
                .and_then(Option::as_ref);
            let here =
                matches!(&self.nav.location, Location::Dir(path) if path.starts_with(&drive.path));
            // Pilotée au clavier : le lecteur choisi par les flèches est en surbrillance.
            let highlighted = cursor.as_ref().map_or(here, |c| *c == drive.path);
            if drive_entry(ui, drive, info, highlighted, self.show_drive_usage).clicked() {
                action = Some(Action::SidebarGo(drive.path.clone()));
            }
        }
        if !any {
            ui.weak(if network_only {
                "Aucun lecteur réseau connecté."
            } else {
                "Aucun lecteur détecté."
            });
        }
        action
    }

    /// Dossier de l'utilisateur et ses sous-dossiers.
    pub(in crate::app) fn home_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let Some(home) = &self.sidebar.home else {
            ui.weak("Dossier de l'utilisateur introuvable.");
            return None;
        };
        // Le titre est le dossier de l'utilisateur lui-même : un clic l'ouvre.
        let cursor = self.sidebar_cursor_path();
        let marked = |path: &PathBuf| {
            cursor
                .as_ref()
                .map_or(self.nav.location == Location::Dir(path.clone()), |c| {
                    c == path
                })
        };
        if ui
            .selectable_label(
                marked(home),
                egui::RichText::new(format!("🏠 {}", favorite_name(home))).heading(),
            )
            .on_hover_text(home.display().to_string())
            .clicked()
        {
            action = Some(Action::SidebarGo(home.clone()));
        }
        for entry in &self.sidebar.home_dirs {
            if entry.hidden && !self.show_hidden {
                continue;
            }
            if ui
                .selectable_label(marked(&entry.path), format!("📁 {}", entry.name))
                .on_hover_text(entry.path.display().to_string())
                .clicked()
            {
                action = Some(Action::SidebarGo(entry.path.clone()));
            }
        }
        action
    }

    /// Onglet « Applications » : ExPLIA, puis les exécutables ajoutés par l'utilisateur
    /// (clic droit sur un `.exe` > « Ajouter aux applications »). Un clic lance l'application.
    pub(in crate::app) fn applications_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.heading("Applications");
        if self.applications.paths().is_empty() {
            ui.weak("Clic droit sur un exécutable (.exe, .bat…) pour l'ajouter.");
        }
        let own = self.own_exe.iter().map(|path| (path, false));
        let added = self.applications.paths().iter().map(|path| (path, true));
        for (path, removable) in own.chain(added) {
            let name = if removable {
                path.file_stem().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            } else {
                "ExPLIA".to_string()
            };
            let response = ui
                .selectable_label(false, format!("🚀 {name}"))
                .on_hover_text(path.display().to_string())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                action = Some(Action::Open(path.clone()));
            }
            response.context_menu(|ui| {
                if let Some(chosen) = self.application_menu(ui, path, removable) {
                    action = Some(chosen);
                }
            });
        }
        action
    }

    pub(in crate::app) fn favorites_list(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.heading("Favoris");
        if self.favorites.paths().is_empty() {
            ui.weak("Clic droit sur un dossier pour l'ajouter.");
        }
        let cursor = self.sidebar_cursor_path();
        for path in self.favorites.paths() {
            let here = cursor
                .as_ref()
                .map_or(self.nav.location == Location::Dir(path.clone()), |c| {
                    c == path
                });
            let response = ui
                .selectable_label(here, format!("⭐ {}", favorite_name(path)))
                .on_hover_text(path.display().to_string());
            if response.clicked() {
                action = Some(Action::SidebarGo(path.clone()));
            }
            response.context_menu(|ui| {
                if let Some(chosen) = self.favorite_menu(ui, path) {
                    action = Some(chosen);
                }
            });
        }
        action
    }
}
