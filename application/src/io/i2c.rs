use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use esp_hal::Async;
use esp_hal::gpio::interconnect::PeripheralInput;
use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::i2c::master::I2c;
use esp_hal::i2c::master::AnyI2c;
use esp_hal::i2c::master::Config as I2CConfig;
use esp_hal::time::Rate;
use static_cell::StaticCell;

/// Create a new I2C shared bus device
pub fn init_async_i2c<'d, SDAIO, SCLIO>(
    i2c: AnyI2c<'d>,
    sda: SDAIO,
    scl: SCLIO,
) -> I2cDevice<'static, NoopRawMutex, I2c<'d, Async>>
where
    SDAIO: PeripheralInput<'d> + PeripheralOutput<'d>,
    SCLIO: PeripheralInput<'d> + PeripheralOutput<'d>,
{
    let config = I2CConfig::default().with_frequency(Rate::from_khz(400));
    let i2c_result = I2c::new(i2c, config);
    
    let Ok(i2c) = i2c_result else {
        panic!("Unable to initialize i2c peripheral")
    };

    let i2c = i2c.with_scl(scl).with_sda(sda).into_async();
    static I2C_BUS: StaticCell<Mutex<NoopRawMutex, I2c<'static, Async>>> = StaticCell::new();
    let i2c = I2C_BUS.init_with(|| {
        Mutex::new(i2c)
    });

    I2cDevice::new(i2c)
}
