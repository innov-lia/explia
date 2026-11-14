//! Connecteur local : disques de la machine et partages SMB (lettre de lecteur ou chemin
//! UNC), par les API de fichiers de Windows. Il s'appuie sur les modules existants
//! (`fs_nav`, `copy`, `fileops`, `trash`, `launch`) et sur leurs règles de sécurité.

use std::path::{Path, PathBuf};

use super::{Capabilities, Connector, Drive, SourceKind, StorageError, drive_of, source_of};
use crate::copy::{self, CopyReport};
use crate::fileops;
use crate::fs_nav::{self, Entry};
use crate::launch;
use crate::places::{self, DriveKind};
use crate::trash;

pub struct LocalConnector;

impl Connector for LocalConnector {
    fn handles(&self, path: &Path) -> bool {
        path.is_absolute()
    }

    fn capabilities(&self, path: &Path) -> Capabilities {
        // Pas de corbeille sur un partage réseau ni sur un disque amovible : Windows supprime
        // alors définitivement, après sa propre confirmation.
        let recycle_bin = source_of(path) == SourceKind::Local
            && match drive_of(path) {
                Drive::Letter(root) => places::drive_kind(&root) != DriveKind::Removable,
                _ => true,
            };
        Capabilities {
            list: true,
            open: true,
            rename: true,
            copy_into: true,
            delete: true,
            recycle_bin,
            shell: true,
        }
    }

    fn list(&self, dir: &Path) -> Result<Vec<Entry>, StorageError> {
        fs_nav::list_dir(dir).map_err(|err| StorageError::from_io(&err))
    }

    fn open(&self, path: &Path) -> Result<(), StorageError> {
        launch::open(path).map_err(StorageError::Other)
    }

    fn open_as_admin(&self, path: &Path) -> Result<(), StorageError> {
        launch::open_as_admin(path).map_err(StorageError::Other)
    }

    fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, StorageError> {
        fileops::rename_in_place(path, new_name).map_err(StorageError::Other)
    }

    fn copy_into(&self, sources: &[PathBuf], dest: &Path) -> CopyReport {
        copy::copy_into(sources, dest)
    }

    fn delete(&self, paths: &[PathBuf]) -> Result<(), StorageError> {
        trash::move_to_recycle_bin(paths).map_err(StorageError::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_manager::StorageManager;
    use std::fs;

    #[test]
    fn local_folders_are_listed_renamed_and_copied_into() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-storage")
            .join("local");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dest")).unwrap();
        fs::write(dir.join("a.txt"), b"x").unwrap();
        let storage = StorageManager::default();
        let caps = storage.capabilities(&dir);
        assert!(caps.list && caps.rename && caps.copy_into && caps.delete);
        let names: Vec<String> = storage
            .list(&dir)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["dest", "a.txt"]);
        let renamed = storage.rename(&dir.join("a.txt"), "b.txt").unwrap();
        assert_eq!(renamed, dir.join("b.txt"));
        let report = storage.copy_into(&[renamed], &dir.join("dest"));
        assert!(report.errors.is_empty());
        assert!(dir.join("dest").join("b.txt").is_file());
        assert!(matches!(
            storage.list(&dir.join("absent")),
            Err(StorageError::NotFound(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
