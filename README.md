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
- Au lancement, ExPLIA ouvre le dossier de l'utilisateur courant (`C:\Users\nom`). La vue « Ce PC » liste les lecteurs détectés (`C:`, `D:`, …) avec leur taille.
- Colonne « Taille » : taille des fichiers et, par défaut, des dossiers (calculée en arrière-plan, « Calcul… » en attendant ; un dossier inaccessible affiche 0 o).
- Navigation : double-clic sur un dossier pour l'ouvrir, « Dossier parent », « Ce PC ».
- Barre d'adresse : fil d'Ariane cliquable (`Ce PC > D: > Projets > …`) pour remonter à n'importe quel niveau. Un clic sur l'espace libre (ou sur le crayon) permet de saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC` ; Entrée valide, Échap annule.
- Colonnes du tableau (menu **Afficher**) : taille et date de modification par défaut ; en option, date de création, **dimensions** des images (PNG, JPEG, BMP, GIF, WebP), **résolution** des vidéos et **durée** des vidéos et des sons. Ces informations sont lues dans les en-têtes des fichiers, en arrière-plan, sans dépendance supplémentaire : MP4/MOV/M4A, MKV/WebM, AVI, WAV, MP3, FLAC et OGG/Opus. Les autres formats affichent « — ». Les dates sont à l'heure locale.
- Barre d'options (au-dessus de l'aperçu) : **▥ Colonnes** (navigation « style Mac » : une colonne par niveau du chemin, de largeur adaptée aux noms, la dernière occupant la place restante ; un clic sur un dossier l'ouvre dans la colonne suivante) et **🖼 Images** (visionnage : grille de miniatures du dossier ; les quatre flèches se déplacent dans la grille, Entrée ouvre). Un second clic revient à la liste. Les mêmes choix existent dans le menu **Afficher**, qui contient aussi **Dossiers et fichiers cachés** (masqués par défaut, atténués quand on les affiche). **Retour arrière** remonte au dossier parent.
- Menus : **Trier** (nom, taille, type, date de modification ; croissant ou décroissant), **Afficher** (colonnes, volet d'aperçu, mode d'affichage), **Extension** (`Tout`, `Custom` pour saisir vos propres extensions, par exemple `mkv, pdf`, ou les catégories `Image`, `Documents`, `Code` ; les dossiers restent visibles), **Option** (« Pas de calcul de taille de dossier », utile sur les gros disques). Tous ces choix sont mémorisés.
- Multi-sélection et suppression : **clic maintenu** (0,5 s) sur une ligne pour démarrer la sélection, puis un clic sur chaque ligne à ajouter ou retirer. **Ctrl+clic** ajoute ou retire une ligne ; **Maj+clic** sélectionne toute la plage entre le dernier élément choisi et celui sur lequel vous cliquez. Un seul type à la fois : uniquement des dossiers, ou uniquement des fichiers. Le volet d'aperçu devient un volet « Actions » : **Supprimer** (envoi à la corbeille, avec confirmation) ou **Désélectionner** (Échap fonctionne aussi).
- Actions sur un élément : un clic sur un **dossier** transforme le volet « Aperçu » en volet « Actions » (**Renommer**, **Supprimer**) ; sur un fichier, ces boutons s'ajoutent sous l'aperçu. **F2**, le bouton « Renommer » ou le clic droit puis « Renommer » permettent de renommer directement dans la liste (Entrée valide, Échap annule ; l'extension n'est pas sélectionnée). Au clavier : **Suppr** puis **Entrée** supprime l'élément sélectionné (Échap annule). **Suppr** ou le bouton « Supprimer » demande la suppression : le bouton devient « Confirmer » **à la même place** (un second clic au même endroit valide ; « Annuler » est juste à côté). Un double-clic accidentel est ignoré pendant 0,6 s.
- Navigation aux flèches : **↑ / ↓** déplacent la sélection (la liste défile pour la garder visible ; **Maj+↑/↓** étend la sélection) ; **←** remonte au dossier parent en resélectionnant le dossier quitté ; **→** entre dans le dossier sélectionné et sélectionne son premier élément. Dans la vue « Ce PC », ↑ / ↓ choisissent un lecteur et → l'ouvre.
- **Lecture audio et vidéo intégrée** (⚠ **VLC requis**, voir ci-dessous) : un clic sur « ▶ Lire », un double-clic ou Entrée sur un fichier audio ou vidéo le lit directement dans le volet d'aperçu, avec lecture / pause (bouton ou **Espace**), arrêt, barre de position et volume. Agrandissez le volet avec la flèche `<` pour une image plus grande. « ↗ Lecteur externe » (ou clic droit) ouvre le fichier dans l'application par défaut de Windows.
- Ouvrir : double-clic sur un document ou un autre fichier pour l'ouvrir avec l'application par défaut de Windows. Clic droit sur un exécutable (`.exe`, `.bat`, `.cmd`, `.msi`) : « Exécuter » ou « Exécuter en tant qu'administrateur » (fenêtre UAC). Le volet d'aperçu propose les mêmes boutons.
- Barre latérale : **Favoris** (vide par défaut : uniquement les dossiers que vous ajoutez), puis la section **Ce PC** (un clic ouvre la vue des lecteurs) avec la liste des lecteurs. Clic droit sur un dossier de la liste : « Ajouter aux favoris » ; clic droit sur un de vos favoris : « Retirer des favoris ».
- Aperçu (décodé hors du thread d'interface) : images PNG, JPEG, BMP, GIF (première image) et WebP ; fichiers texte courants (`.txt`, `.md`, `.json`, `.csv`, `.rs`, `.log`, …, 64 Ko affichés au maximum) ; contenu des archives `.zip` (liste des fichiers, rien n'est extrait) ; pour les autres fichiers (PDF, Office, vidéos…), type et taille. La flèche `<` devant « Aperçu » agrandit le volet pour tout voir d'un coup, `>` le réduit.

## Prérequis

- Windows 10 ou 11, x86_64.
- [Rust stable](https://rustup.rs) (édition 2024, Rust 1.85 ou plus récent).
- Toolchain `x86_64-pc-windows-msvc` : installer les *Visual Studio Build Tools* avec la charge de travail « Développement Desktop en C++ » (inclut le SDK Windows).
- Facultatif, pour la lecture audio et vidéo intégrée : **VLC 3.x 64 bits** (voir la section suivante).

## Lecture audio et vidéo : VLC requis (facultatif)

ExPLIA n'embarque aucun lecteur. Pour lire les vidéos et les sons **dans ExPLIA**, il utilise la bibliothèque `libvlc.dll` d'un **VLC installé sur la machine** :

- **VLC 3.x en 64 bits** (téléchargement : <https://www.videolan.org>). VLC en 32 bits et VLC 4 ne sont pas pris en charge.
- ExPLIA cherche VLC dans l'ordre : (1) le dossier indiqué par la variable d'environnement `EXPLIA_VLC_DIR` ; (2) un dossier `vlc` placé à côté de `explia.exe` (VLC « portable ») ; (3) `C:\Program Files\VideoLAN\VLC`.
- **Sans VLC**, tout le reste fonctionne : « ▶ Lire » ouvre simplement le fichier avec l'application par défaut de Windows et un message indique comment obtenir la lecture intégrée.
- ExPLIA ne redistribue pas VLC (il le charge à la demande, en mémoire, au premier fichier lu) et ne lui transmet que des fichiers locaux : aucun flux réseau.

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
| `src/player.rs` | Lecture audio / vidéo intégrée : chargement dynamique de libVLC (code `unsafe` isolé ici). |
| `src/media.rs` | Lecture des en-têtes multimédia : dimensions, résolution, durée. |
| `src/dates.rs` | Dates à l'heure locale et durées. |
| `src/fileops.rs` | Renommage : validation du nom Windows, refus d'écraser. |
| `src/trash.rs` | Envoi à la corbeille (`SHFileOperationW` avec `FOF_ALLOWUNDO`). |
| `src/launch.rs` | Ouverture d'un fichier / lancement d'un exécutable (`ShellExecuteW`, UAC). |
| `src/places.rs` | Lecteurs logiques et dossiers usuels (API Windows). |
| `src/preview.rs` | Décodage des images et miniatures. |
