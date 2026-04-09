use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};
use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::{Duration, WithTimeout};
use esp_hal::gpio::Input;

use super::RoteryInterface;

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use log::info;

/// Rotery decoder config
/// ## Overview
/// This configuration requires a clock and direction input pin
/// There is an optional switch pin since not all rotery encoders
/// have switch support.
pub struct RoteryDecoderConfig<'d> {
    clock: Input<'d>,
    direction: Input<'d>,
    switch: Option<Input<'d>>,

    debounce: Option<Duration>,
    interface: Box<dyn RoteryInterface>,
}

impl<'d> RoteryDecoderConfig<'d> {
    /// Create a new rotation decoder config
    pub fn new(
        clock: Input<'d>,
        direction: Input<'d>,
        interface: impl RoteryInterface + 'static,
    ) -> Self {
        Self {
            clock,
            direction,
            switch: None,
            debounce: None,
            interface: Box::new(interface),
        }
    }

    /// Assign a optional switch input
    pub fn with_switch(self, switch: Input<'d>) -> Self {
        Self {
            switch: Some(switch),
            ..self
        }
    }

    pub fn with_debounce(self, debounce: Duration) -> Self {
        Self {
            debounce: Some(debounce),
            ..self
        }
    }
}

/// Rotery decoder
///
/// ## Overview
///
/// This decodes clock wise and counter clockwise events from a rotery decoder.
/// If there is a switch given via the config you can assign a switch handler
pub struct RoteryDecoder {
    _state: Arc<RoteryStateRef>,
    _options: Arc<RoteryOptionsRef>,
}

impl RoteryDecoder {
    pub fn new(
        spawner: Spawner,
        config: RoteryDecoderConfig<'static>,
    ) -> Result<Self, RoteryDecoderNewError> {
        if is_at_max_decoders() {
            return Err(RoteryDecoderNewError::MaxNumberOfInstances);
        }

        let state = Arc::new(NoopMutex::new(RefCell::new(RoteryState {
            clock_state: false,
            rotate_state: false,
            fired: false,
        })));

        let options = Arc::new(NoopMutex::new(RoteryOptions {
            debounce: config.debounce,
            interface: RefCell::new(config.interface),
        }));

        info!("Spawn tasks!");

        let clock = config.clock;
        let direction = config.direction;
        if let Some(switch) = config.switch {
            spawner.spawn(decoder_switch_loop(
                spawner,
                Arc::downgrade(&options),
                switch,
            ));
        };

        spawner.must_spawn(decoder_loop(
            spawner,
            Arc::downgrade(&state),
            Arc::downgrade(&options),
            clock,
            direction,
        ));

        Ok(Self {
            _options: options,
            _state: state,
        })
    }
}

impl Drop for RoteryDecoder {
    fn drop(&mut self) {}
}

type RoteryStateRef = NoopMutex<RefCell<RoteryState>>;

/// State machine for rotery decoder
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RoteryState {
    clock_state: bool,
    rotate_state: bool,
    fired: bool,
}

type RoteryOptionsRef = NoopMutex<RoteryOptions>;
struct RoteryOptions {
    debounce: Option<Duration>,
    interface: RefCell<Box<dyn RoteryInterface>>,
}

impl RoteryState {
    fn clock_changed(&mut self, new_clock_state: bool) -> Option<Rotation> {
        if new_clock_state {
            // Rising edge
            self.clock_state = false;
            if !self.rotate_state {
                self.fired = false;
            }
            return None;
        }

        // Falling edge
        if self.clock_state {
            return None;
        }
        self.clock_state = true;

        if !self.rotate_state || self.fired {
            return None;
        }
        self.fired = true;

        return Some(Rotation::Counterclockwise);
    }

    fn rotate_changed(&mut self, new_rotate_state: bool) -> Option<Rotation> {
        if new_rotate_state {
            // Rising edge
            self.rotate_state = false;
            if !self.clock_state {
                self.fired = false;
            }

            return None;
        }

        // Falling edge
        if self.rotate_state {
            return None;
        }
        self.rotate_state = true;

        if !self.clock_state || self.fired {
            return None;
        }
        self.fired = true;

        return Some(Rotation::Clockwise);
    }
}

/// The maximum number of decoders allowed to be created at at time
/// This limit is mostly because we need to limit the number
/// of concurrent embassy tasks for gpio input
/// This can be updated based on the prodject
/// More decoders means more RAM usage
pub const MAX_DECODERS: usize = 3;
static DECODER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Error for creating a new rotery decoder
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoteryDecoderNewError {
    MaxNumberOfInstances,
}

fn is_at_max_decoders() -> bool {
    let mut count = DECODER_COUNT.load(Ordering::Relaxed);
    loop {
        let new_count = count + 1;
        if new_count > MAX_DECODERS {
            return true;
        }

        let exchange =
            DECODER_COUNT.compare_exchange(count, new_count, Ordering::SeqCst, Ordering::Relaxed);
        match exchange {
            Ok(_) => {
                return new_count == MAX_DECODERS;
            }
            Err(value) => count = value,
        };
    }
}

pub type RotationHandler = Box<dyn Fn(Spawner, Rotation)>;
pub type SwitchHandler = Box<dyn Fn(Spawner, bool)>;

#[derive(Clone, Copy, Debug)]
pub enum Rotation {
    Clockwise,
    Counterclockwise,
}

/// Basic wait for any edge with debounce
async fn wait_for_edge(input: &mut Input<'_>, debounce: Option<Duration>) -> bool {
    loop {
        let old_state = input.is_high();
        input.wait_for_any_edge().await;

        let changed = if let Some(time) = debounce {
            let timeout = input.wait_for_any_edge().with_timeout(time);
            timeout.await.is_err() && old_state != input.is_high()
        } else {
            old_state != input.is_high()
        };

        if changed {
            break input.is_high();
        }
    }
}

///////////////////////////////////////////
/// Tasks for checking for gpio updates ///
///////////////////////////////////////////
#[embassy_executor::task(pool_size = MAX_DECODERS)]
async fn decoder_loop(
    spawner: Spawner,
    state: Weak<RoteryStateRef>,
    options: Weak<RoteryOptionsRef>,
    mut clock: Input<'static>,
    mut rotate: Input<'static>,
) {
    loop {
        // Get the debounce time
        let debounce = if let Some(options) = options.upgrade() {
            options.lock(|options| options.debounce)
        } else {
            // Our rotery decoder has been dropped
            break;
        };

        // See which input has an edge first then handle it
        let select = embassy_futures::select::select(
            wait_for_edge(&mut clock, debounce),
            wait_for_edge(&mut rotate, debounce),
        )
        .await;

        // Try to get our state after waiting
        let (Some(state), Some(options)) = (state.upgrade(), options.upgrade()) else {
            // Our rotery decoder has been dropped
            break;
        };

        // Update decoder state with new input
        let result = match select {
            Either::First(clock_state) => {
                state.lock(|state| state.borrow_mut().clock_changed(clock_state))
            }
            Either::Second(rotate_state) => {
                state.lock(|state| state.borrow_mut().rotate_changed(rotate_state))
            }
        };
        let Some(rotation) = result else {
            continue;
        };

        // Fire rotation handler with rotation
        options.lock(|options| {
            let mut handler = options.interface.borrow_mut();
            match rotation {
                Rotation::Clockwise => handler.rotate_cw(spawner),
                Rotation::Counterclockwise => handler.rotate_ccw(spawner),
            }
        })
    }
}

#[embassy_executor::task(pool_size = MAX_DECODERS)]
async fn decoder_switch_loop(
    spawner: Spawner,
    options: Weak<RoteryOptionsRef>,
    mut switch_pin: Input<'static>,
) {
    loop {
        // Get the debounce time
        let debounce = if let Some(options) = options.upgrade() {
            options.lock(|options| options.debounce)
        } else {
            // Our rotery decoder has been dropped
            break;
        };

        let switch_state = wait_for_edge(&mut switch_pin, debounce).await;

        if let Some(options) = options.upgrade() {
            // Fire the switch handler on an edge Low -> High, High -> Low
            options.lock(|options| {
                let mut handler = options.interface.borrow_mut();
                handler.pressed(switch_state, spawner)
            })
        } else {
            // Our rotery decoder has been dropped
            break;
        };
    }
}
