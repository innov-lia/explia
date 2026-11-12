//! Suppression de fichiers et de dossiers : toujours vers la corbeille de Windows.
//!
//! ExPLIA ne supprime jamais définitivement : `FOF_ALLOWUNDO` envoie les éléments à la
//! corbeille. Si Windows ne peut pas utiliser la corbeille (lecteur réseau ou amovible),
//! il affiche lui-même sa propre demande de confirmation avant toute suppression définitive.

use std::path::{Path, PathBuf};

/// Vérifie que la liste est une demande de suppression raisonnable.
fn validate(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Aucun élément à supprimer.".to_string());
    }
    for path in paths {
        if !path.is_absolute() {
            return Err(format!("Chemin non absolu : {}", path.display()));
        }
        // Une racine de lecteur (`C:\`) n'a pas de parent : on ne la supprime jamais.
        if path.parent().is_none() {
            return Err(format!("Suppression refusée : {}", path.display()));
        }
        if path.to_string_lossy().contains('\0') {
            return Err("Chemin invalide.".to_string());
        }
    }
    Ok(())
}

/// Envoie les éléments à la corbeille.
pub fn move_to_recycle_bin(paths: &[PathBuf]) -> Result<(), String> {
    validate(paths)?;
    imp::move_to_recycle_bin(paths)
}

/// Vrai si `path` existe encore (utilisé pour contrôler le résultat).
#[allow(dead_code)]
pub fn exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, SHFILEOPSTRUCTW, SHFileOperationW,
    };

    /// Liste de chemins UTF-16 séparés par un zéro, terminée par deux zéros.
    pub(super) fn double_null_list(paths: &[PathBuf]) -> Vec<u16> {
        let mut list = Vec::new();
        for path in paths {
            list.extend(path.as_os_str().encode_wide());
            list.push(0);
        }
        list.push(0);
        list
    }

    pub fn move_to_recycle_bin(paths: &[PathBuf]) -> Result<(), String> {
        let from = double_null_list(paths);
        let mut operation = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            // Corbeille uniquement. Pas de `FOF_NOCONFIRMATION` : si la corbeille est
            // inutilisable, Windows redemande avant de supprimer définitivement.
            fFlags: FOF_ALLOWUNDO as _,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        // SAFETY: `from` est une liste double-zéro valide qui vit pendant l'appel ;
        // `operation` est entièrement initialisée et exclusive.
        let code = unsafe { SHFileOperationW(&mut operation) };
        if code != 0 {
            return Err(format!("Suppression impossible (code {code:#x})."));
        }
        if operation.fAnyOperationsAborted != 0 {
            return Err("Suppression annulée.".to_string());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    pub fn move_to_recycle_bin(_paths: &[PathBuf]) -> Result<(), String> {
        Err("Suppression disponible uniquement sous Windows.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_relative_and_root_paths() {
        assert!(validate(&[]).is_err());
        assert!(validate(&[PathBuf::from("relatif.txt")]).is_err());
        #[cfg(windows)]
        {
            assert!(validate(&[PathBuf::from("C:\\")]).is_err());
            assert!(validate(&[PathBuf::from("C:\\Users\\a.txt")]).is_ok());
        }
    }

    #[cfg(windows)]
    #[test]
    fn builds_a_double_null_terminated_list() {
        let list = imp::double_null_list(&[PathBuf::from("C:\\a"), PathBuf::from("C:\\b")]);
        let expected: Vec<u16> = "C:\\a\0C:\\b\0\0".encode_utf16().collect();
        assert_eq!(list, expected);
    }

    /// Envoie réellement un fichier temporaire à la corbeille.
    /// Ignoré par défaut : `cargo test -- --ignored recycle`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn recycle_real_temporary_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-trash");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a-supprimer.txt");
        let folder = dir.join("dossier-a-supprimer");
        std::fs::write(&file, b"x").unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        move_to_recycle_bin(std::slice::from_ref(&file)).unwrap();
        move_to_recycle_bin(std::slice::from_ref(&folder)).unwrap();
        assert!(!exists(&file));
        assert!(!exists(&folder));
    }
}
