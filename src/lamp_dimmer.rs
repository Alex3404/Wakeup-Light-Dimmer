use crate::pulse_scheduler::{PulseScheduler, RefPulseScheduler};
use crate::rolling_average::TimeRollingAverage;

use core::option::Option;
use core::option::Option::{None, Some};

use core::cell::{Cell, Ref, RefCell};
use core::slice::RChunks;
use critical_section::{CriticalSection, Mutex};
use esp_hal::gpio::Input;
use esp_hal::handler;
use esp_hal::interrupt::Priority;
use esp_hal::pcnt::channel::EdgeMode;
use esp_hal::pcnt::{Pcnt, unit};
use esp_hal::time::{Duration, Instant};
use log::info;

extern crate alloc;
use alloc::sync::Arc;

const END_WAVE_LENGTH_MARGIN: f64 = 0.20;
const START_WAVE_LENGTH_MARGIN: f64 = 0.10;

const fn fire_angle_loopup_micros<const N: usize>(frequency: u64) -> [u64; N] {
    let mut lookup_table: [u64; N] = [0; N];
    let wave_us: u64 = 1_000_000 / (frequency * 2 as u64);

    let mut i = 0;
    while i < N {
        let reserved_start_us = ((wave_us as f64) * START_WAVE_LENGTH_MARGIN) as u64;
        let reserved_end_us = ((wave_us as f64) * END_WAVE_LENGTH_MARGIN) as u64;

        let brightness = (i as f64) / (N as f64);
        let corrected_brightness = brightness * brightness;
        let trigger_wave_us = wave_us - reserved_end_us - reserved_start_us;
        let trigger_us = (trigger_wave_us as f64 * (1.0 - corrected_brightness)) as u64;

        lookup_table[i] = reserved_start_us + trigger_us;
        i += 1;
    }
    lookup_table
}

const LATCH_TIME_AFTER_ZERO_MICRO: u64 = 4000;
const MINIMUM_LATCH_TIME_MICRO: u64 = 300;
const fn fire_pulse_loopup_micros<const N: usize>(frequency: u64) -> [u64; N] {
    let angle_lookup = fire_angle_loopup_micros::<N>(frequency);
    let mut lookup_table: [u64; N] = [0; N];

    let mut i = 0;
    while i < N {
        let trigger_us = angle_lookup[i];
        let latch_time = if trigger_us > LATCH_TIME_AFTER_ZERO_MICRO {
            MINIMUM_LATCH_TIME_MICRO
        } else {
            LATCH_TIME_AFTER_ZERO_MICRO - trigger_us
        };

        lookup_table[i] = latch_time;
        i += 1;
    }
    lookup_table
}

// Maps the brightess value from 0 to 100
// To the time after a zero cross event to fire the triac
static FIRE_ANGLE_TABLE: [u64; 101] = fire_angle_loopup_micros(60);
static FIRE_WIDTH_TABLE: [u64; 101] = fire_pulse_loopup_micros(60);

pub type LampDimmerRef = Arc<Mutex<RefCell<LampDimmer>>>;
static DIMMER: Mutex<RefCell<Option<LampDimmerRef>>> = Mutex::new(RefCell::new(None));

#[handler(priority = Priority::Priority3)]
fn pcnt_interrupt_handler() {
    critical_section::with(|cs| {
        let now = Instant::now();
        let mut dimmer = DIMMER.borrow_ref_mut(cs);

        if let Some(dimmer) = dimmer.as_mut() {
            let dimmer = dimmer.borrow_ref(cs);
            let unit = dimmer.pcnt_unit.borrow_ref(cs);

            if unit.interrupt_is_set() {
                let events = unit.events();
                if events.high_limit {
                    dimmer.rising_edge(now, cs);
                } else if events.low_limit {
                    dimmer.falling_edge(now, cs);
                }
                unit.reset_interrupt();
            }
        }
    });
}

pub struct LampDimmer {
    pcnt_unit: Mutex<RefCell<unit::Unit<'static, 0>>>,
    avg_time_high: Mutex<RefCell<TimeRollingAverage<10>>>,
    avg_time_low: Mutex<RefCell<TimeRollingAverage<10>>>,
    last_edge: Mutex<Cell<Option<Instant>>>,
    brightness: Mutex<Cell<u8>>,
    // Gate channel
    pulse_scheduler: Mutex<RefCell<RefPulseScheduler>>,
}

impl LampDimmer {
    pub fn initalize(
        mut pcnt: Pcnt<'static>,
        zero_cross_pin: Input<'static>,
        pulse_scheduler: RefPulseScheduler,
    ) -> Result<LampDimmerRef, ()> {
        pcnt.set_interrupt_handler(pcnt_interrupt_handler);

        let pcnt_unit = pcnt.unit0;
        LampDimmer::configure_pnct_unit(&pcnt_unit, zero_cross_pin);

        let pcnt_unit = Mutex::new(RefCell::new(pcnt_unit));
        let dimmer = Self {
            pcnt_unit,
            brightness: Mutex::new(Cell::new(0)),
            avg_time_high: Mutex::new(RefCell::new(TimeRollingAverage::new())),
            avg_time_low: Mutex::new(RefCell::new(TimeRollingAverage::new())),
            last_edge: Mutex::new(Cell::new(None)),
            pulse_scheduler: Mutex::new(RefCell::new(pulse_scheduler)),
        };

        let dimmer_ref = Arc::new(Mutex::new(RefCell::new(dimmer)));

        critical_section::with(|cs| DIMMER.replace(cs, Some(dimmer_ref.clone())));

        Ok(dimmer_ref)
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        critical_section::with(|cs| self.brightness.borrow(cs).set(brightness))
    }

    fn configure_pnct_unit(unit: &unit::Unit<'static, 0>, zero_cross_pin: Input<'static>) {
        unit.unlisten(); // Stop interrupts
        unit.reset_interrupt(); // Reset pending interrupts
        unit.pause();
        unit.clear(); // set counter to 0

        // Set our limits of -1 and 1
        // So an interrupt anytime an increment or decrement is triggered
        let _ = unit.set_low_limit(Some(-1));
        let _ = unit.set_high_limit(Some(1));

        // Each time we are on a falling edge our counter is decremented
        // -> So the low limit interrupt is triggered
        // Each time we are in a rising edge our counter is incremented
        // -> So the high limit interrupt is triggered
        unit.channel0
            .set_input_mode(EdgeMode::Decrement, EdgeMode::Increment);

        // Set our rising and falling edge signal to be our signal_input
        unit.channel0.set_edge_signal(zero_cross_pin);

        // Enable interupts

        unit.listen();
        unit.resume();
    }

    fn handle_dimming(&self, zero_cross: Instant, cs: CriticalSection<'_>) {
        // Lookup table fast and easy
        let brightness = self.brightness.borrow(cs).get();
        let fire_angle_us = FIRE_ANGLE_TABLE[brightness as usize];
        let pulse_time_us = FIRE_WIDTH_TABLE[brightness as usize];

        let trigger_time = zero_cross + Duration::from_micros(fire_angle_us);
        let pulse_duration = Duration::from_micros(pulse_time_us);
        let _ = self
            .pulse_scheduler
            .borrow_ref(cs)
            .borrow_ref_mut(cs)
            .schedule_pulse(trigger_time, pulse_duration);
    }

    fn rising_edge(&self, time: Instant, cs: CriticalSection<'_>) {
        // Update the last edge
        let last_edge_cell = self.last_edge.borrow(cs);
        let last_edge_optional = last_edge_cell.get();
        last_edge_cell.set(Some(time));

        let Some(last_edge) = last_edge_optional else {
            // No previous data
            return;
        };

        // Time spent between now and the last falling edge
        let delta = time - last_edge;
        let _ = self.avg_time_low.borrow_ref_mut(cs).new_sample(delta);

        // Estimated next zero cross on rising edge
        let average_high = self.avg_time_high.borrow_ref(cs).average();
        let estimated_zero_cross =
            self.pulse_scheduler.borrow_ref(cs).borrow_ref(cs).now() + average_high / 2;

        self.handle_dimming(estimated_zero_cross, cs);
    }

    fn falling_edge(&self, time: Instant, cs: CriticalSection<'_>) {
        // Update the last edge
        let last_edge_cell = self.last_edge.borrow(cs);
        let last_edge_optional = last_edge_cell.get();
        last_edge_cell.set(Some(time));

        let Some(last_edge) = last_edge_optional else {
            // No previous data
            return;
        };

        // Time spent between now and the last rising edge
        let delta = time - last_edge;
        let _ = self.avg_time_high.borrow_ref_mut(cs).new_sample(delta);
    }
}
