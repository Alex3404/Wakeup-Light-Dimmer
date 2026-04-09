use embassy_sync::{
    blocking_mutex::{NoopMutex, raw::NoopRawMutex},
    signal::Signal,
};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle, MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};

use ssd1306::{
    I2CDisplayInterface, Ssd1306Async,
    mode::BufferedGraphicsModeAsync,
    prelude::{DisplayRotation, I2CInterface},
    size::DisplaySize128x64,
};

use esp_hal::{Async, i2c::master::I2c};

extern crate alloc;
use alloc::rc::Weak;
use core::cell::RefCell;

use crate::{
    app::core::AppHandle,
    ui::input::{InputEvent, MenuControllerInterface},
};

type Interface = I2CInterface<I2c<'static, Async>>;
type Size = DisplaySize128x64;
type Display = Ssd1306Async<Interface, Size, BufferedGraphicsModeAsync<Size>>;

trait MenuItem {
    async fn update(&mut self, input: InputEvent, controller: &mut MenuController);
    async fn render(&self, controller: &mut MenuController);
}

struct MainMenuItem;
impl MenuItem for MainMenuItem {
    async fn update(&mut self, input: InputEvent, controller: &mut MenuController) {
        match input {
            InputEvent::ButtonLongPress => {
                // Switch to menu settings
                controller.set_current_menu(MenuState::Settings);
            }
            InputEvent::ButtonClick => {
                // Handle button click, maybe reset brightness
                controller.app.upgrade().expect("App dropped").lock(|app| {
                    app.toggle_light();
                });
            }
            InputEvent::RotateClockwise => {
                controller.app.upgrade().expect("App dropped").lock(|app| {
                    app.update_brightness(|b| b.saturating_add(2));
                });
            }
            InputEvent::RotateCounterClockwise => {
                controller.app.upgrade().expect("App dropped").lock(|app| {
                    app.update_brightness(|b| b.saturating_sub(2));
                });
            }
        }
    }

    async fn render(&self, controller: &mut MenuController) {
        let large_text = controller.get_large_text_style();

        let brightness = controller.brightness();
        let display = controller.get_display();

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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum MenuState {
    Main,
    Settings,
}

pub struct MenuController {
    app: AppHandle,

    display: Display,
    menu_text_style: MonoTextStyle<'static, BinaryColor>,
    large_text_style: MonoTextStyle<'static, BinaryColor>,

    current_menu: MenuState,
    render_signal: Signal<NoopRawMutex, ()>,
}

impl MenuController {
    pub(in crate::ui) async fn handle_input(&mut self, input: InputEvent) {
        match self.current_menu {
            MenuState::Main => {
                let mut main_menu = MainMenuItem;
                main_menu.update(input, self).await;
            }
            MenuState::Settings => {
                // Handle settings menu
            }
        }

        if self.render_signal.signaled() {
            self.render_signal.reset();
            self.render().await;
        }
    }

    async fn render(&mut self) {
        match self.current_menu {
            MenuState::Main => {
                let main_menu = MainMenuItem;
                main_menu.render(self).await;
            }
            MenuState::Settings => {
                // Handle settings menu
            }
        }
    }

    #[allow(dead_code)]
    pub(self) fn get_current_menu(&self) -> MenuState {
        self.current_menu
    }

    pub(self) fn set_current_menu(&mut self, menu: MenuState) {
        self.current_menu = menu;
        self.mark_dirty();
    }

    pub fn mark_dirty(&self) {
        self.render_signal.signal(());
    }

    pub fn get_display(&mut self) -> &mut Display {
        &mut self.display
    }

    pub fn get_menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.menu_text_style
    }

    pub fn get_large_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.large_text_style
    }

    pub async fn finish_initialization(&mut self) {
        // Initial render
        self.mark_dirty();
        self.render().await;
    }

    fn brightness(&self) -> u8 {
        self.app.upgrade().expect("App dropped").lock(|app| {
            app.get_user_data()
                .get(|user_data| user_data.dimmer_state.brightness)
        })
    }
}

impl MenuController {
    pub fn new(i2c: I2c<'static, Async>, app: AppHandle) -> Result<Self, ()> {
        let interface = I2CDisplayInterface::new(i2c);

        let display = Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();

        let menu_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        let large_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(BinaryColor::On)
            .build();

        Ok(Self {
            display,
            current_menu: MenuState::Main,
            menu_text_style,
            large_text_style,
            render_signal: Signal::new(),
            app,
        })
    }

    /// Create a RoteryInterface for this menu controller
    /// Takes a reference to allow the Rc to be reused after calling this method
    pub fn create_rotery_interface(
        this: Weak<NoopMutex<RefCell<Self>>>,
    ) -> Result<MenuControllerInterface, ()> {
        Ok(MenuControllerInterface::new(this))
    }
}
