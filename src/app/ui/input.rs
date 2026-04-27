use super::{MenuController, internal::MenuControllerInternal};
use crate::app::input::RoteryInterface;

use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use embassy_time::{Duration, WithTimeout};
use static_cell::StaticCell;

static LONG_BUTTON_PRESS: Duration = Duration::from_millis(1000);

pub enum InputEvent {
    ButtonClick,
    ButtonLongPress,
    RotateClockwise,
    RotateCounterClockwise,
}

static MENU_CONTROLLER_INTERFACE: StaticCell<MenuControllerInterface> = StaticCell::new();

/// RoteryInterface implementation that wraps MenuController
pub struct MenuControllerInterface {
    button_event: Signal<NoopRawMutex, bool>,
    rotate_queue: Channel<NoopRawMutex, InputEvent, 5>,
}

impl MenuControllerInterface {
    pub(super) fn new(menu: &'static MenuController, spawner: Spawner) -> &'static Self {
        let interface = MENU_CONTROLLER_INTERFACE.init(Self {
            button_event: Signal::new(),
            rotate_queue: Channel::new(),
        });

        let token = input_queue_loop(menu, interface).expect("Failed to start input loop");
        spawner.spawn(token);

        let token = button_press_loop(menu, interface).expect("Failed to start button press loop");
        spawner.spawn(token);

        interface
    }
}

impl RoteryInterface for MenuControllerInterface {
    fn pressed(&self, pressed: bool) {
        self.button_event.signal(pressed);
    }

    fn rotate_cw(&self) {
        let _ = self.rotate_queue.try_send(InputEvent::RotateClockwise);
    }

    fn rotate_ccw(&self) {
        let _ = self
            .rotate_queue
            .try_send(InputEvent::RotateCounterClockwise);
    }
}

#[embassy_executor::task]
async fn button_press_loop(
    menu: &'static MenuController,
    interface: &'static MenuControllerInterface,
) {
    loop {
        let pressed = interface.button_event.wait().await;
        if !pressed {
            continue;
        }

        // Next, wait for the button to be released or a long press timeout
        let result = interface
            .button_event
            .wait()
            .with_timeout(LONG_BUTTON_PRESS)
            .await;

        // If the wait timed out, it means the button is still pressed, so we treat it as a long press
        if result.is_err() {
            menu.handle_input(InputEvent::ButtonLongPress).await;
        } else if result.is_ok_and(|pressed| !pressed) {
            menu.handle_input(InputEvent::ButtonClick).await;
        }
    }
}

/// Task to handle rotation event inputs and pass them to the menu controller
#[embassy_executor::task]
async fn input_queue_loop(
    menu: &'static MenuController,
    interface: &'static MenuControllerInterface,
) {
    loop {
        let input = interface.rotate_queue.receive().await;
        menu.handle_input(input).await;
    }
}
