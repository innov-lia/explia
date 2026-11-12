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
- **Barre d'adresse** (en haut de la zone centrale, avec la recherche) : fil d'Ariane cliquable (`Ce PC > D: > Projets > …`) pour remonter à n'importe quel niveau ; dans votre dossier utilisateur, le chemin commence par `Home` (`Ce PC > Home > Videos`) au lieu de `C: > Users > nom` ; chaque mot s'illumine (fond et couleur) au survol. Un clic sur l'espace libre permet de saisir un chemin Windows (`C:\Users`, `D:`) ou `Ce PC` ; Entrée valide, Échap annule. Les barres sont agrandies pour être plus lisibles ; la barre de menus du haut contient le bouton **Ce PC** (ouvre l'onglet des lecteurs), **Afficher** et **Option**.
- **Recherche** (à droite de la barre d'adresse, **Ctrl+F** pour y aller, **Échap** pour l'effacer) : filtre en direct les fichiers **et dossiers** du dossier affiché. Un mot est cherché dans le nom (`rapport`, `.exe`) ; `*` et `?` sont des jokers sur le nom entier (`*.exe`, `img_??.png`) ; plusieurs motifs se séparent par `,` ou `;` (`*.mkv, *.avi`). Majuscules et minuscules sont indifférentes. Avec la case **Récursive** (à gauche de la barre de recherche, **cochée par défaut**), la recherche parcourt aussi les sous-dossiers, en arrière-plan : les résultats (5 000 au plus) apparaissent au fur et à mesure dans la liste, avec leur dossier d'origine, jusqu'au bouton « ✕ Fermer » ; décochée, elle filtre seulement le dossier affiché. Les liens et jonctions ne sont jamais suivis. La recherche s'efface quand on change de dossier.
- **Barre latérale en « classeur »** : la liste de l'onglet actif est à gauche, les onglets (intercalaires) sont sur son bord droit :
  - **Ce PC** : les lecteurs détectés (`C:`, `D:`, …) avec leur nom de volume et, sous chacun, une **barre d'occupation** avec le pourcentage à l'intérieur (verte, orange à partir de 75 %, rouge à partir de 90 %). L'infobulle donne le type, le format, le total, l'espace utilisé et libre. **Option > Occupation des disques** masque ou affiche ces barres.
  - **Home** : le dossier de l'utilisateur et ses sous-dossiers.
  - **Favoris** : vide par défaut, uniquement les dossiers que vous ajoutez par clic droit « Ajouter aux favoris » ; clic droit « Retirer des favoris ».
  - **Applications** : ExPLIA lui-même en tête, puis les applications que vous ajoutez (clic droit sur un exécutable `.exe`, `.bat`, `.cmd`, `.msi` ou `.com` > « Ajouter aux applications »). Un clic lance l'application ; clic droit : lancer en administrateur (fenêtre UAC), afficher dans son dossier, retirer des applications.
  - **Réseau** : les lecteurs réseau connectés (lettres de lecteur mappées). La navigation dans les serveurs et partages réseau n'est pas prise en charge.
- **La vue centrale n'affiche jamais la liste des lecteurs** : « Ce PC » (bouton, fil d'Ariane ou saisie) ouvre l'onglet « Ce PC » de la barre latérale et laisse le dossier affiché en place.
- **Ligne `..`** : première ligne de la liste, un clic revient au dossier précédent (le parent) ; à la racine d'un lecteur, elle ouvre l'onglet « Ce PC ».
- Double-clic sur un dossier pour l'ouvrir ; **Retour arrière** ou **←** remonte au dossier parent ; au plus haut d'un lecteur, **←** passe la main à la liste des lecteurs (onglet « Ce PC »), où **↑ / ↓** changent de lecteur et **→** revient au dossier. Voir aussi les [raccourcis clavier](#raccourcis-clavier).

### Affichages
- **Liste à colonnes** (par défaut) : nom, taille (celle des dossiers est calculée en arrière-plan : « Calcul… » en attendant ; un dossier inaccessible affiche 0 o) et date de modification. Le menu **Afficher** ajoute, en option : date de création, **dimensions** des images (PNG, JPEG, BMP, GIF, WebP), **résolution** des vidéos et **durée** des vidéos et des sons. Ces informations sont lues dans les en-têtes des fichiers, en arrière-plan, sans dépendance supplémentaire : MP4/MOV/M4A, MKV/WebM, AVI, WAV, MP3, FLAC et OGG/Opus (les autres formats affichent « — »). Les dates sont à l'heure locale.
- **▥ Colonnes** (barre d'options, au-dessus de l'aperçu) : navigation « style Mac », une colonne par niveau du chemin, toutes de **largeur fixe** (environ 25 caractères : un nom plus long est coupé par « … »), la dernière occupant la place restante. Un clic sur un dossier l'ouvre dans la colonne suivante. La dernière colonne (le dossier courant) affiche, avec leur en-tête, les mêmes informations que la liste : taille, dates, dimensions, durée… selon les colonnes cochées dans **Afficher**. Mêmes fonctions que la liste : Ctrl+clic / Maj+clic, **F2** ou clic droit pour renommer, clic droit pour ajouter aux favoris (ou ouvrir / lire / exécuter un fichier).
- **🖼 Images** et **🎬 Vidéos** : visionnage rapide, trois zones toujours visibles côte à côte : **les lecteurs** (avec leur occupation ; masqués quand l'onglet « Ce PC » de la barre latérale les affiche déjà), **une colonne de navigation** (le dossier de la barre d'adresse, puis `..` et ses sous-dossiers) et **la grille** : uniquement les images (miniatures décodées au fur et à mesure), ou uniquement les vidéos, du dossier courant. Les vidéos sont représentées par une icône (pas de miniature) ; double-clic ou Entrée les lit. Les quatre flèches se déplacent dans la grille, Entrée ouvre. **Dans votre dossier utilisateur (Home)**, les colonnes des lecteurs et de navigation sont masquées (le chemin et l'onglet Home de la barre latérale suffisent) : la grille occupe toute la place et les sous-dossiers y figurent comme tuiles, pour pouvoir y entrer.
- Un second clic sur ▥, 🖼 ou 🎬 revient à la liste. Les mêmes choix existent dans le menu **Afficher**, qui contient aussi **Dossiers et fichiers cachés** (masqués par défaut, atténués quand on les affiche). Sont considérés comme cachés les éléments ayant l'attribut « caché » de Windows **et ceux dont le nom commence par un point** (`.git`, `.config`, …).
- **Tri par clic sur un en-tête** : un clic sur « Nom », « Créé le », « Modifié le » ou « Taille » (liste, et dernière colonne de la vue en colonnes) trie dessus ; un second clic inverse l'ordre ; une flèche ▲ / ▼ marque le tri actif. « Créé le » n'est pas affiché par défaut (cochez-le dans **Afficher**) ; dimensions et durée ne sont pas triables.
- **Option** : « Pas de calcul de taille de dossier », utile sur les gros disques, et « Occupation des disques » (barre et pourcentage dans l'onglet « Ce PC »).

### Aperçu et lecture
- **Images** PNG, JPEG, BMP, GIF (première image) et WebP ; **texte** courant (`.txt`, `.md`, `.json`, `.csv`, `.rs`, `.log`, …, 64 Ko affichés au maximum) ; **contenu des archives `.zip`** (liste des fichiers, rien n'est extrait). Les autres types (PDF, Office…) affichent leur type et leur taille avec un bouton « Ouvrir ».
- Le volet de détail (aperçu d'un fichier, actions sur un dossier ou une sélection) n'a plus de titre : sa barre du haut porte la flèche `<` (agrandit le volet pour tout voir d'un coup, `>` le réduit) puis les boutons **Colonnes**, **Images** et **Vidéos**. Le menu **Afficher** permet de le masquer. Le volet ne peut pas être plus étroit que cette barre (flèche et trois boutons sur une seule ligne), et la fenêtre fait au moins 900 × 450.
- **Disposition** (menu **Afficher** > Disposition) : la barre latérale, ses onglets et le volet de détail se placent chacun à **gauche ou à droite** à votre convenance ; du même côté, la barre latérale est au plus près du bord de la fenêtre. Le choix est mémorisé.
- **Lecture audio et vidéo intégrée** (⚠ **VLC requis**, voir ci-dessous) : « ▶ Lire », double-clic ou Entrée sur un fichier audio ou vidéo le lit dans le volet, avec lecture / pause (bouton ou **Espace**), arrêt, barre de position et volume. « ↗ Lecteur externe » (ou clic droit) l'ouvre dans l'application par défaut de Windows.

### Sélection et actions
- **Sélection multiple** : **clic maintenu** (0,5 s) sur une ligne pour démarrer, puis un clic sur chaque ligne à ajouter ou retirer ; **Ctrl+clic** ajoute ou retire une ligne (liste, colonnes « style Mac » et grilles) ; **Maj+clic** (ou **Maj+↑/↓**) sélectionne la plage. Un seul type à la fois : uniquement des dossiers, ou uniquement des fichiers.
- **Menu « Sélection »** (barre de menus, entre **Ce PC** et **Afficher** ; il respecte l'option **Récursive**) :
  - **📂 Dossiers vides** : liste puis **sélectionne** tous les dossiers sans aucun fichier (des sous-dossiers vides ne comptent pas) ; seuls les plus hauts sont listés. Il ne reste qu'à cliquer sur « Supprimer » (corbeille, avec confirmation). Non récursif : seulement les sous-dossiers directs.
  - **📑 Doublons > [50] Mo** : liste les fichiers de même contenu (même taille puis même empreinte 64 bits du contenu) plus gros que le seuil, modifiable dans le menu (50 Mo par défaut). Les copies sont groupées (`#1`, `#2`…) avec leur dossier d'origine ; « Sélectionner les copies » garde le premier fichier de chaque groupe et sélectionne les autres. Rien n'est supprimé sans votre confirmation.
- **Emplacement d'un résultat** (volet de détail, quand on sélectionne un doublon ou un résultat de recherche) : le dossier qui le contient, puis chaque dossier parent jusqu'au dossier affiché, chacun avec **↗ Ouvrir** (on arrive dans ce dossier, résultats fermés, le fichier choisi ; `..` permet de remonter) et **🗑** (supprime ce dossier entier : corbeille, avec confirmation). Pratique pour éliminer un dossier complet de doublons.
- **Supprimer le dossier affiché** : quand rien n'est sélectionné, le volet de détail propose « Supprimer ce dossier » (corbeille, avec confirmation) ; ExPLIA remonte ensuite au dossier parent.
- **Volet « Actions »** : un clic sur un dossier (ou une multi-sélection) transforme l'aperçu du volet de détail en actions ; sur un fichier, les boutons s'ajoutent sous l'aperçu.
- **Renommer** : **F2**, bouton « Renommer » ou clic droit, directement dans la liste ou la dernière colonne de la vue en colonnes (Entrée valide, Échap annule ; l'extension n'est pas sélectionnée). Les noms interdits par Windows sont refusés et un élément existant n'est jamais écrasé.
- **Supprimer** : **Suppr** ou bouton « Supprimer », vers la **corbeille de Windows** (récupérable). Le bouton devient « Confirmer » **à la même place** (un second clic au même endroit valide ; « Annuler » est juste à côté) ; au clavier, **Suppr** puis **Entrée**, Échap annule. Un double-clic accidentel est ignoré pendant 0,6 s.
- **Ouvrir** : double-clic sur un fichier l'ouvre avec l'application par défaut de Windows. Clic droit sur un exécutable (`.exe`, `.bat`, `.cmd`, `.msi`) : « Exécuter » ou « Exécuter en tant qu'administrateur » (fenêtre UAC).

### Réglages
Vos réglages sont enregistrés automatiquement dans `%APPDATA%\ExPLIA\settings.txt` (format `clé=valeur`, modifiable à la main) : taille de la fenêtre, tri, volet d'aperçu, disposition des blocs, mode d'affichage, colonnes, éléments cachés, options (taille des dossiers, occupation des disques, récursivité, seuil des doublons), favoris et applications.

## Raccourcis clavier

| Touche ou geste | Action |
| --- | --- |
| **↑ / ↓** | Ligne précédente / suivante (la liste défile) ; **Maj+↑/↓** étend la sélection |
| **←** | Dossier parent, en resélectionnant le dossier quitté ; au plus haut d'un lecteur : liste des lecteurs |
| **↑ / ↓** (liste des lecteurs) | Lecteur précédent / suivant, ouvert aussitôt ; **→** revient au dossier |
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
- La ligne `..` existe dans la liste et dans la colonne de navigation de la grille d'images (pas dans la vue en colonnes, où les niveaux précédents sont visibles) ; Ctrl+clic et Maj+clic fonctionnent dans la liste, les colonnes et les grilles. Dans la grille, **←** déplace la sélection : utilisez Retour arrière pour remonter.
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
| `src/scan.rs` | Parcours en lecture seule : recherche récursive, dossiers vides, doublons (empreintes). |
| `src/places.rs` | Lecteurs logiques, capacité / occupation / nom / format des lecteurs, dossier de l'utilisateur (API Windows). |
| `src/preview.rs` | Aperçus : images, texte, archives ZIP, miniatures. |
| `.cargo/config.toml` | Liaison statique du runtime C. |
