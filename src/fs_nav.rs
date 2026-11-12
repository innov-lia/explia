//! Navigation dans le système de fichiers (lecture seule).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Taille en octets. Pour un dossier : `None` tant qu'elle n'est pas calculée.
    pub size: Option<u64>,
    /// Élément caché (attribut « caché » de Windows, ou nom commençant par un point).
    pub hidden: bool,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

/// Vrai si l'élément est caché : attribut « caché » de Windows (`meta` doit décrire l'élément
/// lui-même, sans suivre les liens) ou nom commençant par un point (`.git`, `.config`, …).
pub fn is_hidden(name: &str, meta: Option<&fs::Metadata>) -> bool {
    name.starts_with('.') || has_hidden_attribute(meta)
}

#[cfg(windows)]
fn has_hidden_attribute(meta: Option<&fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    /// `FILE_ATTRIBUTE_HIDDEN`.
    const HIDDEN: u32 = 0x2;
    meta.is_some_and(|m| m.file_attributes() & HIDDEN != 0)
}

#[cfg(not(windows))]
fn has_hidden_attribute(_meta: Option<&fs::Metadata>) -> bool {
    false
}

/// Lien symbolique ou jonction (point d'analyse Windows) : jamais suivi lors d'un parcours.
/// `meta` doit décrire l'élément lui-même (`DirEntry::metadata`, pas `fs::metadata`).
pub fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        /// `FILE_ATTRIBUTE_REPARSE_POINT`.
        const REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Dossiers qui séparent `path` de `root` (exclu), du plus proche au plus éloigné, en commençant
/// par le dossier qui contient `path` : `a\\b` puis `a` pour `root\\a\\b\\f.txt`. Vide si `path`
/// n'est pas sous `root` ou directement dans `root`.
pub fn levels_below(root: &Path, path: &Path) -> Vec<PathBuf> {
    if !path.starts_with(root) {
        return Vec::new();
    }
    path.ancestors()
        .skip(1)
        .take_while(|ancestor| *ancestor != root && ancestor.starts_with(root))
        .map(Path::to_path_buf)
        .collect()
}

/// Construit l'entrée décrivant un chemin (nom, type, taille, dates). `None` s'il est illisible.
pub fn entry_from_path(path: &Path) -> Option<Entry> {
    let own = fs::symlink_metadata(path).ok()?;
    let meta = fs::metadata(path).unwrap_or_else(|_| own.clone());
    let name = path.file_name()?.to_string_lossy().into_owned();
    let is_dir = meta.is_dir();
    let hidden = is_hidden(&name, Some(&own));
    Some(Entry {
        name,
        path: path.to_path_buf(),
        is_dir,
        size: if is_dir { None } else { Some(meta.len()) },
        hidden,
        modified: meta.modified().ok(),
        created: meta.created().ok(),
    })
}

/// Où se trouve l'utilisateur : l'accueil (liste des lecteurs) ou un dossier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    ThisPc,
    Dir(PathBuf),
}

impl Location {
    /// Texte affiché dans la barre d'adresse.
    pub fn display(&self) -> String {
        match self {
            Location::ThisPc => "Ce PC".to_string(),
            Location::Dir(path) => path.display().to_string(),
        }
    }

    /// Emplacement parent ; depuis la racine d'un lecteur, on revient à « Ce PC ».
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::ThisPc => None,
            Location::Dir(path) => Some(match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    Location::Dir(parent.to_path_buf())
                }
                _ => Location::ThisPc,
            }),
        }
    }
}

/// Segments cliquables d'un chemin : libellé et chemin complet de chaque niveau,
/// par exemple `C:`, `Users`, `Bob` pour `C:\Users\Bob`.
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    use std::path::Component;

    let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match component {
            Component::Prefix(prefix) => {
                crumbs.push((
                    prefix.as_os_str().to_string_lossy().into_owned(),
                    current.clone(),
                ));
            }
            Component::RootDir => match crumbs.last_mut() {
                // `C:` devient `C:\` : la racine du lecteur, pas le dossier courant du lecteur.
                Some(last) => last.1 = current.clone(),
                None => crumbs.push((
                    component.as_os_str().to_string_lossy().into_owned(),
                    current.clone(),
                )),
            },
            Component::Normal(name) => {
                crumbs.push((name.to_string_lossy().into_owned(), current.clone()));
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    crumbs
}

/// Interprète le texte saisi dans la barre d'adresse. `None` si le texte est vide.
pub fn parse_address(input: &str) -> Option<Location> {
    let text = input.trim().trim_matches('"').trim();
    if text.is_empty() {
        return None;
    }
    if text.eq_ignore_ascii_case("ce pc") {
        return Some(Location::ThisPc);
    }
    let bytes = text.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        // `C:` désignerait le dossier courant du lecteur : on vise la racine.
        return Some(Location::Dir(PathBuf::from(format!("{text}\\"))));
    }
    Some(Location::Dir(PathBuf::from(text)))
}

/// Liste le contenu d'un dossier : dossiers d'abord, puis fichiers, triés par nom.
/// Les entrées illisibles sont ignorées ; seule l'ouverture du dossier peut échouer.
pub fn list_dir(path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(path)? {
        let Ok(item) = item else { continue };
        let item_path = item.path();
        // `fs::metadata` suit les liens (jonctions comprises) ; repli sur l'entrée elle-même.
        let meta = fs::metadata(&item_path).or_else(|_| item.metadata()).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        let name = item.file_name().to_string_lossy().into_owned();
        // L'attribut « caché » se lit sur l'élément lui-même (pas sur la cible d'un lien).
        let hidden = is_hidden(&name, item.metadata().ok().as_ref());
        entries.push(Entry {
            name,
            path: item_path,
            is_dir,
            size: if is_dir {
                None
            } else {
                Some(meta.as_ref().map_or(0, |m| m.len()))
            },
            hidden,
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            created: meta.as_ref().and_then(|m| m.created().ok()),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Taille totale des fichiers d'un dossier, sous-dossiers compris.
///
/// Les liens symboliques et jonctions ne sont pas suivis (pas de boucle, pas de
/// double comptage) et les éléments illisibles sont ignorés. `is_cancelled` est
/// consulté régulièrement ; s'il renvoie `true`, le calcul s'arrête et rend `None`.
pub fn dir_size(path: &Path, is_cancelled: &dyn Fn() -> bool) -> Option<u64> {
    let mut total: u64 = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            if is_cancelled() {
                return None;
            }
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(item.path());
            } else if let Ok(meta) = item.metadata() {
                total += meta.len();
            }
        }
    }
    Some(total)
}

/// Taille lisible en unités françaises (o, Ko, Mo, Go, To).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    if bytes < 1024 {
        return format!("{bytes} o");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
}

/// Comme `breadcrumbs`, mais un chemin situé dans le dossier de l'utilisateur (`home`) commence
/// par « Home » au lieu de `C:` > `Users` > nom : `Home`, `Videos` pour `C:\Users\Bob\Videos`.
pub fn breadcrumbs_with_home(path: &Path, home: Option<&Path>) -> Vec<(String, PathBuf)> {
    let Some(home) = home else {
        return breadcrumbs(path);
    };
    let Ok(relative) = path.strip_prefix(home) else {
        return breadcrumbs(path);
    };
    let mut crumbs = vec![("Home".to_string(), home.to_path_buf())];
    let mut current = home.to_path_buf();
    for component in relative.components() {
        if let std::path::Component::Normal(name) = component {
            current.push(name);
            crumbs.push((name.to_string_lossy().into_owned(), current.clone()));
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn levels_below_lists_the_folders_between_root_and_file() {
        let root = Path::new("C:\\Data");
        let deep = Path::new("C:\\Data\\a\\b\\f.txt");
        assert_eq!(
            levels_below(root, deep),
            [
                PathBuf::from("C:\\Data\\a\\b"),
                PathBuf::from("C:\\Data\\a")
            ]
        );
        assert!(levels_below(root, Path::new("C:\\Data\\f.txt")).is_empty());
        assert!(levels_below(root, Path::new("D:\\x\\f.txt")).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn breadcrumbs_inside_home_start_with_home() {
        let home = Path::new("C:\\Users\\Bob");
        let deep = Path::new("C:\\Users\\Bob\\Videos\\2026");
        assert_eq!(
            breadcrumbs_with_home(deep, Some(home)),
            [
                ("Home".to_string(), PathBuf::from("C:\\Users\\Bob")),
                (
                    "Videos".to_string(),
                    PathBuf::from("C:\\Users\\Bob\\Videos")
                ),
                (
                    "2026".to_string(),
                    PathBuf::from("C:\\Users\\Bob\\Videos\\2026")
                ),
            ]
        );
        // Le dossier de l'utilisateur lui-même : « Home » seul.
        assert_eq!(breadcrumbs_with_home(home, Some(home)).len(), 1);
        // Hors du dossier de l'utilisateur (ou sans dossier connu) : chemin complet.
        let other = Path::new("D:\\Data");
        assert_eq!(breadcrumbs_with_home(other, Some(home)), breadcrumbs(other));
        assert_eq!(breadcrumbs_with_home(other, None), breadcrumbs(other));
        // `C:\Users\Bobby` n'est pas dans `C:\Users\Bob`.
        let bobby = Path::new("C:\\Users\\Bobby");
        assert_eq!(breadcrumbs_with_home(bobby, Some(home)), breadcrumbs(bobby));
    }

    #[cfg(windows)]
    #[test]
    fn breadcrumbs_of_a_windows_path() {
        let crumbs = breadcrumbs(Path::new("C:\\Users\\Bob"));
        assert_eq!(
            crumbs,
            [
                ("C:".to_string(), PathBuf::from("C:\\")),
                ("Users".to_string(), PathBuf::from("C:\\Users")),
                ("Bob".to_string(), PathBuf::from("C:\\Users\\Bob")),
            ]
        );
        let root = breadcrumbs(Path::new("D:\\"));
        assert_eq!(root, [("D:".to_string(), PathBuf::from("D:\\"))]);
    }

    #[test]
    fn parse_empty_is_none() {
        assert_eq!(parse_address("   "), None);
    }

    #[test]
    fn parse_this_pc() {
        assert_eq!(parse_address(" Ce PC "), Some(Location::ThisPc));
    }

    #[test]
    fn parse_bare_drive_targets_root() {
        assert_eq!(
            parse_address("d:"),
            Some(Location::Dir(PathBuf::from("d:\\")))
        );
    }

    #[test]
    fn parse_strips_quotes() {
        assert_eq!(
            parse_address("\"C:\\Users\""),
            Some(Location::Dir(PathBuf::from("C:\\Users")))
        );
    }

    #[cfg(windows)]
    #[test]
    fn parent_of_drive_root_is_this_pc() {
        let root = Location::Dir(PathBuf::from("C:\\"));
        assert_eq!(root.parent(), Some(Location::ThisPc));
        let sub = Location::Dir(PathBuf::from("C:\\Users"));
        assert_eq!(sub.parent(), Some(Location::Dir(PathBuf::from("C:\\"))));
        assert_eq!(Location::ThisPc.parent(), None);
    }

    #[test]
    fn names_starting_with_a_dot_are_hidden() {
        assert!(is_hidden(".git", None));
        assert!(is_hidden(".env", None));
        assert!(!is_hidden("src", None));
        assert!(!is_hidden("a.rs", None));
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let entries = list_dir(root).expect("le dossier du projet est lisible");
        assert!(entries.iter().any(|e| e.name == ".cargo" && e.hidden));
        assert!(entries.iter().any(|e| e.name == "src" && !e.hidden));
    }

    #[test]
    fn list_dir_puts_folders_first_and_sorts() {
        // Liste le dépôt lui-même, en lecture seule.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let entries = list_dir(root).expect("le dossier du projet est lisible");
        assert!(entries.iter().any(|e| e.name == "Cargo.toml" && !e.is_dir));
        assert!(entries.iter().any(|e| e.name == "src" && e.is_dir));
        let first_file = entries.iter().position(|e| !e.is_dir).unwrap();
        assert!(entries[..first_file].iter().all(|e| e.is_dir));
        assert!(entries[first_file..].iter().all(|e| !e.is_dir));
    }

    #[test]
    fn list_missing_dir_is_error_not_panic() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("dossier-inexistant");
        assert!(list_dir(&missing).is_err());
    }

    #[test]
    fn dir_size_sums_files_recursively() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let expected: u64 = list_dir(&src).unwrap().iter().filter_map(|e| e.size).sum();
        let size = dir_size(&src, &|| false).unwrap();
        assert!(size >= expected && size > 0);
    }

    #[test]
    fn dir_size_can_be_cancelled() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(dir_size(root, &|| true), None);
    }

    #[test]
    fn dir_size_of_missing_dir_is_zero_not_panic() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("dossier-inexistant");
        assert_eq!(dir_size(&missing, &|| false), Some(0));
    }

    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0 o");
        assert_eq!(format_size(1023), "1023 o");
        assert_eq!(format_size(1536), "1,5 Ko");
        assert_eq!(format_size(5 * 1024 * 1024), "5,0 Mo");
    }
}
