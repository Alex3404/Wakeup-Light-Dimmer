use embassy_futures::select::{Either3};
use embassy_time::{Duration, Instant, WithTimeout};
use esp_hal::gpio::Input;
use fixed::types::{I24F8};

use super::RoteryInterface;

/// Rotery decoder config
/// ## Overview
/// This configuration requires a clock and direction input pin
/// There is an optional switch pin since not all rotery encoders
/// have switch support.
pub struct RoteryDecoderConfig<'a, I>
 where I : RoteryInterface {
    clock: Input<'a>,
    direction: Input<'a>,
    switch: Option<Input<'a>>,

    debounce: Option<Duration>,
    steps_per_rotation: u16,
    interface: &'a I,
}

impl<'a, I> RoteryDecoderConfig<'a, I>
where I : RoteryInterface {
    /// Create a new rotation decoder config
    pub fn new(
        clock: Input<'a>,
        direction: Input<'a>,
        interface: &'a I,
        steps_per_rotation: u16,
    ) -> Self {
        Self {
            clock,
            direction,
            switch: None,
            debounce: None,
            steps_per_rotation,
            interface,
        }
    }

    /// Assign a optional switch input
    pub fn with_switch(self, switch: Input<'a>) -> Self {
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
pub struct RoteryDecoder<'a, I>
    where I : RoteryInterface {
    state: RoteryState,

    steps_per_rotation : u16,
    debounce: Option<Duration>,
    interface: &'a I,

    clock: Input<'a>,
    direction: Input<'a>,
    switch: Option<Input<'a>>,
}

impl<'a, I> RoteryDecoder<'a, I>
    where I : RoteryInterface {
    pub fn new(config: RoteryDecoderConfig<'a, I>) -> Self {
        Self {
            steps_per_rotation: config.steps_per_rotation,
            debounce: config.debounce,
            interface: config.interface,
            state: RoteryState::default(),
            clock: config.clock,
            direction: config.direction,
            switch: config.switch,
        }
    }

    pub async fn run_loop(&mut self) {
        loop {
            // See which input has an edge first then handle it
            let select = embassy_futures::select::select3(
                wait_for_edge(&mut self.clock, self.debounce),
                wait_for_edge(&mut self.direction, self.debounce),
                async {
                    if let Some(sw) = self.switch.as_mut() {
                        wait_for_edge(sw, self.debounce).await
                    } else {
                        core::future::pending::<bool>().await
                    }
                }
            ).await;

            // Handle the selected input edge
            let result = match select {
                Either3::First(clock_state) => self.state.clock_changed(clock_state),
                Either3::Second(rotate_state) => self.state.rotate_changed(rotate_state),
                Either3::Third(switch_state) => {
                    // Handle switch press event
                    self.interface.pressed(switch_state);
                    None
                }
            };

            let Some(rotation) = result else {
                continue;
            };

            let now = Instant::now();
            let delta = now.saturating_duration_since(self.state.last_rotate);
            self.state.last_rotate = now;
            
            // Calculate the RPM in a fixed point fraction
            const MILLISECONDS_PER_MINUTE: I24F8 = I24F8::const_from_int(60_000); 
            let steps_per_rotation = I24F8::saturating_from_num(self.steps_per_rotation);
            let factor = I24F8::saturating_from_num(delta.as_millis()).saturating_mul(steps_per_rotation);
            let rpm = MILLISECONDS_PER_MINUTE.checked_div(factor).unwrap_or(I24F8::const_from_int(0));

            match rotation {
                Rotation::Clockwise => self.interface.rotate_cw(rpm),
                Rotation::Counterclockwise => self.interface.rotate_ccw(rpm),
            }
        }    
    }
}

/// State machine for rotery decoder
#[derive(Debug, Clone, PartialEq, Eq)]
struct RoteryState {
    last_rotate: Instant,
    clock_state: bool,
    rotate_state: bool,
    fired: bool,
}

impl Default for RoteryState {
    fn default() -> Self {
        Self {
            last_rotate: Instant::now(),
            clock_state: false,
            rotate_state: false,
            fired: false,
        }
    }
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

        Some(Rotation::Counterclockwise)
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

        Some(Rotation::Clockwise)
    }
}

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