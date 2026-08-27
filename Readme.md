## Light Dimming on ESP32

A custom designed board using the ESP32 SoC, with an AC Dimming module. Before I get into the details I'd like to cover some of the backstory of this project. During my 2026 semester at my universtity, I had very early classes, and since it was till in the winter the sun doesn't come up by the time I need to wake up. I was reading some information online that waking up to a light is more natural way to wake up since it would memic the typical sun rise/sun set that humans evolved in. So I put that to the test, I had this smart light from Eufy and I programmed it to turn on 20-30minutes before I had to wake up. I saw that I woke up less grogy and more energietic in the mornings. However the biggest con with this approch was that there was no fade option. No way to graudually increase the light only pulse effects. So, I decided to fix this problem myself. I saw that I had an over head light in my room, and a light switch that could be dimmable. Why not combine a sunrise alarm clock with a light switch dimmer? So I got to work. This is project is the result of that question.

## Fully Async Rust Firmware

I landed on the ESP32 for a few main reasons: its direct support for writing in Rust supporting Embassy, its diverse set of peripherals, and that it has WiFI & Bluetooth Module. This repo has the Rust project in its root directory, I've split a few different operations into packages. A application library for unit testing my code, a binary that loads the appliction library on the ESP32, a supporting async storage-derive for the async app state storage driver using the ESP32 NVS partition, and a crate sequental storage built for writing in the format the ESP32 expects.

### Slint UI

My project uses the Slint UI framework for compiling the UI files into the native executable.

## Future Direction for this Project

Currently I'm still working on this project as a solo adventure. If you have issues with it or find any bugs open an issue! I'll get to it. Right now I am still working on the PCB design, Ive gone through 4 iterations now on the PCB, with the last two not working at all for trivial reasons. Once I didn't impedence match the USB data traces to 90 ohms, and another time I shorted the VCC and Ground layers ( Facepalm ). I'm working on PCB design that should work, and will incorperate all my plans.

* Real Time Clock
* SPI Interface for TFT Display
* ESP32
* MOSFET AC Dimming for LCD bulbs
* LiPo battery backup for RTC with intergrated battery charger

I'm also working on the firmware currently I am striving towards getting WIFI and bluethooth to work, then after that I'd love to focus on an Phone application inorder to configure alarm times, and other options exposed though either WIFI or Bluetooth. I'm leaning towards Bluetooth due to WiFi insecurities relating to IoT devices. Supporting only bluetooth should reduce the attack vector.

## Video Showcase
[![Showcase Video](https://img.youtube.com/vi/jobSFEIKs34/0.jpg)](https://www.youtube.com/watch?v=jobSFEIKs34)
