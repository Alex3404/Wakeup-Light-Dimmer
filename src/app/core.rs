use crate::app::io::{self, DimmerIO};
use crate::app::signal_cell::SignalCell;
use crate::app::user_data::{self, UserData, UserDataStorage};
use crate::input::{RoteryDecoder, RoteryDecoderConfig};
use crate::lamp_dimmer::{
    DimmerChannel, DimmerChannelConfig, DimmerSettingsBuilder, MAX_BRIGHTNESS, MIN_BRIGHTNESS,
    TimingConfig,
};
use crate::ui::{self, MenuControllerHandle};

use core::cell::RefCell;
use esp_hal::Async;
use esp_hal::i2c::master::I2c;
use log::info;
use static_cell::StaticCell;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::NoopMutex;

extern crate alloc;
use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};

pub type AppHandle = Weak<NoopMutex<App>>;
pub type StrongAppHandle = Rc<NoopMutex<App>>;
pub type DimmerHandle = Rc<NoopMutex<RefCell<DimmerChannel>>>;

static DEFAULT_TIMING_CONFIG: TimingConfig = TimingConfig::default()
    .with_min_latch_time(150)
    .with_latch_time_after_zero(1500)
    .with_latch_time_before_next_zero(750);

// Provides a static reference to the app instance
static APP_CELL: StaticCell<StrongAppHandle> = StaticCell::new();

#[allow(dead_code)]
pub struct App {
    rotery_decoder: RoteryDecoder,
    main_dimmer: DimmerHandle,
    menu_controller: MenuControllerHandle,
    user_data: Rc<SignalCell<UserData>>,
}

impl App {
    pub fn get_dimmer_handle(&self) -> DimmerHandle {
        self.main_dimmer.clone()
    }

    pub fn get_menu_controller_handle(&self) -> MenuControllerHandle {
        self.menu_controller.clone()
    }

    pub fn get_user_data(&self) -> Rc<SignalCell<UserData>> {
        self.user_data.clone()
    }

    pub fn update_brightness(&self, call: impl FnOnce(u8) -> u8) {
        self.user_data.set(|user_data| {
            let brightness = call(user_data.dimmer_state.brightness);
            user_data.dimmer_state.brightness = brightness.clamp(MIN_BRIGHTNESS, MAX_BRIGHTNESS);
        });
    }

    pub fn toggle_light(&self) {
        self.user_data.set(|user_data| {
            user_data.dimmer_state.is_on = !user_data.dimmer_state.is_on;
        });
    }

    async fn finish_initialization(&self) {
        // Finish initialization of submodules that require async setup
        let mut menu_lock = self.menu_controller.write().await;
        info!("Finishing menu controller initialization...");
        menu_lock.finish_initialization().await;
        drop(menu_lock);
    }

    /// Create a DimmerSettingsBuilder for configuring and previewing dimmer settings
    pub fn dimmer_settings_builder(this: StrongAppHandle) -> Result<DimmerSettingsBuilder, ()> {
        let this_clone = this.clone();
        let callback = Box::new(move |settings| {
            // This callback is called when builder.publish() is invoked
            this_clone.lock(|app| {
                app.user_data.set(|user_data| {
                    user_data.dimmer_settings = settings;
                })
            });
        });

        this.lock(|app| {
            app.main_dimmer
                .lock(|dimmer| dimmer.borrow_mut().new_settings_builder(callback))
        })
    }
}

fn initalize_dimmer(user_data: &UserData, dimmer_io: DimmerIO) -> DimmerHandle {
    // Initalize the dimmer driver
    let dimmer_io = dimmer_io;
    let dimmer_config = DimmerChannelConfig::new(
        60,
        dimmer_io.zero_cross.peripheral_input(),
        dimmer_io.gate.into_peripheral_output(),
        dimmer_io.mcpwm,
    )
    .with_firing_timing(DEFAULT_TIMING_CONFIG)
    .with_dimmer_settings(user_data.dimmer_settings)
    .with_starting_state(user_data.dimmer_state);

    let main_dimmer = DimmerChannel::new(dimmer_config);

    // Wrap main_dimmer result
    let main_dimmer = main_dimmer.expect("Failed to create main dimmer channel");
    Rc::new(NoopMutex::new(RefCell::new(main_dimmer)))
}

fn initalize_menu_controller(i2c: I2c<'static, Async>, app: AppHandle) -> MenuControllerHandle {
    ui::MenuController::new(i2c, app).expect("Failed to create menu controller")
}

fn initalize_rotery_decoder(
    spawner: Spawner,
    rotery_io: io::RoteryIO,
    menu_controller: &MenuControllerHandle,
) -> RoteryDecoder {
    let rotery_interface =
        ui::MenuController::create_rotery_interface(Rc::downgrade(menu_controller))
            .expect("Failed to create rotery interface");

    let rotery_config =
        RoteryDecoderConfig::new(rotery_io.clock, rotery_io.rotate, rotery_interface)
            .with_switch(rotery_io.switch);
    RoteryDecoder::new(spawner, rotery_config).expect("Failed to create rotery decoder")
}

pub(super) async fn app_main(spawner: Spawner, peripherals: io::AppCorePeripherals) {
    info!("App main task started!");

    let mut user_data_storage = user_data::initalize(peripherals.flash).await;
    let loaded_user_data = user_data_storage.read().await;

    let app = APP_CELL.init(Rc::new_cyclic(|app| {
        let main_dimmer = initalize_dimmer(&loaded_user_data, peripherals.dimmer_io);
        let menu_controller = initalize_menu_controller(peripherals.i2c, app.clone());
        let rotery_decoder =
            initalize_rotery_decoder(spawner, peripherals.rotery_io, &menu_controller);

        info!("App created!");
        NoopMutex::new(App {
            rotery_decoder,
            main_dimmer,
            menu_controller,
            user_data: Rc::new(SignalCell::new(loaded_user_data)),
        })
    }));

    app.borrow().finish_initialization().await;

    // Spawn the propagator task with static reference to the dimmer
    spawner.must_spawn(settings_propagator(app.clone(), user_data_storage));
}

/// Task that propagates user data changes to submodules
#[embassy_executor::task]
async fn settings_propagator(app: Rc<NoopMutex<App>>, mut user_data_storage: UserDataStorage) {
    let user_data = app.lock(|app| app.user_data.clone());

    loop {
        user_data.signal().wait().await; // Wait for any changes to user data
        user_data.signal().reset(); // Reset signal to wait for next change
        info!("User data changed, propagating settings...");

        app.lock(|app| {
            app.main_dimmer.lock(|dimmer| {
                dimmer
                    .borrow_mut()
                    .set_state(user_data.get(|user_data| user_data.dimmer_state));
            });
        });

        let updated_user_data = user_data.get(|user_data| user_data.clone());
        user_data_storage
            .write(&updated_user_data)
            .await
            .expect("Failed to write userdata!");
    }
}
