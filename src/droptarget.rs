//! Cible de dépôt Windows (`IDropTarget`) : des fichiers et dossiers glissés depuis
//! l'Explorateur, le Bureau ou ExPLIA lui-même arrivent sur le dossier visé, en copie ou en
//! déplacement, comme dans l'Explorateur.
//!
//! Le module ne connaît que Windows : il lit les fichiers glissés (`CF_HDROP`), la position du
//! pointeur dans la fenêtre et les touches Ctrl / Maj, demande à l'application ce que fera le
//! dépôt (`Handlers::decide`) et lui transmet le survol et les dépôts (`DropEvents`). Un
//! déplacement est fait par ExPLIA lui-même : la source est alors avertie qu'elle n'a rien à
//! supprimer. Tout le code `unsafe` de cette interface COM reste dans ce module.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Ce que fera un dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropKind {
    Copy,
    Move,
}

/// Fichiers qui survolent la fenêtre ou y sont déposés.
#[derive(Debug, Clone, PartialEq)]
pub struct DropRequest {
    pub paths: Vec<PathBuf>,
    /// Position du pointeur dans la fenêtre, en pixels physiques.
    pub x: f32,
    pub y: f32,
    pub ctrl: bool,
    pub shift: bool,
    /// Ce que la source autorise.
    pub can_copy: bool,
    pub can_move: bool,
}

/// Survol en cours et dépôts à traiter, relevés par l'application à chaque image.
#[derive(Debug, Default)]
pub struct DropEvents {
    pub hover: Option<DropRequest>,
    pub dropped: Vec<(DropRequest, DropKind)>,
}

/// Décide d'un dépôt (survol et dépôt) ; `None` : refusé ici.
pub type Decide = Box<dyn Fn(&DropRequest) -> Option<DropKind>>;

/// Ce que l'application fournit à la cible de dépôt.
pub struct Handlers {
    pub decide: Decide,
    pub events: Arc<Mutex<DropEvents>>,
    /// Réveille l'interface (nouvelle image).
    pub wake: Box<dyn Fn()>,
}

/// Enregistre la fenêtre `hwnd` comme cible de dépôt, pour toute la durée du programme.
pub fn register(hwnd: isize, handlers: Handlers) -> Result<(), String> {
    imp::register(hwnd, handlers)
}

/// Le pointeur est-il dans la zone cliente de la fenêtre ? `None` si on ne sait pas le dire.
pub fn cursor_inside(hwnd: isize) -> Option<bool> {
    imp::cursor_inside(hwnd)
}

#[cfg(windows)]
mod imp {
    use super::{DropEvents, DropKind, DropRequest, Handlers};
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{POINT, POINTL, RECT};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::System::Com::{FORMATETC, STGMEDIUM};
    use windows_sys::Win32::System::Ole::{
        DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, OleInitialize, RegisterDragDrop,
        ReleaseStgMedium,
    };
    use windows_sys::Win32::UI::Shell::DragQueryFileW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};
    use windows_sys::core::{GUID, HRESULT};

    const S_OK: HRESULT = 0;
    const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
    const CF_HDROP: u16 = 15;
    const DVASPECT_CONTENT: u32 = 1;
    const TYMED_HGLOBAL: u32 = 1;
    const MK_SHIFT: u32 = 0x0004;
    const MK_CONTROL: u32 = 0x0008;

    const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    const IID_IDROPTARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);

    /// Table des méthodes de `IDropTarget` (dans l'ordre de l'interface COM).
    #[repr(C)]
    struct DropTargetVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        drag_enter:
            unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
        drag_over: unsafe extern "system" fn(*mut c_void, u32, POINTL, *mut u32) -> HRESULT,
        drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    }

    /// Objet `IDropTarget` : le premier champ est le pointeur de table, comme l'exige COM.
    #[repr(C)]
    struct DropTarget {
        vtbl: *const DropTargetVtbl,
        refs: AtomicU32,
        hwnd: isize,
        handlers: Handlers,
        /// Fichiers du glisser en cours (lus à l'entrée dans la fenêtre).
        paths: RefCell<Vec<PathBuf>>,
    }

    static DROP_TARGET_VTBL: DropTargetVtbl = DropTargetVtbl {
        query_interface,
        add_ref,
        release,
        drag_enter,
        drag_over,
        drag_leave,
        drop: on_drop,
    };

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    pub fn register(hwnd: isize, handlers: Handlers) -> Result<(), String> {
        // SAFETY : appelé une fois sur le thread de la fenêtre ; OLE reste initialisé pour la
        // durée du programme (la cible de dépôt en a besoin jusqu'à la fin).
        let ole = unsafe { OleInitialize(std::ptr::null()) };
        if ole < 0 {
            return Err("Glisser-déposer indisponible (OLE).".to_string());
        }
        // La cible vit jusqu'à la fin du programme : Windows garde un pointeur vers elle.
        let target = Box::leak(Box::new(DropTarget {
            vtbl: &DROP_TARGET_VTBL,
            refs: AtomicU32::new(1),
            hwnd,
            handlers,
            paths: RefCell::new(Vec::new()),
        }));
        // SAFETY : `hwnd` est la fenêtre d'ExPLIA, `target` un `IDropTarget` valide pour toujours.
        let hr = unsafe {
            RegisterDragDrop(
                hwnd as *mut c_void,
                (target as *mut DropTarget).cast::<c_void>(),
            )
        };
        if hr < 0 {
            return Err(format!("Glisser-déposer indisponible (code {hr:#x})."));
        }
        Ok(())
    }

    pub fn cursor_inside(hwnd: isize) -> Option<bool> {
        let mut point = POINT { x: 0, y: 0 };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY : structures valides ; `hwnd` est la fenêtre d'ExPLIA.
        unsafe {
            if GetCursorPos(&mut point) == 0
                || ScreenToClient(hwnd as *mut c_void, &mut point) == 0
                || GetClientRect(hwnd as *mut c_void, &mut rect) == 0
            {
                return None;
            }
        }
        Some(
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom,
        )
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        riid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        // SAFETY : COM passe un identifiant et une sortie valides.
        unsafe {
            let wanted = &*riid;
            if guid_eq(&IID_IUNKNOWN, wanted) || guid_eq(&IID_IDROPTARGET, wanted) {
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
        // SAFETY : `this` désigne la cible, qui vit jusqu'à la fin du programme.
        unsafe {
            (*this.cast::<DropTarget>())
                .refs
                .fetch_add(1, Ordering::SeqCst)
                + 1
        }
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        // SAFETY : idem ; la cible n'est jamais libérée (elle a été « fuitée » exprès).
        unsafe {
            (*this.cast::<DropTarget>())
                .refs
                .fetch_sub(1, Ordering::SeqCst)
                .saturating_sub(1)
        }
    }

    /// Chemins `CF_HDROP` d'un `IDataObject` (vide si l'objet n'en contient pas).
    ///
    /// # Safety
    /// `data` est un `IDataObject` valide.
    unsafe fn read_paths(data: *mut c_void) -> Vec<PathBuf> {
        let format = FORMATETC {
            cfFormat: CF_HDROP,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        };
        let mut medium = STGMEDIUM::default();
        // SAFETY (appelant) : `data` est valide ; `GetData` est la 4e entrée de sa table.
        unsafe {
            let vtbl = *data.cast::<*const [usize; 4]>();
            let get_data: unsafe extern "system" fn(
                *mut c_void,
                *const FORMATETC,
                *mut STGMEDIUM,
            ) -> HRESULT = std::mem::transmute((*vtbl)[3]);
            if get_data(data, &format, &mut medium) < 0 {
                return Vec::new();
            }
            let hdrop = medium.u.hGlobal;
            let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let len = DragQueryFileW(hdrop, index, std::ptr::null_mut(), 0);
                let mut buffer = vec![0u16; len as usize + 1];
                let written = DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), len + 1);
                buffer.truncate(written as usize);
                paths.push(PathBuf::from(String::from_utf16_lossy(&buffer)));
            }
            ReleaseStgMedium(&mut medium);
            paths
        }
    }

    impl DropTarget {
        /// Demande à l'application ce que ferait un dépôt ici, et le note comme survol.
        fn request(&self, key_state: u32, point: POINTL, allowed: u32) -> DropRequest {
            let mut client = POINT {
                x: point.x,
                y: point.y,
            };
            // SAFETY : `client` est valide ; `hwnd` est la fenêtre d'ExPLIA.
            unsafe { ScreenToClient(self.hwnd as *mut c_void, &mut client) };
            DropRequest {
                paths: self.paths.borrow().clone(),
                x: client.x as f32,
                y: client.y as f32,
                ctrl: key_state & MK_CONTROL != 0,
                shift: key_state & MK_SHIFT != 0,
                can_copy: allowed & DROPEFFECT_COPY != 0,
                can_move: allowed & DROPEFFECT_MOVE != 0,
            }
        }

        fn events(&self) -> std::sync::MutexGuard<'_, DropEvents> {
            self.handlers
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// Survol : effet proposé au curseur de Windows.
        fn hover(&self, key_state: u32, point: POINTL, effect: *mut u32) {
            // SAFETY : COM passe un pointeur d'effet valide.
            let allowed = unsafe { *effect };
            let request = self.request(key_state, point, allowed);
            let kind = (self.handlers.decide)(&request);
            // SAFETY : idem.
            unsafe {
                *effect = match kind {
                    Some(DropKind::Copy) => DROPEFFECT_COPY,
                    Some(DropKind::Move) => DROPEFFECT_MOVE,
                    None => DROPEFFECT_NONE,
                };
            }
            self.events().hover = Some(request);
            (self.handlers.wake)();
        }
    }

    unsafe extern "system" fn drag_enter(
        this: *mut c_void,
        data: *mut c_void,
        key_state: u32,
        point: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        // SAFETY : `this` est la cible ; `data` un `IDataObject` valide pendant l'appel.
        unsafe {
            let target = &*this.cast::<DropTarget>();
            *target.paths.borrow_mut() = read_paths(data);
            target.hover(key_state, point, effect);
        }
        S_OK
    }

    unsafe extern "system" fn drag_over(
        this: *mut c_void,
        key_state: u32,
        point: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        // SAFETY : `this` est la cible.
        unsafe { (*this.cast::<DropTarget>()).hover(key_state, point, effect) };
        S_OK
    }

    unsafe extern "system" fn drag_leave(this: *mut c_void) -> HRESULT {
        // SAFETY : `this` est la cible.
        let target = unsafe { &*this.cast::<DropTarget>() };
        target.paths.borrow_mut().clear();
        target.events().hover = None;
        (target.handlers.wake)();
        S_OK
    }

    unsafe extern "system" fn on_drop(
        this: *mut c_void,
        data: *mut c_void,
        key_state: u32,
        point: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        // SAFETY : `this` est la cible ; `data` et `effect` sont valides pendant l'appel.
        unsafe {
            let target = &*this.cast::<DropTarget>();
            let paths = read_paths(data);
            if !paths.is_empty() {
                *target.paths.borrow_mut() = paths;
            }
            let request = target.request(key_state, point, *effect);
            let kind = (target.handlers.decide)(&request);
            // Une copie est annoncée telle quelle ; un déplacement est fait par ExPLIA : la
            // source n'a rien à supprimer (déplacement « optimisé » du Shell).
            *effect = match kind {
                Some(DropKind::Copy) => DROPEFFECT_COPY,
                _ => DROPEFFECT_NONE,
            };
            let mut events = target.events();
            events.hover = None;
            if let Some(kind) = kind {
                events.dropped.push((request, kind));
            }
            // Le verrou est libéré avant de réveiller l'interface.
            std::mem::drop(events);
            target.paths.borrow_mut().clear();
            (target.handlers.wake)();
        }
        S_OK
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Handlers;

    pub fn register(_hwnd: isize, _handlers: Handlers) -> Result<(), String> {
        Err("Glisser-déposer disponible uniquement sous Windows.".to_string())
    }

    pub fn cursor_inside(_hwnd: isize) -> Option<bool> {
        None
    }
}
