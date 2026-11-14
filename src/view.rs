//! Tri et recherche dans la liste affichée.

use std::path::Path;

use crate::fs_nav::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
    Modified,
    Created,
}

impl SortKey {
    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Type => "type",
            SortKey::Modified => "modified",
            SortKey::Created => "created",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "name" => Some(SortKey::Name),
            "size" => Some(SortKey::Size),
            "type" => Some(SortKey::Type),
            "modified" => Some(SortKey::Modified),
            "created" => Some(SortKey::Created),
            _ => None,
        }
    }
}

/// Façon d'afficher le contenu d'un dossier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// Tableau à colonnes (par défaut).
    #[default]
    List,
    /// Colonnes successives, une par niveau du chemin (navigation « style Mac »).
    Columns,
    /// Grille de miniatures (visionnage rapide des images du dossier).
    Gallery,
    /// Même grille, pour les vidéos du dossier.
    Videos,
}

impl ViewMode {
    pub fn id(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Columns => "columns",
            ViewMode::Gallery => "gallery",
            ViewMode::Videos => "videos",
        }
    }

    /// Grille de miniatures (images ou vidéos).
    pub fn is_grid(self) -> bool {
        matches!(self, ViewMode::Gallery | ViewMode::Videos)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            ViewMode::List,
            ViewMode::Columns,
            ViewMode::Gallery,
            ViewMode::Videos,
        ]
        .into_iter()
        .find(|mode| mode.id() == id)
    }
}

/// Côté de la fenêtre où se place un bloc (barre latérale, volet de détail, onglets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn id(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Side::Left, Side::Right]
            .into_iter()
            .find(|side| side.id() == id)
    }
}

/// Colonnes optionnelles du tableau (menu « Afficher »). La colonne « Nom » est toujours là.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub size: bool,
    pub modified: bool,
    pub created: bool,
    /// Dimensions des images.
    pub dimensions: bool,
    /// Résolution des vidéos.
    pub resolution: bool,
    /// Durée des vidéos et des sons.
    pub duration: bool,
}

impl Default for Columns {
    /// Par défaut : taille et date de modification.
    fn default() -> Self {
        Self {
            size: true,
            modified: true,
            created: false,
            dimensions: false,
            resolution: false,
            duration: false,
        }
    }
}

impl Columns {
    const IDS: [&'static str; 6] = [
        "size",
        "modified",
        "created",
        "dimensions",
        "resolution",
        "duration",
    ];

    fn flags(&self) -> [bool; 6] {
        [
            self.size,
            self.modified,
            self.created,
            self.dimensions,
            self.resolution,
            self.duration,
        ]
    }

    /// Identifiants des colonnes affichées, pour le fichier de réglages.
    pub fn ids(&self) -> Vec<&'static str> {
        Self::IDS
            .into_iter()
            .zip(self.flags())
            .filter_map(|(id, on)| on.then_some(id))
            .collect()
    }

    /// Reconstruit les colonnes depuis une liste d'identifiants (ceux qui sont inconnus sont ignorés).
    pub fn from_ids(text: &str) -> Self {
        let wanted: Vec<&str> = text.split(',').map(str::trim).collect();
        let has = |id: &str| wanted.contains(&id);
        Self {
            size: has("size"),
            modified: has("modified"),
            created: has("created"),
            dimensions: has("dimensions"),
            resolution: has("resolution"),
            duration: has("duration"),
        }
    }

    /// Une colonne de métadonnées multimédia (lues dans les fichiers) est-elle affichée ?
    pub fn needs_media_info(&self) -> bool {
        self.dimensions || self.resolution || self.duration
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

/// Recherche par nom, insensible à la casse. `query` peut contenir plusieurs motifs séparés par
/// une virgule ou un point-virgule (il suffit qu'un seul convienne). Un motif sans joker est
/// cherché dans le nom (`rapport`, `.exe`) ; avec `*` (suite de caractères) ou `?` (un caractère),
/// il doit correspondre au nom entier (`*.exe`, `img_??.png`). Une recherche vide garde tout.
pub fn matches_query(name: &str, query: &str) -> bool {
    let patterns: Vec<&str> = query
        .split([',', ';'])
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .collect();
    if patterns.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    let chars: Vec<char> = name.chars().collect();
    patterns.iter().any(|pattern| {
        let pattern = pattern.to_lowercase();
        if pattern.contains(['*', '?']) {
            let pattern: Vec<char> = pattern.chars().collect();
            glob_match(&pattern, &chars)
        } else {
            name.contains(&pattern)
        }
    })
}

/// Correspondance d'un motif à jokers (`*`, `?`) avec un texte entier.
fn glob_match(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    // Dernière étoile rencontrée : position dans le motif et dans le texte.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            // L'étoile absorbe un caractère de plus.
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
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
            // Un élément sans date compte comme le plus ancien.
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(by_name),
            SortKey::Created => a.created.cmp(&b.created).then_with(by_name),
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
            ..Default::default()
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir: true,
            size: None,
            ..Default::default()
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
    fn sort_by_creation_date() {
        use std::time::{Duration, UNIX_EPOCH};
        let at = |secs| Some(UNIX_EPOCH + Duration::from_secs(secs));
        let mut a = file("a", 1);
        a.created = at(300);
        let mut b = file("b", 1);
        b.created = at(100);
        let mut c = file("c", 1);
        c.created = at(200);
        let mut v = vec![a, b, c];
        sort_entries(&mut v, SortKey::Created, SortOrder::Ascending);
        assert_eq!(names(&v), ["b", "c", "a"]);
        sort_entries(&mut v, SortKey::Created, SortOrder::Descending);
        assert_eq!(names(&v), ["a", "c", "b"]);
    }

    #[test]
    fn sort_key_ids_round_trip() {
        for key in [
            SortKey::Name,
            SortKey::Size,
            SortKey::Type,
            SortKey::Modified,
            SortKey::Created,
        ] {
            assert_eq!(SortKey::from_id(key.id()), Some(key));
        }
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
    fn sort_by_modified_date() {
        use std::time::{Duration, UNIX_EPOCH};
        let dated = |name: &str, secs: u64| Entry {
            modified: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            ..file(name, 1)
        };
        let mut v = vec![
            dated("recent", 300),
            file("sans-date", 1),
            dated("ancien", 100),
        ];
        sort_entries(&mut v, SortKey::Modified, SortOrder::Ascending);
        assert_eq!(names(&v), ["sans-date", "ancien", "recent"]);
        sort_entries(&mut v, SortKey::Modified, SortOrder::Descending);
        assert_eq!(names(&v), ["recent", "ancien", "sans-date"]);
    }

    #[test]
    fn columns_default_and_round_trip() {
        let default = Columns::default();
        assert_eq!(default.ids(), ["size", "modified"]);
        assert!(!default.needs_media_info());
        let custom = Columns {
            created: true,
            duration: true,
            size: false,
            ..default
        };
        assert_eq!(Columns::from_ids(&custom.ids().join(",")), custom);
        assert!(custom.needs_media_info());
        // Une liste vide signifie « aucune colonne optionnelle ».
        assert_eq!(Columns::from_ids("").ids(), Vec::<&str>::new());
        assert_eq!(Columns::from_ids("size, inconnu").ids(), ["size"]);
    }

    #[test]
    fn view_mode_ids_round_trip() {
        for mode in [
            ViewMode::List,
            ViewMode::Columns,
            ViewMode::Gallery,
            ViewMode::Videos,
        ] {
            assert_eq!(ViewMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(ViewMode::from_id("?"), None);
        assert_eq!(ViewMode::default(), ViewMode::List);
    }

    #[test]
    fn side_ids_round_trip() {
        for side in [Side::Left, Side::Right] {
            assert_eq!(Side::from_id(side.id()), Some(side));
        }
        assert_eq!(Side::from_id("?"), None);
    }

    #[test]
    fn only_galleries_are_grids() {
        assert!(ViewMode::Gallery.is_grid() && ViewMode::Videos.is_grid());
        assert!(!ViewMode::List.is_grid() && !ViewMode::Columns.is_grid());
    }

    #[test]
    fn ids_round_trip() {
        for key in [
            SortKey::Name,
            SortKey::Size,
            SortKey::Type,
            SortKey::Modified,
        ] {
            assert_eq!(SortKey::from_id(key.id()), Some(key));
        }
        for order in [SortOrder::Ascending, SortOrder::Descending] {
            assert_eq!(SortOrder::from_id(order.id()), Some(order));
        }
        assert_eq!(SortKey::from_id("?"), None);
    }

    #[test]
    fn empty_query_keeps_everything() {
        assert!(matches_query("x.weird", ""));
        assert!(matches_query("noext", "  , ; "));
    }

    #[test]
    fn plain_query_searches_the_name_ignoring_case() {
        assert!(matches_query("Rapport 2024.docx", "rapport"));
        assert!(matches_query("Rapport 2024.docx", "PORT 20"));
        assert!(matches_query("setup.EXE", ".exe"));
        assert!(!matches_query("photo.png", "rapport"));
    }

    #[test]
    fn wildcards_match_the_whole_name() {
        assert!(matches_query("setup.exe", "*.exe"));
        assert!(matches_query("SETUP.EXE", "*.exe"));
        assert!(!matches_query("setup.exe.txt", "*.exe"));
        assert!(matches_query("img_01.png", "img_??.png"));
        assert!(!matches_query("img_001.png", "img_??.png"));
        assert!(matches_query("a-b-c.txt", "a*c.txt"));
        assert!(matches_query("abc", "*"));
        assert!(matches_query("abc", "a*b*c*"));
        assert!(!matches_query("abc", "a*d"));
    }

    #[test]
    fn several_patterns_match_any_of_them() {
        assert!(matches_query("film.mkv", "*.avi, *.mkv"));
        assert!(matches_query("notes.txt", "*.avi; notes"));
        assert!(!matches_query("photo.png", "*.avi, *.mkv"));
    }
}
