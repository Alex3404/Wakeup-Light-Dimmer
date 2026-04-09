use core::u16;
use esp_hal::rmt::PulseCode;

use crate::lamp_dimmer::MAX_BRIGHTNESS;
use crate::lamp_dimmer::{DimmerSettings, GammaCorrection, TimingConfig};

// Lookup table size for fire angle and fire pulse width
const LOOKUP_TABLE_SIZE: usize = MAX_BRIGHTNESS as usize + 1;
// Can't allow more mircoseconds then the pulse width allows
const MINIMUM_FREQUENCY: u8 = (1_000_000 / (PulseCode::MAX_LEN as u32 * 2) + 1) as u8;

const _: () = assert!(
    MINIMUM_FREQUENCY < 50,
    "Minimum frequency cannot be above 50Hz ( We support 50Hz )"
);

pub struct LookupTables {
    pub fire_angle_table: [u16; LOOKUP_TABLE_SIZE],
    pub pulse_width_table: [u16; LOOKUP_TABLE_SIZE],
}

macro_rules! fixed_point_mul {
    ($val1:expr, $val2:expr, $bits:expr) => {
        $val1.saturating_mul($val2) >> $bits
    };
}

/// All fractional calculations are done in u32 fixed point
/// 16 least signifcant bits repersent the fraction
/// 16 most siginifcant bits repersent the integer
/// Produces a micro second firing angle and micro second pulse width
/// Based on the provided firing config
impl TimingConfig {
    // Gamma corrects a fractional brightness value
    fn gamma_correct(brightness_fraction: u32, correction: GammaCorrection) -> u32 {
        match correction {
            // Approximated 2.2 by using fixed point power
            GammaCorrection::Exponetinal => brightness_fraction.pow(2) >> 16,
            GammaCorrection::Linear => brightness_fraction,
        }
    }

    pub fn create_lookup_tables(
        &self,
        frequency: u8,
        dimmer_settings: &DimmerSettings,
    ) -> LookupTables {
        let mut fire_angle_table: [u16; LOOKUP_TABLE_SIZE] = [0; LOOKUP_TABLE_SIZE];
        let mut pulse_width_table: [u16; LOOKUP_TABLE_SIZE] = [0; LOOKUP_TABLE_SIZE];

        if frequency == 0 {
            return LookupTables {
                fire_angle_table,
                pulse_width_table,
            };
        }

        let total_angle_time_micros = 1_000_000u32.strict_div(frequency as u32 * 2);

        let mut brightness_index = 0;
        while brightness_index < LOOKUP_TABLE_SIZE {
            // All fractional calculations are done in u32 fixed point
            // 16 least signifcant bits repersent the fraction
            // 16 most siginifcant bits repersent the integer

            // Turn brightness values from 0 to MAX_BRIGHTNESS divided by MAX_BRIGHTNESS
            // To yield a fraction as a fixed point number as described above
            let reserved_start_fraction =
                (MAX_BRIGHTNESS - dimmer_settings.perceived_full_brightness) as u32
                    * u16::MAX as u32
                    / MAX_BRIGHTNESS as u32;

            // Turn brightness values from 0 to MAX_BRIGHTNESS divided by MAX_BRIGHTNESS
            // To yield a fraction as a fixed point number as described above
            let reserved_end_fraction = (dimmer_settings.perceived_zero_brightness) as u32
                * u16::MAX as u32
                / MAX_BRIGHTNESS as u32;

            // Multiply the wave time by the both of the margin fractions to find
            // Phase angle
            let trigger_start_micros =
                fixed_point_mul!(total_angle_time_micros, reserved_start_fraction, 16);
            let trigger_end_micros =
                fixed_point_mul!(total_angle_time_micros, reserved_end_fraction, 16);

            // Compute the brightess fraction
            // as a fixed point number as described above
            let brightness_fraction =
                (u16::MAX as u32) * brightness_index as u32 / MAX_BRIGHTNESS as u32;

            // Gamma correct the fraction
            let brightness_fraction =
                TimingConfig::gamma_correct(brightness_fraction, dimmer_settings.gamma_correction);

            // Get the non reserved part of the wave
            let total_allowed_trigger_micros = total_angle_time_micros
                .saturating_sub(trigger_start_micros)
                .saturating_sub(trigger_end_micros)
                .saturating_sub(self.latching_time_before_next_zero_us as u32);

            // Multiply our wave time by 1.0 - brightness
            let one_minus_fraction = (u16::MAX as u32).saturating_sub(brightness_fraction);
            let trigger_time_us =
                fixed_point_mul!(total_allowed_trigger_micros, one_minus_fraction, 16);

            let trigger_time_micros = trigger_start_micros.saturating_add(trigger_time_us);
            let trigger_time_micros = trigger_time_micros as u16;

            let latch_time_micros = self
                .latching_time_after_zero_us
                .saturating_sub(trigger_time_micros)
                .max(self.minimum_latching_time_us);

            pulse_width_table[brightness_index] = latch_time_micros;
            fire_angle_table[brightness_index] = trigger_time_micros;
            brightness_index += 1;
        }

        LookupTables {
            fire_angle_table,
            pulse_width_table,
        }
    }
}
