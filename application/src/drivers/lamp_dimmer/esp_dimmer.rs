use super::{
    rolling_average::TimeRollingAverage,
};

use core::{cell::RefCell, sync::atomic::AtomicBool};
use core::{sync::atomic::Ordering};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant};

use critical_section::Mutex as CSMutex;
use esp_hal::gpio::interconnect::{InputSignal, OutputSignal};
use esp_hal::handler;

use esp_hal::mcpwm::capture::{CaptureChannel, CaptureChannelConfig, CaptureEdge, CaptureMode, CaptureTimerConfig};
use esp_hal::mcpwm::operator::{PwmActions, PwmPin, PwmPinConfig, PwmUpdateMethod, UpdateAction};
use esp_hal::mcpwm::timer::{
    PeriodUpdatingMethod, PwmWorkingMode, StopCondition, Timer,
};
use esp_hal::mcpwm::{AnyMcPwm, McPwm, PeripheralClockConfig};

use esp_hal::time::Rate;

extern crate alloc;
use fixed::traits::FromFixed;
use static_cell::StaticCell;

use super::*;
use heapless::{arc_pool, pool::arc::{Arc, ArcBlock}};

static MAX_PWM_CHANNELS: usize = 6;

arc_pool!(PwmSignals: EspPwmSignals);
static SIGNALS : StaticCell<[ArcBlock<EspPwmSignals>; MAX_PWM_CHANNELS]> = StaticCell::new();
static PWM_HANDLERS : CSMutex<RefCell<heapless::Vec<DimmerPwmHandler, MAX_PWM_CHANNELS>>> = CSMutex::new(RefCell::new(heapless::Vec::new()));

#[handler]
fn mcpwm_interrupt() {
    critical_section::with(|cs| {
        // Process all PWM handlers
        for handler in PWM_HANDLERS.borrow_ref_mut(cs).iter_mut() {
            handler.interrupt();
        }
    })
}

pub fn init(spawner : Spawner) {
    let signals = SIGNALS.try_init_with(|| [const { ArcBlock::new() }; MAX_PWM_CHANNELS]);

    if signals.is_none() {
        return; // Already initialized
    }

    for signal in signals.unwrap() {
        PwmSignals.manage(signal);
    }

    // spawner.spawn(pwm_handler_zc_watchdog().unwrap());
}

pub fn configure_mcpwm(mcpwm : AnyMcPwm<'static>) -> McPwm<'static> {
    // 1 Megahertz clock gives a resolution of 1 microsecond
    let clock_config = PeripheralClockConfig::with_frequency(Rate::from_mhz(1))
        .expect("Failed to create MCPWM clock config!");

    // Configure the MCPWM peripheral
    let mut mcpwm = McPwm::new(mcpwm, clock_config);
    mcpwm.set_interrupt_handler(mcpwm_interrupt);
    mcpwm
}

#[derive(Debug, Clone, defmt::Format)]
pub enum EspPwmError {
    AllocFailed,
}

pub struct EspPwmDimmer {
    pwm_signals: Arc<PwmSignals>,
    current_power : Power,
    #[allow(dead_code)]
    config : PowerDimmingConfig,
}

pub struct EspPwmDimmerConfig<'a> {
    mcpwm : McPwm<'a>,
    gate : OutputSignal<'a>,
    zero_cross: InputSignal<'a>,
    zero_cross_active_low: bool,
    power : PowerDimmingConfig,
}

impl EspPwmDimmerConfig<'static> {
    pub fn new(
        mcpwm: McPwm<'static>,
        gate: OutputSignal<'static>,
        zero_cross: InputSignal<'static>,
        zero_cross_active_low: bool,
        power: PowerDimmingConfig,
    ) -> Self {
        Self {
            mcpwm,
            gate,
            zero_cross,
            zero_cross_active_low,
            power,
        }
    }
}

pub struct EspPwmSignals {
    pub(super) set_power : Signal<CriticalSectionRawMutex, Power>,
    pub(super) set_config : Signal<CriticalSectionRawMutex, PowerDimmingConfig>,
    pub(super) drop : Signal<CriticalSectionRawMutex, ()>,
}

#[allow(dead_code)]
struct DummyPowerDimmer;

impl PowerDimmingControl for DummyPowerDimmer {
    type Error = ();

        fn set_power(&mut self, _power: Power) -> Result<(), PowerControlError<Self::Error>> {
            // Do nothing
        Ok(())
    }

    fn get_power(&self) -> Power {
        Power::ZERO
    }

    fn set_config(&mut self, _config: PowerDimmingConfig) {
        // Do nothing
    }
}

impl EspPwmDimmer {
    /// Creates a new instance of the EspPwmDimmer.
    pub fn new(config: EspPwmDimmerConfig<'static>) -> Result<Self, EspPwmError> {
        // The pwm signals so they can be shared with interrupt handler
        let pwm_signals =
            PwmSignals.alloc(EspPwmSignals::default()).map_err(|_| EspPwmError::AllocFailed)?;

        let power_config = config.power;
        DimmerPwmHandler::configure(config, pwm_signals.clone());
        
        Ok(Self {
            pwm_signals,
            current_power: Power::ZERO,
            config: power_config,
        })
    }
}

impl PowerDimmingControl for EspPwmDimmer {
    type Error = ();

    fn set_power(&mut self, power: Power) -> Result<(), PowerControlError<Self::Error>> {
        self.current_power = power;
        self.pwm_signals.signal_set_power(power);
        Ok(())
    }

    fn get_power(&self) -> Power {
        self.current_power
    }

    fn set_config(&mut self, config: PowerDimmingConfig) {
        self.pwm_signals.signal_set_config(config);
    }
}

impl Drop for EspPwmDimmer {
    fn drop(&mut self) {
        self.pwm_signals.signal_drop();
    }
}

impl Default for EspPwmSignals {
    fn default() -> Self {
        Self {
            set_power: Signal::new(),
            set_config: Signal::new(),
            drop: Signal::new(),
        }
    }
}

impl EspPwmSignals {
    pub fn signal_set_power(&self, power: Power) {
        self.set_power.signal(power);
    }

    pub fn signal_set_config(&self, config: PowerDimmingConfig) {
        self.set_config.signal(config);
    }

    pub fn signal_drop(&self) {
        self.drop.signal(());
    }
}

/// A handler for managing the MCPWM signals for a single dimmer channel.
/// - Zero cross pulses should be centered around the zero crossing point
///   of the AC voltage signal. They can be active high or active low.
/// - Gate output can be either a leading or trailing edge to control the AC waveform dimming.
/// - Dead time can be configured to prevent latching a triac on during the next cycle
/// 
/// Example timing for a single dimmer channel with
/// active high zero crossing, active high gate output, and trailing edge:
/// 
/// ZC (Zero Crossing) Signal:
///              _       _       _
/// Zero Cross: | |_____| |_____| |
///              __________________
/// 100% On:    | 
///              ___      ___     
/// 50% On:     |   |____|   |____|
/// 
/// 0% On:      ___________________
struct DimmerPwmHandler {
    pos_sign_pulse_time: TimeRollingAverage<15>,
    neg_side_pulse_time: TimeRollingAverage<15>,
    half_wave_length: TimeRollingAverage<30>,
    last_zero_cross_time: Instant,
    pos_phase: AtomicBool,

    pwm_signals: Arc<PwmSignals>,
    power_config : PowerDimmingConfig,

    phase_timer: Timer<'static>,
    gate_pwm_pin: PwmPin<'static, true>,
    capture_channel: CaptureChannel<'static>,
}

static PERIOD_SHIFT : u16 = 0;//u16::MAX / 2;
impl DimmerPwmHandler {

    // Configures the MCPWM for the dimmer channel based on the provided configuration and PWM signals.
    pub fn configure(
        config: EspPwmDimmerConfig<'static>,
        pwm_signals: Arc<PwmSignals>,
    ) {
        let mut mcpwm = config.mcpwm;
        let zero_cross = config.zero_cross;
        let gate = config.gate;

        // 1 Megahertz clock gives a resolution of 1 microsecond
        let clock_config = PeripheralClockConfig::with_frequency(Rate::from_mhz(1))
            .expect("Failed to create MCPWM clock config!");

        // Set sync event to trigger when zero cross signal is received
        mcpwm.sync0.set_invert(config.zero_cross_active_low);
        mcpwm.sync0.set_signal(zero_cross.clone());

        let capture_channel_config = CaptureChannelConfig::default()
            .with_invert(config.zero_cross_active_low);

        // Capture is used to give a average for zero cross pulse length
        let mut capture_channel = mcpwm.capture0.with_signal_input(zero_cross.clone());
        capture_channel.apply_config(capture_channel_config);
        capture_channel.set_enable(true);

        // Reset capture timer on start of zero cross pulseitem
        let cap_timer_config = CaptureTimerConfig::default().with_sync_phase(0);
        mcpwm.capture_timer.apply_config(cap_timer_config);
        mcpwm.capture_timer.set_sync_in(mcpwm.sync0.get_sync_out());

        // Phase timer config for 1uS resolution
        let timer_config = clock_config
            .timer_clock_with_prescaler(u16::MAX, PwmWorkingMode::Increase, 31)
            .with_period_updating_method(PeriodUpdatingMethod::Sync)
            .with_stop_condition(StopCondition::RunContinuously)
            // Sync phase is shifted so at the end of the period ( When gate pin is turned off )
            // the timer will be reset to 0 and this will ensure that the gate will not be triggered again
            // within the same ac cycle.
            .with_phase(PERIOD_SHIFT);

        let mut phase_timer = mcpwm.timer0;

        let _ = phase_timer.apply_config(timer_config);
        phase_timer.set_sync_in(mcpwm.sync0.get_sync_out());
        phase_timer
            .apply_config(timer_config)
            .expect("Phase timer failed to apply config");

        // Setup operator
        // Configure pwm pin to be idle
        let mut gate_op = mcpwm.operator0;
        gate_op.set_timer(&phase_timer);

        let pwm_pin_config = PwmPinConfig::new(PwmActions::empty(), PwmUpdateMethod::SYNC_ON_ZERO);
        let gate_pwm_pin = gate_op.with_pin_a(gate, pwm_pin_config);

        let mut channel = Self {
            pos_sign_pulse_time: TimeRollingAverage::default(),
            neg_side_pulse_time: TimeRollingAverage::default(),
            half_wave_length: TimeRollingAverage::default(),
            pos_phase: AtomicBool::new(true),
            phase_timer,
            gate_pwm_pin,
            last_zero_cross_time: Instant::now(),
            capture_channel,
            pwm_signals,
            power_config: config.power,
        };

        critical_section::with(|cs| {
            channel.capture_channel.listen(CaptureMode::AnyEdge);
            mcpwm.capture_timer.start();
            channel.pwm_signals.signal_set_power(Power::ZERO);
            channel.phase_timer.start();
            if PWM_HANDLERS.borrow_ref_mut(cs).push(channel).is_err() {
                panic!("Failed to push channel to PWM_HANDLERS");
            }
        });
    }


    /// Handles the leading edge of the zero-cross signal. 
    /// Updates the half-wave length measurement and ignores outliers.
    fn leading_edge(&mut self) {
        // Outliers for 50hz and 60hz
        const OUTLIER_MIN : Duration = Duration::from_micros(7000); // 60Hz half-wave is 8.33ms cut off <7ms
        const OUTLIER_MAX : Duration = Duration::from_micros(12000); // 50Hz half-wave is 10ms cut off >12ms

        let now = Instant::now();
        let duration = now.saturating_duration_since(self.last_zero_cross_time);
        self.last_zero_cross_time = now;

        // Ignore outliers
        if duration < OUTLIER_MIN || duration > OUTLIER_MAX {
            return;
        }

        self.half_wave_length.new_sample(duration);
    }

    /// Handles the trailing edge of the zero-cross signal.
    /// Measures the pulse width of the zero-cross signal and
    /// updates the average for positive and negative halves of the AC waveform.
    /// 
    /// Updates the MCPWM with the new information to achieve the correct phase control.
    fn trailing_edge(&mut self) {
        // Get the time between leading and trailing edges
        // The measure of the zero cross pulse in microseconds
        let event = self.capture_channel.events();

        // Adjust time since capture timer doesn't use a prescaler
        let pulse_width = event.time() / 32;
 
        // Zero cross pulse time
        // The negative and positive parts of the AC waveform can have different pulse widths on average.
        // To compensate for this, we have to have seperate averages for each.
        //
        // Caused by the use of a bi-directional octocoupler having different transfer ratios
        // for the positive and negative halves of the AC waveform.
        let average_time_active = if self.pos_phase.fetch_xor(true, Ordering::Relaxed) {
            self.pos_sign_pulse_time
                .new_sample(Duration::from_micros(pulse_width as u64));
            self.neg_side_pulse_time.average() // Next cycle will be negative if positive this cycle
        } else {
            self.neg_side_pulse_time
                .new_sample(Duration::from_micros(pulse_width as u64));
            self.pos_sign_pulse_time.average() // Next cycle will be positive if negative this cycle
        };

        let estimated_zero_cross = (average_time_active.checked_div(2).unwrap_or(Duration::MIN)).as_micros() as u16;

        // Only update when average waveform data is available
        if !self.half_wave_length.is_full() {
            return; // Ignore updates until we have a full average of the waveform
        }

        // Update the current power level based on the latest set power signal
        if let Some(new_power) = self.pwm_signals.set_power.try_take() &&
            self.update_phase_pwm(new_power, estimated_zero_cross).is_err() {
            defmt::error!("Failed to update phase PWM with new power");
        }
    }

    /// Handles the interrupt from the zero-cross detection capture channel.
    /// Determines whether the edge is rising or falling and calls the appropriate handler.
    fn interrupt(&mut self) {
        if self.capture_channel.is_interrupt_set() {
            self.capture_channel.clear_interrupt();

            match self.capture_channel.events().edge() {
                CaptureEdge::Rising => self.leading_edge(),
                CaptureEdge::Falling => self.trailing_edge(),
            }
        }
    }

    /// Updates the phase PWM based on the desired power and the estimated zero-cross timing.
    /// Returns an error if the update fails.
    fn update_phase_pwm(&mut self, power : Power, zero_cross : u16) -> Result<(), ()> {
        use fixed::types::U16F16;

        if power.0 == 0 {
            self.disable_phase_pwm();
            return Ok(())
        }

        defmt::debug!("New power: {:?}", power.0.saturating_to_num::<f32>() * 100.0);

        let wave_length = self.half_wave_length.average().as_micros() as u16;
        let sub_wave_length: u16 = wave_length
            .saturating_sub(self.power_config.gate.leading_deadtime_time)
            .saturating_sub(self.power_config.gate.trailing_deadtime_time);

        let power_fraction = U16F16::checked_from_fixed(power.0).ok_or(())?;
        let gate_active_us = power_fraction.checked_mul(U16F16::from_num(sub_wave_length)).ok_or(())?;
        let gate_active_us = gate_active_us.checked_to_num::<u16>().ok_or(())?;

        let gate_start_us = match self.power_config.dimming_mode {
            AcDimmingMode::TrailingEdge => self.power_config.gate.leading_deadtime_time,
            AcDimmingMode::LeadingEdge => {
                wave_length.saturating_sub(gate_active_us).saturating_sub(self.power_config.gate.leading_deadtime_time)
            }
        };

        defmt::debug!("Average half wave length: {:?} us", wave_length);
        defmt::debug!("Gate start: {:?} us, Gate active: {:?} us, Zero cross: {:?} us", gate_start_us, gate_active_us, zero_cross);

        self.update_pwm(gate_start_us, gate_active_us, zero_cross);
        Ok(())
    }

    /// Disables the phase PWM by setting the gate to its inactive state.
    fn disable_phase_pwm(&mut self) {
        self.gate_pwm_pin.set_actions(
            PwmActions::empty().on_up_counting_timer_equals_period(self.gate_inactive()),
        );
    }

    /// Updates the PWM signals for the gate based on the calculated timings.
    /// 
    /// # Arguments
    ///
    /// * `gate_start_us` - The start time of the gate pulse in microseconds.
    /// * `gate_active_us` - The duration for which the gate should be active in microseconds.
    /// * `zero_cross` - The estimated zero-cross timing in microseconds.
    ///
    /// This function configures the PWM actions for the gate based on the calculated start time,
    /// active duration, and zero-cross timing. It ensures that the gate is activated and deactivated
    /// at the correct times to achieve the desired phase control.
    fn update_pwm(&mut self, gate_start_us: u16, gate_active_us: u16, zero_cross: u16) {
        critical_section::with(|_| {
            let gate_start_us = gate_start_us.saturating_add(PERIOD_SHIFT).saturating_add(zero_cross);
            
            // Output is set high on timestamp
            self.gate_pwm_pin.set_timestamp(gate_start_us);

            // Output is set low on period
            let period = gate_start_us.saturating_add(gate_active_us);
            self.phase_timer.update_period(period);

            defmt::debug!("Sync Period: {:?}, Timestamp: {:?}, Period: {:?}", PERIOD_SHIFT, gate_start_us, period);

            // Update pwm pins actions
            self.gate_pwm_pin.set_actions(
                PwmActions::empty()
                    .on_up_counting_timer_equals_timestamp(self.gate_active())
                    .on_up_counting_timer_equals_period(self.gate_inactive())
                    .on_up_counting_timer_equals_zero(self.gate_inactive()),
            );
        })
    }

    /// Helper function to determine the inactive state of the gate based on the power configuration.
    fn gate_inactive(&self) -> UpdateAction {
        if self.power_config.gate.active_low {
            UpdateAction::SetHigh
        } else {
            UpdateAction::SetLow
        }
    }

    /// Helper function to determine the active state of the gate based on the power configuration.
    fn gate_active(&self) -> UpdateAction {
        if self.power_config.gate.active_low {
            UpdateAction::SetLow
        } else {
            UpdateAction::SetHigh
        }
    }
}
