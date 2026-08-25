mod i2c;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::I2c;
use esp_hal::interrupt::software::{SoftwareInterrupt, SoftwareInterruptControl};
use esp_hal::peripherals::{MCPWM0, Peripherals};
use esp_hal::timer::AnyTimer;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::Async;
use esp_storage::FlashStorage;

/// Splits the raw ESP32 peripherals into RTOS and application-specific peripherals.
pub fn split(peripherals: Peripherals) -> (RtosPeripherals, AppPeripherals) {
    // Initialize the software interrupt control and timer group for the RTOS.
    let sw_int_control = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timer_group = TimerGroup::new(peripherals.TIMG0);

    let rtos_peripherals = RtosPeripherals {
        rtos_timer: timer_group.timer0.into(),
        sw_interrupt_0: sw_int_control.software_interrupt0,
    };

    // Initialize the input configurations for the application-specific peripherals.
    let no_pullup = InputConfig::default().with_pull(Pull::None);
    let pullup = InputConfig::default().with_pull(Pull::Up);

    // Application Pin mappings
    let zero_cross = peripherals.GPIO1;
    let gate = peripherals.GPIO0;
    let clock = peripherals.GPIO5;
    let rotate = peripherals.GPIO4;
    let switch = peripherals.GPIO6;
    let led = peripherals.GPIO2;

    // I2C
    let i2c_sda = peripherals.GPIO10;
    let i2c_scl = peripherals.GPIO11;

    let app_peripherals = AppPeripherals {
        dimmer: DimmerIO {
            zero_cross: Input::new(zero_cross, no_pullup),
            gate: Output::new(gate, Level::Low, OutputConfig::default()),
            mcpwm: peripherals.MCPWM0,
        },
        rotery: RoteryIO {
            clock: Input::new(clock, no_pullup),
            rotate: Input::new(rotate, no_pullup),
            switch: Input::new(switch, pullup),
        },
        flash: FlashStorage::new(peripherals.FLASH),
        bluetooth: peripherals.BT,
        wifi: peripherals.WIFI,
        i2c_device: i2c::init_async_i2c(peripherals.I2C0.into(), i2c_sda, i2c_scl),
        test_led: Output::new(led, Level::Low, OutputConfig::default()),
    };

    (rtos_peripherals, app_peripherals)
}

pub struct RtosPeripherals {
    pub rtos_timer: AnyTimer<'static>,
    pub sw_interrupt_0: SoftwareInterrupt<'static, 0>,
}

pub struct AppPeripherals {
    pub dimmer: DimmerIO,
    pub rotery: RoteryIO,
    pub i2c_device: I2cDevice<'static, NoopRawMutex, I2c<'static, Async>>,
    pub flash: FlashStorage<'static>,
    pub bluetooth: esp_hal::peripherals::BT<'static>,
    pub wifi: esp_hal::peripherals::WIFI<'static>,
    pub test_led: Output<'static>,
}

pub struct DimmerIO {
    pub zero_cross: Input<'static>,
    pub gate: Output<'static>,
    pub mcpwm: MCPWM0<'static>,
}

pub struct RoteryIO {
    pub clock: Input<'static>,
    pub rotate: Input<'static>,
    pub switch: Input<'static>,
}

impl AppPeripherals {
    pub fn new(peripherals: Peripherals) -> Self {
        let no_pullup = InputConfig::default().with_pull(Pull::None);
        let pullup = InputConfig::default().with_pull(Pull::Up);

        AppPeripherals {
            dimmer: DimmerIO {
                zero_cross: Input::new(peripherals.GPIO5, no_pullup),
                gate: Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default()),
                mcpwm: peripherals.MCPWM0,
            },
            rotery: RoteryIO {
                clock: Input::new(peripherals.GPIO11, no_pullup),
                rotate: Input::new(peripherals.GPIO12, no_pullup),
                switch: Input::new(peripherals.GPIO10, pullup),
            },
            flash: FlashStorage::new(peripherals.FLASH),
            bluetooth: peripherals.BT,
            wifi: peripherals.WIFI,
            i2c_device: i2c::init_async_i2c(peripherals.I2C0.into(), peripherals.GPIO6, peripherals.GPIO7),
            test_led: Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default()),
        }
    }
}
