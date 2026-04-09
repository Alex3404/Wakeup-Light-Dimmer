use embedded_graphics::Drawable;
use embedded_graphics::prelude::Dimensions;
use embedded_graphics::text::{Baseline, Text};
use log::info;

use crate::ui::internal::MenuControllerInternal;
use crate::ui::menus::MenuState;
use crate::ui::{MenuController, input::InputEvent, menus::MenuItem};

pub struct SettingsMenuItem;
impl MenuItem for SettingsMenuItem {
    async fn update(&mut self, input: InputEvent, controller: &mut MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                info!("Switching to Main!");
                controller.set_current_menu(MenuState::Main);
            }
            _ => {}
        }

        controller.mark_dirty();
    }

    async fn render(&self, controller: &mut MenuController) {
        let large_text = controller.menu_text_style();
        let display = controller.display();

        display.clear_buffer();

        Text::with_baseline(
            "Settings",
            display.bounding_box().center(),
            large_text,
            Baseline::Top,
        )
        .draw(display)
        .unwrap();

        display.flush().await.unwrap();
    }
}
