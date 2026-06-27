use crate::app::core::app_main;
use crate::app::io::AppPeripherals;

use defmt::info;
use embassy_executor::Spawner;
use esp_hal::peripherals::Peripherals;

// Creates a new instance of the app.
// Initalizes all the peripherals and drivers, and spawns the necessary tasks.
pub async fn run(spawner: Spawner, peripherals: Peripherals) -> ! {
    let app_peripherals = AppPeripherals::new(peripherals);
    info!("Starting app main task");
    app_main(spawner, app_peripherals).await
}
