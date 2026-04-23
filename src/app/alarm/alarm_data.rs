pub struct AlarmData {
    pub time_after_midnight: Duration,
    pub is_enabled: bool,
    pub sunrise_data: SunriseData,
}

pub struct AlarmSchedule {
    pub alarms: [Option<AlarmData>; 7], // One for each day of the week
}
