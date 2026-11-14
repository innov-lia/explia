//! Copie et déplacement de fichiers et de dossiers dans un dossier, sur demande explicite de
//! l'utilisateur (Coller, dépôt d'éléments glissés dans la fenêtre, Couper puis Coller).
//!
//! Règles : un élément existant n'est **jamais écrasé** (le nouveau prend un nom libre,
//! « nom - Copie », « nom - Copie (2) »…), un dossier n'est jamais copié ni déplacé dans
//! lui-même ou un de ses sous-dossiers, les liens et jonctions ne sont pas suivis. Un
//! déplacement entre deux lecteurs copie d'abord tout ; l'original n'est retiré qu'après une
//! copie complète et sans erreur.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Bilan d'une copie.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CopyReport {
    /// Éléments copiés au premier niveau (fichiers ou dossiers entiers).
    pub copied: Vec<PathBuf>,
    /// Problèmes rencontrés (élément ignoré ou copie incomplète).
    pub errors: Vec<String>,
}

/// Nom libre dans `dir` pour un élément nommé `name` : `name` s'il n'existe pas, sinon
/// « base - Copie.ext », puis « base - Copie (2).ext »… L'extension des dossiers est ignorée.
pub fn free_destination(
    dir: &Path,
    name: &str,
    is_dir: bool,
    exists: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (base, ext) = match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let mut n = 1u32;
    loop {
        let candidate = if n == 1 {
            format!("{base} - Copie{ext}")
        } else {
            format!("{base} - Copie ({n}){ext}")
        };
        let path = dir.join(candidate);
        if !exists(&path) {
            return path;
        }
        n += 1;
    }
}

/// Copie `sources` dans `dest_dir`. Chaque élément garde son nom, ou prend un nom libre.
pub fn copy_into(sources: &[PathBuf], dest_dir: &Path) -> CopyReport {
    let mut report = CopyReport::default();
    if !dest_dir.is_dir() {
        report
            .errors
            .push(format!("Destination introuvable : {}", dest_dir.display()));
        return report;
    }
    for source in sources {
        let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            report
                .errors
                .push(format!("Élément ignoré : {}", source.display()));
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(source) else {
            report
                .errors
                .push(format!("Introuvable : {}", source.display()));
            continue;
        };
        if crate::fs_nav::is_link(&meta) {
            report
                .errors
                .push(format!("Lien non copié : {}", source.display()));
            continue;
        }
        if meta.is_dir() && dest_dir.starts_with(source) {
            report.errors.push(format!(
                "Un dossier ne peut pas être copié dans lui-même : {}",
                source.display()
            ));
            continue;
        }
        let target = free_destination(dest_dir, &name, meta.is_dir(), &|p| p.exists());
        let result = if meta.is_dir() {
            copy_dir(source, &target, &mut report.errors)
        } else {
            copy_file(source, &target)
        };
        match result {
            Ok(()) => report.copied.push(target),
            Err(message) => report.errors.push(message),
        }
    }
    report
}

/// Déplace `sources` dans `dest_dir` (Couper puis Coller). Chaque élément garde son nom, ou
/// prend un nom libre ; `CopyReport::copied` liste les éléments arrivés.
pub fn move_into(sources: &[PathBuf], dest_dir: &Path) -> CopyReport {
    let mut report = CopyReport::default();
    if !dest_dir.is_dir() {
        report
            .errors
            .push(format!("Destination introuvable : {}", dest_dir.display()));
        return report;
    }
    for source in sources {
        let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            report
                .errors
                .push(format!("Élément ignoré : {}", source.display()));
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(source) else {
            report
                .errors
                .push(format!("Introuvable : {}", source.display()));
            continue;
        };
        if source.parent() == Some(dest_dir) {
            report
                .errors
                .push(format!("Déjà dans ce dossier : {}", source.display()));
            continue;
        }
        if meta.is_dir() && dest_dir.starts_with(source) {
            report.errors.push(format!(
                "Un dossier ne peut pas être déplacé dans lui-même : {}",
                source.display()
            ));
            continue;
        }
        let target = free_destination(dest_dir, &name, meta.is_dir(), &|p| p.exists());
        // Même lecteur : simple changement de chemin, aucune donnée n'est recopiée.
        let renamed = if target.exists() {
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        } else {
            fs::rename(source, &target)
        };
        match renamed {
            Ok(()) => report.copied.push(target),
            Err(err) if crosses_devices(&err) => {
                move_across(source, &target, &meta, &mut report);
            }
            Err(err) => report.errors.push(format!(
                "Déplacement impossible de {} : {err}",
                source.display()
            )),
        }
    }
    report
}

/// Le renommage a échoué parce que la destination est sur un autre lecteur.
fn crosses_devices(err: &io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE (17).
    err.kind() == io::ErrorKind::CrossesDevices || err.raw_os_error() == Some(17)
}

/// Déplacement entre deux lecteurs : copie complète, puis retrait de l'original. Au moindre
/// problème pendant la copie, l'original reste en place.
fn move_across(source: &Path, target: &Path, meta: &fs::Metadata, report: &mut CopyReport) {
    if crate::fs_nav::is_link(meta) {
        report.errors.push(format!(
            "Lien non déplacé vers un autre lecteur : {}",
            source.display()
        ));
        return;
    }
    let mut errors = Vec::new();
    let copied = if meta.is_dir() {
        copy_dir(source, target, &mut errors)
    } else {
        copy_file(source, target)
    };
    match copied {
        Ok(()) if errors.is_empty() => {
            let removed = if meta.is_dir() {
                fs::remove_dir_all(source)
            } else {
                fs::remove_file(source)
            };
            report.copied.push(target.to_path_buf());
            if let Err(err) = removed {
                report.errors.push(format!(
                    "Copié, mais l'original n'a pas pu être retiré : {} ({err})",
                    source.display()
                ));
            }
        }
        Ok(()) => {
            report.errors.extend(errors);
            report.errors.push(format!(
                "Déplacement incomplet : {} reste en place (copie partielle dans {})",
                source.display(),
                target.display()
            ));
        }
        Err(message) => {
            report.errors.push(message);
            report.errors.extend(errors);
        }
    }
}

/// Copie un fichier vers un chemin qui n'existe pas encore.
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("Existe déjà, non écrasé : {}", target.display()));
    }
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|err| format!("Copie impossible de {} : {err}", source.display()))
}

/// Copie un dossier et son contenu ; les erreurs de contenu sont ajoutées à `errors`
/// (la copie continue avec le reste).
fn copy_dir(source: &Path, target: &Path, errors: &mut Vec<String>) -> Result<(), String> {
    // `create_dir` (et non `create_dir_all`) : échoue si le dossier existe, jamais de fusion.
    fs::create_dir(target)
        .map_err(|err| format!("Création impossible de {} : {err}", target.display()))?;
    let read = fs::read_dir(source)
        .map_err(|err| format!("Lecture impossible de {} : {err}", source.display()))?;
    for item in read.flatten() {
        let path = item.path();
        let child = target.join(item.file_name());
        let Ok(meta) = item.metadata() else {
            errors.push(format!("Illisible : {}", path.display()));
            continue;
        };
        if crate::fs_nav::is_link(&meta) {
            errors.push(format!("Lien non copié : {}", path.display()));
        } else if meta.is_dir() {
            if let Err(message) = copy_dir(&path, &child, errors) {
                errors.push(message);
            }
        } else if let Err(message) = copy_file(&path, &child) {
            errors.push(message);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-copy")
                .join(name);
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, rel: &str, content: &str) -> PathBuf {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn moving_takes_items_out_of_their_folder_without_overwriting() {
        let tree = TempTree::new("move");
        let a = tree.file("src/a.txt", "nouveau");
        tree.file("src/dossier/b.txt", "b");
        tree.file("dest/a.txt", "ancien");
        let dest = tree.0.join("dest");
        let report = move_into(&[a.clone(), tree.0.join("src/dossier")], &dest);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(!a.exists() && !tree.0.join("src/dossier").exists());
        // L'élément existant n'est pas écrasé : le nouveau prend un nom libre.
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "ancien");
        assert_eq!(
            fs::read_to_string(dest.join("a - Copie.txt")).unwrap(),
            "nouveau"
        );
        assert!(dest.join("dossier/b.txt").is_file());
    }

    #[test]
    fn a_folder_is_never_moved_into_itself_or_where_it_already_is() {
        let tree = TempTree::new("move-self");
        tree.file("d/sub/x.txt", "x");
        let d = tree.0.join("d");
        let report = move_into(std::slice::from_ref(&d), &d.join("sub"));
        assert!(report.copied.is_empty() && report.errors.len() == 1);
        let report = move_into(&[d.join("sub")], &d);
        assert!(report.copied.is_empty() && report.errors.len() == 1);
        assert!(d.join("sub/x.txt").is_file());
    }

    #[test]
    fn free_names_follow_the_windows_style() {
        let taken = [
            "C:\\d\\rapport.txt",
            "C:\\d\\rapport - Copie.txt",
            "C:\\d\\Photos",
        ];
        let exists = |p: &Path| taken.iter().any(|t| Path::new(t) == p);
        let dir = Path::new("C:\\d");
        assert_eq!(
            free_destination(dir, "neuf.txt", false, &exists),
            dir.join("neuf.txt")
        );
        assert_eq!(
            free_destination(dir, "rapport.txt", false, &exists),
            dir.join("rapport - Copie (2).txt")
        );
        assert_eq!(
            free_destination(dir, "Photos", true, &exists),
            dir.join("Photos - Copie")
        );
        // Un dossier avec un point garde son nom entier.
        let dotted = |p: &Path| p == dir.join("v1.2");
        assert_eq!(
            free_destination(dir, "v1.2", true, &dotted),
            dir.join("v1.2 - Copie")
        );
    }

    #[test]
    fn copies_files_and_folders_without_overwriting() {
        let tree = TempTree::new("copy");
        let a = tree.file("src/a.txt", "A");
        tree.file("src/dossier/b.txt", "B");
        tree.file("src/dossier/sous/c.txt", "C");
        let dest = tree.0.join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("a.txt"), "ancien").unwrap();
        let report = copy_into(&[a.clone(), tree.0.join("src/dossier")], &dest);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.copied.len(), 2);
        // L'existant n'est pas écrasé ; la copie prend un nom libre.
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "ancien");
        assert_eq!(fs::read_to_string(dest.join("a - Copie.txt")).unwrap(), "A");
        assert_eq!(
            fs::read_to_string(dest.join("dossier/sous/c.txt")).unwrap(),
            "C"
        );
        // La source est intacte (copie, pas déplacement).
        assert!(a.exists());
    }

    #[test]
    fn pasting_into_the_same_folder_makes_a_copy() {
        let tree = TempTree::new("same");
        let a = tree.file("a.txt", "A");
        let report = copy_into(std::slice::from_ref(&a), &tree.0);
        assert_eq!(report.copied, [tree.0.join("a - Copie.txt")]);
    }

    #[test]
    fn a_folder_is_never_copied_into_itself() {
        let tree = TempTree::new("itself");
        tree.file("dossier/x.txt", "x");
        let folder = tree.0.join("dossier");
        let report = copy_into(std::slice::from_ref(&folder), &folder.join("."));
        assert!(report.copied.is_empty());
        assert_eq!(report.errors.len(), 1);
        fs::create_dir_all(folder.join("sous")).unwrap();
        let report = copy_into(std::slice::from_ref(&folder), &folder.join("sous"));
        assert!(report.copied.is_empty());
        assert!(!folder.join("sous").join("dossier").exists());
    }

    #[test]
    fn missing_sources_and_destination_are_reported() {
        let tree = TempTree::new("missing");
        let report = copy_into(&[tree.0.join("absent.txt")], &tree.0);
        assert!(report.copied.is_empty() && report.errors.len() == 1);
        let a = tree.file("a.txt", "A");
        let report = copy_into(&[a], &tree.0.join("nulle-part"));
        assert!(report.copied.is_empty() && report.errors.len() == 1);
    }
}
