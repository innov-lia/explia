//! Menus contextuels.
//!
//! Un menu est d'abord une liste de commandes (`MenuItem`), construite sans interface selon
//! les éléments visés (nature, nombre) et les capacités de leur source : une commande n'est
//! proposée que si elle est pertinente et permise. Il est ensuite dessiné par `show`, qui
//! renvoie l'action de la commande choisie.

use super::*;
use crate::selection_manager::{ItemKind, ItemRef};
use crate::storage_manager::Capabilities;
use action_manager::Command;

/// Ligne d'un menu contextuel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuItem {
    Command(Command),
    Separator,
}

/// Ce qui détermine le menu d'un élément de la liste.
pub(super) struct EntryMenuContext<'a> {
    /// Élément cliqué.
    pub item: &'a ItemRef,
    /// Nombre d'éléments visés (toute la multi-sélection si l'élément en fait partie).
    pub count: usize,
    /// Capacités de la source de l'élément.
    pub caps: Capabilities,
    /// La vue permet de renommer sur place (liste, dernière colonne).
    pub allow_rename: bool,
    pub in_favorites: bool,
    pub in_applications: bool,
    /// Affichage fractionné ouvert : transferts vers l'autre volet.
    pub split: bool,
}

/// Menu d'un fichier ou d'un dossier de la liste.
pub(super) fn entry_items(context: &EntryMenuContext) -> Vec<MenuItem> {
    let EntryMenuContext {
        item,
        count,
        caps,
        allow_rename,
        in_favorites,
        in_applications,
        split,
    } = *context;
    let single = count == 1;
    let is_dir = item.kind == ItemKind::Folder;
    let executable = !is_dir && launch::is_executable(&item.path);
    let mut items = Vec::new();
    let mut add = |condition: bool, command: Command| {
        if condition {
            items.push(MenuItem::Command(command));
        }
    };
    // Ouvrir.
    add(single && caps.open, Command::Open);
    add(single && is_dir && caps.list, Command::OpenInNewTab);
    add(
        single && is_playable(&item.path) && caps.open,
        Command::OpenExternally,
    );
    add(single && executable && caps.open, Command::RunAsAdmin);
    items.push(MenuItem::Separator);
    // Presse-papiers et opérations sur les fichiers.
    let mut add = |condition: bool, command: Command| {
        if condition {
            items.push(MenuItem::Command(command));
        }
    };
    add(split && caps.copy_into, Command::CopyToOtherPane);
    add(split && caps.move_items, Command::MoveToOtherPane);
    add(true, Command::Copy);
    add(caps.move_items, Command::Cut);
    add(single && is_dir && caps.copy_into, Command::Paste);
    add(single && allow_rename && caps.rename, Command::Rename);
    add(caps.delete, Command::Delete);
    add(
        single && is_dir && caps.create_folder,
        Command::CreateFolder,
    );
    add(single, Command::CopyPath);
    items.push(MenuItem::Separator);
    // Raccourcis d'ExPLIA et outils.
    let favorite = if in_favorites {
        Command::RemoveFavorite
    } else {
        Command::AddFavorite
    };
    let application = if in_applications {
        Command::RemoveApplication
    } else {
        Command::AddApplication
    };
    let mut add = |condition: bool, command: Command| {
        if condition {
            items.push(MenuItem::Command(command));
        }
    };
    add(single && is_dir, favorite);
    add(single && is_dir && caps.shell, Command::PowerShell);
    add(single && is_dir && caps.shell, Command::PowerShellAdmin);
    add(single && executable, application);
    items.push(MenuItem::Separator);
    if single && caps.properties {
        items.push(MenuItem::Command(Command::ShowProperties));
    }
    tidy(items)
}

/// Menu d'une application de l'onglet « Applications ».
pub(super) fn application_items(caps: Capabilities, removable: bool) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if caps.open {
        items.push(MenuItem::Command(Command::Open));
        items.push(MenuItem::Command(Command::RunAsAdmin));
    }
    items.push(MenuItem::Command(Command::ShowInFolder));
    if removable {
        items.push(MenuItem::Command(Command::RemoveApplication));
    }
    items
}

/// Menu d'un favori de la barre latérale.
pub(super) fn favorite_items() -> Vec<MenuItem> {
    vec![MenuItem::Command(Command::RemoveFavorite)]
}

/// Bouton d'une commande (menu, volet de détail) ; renvoie son action si elle est choisie.
pub(super) fn command_button(
    ui: &mut egui::Ui,
    command: Command,
    entry: &Entry,
    targets: &[Entry],
) -> Option<Action> {
    // Coller n'est possible que si le presse-papiers contient des fichiers.
    let enabled = command != Command::Paste || clipboard::has_files();
    let label = command.label(&entry.path, entry.is_dir, targets.len());
    let mut response = ui.add_enabled(enabled, egui::Button::new(label));
    if let Some(hint) = command.hint() {
        response = response.on_hover_text(hint);
    }
    response
        .clicked()
        .then(|| command.action(entry, targets.to_vec()))
}

/// Retire les séparateurs en tête, en fin et en double.
fn tidy(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        if item == MenuItem::Separator && out.last().is_none_or(|last| *last == MenuItem::Separator)
        {
            continue;
        }
        out.push(item);
    }
    if out.last() == Some(&MenuItem::Separator) {
        out.pop();
    }
    out
}

/// Dessine un menu pour `entry` ; `targets` : tous les éléments visés. Renvoie l'action de la
/// commande choisie.
pub(super) fn show(
    ui: &mut egui::Ui,
    entry: &Entry,
    items: &[MenuItem],
    targets: &[Entry],
) -> Option<Action> {
    let mut action = None;
    for item in items {
        let MenuItem::Command(command) = *item else {
            ui.separator();
            continue;
        };
        if let Some(chosen) = command_button(ui, command, entry, targets) {
            action = Some(chosen);
            ui.close();
        }
    }
    action
}

impl ExpliaApp {
    /// Menu contextuel d'une ligne (liste, colonnes, colonne de navigation). `allow_rename` :
    /// la vue sait renommer sur place.
    pub(super) fn entry_menu(
        &self,
        ui: &mut egui::Ui,
        entry: &Entry,
        allow_rename: bool,
    ) -> Option<Action> {
        let targets = self.selection.targets_from(entry);
        let item = ItemRef::from_entry(entry);
        let items = entry_items(&EntryMenuContext {
            item: &item,
            count: targets.len(),
            caps: self.storage.capabilities(&entry.id),
            allow_rename,
            in_favorites: self.favorites.contains(&entry.path),
            in_applications: self.applications.contains(&entry.path),
            split: self.split.is_some(),
        });
        show(ui, entry, &items, &targets)
    }

    /// Menu d'une application de l'onglet « Applications ».
    pub(super) fn application_menu(
        &self,
        ui: &mut egui::Ui,
        path: &Path,
        removable: bool,
    ) -> Option<Action> {
        let entry = Entry::file_at(path);
        let items = application_items(self.storage.capabilities(&ItemId::path(path)), removable);
        show(ui, &entry, &items, std::slice::from_ref(&entry))
    }

    /// Menu d'un favori de la barre latérale.
    pub(super) fn favorite_menu(&self, ui: &mut egui::Ui, path: &Path) -> Option<Action> {
        let entry = Entry::folder_at(path);
        show(ui, &entry, &favorite_items(), std::slice::from_ref(&entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_manager::SourceKind;

    fn item(path: &str, kind: ItemKind) -> ItemRef {
        ItemRef {
            id: crate::storage_manager::ItemId::path(path),
            path: PathBuf::from(path),
            kind,
            source: SourceKind::Local,
        }
    }

    fn all_caps() -> Capabilities {
        Capabilities {
            list: true,
            open: true,
            rename: true,
            copy_into: true,
            move_items: true,
            create_folder: true,
            properties: true,
            delete: true,
            recycle_bin: true,
            shell: true,
        }
    }

    fn commands(items: &[MenuItem]) -> Vec<Command> {
        items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Command(c) => Some(*c),
                MenuItem::Separator => None,
            })
            .collect()
    }

    fn menu(item: &ItemRef, count: usize, caps: Capabilities) -> Vec<MenuItem> {
        entry_items(&EntryMenuContext {
            item,
            count,
            caps,
            allow_rename: true,
            in_favorites: false,
            in_applications: false,
            split: false,
        })
    }

    #[test]
    fn a_folder_offers_folder_commands() {
        let folder = item(r"C:\Projets", ItemKind::Folder);
        let found = commands(&menu(&folder, 1, all_caps()));
        for expected in [
            Command::Open,
            Command::OpenInNewTab,
            Command::Copy,
            Command::Cut,
            Command::Paste,
            Command::Rename,
            Command::Delete,
            Command::CreateFolder,
            Command::AddFavorite,
            Command::PowerShell,
            Command::ShowProperties,
        ] {
            assert!(found.contains(&expected), "{expected:?} manque");
        }
        assert!(!found.contains(&Command::RunAsAdmin));
    }

    #[test]
    fn a_file_offers_file_commands() {
        let exe = item(r"C:\outils\setup.exe", ItemKind::File);
        let found = commands(&menu(&exe, 1, all_caps()));
        assert!(found.contains(&Command::RunAsAdmin));
        assert!(found.contains(&Command::AddApplication));
        assert!(!found.contains(&Command::Paste));
        assert!(!found.contains(&Command::OpenInNewTab));
        assert!(!found.contains(&Command::AddFavorite));
    }

    #[test]
    fn a_multi_selection_keeps_only_group_commands() {
        let file = item(r"C:\a.txt", ItemKind::File);
        let found = commands(&menu(&file, 3, all_caps()));
        assert_eq!(found, [Command::Copy, Command::Cut, Command::Delete]);
    }

    #[test]
    fn commands_a_source_lacks_are_hidden() {
        let folder = item(r"R:\lecture-seule", ItemKind::Folder);
        let caps = Capabilities {
            list: true,
            open: true,
            ..Capabilities::NONE
        };
        let found = commands(&menu(&folder, 1, caps));
        for hidden in [
            Command::Cut,
            Command::Paste,
            Command::Rename,
            Command::Delete,
            Command::CreateFolder,
            Command::PowerShell,
            Command::ShowProperties,
        ] {
            assert!(
                !found.contains(&hidden),
                "{hidden:?} ne devrait pas paraître"
            );
        }
    }

    #[test]
    fn separators_never_lead_trail_or_repeat() {
        let file = item(r"C:\a.txt", ItemKind::File);
        let items = menu(&file, 3, Capabilities::NONE);
        assert_eq!(items, [MenuItem::Command(Command::Copy)]);
        let full = menu(&file, 1, all_caps());
        assert_ne!(full.first(), Some(&MenuItem::Separator));
        assert_ne!(full.last(), Some(&MenuItem::Separator));
        assert!(
            full.windows(2)
                .all(|w| !(w[0] == MenuItem::Separator && w[1] == MenuItem::Separator))
        );
    }
}
