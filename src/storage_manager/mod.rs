//! Sources de stockage : interface commune des connecteurs et de leurs capacités.

use std::path::{Component, Path, Prefix};

use crate::places::{self, DriveKind};

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
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => SourceKind::Network,
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                let root = format!("{}:\\", char::from(letter));
                if places::drive_kind(Path::new(&root)) == DriveKind::Network {
                    SourceKind::Network
                } else {
                    SourceKind::Local
                }
            }
            _ => SourceKind::Local,
        },
        _ => SourceKind::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
