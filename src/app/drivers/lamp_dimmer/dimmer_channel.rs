use crate::app::drivers::lamp_dimmer::Brightness;

use super::{
    DimmerSettings, DimmerState, TimingConfig, TriacChannelConfig,
    dimmer_settings_builder::DimmerSettingsBuilder, lookup_tables::LookupTable,
    rolling_average::TimeRollingAverage,
};

use core::{cell::RefCell, sync::atomic::AtomicBool};
use core::{sync::atomic::Ordering, u16};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Duration;

use critical_section::Mutex as CSMutex;
use enumset::EnumSet;
use esp_hal::gpio::interconnect::{InputSignal, OutputSignal};
use esp_hal::handler;

use esp_hal::mcpwm::capture::{CaptureChannel, CaptureEdge, CaptureMode, CaptureTimerConfig};
use esp_hal::mcpwm::operator::{PwmActions, PwmPin, PwmPinConfig, PwmUpdateMethod, UpdateAction};
use esp_hal::mcpwm::timer::{
    PeriodUpdatingMethod, PwmWorkingMode, StopCondition, SyncOutSelect, Timer, TimerEvent,
};
use esp_hal::mcpwm::{AnyMcPwm, McPwm, PeripheralClockConfig};

use esp_hal::time::Rate;

extern crate alloc;
use alloc::boxed::Box;

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
/// of a triac circit.
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
    pub fn new(spawner: Spawner, config: TriacChannelConfig) -> &'static Self {
        // The pwm signals so they can be shared with interrupt handler
        let pwm_signals = Box::leak(Box::new(DimmerPwmSignals::new()));

        let mut lookup_tables = LookupTable::default();
        config.timing_config.populate_lookup_table(
            config.frequency,
            &config.dimmer_settings,
            &mut lookup_tables,
        );

        DimmerPwmHandler::configure(
            config.mcpwm,
            config.zero_cross,
            config.gate,
            &config.starting_state,
            lookup_tables,
            pwm_signals,
        );

        // Leak dimmer channel so it can be shared with builder
        Box::leak(Box::new(Self {
            pwm_signals,
            frequency: config.frequency,
            timing_config: config.timing_config,
            builder_active: AtomicBool::new(false),
            current_state: RefCell::new(config.starting_state),
            current_settings: RefCell::new(config.dimmer_settings),
            spawner,
        }))
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
            let mut lookup_tables = LookupTable::default();
            self.timing_config
                .populate_lookup_table(self.frequency, settings, &mut lookup_tables);
            self.pwm_signals.new_lookup.signal(lookup_tables);
        }
    }

    pub fn get_settings(&self) -> DimmerSettings {
        *self.current_settings.borrow()
    }

    pub fn get_state(&self) -> DimmerState {
        *self.current_state.borrow()
    }

    pub fn update_brightness(&self, func: impl FnOnce(Brightness) -> Brightness) {
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

    pub fn set_brightness(&self, brightness: Brightness) {
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

type DimmerSlot = CSMutex<RefCell<Option<DimmerPwmHandler>>>;
static DIMMER_PWM: DimmerSlot = CSMutex::new(RefCell::new(None));

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
    new_lookup: Signal<CriticalSectionRawMutex, LookupTable>,
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

    pub fn signal_new_lookup(&self, lookup: LookupTable) {
        self.new_lookup.signal(lookup);
    }
}

/// Reference to lamp dimmer state for MCPWM interrupt handler
/// Only 3 slot are available due to hardware limitations
struct DimmerPwmHandler {
    pos_pulse_time: TimeRollingAverage<5>,
    neg_pulse_time: TimeRollingAverage<5>,
    pos_phase: AtomicBool,

    pwm_signals: &'static DimmerPwmSignals,

    state: DimmerState,
    lookup_tables: LookupTable,

    phase_timer: Timer<'static>,
    zc_timer: Timer<'static>,

    gate_pwm_pin: PwmPin<'static, true>,
    capture_channel: CaptureChannel<'static>,
}

/// TODO support multiple dimmer channels by using the
/// other MCPWM timers and capture channels
impl DimmerPwmHandler {
    pub fn configure(
        mcpwm: AnyMcPwm<'static>,
        zero_cross: InputSignal<'static>,
        gate: OutputSignal<'static>,
        state: &DimmerState,
        lookup_tables: LookupTable,
        pwm_signals: &'static DimmerPwmSignals,
    ) {
        // 1 Megahertz clock gives a resolution of 1 microsecond
        let clock_config = PeripheralClockConfig::with_frequency(Rate::from_mhz(1))
            .expect("Failed to create MCPWM clock config!");

        // Create mcpwm driver with interrupt handler
        let mut mcpwm = McPwm::new(mcpwm, clock_config.clone());
        mcpwm.set_interrupt_handler(mcpwm_interrupt);

        // Set sync event on falling edges ( before zero cross pulse )
        // mcpwm.sync0.set_invert(true);
        mcpwm.sync0.set_signal(zero_cross.clone());

        // Capture is used to give a average for zero cross pulse length
        let mut capture_channel = mcpwm.capture0.with_signal_input(zero_cross.clone());
        capture_channel.set_enable(true);

        // Reset capture timer on falling edges
        let cap_timer_config = CaptureTimerConfig::default().with_sync_phase(0);
        mcpwm.capture_timer.apply_config(cap_timer_config);
        mcpwm.capture_timer.set_sync_in(mcpwm.sync0.kind());

        // Timer 0 is used for outputting sync event at zero cross event
        let zc_timer_config = clock_config
            .timer_clock_with_prescaler(u16::MAX, PwmWorkingMode::Increase, 31)
            .with_period_updating_method(PeriodUpdatingMethod::TimerEqualsZeroOrSync)
            .with_sync_out(SyncOutSelect::SyncWhenEqualPeriod)
            .with_stop_condition(StopCondition::StopAtPeriod)
            .with_phase(0);

        let mut zc_timer = mcpwm.timer0;
        let _ = zc_timer.apply_config(zc_timer_config);
        zc_timer.set_sync_in(mcpwm.sync0.kind());

        // Phase timer config
        let timer_config = clock_config
            .timer_clock_with_prescaler(u16::MAX, PwmWorkingMode::Increase, 31)
            .with_period_updating_method(PeriodUpdatingMethod::Sync)
            .with_stop_condition(StopCondition::StopAtPeriod)
            .with_phase(0);
        let mut phase_timer = mcpwm.timer1;

        let _ = phase_timer.apply_config(timer_config);
        phase_timer.set_sync_in(zc_timer.get_sync_out());
        phase_timer
            .apply_config(timer_config)
            .expect("Timer 0 failed to apply config");

        // Setup operator
        // Configure pwm pin to be idle
        let mut gate_op = mcpwm.operator0;
        let pwm_pin_config = PwmPinConfig::new(PwmActions::empty(), PwmUpdateMethod::SYNC_ON_ZERO);
        gate_op.set_timer(&phase_timer);
        let pwm_pin_a = gate_op.with_pin_a(gate, pwm_pin_config);

        let mut channel = Self {
            pos_pulse_time: TimeRollingAverage::new(),
            neg_pulse_time: TimeRollingAverage::new(),
            pos_phase: AtomicBool::new(true),
            phase_timer,
            zc_timer,
            gate_pwm_pin: pwm_pin_a,
            capture_channel,
            pwm_signals,
            state: state.clone(),
            lookup_tables,
        };

        critical_section::with(|cs| {
            channel.capture_channel.listen(CaptureMode::AnyEdge);
            channel
                .zc_timer
                .listen(EnumSet::only(TimerEvent::TimerEqualPeriod));
            mcpwm.capture_timer.start();
            channel.update_phase_pwm();
            *DIMMER_PWM.borrow_ref_mut(cs) = Some(channel);
        });
    }

    fn start_edge(&mut self) {
        self.zc_timer.start();
    }

    fn zc_timer_event(&mut self) {
        // Zero cross event
        self.phase_timer.start();
    }

    fn end_edge(&mut self) {
        let event = self.capture_channel.events();
        let pulse_width = event.time() / 32;

        // Zero cross pulse time
        // zc_timer syncs on the start of the zero cross pulse so use the other sample to get the pulse time
        let average_high = if self.pos_phase.fetch_xor(true, Ordering::Relaxed) {
            self.pos_pulse_time
                .new_sample(Duration::from_micros(pulse_width as u64));
            self.neg_pulse_time.average()
        } else {
            self.neg_pulse_time
                .new_sample(Duration::from_micros(pulse_width as u64));
            self.pos_pulse_time.average()
        };

        let estimated_zero_cross = (average_high / 2).as_micros() as u16;

        // ZC timer is phase aligned to start of zero cross pulse
        // ZC timer outputs a sync event at the estimated zero cross time
        self.zc_timer.update_period(estimated_zero_cross);

        if let Some(new_state) = self.pwm_signals.new_state.try_take() {
            self.state = new_state;
            self.update_phase_pwm();
        }

        if let Some(new_lookup) = self.pwm_signals.new_lookup.try_take() {
            self.lookup_tables = new_lookup;
            self.update_phase_pwm();
        }
    }

    fn interrupt(&mut self) {
        if self.capture_channel.is_interrupt_set() {
            self.capture_channel.clear_interrupt();

            match self.capture_channel.events().edge() {
                CaptureEdge::Rising => self.start_edge(),
                CaptureEdge::Falling => self.end_edge(),
            }
        }

        let events = self.zc_timer.interrupts();
        if events.is_empty() {
            return;
        }

        self.zc_timer.clear_interrupts(events);
        for event in events {
            match event {
                TimerEvent::TimerEqualPeriod => self.zc_timer_event(),
                _ => {}
            }
        }
    }

    fn update_phase_pwm(&mut self) {
        if !self.state.is_on || self.state.brightness == 0 {
            self.disable_phase_pwm();
            return;
        }

        // Configure MCPWM for the desired brightness using the precalculated lookup tables
        let brightness = self.state.brightness;
        let lookup = &self.lookup_tables;

        let fire_angle_us = lookup.fire_angle_table[brightness as usize];
        let pulse_time_us = lookup.pulse_width_table[brightness as usize];

        self.update_pwm_fire_angle(fire_angle_us, pulse_time_us);
    }

    fn disable_phase_pwm(&mut self) {
        self.gate_pwm_pin.set_actions(
            PwmActions::empty().on_up_counting_timer_equals_period(UpdateAction::SetLow),
        );
    }

    fn update_pwm_fire_angle(&mut self, fire_angle_us: u16, pulse_time_us: u16) {
        critical_section::with(|_| {
            // Output is set high on timestamp
            self.gate_pwm_pin.set_timestamp(fire_angle_us);

            // Output is set low on period
            self.phase_timer
                .update_period(fire_angle_us.saturating_add(pulse_time_us));

            // Update pwm pins actions
            self.gate_pwm_pin.set_actions(
                PwmActions::empty()
                    .on_up_counting_timer_equals_timestamp(UpdateAction::SetHigh)
                    .on_up_counting_timer_equals_period(UpdateAction::SetLow)
                    .on_up_counting_timer_equals_zero(UpdateAction::SetLow),
            );
        })
    }
}
