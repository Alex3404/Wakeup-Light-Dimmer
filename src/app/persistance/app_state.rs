use crate::app::lamp_dimmer::{DimmerSettings, DimmerState, MAX_BRIGHTNESS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum SunriseType {
    GentleSunrise,
    FastSunrise,
    OvercastSunrise,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
struct SunrisePresetIndex(pub u8);

#[derive(Debug, Eq, PartialEq, Clone)]
struct SunriseCurve {
    // Precomputed times for each brightness level during animation
    // stored as a fixed point value between 0 and 1 representing the fraction of the total duration
    brightness_at: [u16; MAX_BRIGHTNESS as usize + 1],
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct SunrisePreset {
    pub sunrise_type: SunriseType,
    pub duration_in_seconds: u16,
    pub start_brightness: u8,
    pub end_brightness: u8,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
struct AlarmInfoIndex(pub u8);

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct AlarmInfo {
    pub seconds_after_midnight: u16,
    pub sunrise_preset: SunrisePresetIndex,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
enum AlarmState {
    Snoozing {
        till: u64, // timestamp in seconds when the snooze ends
        alarm: AlarmInfoIndex,
    },
    SoundingAlarm {
        till: u64, // timestamp in seconds when the alarm should stop
        alarm: AlarmInfoIndex,
    },
    AnimatingSunrise {
        sunrise_preset: SunrisePresetIndex,
        started_at: u64, // timestamp in seconds when the animation started
    },
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AppState {
    pub dimmer_state: DimmerState,
    pub dimmer_settings: DimmerSettings,
    pub alarm_state: Option<AlarmState>,

    pub sunrise_presets: [Option<SunrisePreset>; 5],
    pub alarms: [Option<AlarmInfo>; 7],
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            dimmer_state: DimmerState::default(),
            dimmer_settings: DimmerSettings::default(),
            alarm_state: None,
            active_alarm: None,
            sunrise_presets: [None; 5],
            alarms: [None; 7],
        }
    }
}
