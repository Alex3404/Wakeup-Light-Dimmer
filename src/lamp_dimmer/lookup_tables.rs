use core::u16;
use esp_hal::rmt::PulseCode;

use crate::lamp_dimmer::MAX_BRIGHTNESS;
use crate::lamp_dimmer::{FireTimingConfig, GammaCorrection};

// Lookup table size for fire angle and fire pulse width
const LOOKUP_TABLE_SIZE: usize = (MAX_BRIGHTNESS + 1) as usize;
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

/// All fractional calculations are done in u32 fixed point
/// 16 least signifcant bits repersent the fraction
/// 16 most siginifcant bits repersent the integer
/// Produces a micro second firing angle and micro second pulse width
/// Based on the provided firing config
impl FireTimingConfig {
    // Gamma corrects a fractional brightness value
    fn gamma_correct(brightness_fraction: u32, correction: GammaCorrection) -> u32 {
        match correction {
            // Approximated 2.2 by using fixed point power
            GammaCorrection::Exponetinal => brightness_fraction.pow(2) >> 16,
            GammaCorrection::Linear => brightness_fraction,
        }
    }

    pub fn create_lookup_tables(&self, frequency: u8) -> LookupTables {
        let mut fire_angle_table: [u16; LOOKUP_TABLE_SIZE] = [0; LOOKUP_TABLE_SIZE];
        let mut pulse_width_table: [u16; LOOKUP_TABLE_SIZE] = [0; LOOKUP_TABLE_SIZE];

        if frequency == 0 {
            return LookupTables {
                fire_angle_table,
                pulse_width_table,
            };
        }

        let total_angle_time_us = 1_000_000u32.strict_div(frequency as u32 * 2);

        let mut brightness_index = 0;
        while brightness_index < LOOKUP_TABLE_SIZE {
            // All fractional calculations are done in u32 fixed point
            // 16 least signifcant bits repersent the fraction
            // 16 most siginifcant bits repersent the integer

            // Multiply the wave time by the both of the margin fractions to find
            // Phase angle
            let reserved_start_us =
                (total_angle_time_us * self.perceved_zero_brightness as u32) >> 16;

            let reserved_end_us = (total_angle_time_us
                * (MAX_BRIGHTNESS - self.perceved_full_brightness) as u32)
                >> 16;

            // Compute the brightess fraction
            // for example as brightness goes from 0 to 100
            // the fraction is 1.0 * (brightness_index / 100)
            // If brightness_index is the fraction would be close to 0.5
            let brightness_fraction =
                (u16::MAX as u32) * brightness_index as u32 / MAX_BRIGHTNESS as u32;

            let brightness_fraction =
                FireTimingConfig::gamma_correct(brightness_fraction, self.gamma_correction);

            // Get the non reserved part of the wave
            let reserved_angle_time_us = total_angle_time_us
                .saturating_sub(reserved_end_us)
                .saturating_sub(reserved_start_us)
                .saturating_sub(self.latching_time_before_next_zero_us as u32);

            // Multiply our wave time by 1.0 - brightness
            let one_minus_fraction = (u16::MAX as u32).saturating_sub(brightness_fraction);
            let trigger_time_us = reserved_angle_time_us.saturating_mul(one_minus_fraction) >> 16;
            let trigger_angle_us = reserved_start_us.saturating_add(trigger_time_us);
            let trigger_angle_us = (trigger_angle_us as u16) & PulseCode::MAX_LEN;

            let latch_time = self
                .latching_time_after_zero_us
                .saturating_sub(trigger_angle_us)
                .max(self.minimum_latching_time_us);

            pulse_width_table[brightness_index] = latch_time & PulseCode::MAX_LEN;
            fire_angle_table[brightness_index] = trigger_angle_us;
            brightness_index += 1;
        }

        LookupTables {
            fire_angle_table,
            pulse_width_table,
        }
    }
}
