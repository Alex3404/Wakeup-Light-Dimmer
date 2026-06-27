use core::marker::PhantomData;

use esp_hal::{
    Async,
    gpio::interconnect::{InputSignal, PeripheralInput},
    i2c::master::I2c,
};

use crate::app::drivers::rt_clock::devices::Rtc;

pub mod alarm;
pub mod devices;
pub mod encoding;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum HourFormat {
    H24,
    H12,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum AmPm {
    AM,
    PM,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,

    pub hour: u8,
    pub am_pm: AmPm,
    pub hour_format: HourFormat,

    pub minute: u8,
    pub second: u8,
}

struct RealTimeClock<'d, Device>
where
    Device: Rtc,
{
    _phatom: PhantomData<Device>,
    i2c: I2c<'d, Async>,
    multi_function_pin: InputSignal<'d>,
}

impl<'d, Device> RealTimeClock<'d, Device>
where
    Device: Rtc,
{
    pub fn new(i2c: I2c<'d, Async>, mfp: impl PeripheralInput<'d>) -> Self {
        Self {
            _phatom: PhantomData,
            i2c,
            multi_function_pin: mfp.into(),
        }
    }

    pub fn initalize(&mut self) {
        // Set up the RTC registers for the desired timekeeping mode
    }
}
