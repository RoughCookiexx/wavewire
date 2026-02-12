use biquad::{Biquad, Coefficients, DirectForm2Transposed, Hertz, Type};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Standard 10-band graphic EQ frequencies (ISO standard)
pub const GRAPHIC_EQ_BANDS: [f32; 10] = [
    31.0, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Parameters for a single EQ band (serializable for config)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqBandParams {
    pub frequency: f32,  // Center frequency (Hz)
    pub gain_db: f32,    // Gain in dB (-12.0 to +12.0)
    pub q_value: f32,    // Q factor (0.5 to 5.0, default 1.41)
}

impl Default for EqBandParams {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            gain_db: 0.0,
            q_value: 1.41,
        }
    }
}

impl EqBandParams {
    /// Create a new EQ band with the given parameters
    pub fn new(frequency: f32, gain_db: f32, q_value: f32) -> Self {
        Self {
            frequency,
            gain_db: gain_db.clamp(-12.0, 12.0),
            q_value: q_value.clamp(0.5, 5.0),
        }
    }

    /// Clamp parameters to valid ranges
    pub fn clamp(&mut self) {
        self.gain_db = self.gain_db.clamp(-12.0, 12.0);
        self.q_value = self.q_value.clamp(0.5, 5.0);
        self.frequency = self.frequency.clamp(20.0, 20000.0);
    }
}

/// Complete EQ settings for a device (serializable)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqSettings {
    pub bands: [EqBandParams; 10],
    pub bypass: bool,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            bands: GRAPHIC_EQ_BANDS.map(|freq| EqBandParams {
                frequency: freq,
                gain_db: 0.0,
                q_value: 1.41,
            }),
            bypass: false,
        }
    }
}

impl EqSettings {
    /// Create a flat EQ (all gains at 0 dB)
    pub fn flat() -> Self {
        Self::default()
    }

    /// Reset all bands to 0 dB gain
    pub fn reset(&mut self) {
        for band in &mut self.bands {
            band.gain_db = 0.0;
        }
    }

    /// Set a specific band's parameters
    pub fn set_band(&mut self, index: usize, gain_db: f32, q_value: f32) {
        if index < 10 {
            self.bands[index].gain_db = gain_db.clamp(-12.0, 12.0);
            self.bands[index].q_value = q_value.clamp(0.5, 5.0);
        }
    }
}

/// Real-time EQ processor (lives in JACK callback)
pub struct EqProcessor {
    filters: [DirectForm2Transposed<f32>; 10],
    settings: EqSettings,
    sample_rate: f32,
    needs_update: Arc<AtomicBool>,
    pending_settings: Arc<Mutex<Option<EqSettings>>>,
}

impl EqProcessor {
    /// Create a new EQ processor with the given sample rate and settings
    pub fn new(sample_rate: f32, settings: EqSettings) -> Self {
        let filters = Self::create_filters(sample_rate, &settings);
        Self {
            filters,
            settings,
            sample_rate,
            needs_update: Arc::new(AtomicBool::new(false)),
            pending_settings: Arc::new(Mutex::new(None)),
        }
    }

    /// Create biquad filters from EQ settings
    fn create_filters(sr: f32, settings: &EqSettings) -> [DirectForm2Transposed<f32>; 10] {
        settings.bands.clone().map(|band| {
            let coeffs = Coefficients::<f32>::from_params(
                Type::PeakingEQ(band.gain_db),
                Hertz::<f32>::from_hz(sr).unwrap(),
                Hertz::<f32>::from_hz(band.frequency).unwrap(),
                band.q_value,
            )
            .unwrap();
            DirectForm2Transposed::<f32>::new(coeffs)
        })
    }

    /// Process a stereo sample through the EQ
    /// This is the main real-time processing function - must be allocation-free
    #[inline]
    pub fn process_sample(&mut self, left: f32, right: f32) -> (f32, f32) {
        // Check for pending updates (atomic read - very fast)
        if self.needs_update.load(Ordering::Relaxed) {
            self.apply_pending_update();
        }

        // Bypass if enabled
        if self.settings.bypass {
            return (left, right);
        }

        // Cascade through all filters
        let mut l = left;
        let mut r = right;
        for filter in &mut self.filters {
            l = filter.run(l);
            r = filter.run(r);
        }

        (l, r)
    }

    /// Apply pending settings update if available (non-blocking)
    fn apply_pending_update(&mut self) {
        // Use try_lock to avoid blocking the real-time thread
        if let Ok(mut pending) = self.pending_settings.try_lock() {
            if let Some(new_settings) = pending.take() {
                self.settings = new_settings.clone();
                self.filters = Self::create_filters(self.sample_rate, &new_settings);
                self.needs_update.store(false, Ordering::Relaxed);
            }
        }
        // If lock fails, we'll try again on the next buffer - no big deal
    }

    /// Get handles for updating settings from another thread
    pub fn get_update_handles(&self) -> (Arc<AtomicBool>, Arc<Mutex<Option<EqSettings>>>) {
        (
            Arc::clone(&self.needs_update),
            Arc::clone(&self.pending_settings),
        )
    }

    /// Update sample rate (called when sample rate changes)
    pub fn update_sample_rate(&mut self, new_sample_rate: f32) {
        if (self.sample_rate - new_sample_rate).abs() > 0.1 {
            self.sample_rate = new_sample_rate;
            self.filters = Self::create_filters(new_sample_rate, &self.settings);
        }
    }

    /// Get current settings
    pub fn settings(&self) -> &EqSettings {
        &self.settings
    }

    /// Get current sample rate
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

/// Helper function to schedule a settings update from another thread
pub fn update_eq_settings(
    needs_update: &Arc<AtomicBool>,
    pending_settings: &Arc<Mutex<Option<EqSettings>>>,
    new_settings: EqSettings,
) {
    if let Ok(mut pending) = pending_settings.lock() {
        *pending = Some(new_settings);
        needs_update.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graphic_eq_bands_count() {
        assert_eq!(GRAPHIC_EQ_BANDS.len(), 10);
    }

    #[test]
    fn test_eq_band_params_default() {
        let params = EqBandParams::default();
        assert_eq!(params.frequency, 1000.0);
        assert_eq!(params.gain_db, 0.0);
        assert_eq!(params.q_value, 1.41);
    }

    #[test]
    fn test_eq_band_params_clamping() {
        let mut params = EqBandParams::new(1000.0, 15.0, 10.0);
        assert_eq!(params.gain_db, 12.0); // Clamped to max
        assert_eq!(params.q_value, 5.0); // Clamped to max

        params = EqBandParams::new(1000.0, -20.0, 0.1);
        assert_eq!(params.gain_db, -12.0); // Clamped to min
        assert_eq!(params.q_value, 0.5); // Clamped to min
    }

    #[test]
    fn test_eq_settings_default() {
        let settings = EqSettings::default();
        assert_eq!(settings.bands.len(), 10);
        assert!(!settings.bypass);

        // Check that bands match graphic EQ frequencies
        for (i, band) in settings.bands.iter().enumerate() {
            assert_eq!(band.frequency, GRAPHIC_EQ_BANDS[i]);
            assert_eq!(band.gain_db, 0.0);
            assert_eq!(band.q_value, 1.41);
        }
    }

    #[test]
    fn test_eq_settings_flat() {
        let settings = EqSettings::flat();
        for band in &settings.bands {
            assert_eq!(band.gain_db, 0.0);
        }
    }

    #[test]
    fn test_eq_settings_reset() {
        let mut settings = EqSettings::default();
        settings.bands[0].gain_db = 6.0;
        settings.bands[5].gain_db = -3.0;

        settings.reset();

        for band in &settings.bands {
            assert_eq!(band.gain_db, 0.0);
        }
    }

    #[test]
    fn test_eq_settings_set_band() {
        let mut settings = EqSettings::default();
        settings.set_band(5, 3.0, 2.0);

        assert_eq!(settings.bands[5].gain_db, 3.0);
        assert_eq!(settings.bands[5].q_value, 2.0);

        // Test clamping
        settings.set_band(5, 20.0, 10.0);
        assert_eq!(settings.bands[5].gain_db, 12.0);
        assert_eq!(settings.bands[5].q_value, 5.0);
    }

    #[test]
    fn test_eq_processor_bypass() {
        let mut settings = EqSettings::default();
        settings.bypass = true;
        let mut processor = EqProcessor::new(48000.0, settings);

        let (l_out, r_out) = processor.process_sample(0.5, -0.3);
        assert_eq!(l_out, 0.5);
        assert_eq!(r_out, -0.3);
    }

    #[test]
    fn test_eq_processor_flat_unity() {
        // Flat EQ (all gains 0 dB) should pass audio through with minimal change
        let settings = EqSettings::flat();
        let mut processor = EqProcessor::new(48000.0, settings);

        let (l_out, r_out) = processor.process_sample(0.5, -0.3);

        // Allow small numerical error from filter processing
        assert!((l_out - 0.5).abs() < 0.001);
        assert!((r_out + 0.3).abs() < 0.001);
    }

    #[test]
    fn test_eq_processor_update_mechanism() {
        let processor = EqProcessor::new(48000.0, EqSettings::default());
        let (flag, pending) = processor.get_update_handles();

        // Initially, no update pending
        assert!(!flag.load(Ordering::Relaxed));

        // Schedule an update
        let mut new_settings = EqSettings::default();
        new_settings.bands[5].gain_db = 6.0;
        update_eq_settings(&flag, &pending, new_settings);

        // Update should be pending
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_eq_processor_sample_rate_update() {
        let mut processor = EqProcessor::new(48000.0, EqSettings::default());
        assert_eq!(processor.sample_rate(), 48000.0);

        processor.update_sample_rate(44100.0);
        assert_eq!(processor.sample_rate(), 44100.0);

        // Small changes should be ignored
        processor.update_sample_rate(44100.05);
        assert_eq!(processor.sample_rate(), 44100.0);
    }

    #[test]
    fn test_eq_band_params_clamp() {
        let mut params = EqBandParams {
            frequency: 50000.0,
            gain_db: 100.0,
            q_value: 100.0,
        };
        params.clamp();

        assert_eq!(params.frequency, 20000.0);
        assert_eq!(params.gain_db, 12.0);
        assert_eq!(params.q_value, 5.0);
    }

    #[test]
    fn test_settings_serialization() {
        let settings = EqSettings::default();
        let serialized = toml::to_string(&settings).unwrap();
        let deserialized: EqSettings = toml::from_str(&serialized).unwrap();
        assert_eq!(settings, deserialized);
    }
}
