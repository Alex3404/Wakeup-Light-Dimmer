use crate::app::lamp_dimmer::{
    DimmerState, DimmerSettings, GammaCorrection, MAX_BRIGHTNESS, MIN_BRIGHTNESS,
    dimmer_channel::DriverHandle,
};

use embassy_executor::SendSpawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use embassy_time::{Duration, Instant, Timer};

extern crate alloc;
use alloc::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    FullBrightness,
    ZeroBrightness,
    GammaCorrection,
}

/// Builder for configuring dimmer settings with real-time preview
pub struct DimmerSettingsBuilder {
    pending_settings: DimmerSettings,
    publish_signal: Arc<Signal<CriticalSectionRawMutex, DimmerSettings>>,

    preview_mode: Option<PreviewMode>,
    gamma_preview_task: Option<Arc<GammaPreviewTask>>,

    dropped: Arc<Signal<CriticalSectionRawMutex, ()>>,
    handle: DriverHandle,
}

struct GammaPreviewTask {
    stop_signal: Signal<CriticalSectionRawMutex, ()>,
    handle: DriverHandle,
    min_brightness: u8,
    max_brightness: u8,
}

#[embassy_executor::task]
async fn animate_gamma_preview(preview_task: Arc<GammaPreviewTask>) {
    const FRAME_INTERVAL: Duration = Duration::from_millis(50); // 20 FPS
    let brightness_range = preview_task.max_brightness as i16 - preview_task.max_brightness as i16;
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
        preview_task.handle.lock(|d| {
            d.borrow_mut().update_state(DimmerState {
                brightness,
                is_on: true,
            });
        });

        Timer::after(FRAME_INTERVAL).await;
    }
}

impl DimmerSettingsBuilder {
    /// Create a new builder from current settings
    pub(super) fn new(
        publish_signal: Arc<Signal<CriticalSectionRawMutex, DimmerSettings>>,
        handle: DriverHandle,
        settings: DimmerSettings,
        dropped: Arc<Signal<CriticalSectionRawMutex, ()>>,
    ) -> Self {
        Self {
            pending_settings: settings.clone(),
            publish_signal,
            preview_mode: None,
            gamma_preview_task: None,
            handle,
            dropped,
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
            self.update_brightness(brightness);
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
            self.update_brightness(brightness);
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
    }

    /// Call callback with pending settings and exit builder
    pub fn publish(mut self) {
        self.stop_gamma_preview();
        self.publish_signal.signal(self.pending_settings);
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
                self.update_brightness(self.pending_settings.perceived_full_brightness);
            }
            PreviewMode::ZeroBrightness => {
                self.update_brightness(self.pending_settings.perceived_zero_brightness);
            }
            PreviewMode::GammaCorrection => {
                // For gamma preview, start the animation
                self.start_gamma_preview().await;
            }
        };
    }

    fn update_brightness(&mut self, brightness: u8) {
        critical_section::with(|cs| {
            let mut handle = self.handle.borrow(cs).borrow_mut();
            handle.update_state(DimmerState {
                brightness,
                is_on: true,
            });
        });
    }

    /// Update the dimmers config based on the current preview mode
    fn configure_preview(&mut self) {
        let config = match self.preview_mode {
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

        if let Some(config) = config {
            critical_section::with(|cs| {
                let mut handle = self.handle.borrow(cs).borrow_mut();
                handle.update_settings(config);
            });
        }
    }

    /// Start the gamma preview animation
    async fn start_gamma_preview(&mut self) {
        // Start gamma animation
        let stop_signal = Signal::new();

        let preview_task = Arc::new(GammaPreviewTask {
            stop_signal,
            handle: self.handle.clone(),
            min_brightness: self.pending_settings.perceived_zero_brightness,
            max_brightness: self.pending_settings.perceived_full_brightness,
        });

        let spawner = SendSpawner::for_current_executor().await;

        let token = animate_gamma_preview(preview_task.clone());
        let _ = spawner.spawn(token.unwrap());
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
        self.dropped.signal(());
    }
}
