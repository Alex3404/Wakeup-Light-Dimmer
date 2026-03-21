use core::result::Result;
use core::result::Result::Ok;

use core::cell::{Cell, RefCell};
use core::cmp::{Eq, Ord, Ordering, PartialEq, PartialOrd, Reverse};
use core::marker::Copy;
use core::matches;
use core::option::Option;
use core::option::Option::{None, Some};
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering as AtomicOrdering;

use critical_section::CriticalSection;
use critical_section::Mutex;
use esp_hal::gpio::{Level, Output};
use esp_hal::handler;
use esp_hal::interrupt::Priority;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::{AnyTimer, Timer};

extern crate alloc;
use alloc::collections::binary_heap::BinaryHeap;
use log::{debug, info, warn};

#[derive(PartialEq, Eq, Clone, Copy)]
struct QueuedSignal {
    pub time: Instant,
    pub level: Level,
}

impl QueuedSignal {
    fn new(time: Instant, level: Level) -> Self {
        Self { time, level }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct ScheduledSignal {
    alarm_time: Duration,
    level: Level,
}

impl ScheduledSignal {
    pub fn scheduled_time(&self, timer_start_time: Instant) -> Instant {
        timer_start_time + self.alarm_time
    }
}

impl QueuedSignal {
    // Takes a scheduled start time and converts into a scheduled signal
    // Requires taking the timers start time and calculating a duration
    // after the timer start time to fire the signal
    pub fn to_scheduled(&self, timer_start_time: Instant) -> ScheduledSignal {
        ScheduledSignal {
            alarm_time: self.time - timer_start_time,
            level: self.level,
        }
    }
}

impl PartialOrd for QueuedSignal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.time.partial_cmp(&other.time)
    }
}

impl Ord for QueuedSignal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time.cmp(&other.time)
    }
}

/////////////////////////
/// HARDWARE POINTERS ///
/////////////////////////

// Timer for microsecond firing of a pin
type PulseTimer = AnyTimer<'static>;
static TIMER: Mutex<RefCell<Option<PulseTimer>>> = Mutex::new(RefCell::new(None));
static SIGNAL_PIN: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));

static DEFAULT_LEVEL: Mutex<Cell<Level>> = Mutex::new(Cell::new(Level::Low));
static LATE_EVENTS: AtomicU32 = AtomicU32::new(0);

// Min queue to find the next Signal
type SignalQueue = BinaryHeap<Reverse<QueuedSignal>>;
static SIGNAL_QUEUE: Mutex<RefCell<SignalQueue>> = Mutex::new(RefCell::new(BinaryHeap::new()));

static TIMER_START_TIME: Mutex<Cell<Instant>> = Mutex::new(Cell::new(Instant::EPOCH));
static SCHEDULED_SIGNAL: Mutex<Cell<Option<ScheduledSignal>>> = Mutex::new(Cell::new(None));

// Highest priority since timing is crutial
#[handler(priority = Priority::Priority3)]
fn interrupt_handler() {
    critical_section::with(|cs| {
        // The alarm was trigged
        let signal = SCHEDULED_SIGNAL.borrow(cs).get();
        if let Some(signal) = signal {
            if let Some(ref mut pin) = *SIGNAL_PIN.borrow_ref_mut(cs) {
                pin.set_level(signal.level);
            }

            // Counter will reset store our new timer start time
            let new_start_time = TIMER_START_TIME.borrow(cs).get() + signal.alarm_time;
            TIMER_START_TIME.borrow(cs).set(new_start_time);
        }

        SCHEDULED_SIGNAL.borrow(cs).set(None);
        dequeue_and_schedule_interrupt(cs);
    });
}

fn schedule_next_interrupt(cs: CriticalSection<'_>, timer: &PulseTimer, signal: ScheduledSignal) {
    timer.clear_interrupt();

    // #[cfg(debug_assertions)]
    // {
    //     let timer_start_time = TIMER_START_TIME.borrow(cs).get();
    //     info!(
    //         "Alarm in: {:7} ms | Real Time: {:7}ms | Now: {:7}ms",
    //         signal.alarm_time.as_millis(),
    //         (timer_start_time + signal.alarm_time)
    //             .duration_since_epoch()
    //             .as_millis(),
    //         now().duration_since_epoch().as_millis()
    //     );
    // }

    let _ = timer.load_value(signal.alarm_time);
    timer.enable_interrupt(true);
    SCHEDULED_SIGNAL.borrow(cs).set(Some(signal));
}

fn dequeue_and_schedule_interrupt(cs: CriticalSection<'_>) {
    let timer_cell = TIMER.borrow_ref(cs);
    let Some(ref timer) = *timer_cell else {
        return;
    };

    let timer_start_time = TIMER_START_TIME.borrow(cs).get();
    let mut queue = SIGNAL_QUEUE.borrow_ref_mut(cs);

    loop {
        let item = queue.pop();
        let Some(dequed_signal) = item else {
            timer.enable_interrupt(false);
            break;
        };

        // Signal time is in the past
        if dequed_signal.0.time < now() {
            // Execute immediately
            if let Some(ref mut pin) = *SIGNAL_PIN.borrow_ref_mut(cs) {
                pin.set_level(dequed_signal.0.level);
            }

            LATE_EVENTS.fetch_add(1, AtomicOrdering::Relaxed);
            continue;
        }

        let scheduled_signal = dequed_signal.0.to_scheduled(timer_start_time);
        schedule_next_interrupt(cs, &timer, scheduled_signal);
        break;
    }
}

pub fn now() -> Instant {
    critical_section::with(|cs| {
        let timer_cell = TIMER.borrow_ref(cs);
        let timer_start_time = TIMER_START_TIME.borrow(cs).get();
        let Some(ref timer) = *timer_cell else {
            return Instant::EPOCH;
        };

        timer_start_time + timer.now().duration_since_epoch()
    })
}

pub enum ScheduleError {
    InThePast,
    NotIntialized,
}

// Schedules a level either set high o into the future
// Please use the provided now function for getting instant times
pub fn schedule_set_level(time: Instant, level: Level) -> Result<(), ScheduleError> {
    let signal = QueuedSignal::new(time, level);

    critical_section::with(|cs| {
        let timer_cell = TIMER.borrow_ref(cs);
        let Some(ref timer) = *timer_cell else {
            return Err(ScheduleError::NotIntialized);
        };

        if now() > time {
            // Can't schedule past current time
            warn!("Tried to schedule a pulse in the past");
            return Err(ScheduleError::InThePast);
        }

        let mut queue = SIGNAL_QUEUE.borrow_ref_mut(cs);
        let timer_start_time = TIMER_START_TIME.borrow(cs).get();

        let current_scheduled = SCHEDULED_SIGNAL.borrow(cs).get();
        let Some(mut scheduled) = current_scheduled else {
            schedule_next_interrupt(cs, timer, signal.to_scheduled(timer_start_time));
            return Ok(());
        };

        let current_scheduled_time = scheduled.scheduled_time(timer_start_time);

        // If our new item is after the current scheduled item
        if signal.time > current_scheduled_time {
            // Next signal
            queue.push(Reverse(signal));
            return Ok(());
        }

        if signal.time == current_scheduled_time {
            // Scheduled at the same time
            if signal.level != scheduled.level {
                scheduled.level = signal.level;
                // Update currently scheduled level
                SCHEDULED_SIGNAL.borrow(cs).set(Some(scheduled));
            }
            return Ok(());
        }

        let scheduled_signal = signal.to_scheduled(timer_start_time);

        // New scheduled signal arrived before
        // the current signal stored in the interrupt
        schedule_next_interrupt(cs, timer, scheduled_signal);

        // Store old interrupt back into queue
        let queued_time = QueuedSignal::new(current_scheduled_time, scheduled.level);
        queue.push(Reverse(queued_time));
        Ok(())
    })
}

// Schedules a pulse into the future
// Please use the provided now function for getting instant times
pub fn schedule_pulse(start: Instant, duration: Duration) -> Result<(), ScheduleError> {
    let default_level = critical_section::with(|cs| DEFAULT_LEVEL.borrow(cs).get());
    schedule_set_level(start, !default_level)?;
    schedule_set_level(start + duration, default_level)?;
    Ok(())
}

pub fn initalize(
    hardware_timer: AnyTimer<'static>,
    signal_output: Output<'static>,
    default_level: Level,
) {
    info!("Initalizing pulse scheduler!");

    critical_section::with(|cs| {
        hardware_timer.set_interrupt_handler(interrupt_handler);
        hardware_timer.enable_auto_reload(true);
        hardware_timer.clear_interrupt();
        hardware_timer.start();

        SIGNAL_QUEUE.borrow_ref_mut(cs).clear();
        SCHEDULED_SIGNAL.borrow(cs).set(None);
        TIMER.borrow_ref_mut(cs).replace(hardware_timer);
        SIGNAL_PIN.borrow_ref_mut(cs).replace(signal_output);
        DEFAULT_LEVEL.borrow(cs).set(default_level);
    });

    info!("Initalized pulse scheduler!");
}
