#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use alloc::boxed::Box;
use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use esp_backtrace as _;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

use arduino_esp32_dimmer::{
    lamp_dimmer::{
        self, FireTimingConfig, GammaCorrection, LampDimmerChannel, LampDimmerChannelConfig,
        zero_cross_analyzer,
    },
    rotery_decoder::{Rotation, RoteryDecoder, RoteryDecoderConfig},
    ui,
};
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_rtos::embassy::Executor;
use static_cell::StaticCell;
use trouble_host::peripheral;

use core::{
    cell::RefCell,
    sync::atomic::{AtomicU8, Ordering},
};

use embassy_time::Timer;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    gpio::{
        Input, InputConfig, Level, Output, OutputConfig, Pull,
        interconnect::{PeripheralInput, PeripheralOutput},
    },
    handler,
    i2c::master::I2c,
    interrupt::software::SoftwareInterruptControl,
    mcpwm::{
        McPwm, PeripheralClockConfig, PwmPeripheral,
        capture::{CaptureEdge, CaptureMode},
    },
    peripherals::{GPIO, MCPWM0, Peripherals, RMT},
    rmt::Rmt,
    system::Stack,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};
use esp_hal::{i2c::master::AnyI2c, i2c::master::Config as I2CConfig};

extern crate alloc;
use log::{info, warn};
use ui::{DimmerInterface, UserInterface};

static BRIGHTNESS: AtomicU8 = AtomicU8::new(0);
fn rotated(_spawner: Spawner, rotation: Rotation) {
    let value = BRIGHTNESS.load(Ordering::Relaxed);
    let next_value = match rotation {
        Rotation::Clockwise => value.saturating_add(2).min(lamp_dimmer::MAX_BRIGHTNESS),
        Rotation::Counterclockwise => value.saturating_sub(2),
    };
    BRIGHTNESS.store(next_value, Ordering::Relaxed);
    info!("Brightness: {}", next_value);
}

struct RoteryIO<'d> {
    clock: Input<'d>,
    rotate: Input<'d>,
    switch: Input<'d>,
}

struct DimmerIO<'d> {
    zero_cross: Input<'d>,
    gate: Output<'d>,
}

// Drivers for second core to claim after initalization on main core
static IC2_DRIVER: Signal<CriticalSectionRawMutex, I2c<'static, Blocking>> = Signal::new();
// static MCPWM_DRIVER: Signal<CriticalSectionRawMutex, McPwm<'static, MCPWM0<'static>>> =
//     Signal::new();
static ROTERY_IO: Signal<CriticalSectionRawMutex, RoteryIO<'static>> = Signal::new();
static DIMMER_IO: Signal<CriticalSectionRawMutex, DimmerIO<'static>> = Signal::new();

#[embassy_executor::task]
async fn app_initalize(spawner: Spawner) {
    // Get rotery decoder IO
    let rotery_inputs = ROTERY_IO.wait().await;
    let (clock, rotate, switch) = (
        rotery_inputs.clock,
        rotery_inputs.rotate,
        rotery_inputs.switch,
    );

    // Initalize the rotery decoder
    let config = RoteryDecoderConfig::new(clock, rotate)
        .with_switch(switch)
        .with_rotate_handler(Box::new(rotated));

    let rotery_decoder = RoteryDecoder::create(spawner, config);
    let Ok(_rotery_decoder) = rotery_decoder else {
        panic!("Failed to create rotery decoder!");
    };
    info!("Created rotery decoder!");

    // Get our dimmer IO
    let dimmer_io = DIMMER_IO.wait().await;
    let (mut zero_cross, gate) = (dimmer_io.zero_cross, dimmer_io.gate);

    // Detect frequency of AC wave form
    info!("Detecting AC frequency");
    let hz = loop {
        let frequency_result =
            zero_cross_analyzer::determine_frequency::<10>(&mut zero_cross).await;
        let Ok(frequency) = frequency_result else {
            warn!("Unable to determine frequency! Is zero cross pin connected? Retrying in 5s");
            Timer::after_secs(5).await;
            continue;
        };

        let hz = frequency.as_hz();

        if hz < 40 || hz > 140 {
            // Unsupported frequencies
            continue;
        }

        break hz;
    };
    info!("Frequency of AC waveform: {}Hz", hz);

    // let fire_timing = FireTimingConfig::new()
    //     .with_latch_time_after_zero(5000)
    //     .with_latch_time_before_next_zero(500)
    //     .with_min_latch_time(150)
    //     .with_perceived_zero_brightness(51)
    //     .with_perceived_full_brightness(105)
    //     .with_gamma_correction(GammaCorrection::Exponetinal);

    // let dimmer_config =
    //     LampDimmerChannelConfig::new(hz as u8, zero_cross, gate, mcpwm)
    //         .with_firing_timing(fire_timing);

    // // Initalize our lamp dimmer
    // let main_dimmer = LampDimmerChannel::create(spawner, dimmer_config);
    // let Ok(main_dimmer) = main_dimmer else {
    //     panic!("Failed to create main dimmer!");
    // };
    // info!("Created main light dimmer channel!");

    // Wait for main core to create I2C driver
    let i2c = IC2_DRIVER.wait().await.into_async();
    let menu_result = UserInterface::create(i2c).await;
    let Ok(mut menu) = menu_result else {
        panic!("Failed to intialize menu")
    };

    // Spawn some tasks
    let _ = spawner;

    // let mut angle: f32 = 0.0;
    // let mut last_time = Instant::now();

    info!("App core main loop!");
    // let mut last_edge = CaptureEdge::Falling;
    // let mut last_time = 0;
    // loop {
    //     let event = capture.get_event();
    //     if last_edge != event.get_edge() {
    //         let delta = event.get_time().wrapping_sub(last_time) / 160;

    //         match event.get_edge() {
    //             CaptureEdge::Rising => {
    //                 println!("Spent low delta: {}us", delta)
    //             }
    //             CaptureEdge::Falling => println!("Spent high delta: {}us", delta),
    //         }
    //         last_edge = event.get_edge();
    //         last_time = event.get_time();
    //     }
    //     Timer::after_micros(1).await;
    // }

    // Simple
    // let speed_counter = SPEED.load(Ordering::Relaxed).clamp(1, 100);
    // let angular_speed = (speed_counter as f32 / 25.0) * 2.0 * PI;

    // let now = Instant::now();
    // let time_delta = now - last_time;
    // last_time = now;
    // let delta_secs = time_delta.as_micros() as f32 / 1_000_000.0;
    // angle = angle + angular_speed * delta_secs;

    // let brightness_f = (sinf(angle) + 1.0) / 2.0;
    // let brightness = (brightness_f * lamp_dimmer::MAX_BRIGHTNESS as f32) as u8;

    // let brightness = BRIGHTNESS.load(Ordering::Relaxed);
    // menu.update_brightness(brightness).await;

    // main_dimmer.lock(|dimmer| {
    //     dimmer.borrow_mut().set_brightness(brightness);
    // });
}

// This is executed on the second core
fn app_core(spawner: Spawner) -> () {
    // Run app initalize
    spawner.must_spawn(app_initalize(spawner));
}

static MCPWM: Mutex<RefCell<Option<McPwm<'static, MCPWM0>>>> = Mutex::new(RefCell::new(None));
#[handler]
fn mcpwm_interrupt() {
    critical_section::with(|cs| {
        let mut mcpwm = MCPWM.borrow_ref_mut(cs);
        let Some(ref mut mcpwm) = *mcpwm else {
            return;
        };

        if mcpwm.capture0.is_interupt_set() {
            let event = mcpwm.capture0.get_event();
            println!(
                "Capture: {:?}, {}",
                event.get_edge(),
                event.get_time() / 160
            );

            mcpwm.capture0.clear_interrupt();
        }
    })
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Initalize the heap allocator with 72000 bytes of ram
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72000);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    // Start RTOS on main core
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Create stack for app core
    static APP_CORE_STACK: StaticCell<Stack<8192>> = StaticCell::new();
    let app_core_stack = APP_CORE_STACK.init(Stack::new());
    static APP_EXECUTOR: StaticCell<Executor> = StaticCell::new();

    // Start RTOS on second core
    esp_rtos::start_second_core(
        peripherals.CPU_CTRL,
        sw_int.software_interrupt1,
        app_core_stack,
        move || {
            let executor = APP_EXECUTOR.init(Executor::new());
            executor.run(app_core);
        },
    );

    info!("Embassy initialized!");

    // Start up wifi and bluetooth controller
    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    let _connector = BleConnector::new(peripherals.BT, Default::default());

    // Create I2C driver give to app core
    let i2c = initalize_i2c_driver(
        AnyI2c::from(peripherals.I2C0),
        peripherals.GPIO11,
        peripherals.GPIO12,
    );
    IC2_DRIVER.signal(i2c);

    //MCPWM_DRIVER.signal(mcpwm);

    // Give app core rotery inputs
    let no_pullup = InputConfig::default().with_pull(Pull::None);
    let pullup = InputConfig::default().with_pull(Pull::Up);
    ROTERY_IO.signal(RoteryIO {
        clock: Input::new(peripherals.GPIO7, no_pullup),
        rotate: Input::new(peripherals.GPIO8, no_pullup),
        switch: Input::new(peripherals.GPIO9, pullup),
    });

    // Using Pull::None as Pull::Up changes the total resistance of the
    // zero cross detection circitry. Changining the timing for the zero cross pulse.
    // Since it adds the internal pull up resistance in parellel with the on board pull up resistor
    let zero_cross = Input::new(peripherals.GPIO6, no_pullup);
    // DIMMER_IO.signal(DimmerIO {
    //     zero_cross: Input::new(peripherals.GPIO6, no_pullup),
    //     gate: Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default()),
    // });

    // Create MCPWM driver give to app core
    println!("Mcpwm driver!!");
    let mut mcpwm = initlaize_mcpwm_driver(peripherals.MCPWM0);
    mcpwm.set_interrupt_handler(mcpwm_interrupt);
    println!("Capture start!!");
    mcpwm.capture_timer.start();
    mcpwm.capture0.set_signal_capture(zero_cross);
    mcpwm.capture0.listen(CaptureMode::AnyEdge);
    mcpwm.capture0.set_enable(true);
    println!("Capture listen!!");
    critical_section::with(|cs| MCPWM.replace(cs, Some(mcpwm)));

    loop {}
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

fn initlaize_mcpwm_driver<'d, PWM>(mcpwm: PWM) -> McPwm<'d, PWM>
where
    PWM: PwmPeripheral,
{
    // 1 microsecond clock
    let config_result = PeripheralClockConfig::with_frequency(Rate::from_mhz(1));
    let Ok(clock_config) = config_result else {
        let err = config_result.err().unwrap();
        panic!("Failed to create MCPWM driver! {:?}", err)
    };

    McPwm::new(mcpwm, clock_config)
}
