#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![feature(never_type)]

pub mod app;
pub mod drivers;
pub mod persistance;
pub mod io;

/// The main entry point for the application.
/// 
/// Args
/// - `spawner`: The RTOS task spawner.
/// - `peripherals`: The application peripherals.
///
/// This function never returns.
pub async fn app_main(spawner : embassy_executor::Spawner, peripherals: io::AppPeripherals) -> ! {
    app::App::main(spawner, peripherals).await
}