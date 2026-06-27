use crate::app::drivers::rt_clock::{AmPm, DateTime, HourFormat, devices::Rtc};
mod rtc_registers;
use rtc_registers::*;

use bilge::prelude::*;

const RTC_I2C_ADDRESS: u8 = 0x6F;

enum Registers {
    RTCSEC = 0x00,
    RTCMIN = 0x01,
    RTCHOUR = 0x02,
    RTCWEEKDAY = 0x03,
    RTCDATE = 0x04,
    RTCMONTH = 0x05,
    RTYYEAR = 0x06,
    CONTROL = 0x07,
    OSCTRIM = 0x08,
}

use esp_hal::{Async, i2c::master::I2c};

pub struct MCP7940N {
    i2c: I2c<'static, Async>,
}

impl Rtc for MCP7940N {
    type Error = ();

    async fn write_datetime(&mut self, dt: &DateTime) -> Result<(), Self::Error> {
        let buffer = [
            Registers::RTCSEC as u8,
            u8::from(RtcSecond::from(*dt)),
            u8::from(RtcMinute::from(*dt)),
            u8::from(RtcHour::from(*dt)),
            u8::from(RtcWeekday::from(*dt)),
            u8::from(RtcDate::from(*dt)),
            u8::from(RtcMonth::from(*dt)),
            u8::from(RtcYear::from(*dt)),
        ];

        self.i2c
            .write_async(RTC_I2C_ADDRESS, &buffer)
            .await
            .map_err(|_| ())?;

        Ok(())
    }

    async fn read_datetime(&mut self) -> Result<DateTime, Self::Error> {
        let mut buffer = [0u8; 7];
        self.i2c
            .write_async(RTC_I2C_ADDRESS, &[Registers::RTCSEC as u8])
            .await
            .map_err(|_| ())?;
        self.i2c
            .read_async(RTC_I2C_ADDRESS, &mut buffer)
            .await
            .map_err(|_| ())?;

        let second = RtcSecond::from(buffer[0]);
        let minute = RtcMinute::from(buffer[1]);
        let hour = RtcHour::from(UInt::<u8, 8>::new(buffer[2]));
        let weekday = RtcWeekday::from(buffer[3]);
        let date = RtcDate::from(buffer[4]);
        let month = RtcMonth::from(buffer[5]);
        let year = RtcYear::from(buffer[6]);

        Ok(DateTime {
            second: second.seconds(),
            minute: minute.minutes(),
            hour: match hour {
                RtcHour::H24(rtc24hour) => u8::from(rtc24hour.hours()),
                RtcHour::H12(rtc12hour) => u8::from(rtc12hour.hours()),
            },
            am_pm: match hour {
                RtcHour::H24(_) => AmPm::AM, // 24-hour format doesn't have AM/PM
                RtcHour::H12(rtc12hour) => {
                    if rtc12hour.is_pm() {
                        AmPm::PM
                    } else {
                        AmPm::AM
                    }
                }
            },
            hour_format: match hour {
                RtcHour::H24(_) => HourFormat::H24,
                RtcHour::H12(_) => HourFormat::H12,
            },
            day: date.day(),
            month: month.month(),
            weekday: u8::from(weekday.weekday()),
            year: year.year() as u16 + 2000, // Assuming the RTC stores years as an offset from 2000
        })
    }

    async fn initalize(&mut self) {
        todo!()
    }
}
