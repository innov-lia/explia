//! Emplacements système : lecteurs logiques et dossiers usuels (lecture seule).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// Racine du lecteur, par exemple `C:\`.
    pub path: PathBuf,
    pub kind: DriveKind,
}

impl Drive {
    /// Libellé court, par exemple `C:`.
    pub fn label(&self) -> String {
        self.path
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_string()
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            DriveKind::Fixed => "Disque local",
            DriveKind::Removable => "Disque amovible",
            DriveKind::Network => "Lecteur réseau",
            DriveKind::Optical => "Lecteur optique",
            DriveKind::Other => "Lecteur",
        }
    }
}

/// Dossier d'accès rapide (Bureau, Documents, Téléchargements).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: &'static str,
    pub path: PathBuf,
}

/// Convertit le masque renvoyé par `GetLogicalDrives` en lettres de lecteur.
pub fn letters_from_mask(mask: u32) -> Vec<char> {
    (0..26u8)
        .filter(|i| mask & (1u32 << i) != 0)
        .map(|i| char::from(b'A' + i))
        .collect()
}

/// Dossiers d'accès rapide qui existent réellement sur la machine.
pub fn quick_places() -> Vec<Place> {
    imp::quick_places()
        .into_iter()
        .filter(|p| p.path.is_dir())
        .collect()
}

/// Lecteurs logiques détectés.
pub fn detect_drives() -> Vec<Drive> {
    imp::detect_drives()
}

#[cfg(windows)]
mod imp {
    use super::{Drive, DriveKind, Place, letters_from_mask};
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, SHGetKnownFolderPath,
    };
    use windows_sys::core::GUID;

    // Valeurs de retour documentées de `GetDriveTypeW`.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;

    fn kind_from_type(drive_type: u32) -> DriveKind {
        match drive_type {
            DRIVE_FIXED => DriveKind::Fixed,
            DRIVE_REMOVABLE => DriveKind::Removable,
            DRIVE_REMOTE => DriveKind::Network,
            DRIVE_CDROM => DriveKind::Optical,
            _ => DriveKind::Other,
        }
    }

    pub fn detect_drives() -> Vec<Drive> {
        // SAFETY: `GetLogicalDrives` n'a aucun paramètre et ne touche pas à la mémoire.
        let mask = unsafe { GetLogicalDrives() };
        letters_from_mask(mask)
            .into_iter()
            .map(|letter| {
                let root = format!("{letter}:\\");
                let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
                // SAFETY: `wide` est une chaîne UTF-16 terminée par un zéro, valide pendant l'appel.
                let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
                Drive {
                    path: PathBuf::from(root),
                    kind: kind_from_type(drive_type),
                }
            })
            .collect()
    }

    fn known_folder(id: &GUID) -> Option<PathBuf> {
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: `id` est valide, `raw` reçoit une chaîne allouée par le shell
        // que l'on libère avec `CoTaskMemFree` après copie.
        let hr = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw) };
        let path = if hr >= 0 && !raw.is_null() {
            let mut len = 0usize;
            // SAFETY: la chaîne renvoyée est terminée par un zéro.
            unsafe {
                while *raw.add(len) != 0 {
                    len += 1;
                }
                Some(PathBuf::from(OsString::from_wide(
                    std::slice::from_raw_parts(raw, len),
                )))
            }
        } else {
            None
        };
        // SAFETY: `CoTaskMemFree` accepte un pointeur nul ou alloué par le shell.
        unsafe { CoTaskMemFree(raw.cast()) };
        path
    }

    pub fn quick_places() -> Vec<Place> {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let fallback = |name: &str| home.as_ref().map(|h| h.join(name));
        [
            (
                "Bureau",
                known_folder(&FOLDERID_Desktop).or_else(|| fallback("Desktop")),
            ),
            (
                "Documents",
                known_folder(&FOLDERID_Documents).or_else(|| fallback("Documents")),
            ),
            (
                "Téléchargements",
                known_folder(&FOLDERID_Downloads).or_else(|| fallback("Downloads")),
            ),
        ]
        .into_iter()
        .filter_map(|(name, path)| path.map(|path| Place { name, path }))
        .collect()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Drive, DriveKind, Place};
    use std::path::PathBuf;

    pub fn detect_drives() -> Vec<Drive> {
        vec![Drive {
            path: PathBuf::from("/"),
            kind: DriveKind::Fixed,
        }]
    }

    pub fn quick_places() -> Vec<Place> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        [
            ("Bureau", "Desktop"),
            ("Documents", "Documents"),
            ("Téléchargements", "Downloads"),
        ]
        .into_iter()
        .map(|(name, dir)| Place {
            name,
            path: home.join(dir),
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_to_letters() {
        assert_eq!(letters_from_mask(0), Vec::<char>::new());
        assert_eq!(letters_from_mask(0b101), vec!['A', 'C']);
        assert_eq!(letters_from_mask(1 << 25), vec!['Z']);
    }

    #[test]
    fn drive_label_strips_backslash() {
        let drive = Drive {
            path: PathBuf::from("D:\\"),
            kind: DriveKind::Removable,
        };
        assert_eq!(drive.label(), "D:");
    }

    #[cfg(windows)]
    #[test]
    fn detects_at_least_one_drive() {
        assert!(!detect_drives().is_empty());
    }
}
