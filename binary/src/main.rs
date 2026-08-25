#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]
#![feature(impl_trait_in_assoc_type)]

use application::app_main;
use application::io::{split};

use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;
use panic_rtt_target as _;
use static_cell::StaticCell;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
fn main() -> ! {
    static EXECUTOR : StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();
    let executor = EXECUTOR.init(esp_rtos::embassy::Executor::new());

    executor.run(|spawner| {
        spawner.spawn(__async_main(spawner).unwrap());
    })
}

#[embassy_executor::task]
async fn __async_main(spawner: Spawner) -> ! {
    // Configure the ESP32 peripherals and CPU clock
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Initalize the heap allocator with 72000 bytes of ram
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64000);
    esp_alloc::heap_allocator!(size: 52 * 1024); // Additional heap for general use

    // Initialize the RTT (Real-Time Transfer) for defmt logging
    rtt_target::rtt_init_defmt!();
    defmt::info!("Starting main function");

    let (rtos_peripherals, app_peripherals) = split(peripherals);

    // Start the FreeRTOS scheduler
    esp_rtos::start(rtos_peripherals.rtos_timer, rtos_peripherals.sw_interrupt_0);

    defmt::info!("Starting main application task");
    // Start the main application task
    app_main(spawner, app_peripherals).await
}