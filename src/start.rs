use crate::lamp_dimmer::LampDimmer;
use crate::pcnt_handler;

use core::f32::consts::PI;

use esp_hal::{
    Blocking,
    clock::CpuClock,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    pcnt::Pcnt,
    peripherals::Peripherals,
    rmt::Rmt,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};

extern crate alloc;
use libm::sinf;
use log::info;

pub fn initalize_rmt(peripherals: &Peripherals) -> Rmt<'static, Blocking> {
    let peripheral_rmt = unsafe { peripherals.RMT.clone_unchecked() };

    let freq = Rate::from_mhz(80);
    let rmt = Rmt::new(peripheral_rmt, freq);

    let Ok(rmt) = rmt else {
        panic!("Failed to create rmt");
    };

    rmt
}

pub fn initalize_pcnt(peripherals: &Peripherals) -> Pcnt<'static> {
    let peripheral_pcnt = unsafe { peripherals.PCNT.clone_unchecked() };
    let mut pcnt = Pcnt::new(peripheral_pcnt);

    // Initalize the pnct handler to support more then 1 unit using
    // the interrupt handler
    pcnt_handler::initalize(&mut pcnt);
    pcnt
}

pub fn get_lamp_dimmer_io(peripherals: &Peripherals) -> (Input<'static>, Output<'static>) {
    let signal_pin = unsafe { peripherals.GPIO7.clone_unchecked() };
    let gate_pin = unsafe { peripherals.GPIO8.clone_unchecked() };

    let zero_cross_config = InputConfig::default().with_pull(Pull::None);
    let zero_cross_input = Input::new(signal_pin, zero_cross_config);

    let triac_gate_config = OutputConfig::default();
    let triac_gate_output = Output::new(gate_pin, Level::Low, triac_gate_config);

    (zero_cross_input, triac_gate_output)
}

pub fn main_loop() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Initalize the heap allocator with 72000 bytes of ram
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72000);

    // Initalize peripherals
    let pcnt = initalize_pcnt(&peripherals);
    let rmt = initalize_rmt(&peripherals);
    let (zero_cross_input, triac_gate_output) = get_lamp_dimmer_io(&peripherals);
    let timer_group_0 = TimerGroup::new(peripherals.TIMG0);

    info!("Initalizing lamp dimmer!");
    let lamp_dimmer = LampDimmer::initalize(
        pcnt.unit0,
        zero_cross_input,
        triac_gate_output,
        rmt.channel0,
    );
    let Ok(lamp_dimmer) = lamp_dimmer else {
        panic!("Unable to create lamp dimmer");
    };

    info!("Start rtos");
    let rtos_timer = timer_group_0.timer0;
    esp_rtos::start(rtos_timer);

    info!("Main loop started!");
    loop {
        // Simple
        const BREATHING_TIME_MS: f32 = 15000.0;
        let milis = Instant::now().duration_since_epoch().as_millis();
        let angle = ((milis % (BREATHING_TIME_MS) as u64) as f32 / BREATHING_TIME_MS) * 2.0 * PI;
        let brightness = (sinf(angle) + 1.0) / 2.0;

        critical_section::with(|cs| {
            lamp_dimmer
                .borrow_ref_mut(cs)
                .set_brightness((brightness * 100.0) as u8);
        });
        esp_hal::delay::Delay::new().delay_micros(25);
    }
}
