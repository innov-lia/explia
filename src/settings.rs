//! Réglages de l'utilisateur.
//!
//! Seul fichier écrit par ExPLIA : `%APPDATA%\ExPLIA\settings.txt`, au format
//! `clé=valeur` (une ligne par réglage ; `favorite=` et `application=` sont répétés pour chaque favori et chaque application).
//! Les fichiers de l'utilisateur ne sont jamais modifiés.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::view::{Category, Columns, Side, SortKey, SortOrder, ViewMode};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Taille intérieure de la fenêtre, en points ; `None` tant qu'elle n'a pas été mémorisée.
    pub window_size: Option<(f32, f32)>,
    pub sort_key: SortKey,
    pub sort_order: SortOrder,
    pub show_preview: bool,
    pub preview_expanded: bool,
    pub skip_folder_sizes: bool,
    /// Barre d'occupation (et pourcentage) des lecteurs dans la barre latérale.
    pub show_drive_usage: bool,
    pub view_mode: ViewMode,
    /// Afficher les dossiers et fichiers cachés.
    pub show_hidden: bool,
    pub columns: Columns,
    pub categories: Vec<Category>,
    pub custom_extensions: String,
    pub favorites: Vec<PathBuf>,
    /// Disposition : côté de la barre latérale, de ses onglets et du volet de détail.
    pub sidebar_side: Side,
    pub tabs_side: Side,
    pub preview_side: Side,
    /// Applications ajoutées par l'utilisateur (onglet « Applications »).
    pub applications: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window_size: None,
            sort_key: SortKey::Name,
            sort_order: SortOrder::Ascending,
            show_preview: true,
            preview_expanded: false,
            skip_folder_sizes: false,
            show_drive_usage: true,
            view_mode: ViewMode::List,
            show_hidden: false,
            columns: Columns::default(),
            categories: Vec::new(),
            custom_extensions: String::new(),
            favorites: Vec::new(),
            sidebar_side: Side::Left,
            tabs_side: Side::Right,
            preview_side: Side::Right,
            applications: Vec::new(),
        }
    }
}

/// Emplacement du fichier de réglages, s'il est déterminable.
pub fn default_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("ExPLIA").join("settings.txt"))
}

/// Tailles de fenêtre jugées plausibles (évite un fichier corrompu).
fn valid_window_size(width: f32, height: f32) -> bool {
    (200.0..=20_000.0).contains(&width) && (150.0..=20_000.0).contains(&height)
}

impl Settings {
    /// Lit des réglages depuis un texte ; les lignes inconnues ou invalides sont ignorées.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        let (mut width, mut height) = (None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "window_width" => width = value.parse::<f32>().ok(),
                "window_height" => height = value.parse::<f32>().ok(),
                "sort_key" => {
                    if let Some(key) = SortKey::from_id(value) {
                        settings.sort_key = key;
                    }
                }
                "sort_order" => {
                    if let Some(order) = SortOrder::from_id(value) {
                        settings.sort_order = order;
                    }
                }
                "show_preview" => settings.show_preview = value != "false",
                "preview_expanded" => settings.preview_expanded = value == "true",
                "skip_folder_sizes" => settings.skip_folder_sizes = value == "true",
                "show_drive_usage" => settings.show_drive_usage = value != "false",
                "view_mode" => {
                    if let Some(mode) = ViewMode::from_id(value) {
                        settings.view_mode = mode;
                    }
                }
                "show_hidden" => settings.show_hidden = value == "true",
                "columns" => settings.columns = Columns::from_ids(value),
                "categories" => {
                    settings.categories = value
                        .split(',')
                        .filter_map(|id| Category::from_id(id.trim()))
                        .collect();
                }
                "custom_extensions" => settings.custom_extensions = value.to_string(),
                "favorite" if !value.is_empty() => settings.favorites.push(PathBuf::from(value)),
                "sidebar_side" => {
                    if let Some(side) = Side::from_id(value) {
                        settings.sidebar_side = side;
                    }
                }
                "tabs_side" => {
                    if let Some(side) = Side::from_id(value) {
                        settings.tabs_side = side;
                    }
                }
                "preview_side" => {
                    if let Some(side) = Side::from_id(value) {
                        settings.preview_side = side;
                    }
                }
                "application" if !value.is_empty() => {
                    settings.applications.push(PathBuf::from(value));
                }
                _ => {}
            }
        }
        if let (Some(w), Some(h)) = (width, height)
            && valid_window_size(w, h)
        {
            settings.window_size = Some((w, h));
        }
        settings
    }

    pub fn serialize(&self) -> String {
        let mut text = String::from("# Réglages ExPLIA (modifiés automatiquement)\n");
        if let Some((w, h)) = self.window_size {
            text += &format!("window_width={w:.0}\nwindow_height={h:.0}\n");
        }
        text += &format!("sort_key={}\n", self.sort_key.id());
        text += &format!("sort_order={}\n", self.sort_order.id());
        text += &format!("show_preview={}\n", self.show_preview);
        text += &format!("preview_expanded={}\n", self.preview_expanded);
        text += &format!("skip_folder_sizes={}\n", self.skip_folder_sizes);
        text += &format!("show_drive_usage={}\n", self.show_drive_usage);
        text += &format!("view_mode={}\n", self.view_mode.id());
        text += &format!("show_hidden={}\n", self.show_hidden);
        text += &format!("sidebar_side={}\n", self.sidebar_side.id());
        text += &format!("tabs_side={}\n", self.tabs_side.id());
        text += &format!("preview_side={}\n", self.preview_side.id());
        text += &format!("columns={}\n", self.columns.ids().join(","));
        let categories: Vec<&str> = self.categories.iter().map(|c| c.id()).collect();
        text += &format!("categories={}\n", categories.join(","));
        // Une extension personnalisée ne contient jamais de saut de ligne.
        let custom = self.custom_extensions.replace(['\r', '\n'], " ");
        text += &format!("custom_extensions={custom}\n");
        for favorite in &self.favorites {
            text += &format!("favorite={}\n", favorite.display());
        }
        for application in &self.applications {
            text += &format!("application={}\n", application.display());
        }
        text
    }

    /// Charge les réglages ; un fichier absent ou illisible donne les valeurs par défaut.
    /// Si le fichier n'existe pas encore, l'ancien `favoris.txt` (une version précédente
    /// d'ExPLIA) est repris pour ne pas perdre les favoris.
    pub fn load(file: &Path) -> Self {
        if let Ok(text) = fs::read_to_string(file) {
            return Self::parse(&text);
        }
        let mut settings = Self::default();
        let legacy = file.with_file_name("favoris.txt");
        if let Ok(text) = fs::read_to_string(legacy) {
            settings.favorites = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect();
        }
        settings
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(file, self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        Settings {
            window_size: Some((1536.0, 864.0)),
            sort_key: SortKey::Size,
            sort_order: SortOrder::Descending,
            show_preview: false,
            preview_expanded: true,
            skip_folder_sizes: true,
            show_drive_usage: false,
            view_mode: ViewMode::Videos,
            show_hidden: true,
            columns: Columns {
                size: false,
                created: true,
                duration: true,
                ..Columns::default()
            },
            categories: vec![Category::Image, Category::Code],
            custom_extensions: "mkv, .pdf".to_string(),
            favorites: vec![PathBuf::from("C:\\Projets"), PathBuf::from("D:\\Photos")],
            sidebar_side: Side::Right,
            tabs_side: Side::Left,
            preview_side: Side::Left,
            applications: vec![PathBuf::from("C:\\Outils\\app.exe")],
        }
    }

    #[test]
    fn serialize_then_parse_round_trips() {
        let settings = sample();
        assert_eq!(Settings::parse(&settings.serialize()), settings);
    }

    #[test]
    fn defaults_when_empty_or_garbage() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(
            Settings::parse("n'importe quoi\n=\nsort_key=zzz\nwindow_width=abc"),
            Settings::default()
        );
    }

    #[test]
    fn rejects_implausible_window_size() {
        let parsed = Settings::parse("window_width=5\nwindow_height=99999\n");
        assert_eq!(parsed.window_size, None);
    }

    #[test]
    fn window_size_needs_both_dimensions() {
        assert_eq!(Settings::parse("window_width=800\n").window_size, None);
        assert_eq!(
            Settings::parse("window_width=800\nwindow_height=600\n").window_size,
            Some((800.0, 600.0))
        );
    }

    #[test]
    fn missing_columns_line_keeps_the_defaults() {
        let parsed = Settings::parse("sort_key=name\n");
        assert_eq!(parsed.columns, Columns::default());
        assert_eq!(parsed.view_mode, ViewMode::List);
        assert!(!parsed.show_hidden);
        // Une ligne « columns= » vide veut dire : aucune colonne optionnelle.
        assert_eq!(
            Settings::parse("columns=\n").columns.ids(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn unknown_categories_are_ignored() {
        let parsed = Settings::parse("categories=image,video,code\n");
        assert_eq!(parsed.categories, [Category::Image, Category::Code]);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-settings");
        let file = dir.join("settings.txt");
        let settings = sample();
        settings.save(&file).unwrap();
        assert_eq!(Settings::load(&file), settings);
        let _ = fs::remove_file(file);
    }

    #[test]
    fn missing_file_gives_defaults_and_imports_legacy_favorites() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-settings-legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("favoris.txt"), "C:\\a\n\nD:\\b\n").unwrap();
        let loaded = Settings::load(&dir.join("settings.txt"));
        assert_eq!(
            loaded.favorites,
            [PathBuf::from("C:\\a"), PathBuf::from("D:\\b")]
        );
        let missing = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("absent")
            .join("settings.txt");
        assert_eq!(Settings::load(&missing), Settings::default());
        let _ = fs::remove_file(dir.join("favoris.txt"));
    }
}
