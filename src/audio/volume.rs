use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Volume settings for a device (serializable for config)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VolumeSettings {
    pub gain_linear: f32, // Linear gain multiplier (0.001 to 2.0)
    pub gain_db: f32,     // Gain in dB (-60.0 to +6.0)
}

impl Default for VolumeSettings {
    fn default() -> Self {
        Self {
            gain_linear: 1.0, // Unity gain (0 dB)
            gain_db: 0.0,
        }
    }
}

impl VolumeSettings {
    /// Create a new VolumeSettings from dB value
    pub fn from_db(gain_db: f32) -> Self {
        let clamped_db = gain_db.clamp(-60.0, 6.0);
        Self {
            gain_linear: 10f32.powf(clamped_db / 20.0),
            gain_db: clamped_db,
        }
    }

    /// Create a new VolumeSettings from linear gain value
    pub fn from_linear(gain_linear: f32) -> Self {
        let clamped_linear = gain_linear.clamp(0.001, 2.0);
        Self {
            gain_linear: clamped_linear,
            gain_db: 20.0 * clamped_linear.log10(),
        }
    }

    /// Adjust gain by delta dB
    pub fn adjust_db(&mut self, delta_db: f32) {
        self.gain_db = (self.gain_db + delta_db).clamp(-60.0, 6.0);
        self.gain_linear = 10f32.powf(self.gain_db / 20.0);
    }

    /// Clamp settings to valid ranges
    pub fn clamp(&mut self) {
        self.gain_db = self.gain_db.clamp(-60.0, 6.0);
        self.gain_linear = self.gain_linear.clamp(0.001, 2.0);
    }
}

/// Real-time volume processor (lives in JACK callback)
pub struct VolumeProcessor {
    settings: VolumeSettings,
    needs_update: Arc<AtomicBool>,
    pending_settings: Arc<Mutex<Option<VolumeSettings>>>,
}

impl VolumeProcessor {
    /// Create a new volume processor with the given settings
    pub fn new(settings: VolumeSettings) -> Self {
        Self {
            settings,
            needs_update: Arc::new(AtomicBool::new(false)),
            pending_settings: Arc::new(Mutex::new(None)),
        }
    }

    /// Process a stereo sample through the volume control
    /// This is the main real-time processing function - must be allocation-free
    #[inline]
    pub fn process_sample(&mut self, left: f32, right: f32) -> (f32, f32) {
        // Check for pending updates (atomic read - very fast)
        if self.needs_update.load(Ordering::Relaxed) {
            self.apply_pending_update();
        }

        // Apply gain (simple multiplication)
        (left * self.settings.gain_linear, right * self.settings.gain_linear)
    }

    /// Apply pending settings update if available (non-blocking)
    fn apply_pending_update(&mut self) {
        // Use try_lock to avoid blocking the real-time thread
        if let Ok(mut pending) = self.pending_settings.try_lock() {
            if let Some(new_settings) = pending.take() {
                self.settings = new_settings;
                self.needs_update.store(false, Ordering::Relaxed);
            }
        }
        // If lock fails, we'll try again on the next buffer - no big deal
    }

    /// Get handles for updating settings from another thread
    pub fn get_update_handles(&self) -> (Arc<AtomicBool>, Arc<Mutex<Option<VolumeSettings>>>) {
        (
            Arc::clone(&self.needs_update),
            Arc::clone(&self.pending_settings),
        )
    }

    /// Get current settings
    pub fn settings(&self) -> &VolumeSettings {
        &self.settings
    }
}

/// Helper function to schedule a settings update from another thread
pub fn update_volume_settings(
    needs_update: &Arc<AtomicBool>,
    pending_settings: &Arc<Mutex<Option<VolumeSettings>>>,
    new_settings: VolumeSettings,
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
    fn test_volume_settings_default() {
        let settings = VolumeSettings::default();
        assert_eq!(settings.gain_db, 0.0);
        assert_eq!(settings.gain_linear, 1.0);
    }

    #[test]
    fn test_volume_settings_from_db() {
        let settings = VolumeSettings::from_db(6.0);
        assert_eq!(settings.gain_db, 6.0);
        assert!((settings.gain_linear - 2.0).abs() < 0.01);

        let settings = VolumeSettings::from_db(-6.0);
        assert_eq!(settings.gain_db, -6.0);
        assert!((settings.gain_linear - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_volume_settings_from_db_clamping() {
        let settings = VolumeSettings::from_db(100.0);
        assert_eq!(settings.gain_db, 6.0); // Clamped to max

        let settings = VolumeSettings::from_db(-100.0);
        assert_eq!(settings.gain_db, -60.0); // Clamped to min
    }

    #[test]
    fn test_volume_settings_adjust_db() {
        let mut settings = VolumeSettings::default();
        settings.adjust_db(3.0);
        assert_eq!(settings.gain_db, 3.0);
        assert!((settings.gain_linear - 1.412).abs() < 0.01);

        settings.adjust_db(-6.0);
        assert_eq!(settings.gain_db, -3.0);
        assert!((settings.gain_linear - 0.707).abs() < 0.01);
    }

    #[test]
    fn test_volume_settings_clamp() {
        let mut settings = VolumeSettings {
            gain_db: 100.0,
            gain_linear: 100.0,
        };
        settings.clamp();
        assert_eq!(settings.gain_db, 6.0);
        assert_eq!(settings.gain_linear, 2.0);
    }

    #[test]
    fn test_volume_processor_unity_gain() {
        let mut processor = VolumeProcessor::new(VolumeSettings::default());
        let (l_out, r_out) = processor.process_sample(0.5, -0.3);
        assert_eq!(l_out, 0.5);
        assert_eq!(r_out, -0.3);
    }

    #[test]
    fn test_volume_processor_gain() {
        let settings = VolumeSettings::from_db(6.0); // ~2.0x gain
        let mut processor = VolumeProcessor::new(settings);
        let (l_out, r_out) = processor.process_sample(0.5, -0.3);
        assert!((l_out - 1.0).abs() < 0.01);
        assert!((r_out + 0.6).abs() < 0.01);
    }

    #[test]
    fn test_volume_processor_update_mechanism() {
        let processor = VolumeProcessor::new(VolumeSettings::default());
        let (flag, pending) = processor.get_update_handles();

        // Initially, no update pending
        assert!(!flag.load(Ordering::Relaxed));

        // Schedule an update
        let new_settings = VolumeSettings::from_db(3.0);
        update_volume_settings(&flag, &pending, new_settings);

        // Update should be pending
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_settings_serialization() {
        let settings = VolumeSettings::from_db(3.0);
        let serialized = toml::to_string(&settings).unwrap();
        let deserialized: VolumeSettings = toml::from_str(&serialized).unwrap();
        assert_eq!(settings, deserialized);
    }
}
