//! Renommage d'un fichier ou d'un dossier, sur demande explicite de l'utilisateur.
//!
//! Le renommage reste dans le même dossier : seul le dernier élément du chemin change.

use std::fs;
use std::path::{Path, PathBuf};

/// Caractères interdits dans un nom de fichier Windows.
const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
/// Noms réservés par Windows (avec ou sans extension).
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Vérifie qu'un nom est valide pour Windows.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Le nom ne peut pas être vide.".to_string());
    }
    if name == "." || name == ".." {
        return Err("Ce nom n'est pas valide.".to_string());
    }
    if name.chars().count() > 255 {
        return Err("Le nom est trop long (255 caractères au plus).".to_string());
    }
    if let Some(c) = name
        .chars()
        .find(|c| FORBIDDEN_CHARS.contains(c) || c.is_control())
    {
        let shown = if c.is_control() {
            "caractère de contrôle".to_string()
        } else {
            format!("« {c} »")
        };
        return Err(format!("Le nom ne peut pas contenir {shown}."));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err("Le nom ne peut pas se terminer par un espace ou un point.".to_string());
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(format!("« {stem} » est un nom réservé par Windows."));
    }
    Ok(())
}

/// Renomme `path` en `new_name` dans le même dossier. Renvoie le nouveau chemin.
///
/// Refuse d'écraser un élément existant (sauf si seul l'usage des majuscules change).
pub fn rename_in_place(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Impossible de renommer la racine d'un lecteur.".to_string())?;
    let target = parent.join(new_name);
    let old_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if old_name == new_name {
        return Ok(path.to_path_buf());
    }
    // Windows ignore la casse : « a.txt » -> « A.txt » désigne le même élément.
    let only_case_change = old_name.to_lowercase() == new_name.to_lowercase();
    if target.exists() && !only_case_change {
        return Err(format!("« {new_name} » existe déjà dans ce dossier."));
    }
    fs::rename(path, &target).map_err(|e| format!("Renommage impossible : {e}"))?;
    Ok(target)
}

/// Nom proposé pour un nouveau dossier.
pub const NEW_FOLDER_NAME: &str = "Nouveau dossier";

/// Crée un dossier vide dans `parent`, sous un nom libre : « Nouveau dossier », puis
/// « Nouveau dossier (2) »… Rien d'existant n'est jamais réutilisé. Renvoie son chemin.
pub fn create_folder(parent: &Path) -> Result<PathBuf, String> {
    if !parent.is_dir() {
        return Err(format!("Dossier introuvable : {}", parent.display()));
    }
    for n in 1..1000u32 {
        let name = if n == 1 {
            NEW_FOLDER_NAME.to_string()
        } else {
            format!("{NEW_FOLDER_NAME} ({n})")
        };
        let path = parent.join(name);
        // `create_dir` échoue si le nom est pris : jamais de fusion avec un dossier existant.
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("Création du dossier impossible : {err}")),
        }
    }
    Err("Trop de dossiers « Nouveau dossier » ici.".to_string())
}

/// Sélection initiale proposée à l'édition : le nom sans l'extension (pour un fichier).
pub fn stem_len(name: &str, is_dir: bool) -> usize {
    if is_dir {
        return name.chars().count();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-rename")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_normal_names() {
        for name in [
            "a.txt",
            "Mon dossier",
            "rapport (v2).docx",
            ".gitignore",
            "é à ü",
        ] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_invalid_names() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a:b",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "fin.",
            "fin ",
            "tab\tname",
            "CON",
            "con.txt",
            "Lpt1",
            "NUL.tar.gz",
        ] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert!(validate_name(&"x".repeat(256)).is_err());
        assert!(validate_name(&"x".repeat(255)).is_ok());
        // Un nom qui commence comme un nom réservé reste valide.
        assert!(validate_name("console.txt").is_ok());
    }

    #[test]
    fn renames_a_file_and_a_folder() {
        let dir = test_dir("basic");
        let file = dir.join("a.txt");
        fs::write(&file, b"x").unwrap();
        let renamed = rename_in_place(&file, "b.txt").unwrap();
        assert_eq!(renamed, dir.join("b.txt"));
        assert!(!file.exists() && renamed.exists());

        let folder = dir.join("dossier");
        fs::create_dir(&folder).unwrap();
        let renamed = rename_in_place(&folder, "autre").unwrap();
        assert!(renamed.is_dir() && !folder.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_overwrite_an_existing_item() {
        let dir = test_dir("exists");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, b"A").unwrap();
        fs::write(&b, b"B").unwrap();
        assert!(rename_in_place(&a, "b.txt").is_err());
        assert_eq!(fs::read(&b).unwrap(), b"B");
        assert!(a.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_name_is_a_no_op_and_case_change_is_allowed() {
        let dir = test_dir("case");
        let a = dir.join("a.txt");
        fs::write(&a, b"x").unwrap();
        assert_eq!(rename_in_place(&a, "a.txt").unwrap(), a);
        let upper = rename_in_place(&a, "A.txt").unwrap();
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.txt"]);
        assert_eq!(upper, dir.join("A.txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_source_is_an_error_not_a_panic() {
        let dir = test_dir("missing");
        assert!(rename_in_place(&dir.join("absent.txt"), "x.txt").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_folders_take_a_free_name() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-fileops")
            .join("new-folder");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let first = create_folder(&dir).unwrap();
        let second = create_folder(&dir).unwrap();
        assert_eq!(first, dir.join("Nouveau dossier"));
        assert_eq!(second, dir.join("Nouveau dossier (2)"));
        assert!(second.is_dir());
        assert!(create_folder(&dir.join("absent")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stem_length_excludes_the_extension_of_files() {
        assert_eq!(stem_len("rapport.docx", false), 7);
        assert_eq!(stem_len("archive.tar.gz", false), 11);
        assert_eq!(stem_len(".gitignore", false), 10);
        assert_eq!(stem_len("sans-extension", false), 14);
        assert_eq!(stem_len("dossier.v2", true), 10);
    }
}
