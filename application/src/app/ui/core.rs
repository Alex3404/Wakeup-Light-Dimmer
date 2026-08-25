use embassy_embedded_hal::{shared_bus::asynch::i2c::I2cDevice};
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::{NoopRawMutex},
    rwlock::RwLock,
    signal::Signal,
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
    size::{ DisplaySize128x64},
};

use defmt::info;
use esp_hal::{Async};
use static_cell::StaticCell;

use super::input::{InputEvent, MenuControllerInterface};
use super::menus::*;
use crate::app::core::{AppStateReceiver, AppStateSender};

pub type I2c =
    I2cDevice<'static, NoopRawMutex, esp_hal::i2c::master::I2c<'static, Async>>;
pub type Interface = I2CInterface<I2c>;
pub type MenuSize = DisplaySize128x64;
pub type Display = Ssd1306Async<Interface, MenuSize, BufferedGraphicsModeAsync<MenuSize>>;

static CONTROLLER: StaticCell<MenuController> = StaticCell::new();

pub struct MenuController {
    user_data_receive: RwLock<NoopRawMutex, AppStateReceiver>,
    user_data_sender: AppStateSender,

    display: RwLock<NoopRawMutex, Display>,
    menu_text_style: MonoTextStyle<'static, BinaryColor>,
    large_text_style: MonoTextStyle<'static, BinaryColor>,
    selected_menu_text_style: MonoTextStyle<'static, BinaryColor>,

    current_menu: RwLock<NoopRawMutex, Option<MenuState>>,
    render_signal: &'static Signal<NoopRawMutex, ()>,
    next_menu_signal: Signal<NoopRawMutex, MenuSelect>,
}

impl MenuController {
    async fn render(&'static self) {
        // Take current menu to avoid circular mutable borrows
        let mut menu_w = self.current_menu.write().await;

        let current_menu = menu_w.take();
        let Some(mut menu) = current_menu else {
            // No menu to render
            return;
        };

        match &mut menu {
            MenuState::Main(menu) => menu.render(self).await,
            MenuState::Settings(menu) => menu.render(self).await,
        }

        menu_w.replace(menu);
    }

    pub fn create_rotery_interface(
        &'static self,
        spawner: Spawner,
    ) -> &'static MenuControllerInterface {
        MenuControllerInterface::new(self, spawner)
    }

    pub async fn initalize(
        spawner: Spawner,
        i2c: I2c,
        app_state_receive: AppStateReceiver,
        app_state_sender: AppStateSender,
    ) -> &'static MenuController {
        let interface = I2CDisplayInterface::new(i2c);

        let mut display = Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();

        let menu_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        let selected_menu_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::Off)
            .background_color(BinaryColor::On)
            .build();

        let large_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(BinaryColor::On)
            .build();

        static RENDER_SIGNAL: StaticCell<Signal<NoopRawMutex, ()>> = StaticCell::new();
        let render_signal = RENDER_SIGNAL.init(Signal::new());

        display.init().await.unwrap();
        display.flush().await.unwrap();
        render_signal.signal(());

        let controller = CONTROLLER.init(MenuController {
            user_data_receive: RwLock::new(app_state_receive),
            user_data_sender: app_state_sender,
            display: RwLock::new(display),
            menu_text_style,
            selected_menu_text_style,
            large_text_style,
            current_menu: RwLock::new(Some(MenuSelect::default().create_menu_item())),
            next_menu_signal: Signal::new(),
            render_signal,
        });

        let token = render_loop(controller);
        spawner.spawn(token.unwrap());

        controller
    }
}

pub(super) mod internal {
    use crate::app::{
        core::{AppStateReceiver, AppStateSender},
        ui::{core::Display, input::InputEvent, menus::MenuSelect},
    };
    use embassy_sync::{blocking_mutex::raw::NoopRawMutex, rwlock::RwLock};
    use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::BinaryColor};

    #[allow(dead_code)]
    pub(crate) trait MenuControllerInternal {
        async fn current_menu(&self) -> Option<MenuSelect>;
        fn next_menu(&self, menu: MenuSelect);
        fn mark_render(&self);
        fn display(&self) -> &RwLock<NoopRawMutex, Display>;
        fn app_state_sender(&self) -> &AppStateSender;
        fn app_state_receiver(&self) -> &RwLock<NoopRawMutex, AppStateReceiver>;
        fn menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn selected_menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        fn large_text_style(&self) -> MonoTextStyle<'static, BinaryColor>;
        async fn handle_input(&'static self, input: InputEvent);
    }
}

impl internal::MenuControllerInternal for MenuController {
    async fn current_menu(&self) -> Option<MenuSelect> {
        self.current_menu
            .read()
            .await
            .as_ref()
            .map(|menu_state| menu_state.get_menu_select())
    }

    fn next_menu(&self, menu: MenuSelect) {
        self.next_menu_signal.signal(menu);
        self.mark_render();
    }

    fn mark_render(&self) {
        self.render_signal.signal(());
    }

    fn display(&self) -> &RwLock<NoopRawMutex, Display> {
        &self.display
    }

    fn menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.menu_text_style
    }

    fn selected_menu_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.selected_menu_text_style
    }

    fn large_text_style(&self) -> MonoTextStyle<'static, BinaryColor> {
        self.large_text_style
    }

    async fn handle_input(&'static self, input: InputEvent) {
        // Take current menu to avoid circular mutable borrows
        let mut menu_w = self.current_menu.write().await;
        let Some(mut menu) = menu_w.take() else {
            // No menu to render
            return;
        };

        match &mut menu {
            MenuState::Main(menu) => menu.handle_input(input, self).await,
            MenuState::Settings(menu) => menu.handle_input(input, self).await,
        }
        menu_w.replace(menu);
    }

    fn app_state_sender(&self) -> &AppStateSender {
        &self.user_data_sender
    }

    fn app_state_receiver(&self) -> &RwLock<NoopRawMutex, AppStateReceiver> {
        &self.user_data_receive
    }
}

#[embassy_executor::task]
async fn render_loop(controller: &'static MenuController) {
    loop {
        if let Some(menu) = controller.next_menu_signal.try_take() {
            let mut menu_w = controller.current_menu.write().await;
            menu_w.replace(menu.create_menu_item());
            drop(menu_w);
            info!("Switched to menu: {:?}", menu);
            controller.render().await;
        }
        controller.render_signal.wait().await;
        controller.render().await;
    }
}
