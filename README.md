# ExPLIA

Explorateur de fichiers Windows nouvelle génération, écrit en Rust (`egui` / `eframe`).

**MVP en lecture seule** : ExPLIA ne supprime, ne renomme, ne déplace, ne copie et ne modifie aucun de vos fichiers. Aucune connexion réseau, aucune télémétrie.
Seule exception : la liste de vos favoris, enregistrée dans `%APPDATA%\ExPLIA\favoris.txt` (un chemin par ligne).

## Fonctionnalités

- Fenêtre d'explorateur : barre d'adresse, barre latérale, zone centrale, panneau d'aperçu. Elle s'ouvre à 60 % de l'écran, centrée.
- Accueil « Ce PC » : liste des lecteurs détectés (`C:`, `D:`, …) avec leur taille.
- Colonne « Taille » : taille des fichiers et, par défaut, des dossiers (calculée en arrière-plan, « Calcul… » en attendant ; un dossier inaccessible affiche 0 o).
- Navigation : double-clic sur un dossier pour l'ouvrir, « Dossier parent », « Ce PC ».
- Barre d'adresse : saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC`, puis Entrée.
- Barre d'options : **Trier** (nom, taille, type ; croissant ou décroissant), **Afficher** (masquer ou afficher le volet d'aperçu), **Extension** (tout, ou uniquement les extensions courantes cochées ; les dossiers restent visibles), **Option** (« Pas de calcul de taille de dossier », utile sur les gros disques ; non conservé à la fermeture).
- Barre latérale : Favoris (Ce PC, puis les dossiers que vous ajoutez) et lecteurs. Clic droit sur un dossier de la liste : « Ajouter aux favoris » ; clic droit sur un de vos favoris : « Retirer des favoris ».
- Aperçu (décodé hors du thread d'interface) : images PNG, JPEG, BMP, GIF (première image) et WebP ; fichiers texte courants (`.txt`, `.md`, `.json`, `.csv`, `.rs`, `.log`, …, 64 Ko affichés au maximum) ; pour les autres fichiers (PDF, Office, archives…), type et taille. La flèche `<` devant « Aperçu » agrandit le volet pour tout voir d'un coup, `>` le réduit.

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
| `src/view.rs` | Tri et filtre par extension de la liste affichée. |
| `src/favorites.rs` | Favoris de l'utilisateur et leur fichier de sauvegarde. |
| `src/places.rs` | Lecteurs logiques et dossiers usuels (API Windows). |
| `src/preview.rs` | Décodage des images et miniatures. |
