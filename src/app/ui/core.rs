use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    rwlock::RwLock,
    signal::Signal,
    watch::{DynAnonReceiver, DynSender},
};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle, MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_10X20},
    },
    pixelcolor::BinaryColor,
};

use ssd1306::{
    I2CDisplayInterface, Ssd1306Async,
    mode::{BufferedGraphicsModeAsync, DisplayConfigAsync},
    prelude::{DisplayRotation, I2CInterface},
    size::DisplaySize128x64,
};

use esp_hal::{Async, i2c::master::I2c};

extern crate alloc;
use alloc::sync::{Arc, Weak};

use super::input::{InputEvent, MenuControllerInterface};
use super::menus::*;
use crate::app::{core::AppHandle, persistance::AppState};

type Interface = I2CInterface<I2c<'static, Async>>;
type Size = DisplaySize128x64;
type Display = Ssd1306Async<Interface, Size, BufferedGraphicsModeAsync<Size>>;

pub type MenuControllerHandle = Arc<RwLock<NoopRawMutex, MenuController>>;

pub struct MenuController {
    app: AppHandle,
    user_data_receive: DynAnonReceiver<'static, AppState>,
    user_data_sender: DynSender<'static, AppState>,

    display: Display,
    menu_text_style: MonoTextStyle<'static, BinaryColor>,
    large_text_style: MonoTextStyle<'static, BinaryColor>,

    current_menu: Option<MenuState>,
    render_signal: Arc<Signal<NoopRawMutex, ()>>,
    next_menu_signal: Signal<NoopRawMutex, MenuSelect>,
}

impl MenuController {
    pub async fn finish_initialization(&mut self) {
        self.display.init().await.unwrap();
        self.display.flush().await.unwrap();

        // Initial render
        self.mark_render();
        self.render().await;
    }

    async fn render(&mut self) {
        // Take current menu to avoid circular mutable borrows
        let current_menu = self.current_menu.take();
        let Some(mut menu) = current_menu else {
            // No menu to render
            return;
        };

        match &mut menu {
            MenuState::Main(menu) => menu.render(self).await,
            MenuState::Settings(menu) => menu.render(self).await,
        }
        self.current_menu = Some(menu);
    }
}

pub(super) mod internal {
    use crate::app::{
        core::AppHandle,
        persistance::AppState,
        ui::{core::Display, input::InputEvent, menus::MenuSelect},
    };
    use embassy_sync::watch::DynSender;
    use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::BinaryColor};

    #[allow(dead_code)]
    pub trait MenuControllerInternal {
        fn current_menu(&self) -> Option<MenuSelect>;
        fn next_menu(&self, menu: MenuSelect);
        fn mark_render(&self);
        fn display(&mut self) -> &mut Display;
        fn user_data(&self) -> &DynSender<'static, AppState>;
        fn menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn large_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn brightness(&mut self) -> u8;
        async fn handle_input(&mut self, input: InputEvent);
        fn app(&self) -> AppHandle;
    }
}

use internal::MenuControllerInternal;

impl internal::MenuControllerInternal for MenuController {
    fn current_menu(&self) -> Option<MenuSelect> {
        self.current_menu
            .as_ref()
            .map(|menu| menu.get_menu_select())
    }

    fn next_menu(&self, menu: MenuSelect) {
        self.next_menu_signal.signal(menu);
        self.mark_render();
    }

    fn mark_render(&self) {
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

    fn brightness(&mut self) -> u8 {
        // Get brightness from user data
        let userdata = self.user_data_receive.try_get();

        if let Some(userdata) = userdata {
            userdata.dimmer_state.brightness
        } else {
            0
        }
    }

    fn app(&self) -> AppHandle {
        self.app.clone()
    }

    async fn handle_input(&mut self, input: InputEvent) {
        // Take current menu to avoid circular mutable borrows
        let current_menu = self.current_menu.take();
        let Some(mut menu) = current_menu else {
            // No menu to render
            return;
        };

        match &mut menu {
            MenuState::Main(menu) => menu.handle_input(input, self).await,
            MenuState::Settings(menu) => menu.handle_input(input, self).await,
        }
        self.current_menu = Some(menu);
    }

    fn user_data(&self) -> &DynSender<'static, AppState> {
        &self.user_data_sender
    }
}

impl MenuController {
    pub fn new(
        spawner: Spawner,
        i2c: I2c<'static, Async>,
        app: AppHandle,
        user_data_receive: DynAnonReceiver<'static, AppState>,
        user_data_sender: DynSender<'static, AppState>,
    ) -> Result<MenuControllerHandle, ()> {
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

        let controller = Arc::new(RwLock::new(Self {
            app,
            user_data_receive,
            user_data_sender,
            display,
            menu_text_style,
            large_text_style,
            current_menu: Some(MenuSelect::default().create_menu_item()),
            next_menu_signal: Signal::new(),
            render_signal: Arc::new(Signal::new()),
        }));

        let token = render_loop(controller.clone());
        spawner.spawn(token.unwrap());

        Ok(controller)
    }

    /// Create a RoteryInterface for this menu controller
    /// Takes a reference to allow the Arc to be reused after calling this method
    pub fn create_rotery_interface(
        this: Weak<RwLock<NoopRawMutex, MenuController>>,
        spawner: Spawner,
    ) -> Result<MenuControllerInterface, ()> {
        Ok(MenuControllerInterface::new(this, spawner))
    }
}

#[embassy_executor::task]
async fn render_loop(controller: MenuControllerHandle) {
    loop {
        let controller_r = controller.read().await;
        let render_signal = controller_r.render_signal.clone();
        drop(controller_r); // Release lock before awaiting

        // Wait for render signal
        render_signal.wait().await;
        let mut controller_w = controller.write().await;
        controller_w.render().await;
    }
}
