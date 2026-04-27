use crate::app::alarm::sunrise::SunrisePresetIndex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct AlarmPresetIndex(pub u8);

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct AlarmPreset {
    pub seconds_after_midnight: u16,
    pub sunrise_preset: SunrisePresetIndex,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum AlarmState {
    Snoozing {
        till: u64, // timestamp in seconds when the snooze ends
        alarm: AlarmPresetIndex,
    },
    SoundingAlarm {
        till: u64, // timestamp in seconds when the alarm should stop
        alarm: AlarmPresetIndex,
    },
    AnimatingSunrise {
        sunrise_preset: SunrisePresetIndex,
        started_at: u64, // timestamp in seconds when the animation started
    },
}
