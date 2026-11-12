//! Presse-papiers de Windows : des fichiers ou dossiers « copiés » dans ExPLIA se collent
//! ensuite dans l'Explorateur, sur le Bureau ou dans une autre application (Ctrl+V).
//!
//! ExPLIA n'écrit aucun fichier : il dépose seulement la liste des chemins dans le
//! presse-papiers (format `CF_HDROP`, comme l'Explorateur) ; c'est l'application où l'on colle
//! qui fait la copie. Le code `unsafe` de l'API Windows reste dans ce module.

use std::path::PathBuf;

/// Taille de l'en-tête `DROPFILES` : `pFiles` (u32), `pt` (2 × i32), `fNC` et `fWide` (BOOL).
const DROPFILES_SIZE: usize = 20;

/// Contenu `CF_HDROP` : l'en-tête `DROPFILES` (chemins en UTF-16), puis chaque chemin terminé
/// par un zéro, et un zéro final.
pub fn hdrop_payload(paths: &[PathBuf]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(DROPFILES_SIZE + paths.len() * 64);
    bytes.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
    bytes.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    bytes.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    bytes.extend_from_slice(&0i32.to_le_bytes()); // fNC
    bytes.extend_from_slice(&1i32.to_le_bytes()); // fWide : chemins en UTF-16
    for path in paths {
        for unit in path.as_os_str().to_string_lossy().encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Place des fichiers et dossiers dans le presse-papiers de Windows, prêts à être collés.
pub fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Aucun élément à copier.".to_string());
    }
    imp::set_clipboard(&hdrop_payload(paths))
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;

    /// Format standard d'une liste de fichiers (`CF_HDROP`).
    const CF_HDROP: u32 = 15;
    /// `DROPEFFECT_COPY` : le collage copie (ne déplace pas).
    const DROPEFFECT_COPY: u32 = 1;

    /// Copie `bytes` dans un bloc de mémoire globale remis au presse-papiers sous `format`.
    ///
    /// # Safety
    /// Le presse-papiers doit être ouvert par ce thread.
    unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
        // SAFETY (appelant) : presse-papiers ouvert. Le bloc a la taille de `bytes` ; il est
        // verrouillé le temps de la copie, puis confié au système, ou libéré en cas d'échec.
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if memory.is_null() {
                return Err("Mémoire insuffisante pour le presse-papiers.".to_string());
            }
            let target = GlobalLock(memory);
            if target.is_null() {
                GlobalFree(memory);
                return Err("Presse-papiers indisponible.".to_string());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
            GlobalUnlock(memory);
            if SetClipboardData(format, memory).is_null() {
                GlobalFree(memory);
                return Err("Impossible d'écrire dans le presse-papiers.".to_string());
            }
        }
        Ok(())
    }

    pub fn set_clipboard(hdrop: &[u8]) -> Result<(), String> {
        let effect_name: Vec<u16> = "Preferred DropEffect"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY : le presse-papiers est ouvert pour la fenêtre active d'ExPLIA (une fenêtre est
        // nécessaire : sans propriétaire, Windows refuse `SetClipboardData`), puis toujours
        // refermé ; `effect_name` est terminé par un zéro.
        unsafe {
            if OpenClipboard(GetActiveWindow()) == 0 {
                return Err("Presse-papiers occupé par une autre application.".to_string());
            }
            let result = if EmptyClipboard() == 0 {
                Err("Impossible de vider le presse-papiers.".to_string())
            } else {
                put(CF_HDROP, hdrop).and_then(|()| {
                    let format = RegisterClipboardFormatW(effect_name.as_ptr());
                    if format == 0 {
                        Ok(())
                    } else {
                        put(format, &DROPEFFECT_COPY.to_le_bytes())
                    }
                })
            };
            CloseClipboard();
            result
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn set_clipboard(_hdrop: &[u8]) -> Result<(), String> {
        Err("Copie de fichiers disponible uniquement sous Windows.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect()
    }

    #[test]
    fn payload_has_the_dropfiles_header_and_a_double_null_list() {
        let bytes = hdrop_payload(&[PathBuf::from("C:\\a.txt"), PathBuf::from("D:\\é")]);
        assert_eq!(&bytes[0..4], &20u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &1i32.to_le_bytes(), "fWide");
        let expected: Vec<u16> = "C:\\a.txt\0D:\\é\0\0".encode_utf16().collect();
        assert_eq!(units(&bytes[DROPFILES_SIZE..]), expected);
    }

    #[test]
    fn nothing_to_copy_is_an_error() {
        assert!(copy_files(&[]).is_err());
    }
}
