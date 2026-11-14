//! Sources de stockage : interface commune des connecteurs et de leurs capacités.
//!
//! Le gestionnaire (`StorageManager`) choisit le connecteur qui sert un chemin et lui confie
//! l'opération ; il ne contient aucun code propre à une source. Chaque connecteur déclare ce
//! qu'il sait faire (`Capabilities`) : l'interface n'offre que les commandes possibles.
//!
//! Connecteurs : `local` (disques de la machine et partages SMB, par lettre de lecteur ou
//! chemin UNC). SharePoint et OneDrive via Microsoft Graph sont prévus mais reportés : ils
//! demandent une connexion réseau et un compte, interdits pour l'instant (voir AGENTS.md).

mod local;

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Arc;

use crate::copy::CopyReport;
use crate::fs_nav::Entry;
use crate::places::{self, DriveKind};

pub use local::LocalConnector;

/// Source de stockage d'un élément.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Disque de la machine (fixe, amovible, optique).
    Local,
    /// Partage réseau SMB : lecteur réseau (`Z:`) ou chemin UNC (`\\serveur\partage`).
    Network,
}

/// Source de `path`, d'après son préfixe (lettre de lecteur ou chemin UNC).
pub fn source_of(path: &Path) -> SourceKind {
    match drive_of(path) {
        Drive::Unc => SourceKind::Network,
        Drive::Letter(root) if places::drive_kind(&root) == DriveKind::Network => {
            SourceKind::Network
        }
        _ => SourceKind::Local,
    }
}

/// Lecteur qui porte un chemin.
enum Drive {
    Unc,
    Letter(PathBuf),
    Unknown,
}

fn drive_of(path: &Path) -> Drive {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => Drive::Unc,
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Drive::Letter(PathBuf::from(format!("{}:\\", char::from(letter))))
            }
            _ => Drive::Unknown,
        },
        _ => Drive::Unknown,
    }
}

/// Ce qu'une source permet sur un chemin donné.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Lister le contenu d'un dossier.
    pub list: bool,
    /// Ouvrir avec l'application associée, lancer un exécutable (et en administrateur).
    pub open: bool,
    pub rename: bool,
    /// Recevoir une copie (Coller, dépôt).
    pub copy_into: bool,
    /// Déplacer des éléments (Couper puis Coller).
    pub move_items: bool,
    /// Créer un dossier.
    pub create_folder: bool,
    /// Afficher les propriétés (fenêtre de Windows).
    pub properties: bool,
    /// Supprimer (après confirmation dans ExPLIA).
    pub delete: bool,
    /// La suppression passe par la corbeille. Sinon (partage réseau, disque amovible),
    /// Windows supprime définitivement, après sa propre confirmation.
    pub recycle_bin: bool,
    /// Ouvrir une console (PowerShell) dans le dossier.
    pub shell: bool,
}

impl Capabilities {
    /// Aucune opération possible (chemin servi par aucun connecteur).
    pub const NONE: Capabilities = Capabilities {
        list: false,
        open: false,
        rename: false,
        copy_into: false,
        move_items: false,
        create_folder: false,
        properties: false,
        delete: false,
        recycle_bin: false,
        shell: false,
    };
}

/// Erreur d'une opération de stockage, avec un message pour l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// La source ne propose pas cette opération.
    Unsupported(&'static str),
    /// Droits insuffisants.
    Denied(String),
    NotFound(String),
    AlreadyExists(String),
    /// Partage réseau injoignable ou connexion interrompue.
    Network(String),
    Other(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Unsupported(operation) => {
                write!(
                    f,
                    "Opération non disponible sur cette source : {operation}."
                )
            }
            StorageError::Network(detail) => write!(f, "Erreur réseau : {detail}"),
            StorageError::Denied(message)
            | StorageError::NotFound(message)
            | StorageError::AlreadyExists(message)
            | StorageError::Other(message) => f.write_str(message),
        }
    }
}

/// Codes Windows d'un partage réseau injoignable ou d'une connexion perdue.
const NETWORK_ERROR_CODES: &[i32] = &[
    51,   // ERROR_REM_NOT_LIST
    53,   // ERROR_BAD_NETPATH
    59,   // ERROR_UNEXP_NET_ERR
    64,   // ERROR_NETNAME_DELETED
    67,   // ERROR_BAD_NET_NAME
    1222, // ERROR_NO_NETWORK
    1231, // ERROR_NETWORK_UNREACHABLE
];

impl StorageError {
    /// Traduit une erreur d'entrée-sortie (le message de Windows est conservé tel quel ;
    /// l'appelant précise l'élément concerné).
    pub fn from_io(err: &io::Error) -> Self {
        let detail = err.to_string();
        if err
            .raw_os_error()
            .is_some_and(|code| NETWORK_ERROR_CODES.contains(&code))
        {
            return StorageError::Network(detail);
        }
        match err.kind() {
            io::ErrorKind::PermissionDenied => StorageError::Denied(detail),
            io::ErrorKind::NotFound => StorageError::NotFound(detail),
            io::ErrorKind::AlreadyExists => StorageError::AlreadyExists(detail),
            _ => StorageError::Other(detail),
        }
    }
}

/// Interface commune des connecteurs. Un connecteur n'implémente que ce que sa source
/// permet réellement ; les autres opérations répondent `Unsupported`.
pub trait Connector: Send + Sync {
    /// Le connecteur sert ce chemin.
    fn handles(&self, path: &Path) -> bool;

    /// Opérations possibles sur `path`.
    fn capabilities(&self, path: &Path) -> Capabilities;

    /// Contenu d'un dossier.
    fn list(&self, dir: &Path) -> Result<Vec<Entry>, StorageError>;

    /// Ouvre avec l'application associée (ou lance un exécutable).
    fn open(&self, _path: &Path) -> Result<(), StorageError> {
        Err(StorageError::Unsupported("ouvrir"))
    }

    /// Lance en tant qu'administrateur.
    fn open_as_admin(&self, _path: &Path) -> Result<(), StorageError> {
        Err(StorageError::Unsupported("exécuter en administrateur"))
    }

    /// Renomme dans le même dossier, sans écraser ; renvoie le nouveau chemin.
    fn rename(&self, _path: &Path, _new_name: &str) -> Result<PathBuf, StorageError> {
        Err(StorageError::Unsupported("renommer"))
    }

    /// Copie `sources` dans `dest`, sans jamais écraser.
    fn copy_into(&self, _sources: &[PathBuf], _dest: &Path) -> CopyReport {
        CopyReport {
            errors: vec![StorageError::Unsupported("coller").to_string()],
            ..CopyReport::default()
        }
    }

    /// Déplace `sources` dans `dest`, sans jamais écraser.
    fn move_into(&self, _sources: &[PathBuf], _dest: &Path) -> CopyReport {
        CopyReport {
            errors: vec![StorageError::Unsupported("déplacer").to_string()],
            ..CopyReport::default()
        }
    }

    /// Crée un dossier vide sous un nom libre dans `parent` ; renvoie son chemin.
    fn create_folder(&self, _parent: &Path) -> Result<PathBuf, StorageError> {
        Err(StorageError::Unsupported("créer un dossier"))
    }

    /// Affiche les propriétés de l'élément.
    fn show_properties(&self, _path: &Path) -> Result<(), StorageError> {
        Err(StorageError::Unsupported("propriétés"))
    }

    /// Supprime (corbeille si la source en a une).
    fn delete(&self, _paths: &[PathBuf]) -> Result<(), StorageError> {
        Err(StorageError::Unsupported("supprimer"))
    }
}

/// Choisit le connecteur de chaque chemin et lui confie les opérations.
#[derive(Clone)]
pub struct StorageManager {
    connectors: Vec<Arc<dyn Connector>>,
}

impl Default for StorageManager {
    fn default() -> Self {
        Self::with_connectors(vec![Arc::new(LocalConnector)])
    }
}

impl StorageManager {
    /// Gestionnaire servi par `connectors`, consultés dans l'ordre.
    pub fn with_connectors(connectors: Vec<Arc<dyn Connector>>) -> Self {
        Self { connectors }
    }

    fn connector_for(&self, path: &Path) -> Option<&Arc<dyn Connector>> {
        self.connectors.iter().find(|c| c.handles(path))
    }

    fn with<T>(
        &self,
        path: &Path,
        operation: impl FnOnce(&dyn Connector) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        match self.connector_for(path) {
            Some(connector) => operation(connector.as_ref()),
            None => Err(StorageError::NotFound(format!(
                "aucune source pour {}",
                path.display()
            ))),
        }
    }

    pub fn capabilities(&self, path: &Path) -> Capabilities {
        self.connector_for(path)
            .map_or(Capabilities::NONE, |c| c.capabilities(path))
    }

    pub fn list(&self, dir: &Path) -> Result<Vec<Entry>, StorageError> {
        self.with(dir, |c| c.list(dir))
    }

    pub fn open(&self, path: &Path) -> Result<(), StorageError> {
        self.with(path, |c| c.open(path))
    }

    pub fn open_as_admin(&self, path: &Path) -> Result<(), StorageError> {
        self.with(path, |c| c.open_as_admin(path))
    }

    pub fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, StorageError> {
        self.with(path, |c| c.rename(path, new_name))
    }

    /// Copie vers `dest` : c'est le connecteur de la destination qui écrit.
    pub fn copy_into(&self, sources: &[PathBuf], dest: &Path) -> CopyReport {
        match self.connector_for(dest) {
            Some(connector) => connector.copy_into(sources, dest),
            None => CopyReport {
                errors: vec![format!("Aucune source pour {}", dest.display())],
                ..CopyReport::default()
            },
        }
    }

    /// Déplace vers `dest` : sources et destination doivent relever du même connecteur.
    pub fn move_into(&self, sources: &[PathBuf], dest: &Path) -> CopyReport {
        match self.connector_for(dest) {
            Some(connector) if sources.iter().all(|s| connector.handles(s)) => {
                connector.move_into(sources, dest)
            }
            _ => CopyReport {
                errors: vec![
                    StorageError::Unsupported("déplacer entre ces deux sources").to_string(),
                ],
                ..CopyReport::default()
            },
        }
    }

    pub fn create_folder(&self, parent: &Path) -> Result<PathBuf, StorageError> {
        self.with(parent, |c| c.create_folder(parent))
    }

    pub fn show_properties(&self, path: &Path) -> Result<(), StorageError> {
        self.with(path, |c| c.show_properties(path))
    }

    /// Supprime des éléments, regroupés par connecteur.
    pub fn delete(&self, paths: &[PathBuf]) -> Result<(), StorageError> {
        let Some(first) = paths.first() else {
            return Err(StorageError::Other(
                "Aucun élément à supprimer.".to_string(),
            ));
        };
        self.with(first, |c| {
            if paths.iter().all(|p| c.handles(p)) {
                c.delete(paths)
            } else {
                Err(StorageError::Unsupported(
                    "supprimer des éléments de plusieurs sources",
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Connecteur de test qui ne sait que lister `R:\`.
    struct ReadOnly;

    impl Connector for ReadOnly {
        fn handles(&self, path: &Path) -> bool {
            path.starts_with(r"R:\")
        }

        fn capabilities(&self, _path: &Path) -> Capabilities {
            Capabilities {
                list: true,
                ..Capabilities::NONE
            }
        }

        fn list(&self, _dir: &Path) -> Result<Vec<Entry>, StorageError> {
            Ok(Vec::new())
        }
    }

    fn read_only() -> StorageManager {
        StorageManager::with_connectors(vec![Arc::new(ReadOnly)])
    }

    #[test]
    fn operations_a_source_lacks_are_reported_as_unsupported() {
        let storage = read_only();
        let path = Path::new(r"R:\doc.txt");
        assert!(storage.capabilities(path).list);
        assert!(!storage.capabilities(path).rename);
        assert!(!storage.capabilities(path).move_items);
        assert_eq!(
            storage.create_folder(Path::new(r"R:\")),
            Err(StorageError::Unsupported("créer un dossier"))
        );
        let moved = storage.move_into(&[PathBuf::from(r"R:")], Path::new(r"R:"));
        assert!(moved.copied.is_empty() && moved.errors.len() == 1);
        assert_eq!(
            storage.rename(path, "x.txt"),
            Err(StorageError::Unsupported("renommer"))
        );
        let report = storage.copy_into(&[PathBuf::from(r"C:\a")], Path::new(r"R:\"));
        assert!(report.copied.is_empty());
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn a_path_without_connector_has_no_capability() {
        let storage = read_only();
        let path = Path::new(r"Q:\x");
        assert_eq!(storage.capabilities(path), Capabilities::NONE);
        assert!(matches!(storage.list(path), Err(StorageError::NotFound(_))));
    }

    #[test]
    fn deleting_across_sources_is_refused() {
        let storage =
            StorageManager::with_connectors(vec![Arc::new(ReadOnly), Arc::new(LocalConnector)]);
        let paths = [PathBuf::from(r"R:\a"), PathBuf::from(r"C:\b")];
        assert!(matches!(
            storage.delete(&paths),
            Err(StorageError::Unsupported(_))
        ));
    }

    #[test]
    fn io_errors_keep_their_meaning() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(matches!(
            StorageError::from_io(&denied),
            StorageError::Denied(_)
        ));
        let network = io::Error::from_raw_os_error(53);
        assert!(matches!(
            StorageError::from_io(&network),
            StorageError::Network(_)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn unc_paths_are_network_shares() {
        assert_eq!(
            source_of(Path::new(r"\\serveur\partage\dossier")),
            SourceKind::Network
        );
        assert_eq!(
            source_of(Path::new(r"\\?\UNC\serveur\partage")),
            SourceKind::Network
        );
    }
}
