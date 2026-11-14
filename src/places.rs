//! Lecteurs logiques de la machine (lecture seule).

use std::path::{Path, PathBuf};

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

/// Convertit le masque renvoyé par `GetLogicalDrives` en lettres de lecteur.
pub fn letters_from_mask(mask: u32) -> Vec<char> {
    (0..26u8)
        .filter(|i| mask & (1u32 << i) != 0)
        .map(|i| char::from(b'A' + i))
        .collect()
}

/// Dossier de l'utilisateur courant (`C:\Users\nom` sous Windows).
pub fn user_home() -> Option<PathBuf> {
    user_home_from(|name| std::env::var_os(name))
}

/// Cherche le dossier utilisateur avec la fonction de lecture d'environnement donnée.
fn user_home_from(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(get)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
}

/// Lecteurs logiques détectés.
pub fn detect_drives() -> Vec<Drive> {
    imp::detect_drives()
}

/// Capacité, espace libre, nom et système de fichiers d'un lecteur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriveInfo {
    pub total: u64,
    pub free: u64,
    /// Nom du volume (vide s'il n'en a pas).
    pub name: String,
    /// Système de fichiers, par exemple `NTFS` (vide si inconnu).
    pub file_system: String,
}

impl DriveInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Part occupée du lecteur, en pourcentage arrondi (0 à 100).
    pub fn used_percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        let percent =
            (u128::from(self.used()) * 100 + u128::from(self.total) / 2) / u128::from(self.total);
        u8::try_from(percent).map_or(100, |p| p.min(100))
    }
}

/// Informations d'un lecteur. Peut être lent (lecteur réseau) : à appeler hors du thread
/// d'interface. `None` si le lecteur n'est pas prêt.
pub fn drive_info(root: &Path) -> Option<DriveInfo> {
    imp::drive_info(root)
}

/// Évite les boîtes de dialogue système quand un lecteur est vide (CD, lecteur amovible).
pub fn suppress_drive_error_dialogs() {
    imp::suppress_drive_error_dialogs();
}

#[cfg(windows)]
mod imp {
    use super::{Drive, DriveInfo, DriveKind, letters_from_mask};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetErrorMode};

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

    /// Texte d'un tampon UTF-16 terminé par un zéro.
    fn text_of(buffer: &[u16]) -> String {
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..len])
    }

    pub fn drive_info(root: &Path) -> Option<DriveInfo> {
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        // SAFETY: `wide` est terminée par un zéro ; `total` et `free` sont des `u64` valides
        // en écriture ; le pointeur « octets disponibles pour l'appelant » est facultatif.
        let ok = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), std::ptr::null_mut(), &mut total, &mut free)
        };
        if ok == 0 {
            return None;
        }
        let mut name = [0u16; 261];
        let mut file_system = [0u16; 261];
        // SAFETY: `wide` est terminée par un zéro ; les deux tampons ont la taille annoncée ;
        // les autres pointeurs de sortie sont facultatifs et laissés nuls. Un échec (volume
        // sans nom lisible) laisse simplement les textes vides.
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                file_system.as_mut_ptr(),
                file_system.len() as u32,
            )
        };
        let (name, file_system) = if ok != 0 {
            (text_of(&name), text_of(&file_system))
        } else {
            (String::new(), String::new())
        };
        Some(DriveInfo {
            total,
            free,
            name,
            file_system,
        })
    }

    pub fn suppress_drive_error_dialogs() {
        // SAFETY: ne modifie que le mode d'erreur du processus courant.
        unsafe { SetErrorMode(SEM_FAILCRITICALERRORS) };
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Drive, DriveInfo, DriveKind};
    use std::path::{Path, PathBuf};

    pub fn detect_drives() -> Vec<Drive> {
        vec![Drive {
            path: PathBuf::from("/"),
            kind: DriveKind::Fixed,
        }]
    }

    pub fn drive_info(_root: &Path) -> Option<DriveInfo> {
        None
    }

    pub fn suppress_drive_error_dialogs() {}
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
    fn user_home_prefers_userprofile_and_ignores_empty_values() {
        let vars = |name: &str| match name {
            "USERPROFILE" => Some("C:\\Users\\bob".into()),
            "HOME" => Some("/home/bob".into()),
            _ => None,
        };
        assert_eq!(user_home_from(vars), Some(PathBuf::from("C:\\Users\\bob")));
        let only_home = |name: &str| (name == "HOME").then(|| "/home/bob".into());
        assert_eq!(user_home_from(only_home), Some(PathBuf::from("/home/bob")));
        let empty = |_: &str| Some(std::ffi::OsString::new());
        assert_eq!(user_home_from(empty), None);
        assert_eq!(user_home_from(|_| None), None);
    }

    #[test]
    fn drive_label_strips_backslash() {
        let drive = Drive {
            path: PathBuf::from("D:\\"),
            kind: DriveKind::Removable,
        };
        assert_eq!(drive.label(), "D:");
    }

    #[test]
    fn used_percent_is_rounded_and_bounded() {
        let info = |total, free| DriveInfo {
            total,
            free,
            ..DriveInfo::default()
        };
        assert_eq!(info(0, 0).used_percent(), 0);
        assert_eq!(info(1000, 1000).used_percent(), 0);
        assert_eq!(info(1000, 0).used_percent(), 100);
        assert_eq!(info(833, 37).used_percent(), 96);
        assert_eq!(info(3, 1).used_percent(), 67);
        // Espace libre supérieur au total (valeurs incohérentes) : 0 % occupé.
        assert_eq!(info(100, 500).used_percent(), 0);
        assert_eq!(info(1000, 250).used(), 750);
    }

    #[cfg(windows)]
    #[test]
    fn detects_at_least_one_drive_with_a_size() {
        let drives = detect_drives();
        assert!(!drives.is_empty());
        let system_drive = Path::new("C:\\");
        let info = drive_info(system_drive).expect("le lecteur système répond");
        assert!(info.total > 0 && info.free <= info.total);
    }

    #[cfg(windows)]
    #[test]
    fn info_of_missing_drive_is_none() {
        assert_eq!(drive_info(Path::new("Z:\\inexistant-explia\\")), None);
    }
}
