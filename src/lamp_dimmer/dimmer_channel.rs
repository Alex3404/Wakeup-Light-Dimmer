use crate::lamp_dimmer::lookup_tables::LookupTables;
use crate::lamp_dimmer::{LampDimmerChannelConfig, MAX_BRIGHTNESS};
use crate::rolling_average::TimeRollingAverage;

use core::ops::{AddAssign, Rem};
use core::option::Option;
use core::option::Option::{None, Some};

use core::cell::RefCell;
use core::u16;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::{Duration, Instant, WithTimeout};
use esp_hal::gpio::{Input, Level, Output};
use esp_hal::rmt::{ContinuousTxTransaction, PulseCode, Tx, TxChannelConfig, TxChannelCreator};
use esp_hal::{Blocking, rmt};

extern crate alloc;
use alloc::rc::Rc;
use log::info;

pub type LampDimmerChannelReference = Rc<NoopMutex<RefCell<LampDimmerChannel>>>;

enum DimmerTxChannel {
    HasChannel(rmt::Channel<'static, Blocking, Tx>),
    IsTransfering(Option<ContinuousTxTransaction<'static>>),
}

pub struct LampDimmerChannel {
    lookup_tables: LookupTables,
    avg_time_high: TimeRollingAverage<25>,
    last_edge: Option<Instant>,
    brightness: u8,

    event_counter: u64,
    tx_channel: NoopMutex<RefCell<DimmerTxChannel>>,
}

#[embassy_executor::task(pool_size = 1)]
async fn zero_cross_pin_loop(
    _spawner: Spawner,
    this: LampDimmerChannelReference,
    mut zero_cross_pin: Input<'static>,
) {
    info!("Zero cross loop started!");
    loop {
        zero_cross_pin.wait_for_any_edge().await;
        let time = Instant::now();
        let is_high = zero_cross_pin.is_high();

        this.lock(|dimmer| {
            if is_high {
                dimmer.borrow_mut().rising_edge(time);
            } else {
                dimmer.borrow_mut().falling_edge(time);
            }
        })
    }
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
impl LampDimmerChannel {
    pub fn create<Channel>(
        spawner: Spawner,
        config: LampDimmerChannelConfig<Channel>,
    ) -> Result<LampDimmerChannelReference, ()>
    where
        Channel: TxChannelCreator<'static, Blocking> + Sized,
    {
        let tx_channel =
            LampDimmerChannel::configure_rmt_channel(config.rmt_channel, config.gate_output_pin)?;

        let lookup_tables = config
            .fire_timing_config
            .create_lookup_tables(config.frequency);

        // Create the dimmer
        let dimmer_reference = Rc::new(NoopMutex::new(RefCell::new(Self {
            lookup_tables: lookup_tables,
            brightness: config.starting_brightness,
            avg_time_high: TimeRollingAverage::new(),
            last_edge: None,
            event_counter: 0,
            tx_channel: NoopMutex::new(RefCell::new(DimmerTxChannel::HasChannel(tx_channel))),
        })));

        spawner.must_spawn(zero_cross_pin_loop(
            spawner,
            dimmer_reference.clone(),
            config.zero_cross_pin,
        ));

        // Return a referance for rest of code
        Ok(dimmer_reference)
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness.clamp(0, MAX_BRIGHTNESS);
    }

    fn configure_rmt_channel<Channel>(
        rmt_channel: Channel,
        signal_output: Output<'static>,
    ) -> Result<rmt::Channel<'static, Blocking, Tx>, ()>
    where
        Channel: TxChannelCreator<'static, Blocking> + Sized,
    {
        let tx_config = TxChannelConfig::default()
            .with_idle_output_level(Level::Low)
            .with_idle_output(true)
            .with_carrier_modulation(false)
            .with_clk_divider(80); // For 1us

        let tx_channel = rmt_channel.configure_tx(signal_output, tx_config);
        let Ok(tx_channel) = tx_channel else {
            return Err(());
        };

        Ok(tx_channel)
    }

    fn execute_pulse<const N: usize>(&mut self, data: [PulseCode; N]) {
        self.tx_channel.lock(|tx_channel_cell| {
            // Replace with none so we can take ownership
            let dimmer_channel = tx_channel_cell.replace(DimmerTxChannel::IsTransfering(None));
            let tx_channel = match dimmer_channel {
                DimmerTxChannel::HasChannel(tx_channel) => tx_channel,
                DimmerTxChannel::IsTransfering(mut current_tx) => {
                    let Some(transaction) = current_tx.take() else {
                        panic!("Oh no!");
                    };

                    // Stop the current transaction
                    match transaction.stop() {
                        Ok(channel) => channel,
                        Err((_err, channel)) => channel,
                    }
                }
            };

            let transaction =
                tx_channel.transmit_continuously(&data, esp_hal::rmt::LoopMode::Finite(1));
            let Ok(transaction) = transaction else {
                // This means our pulse data was invalid
                panic!("Transaction error cannot continue");
            };

            tx_channel_cell.replace(DimmerTxChannel::IsTransfering(Some(transaction)));
        });
    }

    fn handle_dimming(&mut self, estimated_zero_cross_us: u64, zero_cross_pulse: u64) {
        // Lookup table fast and easy
        let brightness = self.brightness;
        let lookup = &self.lookup_tables;
        let fire_angle_us = lookup.fire_angle_table[brightness as usize];
        let pulse_time_us = lookup.pulse_width_table[brightness as usize];

        let pulses = [
            // Delay to zero cross + fire angle
            PulseCode::new(
                Level::Low,
                estimated_zero_cross_us as u16,
                Level::Low,
                fire_angle_us as u16,
            ),
            // Trigger pulse for pulse time us
            PulseCode::new(Level::High, pulse_time_us as u16, Level::Low, 0),
        ];

        self.execute_pulse(pulses);

        self.event_counter.add_assign(1);
        if self.event_counter.rem(120) == 0 {
            info!(
                "ZC: {}us | Fire angle: {}us | Pulse time: {}us | Brightness: {}%",
                estimated_zero_cross_us, fire_angle_us, pulse_time_us, brightness
            );
        }
    }

    fn rising_edge(&mut self, time: Instant) {
        // Estimated next zero cross on rising edge
        let delta = time.elapsed().as_micros();
        let average_high = self.avg_time_high.average();
        let estimated_zero_cross_us = (average_high / 2).as_micros().saturating_sub(delta);
        self.handle_dimming(estimated_zero_cross_us, average_high.as_micros());

        // Update the last edge
        self.last_edge = Some(time);
    }

    fn falling_edge(&mut self, time: Instant) {
        // Update the last edge
        let last_edge_optional = self.last_edge;
        self.last_edge = Some(time);
        let Some(last_edge) = last_edge_optional else {
            // No previous data
            return;
        };

        // Time spent between now and the last rising edge
        let delta = time - last_edge;
        let _ = self.avg_time_high.new_sample(delta);
    }
}
