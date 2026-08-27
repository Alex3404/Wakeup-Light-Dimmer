extern crate alloc;

use core::cell::RefCell;

use crate::app::ui::slint_ui::DimmerUI;
use crate::drivers::{lamp_dimmer::*};
use crate::drivers::lamp_dimmer::esp_dimmer::{self, *};
use crate::drivers::rotery_decoder::{RoteryDecoder, RoteryDecoderConfig, RoteryInterface};
use crate::persistance::{AsyncStorage, ReactiveAsyncStorage, StorageData};
use crate::app::AppState;

use crate::io::AppPeripherals;
// use crate::app::ui::MenuController;
use crate::persistance::esp_impl::async_nvs_storage;

use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::yield_now;
use embassy_time::Duration;
use esp_hal::gpio::Output;
use esp_radio::wifi::ControllerConfig;
use esp_storage::{FlashStorage};
use fixed::traits::{Fixed, FromFixed};
use fixed::types::{I24F8, U0F16, U16F16};
use pcf85063a::PCF85063;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::watch::{Receiver, Sender, Watch};
use bt_hci::controller::ExternalController;
use esp_hal::{mcpwm::AnyMcPwm};
use esp_radio::{ble::controller::{BleConnector}, wifi::Interfaces};
use sequential_storage::cache::{Cache, Uncached};
use embassy_executor::Spawner;

// App state watcher helper types
const APP_STATE_WATCH_SIZE: usize = 6;
pub type AppStateWatch = Watch<NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;
pub type AppStateReceiver = Receiver<'static, NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;
pub type AppStateSender = Sender<'static, NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;

// Specific timing configuration for our dimmer channel, tuned for our circuit and triac
static PWR_DIMMING_CONFIG: PowerDimmingConfig = PowerDimmingConfig::default()
    .with_dimming_mode(AcDimmingMode::LeadingEdge)
    .with_gate(
        GateConfig::default()
            .with_minimum_gate_time(1500)
            .with_leading_deadtime(500)
            .with_trailing_deadtime(500)
    );


// Type alias for convenience
pub type ReactiveAppStorage = ReactiveAsyncStorage<
    'static, NoopRawMutex, u64,
    BlockingAsync<FlashStorage<'static>>, Cache<Uncached, Uncached, Uncached, u64>,
    AppState, APP_STATE_WATCH_SIZE,
    { AppState::BUFF_SIZE }>;
type AppAsyncStorage = AsyncStorage<NoopRawMutex, u64, BlockingAsync<FlashStorage<'static>>, Cache<Uncached, Uncached, Uncached, u64>>;
type BleController<'a> = ExternalController<BleConnector<'a>, 20>;
type WifiControl<'a> = esp_radio::wifi::WifiController<'a>;

struct Dimmer {
    app_state_sender : AppStateSender,
}

impl Dimmer {
    const ONE_PERCENT : U0F16 = Brightness::FULL.0.checked_div_int(100).unwrap();
}

impl RoteryInterface for Dimmer {
    fn pressed(&self, _pressed: bool) {
        todo!()
    }

    fn rotate_cw(&self, rpm: I24F8) {
        defmt::debug!("Rotating clockwise with rpm: {}", rpm.saturating_to_num::<u16>());
        let increment = U16F16::saturating_from_fixed(rpm)
            .checked_div_int(10).unwrap_or(U16F16::const_from_int(10))
            .saturating_mul(U16F16::checked_from_fixed(Dimmer::ONE_PERCENT).unwrap_or_default());
        let increment = U0F16::checked_from_fixed(increment.frac()).unwrap_or_default();

        self.app_state_sender.send_modify(|state| {
            if let Some(app) = state {
                app.brightness = Brightness(app.brightness.0.saturating_add(increment));
            }
        });
    }

    fn rotate_ccw(&self, rpm: I24F8) {
        defmt::debug!("Rotating counter-clockwise with rpm: {}", rpm.saturating_to_num::<u16>());
        let increment = U16F16::saturating_from_fixed(rpm)
            .checked_div_int(10).unwrap_or(U16F16::const_from_int(10))
            .clamp(U16F16::const_from_int(1), U16F16::const_from_int(20))
            .saturating_mul(U16F16::checked_from_fixed(Dimmer::ONE_PERCENT).unwrap_or_default());
        let increment = U0F16::checked_from_fixed(increment.frac()).unwrap_or_default();

        self.app_state_sender.send_modify(|user_data| {
            if let Some(app_state) = user_data {
                app_state.brightness = Brightness(app_state.brightness.0.saturating_sub(increment));
            }
        });
    }
}

/// App core module that initializes and holds references to all submodules/drivers and shared state
#[allow(unused)]
pub struct App {
    spawner: Spawner,
    // rotery_decoder: RoteryDecoder<'static, Dimmer>,
    // menu_controller: &'static MenuController,

    // App state watch for broadcasting changes to the application state
    state_watch: &'static AppStateWatch,
    // App state receiver for receiving updates to the application state
    state_receiver: AppStateReceiver,

    // Optional these can be unavailable if initialization fails
    main_dimmer: RefCell<Option<BasicDimmer<EspPwmDimmer>>>,
    app_storage: RefCell<Option<AppAsyncStorage>>,
}

impl App {
    /// The main entry point for the application core.
    /// 
    /// Args
    /// - `spawner`: The RTOS task spawner.
    /// - `peripherals`: The application peripherals.
    ///
    /// This function never returns.
    pub async fn main(spawner: Spawner, peripherals: AppPeripherals) -> ! {
        defmt::info!("App main task started!");
        
        // Initialize WiFi controller
        let (mut _wifi_controller, _wifi_interfaces) = Self::init_wifi(peripherals.wifi);

        if let Some(mut _wifi_controller) = _wifi_controller {
            defmt::info!("WiFi controller initialized successfully.");
            _wifi_controller.connect_async().await;
        }

        // Start up bluetooth controller
        let _bluetooth_controller = Self::init_bluetooth(peripherals.bluetooth);

        // Setup application storage and user data
        let mut app_storage = match async_nvs_storage(peripherals.flash) {
            Ok(storage) => Some(storage),
            Err(err) => {
                defmt::error!("Failed to initialize app storage: {:?}", err);
                defmt::error!("No app storage available! Continuing without persistent storage.");
                None
            }
        };
        
        // Init our app state watch
        let mut disable_writes_flag = false;
        let app_state_watch = Self::init_app_state_watch(&mut app_storage, &mut disable_writes_flag).await;

        let reactive_app_storage = if disable_writes_flag {
            defmt::warn!("Disabling app storage due to read error.");
            app_storage = None; // Removed to prevent further writes
            None
        } else {
            defmt::info!("App storage initialized successfully.");
            app_storage.as_ref().map(|app| app.request(app_state_watch.receiver().unwrap()))
        };

        // Initalize the dimmer driver
        esp_dimmer::init(spawner);
        let mcpwm = esp_dimmer::configure_mcpwm(AnyMcPwm::from(peripherals.dimmer.mcpwm));
        let dimmer_config = EspPwmDimmerConfig::new(
            mcpwm,
            peripherals.dimmer.gate.into_peripheral_output(),
            peripherals.dimmer.zero_cross.peripheral_input(),
            true,
            PWR_DIMMING_CONFIG,
        );

        // Attempt to create the ESP PWM dimmer. If it fails, the dimmer will be unavailable.
        let esp_pwm_dimmer = match EspPwmDimmer::new(dimmer_config) {
            Ok(dimmer) => Some(dimmer),
            Err(err) => {
                defmt::warn!("Failed to create ESP PWM dimmer, dimmer will be unavailable: {:?}.", err);
                None
            }
        };

        let main_dimmer = esp_pwm_dimmer.map(|dimmer| {
            defmt::info!("Dimmer initialized!");
            BasicDimmer::new(dimmer, DimmerConfig::default())
        });

        // Initialize the RTC clock
        let _rtc_clock = PCF85063::new(peripherals.i2c_device.clone());

        // Create rotery interface and decoder
        let rotery_dimmer = static_cell::make_static!(Dimmer {
                app_state_sender: app_state_watch.sender(),
            });

        let rotery_config = RoteryDecoderConfig::new(
            peripherals.rotery.clock, peripherals.rotery.rotate, rotery_dimmer, 20
        ).with_switch(peripherals.rotery.switch);

        let rotery_decoder = RoteryDecoder::new(rotery_config);
        defmt::info!("Rotery decoder initialized!");

        let ui = DimmerUI::new(spawner,
            peripherals.i2c_device.clone(),
            app_state_watch.receiver().expect("Increase receiver count!")
        ).await;

        match ui {
            Ok(ui) => {
                defmt::info!("UI initialized successfully");
                spawner.spawn(run_ui(ui).unwrap());
            },
            Err(_) => {
                defmt::error!("Failed to initialize UI");
            }
        }

        let app : &'static mut App = static_cell::make_static!(App {
            spawner,
            // rotery_decoder,
            // menu_controller,
            state_watch: app_state_watch,
            state_receiver: app_state_watch.receiver().expect("Increase receiver count!"),
            app_storage: RefCell::new(app_storage),
            main_dimmer: RefCell::new(main_dimmer),
        });

        defmt::info!("App Created!");

        let bg_tasks = BackgroundTasks {
            rotery_decoder,
            test_led: peripherals.test_led,
            reactive_app_storage,
        };

        app.app_loop(bg_tasks).await;
    }
}

pub struct BackgroundTasks {
    rotery_decoder : RoteryDecoder<'static, Dimmer>,
    test_led : Output<'static>,
    reactive_app_storage: Option<ReactiveAppStorage>,
}

#[embassy_executor::task]
async fn rotery_task(mut rotery_decoder: RoteryDecoder<'static, Dimmer>) {
    rotery_decoder.run_loop().await;
}

impl App {
    /// Main application loop that handles state changes and spawns background tasks.
    async fn app_loop(&'static mut self, bg_tasks: BackgroundTasks) -> ! {
        /// Spawns the background tasks for the application!
        self.spawner.spawn(rotery_task(bg_tasks.rotery_decoder).unwrap());
        self.spawner.spawn(blink_led(bg_tasks.test_led).unwrap());

        // Spawn the reactive storage task if it exists
        if let Some(reactive_app_storage) = bg_tasks.reactive_app_storage {
            self.spawner.spawn(run_reactive_storage(reactive_app_storage).unwrap());
        }

        loop {
            // Wait for changes in the app state
            let new_state = self.state_receiver.changed().await;

            if let Some(ref mut dimmer) = self.main_dimmer.borrow_mut().as_mut() {
                defmt::debug!("New brightness for dimmer: {:?}%", new_state.brightness().0.saturating_to_num::<f32>() * 100.0);
                let _ = dimmer.set_brightness(new_state.brightness());
            }

            // Yield to allow other tasks to run
            yield_now().await;
        }
    }
}

static SSID : &'static str = env!("WIFI_SSID");
static PASSWORD : &'static str = env!("WIFI_PASSWORD");

//============================//
// Application Initialization //
//============================//
impl App {

    /// Initialize the app state watch with the current app state.e
    async fn init_app_state_watch(app_storage : &mut Option<AppAsyncStorage>, disable_writes_flag : &mut bool) -> &'static AppStateWatch {
        use crate::app::app_state::{AppState};
        
        let mut buffer = [0u8; AppState::BUFF_SIZE];
        let app_state = if !*disable_writes_flag {
            if let Some(app_storage) = app_storage {
                app_storage.read_or_default(&mut buffer, AppState::default()).await
            } else {
                Ok(AppState::default())
            }
        } else {
            Ok(AppState::default())
        };

        let app_state = match app_state {
            Ok(state) => state,
            Err(err) => {
                defmt::warn!("Storage error when reading app state: {:?}", err);
                *disable_writes_flag = true; // Disable writes to storage
                AppState::default()
            }
        };

        // App state storage
        let app_state_watch : &'static AppStateWatch = static_cell::make_static!(Watch::new_with(app_state));
        app_state_watch
    }

    /// Initialize the WiFi module with the given peripheral.
    fn init_wifi<'a>(wifi: esp_hal::peripherals::WIFI<'a>) -> (Option<WifiControl<'a>>, Option<Interfaces<'a>>) {
        use esp_radio::wifi::{Config, AuthenticationMethod};
        use esp_radio::wifi::sta::StationConfig;

        let station_config = Config::Station(
            StationConfig::default()
                .with_ssid(alloc::string::String::from(SSID))
                .with_auth_method(AuthenticationMethod::Wpa2Personal)
                .with_password(alloc::string::String::from(PASSWORD))
        );

        let controller_config = ControllerConfig::default();

        match esp_radio::wifi::new(wifi, Default::default()) {
            Ok((ctrl, ifaces)) => {
                defmt::info!("WiFi initialized successfully");
                (Some(ctrl), Some(ifaces))
            },
            Err(e) => {
                defmt::warn!("Failed to initialize wifi: {:?}. Continuing without it", e);
                (None, None)
            },
        }
    }

    /// Initialize the Bluetooth module with the given peripheral.
    fn init_bluetooth<'a>(bluetooth: esp_hal::peripherals::BT<'a>) -> Option<BleController<'a>> {
        let ble_result = BleConnector::new(bluetooth, Default::default()).map(|ble| {
            ExternalController::new(ble)
        });

        match ble_result {
            Ok(ctrl) => {
                defmt::info!("Bluetooth initialized successfully");
                Some(ctrl)
            },
            Err(e) => {
                defmt::warn!("Failed to initialize bluetooth: {:?}. Continuing without it", e);
                None
            },
        }
    }    
}

#[embassy_executor::task]
async fn run_reactive_storage(reactive_app_storage: ReactiveAppStorage) {
    reactive_app_storage.storage_loop().await.unwrap();
}

#[embassy_executor::task]
async fn run_ui(dimmer_ui : DimmerUI) -> () {
    match dimmer_ui.run().await {
        Ok(_) => {
            defmt::warn!("UI loop exited unexpectedly");
        },
        Err(_) => {
            defmt::error!("UI run error");
        }
    };
}

#[embassy_executor::task]
async fn blink_led(mut led: Output<'static>) -> ! {
    loop {
        led.set_high();
        embassy_time::Timer::after(Duration::from_millis(500)).await;
        led.set_low();
        embassy_time::Timer::after(Duration::from_millis(500)).await;
    }
}