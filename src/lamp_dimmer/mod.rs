pub mod dimmer_channel;
pub mod dimmer_config;
pub(in crate::lamp_dimmer) mod lookup_tables;
pub mod timing_config;
pub mod zero_cross_analyzer;

pub const MAX_BRIGHTNESS: u8 = u8::MAX;
pub const MIN_BRIGHTNESS: u8 = 0;

const _: () = assert!(MAX_BRIGHTNESS != 0, "Max brightness cannot be 0");

pub use dimmer_channel::LampDimmerChannel;
pub use dimmer_config::LampDimmerChannelConfig;
pub use timing_config::{FireTimingConfig, GammaCorrection};
