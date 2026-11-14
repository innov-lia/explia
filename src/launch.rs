//! Ouverture d'un fichier avec son application associée, ou lancement d'un exécutable.
//!
//! Ces actions ne sont déclenchées que par un geste explicite de l'utilisateur
//! (double-clic ou menu contextuel). L'exécution en administrateur passe par le
//! contrôle de compte d'utilisateur (UAC) de Windows : c'est lui qui demande la
//! confirmation (ou les identifiants) à l'utilisateur.

use std::path::Path;

/// Extensions des fichiers exécutables proposés « en tant qu'administrateur ».
const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "bat", "cmd", "msi", "com"];

pub fn is_executable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| EXECUTABLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Ouvre le fichier avec l'application associée ; lance l'exécutable s'il en est un.
pub fn open(path: &Path) -> Result<(), String> {
    imp::shell_execute("open", path)
}

/// Ouvre la fenêtre « Propriétés » de Windows pour un fichier ou un dossier (lecture seule
/// pour ExPLIA : ce que l'utilisateur y change, c'est Windows qui l'applique).
pub fn show_properties(path: &Path) -> Result<(), String> {
    imp::show_properties(path)
}

/// Lance le fichier en tant qu'administrateur : Windows affiche la fenêtre UAC.
pub fn open_as_admin(path: &Path) -> Result<(), String> {
    imp::shell_execute("runas", path)
}

/// Arguments de PowerShell pour une console qui reste ouverte dans le dossier `dir`.
/// Le chemin est entre apostrophes (chaîne littérale) ; une apostrophe du chemin est doublée.
pub fn powershell_args(dir: &Path) -> String {
    let literal = dir.display().to_string().replace('\'', "''");
    format!("-NoExit -Command \"Set-Location -LiteralPath '{literal}'\"")
}

/// Ouvre une console PowerShell dans le dossier `dir` ; `admin` : en tant qu'administrateur
/// (Windows affiche la fenêtre UAC).
pub fn open_powershell(dir: &Path, admin: bool) -> Result<(), String> {
    let verb = if admin { "runas" } else { "open" };
    imp::run_program(verb, "powershell.exe", &powershell_args(dir), dir)
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties, ShellExecuteW};

    /// `SW_SHOWNORMAL` : affiche la fenêtre de l'application lancée.
    const SW_SHOWNORMAL: i32 = 1;

    fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        text.as_ref().encode_wide().chain(Some(0)).collect()
    }

    pub fn show_properties(path: &Path) -> Result<(), String> {
        let name = wide(path);
        // SAFETY: `name` est terminée par un zéro et vit pendant l'appel ; fenêtre parente et
        // page de propriétés nulles (autorisé par l'API). La fenêtre de Windows est autonome.
        let shown = unsafe {
            SHObjectProperties(
                std::ptr::null_mut(),
                SHOP_FILEPATH as u32,
                name.as_ptr(),
                std::ptr::null(),
            )
        };
        if shown != 0 {
            Ok(())
        } else {
            Err(format!("Propriétés indisponibles : {}", path.display()))
        }
    }

    /// Lance `program` avec ses arguments, dans le dossier `dir`.
    pub fn run_program(verb: &str, program: &str, args: &str, dir: &Path) -> Result<(), String> {
        let verb = wide(verb);
        let file = wide(program);
        let args = wide(args);
        let dir = wide(dir);
        // SAFETY: toutes les chaînes sont terminées par un zéro et vivent pendant l'appel ;
        // la fenêtre parente est nulle (autorisé par l'API).
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                args.as_ptr(),
                dir.as_ptr(),
                SW_SHOWNORMAL,
            )
        };
        let code = result as isize;
        if code > 32 {
            return Ok(());
        }
        Err(match code {
            2 | 3 => format!("{program} introuvable."),
            5 => "Lancement refusé ou annulé.".to_string(),
            other => format!("Impossible de lancer {program} (code {other})."),
        })
    }

    pub fn shell_execute(verb: &str, path: &Path) -> Result<(), String> {
        let verb = wide(verb);
        let file = wide(path);
        let dir = path.parent().map(wide);
        // SAFETY: toutes les chaînes sont terminées par un zéro et vivent pendant l'appel ;
        // la fenêtre parente et les paramètres sont nuls (autorisé par l'API).
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                SW_SHOWNORMAL,
            )
        };
        // Valeurs <= 32 : codes d'erreur (documentation de `ShellExecuteW`).
        let code = result as isize;
        if code > 32 {
            return Ok(());
        }
        Err(match code {
            2 | 3 => "Fichier introuvable.".to_string(),
            5 => "Lancement refusé ou annulé.".to_string(),
            31 => "Aucune application n'est associée à ce type de fichier.".to_string(),
            other => format!("Impossible d'ouvrir le fichier (code {other})."),
        })
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn show_properties(_path: &Path) -> Result<(), String> {
        Err("Propriétés disponibles uniquement sous Windows.".to_string())
    }

    pub fn shell_execute(_verb: &str, _path: &Path) -> Result<(), String> {
        Err("Ouverture de fichiers disponible uniquement sous Windows.".to_string())
    }

    pub fn run_program(
        _verb: &str,
        _program: &str,
        _args: &str,
        _dir: &Path,
    ) -> Result<(), String> {
        Err("Lancement de programmes disponible uniquement sous Windows.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_starts_in_the_folder_and_stays_open() {
        assert_eq!(
            powershell_args(Path::new("C:\\Mes Dossiers")),
            "-NoExit -Command \"Set-Location -LiteralPath 'C:\\Mes Dossiers'\""
        );
        // Une apostrophe du chemin est doublée dans la chaîne littérale de PowerShell.
        assert!(powershell_args(Path::new("C:\\l'été")).contains("'C:\\l''été'"));
    }

    #[test]
    fn executables_are_recognized() {
        for name in ["a.exe", "a.EXE", "setup.msi", "run.bat", "x.cmd"] {
            assert!(is_executable(Path::new(name)), "{name}");
        }
        for name in ["a.txt", "film.mp4", "a.exe.txt", "exe"] {
            assert!(!is_executable(Path::new(name)), "{name}");
        }
    }
}
