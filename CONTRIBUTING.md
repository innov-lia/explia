# Contribuer à ExPLIA

Merci de votre intérêt ! ExPLIA est un explorateur de fichiers Windows écrit en Rust (`egui` / `eframe`). Toute aide est bienvenue : signaler un bug, proposer une idée, améliorer la documentation ou le code.

## Signaler un bug ou proposer une idée

Ouvrez une [issue](https://github.com/innov-lia/explia/issues/new/choose) avec le modèle adapté. Pour un bug, indiquez comment le reproduire, votre version de Windows et, pour la lecture vidéo, si VLC est installé. Pensez à masquer vos informations personnelles sur les captures.

## Proposer du code

1. Créez une branche depuis `main` avec un nom explicite (`feature/…`, `fix/…`, `docs/…`).
2. Faites des changements petits et cohérents, avec des messages de commit clairs.
3. Avant d'ouvrir la Pull Request, lancez :

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. Mettez à jour le README si votre changement est visible par l'utilisateur.
5. Ouvrez une Pull Request vers `main` en décrivant les changements, les tests exécutés et les limites éventuelles.

## Principes du projet

- **Prudence avec les fichiers** : jamais d'écrasement, jamais de suppression définitive (la corbeille, après confirmation), aucun déplacement.
- **Aucune télémétrie ni connexion réseau.**
- Pas de blocage de l'interface : les opérations longues (tailles, parcours, copies, miniatures) se font en arrière-plan.
- Le code `unsafe` (API Windows, libVLC, OLE) reste isolé dans son module et commenté (`// SAFETY : …`).
- Des tests pour toute logique testable sans interface.

Les règles détaillées du projet sont dans [AGENTS.md](AGENTS.md).
