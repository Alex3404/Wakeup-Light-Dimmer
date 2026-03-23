#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use arduino_esp32_dimmer::{lamp_dimmer, pulse_scheduler};
use esp_hal::rmt::Rmt;
use log::info;

use core::f32::consts::PI;
use libm::sinf;

use esp_hal::clock::CpuClock;
use esp_hal::main;

use esp_hal::gpio::{Input, InputConfig, Level, Output};
use esp_hal::gpio::{OutputConfig, Pull};
use esp_hal::pcnt::Pcnt;
use esp_hal::time::{Duration, Instant, Rate};
use esp_hal::timer::timg::TimerGroup;

use esp_backtrace as _;

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();

    // generator version: 1.2.0
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let timer_group_0 = TimerGroup::new(peripherals.TIMG0);
    let pcnt = Pcnt::new(peripherals.PCNT);

    let freq = Rate::from_mhz(80);
    let rmt = Rmt::new(peripherals.RMT, freq);
    let Ok(rmt) = rmt else {
        panic!("Failed to create rmt");
    };

    // Get the hardware timer and pcnt unit for our dimmer
    // let dimming_timer = timer_group_0.timer1;
    let rtos_timer = timer_group_0.timer0;

    // Discribes which pin the zero cross signal pull up pin is connected to
    let pullup_config = InputConfig::default().with_pull(Pull::Up);
    let signal_pin = Input::new(peripherals.GPIO7, pullup_config);

    // Configure the triac gate pin starting at logical Low
    let gate_pin = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let rmt_channel_0 = rmt.channel0;

    // Initalize the heap allocator with 72000 bytes of ram
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72000);

    // Intitalize the dimmer
    // let pulse_scheduler =
    //     pulse_scheduler::PulseScheduler::new(dimming_timer, gate_pin, Level::Low, rmt_channel_0);

    // let Ok(pulse_scheduler) = pulse_scheduler else {
    //     panic!("Unable to create pulse scheduler");
    // };

    info!("Intitalizing lamp dimmer!");

    let lamp_dimmer = lamp_dimmer::LampDimmer::initalize(pcnt, signal_pin, gate_pin, rmt_channel_0);
    let Ok(lamp_dimmer) = lamp_dimmer else {
        panic!("Unable to create lamp dimmer");
    };

    info!("Start rtos");

    // Start the ESP Real Time Operating System
    // Give the OS a hardware timer for Async
    esp_rtos::start(rtos_timer);

    // let p = esp_hal::init(esp_hal::Config::default());
    // esp_alloc::psram_allocator!(p.PSRAM, esp_hal::psram);

    // Coexist Libraries needs more RAM - so we've added some more
    // Coexist Libraries allow the support of multiple wireless protocols
    // We need this since we are using both Bluetooth and WI-FI
    // esp_alloc::heap_allocator!(size: 64 * 512);

    // Initalize ESP radio for WIFI and/or Bluetooth
    // let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");

    // Intialize the WI-FI controller for the ESP32
    // let (mut _wifi_controller, _interfaces) =
    //     esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
    //         .expect("Failed to initialize Wi-Fi controller");

    // Intialize the Bluetooth Host Controller Interface
    // let _connector = BleConnector::new(&radio_init, peripherals.BT, Default::default());

    info!("Main loop started!");
    loop {
        // let now = pulse_scheduler::now();
        // let pulse_err =
        //     pulse_scheduler::schedule_pulse(now + Duration::from_secs(1), Duration::from_secs(1));

        // esp_hal::delay::Delay::new().delay_millis(2000);

        const BREATHING_TIME_MS: f32 = 15000.0;
        let milis = Instant::now().duration_since_epoch().as_millis();
        let angle = ((milis % (BREATHING_TIME_MS) as u64) as f32 / BREATHING_TIME_MS) * 2.0 * PI;
        let brightness = (sinf(angle) + 1.0) / 2.0;
        critical_section::with(|cs| {
            lamp_dimmer
                .borrow_ref_mut(cs)
                .set_brightness((brightness * 100.0) as u8);
        });

        esp_hal::delay::Delay::new().delay_micros(1);
    }
}
