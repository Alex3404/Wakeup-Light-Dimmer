#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GammaCorrection {
    Exponetinal,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimingConfig {
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
}

/// User facing API
impl TimingConfig {
    pub const fn default() -> Self {
        Self {
            latching_time_after_zero_us: 1500,
            latching_time_before_next_zero_us: 250,
            minimum_latching_time_us: 150,
        }
    }

    /// Sets the minimum latching time total  
    pub const fn with_min_latch_time(mut self, latch_time_us: u16) -> Self {
        self.minimum_latching_time_us = latch_time_us as u16;
        self
    }

    pub const fn with_latch_time_after_zero(mut self, latch_time_us: u16) -> Self {
        self.latching_time_after_zero_us = latch_time_us as u16;
        self
    }

    pub const fn with_latch_time_before_next_zero(mut self, latch_time_us: u16) -> Self {
        self.latching_time_before_next_zero_us = latch_time_us as u16;
        self
    }
}
