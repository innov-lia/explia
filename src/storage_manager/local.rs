//! Connecteur local : disques de la machine et partages SMB (lettre de lecteur ou chemin
//! UNC), par les API de fichiers de Windows. Ses éléments sont identifiés par leur chemin
//! absolu (`ItemId::path`). Il s'appuie sur les modules existants (`fs_nav`, `copy`,
//! `fileops`, `trash`, `launch`) et sur leurs règles de sécurité.

use std::path::{Path, PathBuf};

use super::{
    Capabilities, Connector, Drive, ItemId, SourceKind, StorageError, TransferReport, drive_of,
    source_of,
};
use crate::copy::{self, CopyReport};
use crate::fileops;
use crate::fs_nav::{self, Entry};
use crate::launch;
use crate::places::{self, DriveKind};
use crate::trash;

pub struct LocalConnector;

/// Chemin d'un élément que ce connecteur sert (`handles` l'a vérifié).
fn path_of(id: &ItemId) -> Result<&Path, StorageError> {
    id.as_path()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| StorageError::NotFound(format!("élément inconnu : {id}")))
}

fn paths_of(ids: &[ItemId]) -> Result<Vec<PathBuf>, StorageError> {
    ids.iter()
        .map(|id| path_of(id).map(Path::to_path_buf))
        .collect()
}

fn report_of(report: CopyReport) -> TransferReport {
    TransferReport {
        arrived: report.copied.into_iter().map(ItemId::path).collect(),
        errors: report.errors,
    }
}

impl Connector for LocalConnector {
    fn handles(&self, id: &ItemId) -> bool {
        path_of(id).is_ok()
    }

    fn capabilities(&self, id: &ItemId) -> Capabilities {
        let Ok(path) = path_of(id) else {
            return Capabilities::NONE;
        };
        // Pas de corbeille sur un partage réseau ni sur un disque amovible : Windows supprime
        // alors définitivement, après sa propre confirmation.
        let recycle_bin = source_of(id) == SourceKind::Local
            && match drive_of(path) {
                Drive::Letter(root) => places::drive_kind(&root) != DriveKind::Removable,
                _ => true,
            };
        Capabilities {
            list: true,
            open: true,
            rename: true,
            copy_into: true,
            move_items: true,
            create_folder: true,
            properties: true,
            delete: true,
            recycle_bin,
            shell: true,
        }
    }

    fn list(&self, dir: &ItemId) -> Result<Vec<Entry>, StorageError> {
        fs_nav::list_dir(path_of(dir)?).map_err(|err| StorageError::from_io(&err))
    }

    fn open(&self, id: &ItemId) -> Result<(), StorageError> {
        launch::open(path_of(id)?).map_err(StorageError::Other)
    }

    fn open_as_admin(&self, id: &ItemId) -> Result<(), StorageError> {
        launch::open_as_admin(path_of(id)?).map_err(StorageError::Other)
    }

    fn rename(&self, id: &ItemId, new_name: &str) -> Result<ItemId, StorageError> {
        fileops::rename_in_place(path_of(id)?, new_name)
            .map(ItemId::path)
            .map_err(StorageError::Other)
    }

    fn copy_into(&self, sources: &[ItemId], dest: &ItemId) -> TransferReport {
        match (paths_of(sources), path_of(dest)) {
            (Ok(sources), Ok(dest)) => report_of(copy::copy_into(&sources, dest)),
            (Err(err), _) | (_, Err(err)) => TransferReport::failed(err.to_string()),
        }
    }

    fn move_into(&self, sources: &[ItemId], dest: &ItemId) -> TransferReport {
        match (paths_of(sources), path_of(dest)) {
            (Ok(sources), Ok(dest)) => report_of(copy::move_into(&sources, dest)),
            (Err(err), _) | (_, Err(err)) => TransferReport::failed(err.to_string()),
        }
    }

    fn create_folder(&self, parent: &ItemId) -> Result<ItemId, StorageError> {
        fileops::create_folder(path_of(parent)?)
            .map(ItemId::path)
            .map_err(StorageError::Other)
    }

    fn show_properties(&self, id: &ItemId) -> Result<(), StorageError> {
        launch::show_properties(path_of(id)?).map_err(StorageError::Other)
    }

    fn delete(&self, ids: &[ItemId]) -> Result<(), StorageError> {
        trash::move_to_recycle_bin(&paths_of(ids)?).map_err(StorageError::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_manager::StorageManager;
    use std::fs;

    #[test]
    fn local_folders_are_listed_renamed_copied_into_and_moved() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-storage")
            .join("local");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dest")).unwrap();
        fs::write(dir.join("a.txt"), b"x").unwrap();
        let storage = StorageManager::default();
        let root = ItemId::path(&dir);
        let caps = storage.capabilities(&root);
        assert!(caps.list && caps.rename && caps.copy_into && caps.delete);
        let listed = storage.list(&root).unwrap();
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dest", "a.txt"]);
        // Chaque élément listé porte son identifiant.
        assert_eq!(listed[1].id, ItemId::path(dir.join("a.txt")));
        let renamed = storage
            .rename(&ItemId::path(dir.join("a.txt")), "b.txt")
            .unwrap();
        assert_eq!(renamed, ItemId::path(dir.join("b.txt")));
        let report = storage.copy_into(&[renamed], &ItemId::path(dir.join("dest")));
        assert!(report.errors.is_empty());
        assert_eq!(
            report.arrived,
            [ItemId::path(dir.join("dest").join("b.txt"))]
        );
        let folder = storage.create_folder(&root).unwrap();
        let report = storage.move_into(&[ItemId::path(dir.join("b.txt"))], &folder);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let folder = folder.as_path().unwrap();
        assert!(folder.join("b.txt").is_file() && !dir.join("b.txt").exists());
        assert!(matches!(
            storage.list(&ItemId::path(dir.join("absent"))),
            Err(StorageError::NotFound(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_paths_are_not_served() {
        assert!(!LocalConnector.handles(&ItemId::path("relatif.txt")));
        assert_eq!(
            LocalConnector.capabilities(&ItemId::path("relatif.txt")),
            Capabilities::NONE
        );
    }
}
