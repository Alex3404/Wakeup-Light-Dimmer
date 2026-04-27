use crate::app::io::AppCorePeripherals;

use super::input::{RoteryDecoder, RoteryDecoderConfig};
use super::lamp_dimmer::{DimmerChannel, DimmerChannelConfig, TimingConfig};
use super::persistance::{AppState, AppStorage};
use super::ui::MenuController;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::rwlock::RwLock;
use embassy_sync::watch::{AnonReceiver, Receiver, Sender, Watch};
use log::info;
use static_cell::StaticCell;

use embassy_executor::Spawner;

// Provides a static reference to the app instance
static APP_CELL: StaticCell<App> = StaticCell::new();
static USER_DATA_WATCH: StaticCell<Watch<NoopRawMutex, AppState, 6>> = StaticCell::new();

// Specific timing configuration for our dimmer channel, tuned for our circuit and triac
static DIMMER_TIMING_CONFIG: TimingConfig = TimingConfig::default()
    .with_min_latch_time(150)
    .with_latch_time_after_zero(1500)
    .with_latch_time_before_next_zero(750);

const APP_STATE_WATCH_SIZE: usize = 6;
pub type AppStateReceiver = Receiver<'static, NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;
pub type AppStateSender = Sender<'static, NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;
pub type AppStateAnonReceiver = AnonReceiver<'static, NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;
type AppStateWatch = Watch<NoopRawMutex, AppState, APP_STATE_WATCH_SIZE>;

/// App core module that initializes and holds references to all submodules/drivers and shared state
#[allow(dead_code)]
pub struct App {
    app_thread_spawner: Spawner,
    rotery_decoder: RoteryDecoder,
    main_dimmer: &'static DimmerChannel,
    menu_controller: &'static MenuController,

    user_data: &'static AppStateWatch,
    user_data_sender: AppStateSender,

    app_storage: RwLock<NoopRawMutex, AppStorage>,
}

pub(super) async fn app_main(spawner: Spawner, peripherals: AppCorePeripherals) {
    info!("App main task started!");

    let mut app_storage =
        AppStorage::new(peripherals.flash).expect("Failed to load userdata storage!");

    static BUFFER_CELL: StaticCell<[u8; 128]> = StaticCell::new();
    let buffer = BUFFER_CELL.init_with(|| [0u8; 128]);

    let user_data = app_storage
        .read(buffer)
        .await
        .expect("Failed to read userdata!")
        .map_or(AppState::default(), |v| v);

    // Initalize the dimmer driver
    let dimmer_io = peripherals.dimmer_io;
    let dimmer_config = DimmerChannelConfig::new(
        60,
        DIMMER_TIMING_CONFIG,
        dimmer_io.zero_cross.peripheral_input(),
        dimmer_io.gate.into_peripheral_output(),
        dimmer_io.mcpwm,
    )
    .with_dimmer_settings(user_data.dimmer_settings)
    .with_starting_state(user_data.dimmer_state);

    let main_dimmer = DimmerChannel::new(spawner, dimmer_config);

    let watch = USER_DATA_WATCH.init(Watch::new_with(user_data));

    // Create menu controller
    let menu_controller = MenuController::initalize(
        spawner,
        peripherals.i2c,
        watch.receiver().unwrap(),
        watch.sender(),
    )
    .await;

    // Create rotery interface and decoder
    let rotery_io = peripherals.rotery_io;
    let rotery_interface = menu_controller.create_rotery_interface(spawner.clone());
    let rotery_config =
        RoteryDecoderConfig::new(rotery_io.clock, rotery_io.rotate, rotery_interface)
            .with_switch(rotery_io.switch);

    let rotery_decoder =
        RoteryDecoder::new(spawner, rotery_config).expect("Failed to create rotery decoder");

    let app = APP_CELL.init_with(|| App {
        app_thread_spawner: spawner,
        rotery_decoder,
        main_dimmer,
        menu_controller,
        user_data: watch,
        app_storage: RwLock::new(app_storage),
        user_data_sender: watch.sender(),
    });

    // Spawn the propagator task with static reference to the dimmer
    let token = settings_propagator(app);
    spawner.spawn(token.expect("Settings propagator failed to spawn!"));
}

/// Task that propagates user data changes to submodules
#[embassy_executor::task]
async fn settings_propagator(app: &'static App) {
    let mut buffer = [0u8; 128];
    let user_data_receiver = app.user_data.receiver();
    let Some(mut user_data_receiver) = user_data_receiver else {
        panic!("Failed to create user data receiver!");
    };

    loop {
        let updated_user_data = user_data_receiver.changed().await;

        app.main_dimmer.update_state(updated_user_data.dimmer_state);

        let mut app_storage = app.app_storage.write().await;
        app_storage
            .write(&updated_user_data, &mut buffer)
            .await
            .expect("Failed to write userdata!");
    }
}
