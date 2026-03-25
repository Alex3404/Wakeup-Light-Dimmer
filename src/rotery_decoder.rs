use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::{Mutex, NoopMutex};
use embassy_time::{Duration, Timer, WithTimeout};
use esp_hal::gpio::{Event, Input};

extern crate alloc;
use alloc::boxed::Box;
use alloc::rc::Rc;
use log::info;

/// The maximum number of decoders allowed to be created at at time
/// This limit is mostly because we need to limit the number
/// of concurrent embassy tasks for gpio input
/// This can be updated based on the prodject
/// More decoders means more RAM usage
pub const MAX_DECODERS: usize = 3;
static DECODER_COUNT: AtomicUsize = AtomicUsize::new(0);

pub type RotationChangedTask = Box<dyn Fn(Rotation, Spawner)>;
pub type SwitchChangedTask = Box<dyn Fn(bool, Spawner)>;
pub type RoteryDecoderReference = Rc<NoopMutex<RefCell<RoteryDecoder>>>;

#[derive(Clone, Copy, Debug)]
pub enum Rotation {
    Clockwise,
    Counterclockwise,
}

pub struct RoteryDecoder {
    pin_a_state: bool,
    pin_b_state: bool,
    fired_rotation: bool,

    rotation_task: Option<RotationChangedTask>,
    switch_task: Option<SwitchChangedTask>,
}

pub enum RoteryDecoderCreateError {
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

impl RoteryDecoder {
    pub fn create(
        spawner: Spawner,
        a_pin: Input<'static>,
        b_pin: Input<'static>,
        switch_pin: Input<'static>,
    ) -> Result<RoteryDecoderReference, RoteryDecoderCreateError> {
        if is_at_max_decoders() {
            return Err(RoteryDecoderCreateError::MaxNumberOfInstances);
        }

        let decoder = Rc::new(Mutex::new(RefCell::new(Self {
            pin_a_state: false,
            pin_b_state: false,
            fired_rotation: false,

            rotation_task: None,
            switch_task: None,
        })));

        info!("Spawn tasks!");
        spawner.must_spawn(pin_a_loop(spawner, decoder.clone(), a_pin));
        spawner.must_spawn(pin_b_loop(spawner, decoder.clone(), b_pin));
        spawner.must_spawn(switch_pin_loop(spawner, decoder.clone(), switch_pin));

        Ok(decoder.clone())
    }

    pub fn add_rotation_event(&mut self, task: RotationChangedTask) {
        self.rotation_task = Some(task);
    }

    pub fn on_switch(&mut self, task: SwitchChangedTask) {
        self.switch_task = Some(task)
    }
}

///////////////////////////////////////////
/// Tasks for checking for gpio updates ///
///////////////////////////////////////////
#[embassy_executor::task(pool_size = MAX_DECODERS)]
async fn pin_a_loop(spawner: Spawner, this: RoteryDecoderReference, mut pin_a: Input<'static>) {
    loop {
        pin_a.wait_for(Event::AnyEdge).await;
        let pin_a_state = pin_a.is_high();
        this.lock(|decoder| {
            let mut decoder_ref = decoder.borrow_mut();
            if pin_a_state {
                // Rising edge
                decoder_ref.pin_a_state = false;
                if !decoder_ref.pin_b_state {
                    decoder_ref.fired_rotation = false;
                }
            } else {
                // Falling edge
                if decoder_ref.pin_a_state {
                    return;
                }
                decoder_ref.pin_a_state = true;

                if decoder_ref.pin_b_state && !decoder_ref.fired_rotation {
                    decoder_ref.fired_rotation = true;

                    let Some(ref task) = decoder_ref.rotation_task else {
                        return;
                    };
                    task(Rotation::Clockwise, spawner);
                }
            }
        });
    }
}

#[embassy_executor::task(pool_size = MAX_DECODERS)]
async fn pin_b_loop(spawner: Spawner, this: RoteryDecoderReference, mut pin_b: Input<'static>) {
    loop {
        pin_b.wait_for(Event::AnyEdge).await;
        let pin_b_state = pin_b.is_high();
        this.lock(|decoder| {
            let mut decoder_ref = decoder.borrow_mut();
            if pin_b_state {
                // Rising edge
                decoder_ref.pin_b_state = false;
                if !decoder_ref.pin_a_state {
                    decoder_ref.fired_rotation = false;
                }
            } else {
                // Falling edge
                if decoder_ref.pin_b_state {
                    return;
                }
                decoder_ref.pin_b_state = true;

                if decoder_ref.pin_a_state && !decoder_ref.fired_rotation {
                    decoder_ref.fired_rotation = true;

                    let Some(ref task) = decoder_ref.rotation_task else {
                        return;
                    };
                    task(Rotation::Counterclockwise, spawner);
                }
            }
        });
    }
}

#[embassy_executor::task(pool_size = MAX_DECODERS)]
async fn switch_pin_loop(
    spawner: Spawner,
    this: RoteryDecoderReference,
    mut rotate_pin: Input<'static>,
) {
    loop {
        rotate_pin.wait_for(Event::AnyEdge).await;
        this.lock(|decoder| {
            let Some(ref task) = decoder.borrow().switch_task else {
                return;
            };
            task(rotate_pin.is_high(), spawner);
        })
    }
}
