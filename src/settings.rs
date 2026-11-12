//! Réglages de l'utilisateur.
//!
//! Seul fichier écrit par ExPLIA : `%APPDATA%\ExPLIA\settings.txt`, au format
//! `clé=valeur` (une ligne par réglage ; `favorite=` et `application=` sont répétés pour chaque favori et chaque application).
//! Les fichiers de l'utilisateur ne sont jamais modifiés.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::view::{Columns, Side, SortKey, SortOrder, ViewMode};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Taille intérieure de la fenêtre, en points ; `None` tant qu'elle n'a pas été mémorisée.
    pub window_size: Option<(f32, f32)>,
    pub sort_key: SortKey,
    pub sort_order: SortOrder,
    pub show_preview: bool,
    /// Volet de détail au centre, navigation dans le volet latéral (triangle d'échange).
    pub detail_central: bool,
    pub skip_folder_sizes: bool,
    /// Option « Récursive » : recherche, dossiers vides et doublons parcourent les sous-dossiers.
    pub recursive: bool,
    /// Taille minimale (en Mo) des fichiers examinés par la recherche de doublons.
    pub duplicate_min_mb: u32,
    /// Barre d'occupation (et pourcentage) des lecteurs dans la barre latérale.
    pub show_drive_usage: bool,
    pub view_mode: ViewMode,
    /// Afficher les dossiers et fichiers cachés.
    pub show_hidden: bool,
    pub columns: Columns,
    pub favorites: Vec<PathBuf>,
    /// Disposition : côté des onglets de la barre latérale et du volet de détail.
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
            detail_central: false,
            skip_folder_sizes: false,
            recursive: true,
            duplicate_min_mb: 50,
            show_drive_usage: true,
            view_mode: ViewMode::List,
            show_hidden: false,
            columns: Columns::default(),
            favorites: Vec::new(),
            tabs_side: Side::Left,
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
                "detail_central" => settings.detail_central = value == "true",
                "skip_folder_sizes" => settings.skip_folder_sizes = value == "true",
                "recursive" => settings.recursive = value != "false",
                "duplicate_min_mb" => {
                    if let Ok(mb) = value.parse::<u32>()
                        && (1..=1_000_000).contains(&mb)
                    {
                        settings.duplicate_min_mb = mb;
                    }
                }
                "show_drive_usage" => settings.show_drive_usage = value != "false",
                "view_mode" => {
                    if let Some(mode) = ViewMode::from_id(value) {
                        settings.view_mode = mode;
                    }
                }
                "show_hidden" => settings.show_hidden = value == "true",
                "columns" => settings.columns = Columns::from_ids(value),
                "favorite" if !value.is_empty() => settings.favorites.push(PathBuf::from(value)),
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
        text += &format!("detail_central={}\n", self.detail_central);
        text += &format!("skip_folder_sizes={}\n", self.skip_folder_sizes);
        text += &format!("show_drive_usage={}\n", self.show_drive_usage);
        text += &format!("recursive={}\n", self.recursive);
        text += &format!("duplicate_min_mb={}\n", self.duplicate_min_mb);
        text += &format!("view_mode={}\n", self.view_mode.id());
        text += &format!("show_hidden={}\n", self.show_hidden);
        text += &format!("tabs_side={}\n", self.tabs_side.id());
        text += &format!("preview_side={}\n", self.preview_side.id());
        text += &format!("columns={}\n", self.columns.ids().join(","));
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
            detail_central: true,
            skip_folder_sizes: true,
            show_drive_usage: false,
            recursive: false,
            duplicate_min_mb: 120,
            view_mode: ViewMode::Videos,
            show_hidden: true,
            columns: Columns {
                size: false,
                created: true,
                duration: true,
                ..Columns::default()
            },
            favorites: vec![PathBuf::from("C:\\Projets"), PathBuf::from("D:\\Photos")],
            tabs_side: Side::Right,
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
