use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};
use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::{Duration, WithTimeout};
use esp_hal::gpio::Input;

extern crate alloc;
use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
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
    rotation_handler: Option<RotationHandler>,
    switch_handler: Option<SwitchHandler>,
}

impl<'d> RoteryDecoderConfig<'d> {
    /// Create a new rotation decoder config
    pub fn new(clock: Input<'d>, direction: Input<'d>) -> Self {
        Self {
            clock,
            direction,
            switch: None,
            debounce: None,
            rotation_handler: None,
            switch_handler: None,
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

    /// Assign a rotation handler
    pub fn with_rotate_handler(self, rotate_handler: RotationHandler) -> Self {
        Self {
            rotation_handler: Some(rotate_handler),
            ..self
        }
    }

    /// Assign a switch handler
    pub fn with_switch_handler(self, switch_handler: SwitchHandler) -> Self {
        Self {
            switch_handler: Some(switch_handler),
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
    _state: Rc<RoteryStateRef>,
    _options: Rc<RoteryOptionsRef>,
}

impl RoteryDecoder {
    pub fn create(
        spawner: Spawner,
        config: RoteryDecoderConfig<'static>,
    ) -> Result<Self, RoteryDecoderNewError> {
        if is_at_max_decoders() {
            return Err(RoteryDecoderNewError::MaxNumberOfInstances);
        }

        let state = Rc::new(NoopMutex::new(RefCell::new(RoteryState {
            clock_state: false,
            rotate_state: false,
            fired: false,
        })));

        let options = Rc::new(NoopMutex::new(RefCell::new(RoteryOptions {
            debounce: config.debounce,
            rotation_handler: config.rotation_handler,
            switch_handler: config.switch_handler,
        })));

        info!("Spawn tasks!");
        let clock = config.clock;
        let direction = config.direction;
        if let Some(switch) = config.switch {
            spawner.must_spawn(decoder_switch_loop(
                spawner,
                Rc::downgrade(&options),
                switch,
            ));
        };

        spawner.must_spawn(decoder_loop(
            spawner,
            Rc::downgrade(&state),
            Rc::downgrade(&options),
            clock,
            direction,
        ));

        Ok(Self {
            _options: options,
            _state: state,
        })
    }
}

type RoteryStateRef = NoopMutex<RefCell<RoteryState>>;

/// State machine for rotery decoder
struct RoteryState {
    clock_state: bool,
    rotate_state: bool,
    fired: bool,
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

type RoteryOptionsRef = NoopMutex<RefCell<RoteryOptions>>;
struct RoteryOptions {
    debounce: Option<Duration>,
    rotation_handler: Option<RotationHandler>,
    switch_handler: Option<SwitchHandler>,
}

/// The maximum number of decoders allowed to be created at at time
/// This limit is mostly because we need to limit the number
/// of concurrent embassy tasks for gpio input
/// This can be updated based on the prodject
/// More decoders means more RAM usage
pub const MAX_DECODERS: usize = 3;
static DECODER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Error for
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
            options.lock(|options| options.borrow().debounce)
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
            let Some(ref handler) = options.borrow().rotation_handler else {
                return;
            };

            handler(spawner, rotation);
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
            options.lock(|options| options.borrow().debounce)
        } else {
            // Our rotery decoder has been dropped
            break;
        };

        let switch_state = wait_for_edge(&mut switch_pin, debounce).await;
        if let Some(options) = options.upgrade() {
            // Fire the switch handler on an edge Low -> High, High -> Low
            options.lock(|options| {
                let Some(ref handler) = options.borrow().switch_handler else {
                    return;
                };

                handler(spawner, switch_state);
            })
        } else {
            // Our rotery decoder has been dropped
            break;
        };
    }
}
