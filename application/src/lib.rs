#![no_std]
#![feature(impl_trait_in_assoc_type)]

pub mod app;
pub mod drivers;
pub mod persistance;
pub mod io;

pub async fn app_main(spawner : embassy_executor::Spawner, peripherals: io::AppPeripherals) -> ! {
    app::App::main(spawner, peripherals).await
}