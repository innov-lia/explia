//! Glisser des fichiers et dossiers d'ExPLIA vers une autre application (Bureau, Explorateur,
//! messagerie…), en mode **copie** uniquement : c'est l'application de destination qui copie.
//!
//! Windows fournit l'objet de données (`SHCreateDataObject`, le même que l'Explorateur) ; ExPLIA
//! n'implémente que la « source » du glisser (`IDropSource`), qui suit le bouton de la souris
//! et la touche Échap. Tout le code `unsafe` de cette interface COM reste dans ce module.

use std::path::{Path, PathBuf};

/// Tous les éléments glissés doivent venir du même dossier (contrainte de l'objet de données).
pub fn common_parent(paths: &[PathBuf]) -> Option<&Path> {
    let first = paths.first()?.parent()?;
    paths
        .iter()
        .all(|p| p.parent() == Some(first))
        .then_some(first)
}

/// Lance le glisser-déposer des éléments ; la fonction rend la main quand l'utilisateur lâche
/// le bouton (ou appuie sur Échap). Renvoie `Ok(true)` si les éléments ont été déposés.
pub fn drag_files_out(paths: &[PathBuf]) -> Result<bool, String> {
    if paths.is_empty() {
        return Ok(false);
    }
    if common_parent(paths).is_none() {
        return Err("Glisser : les éléments doivent venir du même dossier.".to_string());
    }
    imp::drag(paths)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows_sys::Win32::System::Ole::{
        DROPEFFECT_COPY, DoDragDrop, OleInitialize, OleUninitialize,
    };
    use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows_sys::Win32::UI::Shell::{
        ILClone, ILFindLastID, ILFree, ILRemoveLastID, SHCreateDataObject, SHParseDisplayName,
    };
    use windows_sys::core::{BOOL, GUID, HRESULT};

    const S_OK: HRESULT = 0;
    const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
    const DRAGDROP_S_DROP: HRESULT = 0x0004_0100;
    const DRAGDROP_S_CANCEL: HRESULT = 0x0004_0101;
    const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x0004_0102;
    /// Bouton gauche de la souris enfoncé (`MK_LBUTTON`).
    const MK_LBUTTON: u32 = 0x0001;

    const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    const IID_IDROPSOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);
    const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);

    /// Table des méthodes de `IDropSource` (dans l'ordre de l'interface COM).
    #[repr(C)]
    struct DropSourceVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        query_continue_drag: unsafe extern "system" fn(*mut c_void, BOOL, u32) -> HRESULT,
        give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    }

    /// Objet `IDropSource` : le premier champ est le pointeur de table, comme l'exige COM.
    #[repr(C)]
    struct DropSource {
        vtbl: *const DropSourceVtbl,
        refs: AtomicU32,
    }

    static DROP_SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
        query_interface,
        add_ref,
        release,
        query_continue_drag,
        give_feedback,
    };

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        riid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        // SAFETY : COM passe un identifiant et une sortie valides.
        unsafe {
            let wanted = &*riid;
            let known = [IID_IUNKNOWN, IID_IDROPSOURCE]
                .iter()
                .any(|iid| guid_eq(iid, wanted));
            if known {
                *out = this;
                add_ref(this);
                S_OK
            } else {
                *out = std::ptr::null_mut();
                E_NOINTERFACE
            }
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        // SAFETY : `this` désigne le `DropSource` vivant pendant tout le glisser.
        unsafe {
            (*this.cast::<DropSource>())
                .refs
                .fetch_add(1, Ordering::SeqCst)
                + 1
        }
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        // SAFETY : idem ; l'objet appartient à `drag`, qui le libère lui-même à la fin.
        unsafe {
            (*this.cast::<DropSource>())
                .refs
                .fetch_sub(1, Ordering::SeqCst)
                .saturating_sub(1)
        }
    }

    unsafe extern "system" fn query_continue_drag(
        _this: *mut c_void,
        escape_pressed: BOOL,
        key_state: u32,
    ) -> HRESULT {
        if escape_pressed != 0 {
            DRAGDROP_S_CANCEL
        } else if key_state & MK_LBUTTON == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    unsafe extern "system" fn give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    /// PIDL absolus, libérés à la fin quoi qu'il arrive.
    struct Pidls(Vec<*mut ITEMIDLIST>);

    impl Drop for Pidls {
        fn drop(&mut self) {
            for pidl in &self.0 {
                // SAFETY : chaque PIDL vient de `SHParseDisplayName` ou `ILClone`.
                unsafe { ILFree(*pidl) };
            }
        }
    }

    pub fn drag(paths: &[PathBuf]) -> Result<bool, String> {
        // SAFETY : appelé sur le thread de la fenêtre ; `OleInitialize` est apparié à
        // `OleUninitialize` quand il réussit (S_OK ou S_FALSE si déjà initialisé).
        let ole = unsafe { OleInitialize(std::ptr::null()) };
        let result = drag_inner(paths);
        if ole >= 0 {
            // SAFETY : appariement de l'initialisation réussie ci-dessus.
            unsafe { OleUninitialize() };
        }
        result
    }

    fn drag_inner(paths: &[PathBuf]) -> Result<bool, String> {
        let mut absolute = Pidls(Vec::new());
        for path in paths {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            // SAFETY : `wide` est terminé par un zéro ; `pidl` reçoit un PIDL à libérer.
            let hr = unsafe {
                SHParseDisplayName(
                    wide.as_ptr(),
                    std::ptr::null_mut(),
                    &mut pidl,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if hr < 0 || pidl.is_null() {
                return Err(format!("Élément introuvable : {}", path.display()));
            }
            absolute.0.push(pidl);
        }
        // Dossier commun (PIDL du parent) et identifiants relatifs de chaque élément.
        // SAFETY : les PIDL sont valides ; le clone est libéré par `Pidls`.
        let folder = unsafe { ILClone(absolute.0[0]) };
        if folder.is_null() {
            return Err("Mémoire insuffisante pour le glisser.".to_string());
        }
        let folder_guard = Pidls(vec![folder]);
        // SAFETY : `folder` est un PIDL modifiable qui nous appartient.
        unsafe { ILRemoveLastID(folder) };
        let children: Vec<*const ITEMIDLIST> = absolute
            .0
            .iter()
            // SAFETY : pointe à l'intérieur du PIDL absolu, qui vit jusqu'à la fin.
            .map(|pidl| unsafe { ILFindLastID(*pidl) }.cast_const())
            .collect();
        let mut data: *mut c_void = std::ptr::null_mut();
        // SAFETY : tableaux valides pendant l'appel ; `data` reçoit un `IDataObject` à libérer.
        let hr = unsafe {
            SHCreateDataObject(
                folder.cast_const(),
                children.len() as u32,
                children.as_ptr(),
                std::ptr::null_mut(),
                &IID_IDATAOBJECT,
                &mut data,
            )
        };
        if hr < 0 || data.is_null() {
            return Err("Glisser impossible (objet de données refusé par Windows).".to_string());
        }
        let mut source = DropSource {
            vtbl: &DROP_SOURCE_VTBL,
            refs: AtomicU32::new(1),
        };
        let mut effect: u32 = 0;
        // SAFETY : `data` est un `IDataObject` valide, `source` vit jusqu'au retour de l'appel
        // (DoDragDrop ne le conserve pas). Copie seulement : jamais de déplacement.
        let hr = unsafe {
            DoDragDrop(
                data,
                (&mut source as *mut DropSource).cast::<c_void>(),
                DROPEFFECT_COPY,
                &mut effect,
            )
        };
        // SAFETY : libère l'`IDataObject` par sa méthode `Release` (3e entrée de la table).
        unsafe {
            let vtbl = *data.cast::<*const [usize; 3]>();
            let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute((*vtbl)[2]);
            release(data);
        }
        drop(folder_guard);
        Ok(hr == DRAGDROP_S_DROP && effect != 0)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    pub fn drag(_paths: &[PathBuf]) -> Result<bool, String> {
        Err("Glisser disponible uniquement sous Windows.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragged_items_must_share_a_folder() {
        let a = PathBuf::from("C:\\d\\a.txt");
        let b = PathBuf::from("C:\\d\\b");
        let c = PathBuf::from("C:\\e\\c.txt");
        assert_eq!(common_parent(&[a.clone(), b]), Some(Path::new("C:\\d")));
        assert_eq!(common_parent(&[a, c]), None);
        assert_eq!(common_parent(&[]), None);
    }
}
