extern crate alloc;
use alloc::rc::Rc;
use alloc::boxed::Box;
use embassy_executor::Spawner;
use embedded_graphics::{draw_target::DrawTarget, geometry::{Point, Size}, pixelcolor::{BinaryColor}, primitives::Rectangle};
use slint::{ComponentHandle, PlatformError, platform::SetPlatformError};
use core::ops::Mul;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::Async;
use ssd1306::{Ssd1306Async, mode::{BufferedGraphicsModeAsync, DisplayConfigAsync}, prelude::I2CInterface, rotation::DisplayRotation, size::{DisplaySize128x64, DisplaySizeAsync}};
use display_interface::DisplayError;

use crate::app::{core::AppStateReceiver};

pub type I2c =
    I2cDevice<'static, NoopRawMutex, esp_hal::i2c::master::I2c<'static, Async>>;
pub type Interface = I2CInterface<I2c>;
pub type DisplaySize = DisplaySize128x64;
pub type Display = Ssd1306Async<Interface, DisplaySize, BufferedGraphicsModeAsync<DisplaySize>>;

type SlintBuffer = [slint::Rgb8Pixel; DisplaySize::HEIGHT as usize * DisplaySize::WIDTH as usize ];

pub struct DimmerUI {
    display : Display,
    line_buffer : SlintBuffer,
    window : Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
    spawner: Spawner,
}

#[derive(Debug)]
pub enum UIError {
    DisplayInitError(DisplayError),
    SetPlatformInitError(SetPlatformError),
    PlatformInitError(PlatformError),
    DisplayFlushError(DisplayError),
}


impl DimmerUI {
    pub async fn new(spawner: Spawner, i2c: I2c, app_state: AppStateReceiver) -> Result<Self, UIError> {
        let interface = ssd1306::I2CDisplayInterface::new(i2c);
        let mut display = Ssd1306Async::new(
            interface,
            DisplaySize128x64,
            DisplayRotation::Rotate0,
        ).into_buffered_graphics_mode();

        if let Err(e) = display.init().await {
            return Err(UIError::DisplayInitError(e));
        }

        // Create buffer
        let buffer : SlintBuffer = [slint::Rgb8Pixel::default(); DisplaySize::HEIGHT as usize * DisplaySize::WIDTH as usize];

        // Create the Slint software window
        let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
        );

        // Initalize the Slint platform with the EspBackend
        let set_platform_result = slint::platform::set_platform(Box::new(EspBackend {
            window: window.clone(),
        }));

        match set_platform_result {
            Ok(_) => { Ok(()) },
            Err(e) => {
                match e {
                    SetPlatformError::AlreadySet => { Ok(()) },
                    _ => {
                        return Err(UIError::SetPlatformInitError(e));
                    }
                }
            }
        }?;


        let size = slint::PhysicalSize::new(DisplaySize::WIDTH as u32, DisplaySize::HEIGHT as u32);
        window.set_size(size);

        let main_window = super::HelloWorld::new().map_err(|e| UIError::PlatformInitError(e))?;
        main_window.show().map_err(|e| UIError::PlatformInitError(e))?;

        spawner.spawn(react_to_app_state(app_state, main_window).unwrap());

        Ok(Self {
            display: display,
            line_buffer: buffer,
            window,
            spawner,
        })
    }

    pub async fn run(mut self) -> Result<(), UIError> {
        loop {
            // Update timers and animations for the Slint platform.
            slint::platform::update_timers_and_animations();
            
            // Render the window if needed.
            self.window.draw_async_if_needed(async |renderer : &i_slint_renderer_software::SoftwareRenderer| {
                renderer.render_by_line(DisplayWrapper {
                    display: &mut self.display,
                    line_buffer: &mut self.line_buffer,
                });

                self.display.flush().await.map_err(|e| UIError::DisplayFlushError(e));
            }).await;

            // If we have no active animations, wait until the next timer update
            if !self.window.has_active_animations()
                && let Some(duration) = slint::platform::duration_until_next_timer_update() {
                Timer::after(Duration::from_millis(duration.as_millis() as u64)).await;
                continue;
            }
            
            // Wait a short period to maintain a roughly 24 FPS update rate.
            Timer::after(Duration::from_millis(1000 / 24)).await;
        }
    }
}

#[embassy_executor::task]
async fn react_to_app_state(mut app_state : AppStateReceiver, window : super::HelloWorld) {
    loop {
        let new = app_state.changed().await;
        window.set_brightness(Into::<f32>::into(new.brightness.0).mul(100.0));
    }
}

/// The backend for the Slint platform on the ESP board.
pub struct EspBackend {
    window: Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}

impl slint::platform::Platform for EspBackend {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(Instant::now().as_millis())
    }
}

/// A wrapper around the display and the Slint line buffer, providing a line buffer for the software renderer.
struct DisplayWrapper<'a> {
    display: &'a mut Display,
    line_buffer: &'a mut SlintBuffer,
}

impl<'a> slint::platform::software_renderer::LineBufferProvider for DisplayWrapper<'a>
{
    type TargetPixel = slint::Rgb8Pixel;
    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        // Render into the line
        render_fn(&mut self.line_buffer[range.clone()]);
        
        self.display
            .fill_contiguous(
                &Rectangle::new(
                    Point::new(range.start as _, line as _),
                    Size::new(range.len() as _, 1),
                ),
                self.line_buffer[range.clone()]
                    .iter()
                    .map(|p| match p.r != 0 || p.g != 0 || p.b != 0 {
                        true => BinaryColor::On,
                        false => BinaryColor::Off,
                    }),
            )
            .map_err(drop)
            .unwrap();
    }
}