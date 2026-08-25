use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Clone, defmt::Format)]
pub struct SunriseCurve {
    // Precomputed times for each brightness level during animation
    // stored as a fixed point value between 0 and 1 representing the fraction of the total duration
    brightness_at: [u16; 100],
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize, defmt::Format)]
pub enum SunriseType {
    GentleSunrise,
    FastSunrise,
    OvercastSunrise,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize, defmt::Format)]
pub struct SunrisePresetIndex(pub u8);

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize, defmt::Format)]
pub struct SunrisePreset {
    pub sunrise_type: SunriseType,
    pub duration_in_seconds: u16,
    pub start_brightness: u8,
    pub end_brightness: u8,
}
