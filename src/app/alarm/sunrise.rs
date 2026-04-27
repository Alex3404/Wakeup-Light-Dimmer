use serde::{Deserialize, Serialize};

use crate::app::MAX_BRIGHTNESS;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SunriseCurve {
    // Precomputed times for each brightness level during animation
    // stored as a fixed point value between 0 and 1 representing the fraction of the total duration
    brightness_at: [u16; MAX_BRIGHTNESS as usize + 1],
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum SunriseType {
    GentleSunrise,
    FastSunrise,
    OvercastSunrise,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct SunrisePresetIndex(pub u8);

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct SunrisePreset {
    pub sunrise_type: SunriseType,
    pub duration_in_seconds: u16,
    pub start_brightness: u8,
    pub end_brightness: u8,
}
