#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use esp_backtrace as _;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

use arduino_esp32_dimmer::{
    lamp_dimmer::{
        self, FireTimingConfig, GammaCorrection, LampDimmerChannel, LampDimmerChannelConfig,
    },
    rotery_decoder::{Rotation, RoteryDecoder},
};
use libm::sinf;

use core::{
    f32::consts::PI,
    sync::atomic::{AtomicI32, AtomicU8, Ordering},
    time::Duration,
};

use embassy_time::Timer;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    gpio::{Input, InputConfig, Io, Level, Output, OutputConfig, Pull},
    pcnt::Pcnt,
    peripherals::Peripherals,
    rmt::Rmt,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};

extern crate alloc;
use alloc::boxed::Box;
use esp_radio::ble::controller::BleConnector;
use log::info;

static BRIGHTNESS: AtomicU8 = AtomicU8::new(10);
fn rotated(rotation: Rotation, _spawner: Spawner) {
    let value = BRIGHTNESS.load(Ordering::Relaxed);
    let next_value = match rotation {
        Rotation::Clockwise => value.saturating_add(5).min(lamp_dimmer::MAX_BRIGHTNESS),
        Rotation::Counterclockwise => value.saturating_sub(5),
    };
    BRIGHTNESS.store(next_value, Ordering::Relaxed);
    info!("Brightness: {}", next_value);
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Initalize peripherals used
    let rmt = initalize_rmt(&peripherals);
    let (zero_cross, triac_gate) = get_dimmer_io(&peripherals);
    let (clock, rotate, switch) = get_rotery_encoder_io(&peripherals);

    // Initalize the heap allocator with 72000 bytes of ram
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72000);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    info!("Embassy initialized!");

    // Initalize the rotery decoder
    let rotery_decoder = RoteryDecoder::create(spawner, clock, rotate, switch);
    let Ok(rotery_decoder) = rotery_decoder else {
        panic!("Failed to create rotery decoder!");
    };
    rotery_decoder
        .borrow()
        .borrow_mut()
        .add_rotation_event(Box::new(rotated));

    info!("Created rotery decoder!");

    // Start up wifi and bluetooth controller
    let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    let _connector = BleConnector::new(&radio_init, peripherals.BT, Default::default());

    let fire_timing = FireTimingConfig::new()
        .with_latch_time_after_zero(5000)
        .with_latch_time_before_next_zero(1000)
        .with_min_latch_time(1000)
        .with_perceived_zero_brightness(0)
        .with_perceived_full_brightness(100)
        .with_gamma_correction(GammaCorrection::Linear);

    let dimmer_config = LampDimmerChannelConfig::new(60, zero_cross, triac_gate, rmt.channel0)
        .with_firing_timing(fire_timing);

    // Initalize our lamp dimmer
    let main_dimmer = LampDimmerChannel::create(spawner, dimmer_config);
    let Ok(main_dimmer) = main_dimmer else {
        panic!("Failed to create main dimmer!");
    };

    info!("Created main light dimmer channel!");

    // Spawn some tasks
    let _ = spawner;

    let mut angle: f32 = 0.0;
    let mut last_time = Instant::now();
    loop {
        // Simple
        // let speed_counter = SPEED.load(Ordering::Relaxed).clamp(1, 100);
        // let angular_speed = (speed_counter as f32 / 25.0) * 2.0 * PI;

        // let now = Instant::now();
        // let time_delta = now - last_time;
        // last_time = now;
        // let delta_secs = time_delta.as_micros() as f32 / 1_000_000.0;
        // angle = angle + angular_speed * delta_secs;

        // let brightness_f = (sinf(angle) + 1.0) / 2.0;
        // let brightness = (brightness_f * lamp_dimmer::MAX_BRIGHTNESS as f32) as u8;

        let brightness = BRIGHTNESS.load(Ordering::Relaxed);
        main_dimmer.lock(|dimmer| {
            dimmer.borrow_mut().set_brightness(brightness);
        });

        Timer::after_micros(250).await;
    }
}

pub fn initalize_rmt(peripherals: &Peripherals) -> Rmt<'static, Blocking> {
    let peripheral_rmt = unsafe { peripherals.RMT.clone_unchecked() };

    let freq = Rate::from_mhz(80);
    let rmt = Rmt::new(peripheral_rmt, freq);

    let Ok(rmt) = rmt else {
        panic!("Failed to create rmt");
    };

    rmt
}

pub fn get_rotery_encoder_io(
    peripherals: &Peripherals,
) -> (Input<'static>, Input<'static>, Input<'static>) {
    let switch_pin = unsafe { peripherals.GPIO9.clone_unchecked() };
    let dt_pin = unsafe { peripherals.GPIO8.clone_unchecked() };
    let clock_pin = unsafe { peripherals.GPIO7.clone_unchecked() };

    // Using Pull::None as Pull::Up changes the total resistance of the
    // zero cross detection circitry. Changining the timing for the zero cross pulse.
    // Since it adds the internal pull up resistance in parellel with the on board pull up resistor
    let no_pullup = InputConfig::default().with_pull(Pull::None);
    let pullup = InputConfig::default().with_pull(Pull::None);
    let clock_input = Input::new(dt_pin, no_pullup);
    let dt_input = Input::new(clock_pin, no_pullup);
    let switch_input = Input::new(switch_pin, pullup);

    (clock_input, dt_input, switch_input)
}

pub fn get_dimmer_io(peripherals: &Peripherals) -> (Input<'static>, Output<'static>) {
    let signal_pin = unsafe { peripherals.GPIO6.clone_unchecked() }; // D3 on arduino
    let gate_pin = unsafe { peripherals.GPIO5.clone_unchecked() }; // D2 on arudino

    // Using Pull::None as Pull::Up changes the total resistance of the
    // zero cross detection circitry. Changining the timing for the zero cross pulse.
    // Since it adds the internal pull up resistance in parellel with the on board pull up resistor
    let zero_cross_config = InputConfig::default().with_pull(Pull::None);
    let zero_cross_input = Input::new(signal_pin, zero_cross_config);

    let triac_gate_config = OutputConfig::default();
    let triac_gate_output = Output::new(gate_pin, Level::Low, triac_gate_config);

    (zero_cross_input, triac_gate_output)
}
