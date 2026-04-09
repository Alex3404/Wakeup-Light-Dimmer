use esp_hal::Async;
use esp_hal::gpio::interconnect::{PeripheralInput, PeripheralOutput};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::I2c;
use esp_hal::interrupt::software::{SoftwareInterrupt, SoftwareInterruptControl};
use esp_hal::peripherals::{MCPWM0, Peripherals};
use esp_hal::time::Rate;
use esp_hal::timer::AnyTimer;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{i2c::master::AnyI2c, i2c::master::Config as I2CConfig};
use esp_storage::FlashStorage;

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

pub(super) struct AppCorePeripherals {
    pub dimmer_io: DimmerIO,
    pub rotery_io: RoteryIO,
    pub i2c: I2c<'static, Async>,
    pub flash: FlashStorage<'static>,
}

unsafe impl Send for AppCorePeripherals {}

pub(super) struct MainCorePeripherals {
    pub rtos_timer: AnyTimer<'static>,
    pub sw_interrupt_0: SoftwareInterrupt<'static, 0>,
    pub sw_interrupt_1: SoftwareInterrupt<'static, 1>,
    pub cpu_control: esp_hal::peripherals::CPU_CTRL<'static>,
    pub bluetooth: esp_hal::peripherals::BT<'static>,
    pub wifi: esp_hal::peripherals::WIFI<'static>,
}

pub(super) fn split_peripherals(
    peripherals: Peripherals,
) -> (AppCorePeripherals, MainCorePeripherals) {
    let no_pullup = InputConfig::default().with_pull(Pull::None);
    let pullup = InputConfig::default().with_pull(Pull::Up);

    let i2c = initalize_i2c_driver(
        AnyI2c::from(peripherals.I2C0),
        peripherals.GPIO11,
        peripherals.GPIO12,
    );

    let app_core = AppCorePeripherals {
        dimmer_io: DimmerIO {
            zero_cross: Input::new(peripherals.GPIO6, no_pullup),
            gate: Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default()),
            mcpwm: peripherals.MCPWM0,
        },
        rotery_io: RoteryIO {
            clock: Input::new(peripherals.GPIO7, no_pullup),
            rotate: Input::new(peripherals.GPIO8, no_pullup),
            switch: Input::new(peripherals.GPIO9, pullup),
        },
        i2c,
        flash: FlashStorage::new(peripherals.FLASH),
    };

    let timer_group_0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int_control = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    let main_core = MainCorePeripherals {
        rtos_timer: timer_group_0.timer0.into(),
        sw_interrupt_0: sw_int_control.software_interrupt0,
        sw_interrupt_1: sw_int_control.software_interrupt1,
        cpu_control: peripherals.CPU_CTRL,
        bluetooth: peripherals.BT,
        wifi: peripherals.WIFI,
    };

    (app_core, main_core)
}

/// Create a new i2c driver with our configurations
fn initalize_i2c_driver<'d, SDAIO, SCLIO>(i2c: AnyI2c<'d>, sda: SDAIO, scl: SCLIO) -> I2c<'d, Async>
where
    SDAIO: PeripheralInput<'d> + PeripheralOutput<'d>,
    SCLIO: PeripheralInput<'d> + PeripheralOutput<'d>,
{
    let config = I2CConfig::default().with_frequency(Rate::from_khz(400));
    let i2c_result = I2c::new(i2c, config);
    let Ok(i2c) = i2c_result else {
        panic!("Unable to initlaize i2c peripheral")
    };

    i2c.with_scl(scl).with_sda(sda).into_async()
}
