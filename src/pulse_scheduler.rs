extern crate alloc;

use core::result::Result;
use core::result::Result::Ok;

use core::cell::{Cell, Ref, RefCell};
use core::cmp::{Eq, Ord, Ordering, PartialEq, PartialOrd};
use core::marker::Copy;
use core::option::Option;
use core::option::Option::{None, Some};
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering as AtomicOrdering;

use critical_section::CriticalSection;
use critical_section::Mutex;
use esp_hal::gpio::{Level, Output};
use esp_hal::rmt::{
    self, ContinuousTxTransaction, Direction, PulseCode, Tx, TxChannelConfig, TxChannelCreator,
};
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::{AnyTimer, Timer};
use esp_hal::{Blocking, handler};
use esp_println::println;
use libm::sin;
use log::{info, warn};

use alloc::sync::Arc;
use heapless::binary_heap::{BinaryHeap, Min};

#[derive(PartialEq, Eq, Clone, Copy)]
struct QueuedPulse {
    pub time: Instant,
    pub pulse_code: PulseCode,
}

impl QueuedPulse {
    fn new(time: Instant, pulse_code: PulseCode) -> Self {
        Self { time, pulse_code }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct ScheduledPulse {
    alarm_time: Duration,
    pulse_code: PulseCode,
}

impl ScheduledPulse {
    pub fn scheduled_time(&self, timer_running_time: Instant) -> Instant {
        timer_running_time + self.alarm_time
    }
}

impl QueuedPulse {
    // Takes a scheduled start time and converts into a scheduled signal
    // Requires taking the timers start time and calculating a duration
    // after the timer start time to fire the signal
    pub fn get_scheduled(&self, timer_running_time: Instant) -> ScheduledPulse {
        ScheduledPulse {
            alarm_time: self.time - timer_running_time,
            pulse_code: self.pulse_code,
        }
    }
}

impl PartialOrd for QueuedPulse {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.time.partial_cmp(&other.time)
    }
}

impl Ord for QueuedPulse {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time.cmp(&other.time)
    }
}

const QUEUE_SIZE: usize = 25;
pub struct PulseScheduler {
    timer: Mutex<RefCell<AnyTimer<'static>>>,
    pulse_queue: Mutex<RefCell<BinaryHeap<QueuedPulse, Min, QUEUE_SIZE>>>,
    timer_running_time: Mutex<Cell<Instant>>,
    current_scheduled_pulse: Mutex<Cell<Option<ScheduledPulse>>>,

    tx_channel_lock: Mutex<RefCell<()>>,
    tx_channel: RefCell<Option<rmt::Channel<'static, Blocking, Tx>>>,
    current_tx_transaction: RefCell<Option<ContinuousTxTransaction<'static>>>,

    late_events: AtomicU32,
    default_level: Level,
}

pub type RefPulseScheduler = Arc<Mutex<RefCell<PulseScheduler>>>;
static SLOT_0_PULSE_SCHEDULER: Mutex<RefCell<Option<RefPulseScheduler>>> =
    Mutex::new(RefCell::new(None));

#[handler(priority = esp_hal::interrupt::Priority::Priority3)]
fn interrupt_handler_slot_0() {
    critical_section::with(|cs| {
        let cell = SLOT_0_PULSE_SCHEDULER.borrow_ref(cs);
        if let Some(ref arc) = *cell {
            arc.borrow_ref_mut(cs).timer_interrupt(cs);
        }
    })
}

pub fn claim_interrupt_handler(pulse_scheduler: Arc<Mutex<RefCell<PulseScheduler>>>) {
    critical_section::with(|cs| {
        if SLOT_0_PULSE_SCHEDULER.borrow_ref(cs).is_none() {
            // Update interrupt
            pulse_scheduler
                .borrow(cs)
                .borrow_mut()
                .timer
                .borrow_ref_mut(cs)
                .set_interrupt_handler(interrupt_handler_slot_0);

            // Claim slot
            SLOT_0_PULSE_SCHEDULER
                .borrow_ref_mut(cs)
                .replace(pulse_scheduler);
        }
    });
}

pub enum ScheduleError {
    InThePast,
    NotIntialized,
}

impl PulseScheduler {
    fn execute_pulse(&mut self, pulse_code: PulseCode) {
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

            let data = [pulse_code];
            let transaction =
                tx_channel.transmit_continuously(&data, esp_hal::rmt::LoopMode::Finite(1));
            let Ok(transaction) = transaction else {
                // This means our pulse data was invalid
                panic!("Transaction error cannot continue");
            };

            self.current_tx_transaction = RefCell::new(Some(transaction));
        });
    }

    fn timer_interrupt(&mut self, cs: CriticalSection<'_>) {
        // The alarm was trigged
        let signal = self.current_scheduled_pulse.borrow(cs).take();
        if let Some(signal) = signal {
            self.execute_pulse(signal.pulse_code);

            // Counter will reset store our new timer start time
            let timer_now = self.timer.borrow_ref(cs).now();
            let new_start_time = self.timer_running_time.borrow(cs).get()
                + signal.alarm_time
                + timer_now.duration_since_epoch();
            self.timer_running_time.borrow(cs).set(new_start_time);
        }

        self.handle_schedule_dequeue(cs);
    }

    fn handle_schedule_dequeue(&mut self, cs: CriticalSection<'_>) {
        let timer_start_time = self.timer_running_time.borrow(cs).get();

        loop {
            let item = self.pulse_queue.borrow_ref_mut(cs).pop();

            let Some(dequed_signal) = item else {
                // No more pulses in the queue
                let timer = self.timer.borrow_ref(cs);
                // turn off the interrupts
                timer.enable_interrupt(false);
                break;
            };

            // Signal time is in the past
            if dequed_signal.time < self.now() {
                // Execute immediatly
                self.execute_pulse(dequed_signal.pulse_code);
                self.late_events.fetch_add(1, AtomicOrdering::Relaxed);
                continue;
            }

            // New item
            let scheduled_signal = dequed_signal.get_scheduled(timer_start_time);
            self.set_next_alarm_time(cs, scheduled_signal);
            break;
        }
    }

    fn set_next_alarm_time(&self, cs: CriticalSection<'_>, signal: ScheduledPulse) {
        self.current_scheduled_pulse.borrow(cs).set(Some(signal));
        let timer = self.timer.borrow_ref(cs);
        let _ = timer.load_value(signal.alarm_time);
        timer.clear_interrupt();
        timer.enable_interrupt(true);
    }

    fn get_pulse_width(&self, duration: Duration) -> u16 {
        let micros = duration.as_micros();
        (micros as u16).clamp(0, PulseCode::MAX_LEN)
    }

    // Schedules a level either set high o into the future
    // Please use the provided now function for getting instant times
    pub fn schedule_pulse(
        &self,
        time: Instant,
        pulse_duration: Duration,
    ) -> Result<(), ScheduleError> {
        critical_section::with(|cs| {
            if self.now() > time {
                // Can't schedule past current time
                warn!("Tried to schedule a pulse in the past");
                return Err(ScheduleError::InThePast);
            }

            let pulse_code = PulseCode::new(
                !self.default_level,
                self.get_pulse_width(pulse_duration),
                self.default_level,
                0,
            );

            let new_pulse = QueuedPulse::new(time, pulse_code);

            let timer_running_time = self.timer_running_time.borrow(cs).get();
            self.set_next_alarm_time(cs, new_pulse.get_scheduled(timer_running_time));
            // return Ok(());

            let currently_scheduled_pulse = self.current_scheduled_pulse.borrow(cs).get();
            let Some(currently_scheduled_pulse) = currently_scheduled_pulse else {
                self.set_next_alarm_time(cs, new_pulse.get_scheduled(timer_running_time));
                return Ok(());
            };

            // Gets the current scheduled time in relation to our running time
            let current_scheduled_time =
                currently_scheduled_pulse.scheduled_time(timer_running_time);

            let mut queue = self.pulse_queue.borrow_ref_mut(cs);
            // If our new item is after the current scheduled item
            if new_pulse.time > current_scheduled_time {
                // Next signal
                queue.push(new_pulse);
                return Ok(());
            }

            let new_scheduled_pulse = new_pulse.get_scheduled(timer_running_time);
            if new_pulse.time == current_scheduled_time {
                // Scheduled at the same time
                self.current_scheduled_pulse
                    .borrow(cs)
                    .set(Some(new_scheduled_pulse));
                return Ok(());
            }

            // New scheduled pulse has a time before our current
            // scheduled one set in the alarm time, update the alarm time to be the
            // new scheduled pulse
            self.set_next_alarm_time(cs, new_scheduled_pulse);

            // Store old scheduled pulse back into queue
            let queued_time =
                QueuedPulse::new(current_scheduled_time, currently_scheduled_pulse.pulse_code);
            queue.push(queued_time);
            Ok(())
        })
    }

    pub fn now(&self) -> Instant {
        critical_section::with(|cs| {
            let timer = self.timer.borrow_ref(cs);
            let timer_start_time = self.timer_running_time.borrow(cs).get();
            timer_start_time + timer.now().duration_since_epoch()
        })
    }

    pub fn new<Channel>(
        hardware_timer: impl Timer + Into<AnyTimer<'static>>,
        signal_output: Output<'static>,
        default_level: Level,
        rmt_channel: Channel,
    ) -> Result<RefPulseScheduler, ()>
    where
        Channel: TxChannelCreator<'static, Blocking> + Sized,
    {
        hardware_timer.enable_auto_reload(true);
        hardware_timer.clear_interrupt();
        hardware_timer.start();

        let timer = Mutex::new(RefCell::new(hardware_timer.into()));

        let tx_config = TxChannelConfig::default()
            .with_idle_output_level(default_level)
            .with_idle_output(true)
            .with_carrier_modulation(false)
            .with_clk_divider(80); // For 1us

        let tx_channel = rmt_channel.configure_tx(signal_output, tx_config);
        let Ok(tx_channel) = tx_channel else {
            return Err(());
        };
        let tx_channel = RefCell::new(Some(tx_channel));

        let pulse_scheduler = Self {
            timer,
            pulse_queue: Mutex::new(RefCell::new(BinaryHeap::new())),
            timer_running_time: Mutex::new(Cell::new(Instant::EPOCH)),
            current_scheduled_pulse: Mutex::new(Cell::new(None)),
            tx_channel_lock: Mutex::new(RefCell::new(())),
            tx_channel: tx_channel,
            current_tx_transaction: RefCell::new(None),
            late_events: AtomicU32::new(0),
            default_level: default_level,
        };
        let scheduler = Arc::new(Mutex::new(RefCell::new(pulse_scheduler)));

        claim_interrupt_handler(scheduler.clone());
        Ok(scheduler)
    }
}
