# ExPLIA

Explorateur de fichiers Windows nouvelle génération, écrit en Rust (`egui` / `eframe`) : navigation rapide, trois façons d'afficher un dossier (liste, colonnes « style Mac », grille d'images), aperçus, lecture audio et vidéo intégrée, renommage et suppression vers la corbeille.

## Télécharger et lancer

1. Ouvrez la page des [**versions**](https://github.com/innov-lia/explia/releases) et téléchargez le fichier `explia-…-windows-x64.zip` de la dernière version (par exemple `explia-v0.1.0-windows-x64.zip`).
2. Décompressez-le et lancez `explia.exe` : il n'y a rien à installer.

Prérequis : Windows 10 ou 11, 64 bits. Windows peut afficher un avertissement SmartScreen (l'exécutable n'est pas signé) : « Informations complémentaires » puis « Exécuter quand même ». La somme SHA-256 du zip est donnée dans la page de la version.

Facultatif : installez [VLC](https://www.videolan.org) (3.x, 64 bits) pour **lire les vidéos et les sons dans ExPLIA** (voir [Lecture audio et vidéo](#lecture-audio-et-vidéo--vlc-requis-facultatif)).

## Fonctionnalités

### Navigation
- Au lancement, ExPLIA ouvre le dossier de l'utilisateur courant (`C:\Users\nom`). La fenêtre s'ouvre à 80 % de l'écran, centrée, puis la taille que vous lui donnez est mémorisée.
- **Barre d'adresse** : fil d'Ariane cliquable (`Ce PC > D: > Projets > …`) pour remonter à n'importe quel niveau ; chaque mot s'illumine (fond et couleur) au survol. Un clic sur l'espace libre permet de saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC` ; Entrée valide, Échap annule. Les barres du haut sont agrandies pour être plus lisibles.
- **Barre latérale en « classeur »** : la liste de l'onglet actif est à gauche, les onglets (intercalaires) sont sur son bord droit :
  - **Ce PC** : les lecteurs détectés (`C:`, `D:`, …) avec leur nom de volume et, sous chacun, une **barre d'occupation** avec le pourcentage à l'intérieur (verte, orange à partir de 75 %, rouge à partir de 90 %). L'infobulle donne le type, le format, le total, l'espace utilisé et libre. **Option > Occupation des disques** masque ou affiche ces barres.
  - **Home** : le dossier de l'utilisateur et ses sous-dossiers.
  - **Favoris** : vide par défaut, uniquement les dossiers que vous ajoutez par clic droit « Ajouter aux favoris » ; clic droit « Retirer des favoris ».
  - **Réseau** : les lecteurs réseau connectés (lettres de lecteur mappées). La navigation dans les serveurs et partages réseau n'est pas prise en charge.
- **La vue centrale n'affiche jamais la liste des lecteurs** : « Ce PC » (bouton, fil d'Ariane ou saisie) ouvre l'onglet « Ce PC » de la barre latérale et laisse le dossier affiché en place.
- **Ligne `..`** : première ligne de la liste, un clic revient au dossier précédent (le parent) ; à la racine d'un lecteur, elle ouvre l'onglet « Ce PC ».
- Double-clic sur un dossier pour l'ouvrir ; **Retour arrière** ou **←** remonte au dossier parent. Voir aussi les [raccourcis clavier](#raccourcis-clavier).

### Affichages
- **Liste à colonnes** (par défaut) : nom, taille (celle des dossiers est calculée en arrière-plan : « Calcul… » en attendant ; un dossier inaccessible affiche 0 o) et date de modification. Le menu **Afficher** ajoute, en option : date de création, **dimensions** des images (PNG, JPEG, BMP, GIF, WebP), **résolution** des vidéos et **durée** des vidéos et des sons. Ces informations sont lues dans les en-têtes des fichiers, en arrière-plan, sans dépendance supplémentaire : MP4/MOV/M4A, MKV/WebM, AVI, WAV, MP3, FLAC et OGG/Opus (les autres formats affichent « — »). Les dates sont à l'heure locale.
- **▥ Colonnes** (barre d'options, au-dessus de l'aperçu) : navigation « style Mac », une colonne par niveau du chemin, de largeur adaptée aux noms, la dernière occupant la place restante. Un clic sur un dossier l'ouvre dans la colonne suivante.
- **🖼 Images** : visionnage rapide, grille de miniatures du dossier (décodées au fur et à mesure). Les quatre flèches se déplacent dans la grille, Entrée ouvre.
- Un second clic sur ▥ ou 🖼 revient à la liste. Les mêmes choix existent dans le menu **Afficher**, qui contient aussi **Dossiers et fichiers cachés** (masqués par défaut, atténués quand on les affiche).
- **Trier** (nom, taille, type, date de modification ; croissant ou décroissant) et **Extension** (`Tout`, `Custom` pour saisir vos propres extensions, par exemple `mkv, pdf`, ou les catégories `Image`, `Documents`, `Code` ; les dossiers restent visibles).
- **Option** : « Pas de calcul de taille de dossier », utile sur les gros disques, et « Occupation des disques » (barre et pourcentage dans l'onglet « Ce PC »).

### Aperçu et lecture
- **Images** PNG, JPEG, BMP, GIF (première image) et WebP ; **texte** courant (`.txt`, `.md`, `.json`, `.csv`, `.rs`, `.log`, …, 64 Ko affichés au maximum) ; **contenu des archives `.zip`** (liste des fichiers, rien n'est extrait). Les autres types (PDF, Office…) affichent leur type et leur taille avec un bouton « Ouvrir ».
- La flèche `<` devant « Aperçu » agrandit le volet pour tout voir d'un coup, `>` le réduit ; le menu **Afficher** permet de le masquer.
- **Lecture audio et vidéo intégrée** (⚠ **VLC requis**, voir ci-dessous) : « ▶ Lire », double-clic ou Entrée sur un fichier audio ou vidéo le lit dans le volet, avec lecture / pause (bouton ou **Espace**), arrêt, barre de position et volume. « ↗ Lecteur externe » (ou clic droit) l'ouvre dans l'application par défaut de Windows.

### Sélection et actions
- **Sélection multiple** : **clic maintenu** (0,5 s) sur une ligne pour démarrer, puis un clic sur chaque ligne à ajouter ou retirer ; **Ctrl+clic** ajoute ou retire une ligne (liste et colonnes « style Mac ») ; **Maj+clic** (ou **Maj+↑/↓**) sélectionne la plage. Un seul type à la fois : uniquement des dossiers, ou uniquement des fichiers.
- **Volet « Actions »** : un clic sur un dossier (ou une multi-sélection) transforme le volet « Aperçu » en volet « Actions » ; sur un fichier, les boutons s'ajoutent sous l'aperçu.
- **Renommer** : **F2**, bouton « Renommer » ou clic droit, directement dans la liste (Entrée valide, Échap annule ; l'extension n'est pas sélectionnée). Les noms interdits par Windows sont refusés et un élément existant n'est jamais écrasé.
- **Supprimer** : **Suppr** ou bouton « Supprimer », vers la **corbeille de Windows** (récupérable). Le bouton devient « Confirmer » **à la même place** (un second clic au même endroit valide ; « Annuler » est juste à côté) ; au clavier, **Suppr** puis **Entrée**, Échap annule. Un double-clic accidentel est ignoré pendant 0,6 s.
- **Ouvrir** : double-clic sur un fichier l'ouvre avec l'application par défaut de Windows. Clic droit sur un exécutable (`.exe`, `.bat`, `.cmd`, `.msi`) : « Exécuter » ou « Exécuter en tant qu'administrateur » (fenêtre UAC).

### Réglages
Vos réglages sont enregistrés automatiquement dans `%APPDATA%\ExPLIA\settings.txt` (format `clé=valeur`, modifiable à la main) : taille de la fenêtre, tri, volet d'aperçu, mode d'affichage, colonnes, éléments cachés, filtre d'extension, options (taille des dossiers, occupation des disques) et favoris.

## Raccourcis clavier

| Touche ou geste | Action |
| --- | --- |
| **↑ / ↓** | Ligne précédente / suivante (la liste défile) ; **Maj+↑/↓** étend la sélection |
| **←** | Dossier parent, en resélectionnant le dossier quitté |
| **→** | Entre dans le dossier sélectionné et sélectionne son premier élément |
| **Retour arrière** | Dossier parent (tous les affichages) |
| **Grille d'images** | ↑ ↓ ← → se déplacent dans la grille, **Entrée** ouvre |
| **F2** | Renomme l'élément sélectionné |
| **Suppr**, puis **Entrée** | Supprime (corbeille) ; **Échap** annule |
| **Espace** | Lecture / pause du lecteur intégré |
| **Échap** | Annule une saisie, une confirmation ou la multi-sélection |
| **Ctrl+clic**, **Maj+clic**, **clic maintenu** | Sélection multiple |
| **Double-clic** | Ouvre un dossier ; lit un fichier audio / vidéo ; ouvre un autre fichier |

## Sécurité et données

- **Aucune télémétrie, aucune connexion réseau.**
- ExPLIA ne modifie vos fichiers que sur une action explicite de votre part : **renommer** (dans le même dossier, sans écraser) et **envoyer à la corbeille** (après confirmation). Il ne supprime jamais définitivement de lui-même, et la copie et le déplacement de fichiers ne sont pas disponibles.
- Il ouvre un fichier avec son application par défaut ou lance un exécutable (y compris « en tant qu'administrateur » : Windows demande alors sa confirmation UAC) uniquement sur votre geste.
- Le seul fichier qu'il écrit de lui-même est `%APPDATA%\ExPLIA\settings.txt`.

## Lecture audio et vidéo : VLC requis (facultatif)

ExPLIA n'embarque aucun lecteur. Pour lire les vidéos et les sons **dans ExPLIA**, il utilise la bibliothèque `libvlc.dll` d'un **VLC installé sur la machine** :

- **VLC 3.x en 64 bits** (téléchargement : <https://www.videolan.org>). VLC en 32 bits et VLC 4 ne sont pas pris en charge.
- ExPLIA cherche VLC dans l'ordre : (1) le dossier indiqué par la variable d'environnement `EXPLIA_VLC_DIR` ; (2) un dossier `vlc` placé à côté de `explia.exe` (VLC « portable ») ; (3) `C:\Program Files\VideoLAN\VLC`.
- **Sans VLC**, tout le reste fonctionne : « ▶ Lire » ouvre simplement le fichier avec l'application par défaut de Windows et un message indique comment obtenir la lecture intégrée.
- ExPLIA ne redistribue pas VLC (il le charge à la demande, en mémoire, au premier fichier lu) et ne lui transmet que des fichiers locaux : aucun flux réseau.

## Limites connues

- Préversion (0.1.0), testée sur Windows 11 uniquement ; exécutable non signé.
- Lecture intégrée vérifiée avec une vidéo AVI ; MP4 et MKV réels, et le son avec réglage du volume, ne sont pas encore testés.
- La ligne `..` n'existe que dans la liste (ni dans les colonnes, où les niveaux précédents sont visibles, ni dans la grille d'images : utilisez Retour arrière) ; Ctrl+clic fonctionne dans la liste et les colonnes, pas dans la grille d'images.
- Pas encore disponible : copier / couper / coller et déplacer, créer un dossier ou un fichier, recherche, onglets, historique précédent / suivant.

## Pour les développeurs

### Prérequis pour compiler

- Windows 10 ou 11, x86_64.
- [Rust stable](https://rustup.rs) (édition 2024, Rust 1.85 ou plus récent).
- Toolchain `x86_64-pc-windows-msvc` : installer les *Visual Studio Build Tools* avec la charge de travail « Développement Desktop en C++ » (inclut le SDK Windows).
- Facultatif, pour la lecture audio et vidéo intégrée : **VLC 3.x 64 bits**.

### Compilation et exécution

```bash
cargo run --release
```

L'exécutable est produit dans `target\release\explia.exe`. Le fichier `.cargo/config.toml` du dépôt lie le runtime C de Microsoft **statiquement** (`-C target-feature=+crt-static`) : `explia.exe` fonctionne donc sur un Windows qui n'a pas le « Visual C++ Redistributable », sans autre action de votre part. Ne définissez pas la variable d'environnement `RUSTFLAGS` en compilant, car elle remplace ce réglage.

### Vérifications

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

### Organisation du code

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Point d'entrée, fenêtre `eframe`. |
| `src/app.rs` | Interface : panneaux, barre d'adresse, vues liste / colonnes / grille, volets d'aperçu et d'actions, clavier. |
| `src/fs_nav.rs` | Entrées de dossier (nom, taille, dates, caché), listage, fil d'Ariane, analyse d'adresse, taille d'un dossier. |
| `src/view.rs` | Tri, filtre par extension, colonnes, modes d'affichage, sélection d'une plage, calculs de défilement. |
| `src/favorites.rs` | Liste des favoris. |
| `src/settings.rs` | Fichier de réglages (`settings.txt`) : lecture, écriture, valeurs par défaut. |
| `src/player.rs` | Lecture audio / vidéo intégrée : chargement dynamique de libVLC (code `unsafe` isolé ici). |
| `src/media.rs` | Lecture des en-têtes multimédia : dimensions, résolution, durée. |
| `src/dates.rs` | Dates à l'heure locale et durées. |
| `src/fileops.rs` | Renommage : validation du nom Windows, refus d'écraser. |
| `src/trash.rs` | Envoi à la corbeille (`SHFileOperationW` avec `FOF_ALLOWUNDO`). |
| `src/launch.rs` | Ouverture d'un fichier / lancement d'un exécutable (`ShellExecuteW`, UAC). |
| `src/places.rs` | Lecteurs logiques, capacité / occupation / nom / format des lecteurs, dossier de l'utilisateur (API Windows). |
| `src/preview.rs` | Aperçus : images, texte, archives ZIP, miniatures. |
| `.cargo/config.toml` | Liaison statique du runtime C. |
