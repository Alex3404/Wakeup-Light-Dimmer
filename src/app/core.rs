use crate::app::lamp_dimmer::DimmerSettings;

use super::input::{RoteryDecoder, RoteryDecoderConfig};
use super::io::{self, DimmerIO};
use super::lamp_dimmer::{DimmerChannel, DimmerChannelConfig, DimmerSettingsBuilder, TimingConfig};
use super::persistance::{AppState, AppStorage};
use super::ui::{self, MenuController, MenuControllerHandle};

use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::rwlock::RwLock;
use embassy_sync::signal::Signal;
use embassy_sync::watch::{DynAnonReceiver, DynSender, Watch};
use esp_hal::Async;
use esp_hal::i2c::master::I2c;
use log::info;
use static_cell::StaticCell;

use embassy_executor::Spawner;

extern crate alloc;
use alloc::sync::{Arc, Weak};

pub type AppHandle = Weak<App>;
pub type DimmerHandle = Arc<RwLock<NoopRawMutex, DimmerChannel>>;

// Provides a static reference to the app instance
pub type StrongAppHandle = Arc<App>;
static APP_CELL: StaticCell<StrongAppHandle> = StaticCell::new();
static USER_DATA_WATCH: StaticCell<Watch<NoopRawMutex, AppState, 6>> = StaticCell::new();

// Specific timing configuration for our dimmer channel, tuned for our circuit and triac
static CIRCIT_TIMING_CONFIG: TimingConfig = TimingConfig::default()
    .with_min_latch_time(150)
    .with_latch_time_after_zero(1500)
    .with_latch_time_before_next_zero(750);

/// App core module that initializes and holds references to all submodules/drivers and shared state
#[allow(dead_code)]
pub struct App {
    app_thread_spawner: Spawner,
    rotery_decoder: RoteryDecoder,
    main_dimmer: DimmerHandle,
    menu_controller: MenuControllerHandle,

    user_data: &'static Watch<NoopRawMutex, AppState, 6>,
    user_data_sender: DynSender<'static, AppState>,

    app_storage: RwLock<NoopRawMutex, AppStorage>,
}

impl App {
    /// Create a DimmerSettingsBuilder for configuring and previewing dimmer settings
    pub async fn dimmer_settings_builder(
        this: StrongAppHandle,
    ) -> Result<DimmerSettingsBuilder, ()> {
        let publish_signal = Arc::new(Signal::new());
        let mut dimmer = this.main_dimmer.write().await;
        let spawner = this.app_thread_spawner.clone();
        let builder = dimmer.new_settings_builder(publish_signal.clone()); // Pass the signal to the builder

        if let Ok(builder) = builder {
            // Spawn the task that waits for the publish signal
            let token = wait_for_publish(this.clone(), publish_signal);
            if let Ok(token) = token {
                spawner.spawn(token);
            }

            Ok(builder)
        } else {
            Err(())
        }
    }

    /// Finish initialization of submodules that require async setup
    async fn finish_initialization(&self) {
        let mut menu_lock = self.menu_controller.write().await;
        menu_lock.finish_initialization().await;
        drop(menu_lock);
    }
}

pub(super) async fn app_main(spawner: Spawner, peripherals: io::AppCorePeripherals) {
    info!("App main task started!");

    let mut app_storage =
        AppStorage::new(peripherals.flash).expect("Failed to load userdata storage!");

    let mut buffer = [0u8; 128];
    let user_data = app_storage
        .read(&mut buffer)
        .await
        .expect("Failed to read userdata!")
        .map_or(AppState::default(), |v| v);

    let app = APP_CELL.init(Arc::new_cyclic(|app| {
        let watch = USER_DATA_WATCH.init(Watch::new_with(user_data));

        let main_dimmer = initalize_dimmer(&user_data, peripherals.dimmer_io);
        let menu_controller = initalize_menu_controller(
            spawner,
            peripherals.i2c,
            app.clone(),
            watch.dyn_anon_receiver(),
            watch.dyn_sender(),
        );
        let rotery_decoder =
            initalize_rotery_decoder(spawner, peripherals.rotery_io, &menu_controller);

        App {
            app_thread_spawner: spawner,
            rotery_decoder,
            main_dimmer,
            menu_controller,
            user_data: watch,
            app_storage: RwLock::new(app_storage),
            user_data_sender: watch.dyn_sender(),
        }
    }));

    app.finish_initialization().await;

    // Spawn the propagator task with static reference to the dimmer
    let token = settings_propagator(app.clone());
    spawner.spawn(token.expect("Settings propagator failed to spawn!"));
}

/// Task that waits for a publish signal from the dimmer settings builder
#[embassy_executor::task]
async fn wait_for_publish(
    app: StrongAppHandle,
    publish_signal: Arc<Signal<CriticalSectionRawMutex, DimmerSettings>>,
) {
    let new_settings = publish_signal.wait().await;

    app.user_data_sender.send_modify(|user_data| {
        if let Some(user_data) = user_data {
            user_data.dimmer_settings = new_settings;
        }
    });
}

/// Task that propagates user data changes to submodules
#[embassy_executor::task]
async fn settings_propagator(app: StrongAppHandle) {
    let user_data_receiver = app.user_data.receiver();
    let Some(mut user_data_receiver) = user_data_receiver else {
        panic!("Failed to create user data receiver!");
    };

    loop {
        let updated_user_data = user_data_receiver.changed().await;

        let mut dimmer = app.main_dimmer.write().await;
        dimmer.set_state(updated_user_data.dimmer_state);
        drop(dimmer);

        let mut app_storage = app.app_storage.write().await;
        let mut buffer = [0u8; 128];

        app_storage
            .write(&updated_user_data, &mut buffer)
            .await
            .expect("Failed to write userdata!");
    }
}

fn initalize_dimmer(user_data: &AppState, dimmer_io: DimmerIO) -> DimmerHandle {
    // Initalize the dimmer driver
    let dimmer_io = dimmer_io;
    let dimmer_config = DimmerChannelConfig::new(
        60,
        CIRCIT_TIMING_CONFIG,
        dimmer_io.zero_cross.peripheral_input(),
        dimmer_io.gate.into_peripheral_output(),
        dimmer_io.mcpwm,
    )
    .with_dimmer_settings(user_data.dimmer_settings)
    .with_starting_state(user_data.dimmer_state);

    let main_dimmer = DimmerChannel::new(dimmer_config);

    // Wrap main_dimmer result
    let main_dimmer = main_dimmer.expect("Failed to create main dimmer channel");
    Arc::new(RwLock::new(main_dimmer))
}

fn initalize_menu_controller(
    spawner: Spawner,
    i2c: I2c<'static, Async>,
    app: AppHandle,
    user_data_receive: DynAnonReceiver<'static, AppState>,
    user_data_sender: DynSender<'static, AppState>,
) -> MenuControllerHandle {
    MenuController::new(spawner, i2c, app, user_data_receive, user_data_sender)
        .expect("Failed to create menu controller")
}

fn initalize_rotery_decoder(
    spawner: Spawner,
    rotery_io: io::RoteryIO,
    menu_controller: &MenuControllerHandle,
) -> RoteryDecoder {
    let rotery_interface =
        ui::MenuController::create_rotery_interface(Arc::downgrade(menu_controller), spawner)
            .expect("Failed to create rotery interface");

    let rotery_config =
        RoteryDecoderConfig::new(rotery_io.clock, rotery_io.rotate, rotery_interface)
            .with_switch(rotery_io.switch);
    RoteryDecoder::new(spawner, rotery_config).expect("Failed to create rotery decoder")
}
