use super::MAX_BRIGHTNESS;
use super::{DimmerSettings, GammaCorrection, GateConfig};
use fixed::FixedU32;
use fixed::types::extra::*;

// Lookup table size for fire angle and fire pulse width
const LOOKUP_TABLE_SIZE: usize = MAX_BRIGHTNESS as usize + 1;

#[derive(Debug, Clone)]
pub struct LookupTable {
    pub fire_angle_table: [u16; LOOKUP_TABLE_SIZE],
    pub pulse_width_table: [u16; LOOKUP_TABLE_SIZE],
}

macro_rules! fixed_point_mul {
    ($val1:expr, $val2:expr, $bits:expr) => {
        $val1.saturating_mul($val2) >> $bits
    };
}

enum Error {
    // When the perceived full and zero brightness values share the same value
    InvalidPreceivedBrightness,
}

/// All fractional calculations are done in u32 fixed point
/// 16 least signifcant bits repersent the fraction
/// 16 most siginifcant bits repersent the integer
/// Produces a micro second firing angle and micro second pulse width
/// Based on the provided firing config
impl GateConfig {
    // Gamma corrects a fractional brightness value
    fn gamma_correct(brightness_fraction: u32, correction: GammaCorrection) -> u32 {
        match correction {
            // Approximated 2.2 by using fixed point power
            GammaCorrection::Exponential => brightness_fraction.pow(2) >> 16,
            GammaCorrection::Linear => brightness_fraction,
        }
    }

    pub fn populate_lookup_table(
        &self,
        frequency: u8,
        dimmer_settings: &DimmerSettings,
        lookup_table: &mut LookupTable,
    ) -> Result<(), Error> {
        // Calculate the time for one complete waveform
        let wave_form_time : FixedU32<U0> = FixedU32::<U0>::from_num(1_000_000u32.strict_div(frequency as u32 * 2));

        if dimmer_settings.perceived_full_brightness <= dimmer_settings.perceived_zero_brightness {
            return Err(Error::InvalidPreceivedBrightness);
        }

        let mut brightness_index : usize = 0;
        while brightness_index < LOOKUP_TABLE_SIZE {
            static MAX_BRIGHTNESS_FRACTION: FixedU32<U16> = FixedU32::<U16>::const_from_int(MAX_BRIGHTNESS as u32);
            
            // The sub-range of brightness values as a fraction
            let brightness_sub_range = FixedU32::<U16>::from_num(MAX_BRIGHTNESS
                .saturating_sub(dimmer_settings.perceived_full_brightness)
                .saturating_sub(dimmer_settings.perceived_zero_brightness))
                .checked_div(MAX_BRIGHTNESS_FRACTION);

            let brightness_sub_range_start =
                FixedU32::<U16>::from_num(dimmer_settings.perceived_zero_brightness)
                .checked_div(MAX_BRIGHTNESS_FRACTION);

            let brightness_sub_range = match brightness_sub_range {
                Some(val) => val,
                None => break,
            };

            if brightness_sub_range == 0 {
                // If the brightness range is 0, we can't compute a meaningful fraction
                break;
            }

            // Compute the brightess fraction within the full range of brightness
            let brightness_fraction =
                FixedU32::<U16>::from_num(brightness_index as u32).checked_div(MAX_BRIGHTNESS_FRACTION).map_or()
                
            // Map the brightness fraction to the sub-range
            // brightness_fraction (0..1) * brightness_sub_range (0..MAX_BRIGHTNESS) / MAX_BRIGHTNESS
            // perceived_zero_brightness / MAX_BRIGHTNESS
            let brightness_fraction = fixed_point_mul!(brightness_fraction, brightness_sub_range as u32, 16);

            // A fraction from 0 to 1 representing the full power level
            // shifted by the perceived full brightness
            let full_power_fraction: u32 =
                (MAX_BRIGHTNESS.saturating_sub(dimmer_settings.perceived_full_brightness)) as u32
                    * u16::MAX as u32
                    / MAX_BRIGHTNESS as u32;

            // A fraction from 0 to 1 representing the zero power level
            // shifted by the perceived zero brightness
            let zero_power_fraction = (dimmer_settings.perceived_zero_brightness) as u32
                * u16::MAX as u32
                / MAX_BRIGHTNESS as u32;
            
            // Full power time in microseconds
            let full_power_time_micros =
                fixed_point_mul!(total_angle_time_micros, full_power_fraction, 16);
                
            // Zero power time in microseconds
            let zero_power_time_micros =
                fixed_point_mul!(total_angle_time_micros, zero_power_fraction, 16)
                    .saturating_add(self.trailing_deadtime_time_us as u32);

            // Gamma correct the fraction
            let brightness_fraction =
                GateConfig::gamma_correct(brightness_fraction, dimmer_settings.gamma_correction);

            // Get the non reserved part of the wave
            let total_allowed_trigger_micros = total_angle_time_micros
                .saturating_sub(full_power_time_micros)
                .saturating_sub(zero_power_time_micros);

            // Multiply our wave time by 1.0 - brightness
            let one_minus_fraction = (u16::MAX as u32).saturating_sub(brightness_fraction);
            let trigger_time_us =
                fixed_point_mul!(total_allowed_trigger_micros, one_minus_fraction, 16);

            let trigger_time_micros = trigger_start_micros.saturating_add(trigger_time_us);
            let trigger_time_micros = trigger_time_micros as u16;

            // Cut off time where the pulse should go low
            let cut_off_time_micros =
                total_angle_time_micros.saturating_sub(trigger_end_micros) as u16;

            let latch_time_micros = self
                .leading_deadtime_time_us
                .saturating_sub(trigger_time_micros)
                .max(self.minimum_gate_time)
                .min(cut_off_time_micros.saturating_sub(trigger_time_micros));

            lookup_table.pulse_width_table[brightness_index] = latch_time_micros;
            lookup_table.fire_angle_table[brightness_index] = trigger_time_micros;
            brightness_index += 1;
        }
        Ok(())
    }
}

impl Default for LookupTable {
    fn default() -> Self {
        Self {
            fire_angle_table: [0; LOOKUP_TABLE_SIZE],
            pulse_width_table: [0; LOOKUP_TABLE_SIZE],
        }
    }
}
