use static_cell::StaticCell;

use crate::app::core::AppStateReceiver;

pub mod alarm_data;
pub mod sunrise;

static _ALARM_MANGER_CELL: StaticCell<AlarmManager> = StaticCell::new();

pub struct AlarmManager {
    _app_state_recv: &'static AppStateReceiver,
}

// Interrupt when our hardware alarm goes off
#[esp_hal::handler]
fn _alarm_interrupt() {}

impl AlarmManager {
    pub fn initalize(app_state_recv: AppStateReceiver) -> Self {
        static APP_STATE_RECV_CELL: StaticCell<AppStateReceiver> = StaticCell::new();
        let app_state_recv = APP_STATE_RECV_CELL.init(app_state_recv);

        Self { _app_state_recv: app_state_recv }
    }

    pub fn update_next_alarms() {}
}
