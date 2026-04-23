use crate::app::core::app_main;
use crate::app::io;

use esp_hal::peripherals::Peripherals;
use log::info;
use static_cell::StaticCell;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use esp_hal::system::Stack;
use esp_radio::ble::controller::BleConnector;
use esp_rtos::embassy::Executor;

static APP_PERIPHERALS: Signal<CriticalSectionRawMutex, io::AppCorePeripherals> = Signal::new();
static APP_CORE_STACK: StaticCell<Stack<16384>> = StaticCell::new();
static APP_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

#[embassy_executor::task]
async fn app_runner(spawner: Spawner) {
    let peripherals = APP_PERIPHERALS.wait().await;
    app_main(spawner, peripherals).await;
}

fn app_core(spawner: Spawner) -> () {
    // Run app main
    let token = app_runner(spawner);
    spawner.spawn(token.expect("App runner failed to spawn!"));
}

// Creates a new instance of the app.
// Initalizes all the peripherals and drivers, and spawns the necessary tasks.
pub fn run(_main_spawner: Spawner, peripherals: Peripherals) -> ! {
    let (app_core_peripherals, main_core_peripherals) = io::split_peripherals(peripherals);

    // Start RTOS on main core
    esp_rtos::start(
        main_core_peripherals.rtos_timer,
        main_core_peripherals.sw_interrupt_0,
    );
    info!("RTOS started!");

    // Create stack for app core
    let app_core_stack = APP_CORE_STACK.init(Stack::new());

    // Start RTOS on second core
    esp_rtos::start_second_core(
        main_core_peripherals.cpu_control,
        main_core_peripherals.sw_interrupt_1,
        app_core_stack,
        move || {
            let executor = APP_EXECUTOR.init(Executor::new());
            executor.run(app_core);
        },
    );
    info!("APP core started!");

    // Start up wifi and bluetooth controller
    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(main_core_peripherals.wifi, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    let _connector = BleConnector::new(main_core_peripherals.bluetooth, Default::default());

    APP_PERIPHERALS.signal(app_core_peripherals);
    loop {}
}
