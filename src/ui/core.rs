use embassy_sync::{blocking_mutex::raw::NoopRawMutex, rwlock::RwLock, signal::Signal};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle, MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::BinaryColor,
};

use log::info;
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async,
    mode::{BufferedGraphicsModeAsync, DisplayConfigAsync},
    prelude::{DisplayRotation, I2CInterface},
    size::DisplaySize128x64,
};

use esp_hal::{Async, i2c::master::I2c};

extern crate alloc;
use alloc::rc::{Rc, Weak};

use crate::{
    app::core::AppHandle,
    ui::input::{InputEvent, MenuControllerInterface},
};

type Interface = I2CInterface<I2c<'static, Async>>;
type Size = DisplaySize128x64;
type Display = Ssd1306Async<Interface, Size, BufferedGraphicsModeAsync<Size>>;

pub type MenuControllerHandle = Rc<RwLock<NoopRawMutex, MenuController>>;

use crate::ui::menus::*;

pub struct MenuController {
    app: AppHandle,

    display: Display,
    menu_text_style: MonoTextStyle<'static, BinaryColor>,
    large_text_style: MonoTextStyle<'static, BinaryColor>,

    current_menu: MenuState,
    render_signal: Signal<NoopRawMutex, ()>,
}

impl MenuController {
    pub async fn finish_initialization(&mut self) {
        self.display.init().await.unwrap();
        self.display.flush().await.unwrap();

        // Initial render
        self.mark_dirty();
        self.render().await;
    }

    async fn render(&mut self) {
        match self.current_menu {
            MenuState::Main => {
                let main_menu = MainMenuItem;
                main_menu.render(self).await;
            }
            MenuState::Settings => {
                // Handle settings menu
                let settings = SettingsMenuItem;
                settings.render(self).await;
            }
        }
    }
}

pub(in crate::ui) mod internal {
    use crate::{
        app::core::AppHandle,
        ui::{core::Display, input::InputEvent, menus::MenuState},
    };
    use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::BinaryColor};

    #[allow(dead_code)]
    pub trait MenuControllerInternal {
        fn current_menu(&self) -> MenuState;
        fn set_current_menu(&mut self, menu: MenuState);
        fn mark_dirty(&self);
        fn display(&mut self) -> &mut Display;
        fn menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn large_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn brightness(&self) -> u8;
        fn app(&self) -> AppHandle;
        async fn handle_input(&mut self, input: InputEvent);
        async fn render(&mut self);
    }
}

use internal::MenuControllerInternal;

impl internal::MenuControllerInternal for MenuController {
    fn current_menu(&self) -> MenuState {
        self.current_menu
    }

    fn set_current_menu(&mut self, menu: MenuState) {
        self.current_menu = menu;
        self.mark_dirty();
    }

    fn mark_dirty(&self) {
        self.render_signal.signal(());
    }

    fn display(&mut self) -> &mut Display {
        &mut self.display
    }

    fn menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.menu_text_style
    }

    fn large_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.large_text_style
    }

    async fn handle_input(&mut self, input: InputEvent) {
        info!("Menu Input!");
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

    fn brightness(&self) -> u8 {
        self.app.upgrade().expect("App dropped").lock(|app| {
            app.get_user_data()
                .get(|user_data| user_data.dimmer_state.brightness)
        })
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

    fn app(&self) -> AppHandle {
        self.app.clone()
    }
}

impl MenuController {
    pub fn new(i2c: I2c<'static, Async>, app: AppHandle) -> Result<MenuControllerHandle, ()> {
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

        let controller = Rc::new(RwLock::new(Self {
            app,
            display,
            menu_text_style,
            large_text_style,
            current_menu: MenuState::Main,
            render_signal: Signal::new(),
        }));

        Ok(controller)
    }

    /// Create a RoteryInterface for this menu controller
    /// Takes a reference to allow the Rc to be reused after calling this method
    pub fn create_rotery_interface(
        this: Weak<RwLock<NoopRawMutex, MenuController>>,
    ) -> Result<MenuControllerInterface, ()> {
        Ok(MenuControllerInterface::new(this))
    }
}
