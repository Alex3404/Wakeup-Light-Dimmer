use esp_hal::{Async, i2c::master::I2c};

use crate::app::drivers::rt_clock::DateTime;
pub mod mcp7940n;

pub trait Rtc {
    type Error;

    async fn read_datetime(&mut self) -> Result<DateTime, Self::Error>;
    async fn write_datetime(&mut self, dt: &DateTime) -> Result<(), Self::Error>;
    async fn initalize(&mut self);
}
