use core::cell::RefCell;

use embassy_embedded_hal::shared_bus::blocking::i2c::I2cDevice;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::gpio::interconnect::{PeripheralInput, PeripheralOutput};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::I2c;
use esp_hal::interrupt::software::{SoftwareInterrupt, SoftwareInterruptControl};
use esp_hal::peripherals::{MCPWM0, Peripherals};
use esp_hal::time::Rate;
use esp_hal::timer::AnyTimer;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Async, Blocking};
use esp_hal::{i2c::master::AnyI2c, i2c::master::Config as I2CConfig};
use esp_storage::FlashStorage;
use static_cell::StaticCell;

pub(super) struct DimmerIO {
    pub zero_cross: Input<'static>,
    pub gate: Output<'static>,
    pub mcpwm: MCPWM0<'static>,
}

pub(super) struct RoteryIO {
    pub clock: Input<'static>,
    pub rotate: Input<'static>,
    pub switch: Input<'static>,
}

pub(super) struct AppPeripherals {
    pub dimmer_io: DimmerIO,
    pub rotery_io: RoteryIO,
    pub i2c_device: I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, Blocking>>,
    pub flash: FlashStorage<'static>,
    pub rtos_timer: AnyTimer<'static>,
    pub bluetooth: esp_hal::peripherals::BT<'static>,
    pub wifi: esp_hal::peripherals::WIFI<'static>,
    pub sw_interrupt_0: SoftwareInterrupt<'static, 0>,
}

impl AppPeripherals {
    pub fn new(peripherals: Peripherals) -> Self {
        let no_pullup = InputConfig::default().with_pull(Pull::None);
        let pullup = InputConfig::default().with_pull(Pull::Up);

        static I2C_BUS: StaticCell<
            Mutex<CriticalSectionRawMutex, RefCell<I2c<'static, Blocking>>>,
        > = StaticCell::new();

        let i2c = I2C_BUS.init_with(|| {
            Mutex::new(RefCell::new(initalize_i2c_driver(
                AnyI2c::from(peripherals.I2C0),
                peripherals.GPIO0,
                peripherals.GPIO1,
            )))
        });
        let i2c_device = I2cDevice::new(i2c);

        let sw_int_control = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        AppPeripherals {
            dimmer_io: DimmerIO {
                zero_cross: Input::new(peripherals.GPIO21, no_pullup),
                gate: Output::new(peripherals.GPIO20, Level::Low, OutputConfig::default()),
                mcpwm: peripherals.MCPWM0,
            },
            rotery_io: RoteryIO {
                clock: Input::new(peripherals.GPIO3, no_pullup),
                rotate: Input::new(peripherals.GPIO4, no_pullup),
                switch: Input::new(peripherals.GPIO5, pullup),
            },
            sw_interrupt_0: sw_int_control.software_interrupt0,
            flash: FlashStorage::new(peripherals.FLASH),
            rtos_timer: TimerGroup::new(peripherals.TIMG0).timer0.into(),
            bluetooth: peripherals.BT,
            wifi: peripherals.WIFI,
            i2c_device,
        }
    }
}

/// Create a new i2c driver with our configurations
fn initalize_i2c_driver<'d, SDAIO, SCLIO>(
    i2c: AnyI2c<'d>,
    sda: SDAIO,
    scl: SCLIO,
) -> I2c<'d, Blocking>
where
    SDAIO: PeripheralInput<'d> + PeripheralOutput<'d>,
    SCLIO: PeripheralInput<'d> + PeripheralOutput<'d>,
{
    let config = I2CConfig::default().with_frequency(Rate::from_khz(400));
    let i2c_result = I2c::new(i2c, config);
    let Ok(i2c) = i2c_result else {
        panic!("Unable to initlaize i2c peripheral")
    };

    i2c.with_scl(scl).with_sda(sda)
}
