struct SunriseCurve {
    // Precomputed times for each brightness level during animation
    // stored as a fixed point value between 0 and 1 representing the fraction of the total duration
    brightness_at: [u16; MAX_BRIGHTNESS as usize + 1],
}

pub enum SunriseType {
    GentleSunrise,
    FastSunrise,
    OvercastSunrise,
}

pub struct SunriseData {
    pub sunrise_type: SunriseType,
    pub duration: Duration,
    pub start_brightness: u8,
    pub end_brightness: u8,
}
