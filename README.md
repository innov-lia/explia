# ExPLIA

Explorateur de fichiers Windows nouvelle génération, écrit en Rust (`egui` / `eframe`).

**MVP en lecture seule** : ExPLIA ne supprime, ne renomme, ne déplace, ne copie et n'écrit aucun fichier. Aucune connexion réseau, aucune télémétrie.

## Fonctionnalités

- Fenêtre d'explorateur : barre d'adresse, barre latérale, zone centrale, panneau d'aperçu.
- Accueil « Ce PC » : liste des lecteurs détectés (`C:`, `D:`, …).
- Navigation : double-clic sur un dossier pour l'ouvrir, « Dossier parent », « Ce PC ».
- Barre d'adresse : saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC`, puis Entrée ou « Aller ».
- Barre latérale : Ce PC, Bureau, Documents, Téléchargements et lecteurs.
- Aperçu : miniature des images PNG, JPEG et BMP (décodée hors du thread d'interface) ; pour les autres fichiers, type et taille.

## Prérequis

- Windows 10 ou 11, x86_64.
- [Rust stable](https://rustup.rs) (édition 2024, Rust 1.85 ou plus récent).
- Toolchain `x86_64-pc-windows-msvc` : installer les *Visual Studio Build Tools* avec la charge de travail « Développement Desktop en C++ » (inclut le SDK Windows).

## Compilation et exécution

```bash
cargo run --release
```

## Vérifications

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

## Organisation du code

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Point d'entrée, fenêtre `eframe`. |
| `src/app.rs` | Interface (panneaux, barre d'adresse, aperçu). |
| `src/fs_nav.rs` | Navigation : analyse d'adresse, listage des dossiers, parent. |
| `src/places.rs` | Lecteurs logiques et dossiers usuels (API Windows). |
| `src/preview.rs` | Décodage des images et miniatures. |
