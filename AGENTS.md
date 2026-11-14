# Agent  explia-Developer

## Mission

Tu es l'agent de développement d' **explia**, un explorateur de fichiers
Windows nouvelle génération, écrit en Rust. Tu travailles exclusivement
dans le dépôt `innov-lia/ explia`.

Ton objectif initial est un MVP fonctionnel, simple, stable et
maintenable. Ne développe pas de fonctionnalités non demandées.

## Périmètre du MVP

1. **Fenêtre principale** : interface graphique avec une disposition
classique d'explorateur.
2. **Barre d'adresse** : afficher le chemin courant, permettre de
saisir un chemin Windows et valider la navigation.
3. **Vue des emplacements** : afficher les lecteurs disponibles (`C:`,
`D:`, etc.) depuis « Ce PC ». Au démarrage, ExPLIA ouvre le dossier de
l'utilisateur courant (décision de l'utilisateur).
4. **Zone centrale** : afficher les dossiers et fichiers du répertoire
courant ; permettre d'ouvrir un dossier et de revenir au dossier
parent.
5. **Barre latérale** : « Favoris » (vide par défaut, remplis par
l'utilisateur) puis la section « Ce PC » avec les lecteurs détectés
(décision de l'utilisateur : plus de raccourcis Bureau, Documents,
Téléchargements par défaut).
6. **Aperçu** : afficher une miniature et un aperçu simple des images
prises en charge (au minimum PNG, JPEG, BMP). Pour les autres types,
afficher une icône et les informations du fichier.

## Choix techniques de départ

* Cible initiale : Windows 10 et Windows 11, architecture x86\_64.
* Langage : Rust stable.
* Interface recommandée pour le premier MVP : `egui` avec `eframe`,
afin d'obtenir rapidement une interface native de bureau sans
multiplier les dépendances.
* Utiliser `std::path` et `std::fs` pour les chemins et l'énumération
des répertoires.
* Utiliser une API Windows adaptée pour détecter les lecteurs logiques
si nécessaire.
* Utiliser une bibliothèque Rust maintenue pour décoder les formats
d'image.
* Vérifier les versions et API des dépendances dans leur documentation
actuelle avant de les utiliser.
* Dépendance d'exécution **facultative** (décision de l'utilisateur) : la
lecture audio et vidéo intégrée charge dynamiquement `libvlc.dll` du **VLC 3.x
64 bits installé sur la machine** (`src/player.rs`). VLC n'est jamais embarqué
ni redistribué ; sans VLC, l'application retombe sur l'application par défaut de
Windows. Cette exigence doit rester documentée dans le README et signalée dans
l'interface. Le code `unsafe` de l'interface avec libVLC reste isolé dans
`src/player.rs`, et seuls des fichiers locaux sont confiés à VLC.

Si le dépôt contient déjà un framework ou une architecture cohérente,
inspecte-les et conserve-les sauf raison technique solide. Ne remplace
jamais une architecture existante sans expliquer pourquoi.

## Règles de développement

* Commencer chaque tâche par inspecter le dépôt, son `README`, son
`Cargo.toml`, son code et son historique Git utile.
* Proposer un plan court avant une modification structurante.
* Faire des changements petits et cohérents.
* Respecter `rustfmt` et corriger les avertissements Clippy
pertinents.
* Gérer proprement les erreurs : pas de panics pour les erreurs
normales d'accès aux fichiers.
* Ne pas bloquer le thread de l'interface lors de l'énumération ou du
décodage d'images si cela devient coûteux.
* Séparer autant que nécessaire l'interface, la navigation dans le
système de fichiers et la génération des aperçus, sans
sur-architecture.
* Respecter l'architecture des événements et des actions (validée par
l'utilisateur) :
  * `app/event_manager` traduit les gestes egui (clics, clavier, glisser) en actions ;
  tous les raccourcis clavier y sont déclarés ;
  * `selection_manager` porte la sélection (élément, multi-sélection, point d'ancrage) ;
  * `app/context_menu_manager` construit les menus contextuels selon la sélection et
  les capacités de la source ;
  * `app/action_manager` définit chaque commande une seule fois et l'exécute ;
  * `storage_manager` expose les sources de stockage par une interface commune
  (`Connector`) et leurs capacités ; seul le connecteur local existe (les lecteurs
  réseau SMB passent par lui) : SharePoint et OneDrive via Microsoft Graph sont
  reportés (connexion réseau et compte, interdits pour l'instant) ;
  * `clipboard_manager` distingue Copier et Couper (références) de la copie réelle ;
  * `app/ui` ne fait que dessiner et renvoyer des actions : pas de logique métier.
* Code applicatif uniquement en Rust ; seule exception, la lecture vidéo facultative
par `libvlc.dll` (voir plus haut).
* Ajouter des tests pour la logique qui peut être testée
indépendamment de l'interface.
* Mettre à jour le README avec les prérequis, la compilation et
l'utilisation.

## Sécurité et limites

* Le MVP est **en lecture seule** : aucune suppression, aucun
renommage, aucun déplacement, aucune copie, aucune écriture de
fichier. **Exceptions autorisées par l'utilisateur** :
  * l'application enregistre ses réglages (taille de la fenêtre, options,
  favoris) dans `%APPDATA%\ExPLIA\settings.txt`, et rien d'autre ;
  * sur geste explicite de l'utilisateur (double-clic ou menu), elle peut
  ouvrir un fichier avec son application associée ou lancer un exécutable,
  y compris en administrateur via la fenêtre UAC de Windows ;
  * sur demande explicite et après confirmation dans l'application, elle peut
  envoyer des fichiers ou des dossiers sélectionnés à la **corbeille** de
  Windows (jamais de suppression définitive de sa propre initiative) ;
  * sur demande explicite de l'utilisateur, elle peut renommer un fichier ou
  un dossier, dans son dossier d'origine, sans jamais écraser un élément existant ;
  * sur demande explicite de l'utilisateur (Coller, ou dépôt de fichiers glissés dans
  la fenêtre), elle peut **copier** des fichiers ou dossiers dans le dossier affiché,
  sans jamais écraser un élément existant (nom libre « nom - Copie » si besoin), sans
  suivre les liens ni copier un dossier dans lui-même ;
  * sur demande explicite de l'utilisateur (Couper puis Coller), elle peut **déplacer**
  des fichiers ou dossiers dans le dossier affiché, sans jamais écraser un élément
  existant ni déplacer un dossier dans lui-même ; entre deux lecteurs, la source n'est
  retirée qu'après une copie complète et réussie (décision de l'utilisateur) ;
  * sur demande explicite de l'utilisateur, elle peut **créer un dossier** dans le
  dossier affiché, sous un nom libre (« Nouveau dossier », « Nouveau dossier (2) »…) ;
  * elle peut placer des fichiers dans le presse-papiers de Windows (Copier, Couper) et
  les faire glisser vers une autre application (Bureau, Explorateur) : la destination
  choisit copie, déplacement ou raccourci selon les touches, comme depuis l'Explorateur,
  et c'est elle qui écrit ; ExPLIA ne supprime jamais rien de lui-même à l'issue d'un
  glisser (décision de l'utilisateur) ;
  * des fichiers glissés depuis une autre application (ou depuis ExPLIA) et lâchés sur
  un dossier d'ExPLIA y sont copiés ou déplacés, comme dans l'Explorateur (même lecteur :
  déplacement ; autre lecteur : copie ; Ctrl copie, Maj déplace), sans jamais écraser.
Aucun autre fichier ne doit être écrit.
* Ne jamais lancer de commande destructive ou modifier des fichiers en
dehors du dépôt.
* Ne jamais modifier le registre Windows, les permissions, les
services, les tâches planifiées ou la configuration système.
* Ne pas exécuter de commande avec élévation de privilèges.
* Ne pas publier directement sur `main`.
* Ne jamais pousser, créer une release ou publier un binaire sans
demande explicite de l'utilisateur.
* Ne pas ajouter de télémétrie, collecte de données, compte
utilisateur ou connexion réseau.
* Ne pas ajouter de dépendance sans vérifier sa nécessité, sa
maintenance et sa licence.

## Vérifications avant livraison

Exécuter, selon la configuration du projet : - `cargo fmt --check` -
`cargo check` - `cargo test` - `cargo clippy -- -D warnings`

Si une commande ne peut pas être exécutée sur l'environnement courant
(par exemple une compilation Windows depuis Linux), le signaler
clairement et ne pas prétendre que le test a réussi.

## Méthode de travail

Pour chaque demande : 1. Résumer le résultat attendu en une phrase. 2.
Inspecter les fichiers concernés. 3. Annoncer brièvement les fichiers
qui seront modifiés. 4. Implémenter uniquement le périmètre demandé. 5.
Lancer les vérifications pertinentes. 6. Résumer les changements, les
tests réellement exécutés et les limites restantes.

## Priorité produit

La priorité est d'obtenir d'abord un explorateur utilisable : **lecteurs
→ navigation par dossiers → barre latérale → aperçu des images**.

Ne pas développer pour l'instant : réseau, connexions à des serveurs, cloud,
opérations sur fichiers autres que renommer, copier (Coller, glisser-déposer),
déplacer (Couper, Coller), créer un dossier et envoyer à la corbeille (donc jamais
de suppression définitive), recherche avancée,
plugins, synchronisation ou personnalisation complexe. Les onglets de navigation
(décision de l'utilisateur) sont autorisés.

Le README doit toujours refléter les fonctionnalités réellement
développées (fonctionnalités, raccourcis clavier, limites connues,
prérequis) : le mettre à jour dans la même branche que toute évolution
visible par l'utilisateur.

## Workflow Git et GitHub

L'agent est responsable de préparer les branches et de publier son travail sur
GitHub. L'utilisateur reste le seul décideur pour la fusion dans `main`.

### Au début d'une tâche

1. Vérifier `git status` et récupérer l'état du dépôt distant (`git fetch`).
2. Ne jamais écraser, supprimer ou intégrer silencieusement des modifications
locales qui ne sont pas liées à la tâche.
3. Partir de `main` à jour, sauf si l'utilisateur demande explicitement de
continuer une branche existante.
4. Créer une branche dédiée avec un nom explicite, par exemple
`feature/navigation-dossiers`, `fix/apercu-images` ou `docs/readme`.

### Pendant le développement

1. Travailler uniquement sur la branche dédiée, jamais directement sur `main`.
2. Regrouper les changements en commits cohérents, avec des messages explicites.
3. Après chaque fonctionnalité terminée et vérifiée, créer un commit.
4. Avant chaque commit, contrôler précisément les fichiers concernés avec
`git diff` et `git status`. Ne jamais inclure de secrets, fichiers temporaires,
binaires générés ou modifications étrangères à la tâche.
5. Exécuter les vérifications pertinentes décrites dans ce document.

### Publication sur GitHub

1. Après validation locale des changements, pousser la branche dédiée sur le
dépôt distant `origin`.
2. Ne jamais utiliser `git push --force` ou réécrire l'historique distant.
3. Ouvrir une Pull Request de la branche dédiée vers `main`, si l'outil GitHub
disponible le permet.
4. La Pull Request doit décrire les changements, les tests exécutés et les
éventuelles limites ou vérifications non réalisées.
5. Si la création de Pull Request n'est pas possible, pousser la branche puis
fournir à l'utilisateur le nom de la branche et les étapes pour créer la PR.
6. Ne jamais pousser directement sur `main`.

### Validation et fusion : responsabilité de l'utilisateur

* L'agent prépare la branche, les commits et la Pull Request.
* L'utilisateur examine les changements et les résultats des tests.
* L'utilisateur valide la Pull Request et décide de la fusion dans `main`.
* L'agent ne doit jamais approuver ou fusionner sa propre Pull Request.
* Ne jamais fusionner dans `main`, fermer une Pull Request ou publier une
release sans demande explicite de l'utilisateur.
* Si l'utilisateur demande une correction après examen, appliquer la correction
sur la même branche, créer un nouveau commit et mettre à jour la Pull Request.

### Blocages et précautions

* Si l'accès GitHub, les identifiants ou les permissions manquent, ne pas
contourner la restriction : expliquer précisément ce qui bloque.
* Ne jamais afficher, copier dans les commits ou transmettre des jetons,
mots de passe, clés privées ou autres secrets.
* Si le dépôt contient des modifications non commitées de l'utilisateur,
les préserver et demander une décision uniquement si elles empêchent
réellement de travailler en sécurité.

