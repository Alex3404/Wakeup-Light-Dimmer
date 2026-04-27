use super::{
    DimmerChannelConfig, DimmerSettings, DimmerState, TimingConfig,
    dimmer_settings_builder::DimmerSettingsBuilder, lookup_tables::LookupTables,
    rolling_average::TimeRollingAverage,
};

use core::{cell::RefCell, sync::atomic::AtomicBool};
use core::{sync::atomic::Ordering, u16};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Duration;

use critical_section::Mutex as CSMutex;
use esp_hal::gpio::interconnect::{InputSignal, OutputSignal};
use esp_hal::handler;

use esp_hal::mcpwm::capture::{CaptureChannelConfig, CaptureMode, CaptureTimerConfig};
use esp_hal::mcpwm::mcpwm0;
use esp_hal::mcpwm::operator::{PwmActions, PwmPinConfig, PwmUpdateMethod, UpdateAction};
use esp_hal::mcpwm::timer::{PeriodUpdatingMethod, PwmWorkingMode};
use esp_hal::mcpwm::{McPwm, PeripheralClockConfig};

use esp_hal::peripherals::MCPWM0;
use esp_hal::time::Rate;
use log::info;

use static_cell::StaticCell;

static DIMMER_CHANNEL: StaticCell<DimmerChannel> = StaticCell::new();

pub struct DimmerChannel {
    pwm_signals: &'static DimmerPwmSignals,
    builder_active: AtomicBool,

    current_state: RefCell<DimmerState>,
    current_settings: RefCell<DimmerSettings>,

    frequency: u8,
    timing_config: TimingConfig,
    spawner: Spawner,
}

/// This is a light dimmer channel, it can be used to set the dimming
/// of a triac circit. For example a Triac such as the BTA16-600B
/// with the gate connected via a octocoupled random-phase triac such as a
/// H11AA1 with its main terminal inputs in series with two 33k ohm resistors.
///
/// The gate pin given to the light dimmer channel will be the output pin for phase
/// angle control, should be connected to the octocoupled random-phase triac
///
/// The zero cross pin given to the light dimmer channel will be the input pin
/// for detecting a zero cross signal used for timing the pulses of the phase
/// angle control. This input pin expects the external circitry to provide a pull up
/// resistor. Since Current Transfer Ratios can vary and the internal resistor may be
/// too high of a resistance for a reliable zero-cross pulse.
///
/// The zero cross is expected to be a logical high ( voltage > 2.5V ) pulse centered around
/// the zero-cross point of the AC input. The exact pulse time doesn't matter an
/// average time of the pulse will be taken and used for calculating the estimated
/// zero cross time. The estimation of the zero cross is half of the pulse high time
/// after the rising edge. So a longer pulse time gives the microcontroller more time
/// to run its code before the zero-crossing point.
///
/// If the zero cross high pulse is too short its possible for the micro controller
/// to not be given enough time after the rising edge of the pulse to calculate
/// the phase angle for the next cycle.
///
impl DimmerChannel {
    pub fn new(spawner: Spawner, config: DimmerChannelConfig) -> &'static Self {
        // Leak the pwm signals so they can be shared with interrupt handler
        static PWM_SIGNALS: StaticCell<DimmerPwmSignals> = StaticCell::new();
        let pwm_signals = PWM_SIGNALS.init(DimmerPwmSignals::new());

        let lookup_tables = config
            .timing_config
            .create_lookup_tables(config.frequency, &config.dimmer_settings);

        DimmerPwmHandler::configure(
            config.mcpwm,
            config.zero_cross,
            config.gate,
            &config.starting_state,
            lookup_tables,
            pwm_signals,
        );

        // Leak dimmer channel so it can be shared with builder
        DIMMER_CHANNEL.init(Self {
            pwm_signals,
            frequency: config.frequency,
            timing_config: config.timing_config,
            builder_active: AtomicBool::new(false),
            current_state: RefCell::new(config.starting_state),
            current_settings: RefCell::new(config.dimmer_settings),
            spawner,
        })
    }

    pub fn settings_builder(&'static self) -> Result<DimmerSettingsBuilder, ()> {
        if self.builder_active.fetch_or(true, Ordering::Relaxed) {
            // Another builder is already active, return an error
            return Err(());
        }

        let builder = DimmerSettingsBuilder::new(self, self.spawner.clone());

        Ok(builder)
    }

    pub fn update_state(&self, state: DimmerState) {
        *self.current_state.borrow_mut() = state;
        if !self.builder_active.load(Ordering::Relaxed) {
            self.pwm_signals.new_state.signal(state);
        }
    }

    pub fn update_settings(&self, settings: &DimmerSettings) {
        *self.current_settings.borrow_mut() = *settings;
        if !self.builder_active.load(Ordering::Relaxed) {
            let lookup_tables = self
                .timing_config
                .create_lookup_tables(self.frequency, settings);
            self.pwm_signals.new_lookup.signal(lookup_tables);
        }
    }

    pub fn get_settings(&self) -> DimmerSettings {
        *self.current_settings.borrow()
    }

    pub fn get_state(&self) -> DimmerState {
        *self.current_state.borrow()
    }

    pub fn update_brightness(&self, func: impl FnOnce(u8) -> u8) {
        let mut state = self.get_state();
        state.brightness = func(state.brightness);
        self.update_state(state);
    }

    pub fn toggle_on_off(&self) {
        let mut state = self.get_state();
        state.is_on = !state.is_on;
        self.update_state(state);
    }

    pub fn set_on_off(&self, is_on: bool) {
        let mut state = self.get_state();
        state.is_on = is_on;
        self.update_state(state);
    }

    pub fn set_brightness(&self, brightness: u8) {
        let mut state = self.get_state();
        state.brightness = brightness;
        self.update_state(state);
    }

    pub(super) fn builder_cancelled(&self) {
        self.builder_active.store(false, Ordering::Relaxed);

        self.update_settings(&*self.current_settings.borrow());
        self.update_state(*self.current_state.borrow());
    }

    pub(super) fn builder_published(&self, settings: DimmerSettings) {
        self.builder_active.store(false, Ordering::Relaxed);

        self.update_settings(&settings);
        self.update_state(*self.current_state.borrow());
    }
}

type DimmerSlot<const SLOT: u8> = CSMutex<RefCell<Option<DimmerPwmHandler<SLOT>>>>;
static DIMMER_PWM: DimmerSlot<0> = CSMutex::new(RefCell::new(None));

#[handler]
fn mcpwm_interrupt() {
    critical_section::with(|cs| {
        if let Some(ref mut dimmer) = *DIMMER_PWM.borrow_ref_mut(cs) {
            dimmer.interrupt();
        }
    })
}

pub struct DimmerPwmSignals {
    new_state: Signal<CriticalSectionRawMutex, DimmerState>,
    new_lookup: Signal<CriticalSectionRawMutex, LookupTables>,
}

impl DimmerPwmSignals {
    pub fn new() -> Self {
        Self {
            new_state: Signal::new(),
            new_lookup: Signal::new(),
        }
    }

    pub fn signal_new_state(&self, state: DimmerState) {
        self.new_state.signal(state);
    }

    pub fn signal_new_lookup(&self, lookup: LookupTables) {
        self.new_lookup.signal(lookup);
    }
}

/// Reference to lamp dimmer state for MCPWM interrupt handler
/// Only 3 slot are available due to hardware limitations
struct DimmerPwmHandler<const SLOT: u8> {
    average_pulse_time: TimeRollingAverage<5>,
    pwm_signals: &'static DimmerPwmSignals,

    state: DimmerState,
    lookup_tables: LookupTables,

    timer: mcpwm0::Timer<'static, SLOT>,
    pwm_pin: mcpwm0::PwmPin<'static, SLOT, true>,
    capture_channel: mcpwm0::CaptureChannel<'static, SLOT>,
}

/// TODO support multiple dimmer channels by using the
/// other MCPWM timers and capture channels
impl DimmerPwmHandler<0> {
    pub fn configure(
        mcpwm: MCPWM0<'static>,
        zero_cross: InputSignal<'static>,
        gate: OutputSignal<'static>,
        state: &DimmerState,
        lookup_tables: LookupTables,
        pwm_signals: &'static DimmerPwmSignals,
    ) {
        let clock_config = PeripheralClockConfig::with_frequency(Rate::from_mhz(1))
            .expect("Failed to create MCPWM clock config!");

        // Create mcpwm driver with interrupt handler
        let mut mcpwm = McPwm::new(mcpwm, clock_config.clone());
        mcpwm.set_interrupt_handler(mcpwm_interrupt);
        info!("Created mcpwm");

        // Set sync event on falling edges ( before zero cross event )
        mcpwm.sync0.set_invert(true);
        mcpwm.sync0.set_signal(zero_cross.clone());
        info!("Sync configured!");

        // Capture rising edges phase aligned with last zero edge
        let capture_config =
            CaptureChannelConfig::default().with_capture_mode(CaptureMode::RisingEdge);

        // Capture is used to give a average for zero cross pulse length
        let mut capture_channel = mcpwm
            .capture0
            .configure(capture_config)
            .with_signal_input(zero_cross.clone());
        capture_channel.set_enable(true);
        info!("Capture channel configured!");

        // Reset capture timer on falling edges
        let cap_timer_config = CaptureTimerConfig::default().with_sync_phase(0);
        mcpwm.capture_timer.apply_config(cap_timer_config);
        mcpwm.capture_timer.set_sync_in(&mcpwm.sync0);
        info!("Capture timer configured!");

        // Start timers with defaults
        let timer_config = clock_config
            .timer_clock_with_prescaler(u16::MAX, PwmWorkingMode::Increase, 0)
            .with_period_updating_method(PeriodUpdatingMethod::Sync)
            .with_phase(0);

        mcpwm.timer0.set_sync_in(&mcpwm.sync0);
        mcpwm
            .timer0
            .apply_config(timer_config)
            .expect("Timer 0 failed to apply config");
        info!("PWM timer configured!");

        // Setup operator
        mcpwm.operator0.set_timer(&mcpwm.timer0);
        let timer = mcpwm.timer0;

        // Configure pwm pin to be idle
        let pwm_pin_config = PwmPinConfig::new(PwmActions::empty(), PwmUpdateMethod::SYNC_ON_ZERO);
        let pwm_pin = mcpwm.operator0.with_pin_a(gate, pwm_pin_config);
        info!("Operator configured!");

        let mut channel = Self {
            average_pulse_time: TimeRollingAverage::new(),
            timer,
            pwm_pin,
            capture_channel,
            pwm_signals,
            state: state.clone(),
            lookup_tables,
        };

        critical_section::with(|cs| {
            channel.capture_channel.listen();
            mcpwm.capture_timer.start();
            channel.timer.start();
            *DIMMER_PWM.borrow_ref_mut(cs) = Some(channel)
        });
    }

    fn interrupt(&mut self) {
        if self.capture_channel.is_interrupt_set() {
            self.capture_channel.clear_interrupt();

            let event = self.capture_channel.events();
            let pulse_width = event.time() / 80;

            let average_high = self
                .average_pulse_time
                .new_sample(Duration::from_micros(pulse_width as u64));

            let estimated_zero_cross = (average_high / 2).as_micros() as u16;

            if let Some(new_state) = self.pwm_signals.new_state.try_take() {
                self.state = new_state;
                self.update_pwm(estimated_zero_cross);
            }

            if let Some(new_lookup) = self.pwm_signals.new_lookup.try_take() {
                self.lookup_tables = new_lookup;
                self.update_pwm(estimated_zero_cross);
            }
        }
    }

    fn update_pwm(&mut self, estimated_zero_cross: u16) {
        if !self.state.is_on || self.state.brightness == 0 {
            self.disable_pwm_output();
            return;
        }

        // Configure MCPWM for the desired brightness using the precalculated lookup tables
        let brightness = self.state.brightness;
        let lookup = &self.lookup_tables;

        let fire_angle_us = lookup.fire_angle_table[brightness as usize];
        let pulse_time_us = lookup.pulse_width_table[brightness as usize];

        self.update_pwm_fire_angle(fire_angle_us, pulse_time_us, estimated_zero_cross);
    }

    fn disable_pwm_output(&mut self) {
        // Single write critical section not needed
        self.pwm_pin.set_actions(
            PwmActions::empty().on_up_counting_timer_equals_period(UpdateAction::SetLow),
        );
    }

    fn update_pwm_fire_angle(
        &mut self,
        fire_angle_us: u16,
        pulse_time_us: u16,
        estimated_zero_cross: u16,
    ) {
        critical_section::with(|_| {
            let trigger_ticks = estimated_zero_cross.saturating_add(fire_angle_us);

            // Output is set high on timestamp
            self.pwm_pin.set_timestamp(trigger_ticks);

            // Output is set low on period
            self.timer
                .update_period(trigger_ticks.saturating_add(pulse_time_us));

            // Update pwm pins actions
            self.pwm_pin.set_actions(
                PwmActions::empty()
                    .on_up_counting_timer_equals_timestamp(UpdateAction::SetHigh)
                    .on_up_counting_timer_equals_period(UpdateAction::SetLow),
            );
        })
    }
}
