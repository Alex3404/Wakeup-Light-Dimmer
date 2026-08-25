pub mod main_menu;
pub mod settings;

use super::{MenuController, input::InputEvent};
pub use main_menu::MainMenuItem;
pub use settings::SettingsMenuItem;

pub(super) trait MenuItem: Default {
    #[allow(unused)]
    async fn on_enter(&mut self, controller: &'static MenuController) {}
    async fn handle_input(&mut self, input: InputEvent, controller: &'static MenuController);
    async fn render(&self, controller: &'static MenuController);
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, defmt::Format)]
pub(crate) enum MenuSelect {
    #[default]
    Main,
    Settings,
}

#[derive(Debug, PartialEq, Eq, defmt::Format)]
pub(super) enum MenuState {
    Main(MainMenuItem),
    Settings(SettingsMenuItem),
}

impl MenuState {
    pub(super) fn get_menu_select(&self) -> MenuSelect {
        match self {
            MenuState::Main(_) => MenuSelect::Main,
            MenuState::Settings(_) => MenuSelect::Settings,
        }
    }
}

impl MenuSelect {
    pub(super) fn create_menu_item(&self) -> MenuState {
        match self {
            MenuSelect::Main => MenuState::Main(MainMenuItem::default()),
            MenuSelect::Settings => MenuState::Settings(SettingsMenuItem::default()),
        }
    }
}
