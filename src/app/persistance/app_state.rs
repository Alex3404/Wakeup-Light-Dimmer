use crate::app::{
    alarm::{
        alarm_data::{AlarmPreset, AlarmState},
        sunrise::SunrisePreset,
    },
    drivers::lamp_dimmer::{DimmerSettings, DimmerState},
    persistance::storage::StorageData,
};
use sequential_storage::map::PostcardValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub dimmer_state: DimmerState,
    pub dimmer_settings: DimmerSettings,
    pub alarm_state: Option<AlarmState>,

    pub sunrise_presets: [Option<SunrisePreset>; 5],
    pub alarms: [Option<AlarmPreset>; 7],
}

impl PostcardValue<'static> for AppState {}
impl StorageData<'static> for AppState {
    const KEY: u8 = 0x01;
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dimmer_state: DimmerState::default(),
            dimmer_settings: DimmerSettings::default(),
            alarm_state: None,
            sunrise_presets: [None; 5],
            alarms: [None; 7],
        }
    }
}
