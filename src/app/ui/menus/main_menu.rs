use core::ops::DerefMut;

use embedded_graphics::Drawable;
use embedded_graphics::prelude::Dimensions;
use embedded_graphics::text::{Alignment, Text};

use crate::app::ui::internal::MenuControllerInternal;
use crate::app::ui::menus::MenuSelect;
use crate::app::ui::{MenuController, input::InputEvent, menus::MenuItem};
use crate::app::{MAX_BRIGHTNESS, MIN_BRIGHTNESS};

#[derive(Debug, PartialEq, Eq, Default, defmt::Format)]
pub struct MainMenuItem;

impl MenuItem for MainMenuItem {
    async fn handle_input(&mut self, input: InputEvent, controller: &'static MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                controller.next_menu(MenuSelect::Settings);
            }
            InputEvent::ButtonClick => {
                controller.app_state_sender().send_modify(|user_data| {
                    if let Some(userdata) = user_data {
                        userdata.dimmer_state.is_on = !userdata.dimmer_state.is_on;
                    }
                });
            }
            InputEvent::RotateClockwise(speed) => {
                let increment = if speed < 50 {
                    50
                } else if speed < 100 {
                    20
                } else {
                    5
                };

                controller.app_state_sender().send_modify(|user_data| {
                    if let Some(userdata) = user_data {
                        userdata.dimmer_state.brightness = userdata
                            .dimmer_state
                            .brightness
                            .saturating_add(increment)
                            .clamp(MIN_BRIGHTNESS, MAX_BRIGHTNESS);
                    }
                });
                controller.mark_render();
            }
            InputEvent::RotateCounterClockwise(speed) => {
                let increment = if speed < 50 {
                    50
                } else if speed < 100 {
                    20
                } else {
                    5
                };

                controller.app_state_sender().send_modify(|user_data| {
                    if let Some(userdata) = user_data {
                        userdata.dimmer_state.brightness = userdata
                            .dimmer_state
                            .brightness
                            .saturating_sub(increment)
                            .clamp(MIN_BRIGHTNESS, MAX_BRIGHTNESS);
                    }
                });
                controller.mark_render();
            }
        }
    }

    async fn render(&self, controller: &'static MenuController) {
        let large_text = controller.large_text_style();

        // Get brightness from app state
        let app_state_recv = controller.app_state_receiver();
        let mut app_state_recv_w = app_state_recv.write().await;
        let brightness = app_state_recv_w.get().await.dimmer_state.brightness;
        drop(app_state_recv_w);

        let mut display = controller.display().write().await;

        display.clear_buffer();
        let text: Result<heapless::String<6>, _> =
            heapless::format!("{}.{:}%", brightness / 10, brightness % 10);
        let Ok(text) = text else {
            return;
        };

        Text::with_alignment(
            text.as_str(),
            display.bounding_box().center(),
            large_text,
            Alignment::Center,
        )
        .draw(display.deref_mut())
        .unwrap();

        display.flush().await.unwrap();
    }
}
