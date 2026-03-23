use crate::rolling_average::TimeRollingAverage;

use core::ops::{AddAssign, Rem};
use core::option::Option;
use core::option::Option::{None, Some};

use core::cell::{Cell, RefCell};
use critical_section::{CriticalSection, Mutex};
use esp_hal::gpio::{Input, Level, Output};
use esp_hal::interrupt::Priority;
use esp_hal::pcnt::channel::EdgeMode;
use esp_hal::pcnt::{Pcnt, unit};
use esp_hal::rmt::{ContinuousTxTransaction, PulseCode, Tx, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Instant;
use esp_hal::{Blocking, handler, rmt};

extern crate alloc;
use alloc::sync::Arc;
use log::info;

const END_WAVE_LENGTH_MARGIN: f64 = 0.25;
const START_WAVE_LENGTH_MARGIN: f64 = 0.00;

const fn gamma_correct(brightness: f64) -> f64 {
    brightness * brightness
}

const fn fire_angle_loopup_micros<const N: usize>(frequency: u64) -> [u64; N] {
    let mut lookup_table: [u64; N] = [0; N];
    let wave_us: u64 = 1_000_000 / (frequency * 2 as u64);

    let mut i = 0;
    while i < N {
        let reserved_start_us = ((wave_us as f64) * START_WAVE_LENGTH_MARGIN) as u64;
        let reserved_end_us = ((wave_us as f64) * END_WAVE_LENGTH_MARGIN) as u64;

        let brightness = (i as f64) / (N as f64);
        // let corrected_brightness = gamma_correct(brightness);

        let trigger_wave_us = wave_us - reserved_end_us - reserved_start_us;
        let trigger_us = (trigger_wave_us as f64 * (1.0 - brightness)) as u64;

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
    last_edge: Mutex<Cell<Option<Instant>>>,
    brightness: Mutex<Cell<u8>>,

    event_counter: Mutex<RefCell<u64>>,

    // Gate channel
    tx_channel_lock: Mutex<RefCell<()>>,
    tx_channel: RefCell<Option<rmt::Channel<'static, Blocking, Tx>>>,
    current_tx_transaction: RefCell<Option<ContinuousTxTransaction<'static>>>,
}

impl LampDimmer {
    pub fn initalize<Channel>(
        mut pcnt: Pcnt<'static>,
        zero_cross_pin: Input<'static>,
        signal_output: Output<'static>,
        rmt_channel: Channel,
    ) -> Result<LampDimmerRef, ()>
    where
        Channel: TxChannelCreator<'static, Blocking> + Sized,
    {
        pcnt.set_interrupt_handler(pcnt_interrupt_handler);

        let pcnt_unit = pcnt.unit0;
        info!("Configure pnct!");
        let _ = LampDimmer::configure_pnct_unit(&pcnt_unit, zero_cross_pin)?;
        info!("Configure rmt!");
        let tx_channel = LampDimmer::configure_rmt_channel(rmt_channel, signal_output)?;
        info!("make mutex!");
        let pcnt_unit = Mutex::new(RefCell::new(pcnt_unit));
        info!("make mutex?");

        let dimmer = Self {
            pcnt_unit,
            brightness: Mutex::new(Cell::new(0)),
            avg_time_high: Mutex::new(RefCell::new(TimeRollingAverage::new())),
            last_edge: Mutex::new(Cell::new(None)),

            event_counter: Mutex::new(RefCell::new(0)),

            tx_channel_lock: Mutex::new(RefCell::new(())),
            tx_channel: RefCell::new(Some(tx_channel)),
            current_tx_transaction: RefCell::new(None),
        };

        let dimmer_ref = Arc::new(Mutex::new(RefCell::new(dimmer)));

        critical_section::with(|cs| DIMMER.replace(cs, Some(dimmer_ref.clone())));

        Ok(dimmer_ref)
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        critical_section::with(|cs| self.brightness.borrow(cs).set(brightness))
    }

    fn configure_rmt_channel<Channel>(
        rmt_channel: Channel,
        signal_output: Output<'static>,
    ) -> Result<rmt::Channel<'static, Blocking, Tx>, ()>
    where
        Channel: TxChannelCreator<'static, Blocking> + Sized,
    {
        let tx_config = TxChannelConfig::default()
            .with_idle_output_level(Level::Low)
            .with_idle_output(true)
            .with_carrier_modulation(false)
            .with_clk_divider(80); // For 1us

        let tx_channel = rmt_channel.configure_tx(signal_output, tx_config);
        let Ok(tx_channel) = tx_channel else {
            return Err(());
        };

        Ok(tx_channel)
    }

    fn configure_pnct_unit(
        unit: &unit::Unit<'static, 0>,
        zero_cross_pin: Input<'static>,
    ) -> Result<(), ()> {
        unit.unlisten(); // Stop interrupts
        unit.reset_interrupt(); // Reset pending interrupts
        unit.pause();
        unit.clear(); // set counter to 0

        // Set our limits of -1 and 1
        // So an interrupt anytime an increment or decrement is triggered
        let _ = unit.set_low_limit(Some(-1));
        let _ = unit.set_high_limit(Some(1));
        unit.set_filter(Some(1023));

        // Each time we are on a falling edge our counter is decremented
        // -> So the low limit interrupt is triggered
        // Each time we are in a rising edge our counter is incremented
        // -> So the high limit interrupt is triggered
        info!("Set input mode!");
        unit.channel0
            .set_input_mode(EdgeMode::Decrement, EdgeMode::Increment);
        info!("Set edge signal!");
        // Set our rising and falling edge signal to be our signal_input
        unit.channel0.set_edge_signal(zero_cross_pin);

        // Enable interupts
        info!("Enable interrupts!");
        unit.listen();
        unit.resume();

        info!("Tests!");
        Ok(())
    }

    fn execute_pulse<const N: usize>(&self, data: [PulseCode; N]) {
        critical_section::with(|cs| {
            let _lock_guard = self.tx_channel_lock.borrow_ref(cs);

            // Get channel forcefully ( Stop current pulse )
            let tx_channel_option = self.tx_channel.take();

            let tx_channel = if let Some(channel) = tx_channel_option {
                channel
            } else {
                // Pulse is still in progress
                let current_tx_option = self.current_tx_transaction.take();
                let Some(current_tx) = current_tx_option else {
                    // No channel and no transaction.. Nothing we can do
                    panic!("Fatal error in execute pulse");
                };

                // Stop the current transaction
                match current_tx.stop() {
                    Ok(channel) => channel,
                    Err((_err, channel)) => channel,
                }
            };

            let transaction =
                tx_channel.transmit_continuously(&data, esp_hal::rmt::LoopMode::Finite(1));
            let Ok(transaction) = transaction else {
                // This means our pulse data was invalid
                panic!("Transaction error cannot continue");
            };

            let _ = self
                .current_tx_transaction
                .borrow_mut()
                .replace(transaction);
        });
    }

    fn handle_dimming(&self, estimated_zero_cross_us: u64, cs: CriticalSection<'_>) {
        // Lookup table fast and easy
        let brightness = self.brightness.borrow(cs).get();
        let fire_angle_us = FIRE_ANGLE_TABLE[brightness as usize];
        let pulse_time_us = FIRE_WIDTH_TABLE[brightness as usize];

        let pulses = [
            // Delay to zero cross + fire angle
            PulseCode::new(
                Level::Low,
                estimated_zero_cross_us as u16,
                Level::Low,
                fire_angle_us as u16,
            ),
            // Trigger pulse for pulse time us
            PulseCode::new(Level::High, pulse_time_us as u16, Level::Low, 0),
        ];

        self.execute_pulse(pulses);
    }

    fn rising_edge(&self, time: Instant, cs: CriticalSection<'_>) {
        // Update the last edge
        let last_edge_cell = self.last_edge.borrow(cs);
        let _ = last_edge_cell.get();
        last_edge_cell.set(Some(time));

        // Estimated next zero cross on rising edge
        let average_high = self.avg_time_high.borrow_ref(cs).average();
        let estimated_zero_cross_us = (average_high / 2).as_micros();
        self.handle_dimming(estimated_zero_cross_us, cs);

        let event_cell = self.event_counter.borrow(cs);
        event_cell.borrow_mut().add_assign(1);

        if event_cell.borrow().rem(120) == 0 {
            info!("Average pulse: {}", average_high);
        }
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
