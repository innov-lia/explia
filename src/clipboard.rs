//! Presse-papiers de Windows : des fichiers ou dossiers « copiés » dans ExPLIA se collent
//! ensuite dans l'Explorateur, sur le Bureau ou dans une autre application (Ctrl+V).
//!
//! ExPLIA n'écrit aucun fichier : il dépose seulement la liste des chemins dans le
//! presse-papiers (format `CF_HDROP`, comme l'Explorateur), avec l'effet voulu (copier ou
//! déplacer, format « Preferred DropEffect ») ; c'est l'application où l'on colle qui écrit.
//! Le code `unsafe` de l'API Windows reste dans ce module ; la logique Copier / Couper est
//! dans `clipboard_manager`.

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

/// Fichiers et dossiers présents dans le presse-papiers (copiés dans l'Explorateur ou ExPLIA).
pub fn clipboard_files() -> Result<Vec<PathBuf>, String> {
    imp::read_files()
}

/// Le presse-papiers contient des fichiers à coller.
pub fn has_files() -> bool {
    imp::has_files()
}

/// Ctrl+V est enfoncé. egui ne signale pas ce raccourci quand le presse-papiers ne contient
/// que des fichiers (pas de texte) : on lit donc l'état du clavier directement.
pub fn paste_keys_down() -> bool {
    imp::paste_keys_down()
}

/// `DROPEFFECT_COPY` : le collage copie.
pub const DROPEFFECT_COPY: u32 = 1;
/// `DROPEFFECT_MOVE` : le collage déplace (éléments coupés).
pub const DROPEFFECT_MOVE: u32 = 2;

/// Place des fichiers et dossiers dans le presse-papiers de Windows, prêts à être collés ;
/// `effect` : `DROPEFFECT_COPY` ou `DROPEFFECT_MOVE`.
pub fn put_files(paths: &[PathBuf], effect: u32) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Aucun élément sélectionné.".to_string());
    }
    imp::set_clipboard(&hdrop_payload(paths), effect)
}

/// Effet demandé par l'application qui a rempli le presse-papiers (`None` : non précisé,
/// c'est-à-dire copier).
pub fn drop_effect() -> Option<u32> {
    imp::read_effect()
}

/// Vide le presse-papiers (après un déplacement : les éléments coupés n'y sont plus).
pub fn clear() -> Result<(), String> {
    imp::clear()
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetActiveWindow, GetAsyncKeyState, VK_CONTROL, VK_V,
    };
    use windows_sys::Win32::UI::Shell::DragQueryFileW;

    /// Format standard d'une liste de fichiers (`CF_HDROP`).
    const CF_HDROP: u32 = 15;

    /// Format « Preferred DropEffect » (enregistré par l'Explorateur) ; 0 si indisponible.
    fn effect_format() -> u32 {
        let name: Vec<u16> = "Preferred DropEffect"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY : `name` est terminée par un zéro et vit pendant l'appel.
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    }

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

    pub fn has_files() -> bool {
        // SAFETY : simple question au système, sans pointeur.
        unsafe { IsClipboardFormatAvailable(CF_HDROP) != 0 }
    }

    pub fn paste_keys_down() -> bool {
        // SAFETY : lecture de l'état de deux touches, sans pointeur.
        unsafe {
            GetAsyncKeyState(i32::from(VK_CONTROL)) < 0 && GetAsyncKeyState(i32::from(VK_V)) < 0
        }
    }

    pub fn read_files() -> Result<Vec<PathBuf>, String> {
        if !has_files() {
            return Ok(Vec::new());
        }
        // SAFETY : presse-papiers ouvert puis toujours refermé ; la poignée `CF_HDROP` reste
        // au système (on ne la libère pas) ; chaque tampon a la taille annoncée à l'API.
        unsafe {
            if OpenClipboard(GetActiveWindow()) == 0 {
                return Err("Presse-papiers occupé par une autre application.".to_string());
            }
            let handle = GetClipboardData(CF_HDROP);
            let mut files = Vec::new();
            if !handle.is_null() {
                let count = DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0);
                for index in 0..count {
                    let len = DragQueryFileW(handle, index, std::ptr::null_mut(), 0);
                    let mut buffer = vec![0u16; len as usize + 1];
                    let written = DragQueryFileW(handle, index, buffer.as_mut_ptr(), len + 1);
                    buffer.truncate(written as usize);
                    files.push(PathBuf::from(String::from_utf16_lossy(&buffer)));
                }
            }
            CloseClipboard();
            Ok(files)
        }
    }

    pub fn read_effect() -> Option<u32> {
        let format = effect_format();
        // SAFETY : presse-papiers ouvert puis toujours refermé ; la poignée reste au système ;
        // le bloc verrouillé fait au moins 4 octets avant d'être lu, puis est déverrouillé.
        unsafe {
            if format == 0 || IsClipboardFormatAvailable(format) == 0 {
                return None;
            }
            if OpenClipboard(GetActiveWindow()) == 0 {
                return None;
            }
            let handle = GetClipboardData(format);
            let mut effect = None;
            if !handle.is_null() && GlobalSize(handle) >= 4 {
                let data = GlobalLock(handle);
                if !data.is_null() {
                    effect = Some(u32::from_le_bytes(*data.cast::<[u8; 4]>()));
                    GlobalUnlock(handle);
                }
            }
            CloseClipboard();
            effect
        }
    }

    pub fn clear() -> Result<(), String> {
        // SAFETY : presse-papiers ouvert pour la fenêtre active, puis toujours refermé.
        unsafe {
            if OpenClipboard(GetActiveWindow()) == 0 {
                return Err("Presse-papiers occupé par une autre application.".to_string());
            }
            let emptied = EmptyClipboard() != 0;
            CloseClipboard();
            if emptied {
                Ok(())
            } else {
                Err("Impossible de vider le presse-papiers.".to_string())
            }
        }
    }

    pub fn set_clipboard(hdrop: &[u8], effect: u32) -> Result<(), String> {
        // SAFETY : le presse-papiers est ouvert pour la fenêtre active d'ExPLIA (une fenêtre est
        // nécessaire : sans propriétaire, Windows refuse `SetClipboardData`), puis toujours
        // refermé.
        unsafe {
            if OpenClipboard(GetActiveWindow()) == 0 {
                return Err("Presse-papiers occupé par une autre application.".to_string());
            }
            let result = if EmptyClipboard() == 0 {
                Err("Impossible de vider le presse-papiers.".to_string())
            } else {
                put(CF_HDROP, hdrop).and_then(|()| {
                    let format = effect_format();
                    if format == 0 {
                        Ok(())
                    } else {
                        put(format, &effect.to_le_bytes())
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
    use std::path::PathBuf;

    pub fn set_clipboard(_hdrop: &[u8], _effect: u32) -> Result<(), String> {
        Err("Copie de fichiers disponible uniquement sous Windows.".to_string())
    }

    pub fn read_effect() -> Option<u32> {
        None
    }

    pub fn clear() -> Result<(), String> {
        Ok(())
    }

    pub fn has_files() -> bool {
        false
    }

    pub fn paste_keys_down() -> bool {
        false
    }

    pub fn read_files() -> Result<Vec<PathBuf>, String> {
        Ok(Vec::new())
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
        assert!(put_files(&[], DROPEFFECT_COPY).is_err());
    }
}
