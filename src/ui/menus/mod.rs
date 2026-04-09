pub mod main_menu;
pub mod settings;

pub use main_menu::MainMenuItem;
pub use settings::SettingsMenuItem;

use crate::ui::{MenuController, input::InputEvent};

pub(in crate::ui) trait MenuItem {
    async fn update(&mut self, input: InputEvent, controller: &mut MenuController);
    async fn render(&self, controller: &mut MenuController);
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(in crate::ui) enum MenuState {
    Main,
    Settings,
}
