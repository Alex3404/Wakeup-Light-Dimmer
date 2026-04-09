pub mod dimmer_channel;
pub mod dimmer_config;
pub mod dimmer_settings_builder;
pub(in crate::lamp_dimmer) mod lookup_tables;
pub mod rolling_average;
pub mod timing_config;
pub mod zero_cross_analyzer;

pub use dimmer_channel::DimmerChannel;
pub use dimmer_config::DimmerChannelConfig;
pub use dimmer_settings_builder::{DimmerSettingsBuilder, PublishCallback};
pub use timing_config::{GammaCorrection, TimingConfig};

extern crate alloc;
use alloc::sync::Arc;
use core::cell::RefCell;
use embassy_sync::blocking_mutex::NoopMutex;

pub type DimmerChannelHandle = Arc<NoopMutex<RefCell<DimmerChannel>>>;

pub const MAX_BRIGHTNESS: u8 = 100;
pub const MIN_BRIGHTNESS: u8 = 0;
const _: () = assert!(MAX_BRIGHTNESS != 0, "Max brightness cannot be 0");

/// Settings for brightness and gamma correction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DimmerSettings {
    pub perceived_zero_brightness: u8,
    pub perceived_full_brightness: u8,
    pub gamma_correction: GammaCorrection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DimmerChannelState {
    pub brightness: u8,
    pub is_on: bool,
}

impl Default for DimmerSettings {
    fn default() -> Self {
        Self {
            perceived_zero_brightness: 0,
            perceived_full_brightness: MAX_BRIGHTNESS,
            gamma_correction: GammaCorrection::Linear,
        }
    }
}

impl Default for DimmerChannelState {
    fn default() -> Self {
        Self {
            brightness: 0,
            is_on: false,
        }
    }
}
