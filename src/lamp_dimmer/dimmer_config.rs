use esp_hal::Blocking;
use esp_hal::gpio::{Input, Output};
use esp_hal::rmt::TxChannelCreator;

use crate::lamp_dimmer::{FireTimingConfig, MIN_BRIGHTNESS};

/// The lamp dimmer channel config specifies the configuration
/// of an lamp dimmer channel.
///
/// Required properties of a lamp dimmer are:
/// A zero cross input and triac gate output pin.
/// A RMT channel uses for cpu independant firing of the triac gate pin.
///
/// Optional:
/// A frequency of the AC wave, defaults to Determine AC Frequency.
/// A latching time around zero cross events, defaults to 2ms.
/// A mininum triac gate pulse, defaults to 150us
/// A gamma correction defaults to linear
/// - Use Exponental for LED bulbs it offers a more realistic dimming
///
pub struct LampDimmerChannelConfig<RTMChannel>
where
    RTMChannel: TxChannelCreator<'static, Blocking> + Sized,
{
    // Required fields
    pub(in crate::lamp_dimmer) frequency: u8,
    pub(in crate::lamp_dimmer) gate_output_pin: Output<'static>,
    pub(in crate::lamp_dimmer) zero_cross_pin: Input<'static>,
    pub(in crate::lamp_dimmer) rmt_channel: RTMChannel,

    /// Defaults to 0
    pub(in crate::lamp_dimmer) starting_brightness: u8,
    /// Defaults to FireTimingConfig::Default()
    pub(in crate::lamp_dimmer) fire_timing_config: FireTimingConfig,
}

impl<RTMChannel> LampDimmerChannelConfig<RTMChannel>
where
    RTMChannel: TxChannelCreator<'static, Blocking> + Sized,
{
    pub fn new(
        frequency: u8,
        zero_cross_pin: Input<'static>,
        gate_output_pin: Output<'static>,
        rmt_channel: RTMChannel,
    ) -> Self {
        Self {
            frequency,
            zero_cross_pin,
            gate_output_pin,
            rmt_channel,
            starting_brightness: MIN_BRIGHTNESS,
            // Choose some pretty basic starting
            // values that should work well in most cases
            fire_timing_config: FireTimingConfig::default(),
        }
    }

    pub fn with_firing_timing(mut self, fire_timing_config: FireTimingConfig) -> Self {
        self.fire_timing_config = fire_timing_config;
        self
    }
}
