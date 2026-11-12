//! Lecture audio et vidéo intégrée, avec libVLC.
//!
//! ExPLIA ne contient pas VLC : la bibliothèque `libvlc.dll` du **VLC (64 bits) installé sur la
//! machine** est chargée à la demande, au premier lancement d'une lecture. Si VLC est absent,
//! la lecture intégrée est indisponible et l'appelant ouvre le fichier avec l'application par
//! défaut de Windows (voir `launch`). Aucune dépendance Rust supplémentaire : le chargement
//! passe par l'API Windows, et tout le code `unsafe` (appels FFI) est regroupé ici.
//!
//! Recherche de VLC, dans l'ordre :
//! 1. le dossier indiqué par la variable d'environnement `EXPLIA_VLC_DIR` ;
//! 2. un dossier `vlc` placé à côté de `explia.exe` (VLC « portable ») ;
//! 3. `%ProgramW6432%\VideoLAN\VLC` puis `%ProgramFiles%\VideoLAN\VLC`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Message affiché quand la lecture intégrée n'est pas possible.
pub const VLC_MISSING_MESSAGE: &str = "Lecture intégrée indisponible : VLC (64 bits) est introuvable. \
Installez VLC (videolan.org) pour lire les vidéos et les sons dans ExPLIA.";

/// Largeur maximale des images décodées : VLC les réduit au besoin (économise de la mémoire).
const MAX_VIDEO_WIDTH: u32 = 1280;

/// Image décodée, prête à devenir une texture.
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Pixels RGBA.
    pub rgba: Vec<u8>,
    /// Incrémenté à chaque nouvelle image.
    pub version: u64,
}

/// État de la lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Loading,
    Playing,
    Paused,
    Ended,
    Error,
}

/// Dossiers où chercher VLC, dans l'ordre de préférence.
pub fn candidate_dirs(
    get_env: impl Fn(&str) -> Option<OsString>,
    exe_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(custom) = get_env("EXPLIA_VLC_DIR").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(custom));
    }
    if let Some(dir) = exe_dir {
        dirs.push(dir.join("vlc"));
    }
    for variable in ["ProgramW6432", "ProgramFiles"] {
        if let Some(base) = get_env(variable).filter(|v| !v.is_empty()) {
            let dir = PathBuf::from(base).join("VideoLAN").join("VLC");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Taille de l'image à décoder : la taille d'origine, réduite à `max_width` si besoin
/// (proportions conservées, dimensions paires et jamais nulles).
pub fn scaled_size(width: u32, height: u32, max_width: u32) -> (u32, u32) {
    let (width, height) = (width.max(2), height.max(2));
    let (w, h) = if width > max_width {
        let h = (u64::from(height) * u64::from(max_width) / u64::from(width)) as u32;
        (max_width, h)
    } else {
        (width, height)
    };
    ((w & !1).max(2), (h & !1).max(2))
}

/// Un VLC utilisable est-il installé ? (Vérification rapide : la bibliothèque n'est pas chargée.)
pub fn is_available() -> bool {
    imp::find_vlc_dir().is_some()
}

/// Lecteur d'un fichier audio ou vidéo.
pub struct Player {
    inner: imp::PlayerImpl,
}

impl Player {
    /// Ouvre le fichier et démarre la lecture. Peut prendre un moment au premier appel
    /// (chargement de VLC) : à appeler hors du thread d'interface.
    pub fn open(path: &Path) -> Result<Player, String> {
        imp::PlayerImpl::open(path).map(|inner| Player { inner })
    }

    pub fn state(&self) -> PlayerState {
        self.inner.state()
    }

    /// Met en pause, ou reprend ; relance depuis le début si la lecture est terminée.
    pub fn toggle_pause(&self) {
        self.inner.toggle_pause();
    }

    pub fn time(&self) -> Duration {
        self.inner.time()
    }

    /// Durée totale, si elle est connue.
    pub fn length(&self) -> Option<Duration> {
        self.inner.length()
    }

    pub fn set_time(&self, time: Duration) {
        self.inner.set_time(time);
    }

    /// Volume de 0 à 100.
    pub fn volume(&self) -> i32 {
        self.inner.volume()
    }

    pub fn set_volume(&self, volume: i32) {
        self.inner.set_volume(volume.clamp(0, 100));
    }

    /// Dernière image décodée, si elle est plus récente que `last_version`.
    pub fn frame_newer_than(&self, last_version: u64) -> Option<Frame> {
        self.inner.frame_newer_than(last_version)
    }
}

/// Image partagée entre le fil de décodage de VLC et l'interface.
struct SharedFrame {
    frame: Mutex<Frame>,
}

impl SharedFrame {
    fn new() -> Self {
        Self {
            frame: Mutex::new(Frame {
                width: 0,
                height: 0,
                rgba: Vec::new(),
                version: 0,
            }),
        }
    }

    fn newer_than(&self, last_version: u64) -> Option<Frame> {
        let frame = self.frame.lock().ok()?;
        (frame.version != last_version && !frame.rgba.is_empty()).then(|| frame.clone())
    }
}

#[cfg(windows)]
mod imp {
    use super::{
        Duration, Frame, MAX_VIDEO_WIDTH, Path, PathBuf, PlayerState, SharedFrame,
        VLC_MISSING_MESSAGE, candidate_dirs, scaled_size,
    };
    use std::cell::UnsafeCell;
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::sync::{Arc, OnceLock};
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
    };

    type Instance = *mut c_void;
    type Media = *mut c_void;
    type MediaPlayer = *mut c_void;

    type LockCb = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
    type UnlockCb = unsafe extern "C" fn(*mut c_void, *mut c_void, *const *mut c_void);
    type DisplayCb = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type SetupCb = unsafe extern "C" fn(
        *mut *mut c_void,
        *mut c_char,
        *mut u32,
        *mut u32,
        *mut u32,
        *mut u32,
    ) -> u32;
    type CleanupCb = unsafe extern "C" fn(*mut c_void);

    /// Fonctions de libVLC (API 3.x) utilisées par ExPLIA.
    struct Api {
        /// Instance libVLC : créée au premier usage, conservée jusqu'à la fin du processus.
        instance: Instance,
        media_new_path: unsafe extern "C" fn(Instance, *const c_char) -> Media,
        media_release: unsafe extern "C" fn(Media),
        player_new_from_media: unsafe extern "C" fn(Media) -> MediaPlayer,
        player_release: unsafe extern "C" fn(MediaPlayer),
        player_play: unsafe extern "C" fn(MediaPlayer) -> c_int,
        player_stop: unsafe extern "C" fn(MediaPlayer),
        player_set_pause: unsafe extern "C" fn(MediaPlayer, c_int),
        player_get_state: unsafe extern "C" fn(MediaPlayer) -> c_int,
        player_get_time: unsafe extern "C" fn(MediaPlayer) -> i64,
        player_set_time: unsafe extern "C" fn(MediaPlayer, i64),
        player_get_length: unsafe extern "C" fn(MediaPlayer) -> i64,
        audio_get_volume: unsafe extern "C" fn(MediaPlayer) -> c_int,
        audio_set_volume: unsafe extern "C" fn(MediaPlayer, c_int) -> c_int,
        video_set_callbacks:
            unsafe extern "C" fn(MediaPlayer, LockCb, UnlockCb, DisplayCb, *mut c_void),
        video_set_format_callbacks: unsafe extern "C" fn(MediaPlayer, SetupCb, CleanupCb),
    }

    // SAFETY: libVLC est conçue pour être appelée depuis plusieurs fils ; `Api` ne contient que
    // des pointeurs de fonctions et le pointeur de l'instance, jamais modifiés après le chargement.
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    static API: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();

    pub fn find_vlc_dir() -> Option<PathBuf> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        candidate_dirs(|name| std::env::var_os(name), exe_dir.as_deref())
            .into_iter()
            .find(|dir| dir.join("libvlc.dll").is_file() && dir.join("libvlccore.dll").is_file())
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn load() -> Result<Arc<Api>, String> {
        let dir = find_vlc_dir().ok_or_else(|| VLC_MISSING_MESSAGE.to_string())?;
        // SAFETY: chemin UTF-16 terminé par zéro ; le drapeau « chemin modifié » fait chercher
        // `libvlccore.dll` et les modules dans le dossier de VLC.
        let module = unsafe {
            LoadLibraryExW(
                wide(&dir.join("libvlc.dll")).as_ptr(),
                std::ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        if module.is_null() {
            return Err(format!(
                "{VLC_MISSING_MESSAGE} (impossible de charger {}).",
                dir.join("libvlc.dll").display()
            ));
        }
        macro_rules! symbol {
            ($name:literal) => {{
                // SAFETY: nom terminé par zéro ; la conversion vers le type de fonction attendu
                // respecte la signature documentée de libVLC 3.x.
                match unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) } {
                    Some(function) => unsafe { std::mem::transmute_copy(&function) },
                    None => {
                        return Err(format!(
                            "VLC incompatible : la fonction {} est introuvable (VLC 3.x est requis).",
                            $name
                        ));
                    }
                }
            }};
        }
        let new_instance: unsafe extern "C" fn(c_int, *const *const c_char) -> Instance =
            symbol!("libvlc_new");
        let arguments: Vec<CString> = [
            "--quiet",
            "--no-video-title-show",
            "--no-osd",
            "--no-snapshot-preview",
            "--no-stats",
        ]
        .iter()
        .filter_map(|argument| CString::new(*argument).ok())
        .collect();
        let pointers: Vec<*const c_char> = arguments.iter().map(|a| a.as_ptr()).collect();
        // SAFETY: `pointers` désigne des chaînes C valides pendant l'appel.
        let instance = unsafe { new_instance(pointers.len() as c_int, pointers.as_ptr()) };
        if instance.is_null() {
            return Err("VLC n'a pas pu démarrer (instance libVLC refusée).".to_string());
        }
        Ok(Arc::new(Api {
            instance,
            media_new_path: symbol!("libvlc_media_new_path"),
            media_release: symbol!("libvlc_media_release"),
            player_new_from_media: symbol!("libvlc_media_player_new_from_media"),
            player_release: symbol!("libvlc_media_player_release"),
            player_play: symbol!("libvlc_media_player_play"),
            player_stop: symbol!("libvlc_media_player_stop"),
            player_set_pause: symbol!("libvlc_media_player_set_pause"),
            player_get_state: symbol!("libvlc_media_player_get_state"),
            player_get_time: symbol!("libvlc_media_player_get_time"),
            player_set_time: symbol!("libvlc_media_player_set_time"),
            player_get_length: symbol!("libvlc_media_player_get_length"),
            audio_get_volume: symbol!("libvlc_audio_get_volume"),
            audio_set_volume: symbol!("libvlc_audio_set_volume"),
            video_set_callbacks: symbol!("libvlc_video_set_callbacks"),
            video_set_format_callbacks: symbol!("libvlc_video_set_format_callbacks"),
        }))
    }

    fn api() -> Result<Arc<Api>, String> {
        API.get_or_init(load).clone()
    }

    /// Mémoire partagée avec les fonctions de rappel de VLC.
    struct Shared {
        /// Image en cours de décodage : touchée uniquement par le fil vidéo de VLC,
        /// entre `lock` et `display`.
        decode: UnsafeCell<Vec<u8>>,
        size: std::sync::Mutex<(u32, u32)>,
        frame: SharedFrame,
    }

    // SAFETY: `decode` n'est accédé que par le fil de sortie vidéo de VLC (appels séquentiels de
    // `setup`, `lock`, `display`) ; le reste est protégé par des verrous.
    unsafe impl Sync for Shared {}
    unsafe impl Send for Shared {}

    unsafe extern "C" fn on_setup(
        opaque: *mut *mut c_void,
        chroma: *mut c_char,
        width: *mut u32,
        height: *mut u32,
        pitches: *mut u32,
        lines: *mut u32,
    ) -> u32 {
        // SAFETY: VLC fournit des pointeurs valides ; `*opaque` est celui passé à
        // `libvlc_video_set_callbacks`, c'est-à-dire notre `Shared`, vivant tant que le lecteur existe.
        unsafe {
            let shared = &*(*opaque as *const Shared);
            let (w, h) = scaled_size(*width, *height, MAX_VIDEO_WIDTH);
            // Format RGBA, quatre caractères suivis d'un zéro.
            for (index, byte) in b"RGBA\0".iter().enumerate() {
                *chroma.add(index) = *byte as c_char;
            }
            *width = w;
            *height = h;
            *pitches = w * 4;
            *lines = h;
            (*shared.decode.get()).resize(w as usize * h as usize * 4, 0);
            if let Ok(mut size) = shared.size.lock() {
                *size = (w, h);
            }
        }
        1
    }

    unsafe extern "C" fn on_cleanup(_opaque: *mut c_void) {}

    unsafe extern "C" fn on_lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
        // SAFETY: voir `on_setup`.
        unsafe {
            let shared = &*(opaque as *const Shared);
            *planes = (*shared.decode.get()).as_mut_ptr().cast();
        }
        std::ptr::null_mut()
    }

    unsafe extern "C" fn on_unlock(
        _opaque: *mut c_void,
        _picture: *mut c_void,
        _planes: *const *mut c_void,
    ) {
    }

    unsafe extern "C" fn on_display(opaque: *mut c_void, _picture: *mut c_void) {
        // SAFETY: voir `on_setup`.
        let shared = unsafe { &*(opaque as *const Shared) };
        let (w, h) = shared.size.lock().map_or((0, 0), |size| *size);
        // SAFETY: seul le fil vidéo de VLC touche `decode` (voir l'`unsafe impl Sync`).
        let decoded = unsafe { &*shared.decode.get() };
        if decoded.len() != w as usize * h as usize * 4 || decoded.is_empty() {
            return;
        }
        if let Ok(mut frame) = shared.frame.frame.lock() {
            frame.width = w as usize;
            frame.height = h as usize;
            frame.rgba.clear();
            frame.rgba.extend_from_slice(decoded);
            // Le format RGBA de VLC laisse l'opacité à zéro : on la force à 255.
            for pixel in frame.rgba.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            frame.version += 1;
        }
    }

    pub struct PlayerImpl {
        api: Arc<Api>,
        player: MediaPlayer,
        /// Doit survivre au lecteur : VLC y accède pendant la lecture.
        shared: Box<Shared>,
    }

    // SAFETY: les objets libVLC peuvent être utilisés depuis n'importe quel fil.
    unsafe impl Send for PlayerImpl {}

    impl PlayerImpl {
        pub fn open(path: &Path) -> Result<Self, String> {
            let api = api()?;
            let c_path = CString::new(path.to_string_lossy().as_bytes())
                .map_err(|_| "Chemin de fichier invalide.".to_string())?;
            // SAFETY: `c_path` est une chaîne C (UTF-8) valide pendant l'appel.
            let media = unsafe { (api.media_new_path)(api.instance, c_path.as_ptr()) };
            if media.is_null() {
                return Err("VLC n'a pas pu ouvrir ce fichier.".to_string());
            }
            // SAFETY: `media` est valide ; le lecteur garde sa propre référence.
            let player = unsafe {
                let player = (api.player_new_from_media)(media);
                (api.media_release)(media);
                player
            };
            if player.is_null() {
                return Err("VLC n'a pas pu créer le lecteur.".to_string());
            }
            let shared = Box::new(Shared {
                decode: UnsafeCell::new(Vec::new()),
                size: std::sync::Mutex::new((0, 0)),
                frame: SharedFrame::new(),
            });
            let opaque = (&*shared as *const Shared).cast_mut().cast::<c_void>();
            // SAFETY: `player` est valide ; `shared` (sur le tas) vit au moins aussi longtemps que
            // le lecteur (voir `Drop`).
            let started = unsafe {
                (api.video_set_callbacks)(player, on_lock, on_unlock, on_display, opaque);
                (api.video_set_format_callbacks)(player, on_setup, on_cleanup);
                (api.player_play)(player)
            };
            let this = Self {
                api: Arc::clone(&api),
                player,
                shared,
            };
            if started != 0 {
                return Err("VLC n'a pas pu démarrer la lecture.".to_string());
            }
            Ok(this)
        }

        pub fn state(&self) -> PlayerState {
            // SAFETY: `self.player` est valide jusqu'au `Drop`.
            match unsafe { (self.api.player_get_state)(self.player) } {
                0..=2 => PlayerState::Loading,
                3 => PlayerState::Playing,
                4 => PlayerState::Paused,
                5 | 6 => PlayerState::Ended,
                _ => PlayerState::Error,
            }
        }

        pub fn toggle_pause(&self) {
            // SAFETY: `self.player` est valide.
            unsafe {
                match self.state() {
                    PlayerState::Ended => {
                        (self.api.player_stop)(self.player);
                        (self.api.player_play)(self.player);
                    }
                    PlayerState::Playing => (self.api.player_set_pause)(self.player, 1),
                    _ => (self.api.player_set_pause)(self.player, 0),
                }
            }
        }

        pub fn time(&self) -> Duration {
            // SAFETY: `self.player` est valide.
            let millis = unsafe { (self.api.player_get_time)(self.player) };
            Duration::from_millis(millis.max(0) as u64)
        }

        pub fn length(&self) -> Option<Duration> {
            // SAFETY: `self.player` est valide.
            let millis = unsafe { (self.api.player_get_length)(self.player) };
            (millis > 0).then(|| Duration::from_millis(millis as u64))
        }

        pub fn set_time(&self, time: Duration) {
            // SAFETY: `self.player` est valide.
            unsafe { (self.api.player_set_time)(self.player, time.as_millis() as i64) };
        }

        pub fn volume(&self) -> i32 {
            // SAFETY: `self.player` est valide.
            unsafe { (self.api.audio_get_volume)(self.player) }.max(0)
        }

        pub fn set_volume(&self, volume: i32) {
            // SAFETY: `self.player` est valide.
            unsafe { (self.api.audio_set_volume)(self.player, volume) };
        }

        pub fn frame_newer_than(&self, last_version: u64) -> Option<Frame> {
            self.shared.frame.newer_than(last_version)
        }
    }

    impl Drop for PlayerImpl {
        fn drop(&mut self) {
            // SAFETY: `stop` attend la fin de la lecture : ensuite VLC n'appelle plus nos fonctions
            // de rappel et `shared` peut être libéré.
            unsafe {
                (self.api.player_stop)(self.player);
                (self.api.player_release)(self.player);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Duration, Frame, Path, PathBuf, PlayerState, VLC_MISSING_MESSAGE};

    pub fn find_vlc_dir() -> Option<PathBuf> {
        None
    }

    pub struct PlayerImpl;

    impl PlayerImpl {
        pub fn open(_path: &Path) -> Result<Self, String> {
            Err(VLC_MISSING_MESSAGE.to_string())
        }
        pub fn state(&self) -> PlayerState {
            PlayerState::Error
        }
        pub fn toggle_pause(&self) {}
        pub fn time(&self) -> Duration {
            Duration::ZERO
        }
        pub fn length(&self) -> Option<Duration> {
            None
        }
        pub fn set_time(&self, _time: Duration) {}
        pub fn volume(&self) -> i32 {
            0
        }
        pub fn set_volume(&self, _volume: i32) {}
        pub fn frame_newer_than(&self, _last_version: u64) -> Option<Frame> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    #[test]
    fn vlc_is_searched_in_the_documented_order() {
        let dirs = candidate_dirs(
            env(&[
                ("EXPLIA_VLC_DIR", "D:\\MonVLC"),
                ("ProgramW6432", "C:\\Program Files"),
                ("ProgramFiles", "C:\\Program Files"),
            ]),
            Some(Path::new("E:\\apps\\explia")),
        );
        assert_eq!(
            dirs,
            [
                PathBuf::from("D:\\MonVLC"),
                PathBuf::from("E:\\apps\\explia").join("vlc"),
                PathBuf::from("C:\\Program Files")
                    .join("VideoLAN")
                    .join("VLC"),
            ]
        );
    }

    #[test]
    fn empty_environment_gives_only_the_portable_folder() {
        assert_eq!(
            candidate_dirs(|_| None, Some(Path::new("E:\\x"))),
            [PathBuf::from("E:\\x").join("vlc")]
        );
        assert!(candidate_dirs(|_| None, None).is_empty());
    }

    #[test]
    fn video_size_is_scaled_down_only_when_needed() {
        assert_eq!(scaled_size(640, 360, 1280), (640, 360));
        assert_eq!(scaled_size(1920, 1080, 1280), (1280, 720));
        assert_eq!(scaled_size(3840, 2160, 1280), (1280, 720));
        // Dimensions impaires ou nulles : toujours paires et non nulles.
        assert_eq!(scaled_size(641, 361, 1280), (640, 360));
        assert_eq!(scaled_size(0, 0, 1280), (2, 2));
        // Format vertical.
        assert_eq!(scaled_size(2000, 4000, 1000), (1000, 2000));
    }

    #[test]
    fn missing_message_names_vlc_and_the_download_site() {
        assert!(VLC_MISSING_MESSAGE.contains("VLC"));
        assert!(VLC_MISSING_MESSAGE.contains("videolan.org"));
    }

    #[test]
    fn shared_frame_only_reports_newer_frames() {
        let shared = SharedFrame::new();
        assert!(shared.newer_than(0).is_none());
        {
            let mut frame = shared.frame.lock().unwrap();
            frame.rgba = vec![1, 2, 3, 255];
            frame.width = 1;
            frame.height = 1;
            frame.version = 5;
        }
        assert!(shared.newer_than(5).is_none());
        assert_eq!(shared.newer_than(4).map(|f| f.version), Some(5));
    }

    #[cfg(windows)]
    #[test]
    fn opening_a_missing_file_never_panics() {
        // Avec ou sans VLC installé : un résultat (Ok ou Err), jamais un plantage.
        let _ = Player::open(Path::new("Z:\\inexistant\\video.mp4"));
    }
}
