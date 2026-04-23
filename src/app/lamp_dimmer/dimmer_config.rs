use esp_hal::gpio::interconnect::{InputSignal, OutputSignal};
use esp_hal::peripherals::MCPWM0;

use crate::app::lamp_dimmer::{DimmerState, DimmerSettings, TimingConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GammaCorrection {
    Exponetinal,
    Linear,
}

/// The lamp dimmer channel config specifies the configuration
/// of an lamp dimmer channel.
///
/// Required properties of a lamp dimmer are:
/// A zero cross input and triac gate output pin.
///
/// Optional:
/// A frequency of the AC wave, defaults to Determine AC Frequency.
/// A latching time around zero cross events, defaults to 2ms.
/// A mininum triac gate pulse, defaults to 150us
/// A gamma correction defaults to linear
/// - Use Exponental for LED bulbs it offers a more realistic dimming
pub struct DimmerChannelConfig {
    // Required properties
    pub(super) frequency: u8,
    pub(super) gate: OutputSignal<'static>,
    pub(super) zero_cross: InputSignal<'static>,
    pub(super) mcpwm: MCPWM0<'static>,

    /// Defaults to off
    pub(super) starting_state: DimmerState,

    /// Defaults to TimingConfig::Default()
    pub(super) timing_config: TimingConfig,
    pub(super) dimmer_settings: DimmerSettings,
}

impl DimmerChannelConfig {
    pub fn new(
        frequency: u8,
        timing_config: TimingConfig,
        zero_cross_pin: InputSignal<'static>,
        gate_output_pin: OutputSignal<'static>,
        mcpwm: MCPWM0<'static>,
    ) -> Self {
        Self {
            frequency,
            zero_cross: zero_cross_pin,
            gate: gate_output_pin,
            mcpwm,
            timing_config,
            starting_state: DimmerState::default(),
            dimmer_settings: DimmerSettings::default(),
        }
    }

    pub fn with_starting_state(mut self, starting_state: DimmerState) -> Self {
        self.starting_state = starting_state;
        self
    }

    pub fn with_dimmer_settings(mut self, dimmer_settings: DimmerSettings) -> Self {
        self.dimmer_settings = dimmer_settings;
        self
    }

    pub fn with_firing_timing(mut self, fire_timing_config: TimingConfig) -> Self {
        self.timing_config = fire_timing_config;
        self
    }
}
