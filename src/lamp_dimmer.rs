use crate::pulse_scheduler;
use crate::rolling_average::TimeRollingAverage;

use core::option::Option;
use core::option::Option::{None, Some};

use core::cell::{Cell, RefCell};
use core::sync::atomic::AtomicU32;
use critical_section::{CriticalSection, Mutex};
use esp_hal::gpio::Level;
use esp_hal::gpio::{Input, Output};
use esp_hal::handler;
use esp_hal::interrupt::Priority;
use esp_hal::pcnt::channel::EdgeMode;
use esp_hal::pcnt::{Pcnt, unit};
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::{AnyTimer, Timer};
use log::info;

// Timing averages
static AVG_TIME_HIGH: Mutex<RefCell<TimeRollingAverage<25>>> =
    Mutex::new(RefCell::new(TimeRollingAverage::new()));
static AVG_TIME_LOW: Mutex<RefCell<TimeRollingAverage<25>>> =
    Mutex::new(RefCell::new(TimeRollingAverage::new()));
static LAST_EDGE: Mutex<Cell<Option<Instant>>> = Mutex::new(Cell::new(None));

static DIMMING_REQUEST: Mutex<Cell<Option<Instant>>> = Mutex::new(Cell::new(None));

static EVENT_COUNT: AtomicU32 = AtomicU32::new(0);

// Inital brightness of 0.0
// Fixed point
static BRIGHTNESS: Mutex<Cell<u16>> = Mutex::new(Cell::new(0));

/////////////////////////
/// HARDWARE POINTERS ///
/////////////////////////

// Currently used PCNT Unit
pub static PCNT_UNIT_NUMBER: usize = 0;
type Unit = unit::Unit<'static, PCNT_UNIT_NUMBER>;
static UNIT: Mutex<RefCell<Option<Unit>>> = Mutex::new(RefCell::new(None));

fn handle_dimming(next_zero_cross: Instant) {
    let (average_high, average_low, brightness) = critical_section::with(|cs| {
        (
            AVG_TIME_HIGH.borrow_ref(cs).average(),
            AVG_TIME_LOW.borrow_ref(cs).average(),
            BRIGHTNESS.borrow(cs).get(),
        )
    });
    let average_wavelength = average_low + average_high;

    // Latching time is about 1/5th the wave length after zero cross
    // Makes sure triac is fired
    let latching_time = average_wavelength / 2;

    const MINIMUM_PULSE: Duration = Duration::from_micros(150);

    // Give more margin at the start of the wave form
    let reserved_wavelength_start = Duration::ZERO;
    // Give more margin at the end of the wave form
    let resvered_wavelength_end = average_high / 2 + average_wavelength / 8;

    let usable_wavelength = average_wavelength
        .checked_sub(reserved_wavelength_start)
        .and_then(|s| s.checked_sub(resvered_wavelength_end));

    let Some(usable_wavelength) = usable_wavelength else {
        return;
    };

    // Super fast fixed point division where brightness was 0.0 to 1.0
    let scaled_wavelength = usable_wavelength
        .as_micros()
        .checked_mul((u16::MAX - brightness) as u64)
        .and_then(|w| Some(w >> 16));
    let Some(scaled_wavelength) = scaled_wavelength else {
        return;
    };

    // info!(
    //     "Wavelength: {}, Scaled: {}",
    //     average_wavelength.as_micros(),
    //     scaled_wavelength
    // );
    let scaled_wavelength = Duration::from_micros(scaled_wavelength as u64);

    let trigger_time = next_zero_cross + reserved_wavelength_start + scaled_wavelength;
    let pulse_length = latching_time
        .saturating_sub(scaled_wavelength)
        .max(MINIMUM_PULSE);

    let _ = pulse_scheduler::schedule_pulse(trigger_time, pulse_length);
}

// Our signal pin is at a falling edge
fn falling_edge(cs: CriticalSection<'_>) {
    let now = pulse_scheduler::now();

    let last_edge_cell = LAST_EDGE.borrow(cs);
    let last_edge_optional = last_edge_cell.get();
    last_edge_cell.set(Some(now));

    let Some(last_edge) = last_edge_optional else {
        // No previous data
        return;
    };

    // Time spent between now and the last rising edge
    let delta = now - last_edge;
    let average_high = AVG_TIME_HIGH.borrow_ref_mut(cs).new_sample(delta);

    // Set estimated next zero cross on falling edge
    // Time spent at ZERO is much larger then time spent high
    // Gives us more time to compute dimming timing
    let average_low = AVG_TIME_LOW.borrow_ref(cs).average();
    let estimated_zero_cross = now + average_low + average_high / 2;

    DIMMING_REQUEST.borrow(cs).set(Some(estimated_zero_cross));
}

// Our signal pin is at a rising edge
fn rising_edge(cs: CriticalSection<'_>) {
    let now = pulse_scheduler::now();

    let last_edge_cell = LAST_EDGE.borrow(cs);
    let last_edge_optional = last_edge_cell.get();
    last_edge_cell.set(Some(now));

    let Some(last_edge) = last_edge_optional else {
        // No previous data
        return;
    };

    // Time spent between now and the last rising edge
    let delta = now - last_edge;
    let _ = AVG_TIME_LOW.borrow_ref_mut(cs).new_sample(delta);
}

// Highest priority since timing is crutial
#[handler(priority = Priority::Priority3)]
fn interrupt_handler() {
    critical_section::with(|cs| {
        let mut u0 = UNIT.borrow_ref_mut(cs);
        if let Some(u0) = u0.as_mut() {
            if u0.interrupt_is_set() {
                let events = u0.events();
                if events.high_limit {
                    rising_edge(cs);
                } else if events.low_limit {
                    falling_edge(cs);
                }
                u0.reset_interrupt();
            }
        }
    });
}

pub fn do_pending_work() {
    let dimming_request = critical_section::with(|cs| {
        let request = DIMMING_REQUEST.borrow(cs).get();
        DIMMING_REQUEST.borrow(cs).set(None);
        request
    });

    if let Some(zero_cross) = dimming_request {
        // info!("Dimming!");
        handle_dimming(zero_cross);
    }
}

pub fn set_brightness(brightness: f32) {
    let fixed_point_brightness = (brightness.clamp(0.0, 1.0) * u16::MAX as f32) as u16;
    critical_section::with(|cs| {
        BRIGHTNESS.borrow(cs).set(fixed_point_brightness);
    })
}

pub fn initalize(
    signal_input: Input<'static>,
    gate_output: Output<'static>,
    dimming_hw_timer: impl Timer + Into<AnyTimer<'static>>,
    mut pcnt: Pcnt<'static>,
) {
    info!("Initalizing lamp dimmer!");
    pulse_scheduler::initalize(dimming_hw_timer.into(), gate_output, Level::Low);

    critical_section::with(|cs| {
        pcnt.set_interrupt_handler(interrupt_handler);

        let pcnt_unit = pcnt.unit0;

        // Reset our unit 0
        pcnt_unit.unlisten(); // Stop interrupts
        pcnt_unit.reset_interrupt(); // Reset pending interrupts
        pcnt_unit.pause();
        pcnt_unit.clear(); // set counter to 0

        // Set our limits of -1 and 1
        // So an interrupt anytime an increment or decrement is triggered
        let _ = pcnt_unit.set_low_limit(Some(-1));
        let _ = pcnt_unit.set_high_limit(Some(1));

        // Each time we are on a falling edge our counter is decremented
        // -> So the low limit interrupt is triggered
        // Each time we are in a rising edge our counter is incremented
        // -> So the high limit interrupt is triggered
        pcnt_unit
            .channel0
            .set_input_mode(EdgeMode::Decrement, EdgeMode::Increment);

        // Set our rising and falling edge signal to be our signal_input
        pcnt_unit.channel0.set_edge_signal(signal_input);

        // Enable interupts

        pcnt_unit.listen();
        pcnt_unit.resume();

        UNIT.borrow_ref_mut(cs).replace(pcnt_unit);
        LAST_EDGE.borrow(cs).set(None);
        BRIGHTNESS.borrow(cs).set(0);
    });

    info!("Lamp Dimmer Initalized!");
}
