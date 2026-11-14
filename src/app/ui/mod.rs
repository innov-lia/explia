//! Gestion de l'interface : fenêtre, panneaux, listes, icônes, menus et actualisations.
//!
//! Ces modules ne font que dessiner l'état de l'application et renvoyer des `Action` ;
//! la logique métier reste dans les gestionnaires (`event_manager`, `action_manager`,
//! `selection_manager`, `storage_manager`, `clipboard_manager`).

mod columns;
mod detail;
mod gallery;
mod listing;
mod sidebar;
mod toolbar;
mod widgets;

pub(super) use widgets::*;

use super::*;

impl eframe::App for ExpliaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.size_window_once(&ctx);
        self.poll_events(&ctx);
        self.update_player(&ctx);
        let mut action = self.input_events(&ctx);
        // La cible d'un glisser se repère à nouveau pendant le dessin de cette image.
        self.drop_target.replace(None);
        self.drop_zones.borrow_mut().clear();

        // Vidéo en plein écran : elle seule occupe la fenêtre.
        if self.player.fullscreen && self.fullscreen_video(ui) {
            // Le clavier commande le lecteur en plein écran (voir `input_events`).
            if let Some(action) = action.take() {
                self.apply(&ctx, action);
            }
            self.track_window_size(&ctx);
            self.save_settings_if_changed();
            return;
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            enlarge_toolbar(ui);
            ui.add_space(4.0);
            action = self.options_bar(ui).or(action.take());
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                // Dossier en cours de lecture (partage réseau lent, par exemple).
                if self.listing_pending() {
                    ui.spinner();
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                } else if let Some(notice) = &self.notice {
                    ui.label(notice);
                } else {
                    ui.label(self.status_text());
                }
            });
        });
        // Barre latérale à gauche ; volet latéral (détail, ou navigation si le détail est au
        // centre) à gauche ou à droite selon la disposition ; le reste au centre.
        action = self.sidebar_panel(ui).or(action.take());
        action = self.side_panel(ui).or(action.take());
        let detail_in_center = self.detail_central && self.detail_shown();
        egui::CentralPanel::default().show(ui, |ui| {
            let central_action = if detail_in_center {
                self.preview_panel(ui)
            } else {
                self.navigation_area(ui)
            };
            action = central_action.or(action.take());
        });

        self.drag_hint(&ctx);
        self.publish_drop_zones(&ctx);
        if let Some(action) = action {
            self.apply(&ctx, action);
        }
        self.track_window_size(&ctx);
        self.save_settings_if_changed();
    }

    /// À la fermeture : mémorise aussi la taille de la fenêtre.
    fn on_exit(&mut self) {
        self.stop_player();
        let settings = self.current_settings();
        self.write_settings(settings);
    }
}
