use crate::app::lamp_dimmer::{
    DimmerChannel, DimmerSettings, DimmerState, GammaCorrection, MAX_BRIGHTNESS, MIN_BRIGHTNESS,
};

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::signal::Signal;

use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    FullBrightness,
    ZeroBrightness,
    GammaCorrection,
}

/// Builder for configuring dimmer settings with real-time preview
pub struct DimmerSettingsBuilder {
    channel: &'static DimmerChannel,
    stop_gamma_signal: &'static Signal<NoopRawMutex, ()>,
    spawner: Spawner,

    pending_settings: DimmerSettings,
    preview_mode: Option<PreviewMode>,
    gamma_preview_task: Option<GammaPreviewTask>,
}

#[derive(Clone)]
struct GammaPreviewTask {
    channel: &'static DimmerChannel,
    stop_signal: &'static Signal<NoopRawMutex, ()>,
    min_brightness: u8,
    max_brightness: u8,
}

#[embassy_executor::task]
async fn animate_gamma_preview(preview_task: GammaPreviewTask) {
    const FRAME_INTERVAL: Duration = Duration::from_millis(50); // 20 FPS
    let brightness_range = preview_task.max_brightness as i16 - preview_task.min_brightness as i16;
    let start_time = Instant::now();
    static PERIOD: Duration = Duration::from_secs(5); // 5 second full cycle

    while !preview_task.stop_signal.signaled() {
        let elapsed = start_time.elapsed();
        let phase = elapsed.as_millis() % PERIOD.as_millis();

        // Sawtooth wave: 0->1 linear rise, then 1->0 linear fall
        let normalized = if phase < PERIOD.as_millis() / 2 {
            // First half: ramp up from 0 to 1
            phase.saturating_sub(2)
        } else {
            // Second half: ramp down from 1 to 0
            (PERIOD.as_millis() / 2)
                .saturating_sub(phase)
                .saturating_mul(2)
        };

        let brightness = preview_task
            .min_brightness
            .saturating_add(
                (normalized
                    .saturating_mul(brightness_range as u64)
                    .div_ceil(PERIOD.as_millis())) as u8,
            )
            .clamp(preview_task.min_brightness, preview_task.max_brightness);

        // Update dimmer
        preview_task.channel.update_state(DimmerState {
            brightness: brightness,
            is_on: true,
        });

        Timer::after(FRAME_INTERVAL).await;
    }
}

impl DimmerSettingsBuilder {
    /// Create a new builder from current settings
    pub(super) fn new(channel: &'static DimmerChannel, spawner: Spawner) -> Self {
        static STOP_GAMMA_SIGNAL: StaticCell<Signal<NoopRawMutex, ()>> = StaticCell::new();
        STOP_GAMMA_SIGNAL.uninit();
        let stop_gamma_signal = STOP_GAMMA_SIGNAL.init(Signal::new());

        Self {
            channel,
            spawner,
            pending_settings: channel.get_settings(),
            preview_mode: None,
            gamma_preview_task: None,
            stop_gamma_signal,
        }
    }

    /// Set the full perceived brightness during preview
    pub async fn set_full_brightness(&mut self, brightness: u8) {
        let brightness = brightness.clamp(
            self.pending_settings
                .perceived_zero_brightness
                .saturating_add(1)
                .min(MAX_BRIGHTNESS),
            MAX_BRIGHTNESS,
        );

        let changed = self.pending_settings.perceived_full_brightness != brightness;
        self.pending_settings.perceived_full_brightness = brightness;

        if changed
            && self
                .preview_mode
                .is_some_and(|mode| mode == PreviewMode::FullBrightness)
        {
            self.channel.set_brightness(brightness);
        }
    }

    /// Set the zero perceived brightness during preview
    pub async fn set_zero_brightness(&mut self, brightness: u8) {
        let brightness = brightness.clamp(
            MIN_BRIGHTNESS,
            self.pending_settings
                .perceived_full_brightness
                .saturating_sub(1)
                .max(MIN_BRIGHTNESS),
        );

        let changed = self.pending_settings.perceived_full_brightness != brightness;
        self.pending_settings.perceived_zero_brightness = brightness;

        if changed
            && self
                .preview_mode
                .is_some_and(|mode| mode == PreviewMode::ZeroBrightness)
        {
            self.channel.set_brightness(brightness);
        }
    }

    /// Set gamma correction and update preview if active
    pub async fn set_gamma_correction(&mut self, gamma: GammaCorrection) {
        let changed = self.pending_settings.gamma_correction != gamma;
        self.pending_settings.gamma_correction = gamma;

        if changed
            && self
                .preview_mode
                .is_some_and(|mode| mode == PreviewMode::GammaCorrection)
        {
            self.configure_preview();
        }
    }

    /// Switch preview mode (triggers animation for GAMMA mode)
    pub async fn set_preview_mode(&mut self, new_mode: PreviewMode) {
        if self.preview_mode != Some(new_mode) {
            self.update_preview_mode(new_mode).await;
        }
    }

    /// Exit builder without publishing
    pub fn cancel(mut self) {
        self.stop_gamma_preview();
        self.channel.builder_cancelled();
    }

    /// Call callback with pending settings and exit builder
    pub fn publish(mut self) {
        self.stop_gamma_preview();
        self.channel.builder_published(self.pending_settings);
    }

    /// Get the pending settings (for display/validation)
    pub fn get_pending_settings(&self) -> DimmerSettings {
        self.pending_settings
    }

    /// Update the dimmer based on current preview mode
    async fn update_preview_mode(&mut self, new_mode: PreviewMode) {
        if self.preview_mode == Some(PreviewMode::GammaCorrection) {
            self.stop_gamma_preview();
        }

        self.preview_mode = Some(new_mode);
        self.configure_preview(); // Configure dimmer for new preview mode

        match new_mode {
            PreviewMode::FullBrightness => {
                self.channel
                    .set_brightness(self.pending_settings.perceived_full_brightness);
            }
            PreviewMode::ZeroBrightness => {
                self.channel
                    .set_brightness(self.pending_settings.perceived_zero_brightness);
            }
            PreviewMode::GammaCorrection => {
                // For gamma preview, start the animation
                self.start_gamma_preview().await;
            }
        };
    }

    /// Update the dimmers config based on the current preview mode
    fn configure_preview(&mut self) {
        let preview_settings = match self.preview_mode {
            Some(PreviewMode::FullBrightness) | Some(PreviewMode::ZeroBrightness) => {
                Some(DimmerSettings {
                    perceived_zero_brightness: MIN_BRIGHTNESS,
                    perceived_full_brightness: MAX_BRIGHTNESS,
                    gamma_correction: GammaCorrection::Linear,
                })
            }
            Some(PreviewMode::GammaCorrection) => Some(self.pending_settings),
            None => None,
        };
        if let Some(settings) = preview_settings {
            self.channel.update_settings(&settings);
        }
    }

    /// Start the gamma preview animation
    async fn start_gamma_preview(&mut self) {
        let preview_task = GammaPreviewTask {
            stop_signal: self.stop_gamma_signal,
            channel: self.channel,
            min_brightness: self.pending_settings.perceived_zero_brightness,
            max_brightness: self.pending_settings.perceived_full_brightness,
        };

        let token = animate_gamma_preview(preview_task.clone());
        let _ = self.spawner.spawn(token.unwrap());
        self.gamma_preview_task = Some(preview_task);
    }

    /// Stop the gamma preview animation
    fn stop_gamma_preview(&mut self) {
        // Stop any existing animation
        self.gamma_preview_task.take().and_then(|task| {
            task.stop_signal.signal(());
            Some(())
        });
    }
}

impl Drop for DimmerSettingsBuilder {
    fn drop(&mut self) {
        // Ensure any running preview is stopped when builder is dropped
        self.stop_gamma_preview();
        self.channel.builder_cancelled();
    }
}
