use embedded_graphics::{
    mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};

use display_interface::AsyncWriteOnlyDataCommand;
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async,
    mode::BufferedGraphicsModeAsync,
    prelude::{DisplayRotation, I2CInterface},
    size::DisplaySize128x64,
};
use ssd1306::{mode::DisplayConfigAsync, size::DisplaySizeAsync};

extern crate alloc;
use alloc::format;

#[expect(async_fn_in_trait)]
pub trait DimmerInterface {
    async fn update_brightness(&mut self, brightness: u8);
}

#[expect(async_fn_in_trait)]
pub trait ControlInterface {
    async fn enter_settings(&mut self);
    async fn move_up(&mut self);
    async fn move_down(&mut self);
}

pub struct UserInterface<Interface, Size>
where
    Size: DisplaySizeAsync,
    Interface: AsyncWriteOnlyDataCommand,
{
    display: Ssd1306Async<Interface, Size, BufferedGraphicsModeAsync<Size>>,
    _menu_text_style: MonoTextStyle<'static, BinaryColor>,
    large_text_style: MonoTextStyle<'static, BinaryColor>,
    brightness: u8,
}

impl<Interface, Size> DimmerInterface for UserInterface<Interface, Size>
where
    Size: DisplaySizeAsync,
    Interface: AsyncWriteOnlyDataCommand,
{
    async fn update_brightness(&mut self, brightness: u8) {
        let brightness_changed = self.brightness != brightness;
        self.brightness = brightness;

        if brightness_changed {
            let brightness_text = format!("{}%", brightness);

            self.display.clear_buffer();

            let center = Point::new((Size::WIDTH / 2) as i32, (Size::HEIGHT / 2) as i32);
            Text::with_alignment(
                brightness_text.as_str(),
                center,
                self.large_text_style,
                Alignment::Center,
            )
            .draw(&mut self.display)
            .unwrap();

            let _r = self.display.flush().await;
        }
    }
}

impl<Driver> UserInterface<I2CInterface<Driver>, DisplaySize128x64>
where
    Driver: embedded_hal_async::i2c::I2c,
    I2CInterface<Driver>: AsyncWriteOnlyDataCommand,
{
    pub async fn create(i2c: Driver) -> Result<Self, ()> {
        let interface = I2CDisplayInterface::new(i2c);

        let mut display = Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();

        let menu_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        let large_text_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(BinaryColor::On)
            .build();

        display.init().await.map_err(|_err| ())?;

        Ok(Self {
            display,
            _menu_text_style: menu_text_style,
            large_text_style,
            brightness: 0,
        })
    }
}
