//! Favoris de l'utilisateur.
//!
//! Seul fichier écrit par ExPLIA : `%APPDATA%\ExPLIA\favoris.txt`
//! (un chemin par ligne). Les fichiers de l'utilisateur ne sont jamais modifiés.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Favorites {
    paths: Vec<PathBuf>,
    file: Option<PathBuf>,
}

/// Clé de comparaison : insensible à la casse et au séparateur final (Windows).
fn key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

pub fn parse(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn serialize(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("{}\n", p.display())).collect()
}

/// Emplacement du fichier de favoris, s'il est déterminable.
pub fn default_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("ExPLIA").join("favoris.txt"))
}

impl Favorites {
    /// Charge les favoris ; un fichier absent ou illisible donne une liste vide.
    pub fn load(file: Option<PathBuf>) -> Self {
        let mut favorites = Self::default();
        if let Some(text) = file.as_ref().and_then(|f| fs::read_to_string(f).ok()) {
            for path in parse(&text) {
                favorites.add(path);
            }
        }
        favorites.file = file;
        favorites
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn contains(&self, path: &Path) -> bool {
        let wanted = key(path);
        self.paths.iter().any(|p| key(p) == wanted)
    }

    /// Ajoute un dossier ; `false` s'il était déjà présent.
    pub fn add(&mut self, path: PathBuf) -> bool {
        if self.contains(&path) {
            return false;
        }
        self.paths.push(path);
        true
    }

    pub fn remove(&mut self, path: &Path) {
        let wanted = key(path);
        self.paths.retain(|p| key(p) != wanted);
    }

    /// Enregistre la liste dans le fichier de favoris.
    pub fn save(&self) -> io::Result<()> {
        let Some(file) = &self.file else {
            return Err(io::Error::other("dossier de configuration introuvable"));
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(file, serialize(&self.paths))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_blank_lines() {
        assert_eq!(
            parse("C:\\a\n\n  D:\\b  \r\n"),
            vec![PathBuf::from("C:\\a"), PathBuf::from("D:\\b")]
        );
    }

    #[test]
    fn add_is_case_insensitive_and_deduplicated() {
        let mut fav = Favorites::default();
        assert!(fav.add(PathBuf::from("C:\\Projets")));
        assert!(!fav.add(PathBuf::from("c:\\projets\\")));
        assert_eq!(fav.paths().len(), 1);
        assert!(fav.contains(Path::new("C:\\PROJETS")));
    }

    #[test]
    fn remove_works() {
        let mut fav = Favorites::default();
        fav.add(PathBuf::from("C:\\a"));
        fav.add(PathBuf::from("C:\\b"));
        fav.remove(Path::new("c:\\A"));
        assert_eq!(fav.paths(), [PathBuf::from("C:\\b")]);
    }

    #[test]
    fn save_and_load_round_trip() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-favoris")
            .join("favoris.txt");
        let mut fav = Favorites::load(Some(file.clone()));
        fav.add(PathBuf::from("C:\\Projets"));
        fav.add(PathBuf::from("D:\\Photos"));
        fav.save().unwrap();
        let reloaded = Favorites::load(Some(file.clone()));
        assert_eq!(reloaded.paths(), fav.paths());
        let _ = fs::remove_file(file);
    }

    #[test]
    fn save_without_file_is_an_error() {
        assert!(Favorites::default().save().is_err());
    }

    #[test]
    fn load_missing_file_gives_empty_list() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("absent.txt");
        assert!(Favorites::load(Some(missing)).paths().is_empty());
    }
}
