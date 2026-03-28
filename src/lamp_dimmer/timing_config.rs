use core::time::Duration;
use core::u16;
use log::warn;

use crate::lamp_dimmer::MAX_BRIGHTNESS;

#[derive(PartialEq, Clone, Copy)]
pub enum GammaCorrection {
    Exponetinal,
    Linear,
}

pub struct FireTimingConfig {
    // The minimum latching time around zero points
    // Since voltage could be too low to reliabily trigger the gate
    // Gives some extra padding for more reliable triggers
    pub(in crate::lamp_dimmer) latching_time_after_zero_us: u16,
    // Gives some margin for the latching time before the the next zero cross
    // For the same reason above. Also prevents the pulse from bleeding
    // into the next phase angles
    pub(in crate::lamp_dimmer) latching_time_before_next_zero_us: u16,
    // The minimum latching time required
    pub(in crate::lamp_dimmer) minimum_latching_time_us: u16,

    // Constrains the brightness values of pereceived brightness
    // For example if the user perceves the zero brightness for their bulb
    // Is actually at 25% brightness this will add some margin at the end of the
    // Phase angle to give the look up value of brightness 0
    pub(in crate::lamp_dimmer) perceved_zero_brightness: u8,
    // Same reasoning as above except constains what preceved full brightness looks
    // like adds some margin at the start of the phase angle to give the look up
    // value at max brightness the appororiate value
    pub(in crate::lamp_dimmer) perceved_full_brightness: u8,

    // User adjusted gamma correction LED bulbs give a more linear brightness
    // with using a exponetial gamma correction while incandecent gives more linear
    // with a linear style curve
    pub(in crate::lamp_dimmer) gamma_correction: GammaCorrection,
}

/// User facing API
impl FireTimingConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the minimum latching time total  
    pub fn with_min_latch_time(mut self, latch_time_us: u16) -> Self {
        self.minimum_latching_time_us = latch_time_us as u16;
        self
    }

    pub fn with_latch_time_after_zero(mut self, latch_time_us: u16) -> Self {
        self.latching_time_after_zero_us = latch_time_us as u16;
        self
    }

    pub fn with_latch_time_before_next_zero(mut self, latch_time_us: u16) -> Self {
        self.latching_time_before_next_zero_us = latch_time_us as u16;
        self
    }

    /// Sets the fraction of the wavelength at the start to not trigger in
    pub fn with_perceived_zero_brightness(mut self, brightness: u8) -> Self {
        self.perceved_zero_brightness = brightness.min(MAX_BRIGHTNESS);
        self
    }

    /// Sets the fraction of the wavelength at the end to not trigger in
    pub fn with_perceived_full_brightness(mut self, brightness: u8) -> Self {
        self.perceved_full_brightness = brightness.min(MAX_BRIGHTNESS);
        self
    }

    pub fn with_gamma_correction(mut self, correction: GammaCorrection) -> Self {
        self.gamma_correction = correction;
        self
    }
}

impl Default for FireTimingConfig {
    fn default() -> Self {
        Self {
            latching_time_after_zero_us: 1500,
            latching_time_before_next_zero_us: 250,
            minimum_latching_time_us: 150,
            perceved_zero_brightness: 0,
            perceved_full_brightness: MAX_BRIGHTNESS,
            gamma_correction: GammaCorrection::Linear,
        }
    }
}
