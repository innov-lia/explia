# ExPLIA

Explorateur de fichiers Windows nouvelle génération, écrit en Rust (`egui` / `eframe`).

**MVP en lecture seule** : ExPLIA ne supprime, ne renomme, ne déplace, ne copie et ne modifie aucun de vos fichiers. Aucune connexion réseau, aucune télémétrie.
Exceptions :
- vos réglages (taille de la fenêtre, options, favoris) sont enregistrés dans `%APPDATA%\ExPLIA\settings.txt` (format `clé=valeur`, modifiable à la main) ; un ancien `favoris.txt` est repris automatiquement ;
- sur action explicite de votre part (double-clic, menu contextuel, boutons du volet d'aperçu), ExPLIA ouvre un fichier avec son application par défaut ou lance un exécutable, y compris « en tant qu'administrateur » (Windows affiche alors sa fenêtre UAC) ;
- sur demande explicite et après confirmation, ExPLIA envoie les éléments sélectionnés à la **corbeille de Windows** (récupérables) ; il ne supprime jamais définitivement de lui-même ;
- sur demande explicite, ExPLIA **renomme** un fichier ou un dossier, dans son dossier d'origine, sans jamais écraser un élément existant.

## Fonctionnalités

- Fenêtre d'explorateur : barre d'adresse, barre latérale, zone centrale, panneau d'aperçu. Elle s'ouvre à 80 % de l'écran, centrée, puis la taille que vous lui donnez est mémorisée.
- Accueil « Ce PC » : liste des lecteurs détectés (`C:`, `D:`, …) avec leur taille.
- Colonne « Taille » : taille des fichiers et, par défaut, des dossiers (calculée en arrière-plan, « Calcul… » en attendant ; un dossier inaccessible affiche 0 o).
- Navigation : double-clic sur un dossier pour l'ouvrir, « Dossier parent », « Ce PC ».
- Barre d'adresse : fil d'Ariane cliquable (`Ce PC > D: > Projets > …`) pour remonter à n'importe quel niveau. Un clic sur l'espace libre (ou sur le crayon) permet de saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC` ; Entrée valide, Échap annule.
- Barre d'options : **Trier** (nom, taille, type ; croissant ou décroissant), **Afficher** (masquer ou afficher le volet d'aperçu), **Extension** (`Tout`, `Custom` pour saisir vos propres extensions, par exemple `mkv, pdf`, ou les catégories `Image`, `Documents`, `Code` ; les dossiers restent visibles), **Option** (« Pas de calcul de taille de dossier », utile sur les gros disques). Tous ces choix sont mémorisés.
- Multi-sélection et suppression : **clic maintenu** (0,5 s) sur une ligne pour démarrer la sélection, puis un clic sur chaque ligne à ajouter ou retirer. **Ctrl+clic** ajoute ou retire une ligne ; **Maj+clic** sélectionne toute la plage entre le dernier élément choisi et celui sur lequel vous cliquez. Un seul type à la fois : uniquement des dossiers, ou uniquement des fichiers. Le volet d'aperçu devient un volet « Actions » : **Supprimer** (envoi à la corbeille, avec confirmation) ou **Désélectionner** (Échap fonctionne aussi).
- Actions sur un élément : un clic sur un **dossier** transforme le volet « Aperçu » en volet « Actions » (**Renommer**, **Supprimer**) ; sur un fichier, ces boutons s'ajoutent sous l'aperçu. **F2**, le bouton « Renommer » ou le clic droit puis « Renommer » permettent de renommer directement dans la liste (Entrée valide, Échap annule ; l'extension n'est pas sélectionnée). **Suppr** ou le bouton « Supprimer » demande la suppression : le bouton devient « Confirmer » **à la même place** (un second clic au même endroit valide ; « Annuler » est juste à côté). Un double-clic accidentel est ignoré pendant 0,6 s.
- Ouvrir : double-clic sur un fichier (vidéo `.mp4`/`.mkv`, document, exécutable…) pour l'ouvrir avec l'application par défaut de Windows, qui lit les vidéos. Clic droit sur un exécutable (`.exe`, `.bat`, `.cmd`, `.msi`) : « Exécuter » ou « Exécuter en tant qu'administrateur » (fenêtre UAC). Le volet d'aperçu propose les mêmes boutons.
- Barre latérale : Favoris (Ce PC, puis les dossiers que vous ajoutez) et lecteurs. Clic droit sur un dossier de la liste : « Ajouter aux favoris » ; clic droit sur un de vos favoris : « Retirer des favoris ».
- Aperçu (décodé hors du thread d'interface) : images PNG, JPEG, BMP, GIF (première image) et WebP ; fichiers texte courants (`.txt`, `.md`, `.json`, `.csv`, `.rs`, `.log`, …, 64 Ko affichés au maximum) ; contenu des archives `.zip` (liste des fichiers, rien n'est extrait) ; pour les autres fichiers (PDF, Office, vidéos…), type et taille. La flèche `<` devant « Aperçu » agrandit le volet pour tout voir d'un coup, `>` le réduit.

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
| `src/favorites.rs` | Liste des favoris. |
| `src/settings.rs` | Fichier de réglages (`settings.txt`) : lecture, écriture, valeurs par défaut. |
| `src/fileops.rs` | Renommage : validation du nom Windows, refus d'écraser. |
| `src/trash.rs` | Envoi à la corbeille (`SHFileOperationW` avec `FOF_ALLOWUNDO`). |
| `src/launch.rs` | Ouverture d'un fichier / lancement d'un exécutable (`ShellExecuteW`, UAC). |
| `src/places.rs` | Lecteurs logiques et dossiers usuels (API Windows). |
| `src/preview.rs` | Décodage des images et miniatures. |
