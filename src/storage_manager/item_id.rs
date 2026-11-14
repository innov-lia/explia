//! Identifiant d'un élément dans sa source de stockage.
//!
//! Les connecteurs reçoivent et renvoient des `ItemId`, jamais des chemins : une source
//! « système de fichiers » (disques, partages SMB) identifie un élément par son chemin, une
//! source distante (Microsoft Graph, plus tard) par un identifiant opaque. Seul le connecteur
//! qui a produit un identifiant sait l'interpréter.

use std::fmt;
use std::path::{Path, PathBuf};

/// Identifiant d'un élément (fichier ou dossier) dans sa source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ItemId(Key);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// Élément d'un système de fichiers Windows : son chemin absolu.
    Path(PathBuf),
}

impl Default for Key {
    fn default() -> Self {
        Key::Path(PathBuf::new())
    }
}

impl ItemId {
    /// Élément d'un système de fichiers (disque de la machine ou partage SMB).
    pub fn path(path: impl Into<PathBuf>) -> Self {
        ItemId(Key::Path(path.into()))
    }

    /// Chemin de l'élément, pour une source « système de fichiers » ; `None` pour une source
    /// distante, dont les éléments n'ont pas de chemin Windows.
    pub fn as_path(&self) -> Option<&Path> {
        match &self.0 {
            Key::Path(path) => Some(path),
        }
    }
}

impl From<&Path> for ItemId {
    fn from(path: &Path) -> Self {
        ItemId::path(path)
    }
}

impl From<&PathBuf> for ItemId {
    fn from(path: &PathBuf) -> Self {
        ItemId::path(path.clone())
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Key::Path(path) => write!(f, "{}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_system_item_keeps_its_path() {
        let id = ItemId::path(r"C:\Projets\a.txt");
        assert_eq!(id.as_path(), Some(Path::new(r"C:\Projets\a.txt")));
        assert_eq!(id.to_string(), r"C:\Projets\a.txt");
        assert_eq!(ItemId::from(Path::new(r"C:\x")), ItemId::path(r"C:\x"));
    }
}
