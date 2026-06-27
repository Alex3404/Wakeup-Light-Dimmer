pub mod dimmer_channel;
pub mod dimmer_config;
pub mod dimmer_settings_builder;
pub(super) mod lookup_tables;
pub mod rolling_average;
pub mod timing_config;

pub use dimmer_channel::DimmerChannel;
pub use dimmer_config::TriacChannelConfig;
pub use dimmer_settings_builder::DimmerSettingsBuilder;
pub use timing_config::TimingConfig;

use serde::{Deserialize, Serialize};

pub type Brightness = u16;
pub const MAX_BRIGHTNESS: Brightness = 1000;
pub const MIN_BRIGHTNESS: Brightness = 0;
const _: () = assert!(MAX_BRIGHTNESS != 0, "Max brightness cannot be 0");

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub enum GammaCorrection {
    Exponetinal = 0,
    Linear = 1,
}

/// Settings for brightness and gamma correction
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub struct DimmerSettings {
    pub perceived_zero_brightness: Brightness,
    pub perceived_full_brightness: Brightness,
    pub gamma_correction: GammaCorrection,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub struct DimmerState {
    pub brightness: Brightness,
    pub is_on: bool,
}

impl Default for DimmerSettings {
    fn default() -> Self {
        Self {
            perceived_zero_brightness: MIN_BRIGHTNESS,
            perceived_full_brightness: MAX_BRIGHTNESS,
            gamma_correction: GammaCorrection::Linear,
        }
    }
}

impl Default for DimmerState {
    fn default() -> Self {
        Self {
            brightness: MAX_BRIGHTNESS,
            is_on: true,
        }
    }
}
