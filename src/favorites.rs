//! Liste de chemins sans doublon : favoris et applications de l'utilisateur (sauvegardées
//! avec les réglages, voir `settings`).

use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Favorites {
    paths: Vec<PathBuf>,
}

/// Clé de comparaison : insensible à la casse et au séparateur final (Windows).
fn key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

impl Favorites {
    /// Construit la liste en ignorant les doublons.
    pub fn from_paths(paths: Vec<PathBuf>) -> Self {
        let mut favorites = Self::default();
        for path in paths {
            favorites.add(path);
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn from_paths_drops_duplicates() {
        let fav = Favorites::from_paths(vec![
            PathBuf::from("C:\\a"),
            PathBuf::from("c:\\A"),
            PathBuf::from("D:\\b"),
        ]);
        assert_eq!(fav.paths().len(), 2);
    }
}
