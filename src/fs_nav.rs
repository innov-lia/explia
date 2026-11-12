//! Navigation dans le système de fichiers (lecture seule).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
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
        entries.push(Entry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item_path,
            is_dir,
            size: meta.as_ref().filter(|m| m.is_file()).map_or(0, |m| m.len()),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn size_formatting() {
        assert_eq!(format_size(0), "0 o");
        assert_eq!(format_size(1023), "1023 o");
        assert_eq!(format_size(1536), "1,5 Ko");
        assert_eq!(format_size(5 * 1024 * 1024), "5,0 Mo");
    }
}
