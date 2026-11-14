//! Éléments graphiques réutilisables (lignes, en-têtes, tuiles, boutons).

use super::super::*;

/// Bouton aux couleurs de PowerShell (fond bleu nuit, texte blanc).
pub(in crate::app) fn powershell_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(1, 36, 86)),
    )
}

/// Ligne « .. » en tête de liste : un clic revient au dossier précédent (le parent).
pub(in crate::app) fn parent_row(ui: &mut egui::Ui) -> egui::Response {
    table_row(ui, false, false, "⬆ ..", &[], &[])
        .on_hover_text("Dossier parent (Retour arrière)")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Mot du fil d'Ariane : sans cadre au repos, bouton coloré au survol (voir `breadcrumb_bar`).
pub(in crate::app) fn crumb_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).frame_when_inactive(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Agrandit le texte et les boutons d'une barre du haut (style local à `ui`).
pub(in crate::app) fn enlarge_toolbar(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size += TOOLBAR_TEXT_BOOST;
        }
    }
    style.spacing.interact_size.y = 30.0;
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
}

/// Bouton de menu qui reste ouvert tant qu'on ne clique pas à l'extérieur.
pub(in crate::app) fn menu(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    MenuButton::new(title)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, add_contents)
        .0
}

/// Ligne en cours de renommage : Entrée valide, Échap ou un clic ailleurs annule.
pub(in crate::app) fn rename_row(
    ui: &mut egui::Ui,
    state: &mut RenameState,
    reserved: f32,
) -> Option<Action> {
    let id = egui::Id::unique("rename-edit");
    let width = (ui.available_width() - reserved).max(120.0);
    let edit = ui.add_sized(
        [width, table_row_height(ui)],
        egui::TextEdit::singleline(&mut state.text).id(id),
    );
    if state.frames == 0 {
        edit.request_focus();
    } else if state.frames == 1 {
        // Sélectionne le nom sans l'extension, comme l'Explorateur.
        if let Some(mut edit_state) = egui::TextEdit::load_state(ui.ctx(), id) {
            edit_state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(state.initial_selection),
                )));
            edit_state.store(ui.ctx(), id);
        }
    }
    state.frames += 1;
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if edit.lost_focus() && enter {
        return Some(Action::CommitRename {
            path: state.path.clone(),
            new_name: state.text.clone(),
        });
    }
    if escape || (edit.lost_focus() && state.frames > 2) {
        return Some(Action::CancelRename);
    }
    None
}

pub(in crate::app) fn table_row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y
}

/// En-tête du tableau : « Nom » à gauche, puis les colonnes demandées alignées à droite
/// (`columns` dans l'ordre d'affichage, de gauche à droite). Un clic sur un titre renvoie le
/// critère de tri correspondant ; le tri actif (`sort`) est marqué d'une flèche.
pub(in crate::app) fn table_header(
    ui: &mut egui::Ui,
    columns: &[Column],
    sort: (SortKey, SortOrder),
) -> Option<SortKey> {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::hover(),
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pad = ui.spacing().button_padding.x;
    let mut clicked = None;
    // Zones des titres : « Nom » prend la place restante, chaque colonne sa largeur.
    let mut cells: Vec<(&str, Option<SortKey>, egui::Rect, egui::Align2)> = Vec::new();
    let mut right = rect.right() - pad;
    for column in columns.iter().rev() {
        let cell = egui::Rect::from_min_max(
            egui::pos2(right - column.width() + pad, rect.top()),
            egui::pos2(right + pad, rect.bottom()),
        );
        cells.push((
            column.title(),
            column.sort_key(),
            cell,
            egui::Align2::RIGHT_CENTER,
        ));
        right -= column.width();
    }
    let name_cell = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((right + pad).max(rect.left() + pad), rect.bottom()),
    );
    cells.push((
        "Nom",
        Some(SortKey::Name),
        name_cell,
        egui::Align2::LEFT_CENTER,
    ));
    for (title, key, cell, align) in cells {
        let response = key.map(|_| {
            ui.interact(
                cell,
                ui.scope_id().with(("header", title)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
        });
        let hovered = response.as_ref().is_some_and(egui::Response::hovered);
        let active = key == Some(sort.0);
        let color = if hovered {
            ui.visuals().hyperlink_color
        } else {
            ui.visuals().strong_text_color()
        };
        let arrow = match (active, sort.1) {
            (false, _) => "",
            (true, SortOrder::Ascending) => " ▲",
            (true, SortOrder::Descending) => " ▼",
        };
        let anchor = match align {
            egui::Align2::LEFT_CENTER => egui::pos2(cell.left() + pad, cell.center().y),
            _ => egui::pos2(cell.right() - pad, cell.center().y),
        };
        ui.painter().text(
            anchor,
            align,
            format!("{title}{arrow}"),
            font.clone(),
            color,
        );
        if response.is_some_and(|r| r.clicked()) {
            clicked = key;
        }
    }
    ui.separator();
    clicked
}

/// Ligne du tableau : nom à gauche, puis une cellule par colonne, alignées à droite.
/// `dimmed` : élément caché (texte atténué). Renvoie la réponse au clic.
pub(in crate::app) fn table_row(
    ui: &mut egui::Ui,
    selected: bool,
    dimmed: bool,
    name: &str,
    columns: &[Column],
    cells: &[String],
) -> egui::Response {
    // Clic et glisser : une ligne se prend à la souris (glisser-déposer). Avec un simple clic,
    // egui abandonnerait la ligne dès que le pointeur bouge et la liste défilerait à la place.
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), table_row_height(ui)),
        egui::Sense::click_and_drag(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let corner = visuals.widgets.inactive.corner_radius;
        let normal_text = if dimmed {
            visuals.weak_text_color()
        } else {
            visuals.text_color()
        };
        let (background, text_color) = if selected {
            (
                Some(visuals.selection.bg_fill),
                visuals.selection.stroke.color,
            )
        } else if response.hovered() {
            (Some(visuals.widgets.hovered.weak_bg_fill), normal_text)
        } else {
            (None, normal_text)
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let pad = ui.spacing().button_padding.x;
        let painter = ui.painter();
        if let Some(fill) = background {
            painter.rect_filled(rect, corner, fill);
        }
        let mut right = rect.right() - pad;
        for (column, cell) in columns.iter().zip(cells).rev() {
            painter.text(
                egui::pos2(right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                cell,
                font.clone(),
                text_color,
            );
            right -= column.width();
        }
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + pad, rect.top()),
            egui::pos2((right - pad).max(rect.left() + pad), rect.bottom()),
        );
        // Un nom trop long est coupé par « … ».
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width());
        let galley = painter.layout_job(job);
        let top_left = egui::pos2(name_rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(top_left, galley, text_color);
    }
    response
}

/// Largeur fixe d'une colonne de la vue « style Mac » : celle du nom de référence
/// (`COLUMN_VIEW_REFERENCE_NAME`) suivi de « … », avec l'icône et une marge pour le remplissage
/// des lignes et la barre de défilement.
pub(in crate::app) fn fixed_column_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let reference = ui.painter().layout_no_wrap(
        format!("📁 {COLUMN_VIEW_REFERENCE_NAME}…"),
        font,
        egui::Color32::WHITE,
    );
    reference.size().x + 44.0
}

/// Tuile de la grille d'images : miniature (ou icône) et nom dessous.
pub(in crate::app) fn gallery_tile(
    ui: &mut egui::Ui,
    entry: &Entry,
    selected: bool,
    thumb: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(GALLERY_TILE, GALLERY_TILE + GALLERY_LABEL_HEIGHT),
        // Clic et glisser, comme les lignes (voir `table_row`).
        egui::Sense::click_and_drag(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals();
    let corner = visuals.widgets.inactive.corner_radius;
    let (background, text_color) = if selected {
        (
            Some(visuals.selection.bg_fill),
            visuals.selection.stroke.color,
        )
    } else if response.hovered() {
        (
            Some(visuals.widgets.hovered.weak_bg_fill),
            visuals.text_color(),
        )
    } else if entry.hidden {
        (None, visuals.weak_text_color())
    } else {
        (None, visuals.text_color())
    };
    let painter = ui.painter();
    if let Some(fill) = background {
        painter.rect_filled(rect, corner, fill);
    }
    let picture = egui::Rect::from_min_size(
        rect.min + egui::vec2(6.0, 6.0),
        egui::vec2(GALLERY_TILE - 12.0, GALLERY_TILE - 12.0),
    );
    match thumb {
        Some(texture) => {
            // Miniature ajustée à la zone, proportions conservées et centrée.
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y);
            let fitted = egui::Rect::from_center_size(picture.center(), size * scale);
            painter.image(
                texture.id(),
                fitted,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
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
            painter.text(
                picture.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(54.0),
                text_color,
            );
        }
    }
    let label = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.bottom() - GALLERY_LABEL_HEIGHT),
        egui::pos2(rect.right() - 4.0, rect.bottom()),
    );
    painter.with_clip_rect(label).text(
        label.center(),
        egui::Align2::CENTER_CENTER,
        &entry.name,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    response
}

/// Ligne simple : nom et taille (listes de lecteurs, de sélection, d'archive).
pub(in crate::app) fn size_row(
    ui: &mut egui::Ui,
    selected: bool,
    name: &str,
    size: &str,
) -> egui::Response {
    table_row(
        ui,
        selected,
        false,
        name,
        &[Column::Size],
        &[size.to_string()],
    )
}

/// Couleur de la barre d'occupation : verte, orange à partir de 75 %, rouge à partir de 90 %.
pub(in crate::app) fn usage_color(percent: u8) -> egui::Color32 {
    match percent {
        90.. => egui::Color32::from_rgb(190, 35, 45),
        75..=89 => egui::Color32::from_rgb(190, 125, 0),
        _ => egui::Color32::from_rgb(30, 140, 55),
    }
}

/// Texte de l'infobulle d'un lecteur : type, nom, format, capacité, occupation.
pub(in crate::app) fn drive_tooltip(drive: &Drive, info: Option<&DriveInfo>) -> String {
    let mut text = format!("{}  —  {}", drive.label(), drive.kind_name());
    let Some(info) = info else {
        return text;
    };
    if !info.name.is_empty() {
        text += &format!("\nNom : {}", info.name);
    }
    if !info.file_system.is_empty() {
        text += &format!("\nFormat : {}", info.file_system);
    }
    text += &format!(
        "\nTotal : {}\nUtilisé : {} ({} %)\nLibre : {}",
        fs_nav::format_size(info.total),
        fs_nav::format_size(info.used()),
        info.used_percent(),
        fs_nav::format_size(info.free)
    );
    text
}

/// Ligne d'un lecteur dans la barre latérale : icône, lettre et nom du volume ; dessous, si
/// l'option est cochée, la barre d'occupation avec le pourcentage à l'intérieur.
pub(in crate::app) fn drive_entry(
    ui: &mut egui::Ui,
    drive: &Drive,
    info: Option<&DriveInfo>,
    selected: bool,
    show_usage: bool,
) -> egui::Response {
    let line = table_row_height(ui);
    let usage = info.filter(|info| show_usage && info.total > 0);
    let height = if usage.is_some() {
        line + DRIVE_BAR_HEIGHT + 4.0
    } else {
        line
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let (background, text_color) = if selected {
            (
                Some(visuals.selection.bg_fill),
                visuals.selection.stroke.color,
            )
        } else if response.hovered() {
            (
                Some(visuals.widgets.hovered.weak_bg_fill),
                visuals.text_color(),
            )
        } else {
            (None, visuals.text_color())
        };
        let corner = visuals.widgets.inactive.corner_radius;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let pad = ui.spacing().button_padding.x;
        let mut title = format!("{} {}", drive_icon(drive.kind), drive.label());
        if let Some(info) = info
            && !info.name.is_empty()
        {
            title += &format!("  {}", info.name);
        }
        let painter = ui.painter();
        if let Some(fill) = background {
            painter.rect_filled(rect, corner, fill);
        }
        let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), line));
        painter.with_clip_rect(title_rect).text(
            egui::pos2(rect.left() + pad, title_rect.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            font.clone(),
            text_color,
        );
        if let Some(info) = usage {
            let percent = info.used_percent();
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + pad, rect.top() + line),
                egui::vec2((rect.width() - 2.0 * pad).max(20.0), DRIVE_BAR_HEIGHT),
            );
            painter.rect_filled(bar, 3.0, egui::Color32::from_gray(60));
            let mut filled = bar;
            filled.set_width(bar.width() * f32::from(percent) / 100.0);
            painter.rect_filled(filled, 3.0, usage_color(percent));
            painter.text(
                bar.center(),
                egui::Align2::CENTER_CENTER,
                format!("{percent} %"),
                egui::FontId::proportional(font.size - 1.5),
                egui::Color32::WHITE,
            );
        }
    }
    response.on_hover_text(drive_tooltip(drive, info))
}

pub(in crate::app) fn drive_icon(kind: DriveKind) -> &'static str {
    match kind {
        // Symboles présents dans la police d'egui (le symbole « disque dur » n'y est pas).
        DriveKind::Fixed => "💽",
        DriveKind::Removable => "🔌",
        DriveKind::Network => "🌐",
        DriveKind::Optical => "📀",
        DriveKind::Other => "💽",
    }
}
