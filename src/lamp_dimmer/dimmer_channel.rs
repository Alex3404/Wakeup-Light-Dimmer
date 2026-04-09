use crate::lamp_dimmer::{
    DimmerChannelConfig, DimmerChannelState, DimmerSettings, MAX_BRIGHTNESS, TimingConfig,
    dimmer_settings_builder::{DimmerSettingsBuilder, PublishCallback},
    lookup_tables::LookupTables,
    rolling_average::TimeRollingAverage,
};

use core::cell::RefCell;
use core::u16;
use embassy_sync::{
    blocking_mutex::{CriticalSectionMutex, Mutex, raw::CriticalSectionRawMutex},
    signal::Signal,
};
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
use log::trace;

extern crate alloc;
use alloc::sync::{Arc, Weak};

pub struct DimmerChannel {
    settings: DimmerSettings,
    state: DimmerChannelState,
    handle: DriverHandle,
    builder_dropped: Arc<Signal<CriticalSectionRawMutex, ()>>,
    builder_active: bool,
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
    pub fn new(config: DimmerChannelConfig) -> Result<DimmerChannel, ()> {
        let dimmer_data = DimmerDriverData::new(
            config.starting_state,
            config.dimmer_settings,
            config.timing_config,
            config.frequency,
        );
        let dimmer_data = Arc::new(Mutex::new(RefCell::new(dimmer_data)));

        DimmerChannelDriver::configure(
            config.mcpwm,
            config.zero_cross,
            config.gate,
            Arc::downgrade(&dimmer_data),
        );

        Ok(Self {
            settings: config.dimmer_settings,
            state: config.starting_state,
            handle: dimmer_data.clone(),
            builder_dropped: Arc::new(Signal::new()),
            builder_active: false,
        })
    }

    pub fn new_settings_builder(
        &mut self,
        publish_callback: PublishCallback,
    ) -> Result<DimmerSettingsBuilder, ()> {
        if self.builder_active {
            // Only one builder can be active at a time to prevent conflicts
            return Err(());
        }
        self.builder_active = true;
        self.builder_dropped.reset();

        let builder = DimmerSettingsBuilder::new(
            publish_callback,
            self.handle.clone(),
            self.settings.clone(),
            self.builder_dropped.clone(),
        );

        Ok(builder)
    }

    pub fn get_state(&self) -> DimmerChannelState {
        self.state
    }

    pub fn set_state(&mut self, state: DimmerChannelState) {
        self.state = state;
        self.update_state();
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        if self.state.brightness != brightness {
            self.state.brightness = brightness;
            self.update_state();
        }
    }

    pub fn set_on(&mut self, on: bool) {
        if self.state.is_on != on {
            self.state.is_on = on;
            self.update_state();
        }
    }

    pub fn toggle(&mut self) {
        self.state.is_on = !self.state.is_on;
        self.update_state();
    }

    pub fn get_settings(&self) -> DimmerSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: DimmerSettings) {
        self.settings = settings;
        self.update_settings();
    }

    pub async fn animate_brightness(&mut self, target_brightness: u8, duration: Duration) {
        let current_brightness = self.get_state().brightness;
        let steps = current_brightness.abs_diff(target_brightness) as i16;
        let step_duration = duration / steps as u32;
        let brightness_step: i16 = if target_brightness > current_brightness {
            1
        } else if target_brightness < current_brightness {
            -1
        } else {
            return;
        };

        for _ in 1..=steps {
            let new_brightness =
                (current_brightness as i16 + brightness_step).clamp(0, MAX_BRIGHTNESS as i16) as u8;
            self.set_brightness(new_brightness);

            embassy_time::Timer::after(step_duration).await;
        }
    }

    fn builder_active(&mut self) -> bool {
        if self.builder_active && self.builder_dropped.signaled() {
            // Builder was dropped, reset state
            self.builder_active = false;
        }
        self.builder_active
    }

    fn update_settings(&mut self) {
        if !self.builder_active() {
            self.handle
                .lock(|data| data.borrow_mut().settings = self.settings);
        }
    }

    fn update_state(&mut self) {
        if !self.builder_active() {
            self.handle
                .lock(|data| data.borrow_mut().state = self.state);
        }
    }
}

type DimmerSlot<const SLOT: u8> = CSMutex<RefCell<Option<DimmerChannelDriver<SLOT>>>>;
static SLOT_0: DimmerSlot<0> = CSMutex::new(RefCell::new(None));
#[allow(dead_code)]
static SLOT_1: DimmerSlot<1> = CSMutex::new(RefCell::new(None));
#[allow(dead_code)]
static SLOT_2: DimmerSlot<2> = CSMutex::new(RefCell::new(None));

#[handler]
fn mcpwm_interrupt() {
    critical_section::with(|cs| {
        if let Some(ref mut dimmer) = *SLOT_0.borrow_ref_mut(cs) {
            dimmer.interrupt();
        }
    })
}

pub(super) type DriverHandle = Arc<CriticalSectionMutex<RefCell<DimmerDriverData>>>;

/// Provides the dimming data and lookup tables for the dimmer channels.
/// This is used to share the data between the dimmer channels and the
/// interrupt handlers since the interrupt handlers need to access the
/// dimmer state and lookup.
pub(super) struct DimmerDriverData {
    frequency: u8,
    state: DimmerChannelState,
    settings: DimmerSettings,

    timing_confg: TimingConfig,
    lookup_tables: LookupTables,
}

#[allow(dead_code)]
impl DimmerDriverData {
    pub fn new(
        state: DimmerChannelState,
        settings: DimmerSettings,
        timing_confg: TimingConfig,
        frequency: u8,
    ) -> Self {
        let lookup_tables = timing_confg.create_lookup_tables(frequency, &settings);
        Self {
            frequency,
            state,
            settings,
            timing_confg,
            lookup_tables,
        }
    }

    pub fn get_state(&self) -> DimmerChannelState {
        self.state
    }

    pub fn get_settings(&self) -> DimmerSettings {
        self.settings
    }

    pub fn get_timing_config(&self) -> TimingConfig {
        self.timing_confg
    }

    pub fn update_state(&mut self, state: DimmerChannelState) {
        self.state = state;
    }

    pub fn update_settings(&mut self, settings: DimmerSettings) {
        self.settings = settings;
        self.build_lookup_tables(self.frequency);
    }

    pub fn update_timing_config(&mut self, timing_config: TimingConfig) {
        self.timing_confg = timing_config;
        self.build_lookup_tables(self.frequency);
    }

    fn build_lookup_tables(&mut self, frequency: u8) {
        self.lookup_tables = self
            .timing_confg
            .create_lookup_tables(frequency, &self.settings);
    }
}

/// Reference to lamp dimmer state for MCPWM interrupt handler
/// Only 3 slot are available due to hardware limitations
struct DimmerChannelDriver<const SLOT: u8> {
    avg_time_high: TimeRollingAverage<5>,
    timer: mcpwm0::Timer<'static, SLOT>,
    pwm_pin: mcpwm0::PwmPin<'static, SLOT, true>,
    capture_channel: mcpwm0::CaptureChannel<'static, SLOT>,
    data: WeakDriverHandle,
}

type WeakDriverHandle = Weak<CriticalSectionMutex<RefCell<DimmerDriverData>>>;

/// TODO support multiple dimmer channels by using the
/// other MCPWM timers and capture channels
impl DimmerChannelDriver<0> {
    pub fn configure(
        mcpwm: MCPWM0<'static>,
        zero_cross: InputSignal<'static>,
        gate: OutputSignal<'static>,
        state: WeakDriverHandle,
    ) {
        let clock_config = PeripheralClockConfig::with_frequency(Rate::from_mhz(1))
            .expect("Failed to create MCPWM clock config!");

        // Create mcpwm driver with interrupt handler
        let mut mcpwm = McPwm::new(mcpwm, clock_config.clone());
        mcpwm.set_interrupt_handler(mcpwm_interrupt);
        trace!("Created mcpwm");

        // Set sync event on falling edges ( before zero cross event )
        mcpwm.sync0.set_invert(true);
        mcpwm.sync0.set_signal(zero_cross.clone());
        trace!("Sync configured!");

        // Capture rising edges phase aligned with last zero edge
        let capture_config =
            CaptureChannelConfig::default().with_capture_mode(CaptureMode::RisingEdge);

        // Capture is used to give a average for zero cross pulse length
        let mut capture_channel = mcpwm
            .capture0
            .configure(capture_config)
            .with_signal_input(zero_cross.clone());
        capture_channel.set_enable(true);
        trace!("Capture channel configured!");

        // Reset capture timer on falling edges
        let cap_timer_config = CaptureTimerConfig::default().with_sync_phase(0);
        mcpwm.capture_timer.set_config(cap_timer_config);
        mcpwm.capture_timer.set_sync_in(&mcpwm.sync0);
        trace!("Capture timer configured!");

        // Start timers with defaults
        let timer_config = clock_config
            .timer_clock_with_prescaler(u16::MAX, PwmWorkingMode::Increase, 0)
            .with_sync_phase(0)
            .with_period_updating_method(PeriodUpdatingMethod::Sync);

        mcpwm.timer0.set_sync_in(&mcpwm.sync0);
        mcpwm.timer0.set_config(timer_config);
        trace!("PWM timer configured!");

        // Setup operator
        mcpwm.operator0.set_timer(&mcpwm.timer0);
        let timer = mcpwm.timer0;

        // Configure pwm pin to be idle
        let pwm_pin_config = PwmPinConfig::new(PwmActions::empty(), PwmUpdateMethod::SYNC_ON_ZERO);
        let pwm_pin = mcpwm.operator0.with_pin_a(gate, pwm_pin_config);
        trace!("Operator configured!");

        let mut channel = Self {
            avg_time_high: TimeRollingAverage::new(),
            timer,
            pwm_pin,
            capture_channel,
            data: state,
        };

        critical_section::with(|_cs| {
            channel.capture_channel.listen();
            mcpwm.capture_timer.start();
            channel.timer.start();
            channel.add_to_slot();
        });
    }

    fn add_to_slot(self) {
        critical_section::with(|cs| *SLOT_0.borrow_ref_mut(cs) = Some(self));
    }

    fn remove_from_slot() {
        critical_section::with(|cs| *SLOT_0.borrow_ref_mut(cs) = None);
    }

    fn interrupt(&mut self) {
        if self.capture_channel.is_interrupt_set() {
            self.capture_channel.clear_interrupt();

            let event = self.capture_channel.get_event();
            let pulse_width = event.time() / 80;
            self.falling_edge(pulse_width);
        }
    }

    fn falling_edge(&mut self, pulse_width: u32) {
        // Estimated next zero cross on rising edge
        let average_high = self
            .avg_time_high
            .new_sample(Duration::from_micros(pulse_width as u64));
        let estimated_zero_cross = (average_high / 2).as_micros() as u16;

        self.handle_dimming(estimated_zero_cross);
    }

    fn handle_dimming(&mut self, estimated_zero_cross: u16) {
        let Some(data_mutex) = self.data.upgrade() else {
            // Our dimmer channel has been dropped
            Self::remove_from_slot();
            return;
        };

        critical_section::with(|cs| {
            let data_ref = data_mutex.borrow(cs);
            let data = data_ref.borrow();

            // Output is set low on period
            if !data.state.is_on {
                self.pwm_pin.set_actions(
                    PwmActions::empty().on_up_counting_timer_equals_period(UpdateAction::SetLow),
                );
                return;
            }

            let brightness = data.state.brightness;
            let lookup = &data.lookup_tables;

            let fire_angle_us = lookup.fire_angle_table[brightness as usize];
            let pulse_time_us = lookup.pulse_width_table[brightness as usize];

            let trigger_ticks = estimated_zero_cross.saturating_add(fire_angle_us);

            self.pwm_pin.set_actions(
                PwmActions::empty()
                    .on_up_counting_timer_equals_timestamp(UpdateAction::SetHigh)
                    .on_up_counting_timer_equals_period(UpdateAction::SetLow),
            );

            // Output is set high on timestamp
            self.pwm_pin.set_timestamp(trigger_ticks);

            self.timer
                .update_period(trigger_ticks.saturating_add(pulse_time_us));
        });
    }
}
