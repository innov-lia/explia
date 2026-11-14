//! Presse-papiers : Copier, Couper, Coller.
//!
//! Deux choses distinctes :
//! - placer des **références** dans le presse-papiers (Copier, Couper) : seule la liste des
//!   chemins change de place, aucun fichier n'est écrit ; le presse-papiers de Windows est
//!   partagé avec l'Explorateur et les autres applications ;
//! - **coller**, qui copie ou déplace réellement les fichiers : c'est le travail du
//!   `storage_manager`, à qui l'on remet le contenu lu ici (`ClipboardContent`).
//!
//! Le texte (Copier le chemin) passe par le presse-papiers texte d'egui.

use std::path::{Path, PathBuf};

use crate::clipboard::{self, DROPEFFECT_COPY, DROPEFFECT_MOVE};

/// Ce que fera le collage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    /// Éléments coupés : le collage les déplace.
    Cut,
}

impl ClipMode {
    /// Mode d'après l'effet déposé par l'application qui a rempli le presse-papiers : couper
    /// dans l'Explorateur demande aussi un déplacement.
    pub fn from_effect(effect: Option<u32>) -> Self {
        match effect {
            Some(effect) if effect & DROPEFFECT_MOVE != 0 && effect & DROPEFFECT_COPY == 0 => {
                ClipMode::Cut
            }
            _ => ClipMode::Copy,
        }
    }

    fn effect(self) -> u32 {
        match self {
            ClipMode::Copy => DROPEFFECT_COPY,
            ClipMode::Cut => DROPEFFECT_MOVE,
        }
    }
}

/// Fichiers et dossiers à coller, et ce qu'il faut en faire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardContent {
    pub paths: Vec<PathBuf>,
    pub mode: ClipMode,
}

/// Presse-papiers de fichiers d'ExPLIA, adossé à celui de Windows.
#[derive(Debug, Default)]
pub struct ClipboardManager {
    /// Éléments coupés depuis ExPLIA (affichés en grisé jusqu'au collage).
    cut: Vec<PathBuf>,
}

impl ClipboardManager {
    /// Copier : les éléments seront copiés au collage.
    pub fn copy(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.put(paths, ClipMode::Copy)
    }

    /// Couper : les éléments seront déplacés au collage.
    pub fn cut(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.put(paths, ClipMode::Cut)
    }

    fn put(&mut self, paths: &[PathBuf], mode: ClipMode) -> Result<(), String> {
        clipboard::put_files(paths, mode.effect())?;
        self.cut = match mode {
            ClipMode::Cut => paths.to_vec(),
            ClipMode::Copy => Vec::new(),
        };
        Ok(())
    }

    /// Le presse-papiers de Windows contient des fichiers à coller.
    pub fn has_files(&self) -> bool {
        clipboard::has_files()
    }

    /// Contenu à coller (`None` : aucun fichier), qu'il vienne d'ExPLIA ou d'ailleurs.
    pub fn content(&mut self) -> Result<Option<ClipboardContent>, String> {
        let paths = clipboard::clipboard_files()?;
        if paths.is_empty() {
            self.cut.clear();
            return Ok(None);
        }
        let mode = ClipMode::from_effect(clipboard::drop_effect());
        // Une autre application a remplacé le presse-papiers : plus rien n'est « coupé » ici.
        if mode == ClipMode::Copy || paths != self.cut {
            self.cut.clear();
        }
        Ok(Some(ClipboardContent { paths, mode }))
    }

    /// Après le collage d'éléments coupés : ils ont changé de place, le presse-papiers est
    /// vidé (comme dans l'Explorateur).
    pub fn moved(&mut self) {
        self.cut.clear();
        let _ = clipboard::clear();
    }

    /// `path` a été coupé et attend d'être collé.
    pub fn is_cut(&self, path: &Path) -> bool {
        self.cut.iter().any(|p| p == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drop_effect_tells_copy_from_cut() {
        assert_eq!(ClipMode::from_effect(None), ClipMode::Copy);
        assert_eq!(ClipMode::from_effect(Some(DROPEFFECT_COPY)), ClipMode::Copy);
        assert_eq!(ClipMode::from_effect(Some(DROPEFFECT_MOVE)), ClipMode::Cut);
        // Copier ou déplacer au choix : copier, le plus prudent.
        assert_eq!(
            ClipMode::from_effect(Some(DROPEFFECT_COPY | DROPEFFECT_MOVE)),
            ClipMode::Copy
        );
    }

    #[test]
    fn nothing_is_cut_at_first() {
        let manager = ClipboardManager::default();
        assert!(!manager.is_cut(Path::new(r"C:\a.txt")));
    }
}
