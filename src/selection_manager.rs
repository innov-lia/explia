//! Gestion de la sélection : l'élément sélectionné, la multi-sélection et son point d'ancrage.
//!
//! Logique pure, sans interface : la sélection ne connaît ni egui ni l'aperçu. Elle applique
//! la règle d'ExPLIA « un seul type à la fois » (dossiers ou fichiers, jamais les deux) et
//! décrit les éléments visés par une commande (`targets`, `items`). Les effets visibles
//! (aperçu, messages, curseur) restent du ressort de l'application.

use std::path::{Path, PathBuf};

use crate::fs_nav::Entry;
use crate::storage_manager::{self, ItemId, SourceKind};

/// Message affiché quand on tente de mélanger dossiers et fichiers.
pub const MIXED_TYPES_MESSAGE: &str =
    "Sélection d'un seul type à la fois : dossiers ou fichiers, pas les deux.";

/// Nature d'un élément sélectionné.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Folder,
}

impl ItemKind {
    pub fn of(entry: &Entry) -> Self {
        if entry.is_dir {
            ItemKind::Folder
        } else {
            ItemKind::File
        }
    }
}

/// Référence à un élément visé par une commande : identifiant dans sa source, chemin affiché,
/// nature et source de stockage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    pub id: ItemId,
    pub path: PathBuf,
    pub kind: ItemKind,
    pub source: SourceKind,
}

impl ItemRef {
    pub fn from_entry(entry: &Entry) -> Self {
        Self {
            id: entry.id.clone(),
            path: entry.path.clone(),
            kind: ItemKind::of(entry),
            source: storage_manager::source_of(&entry.id),
        }
    }
}

/// Pourquoi une modification de la sélection a échoué.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    /// L'ancre ou la cible n'est pas dans la liste affichée.
    NotFound,
    /// Dossiers et fichiers mélangés.
    MixedTypes,
}

/// Effet d'un Ctrl+clic (`Selection::toggle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    /// L'élément rejoint la multi-sélection.
    Added,
    /// L'élément quitte la multi-sélection.
    Removed,
    /// Ctrl+clic sur l'unique élément sélectionné : plus rien n'est sélectionné.
    Deselected,
}

/// Sélection courante de la liste affichée.
#[derive(Debug, Default, Clone)]
pub struct Selection {
    /// Élément sélectionné seul (aperçu, détail).
    pub single: Option<Entry>,
    /// Multi-sélection : un seul type à la fois (tous dossiers, ou tous fichiers).
    pub multi: Vec<Entry>,
    /// Dernier élément choisi : point de départ d'une sélection Maj+clic.
    pub anchor: Option<PathBuf>,
    /// Multi-sélection démarrée par un clic maintenu : un simple clic ajoute ou retire
    /// une ligne. Sinon (Ctrl/Maj), un simple clic remplace la sélection, comme l'Explorateur.
    pub toggle_mode: bool,
}

impl Selection {
    /// Clic simple : `entry` devient le seul élément sélectionné.
    pub fn select_one(&mut self, entry: Entry) {
        self.clear_multi();
        self.anchor = Some(entry.path.clone());
        self.single = Some(entry);
    }

    /// Quitte la multi-sélection (l'élément seul éventuel est conservé).
    pub fn clear_multi(&mut self) {
        self.multi.clear();
        self.toggle_mode = false;
    }

    /// `path` fait partie de la multi-sélection.
    pub fn in_multi(&self, path: &Path) -> bool {
        self.multi.iter().any(|m| m.path == path)
    }

    /// `path` est sélectionné (seul ou dans la multi-sélection).
    pub fn contains(&self, path: &Path) -> bool {
        self.single.as_ref().is_some_and(|s| s.path == path) || self.in_multi(path)
    }

    /// Ctrl+clic : ajoute ou retire `entry` de la multi-sélection. Comme l'Explorateur, le
    /// premier Ctrl+clic garde l'élément déjà sélectionné seul.
    pub fn toggle(&mut self, entry: Entry) -> Result<Toggle, SelectionError> {
        if self.multi.is_empty()
            && let Some(current) = self.single.clone()
        {
            if current.path == entry.path {
                self.single = None;
                return Ok(Toggle::Deselected);
            }
            if current.is_dir != entry.is_dir {
                return Err(SelectionError::MixedTypes);
            }
            self.multi.push(current);
        }
        self.single = None;
        if let Some(position) = self.multi.iter().position(|e| e.path == entry.path) {
            self.multi.remove(position);
            if self.multi.is_empty() {
                self.toggle_mode = false;
            }
            return Ok(Toggle::Removed);
        }
        if let Some(first) = self.multi.first()
            && first.is_dir != entry.is_dir
        {
            return Err(SelectionError::MixedTypes);
        }
        self.anchor = Some(entry.path.clone());
        self.multi.push(entry);
        Ok(Toggle::Added)
    }

    /// Maj+clic : sélectionne, dans `list`, la plage entre l'ancre (ou l'élément seul) et
    /// `target`. `NotFound` : pas d'ancre utilisable, l'appelant se rabat sur `toggle`.
    pub fn select_range(&mut self, list: &[Entry], target: &Path) -> Result<(), SelectionError> {
        let anchor = self
            .anchor
            .clone()
            .or_else(|| self.single.as_ref().map(|s| s.path.clone()))
            .ok_or(SelectionError::NotFound)?;
        let range = range_between(list, &anchor, target)?;
        self.single = None;
        self.anchor = Some(anchor);
        self.multi = range;
        Ok(())
    }

    /// Éléments visés par une commande sur la sélection : la multi-sélection, sinon l'élément
    /// seul.
    pub fn targets(&self) -> Vec<Entry> {
        if self.multi.is_empty() {
            self.single.iter().cloned().collect()
        } else {
            self.multi.clone()
        }
    }

    /// Éléments visés par une commande lancée depuis `entry` (menu contextuel, glisser) :
    /// toute la multi-sélection si `entry` en fait partie, sinon `entry` seul.
    pub fn targets_from(&self, entry: &Entry) -> Vec<Entry> {
        if self.in_multi(&entry.path) {
            self.multi.clone()
        } else {
            vec![entry.clone()]
        }
    }

    /// Identifiants (dans leur source) des éléments visés par une commande sur la sélection.
    pub fn target_ids(&self) -> Vec<ItemId> {
        self.targets().into_iter().map(|e| e.id).collect()
    }

    /// Chemins de `targets_from`.
    pub fn paths_from(&self, entry: &Entry) -> Vec<PathBuf> {
        self.targets_from(entry)
            .into_iter()
            .map(|e| e.path)
            .collect()
    }
}

/// Plage (Maj+clic) : tous les éléments affichés entre l'ancre et la cible, bornes comprises,
/// dans l'ordre de la liste. Un seul type à la fois : seuls les éléments du même type que
/// l'ancre sont retenus.
pub fn range_between(
    visible: &[Entry],
    anchor: &Path,
    target: &Path,
) -> Result<Vec<Entry>, SelectionError> {
    let position = |path: &Path| visible.iter().position(|e| e.path == path);
    let (Some(a), Some(b)) = (position(anchor), position(target)) else {
        return Err(SelectionError::NotFound);
    };
    let anchor_is_dir = visible[a].is_dir;
    if visible[b].is_dir != anchor_is_dir {
        return Err(SelectionError::MixedTypes);
    }
    let (first, last) = (a.min(b), a.max(b));
    Ok(visible[first..=last]
        .iter()
        .filter(|e| e.is_dir == anchor_is_dir)
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            id: ItemId::path(name),
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir,
            size: if is_dir { None } else { Some(1) },
            hidden: false,
            modified: None,
            created: None,
        }
    }

    fn file(name: &str) -> Entry {
        entry(name, false)
    }

    fn dir(name: &str) -> Entry {
        entry(name, true)
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn range_covers_both_ends_in_either_direction() {
        let v = vec![file("a"), file("b"), file("c"), file("d")];
        let forward = range_between(&v, Path::new("b"), Path::new("d")).unwrap();
        assert_eq!(names(&forward), ["b", "c", "d"]);
        let backward = range_between(&v, Path::new("d"), Path::new("b")).unwrap();
        assert_eq!(names(&backward), ["b", "c", "d"]);
        let single = range_between(&v, Path::new("c"), Path::new("c")).unwrap();
        assert_eq!(names(&single), ["c"]);
    }

    #[test]
    fn range_keeps_a_single_type() {
        let v = vec![dir("d1"), dir("d2"), file("f1"), file("f2")];
        let dirs = range_between(&v, Path::new("d1"), Path::new("d2")).unwrap();
        assert_eq!(names(&dirs), ["d1", "d2"]);
        assert_eq!(
            range_between(&v, Path::new("d1"), Path::new("f2")),
            Err(SelectionError::MixedTypes)
        );
        // Plage de fichiers : les dossiers intercalés sont ignorés.
        let mixed = vec![file("f1"), dir("d1"), file("f2")];
        let files = range_between(&mixed, Path::new("f1"), Path::new("f2")).unwrap();
        assert_eq!(names(&files), ["f1", "f2"]);
    }

    #[test]
    fn range_needs_both_ends_in_the_list() {
        let v = vec![file("a")];
        assert_eq!(
            range_between(&v, Path::new("a"), Path::new("zzz")),
            Err(SelectionError::NotFound)
        );
    }

    #[test]
    fn first_ctrl_click_keeps_the_single_selection() {
        let mut selection = Selection::default();
        selection.select_one(file("a"));
        assert_eq!(selection.toggle(file("b")), Ok(Toggle::Added));
        assert_eq!(names(&selection.multi), ["a", "b"]);
        assert!(selection.single.is_none());
        assert_eq!(selection.anchor.as_deref(), Some(Path::new("b")));
        assert_eq!(selection.toggle(file("a")), Ok(Toggle::Removed));
        assert_eq!(names(&selection.multi), ["b"]);
    }

    #[test]
    fn ctrl_click_on_the_single_selection_deselects_it() {
        let mut selection = Selection::default();
        selection.select_one(file("a"));
        assert_eq!(selection.toggle(file("a")), Ok(Toggle::Deselected));
        assert!(selection.single.is_none() && selection.multi.is_empty());
    }

    #[test]
    fn folders_and_files_are_never_mixed() {
        let mut selection = Selection::default();
        selection.select_one(dir("d"));
        assert_eq!(selection.toggle(file("f")), Err(SelectionError::MixedTypes));
        // La sélection d'origine est intacte.
        assert_eq!(
            selection.single.as_ref().map(|e| e.name.as_str()),
            Some("d")
        );
        selection.toggle(dir("e")).unwrap();
        assert_eq!(selection.toggle(file("f")), Err(SelectionError::MixedTypes));
        assert_eq!(names(&selection.multi), ["d", "e"]);
    }

    #[test]
    fn emptying_the_multi_selection_leaves_toggle_mode() {
        let mut selection = Selection {
            toggle_mode: true,
            ..Selection::default()
        };
        selection.toggle(file("a")).unwrap();
        selection.toggle(file("a")).unwrap();
        assert!(!selection.toggle_mode);
    }

    #[test]
    fn shift_click_selects_from_the_anchor() {
        let list = vec![file("a"), file("b"), file("c")];
        let mut selection = Selection::default();
        assert_eq!(
            selection.select_range(&list, Path::new("c")),
            Err(SelectionError::NotFound)
        );
        selection.select_one(file("a"));
        selection.select_range(&list, Path::new("c")).unwrap();
        assert_eq!(names(&selection.multi), ["a", "b", "c"]);
        assert!(selection.single.is_none());
    }

    #[test]
    fn commands_target_the_multi_selection_only_from_one_of_its_items() {
        let mut selection = Selection::default();
        selection.select_one(file("a"));
        selection.toggle(file("b")).unwrap();
        assert_eq!(names(&selection.targets()), ["a", "b"]);
        assert_eq!(names(&selection.targets_from(&file("b"))), ["a", "b"]);
        assert_eq!(names(&selection.targets_from(&file("z"))), ["z"]);
        assert_eq!(ItemKind::of(&dir("d")), ItemKind::Folder);
    }
}
