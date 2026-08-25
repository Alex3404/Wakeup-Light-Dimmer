pub mod esp_dimmer;
// pub mod dimmer_settings_builder;
// pub(super) mod lookup_tables;
pub mod rolling_average;

// pub use dimmer_settings_builder::DimmerSettingsBuilder;
use serde::{Deserialize, Serialize};
use fixed::types::U0F16;

/// This is the actual 0% to 100% power level of the dimmer
/// Where the precentage represents the cutoff point in the AC waveform
/// Ie 50% repersents half of the waveform present the with the rest of
/// the waveform absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power(pub U0F16);

impl Power {
    const ZERO: Self = Self(U0F16::const_from_int(0));
    const FULL: Self = Self(U0F16::from_bits(u16::MAX));

    pub fn from_fixed(value: U0F16) -> Self {
        Self(value)
    }

    pub fn from_brightness(value: U0F16) -> Self {
        Self(value)
    }
}

/// This is the brightness level of the dimmer
/// Where the value represents the perceived brightness of the light.
/// The brightness level is a non-linear representation of the actual power level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brightness(pub U0F16);

impl Brightness {
    pub const ZERO: Self = Self(U0F16::const_from_int(0));
    pub const FULL: Self = Self(U0F16::from_bits(u16::MAX));

    pub fn to_power_with_range(self, min: Power, max: Power) -> Option<Power> {
        let range = max.0.checked_sub(min.0)?;
        let fraction = self.0.checked_mul(range)?;
        let value = min.0.checked_add(fraction)?;
        Some(Power(value))
    }
}

/// The mode of operation for the AC dimming
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcDimmingMode {
    LeadingEdge,
    TrailingEdge
}

impl Default for AcDimmingMode {
    fn default() -> Self {
        Self::default()
    }
}

impl AcDimmingMode {
    pub const fn default() -> Self {
        Self::LeadingEdge
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateConfig {
    leading_deadtime_time: u16,
    trailing_deadtime_time: u16,
    minimum_gate_time: u16,
    active_low : bool,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self::default()
    }
}

impl GateConfig {
    pub const fn default() -> Self {
        Self {
            leading_deadtime_time: 1500,
            trailing_deadtime_time: 750,
            minimum_gate_time: 150,
            active_low: false,
        }
    }

    /// Sets the minimum gate time
    pub const fn with_minimum_gate_time(mut self, gate_time_us: u16) -> Self {
        self.minimum_gate_time = gate_time_us;
        self
    }

    /// Sets the leading deadtime
    pub const fn with_leading_deadtime(mut self, deadtime_us: u16) -> Self {
        self.leading_deadtime_time = deadtime_us;
        self
    }

    /// Sets the trailing deadtime
    pub const fn with_trailing_deadtime(mut self, deadtime_us: u16) -> Self {
        self.trailing_deadtime_time = deadtime_us;
        self
    }
}

/// Configuration for the power settings of the dimmer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerDimmingConfig {
    dimming_mode: AcDimmingMode,
    gate: GateConfig,
}

impl PowerDimmingConfig {
    pub const fn default() -> Self {
        Self {
            dimming_mode: AcDimmingMode::default(),
            gate: GateConfig::default(),
        }
    }

    pub const fn new(dimming_mode: AcDimmingMode, gate: GateConfig) -> Self {
        Self { dimming_mode, gate }
    }

    pub const fn with_dimming_mode(mut self, dimming_mode: AcDimmingMode) -> Self {
        self.dimming_mode = dimming_mode;
        self
    }

    pub const fn with_gate(mut self, gate: GateConfig) -> Self {
        self.gate = gate;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum PowerControlError<T : defmt::Format> {
    #[allow(dead_code)]
    InvalidPowerLevel,
    #[allow(dead_code)]
    Other(T),
}

/// A trait for controlling the power of a dimmer
pub trait PowerDimmingControl {
    type Error: defmt::Format;

    fn set_power(&mut self, power: Power) -> Result<(), PowerControlError<Self::Error>>;
    fn get_power(&self) -> Power;
    fn set_config(&mut self, config: PowerDimmingConfig);
}

/// Gamma correction type
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub enum GammaCorrection {
    /// Exponential gamma correction
    Exponential = 0,
    /// Linear gamma correction
    Linear = 1,
}

impl GammaCorrection {
    pub(crate) fn apply(&self, brightness: Brightness) -> Brightness {
        match self {
            GammaCorrection::Exponential => {
                // Apply exponential gamma correction
                // Placeholder implementation
                Brightness(brightness.0.saturating_mul(brightness.0))
            }
            GammaCorrection::Linear => brightness,
        }
    }
}

/// Settings for brightness and gamma correction
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct DimmerConfig {
    /// Where the dimmer's brightness levels start and end in terms of power
    pub brightness_min : Power,
    pub brightness_max : Power,

    /// The gamma correction to apply to the brightness levels
    pub gamma_correction: GammaCorrection,
}

#[allow(dead_code)]
enum ConfigError {
    InvalidBrightnessRange,
}

impl DimmerConfig {
    pub const fn default() -> Self {
        Self {
            brightness_min: Power::ZERO,
            brightness_max: Power::FULL,
            gamma_correction: GammaCorrection::Linear,
        }
    }

    pub const fn new(brightness_min: Power, brightness_max: Power, gamma_correction: GammaCorrection) -> Self {
        Self { brightness_min, brightness_max, gamma_correction }
    }
}

pub enum DimmerError<D: defmt::Format> {
    InvalidBrightness,
    DimmerError(D),
}

/// A trait for controlling a dimmer based on brightness
pub trait DimmerControl<T : PowerDimmingControl> {
    type Error: defmt::Format;
    
    fn set_brightness(&mut self, brightness: Brightness) -> Result<(), DimmerError<Self::Error>>;
    fn get_brightness(&self) -> Brightness;
    fn turn_on(&mut self) -> Result<(), DimmerError<Self::Error>>;
    fn turn_off(&mut self) -> Result<(), DimmerError<Self::Error>>;
    fn toggle_on_off(&mut self) -> Result<(), DimmerError<Self::Error>>;
    fn is_on(&self) -> bool;

    fn get_config(&self) -> &DimmerConfig;
    fn apply_config(&mut self, config: DimmerConfig);
    fn borrow_power_control(&self) -> &T;
}

pub struct BasicDimmer<T: PowerDimmingControl> {
    power_control: T,
    config: DimmerConfig,
    brightness: Brightness,
}

impl<T: PowerDimmingControl> BasicDimmer<T> {
    pub fn new(power_control: T, config: DimmerConfig) -> Self {
        Self { power_control, config, brightness: Brightness::ZERO }
    }
}

impl<T: PowerDimmingControl> DimmerControl<T> for BasicDimmer<T> {
    type Error = ();

    fn set_brightness(&mut self, brightness: Brightness) -> Result<(), DimmerError<Self::Error>> {
        let corrected = self.config.gamma_correction.apply(brightness);
        let power = corrected
            .to_power_with_range(self.config.brightness_min, self.config.brightness_max)
            .ok_or(DimmerError::InvalidBrightness)?;

        self.power_control.set_power(power).map_err(|_e| DimmerError::DimmerError(()))?;
        self.brightness = brightness;
        Ok(())
    }

    fn get_brightness(&self) -> Brightness {
        // Implementation for getting brightness
        self.brightness
    }

    fn turn_on(&mut self) -> Result<(), DimmerError<Self::Error>> {
        // Implementation for turning on
        unimplemented!("Turning on the dimmer is not yet implemented");
    }

    fn turn_off(&mut self) -> Result<(), DimmerError<Self::Error>> {
        // Implementation for turning off
        unimplemented!("Turning off the dimmer is not yet implemented");
    }

    fn toggle_on_off(&mut self) -> Result<(), DimmerError<Self::Error>> {
        // Implementation for toggling on/off
        unimplemented!("Toggling the dimmer on/off is not yet implemented");
    }

    fn is_on(&self) -> bool {
        // Implementation for checking if the dimmer is on
        false
    }

    fn get_config(&self) -> &DimmerConfig {
        &self.config
    }

    fn apply_config(&mut self, config: DimmerConfig) {
        self.config = config;
    }

    fn borrow_power_control(&self) -> &T {
        &self.power_control
    }
}