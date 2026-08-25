use core::ops::DerefMut;

use core::fmt::Write; // Required for the write! macro
use defmt::info;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::rwlock::RwLockWriteGuard;
use embedded_graphics::Drawable;
use embedded_graphics::geometry::OriginDimensions;
use embedded_graphics::prelude::Point;
use embedded_graphics::text::{Alignment, Baseline, Text};
use heapless::String;
use strum::IntoEnumIterator;

use crate::app::ui::internal::MenuControllerInternal;
use crate::app::ui::menus::MenuSelect;
use crate::app::ui::{MenuController, input::InputEvent, menus::MenuItem};

#[derive(Debug, PartialEq, Eq, defmt::Format, Clone, Copy, strum_macros::EnumIter)]
#[repr(u8)]
enum SettingsMenuOption {
    PerceivedBrightness = 0,
}

#[derive(Debug, PartialEq, Eq, defmt::Format)]
pub struct SettingsMenuItem {
    selected_option: SettingsMenuOption,
}

impl Default for SettingsMenuItem {
    fn default() -> Self {
        Self {
            selected_option: SettingsMenuOption::PerceivedBrightness,
        }
    }
}

impl MenuItem for SettingsMenuItem {
    async fn handle_input(&mut self, input: InputEvent, controller: &'static MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                info!("Switching to Main!");
                controller.next_menu(MenuSelect::Main);
            }
            _ => {}
        }
    }

    async fn render(&self, controller: &'static MenuController) {
        let large_text = controller.menu_text_style();
        let mut display = controller.display().write().await;

        display.clear_buffer();

        Text::with_alignment(
            "Settings",
            Point::new(display.size().width as i32 / 2, 10),
            large_text,
            Alignment::Center,
        )
        .draw(display.deref_mut())
        .unwrap();

        self.render_options(controller, &mut display).await;

        display.flush().await.unwrap();
    }
}

impl SettingsMenuItem {
    async fn render_options(
        &self,
        controller: &'static MenuController,
        display: &mut RwLockWriteGuard<'_, NoopRawMutex, crate::app::ui::core::Display>,
    ) {
        for option in SettingsMenuOption::iter() {
            let setting_text = match option {
                SettingsMenuOption::PerceivedBrightness => "Preceived Bright",
            };

            let mut text: heapless::String<32> = String::new();
            if option == self.selected_option {
                let _ = write!(text, "> {}", setting_text);
            } else {
                let _ = write!(text, "  {}", setting_text);
            }

            let style = if option == self.selected_option {
                controller.selected_menu_text_style()
            } else {
                controller.menu_text_style()
            };

            Text::with_baseline(
                text.as_str(),
                Point::new(0, 20 + (option as i32 * 20)),
                style,
                Baseline::Top,
            )
            .draw(display.deref_mut())
            .unwrap();
        }
    }
}
