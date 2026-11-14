//! Parcours de dossiers en lecture seule : recherche récursive, doublons, gros fichiers.
//!
//! Ces fonctions s'exécutent dans un thread de travail ; elles testent `cancel` régulièrement
//! pour s'arrêter dès qu'un nouveau parcours remplace le courant. Les liens symboliques et les
//! jonctions ne sont jamais suivis (pas de boucle, pas de sortie du dossier demandé).

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::fs_nav::{self, Entry};
use crate::view;

/// Nombre maximal de résultats d'une recherche récursive.
pub const SEARCH_LIMIT: usize = 5000;
/// Octets lus pour l'empreinte rapide qui écarte les faux candidats avant l'empreinte complète.
const PARTIAL_HASH_BYTES: u64 = 64 * 1024;

/// Recherche par nom (voir `view::matches_query`) dans `root` et ses sous-dossiers. Les résultats
/// sont transmis par lots à `emit`. Renvoie `true` si la recherche a été tronquée à `limit`.
pub fn search(
    root: &Path,
    query: &str,
    show_hidden: bool,
    limit: usize,
    cancel: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(Vec<Entry>),
) -> bool {
    let mut stack = vec![root.to_path_buf()];
    let mut batch: Vec<Entry> = Vec::new();
    let mut found = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            if cancel() {
                return false;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if !show_hidden && fs_nav::is_hidden(&name, Some(&meta)) {
                continue;
            }
            let is_real_dir = meta.is_dir() && !fs_nav::is_link(&meta);
            if view::matches_query(&name, query)
                && let Some(entry) = fs_nav::entry_from_path(&item.path())
            {
                batch.push(entry);
                found += 1;
                if batch.len() >= 100 {
                    emit(std::mem::take(&mut batch));
                }
                if found >= limit {
                    emit(batch);
                    return true;
                }
            }
            if is_real_dir {
                stack.push(item.path());
            }
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    false
}

/// Fichiers identiques (même taille et même empreinte de contenu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub size: u64,
    pub files: Vec<PathBuf>,
}

/// Cherche les fichiers de plus de `min_bytes` octets qui ont exactement le même contenu.
/// L'empreinte est un hachage 64 bits du contenu entier, comparée seulement entre fichiers de
/// même taille. `progress` reçoit le nombre de fichiers retenus puis analysés. Les groupes sont
/// triés du plus gros au plus petit.
pub fn find_duplicates(
    root: &Path,
    recursive: bool,
    min_bytes: u64,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Vec<DuplicateGroup> {
    // 1. Fichiers assez gros, groupés par taille.
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            if cancel() {
                return Vec::new();
            }
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if fs_nav::is_link(&meta) {
                continue;
            }
            if meta.is_dir() {
                if recursive {
                    stack.push(item.path());
                }
            } else if meta.len() > min_bytes {
                by_size.entry(meta.len()).or_default().push(item.path());
                seen += 1;
                if seen.is_multiple_of(200) {
                    progress(seen);
                }
            }
        }
    }
    // 2. Seules les tailles partagées par plusieurs fichiers sont candidates.
    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    let total: usize = candidates.iter().map(|(_, files)| files.len()).sum();
    let mut hashed = 0usize;
    let mut groups = Vec::new();
    for (size, files) in candidates {
        // Empreinte rapide (début du fichier), puis complète pour les survivants.
        let mut partial: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for file in files {
            if cancel() {
                return Vec::new();
            }
            if let Some(hash) = hash_file(&file, Some(PARTIAL_HASH_BYTES), cancel) {
                partial.entry(hash).or_default().push(file);
            }
        }
        for same_start in partial.into_values().filter(|files| files.len() > 1) {
            let mut full: HashMap<u64, Vec<PathBuf>> = HashMap::new();
            for file in same_start {
                if cancel() {
                    return Vec::new();
                }
                if let Some(hash) = hash_file(&file, None, cancel) {
                    full.entry(hash).or_default().push(file);
                }
                hashed += 1;
                progress(seen.max(total) + hashed);
            }
            for mut files in full.into_values().filter(|files| files.len() > 1) {
                files.sort();
                groups.push(DuplicateGroup { size, files });
            }
        }
    }
    groups.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.files.cmp(&b.files)));
    groups
}

/// Fichiers de plus de `min_bytes` octets, triés du plus gros au plus petit (puis par chemin).
/// `recursive` : tout l'arbre de `root` ; sinon, seulement les fichiers directs de `root`.
/// `progress` reçoit régulièrement le nombre de fichiers trouvés.
pub fn find_large_files(
    root: &Path,
    recursive: bool,
    min_bytes: u64,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Vec<(PathBuf, u64)> {
    let mut found: Vec<(PathBuf, u64)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            if cancel() {
                return Vec::new();
            }
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if fs_nav::is_link(&meta) {
                continue;
            }
            if meta.is_dir() {
                if recursive {
                    stack.push(item.path());
                }
            } else if meta.len() > min_bytes {
                found.push((item.path(), meta.len()));
                if found.len().is_multiple_of(50) {
                    progress(found.len());
                }
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    found
}

/// Hachage 64 bits du contenu (au plus `limit` octets si donné). `None` en cas d'erreur de
/// lecture ou d'annulation.
fn hash_file(path: &Path, limit: Option<u64>, cancel: &dyn Fn() -> bool) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut remaining = limit.unwrap_or(u64::MAX);
    while remaining > 0 {
        if cancel() {
            return None;
        }
        let want = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file.read(&mut buffer[..want]).ok()?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
        remaining -= read as u64;
    }
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dossier de test jetable, dans `target/`.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-scan")
                .join(name);
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn dir(&self, rel: &str) {
            fs::create_dir_all(self.0.join(rel)).unwrap();
        }

        fn file(&self, rel: &str, content: &[u8]) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        fn rel(&self, paths: &[PathBuf]) -> Vec<String> {
            let mut rel: Vec<String> = paths
                .iter()
                .map(|p| {
                    p.strip_prefix(&self.0)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect();
            rel.sort();
            rel
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn recursive_search_finds_matches_in_subfolders() {
        let tree = TempTree::new("search");
        tree.file("setup.exe", b"x");
        tree.file("sub/deep/tool.EXE", b"x");
        tree.file("sub/readme.txt", b"x");
        tree.file(".hidden/secret.exe", b"x");
        let mut found: Vec<Entry> = Vec::new();
        let truncated = search(&tree.0, "*.exe", false, 100, &never, &mut |batch| {
            found.extend(batch);
        });
        assert!(!truncated);
        let paths: Vec<PathBuf> = found.into_iter().map(|e| e.path).collect();
        assert_eq!(tree.rel(&paths), ["setup.exe", "sub/deep/tool.EXE"]);
        // Avec les éléments cachés.
        let mut with_hidden = 0;
        search(&tree.0, "*.exe", true, 100, &never, &mut |batch| {
            with_hidden += batch.len();
        });
        assert_eq!(with_hidden, 3);
    }

    #[test]
    fn search_is_truncated_at_the_limit() {
        let tree = TempTree::new("search-limit");
        for i in 0..5 {
            tree.file(&format!("f{i}.txt"), b"x");
        }
        let mut count = 0;
        let truncated = search(&tree.0, "f", false, 3, &never, &mut |batch| {
            count += batch.len();
        });
        assert!(truncated);
        assert_eq!(count, 3);
    }

    #[test]
    fn duplicates_have_the_same_size_and_content() {
        let tree = TempTree::new("dups");
        tree.file("a.bin", &[7u8; 2000]);
        tree.file("sub/b.bin", &[7u8; 2000]);
        // Même taille, contenu différent.
        let mut other = vec![7u8; 2000];
        other[1999] = 8;
        tree.file("c.bin", &other);
        // Même contenu mais trop petit pour le seuil.
        tree.file("small1.bin", &[1u8; 10]);
        tree.file("small2.bin", &[1u8; 10]);
        let groups = find_duplicates(&tree.0, true, 1000, &never, &mut |_| {});
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].size, 2000);
        assert_eq!(tree.rel(&groups[0].files), ["a.bin", "sub/b.bin"]);
        // Seuil plus haut que les fichiers : rien.
        assert!(find_duplicates(&tree.0, true, 2000, &never, &mut |_| {}).is_empty());
        // Sans récursivité : le doublon du sous-dossier n'est pas vu.
        assert!(find_duplicates(&tree.0, false, 1000, &never, &mut |_| {}).is_empty());
    }

    #[test]
    fn large_files_are_sorted_from_the_biggest() {
        let tree = TempTree::new("large");
        tree.file("moyen.bin", &[0u8; 3000]);
        tree.file("sub/gros.bin", &[0u8; 5000]);
        tree.file("petit.bin", &[0u8; 500]);
        tree.dir("vide");
        let found = find_large_files(&tree.0, true, 1000, &never, &mut |_| {});
        let paths: Vec<PathBuf> = found.iter().map(|(p, _)| p.clone()).collect();
        let sizes: Vec<u64> = found.iter().map(|(_, s)| *s).collect();
        assert_eq!(sizes, [5000, 3000]);
        assert!(paths[0].ends_with("sub/gros.bin"));
        // Sans récursivité : le fichier du sous-dossier n'est pas vu.
        let direct = find_large_files(&tree.0, false, 1000, &never, &mut |_| {});
        assert_eq!(direct.len(), 1);
        assert!(direct[0].0.ends_with("moyen.bin"));
        // Strictement plus gros que le seuil.
        assert_eq!(
            find_large_files(&tree.0, true, 3000, &never, &mut |_| {}).len(),
            1
        );
        assert!(find_large_files(&tree.0, true, 1000, &|| true, &mut |_| {}).is_empty());
    }

    #[test]
    fn duplicates_differing_only_after_the_partial_hash_are_not_reported() {
        let tree = TempTree::new("dups-late");
        let size = usize::try_from(PARTIAL_HASH_BYTES).unwrap() + 100;
        let first = vec![5u8; size];
        let mut second = first.clone();
        second[size - 1] = 6;
        tree.file("one.bin", &first);
        tree.file("two.bin", &second);
        tree.file("three.bin", &first);
        let groups = find_duplicates(&tree.0, true, 10, &never, &mut |_| {});
        assert_eq!(groups.len(), 1);
        assert_eq!(tree.rel(&groups[0].files), ["one.bin", "three.bin"]);
    }
}
