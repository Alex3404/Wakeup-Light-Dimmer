use crate::app::{
    alarm::{
        alarm_data::{AlarmPreset, AlarmState},
        sunrise::SunrisePreset,
    },
};

use crate::drivers::lamp_dimmer::Brightness;
use serde::{Deserialize, Serialize};
use storage_derive::StorageData;


#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize, StorageData)]
#[storage(key_type = u64, key = "0x74_65_73_74")]
pub struct AppState {
    pub brightness : Brightness,
    pub light_is_on : bool,

    pub alarm_state: Option<AlarmState>,

    pub sunrise_presets: [Option<SunrisePreset>; 5],
    pub alarms: [Option<AlarmPreset>; 7],
}

impl AppState {
    pub fn brightness(&self) -> Brightness {
        if self.light_is_on {
            self.brightness
        } else {
            Brightness::ZERO
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            brightness: Brightness::FULL,
            light_is_on: true,
           
            alarm_state: None,
            sunrise_presets: [None; 5],
            alarms: [None; 7],
        }
    }
}
