use crate::app::drivers::rt_clock::{AmPm, DateTime, HourFormat};
use bilge::prelude::*;

const RTC_12HOUR_BIT: u8 = 1 << 6;
const RTC_PM_BIT: u8 = 1 << 5;

// Seconds from 0 to 59
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcSecond {
    pub ones: u4,
    pub tens: u3,
    pub start_oscillator: bool,
}

// Minutes from 0 to 59
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcMinute {
    pub ones: u4,
    pub tens: u3,
    _reserved: bool,
}

#[bitsize(6)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct Rtc24Hour {
    pub minutes: u4,
    pub hours: u2,
}

#[bitsize(6)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct Rtc12Hour {
    pub minutes: u4,
    pub hours: u1,
    pub is_pm: bool,
}

// Hours in 12 or 24 hour format
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RtcHour {
    H24(Rtc24Hour),
    H12(Rtc12Hour),
}

// Weekday from 1 to 7
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcWeekday {
    pub weekday: u3,
    pub vbat_enable: bool,
    pub power_failure: bool,
    pub oscillator_status: bool,
    _reserved: u2,
}

// Day from 1 to 31
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcDate {
    pub ones: u4,
    pub tens: u2,
    _reserved: u2,
}

// Month from 1 to 12
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcMonth {
    pub ones: u4,
    pub tens: u1,
    pub leap_year: bool,
    _reserved: u2,
}

// Year from xx00 to xx99, so we can just store the last two digits.
#[bitsize(8)]
#[derive(Clone, Copy, PartialEq, Eq, FromBits)]
pub struct RtcYear {
    pub ones: u4,
    pub tens: u4,
}

impl RtcSecond {
    pub fn create(seconds: u8, start_oscillator: bool) -> Self {
        RtcSecond::new(
            UInt::<u8, 4>::new(seconds % 10),
            UInt::<u8, 3>::new(seconds / 10),
            start_oscillator,
        )
    }

    pub fn seconds(&self) -> u8 {
        u8::from(self.ones()) + u8::from(self.tens()) * 10
    }
}

impl From<DateTime> for RtcSecond {
    fn from(dt: DateTime) -> Self {
        RtcSecond::create(dt.second, false)
    }
}

impl RtcMinute {
    pub fn create(minutes: u8) -> Self {
        RtcMinute::new(
            UInt::<u8, 4>::new(minutes % 10),
            UInt::<u8, 3>::new(minutes / 10),
        )
    }

    pub fn minutes(&self) -> u8 {
        u8::from(self.ones()) + u8::from(self.tens()) * 10
    }
}

impl From<DateTime> for RtcMinute {
    fn from(dt: DateTime) -> Self {
        RtcMinute::create(dt.minute)
    }
}

impl RtcMonth {
    pub fn create(month: u8, year: u16) -> Self {
        RtcMonth::new(
            UInt::<u8, 4>::new(month % 10),
            UInt::<u8, 1>::new(month / 10),
            year % 4 == 0,
        )
    }

    pub fn month(&self) -> u8 {
        u8::from(self.ones()) + u8::from(self.tens()) * 10
    }
}

impl From<DateTime> for RtcMonth {
    fn from(dt: DateTime) -> Self {
        RtcMonth::create(dt.month as u8, dt.year)
    }
}

impl RtcDate {
    pub fn create(day: u8) -> Self {
        RtcDate::new(UInt::<u8, 4>::new(day % 10), UInt::<u8, 2>::new(day / 10))
    }

    pub fn day(&self) -> u8 {
        u8::from(self.ones()) + u8::from(self.tens()) * 10
    }
}

impl From<DateTime> for RtcDate {
    fn from(dt: DateTime) -> Self {
        RtcDate::create(dt.day)
    }
}

impl RtcWeekday {
    pub fn create(
        weekday: u8,
        vbat_enable: bool,
        power_failure: bool,
        oscillator_status: bool,
    ) -> Self {
        RtcWeekday::new(
            UInt::<u8, 3>::new(weekday % 8),
            vbat_enable,
            power_failure,
            oscillator_status,
        )
    }
}

impl From<DateTime> for RtcWeekday {
    fn from(dt: DateTime) -> Self {
        RtcWeekday::create(dt.weekday, false, false, false)
    }
}

impl RtcYear {
    pub fn create(year: u16) -> Self {
        let year = year % 100;
        RtcYear::new(
            UInt::<u8, 4>::new(year as u8 % 10),
            UInt::<u8, 4>::new(year as u8 / 10),
        )
    }

    pub fn year(&self) -> u8 {
        u8::from(self.ones()) + u8::from(self.tens()) * 10
    }
}

impl From<DateTime> for RtcYear {
    fn from(dt: DateTime) -> Self {
        RtcYear::create(dt.year)
    }
}

impl From<DateTime> for RtcHour {
    fn from(dt: DateTime) -> Self {
        match dt.hour_format {
            HourFormat::H24 => RtcHour::H24(Rtc24Hour::new(
                UInt::<u8, 4>::new(dt.minute),
                UInt::<u8, 2>::new(dt.hour / 10),
            )),
            HourFormat::H12 => RtcHour::H12(Rtc12Hour::new(
                UInt::<u8, 4>::new(dt.minute),
                UInt::<u8, 1>::new(dt.hour % 12),
                dt.am_pm == AmPm::PM,
            )),
        }
    }
}

impl From<UInt<u8, 8>> for RtcHour {
    fn from(value: UInt<u8, 8>) -> Self {
        let is_12h = (value.value() & RTC_12HOUR_BIT) != 0;
        let minutes = UInt::<u8, 4>::new(value.value() & 0x0F);

        if is_12h {
            let hours = UInt::<u8, 1>::new((value.value() >> 4) & 0x01);
            let am_pm = if (value.value() & RTC_PM_BIT) != 0 {
                AmPm::PM
            } else {
                AmPm::AM
            };

            RtcHour::H12(Rtc12Hour::new(minutes, hours, am_pm == AmPm::PM))
        } else {
            let hours = UInt::<u8, 2>::new((value.value() >> 4) & 0x03);
            RtcHour::H24(Rtc24Hour::new(minutes, hours))
        }
    }
}

impl From<RtcHour> for u8 {
    fn from(value: RtcHour) -> Self {
        match value {
            RtcHour::H24(rtc24hour) => rtc24hour.value.into(),
            RtcHour::H12(rtc12hour) => u8::from(rtc12hour.value) | RTC_12HOUR_BIT,
        }
    }
}
