use crate::rolling_average::TimeRollingAverage;
use embassy_time::{Duration, Instant, WithTimeout};
use esp_hal::{gpio::Input, time::Rate};

pub async fn determine_frequency<const SAMPLES: usize>(
    zero_cross_pin: &mut Input<'static>,
) -> Result<Rate, ()> {
    let mut rolling_average = TimeRollingAverage::<SAMPLES>::new();
    let mut last_rising: Option<Instant> = None;

    let mut i = 0;
    while i < SAMPLES {
        let future = zero_cross_pin.wait_for_rising_edge().into_future();
        let result = future.with_timeout(Duration::from_secs(1)).await;
        let now = Instant::now();

        if zero_cross_pin.is_low() {
            // Should be high after rising edge
            continue;
        }

        // Detects any noise thats shorter then 10 micro seconds
        let glitch_detect = zero_cross_pin
            .wait_for_any_edge()
            .with_timeout(Duration::from_micros(10))
            .await;
        let Err(_) = glitch_detect else {
            // Another edge detected shortly after the rising edge
            // Must be noise
            continue;
        };

        let Ok(_) = result else { return Err(()) }; // Timed out

        let last_time = last_rising.replace(now);
        let Some(last_time) = last_time else {
            continue; // No last sample
        };

        let delta = now - last_time;
        rolling_average.new_sample(delta);
        i += 1;
    }

    // Detects both zero points therefore wave length is 2x the average
    let average_wave_us = rolling_average.average().as_micros() * 2;
    if average_wave_us == 0 {
        Err(())
    } else {
        Ok(Rate::from_hz((1_000_000 / average_wave_us) as u32))
    }
}
