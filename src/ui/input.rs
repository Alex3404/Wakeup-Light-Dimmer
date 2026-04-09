use crate::input::RoteryInterface;
use crate::ui::MenuController;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::Instant;

extern crate alloc;
use alloc::rc::{Rc, Weak};
use core::cell::RefCell;

pub enum InputEvent {
    ButtonClick,
    ButtonLongPress,
    RotateClockwise,
    RotateCounterClockwise,
}

/// RoteryInterface implementation that wraps MenuController
pub struct MenuControllerInterface {
    menu: Weak<NoopMutex<RefCell<MenuController>>>,
    pressed_time: Option<Instant>,
}

impl MenuControllerInterface {
    pub(super) fn new(menu: Weak<NoopMutex<RefCell<MenuController>>>) -> Self {
        Self {
            menu,
            pressed_time: None,
        }
    }
}

impl RoteryInterface for MenuControllerInterface {
    fn pressed(&mut self, pressed: bool, spawner: Spawner) {
        if pressed {
            self.pressed_time = Some(Instant::now());
            return;
        }

        let Some(pressed_time) = self.pressed_time else {
            return;
        };

        let press_time = pressed_time.elapsed();
        let long_press = press_time.as_millis() > 1000;

        let Some(menu) = self.menu.upgrade() else {
            // Menu controller was dropped
            return;
        };

        let _ = spawner.spawn(handle_menu_input(
            menu,
            if long_press {
                InputEvent::ButtonLongPress
            } else {
                InputEvent::ButtonClick
            },
        ));
    }

    fn rotate_cw(&mut self, spawner: Spawner) {
        let Some(menu) = self.menu.upgrade() else {
            // Menu controller was dropped
            return;
        };
        let _ = spawner.spawn(handle_menu_input(menu, InputEvent::RotateClockwise));
    }

    fn rotate_ccw(&mut self, spawner: Spawner) {
        let Some(menu) = self.menu.upgrade() else {
            // Menu controller was dropped
            return;
        };
        let _ = spawner.spawn(handle_menu_input(menu, InputEvent::RotateCounterClockwise));
    }
}

/// Task to handle menu input asynchronously
#[embassy_executor::task(pool_size = 3)]
async fn handle_menu_input(menu: Rc<NoopMutex<RefCell<MenuController>>>, input: InputEvent) {
    let mut menu = menu.borrow().borrow_mut();
    menu.handle_input(input).await;
}
