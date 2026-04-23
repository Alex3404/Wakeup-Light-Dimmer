use embedded_graphics::Drawable;
use embedded_graphics::prelude::Point;
use embedded_graphics::text::{Baseline, Text};
use log::info;

use crate::app::ui::internal::MenuControllerInternal;
use crate::app::ui::menus::MenuSelect;
use crate::app::ui::{MenuController, input::InputEvent, menus::MenuItem};

#[derive(Debug, PartialEq, Eq)]
pub struct SettingsMenuItem {
    selected_option: usize,
}

impl Default for SettingsMenuItem {
    fn default() -> Self {
        Self { selected_option: 0 }
    }
}

impl MenuItem for SettingsMenuItem {
    async fn handle_input(&mut self, input: InputEvent, controller: &mut MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                info!("Switching to Main!");
                controller.next_menu(MenuSelect::Main);
            }
            _ => {}
        }

        controller.mark_render();
    }

    async fn render(&self, controller: &mut MenuController) {
        let large_text = controller.menu_text_style();
        let display = controller.display();

        display.clear_buffer();

        Text::with_baseline("Settings", Point::new(0, 0), large_text, Baseline::Top)
            .draw(display)
            .unwrap();

        display.flush().await.unwrap();
    }
}
