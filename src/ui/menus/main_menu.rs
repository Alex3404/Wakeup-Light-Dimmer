use embedded_graphics::Drawable;
use embedded_graphics::prelude::Dimensions;
use embedded_graphics::text::{Alignment, Text};
use log::info;

use crate::ui::internal::MenuControllerInternal;
use crate::ui::menus::MenuState;
use crate::ui::{MenuController, input::InputEvent, menus::MenuItem};

pub struct MainMenuItem;
impl MenuItem for MainMenuItem {
    async fn update(&mut self, input: InputEvent, controller: &mut MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                info!("Switching to settings!");
                controller.set_current_menu(MenuState::Settings);
            }
            InputEvent::ButtonClick => {
                // Handle button click, maybe reset brightness
                controller
                    .app()
                    .upgrade()
                    .expect("App dropped")
                    .lock(|app| {
                        info!("Toggling light!");
                        app.toggle_light();
                    });
            }
            InputEvent::RotateClockwise => {
                controller
                    .app()
                    .upgrade()
                    .expect("App dropped")
                    .lock(|app| {
                        info!("Increasing brightness!");
                        app.update_brightness(|b| b.saturating_add(2));
                    });
            }
            InputEvent::RotateCounterClockwise => {
                controller
                    .app()
                    .upgrade()
                    .expect("App dropped")
                    .lock(|app| {
                        info!("Decreasing brightness!");
                        app.update_brightness(|b| b.saturating_sub(2));
                    });
            }
        }
        controller.mark_dirty();
    }

    async fn render(&self, controller: &mut MenuController) {
        let large_text = controller.large_text_style();

        let brightness = controller.brightness();
        let display = controller.display();

        display.clear_buffer();
        let text: Result<heapless::String<4>, _> = heapless::format!("{}%", brightness);
        let Ok(text) = text else {
            return;
        };

        Text::with_alignment(
            text.as_str(),
            display.bounding_box().center(),
            large_text,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();

        display.flush().await.unwrap();
    }
}
