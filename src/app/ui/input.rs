use super::{MenuController, internal::MenuControllerInternal};
use crate::app::input::RoteryInterface;

use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, rwlock::RwLock, signal::Signal};
use embassy_time::{Duration, WithTimeout};

extern crate alloc;
use alloc::sync::{Arc, Weak};

static LONG_BUTTON_PRESS: Duration = Duration::from_millis(1000);

pub enum InputEvent {
    ButtonClick,
    ButtonLongPress,
    RotateClockwise,
    RotateCounterClockwise,
}

/// RoteryInterface implementation that wraps MenuController
pub struct MenuControllerInterface {
    button_event: Arc<Signal<NoopRawMutex, bool>>,
    rotate_queue: Arc<Channel<NoopRawMutex, InputEvent, 5>>,
}

impl MenuControllerInterface {
    pub(super) fn new(menu: Weak<RwLock<NoopRawMutex, MenuController>>, spawner: Spawner) -> Self {
        let input_channel = Arc::new(Channel::new());
        let button_event = Arc::new(Signal::new());

        let token = input_queue_loop(menu.clone(), input_channel.clone())
            .expect("Failed to start input loop");
        spawner.spawn(token);

        let token = button_press_loop(menu.clone(), button_event.clone())
            .expect("Failed to start button press loop");
        spawner.spawn(token);

        Self {
            button_event: button_event,
            rotate_queue: input_channel,
        }
    }
}

impl RoteryInterface for MenuControllerInterface {
    fn pressed(&mut self, pressed: bool) {
        self.button_event.signal(pressed);
    }

    fn rotate_cw(&mut self) {
        let _ = self.rotate_queue.try_send(InputEvent::RotateClockwise);
    }

    fn rotate_ccw(&mut self) {
        let _ = self
            .rotate_queue
            .try_send(InputEvent::RotateCounterClockwise);
    }
}

#[embassy_executor::task]
async fn button_press_loop(
    menu: Weak<RwLock<NoopRawMutex, MenuController>>,
    button_event: Arc<Signal<NoopRawMutex, bool>>,
) {
    loop {
        let pressed = button_event.wait().await;
        if !pressed {
            continue;
        }

        // Next, wait for the button to be released or a long press timeout
        let result = button_event.wait().with_timeout(LONG_BUTTON_PRESS).await;
        let Some(menu) = menu.upgrade() else {
            // Menu controller was dropped
            return;
        };
        let mut menu_lock = menu.write().await;

        if result.is_err() {
            menu_lock.handle_input(InputEvent::ButtonLongPress).await;
        } else if result.is_ok_and(|pressed| !pressed) {
            menu_lock.handle_input(InputEvent::ButtonClick).await;
        }
    }
}

/// Task to handle rotation event inputs and pass them to the menu controller
#[embassy_executor::task]
async fn input_queue_loop(
    menu_cell: Weak<RwLock<NoopRawMutex, MenuController>>,
    input_channel: Arc<Channel<NoopRawMutex, InputEvent, 5>>,
) {
    loop {
        let input = input_channel.receive().await;
        let Some(menu) = menu_cell.upgrade() else {
            // Menu controller was dropped
            return;
        };

        let mut menu_lock = menu.write().await;
        menu_lock.handle_input(input).await;
    }
}
