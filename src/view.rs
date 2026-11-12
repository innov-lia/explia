//! Tri et filtrage par extension de la liste affichée.

use std::path::Path;

use crate::fs_nav::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
}

impl SortKey {
    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Type => "type",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "name" => Some(SortKey::Name),
            "size" => Some(SortKey::Size),
            "type" => Some(SortKey::Type),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn id(self) -> &'static str {
        match self {
            SortOrder::Ascending => "asc",
            SortOrder::Descending => "desc",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "asc" => Some(SortOrder::Ascending),
            "desc" => Some(SortOrder::Descending),
            _ => None,
        }
    }
}

/// Catégorie d'extensions proposée dans le menu « Extension ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Image,
    Documents,
    Code,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Image, Category::Documents, Category::Code];

    pub fn id(self) -> &'static str {
        match self {
            Category::Image => "image",
            Category::Documents => "documents",
            Category::Code => "code",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Image => "Image",
            Category::Documents => "Documents",
            Category::Code => "Code",
        }
    }

    /// Extensions (en minuscules) de la catégorie.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Category::Image => &[
                "png", "jpg", "jpeg", "bmp", "gif", "webp", "ico", "tif", "tiff", "svg", "heic",
            ],
            Category::Documents => &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf",
                "txt", "md", "csv",
            ],
            Category::Code => &[
                "rs", "py", "js", "ts", "jsx", "tsx", "c", "h", "cpp", "hpp", "cs", "java", "go",
                "rb", "php", "html", "css", "scss", "json", "xml", "yaml", "yml", "toml", "sql",
                "sh", "bat", "ps1", "lua", "kt", "swift",
            ],
        }
    }
}

/// Filtre par extension : « tout » tant qu'aucune catégorie n'est cochée et qu'aucune
/// extension personnalisée n'est saisie ; sinon, uniquement ce qui correspond à l'une
/// des catégories cochées ou des extensions personnalisées. Les dossiers restent
/// toujours visibles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionFilter {
    categories: Vec<Category>,
    /// Texte saisi par l'utilisateur, par exemple « mkv, .pdf ».
    custom: String,
}

/// Extrait les extensions d'un texte libre (séparées par virgule, point-virgule ou espace).
pub fn parse_custom_extensions(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(|part| part.trim().trim_start_matches(['*', '.']).to_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

impl ExtensionFilter {
    pub fn new(categories: Vec<Category>, custom: String) -> Self {
        let mut filter = Self {
            categories: Vec::new(),
            custom,
        };
        for category in categories {
            filter.set_category(category, true);
        }
        filter
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub fn custom(&self) -> &str {
        &self.custom
    }

    pub fn custom_mut(&mut self) -> &mut String {
        &mut self.custom
    }

    pub fn has_category(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }

    pub fn set_category(&mut self, category: Category, on: bool) {
        self.categories.retain(|c| *c != category);
        if on {
            self.categories.push(category);
        }
    }

    pub fn is_all(&self) -> bool {
        self.categories.is_empty() && parse_custom_extensions(&self.custom).is_empty()
    }

    /// Nombre de critères actifs (catégories cochées, plus un si une extension est saisie).
    pub fn count(&self) -> usize {
        self.categories.len() + usize::from(!parse_custom_extensions(&self.custom).is_empty())
    }

    pub fn clear(&mut self) {
        self.categories.clear();
        self.custom.clear();
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        if entry.is_dir || self.is_all() {
            return true;
        }
        let ext = extension_of(&entry.path);
        self.categories
            .iter()
            .any(|c| c.extensions().contains(&ext.as_str()))
            || parse_custom_extensions(&self.custom).contains(&ext)
    }
}

/// Déplace une position dans une liste de `len` éléments (flèches haut / bas).
/// Sans position courante, « bas » va au premier élément et « haut » au dernier.
/// La position reste dans la liste (pas de retour à la ligne).
pub fn step_index(current: Option<usize>, len: usize, delta: i32) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    Some(match current {
        None if delta >= 0 => 0,
        None => last,
        Some(index) => {
            let moved = index as i64 + i64::from(delta);
            moved.clamp(0, last as i64) as usize
        }
    })
}

/// Décalage de défilement minimal pour qu'une ligne soit entièrement visible.
pub fn scroll_offset_for(row: usize, row_height: f32, offset: f32, view_height: f32) -> f32 {
    let top = row as f32 * row_height;
    let bottom = top + row_height;
    if top < offset {
        top
    } else if bottom > offset + view_height {
        (bottom - view_height).max(0.0)
    } else {
        offset
    }
}

/// Pourquoi une sélection d'une plage a échoué.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    /// L'ancre ou la cible n'est pas dans la liste affichée.
    NotFound,
    /// L'ancre et la cible ne sont pas du même type (dossier / fichier).
    MixedTypes,
}

/// Sélection d'une plage (Maj+clic) : tous les éléments affichés entre l'ancre et la cible,
/// bornes comprises, dans l'ordre de la liste. Un seul type à la fois : seuls les éléments
/// du même type que l'ancre sont retenus.
pub fn select_range(
    visible: &[Entry],
    anchor: &Path,
    target: &Path,
) -> Result<Vec<Entry>, RangeError> {
    let position = |path: &Path| visible.iter().position(|e| e.path == path);
    let (Some(a), Some(b)) = (position(anchor), position(target)) else {
        return Err(RangeError::NotFound);
    };
    let anchor_is_dir = visible[a].is_dir;
    if visible[b].is_dir != anchor_is_dir {
        return Err(RangeError::MixedTypes);
    }
    let (first, last) = (a.min(b), a.max(b));
    Ok(visible[first..=last]
        .iter()
        .filter(|e| e.is_dir == anchor_is_dir)
        .cloned()
        .collect())
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
            // Un dossier dont la taille n'est pas encore connue compte pour 0.
            SortKey::Size => a
                .size
                .unwrap_or(0)
                .cmp(&b.size.unwrap_or(0))
                .then_with(by_name),
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
            size: Some(size),
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir: true,
            size: None,
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
    fn sort_by_size_uses_computed_folder_sizes() {
        let mut big = dir("big");
        big.size = Some(500);
        let mut small = dir("small");
        small.size = Some(5);
        let unknown = dir("unknown");
        let mut v = vec![big, unknown, small];
        sort_entries(&mut v, SortKey::Size, SortOrder::Descending);
        assert_eq!(names(&v), ["big", "small", "unknown"]);
    }

    #[test]
    fn sort_by_type_then_name() {
        let mut v = vec![file("z.png", 1), file("a.txt", 1), file("b.png", 1)];
        sort_entries(&mut v, SortKey::Type, SortOrder::Ascending);
        assert_eq!(names(&v), ["b.png", "z.png", "a.txt"]);
    }

    #[test]
    fn step_index_moves_and_stays_inside_the_list() {
        assert_eq!(step_index(None, 0, 1), None);
        assert_eq!(step_index(None, 5, 1), Some(0));
        assert_eq!(step_index(None, 5, -1), Some(4));
        assert_eq!(step_index(Some(2), 5, 1), Some(3));
        assert_eq!(step_index(Some(2), 5, -1), Some(1));
        assert_eq!(step_index(Some(0), 5, -1), Some(0));
        assert_eq!(step_index(Some(4), 5, 1), Some(4));
        assert_eq!(step_index(Some(0), 1, 1), Some(0));
    }

    #[test]
    fn scroll_offset_keeps_the_row_visible() {
        // Lignes de 20 px, fenêtre de 100 px (5 lignes visibles).
        assert_eq!(scroll_offset_for(2, 20.0, 0.0, 100.0), 0.0);
        // Ligne sous la zone visible : on descend juste assez.
        assert_eq!(scroll_offset_for(7, 20.0, 0.0, 100.0), 60.0);
        // Ligne au-dessus de la zone visible : on remonte à son bord haut.
        assert_eq!(scroll_offset_for(1, 20.0, 60.0, 100.0), 20.0);
        // Déjà visible : rien ne bouge.
        assert_eq!(scroll_offset_for(5, 20.0, 40.0, 100.0), 40.0);
    }

    #[test]
    fn range_selection_covers_both_ends_in_either_direction() {
        let v = vec![file("a", 1), file("b", 1), file("c", 1), file("d", 1)];
        let forward = select_range(&v, Path::new("b"), Path::new("d")).unwrap();
        assert_eq!(names(&forward), ["b", "c", "d"]);
        let backward = select_range(&v, Path::new("d"), Path::new("b")).unwrap();
        assert_eq!(names(&backward), ["b", "c", "d"]);
        let single = select_range(&v, Path::new("c"), Path::new("c")).unwrap();
        assert_eq!(names(&single), ["c"]);
    }

    #[test]
    fn range_selection_keeps_a_single_type() {
        let v = vec![dir("d1"), dir("d2"), file("f1", 1), file("f2", 1)];
        let dirs = select_range(&v, Path::new("d1"), Path::new("d2")).unwrap();
        assert_eq!(names(&dirs), ["d1", "d2"]);
        // Ancre et cible de types différents : refus.
        assert_eq!(
            select_range(&v, Path::new("d1"), Path::new("f2")),
            Err(RangeError::MixedTypes)
        );
        // Plage de fichiers : les dossiers intercalés sont ignorés.
        let mixed = vec![file("f1", 1), dir("d1"), file("f2", 1)];
        let files = select_range(&mixed, Path::new("f1"), Path::new("f2")).unwrap();
        assert_eq!(names(&files), ["f1", "f2"]);
    }

    #[test]
    fn range_selection_needs_both_ends_in_the_list() {
        let v = vec![file("a", 1)];
        assert_eq!(
            select_range(&v, Path::new("a"), Path::new("zzz")),
            Err(RangeError::NotFound)
        );
    }

    #[test]
    fn ids_round_trip() {
        for key in [SortKey::Name, SortKey::Size, SortKey::Type] {
            assert_eq!(SortKey::from_id(key.id()), Some(key));
        }
        for order in [SortOrder::Ascending, SortOrder::Descending] {
            assert_eq!(SortOrder::from_id(order.id()), Some(order));
        }
        for category in Category::ALL {
            assert_eq!(Category::from_id(category.id()), Some(category));
        }
        assert_eq!(SortKey::from_id("?"), None);
    }

    #[test]
    fn parses_custom_extensions() {
        assert_eq!(
            parse_custom_extensions(" .MKV, *.pdf ; mp4  "),
            ["mkv", "pdf", "mp4"]
        );
        assert!(parse_custom_extensions(" , . ").is_empty());
    }

    #[test]
    fn empty_filter_shows_everything() {
        let filter = ExtensionFilter::default();
        assert!(filter.is_all());
        assert!(filter.matches(&file("x.weird", 1)));
        assert!(filter.matches(&file("noext", 1)));
    }

    #[test]
    fn category_filter_keeps_matching_files_and_all_folders() {
        let mut filter = ExtensionFilter::default();
        filter.set_category(Category::Image, true);
        assert!(!filter.is_all());
        assert_eq!(filter.count(), 1);
        assert!(filter.matches(&file("a.JPG", 1)));
        assert!(filter.matches(&file("a.png", 1)));
        assert!(!filter.matches(&file("a.pdf", 1)));
        assert!(!filter.matches(&file("noext", 1)));
        assert!(filter.matches(&dir("dossier")));
        filter.set_category(Category::Image, false);
        assert!(filter.is_all());
    }

    #[test]
    fn custom_extensions_combine_with_categories() {
        let mut filter = ExtensionFilter::new(vec![Category::Code], ".mkv, avi".to_string());
        assert_eq!(filter.count(), 2);
        assert!(filter.matches(&file("main.rs", 1)));
        assert!(filter.matches(&file("film.MKV", 1)));
        assert!(filter.matches(&file("film.avi", 1)));
        assert!(!filter.matches(&file("photo.png", 1)));
        filter.clear();
        assert!(filter.is_all());
        assert!(filter.matches(&file("photo.png", 1)));
    }
}
