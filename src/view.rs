//! Tri et filtrage par extension de la liste affichée.

use std::path::Path;

use crate::fs_nav::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Extensions courantes proposées dans le menu « Extension » :
/// libellé affiché et extensions correspondantes (en minuscules).
pub const COMMON_EXTENSIONS: &[(&str, &[&str])] = &[
    (".png", &["png"]),
    (".jpg / .jpeg", &["jpg", "jpeg"]),
    (".bmp", &["bmp"]),
    (".gif", &["gif"]),
    (".pdf", &["pdf"]),
    (".txt", &["txt"]),
    (".doc / .docx", &["doc", "docx"]),
    (".xls / .xlsx", &["xls", "xlsx"]),
    (".ppt / .pptx", &["ppt", "pptx"]),
    (".zip", &["zip"]),
    (".mp3", &["mp3"]),
    (".mp4", &["mp4"]),
    (".exe", &["exe"]),
];

/// Filtre par extension : « tout » tant que rien n'est coché, sinon uniquement
/// les extensions cochées. Les dossiers restent toujours visibles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionFilter {
    selected: Vec<bool>,
}

impl Default for ExtensionFilter {
    fn default() -> Self {
        Self {
            selected: vec![false; COMMON_EXTENSIONS.len()],
        }
    }
}

impl ExtensionFilter {
    pub fn is_all(&self) -> bool {
        !self.selected.iter().any(|&on| on)
    }

    pub fn count(&self) -> usize {
        self.selected.iter().filter(|&&on| on).count()
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.get(index).copied().unwrap_or(false)
    }

    pub fn set(&mut self, index: usize, on: bool) {
        if let Some(slot) = self.selected.get_mut(index) {
            *slot = on;
        }
    }

    pub fn clear(&mut self) {
        self.selected.fill(false);
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        if entry.is_dir || self.is_all() {
            return true;
        }
        let ext = extension_of(&entry.path);
        COMMON_EXTENSIONS
            .iter()
            .zip(&self.selected)
            .any(|((_, exts), &on)| on && exts.contains(&ext.as_str()))
    }
}

/// Extension en minuscules, vide s'il n'y en a pas.
pub fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Trie les entrées ; les dossiers restent toujours avant les fichiers.
pub fn sort_entries(entries: &mut [Entry], key: SortKey, order: SortOrder) {
    entries.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let ordering = match key {
            SortKey::Name => by_name(),
            SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
            SortKey::Type => extension_of(&a.path)
                .cmp(&extension_of(&b.path))
                .then_with(by_name),
        };
        let ordering = match order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        };
        b.is_dir.cmp(&a.is_dir).then(ordering)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str, size: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir: false,
            size,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir: true,
            size: 0,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn sort_by_name_keeps_folders_first() {
        let mut v = vec![file("b.txt", 1), dir("Zdir"), file("A.txt", 2), dir("adir")];
        sort_entries(&mut v, SortKey::Name, SortOrder::Ascending);
        assert_eq!(names(&v), ["adir", "Zdir", "A.txt", "b.txt"]);
        sort_entries(&mut v, SortKey::Name, SortOrder::Descending);
        assert_eq!(names(&v), ["Zdir", "adir", "b.txt", "A.txt"]);
    }

    #[test]
    fn sort_by_size() {
        let mut v = vec![file("a", 30), file("b", 10), file("c", 20)];
        sort_entries(&mut v, SortKey::Size, SortOrder::Ascending);
        assert_eq!(names(&v), ["b", "c", "a"]);
        sort_entries(&mut v, SortKey::Size, SortOrder::Descending);
        assert_eq!(names(&v), ["a", "c", "b"]);
    }

    #[test]
    fn sort_by_type_then_name() {
        let mut v = vec![file("z.png", 1), file("a.txt", 1), file("b.png", 1)];
        sort_entries(&mut v, SortKey::Type, SortOrder::Ascending);
        assert_eq!(names(&v), ["b.png", "z.png", "a.txt"]);
    }

    #[test]
    fn empty_filter_shows_everything() {
        let filter = ExtensionFilter::default();
        assert!(filter.is_all());
        assert!(filter.matches(&file("x.weird", 1)));
        assert!(filter.matches(&file("noext", 1)));
    }

    #[test]
    fn filter_keeps_only_checked_extensions_and_all_folders() {
        let mut filter = ExtensionFilter::default();
        filter.set(1, true); // .jpg / .jpeg
        assert!(!filter.is_all());
        assert_eq!(filter.count(), 1);
        assert!(filter.matches(&file("a.JPG", 1)));
        assert!(filter.matches(&file("a.jpeg", 1)));
        assert!(!filter.matches(&file("a.png", 1)));
        assert!(!filter.matches(&file("noext", 1)));
        assert!(filter.matches(&dir("dossier")));
        filter.clear();
        assert!(filter.matches(&file("a.png", 1)));
    }
}
