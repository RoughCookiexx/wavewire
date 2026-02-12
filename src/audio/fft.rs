use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;

/// FFT processor for converting audio samples to frequency spectrum
pub struct FftProcessor {
    /// FFT size (number of samples to process)
    fft_size: usize,
    /// Number of output bins for display
    num_bins: usize,
    /// Sample rate of the audio source
    sample_rate: u32,
    /// FFT planner for creating FFT instances
    planner: FftPlanner<f32>,
    /// Scratch buffer for FFT input (reused across calls)
    fft_input: Vec<Complex<f32>>,
    /// Window function (Hann window) applied before FFT
    window: Vec<f32>,
    /// Frequency ranges for logarithmic binning
    bin_edges: Vec<f32>,
}

impl FftProcessor {
    /// Create a new FFT processor
    ///
    /// # Arguments
    /// * `fft_size` - Size of the FFT (power of 2, typically 2048)
    /// * `num_bins` - Number of output frequency bins for display (typically 64-128)
    /// * `sample_rate` - Sample rate of the audio source (Hz)
    pub fn new(fft_size: usize, num_bins: usize, sample_rate: u32) -> Self {
        // Generate Hann window
        let window = Self::generate_hann_window(fft_size);

        // Generate logarithmic bin edges
        let bin_edges = Self::generate_log_bin_edges(num_bins, sample_rate);

        Self {
            fft_size,
            num_bins,
            sample_rate,
            planner: FftPlanner::new(),
            fft_input: vec![Complex::new(0.0, 0.0); fft_size],
            window,
            bin_edges,
        }
    }

    /// Generate a Hann window function
    ///
    /// The Hann window reduces spectral leakage by smoothly tapering the signal
    /// to zero at the edges of the window.
    fn generate_hann_window(size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| {
                let phase = 2.0 * PI * i as f32 / (size - 1) as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect()
    }

    /// Generate logarithmic bin edges for frequency grouping
    ///
    /// Human hearing is logarithmic, so we use more bins for low frequencies
    /// and fewer bins for high frequencies.
    ///
    /// Frequency range: 18 Hz to 20 kHz (extended low-frequency range)
    fn generate_log_bin_edges(num_bins: usize, sample_rate: u32) -> Vec<f32> {
        const MIN_FREQ: f32 = 18.0; // 18 Hz (10% lower than standard 20 Hz)
        let max_freq = (sample_rate as f32 / 2.0).min(20000.0); // Nyquist or 20 kHz

        let log_min = MIN_FREQ.ln();
        let log_max = max_freq.ln();
        let log_step = (log_max - log_min) / num_bins as f32;

        (0..=num_bins)
            .map(|i| (log_min + i as f32 * log_step).exp())
            .collect()
    }

    /// Process audio samples and return frequency spectrum
    ///
    /// # Arguments
    /// * `samples` - Audio samples to process (must be at least `fft_size` samples)
    ///
    /// # Returns
    /// A tuple of (bin_magnitudes, bin_frequencies):
    /// - bin_magnitudes: Magnitude of each frequency bin in dB
    /// - bin_frequencies: Center frequency of each bin in Hz
    pub fn process(&mut self, samples: &[f32]) -> (Vec<f32>, Vec<f32>) {
        if samples.len() < self.fft_size {
            // Not enough samples, return empty result
            return (vec![0.0; self.num_bins], self.bin_centers());
        }

        // Take the most recent fft_size samples
        let start_idx = samples.len() - self.fft_size;
        let samples_slice = &samples[start_idx..];

        // Apply window function and convert to complex
        for (i, &sample) in samples_slice.iter().enumerate() {
            let windowed = sample * self.window[i];
            self.fft_input[i] = Complex::new(windowed, 0.0);
        }

        // Perform FFT
        let fft = self.planner.plan_fft_forward(self.fft_size);
        fft.process(&mut self.fft_input);

        // Convert FFT output to magnitudes
        let magnitudes: Vec<f32> = self.fft_input
            .iter()
            .take(self.fft_size / 2) // Only use positive frequencies
            .map(|c| {
                // Calculate magnitude: sqrt(re^2 + im^2)
                let mag = c.norm();
                // Normalize by FFT size
                let normalized = mag / self.fft_size as f32;
                // Convert to dB (with floor to avoid log(0))
                let db = 20.0 * (normalized.max(1e-10)).log10();
                // Clamp to reasonable range
                db.max(-60.0).min(0.0)
            })
            .collect();

        // Group into logarithmic bins
        let binned_magnitudes = self.bin_magnitudes(&magnitudes);
        let bin_frequencies = self.bin_centers();

        (binned_magnitudes, bin_frequencies)
    }

    /// Group FFT magnitudes into logarithmic frequency bins
    fn bin_magnitudes(&self, magnitudes: &[f32]) -> Vec<f32> {
        let freq_per_bin = self.sample_rate as f32 / self.fft_size as f32;

        let mut binned = Vec::with_capacity(self.num_bins);

        for i in 0..self.num_bins {
            let freq_start = self.bin_edges[i];
            let freq_end = self.bin_edges[i + 1];

            // Find FFT bins that fall in this frequency range
            let bin_start = (freq_start / freq_per_bin).floor() as usize;
            let bin_end = (freq_end / freq_per_bin).ceil() as usize;

            // Average the magnitudes in this range
            let count = (bin_end - bin_start).max(1);
            let sum: f32 = magnitudes
                .iter()
                .skip(bin_start)
                .take(count)
                .sum();
            let avg = sum / count as f32;

            binned.push(avg);
        }

        binned
    }

    /// Get the center frequency for each bin
    fn bin_centers(&self) -> Vec<f32> {
        (0..self.num_bins)
            .map(|i| {
                let freq_start = self.bin_edges[i];
                let freq_end = self.bin_edges[i + 1];
                // Geometric mean for logarithmic scale
                (freq_start * freq_end).sqrt()
            })
            .collect()
    }

    /// Get the FFT size
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_window() {
        let window = FftProcessor::generate_hann_window(8);
        assert_eq!(window.len(), 8);
        // First and last values should be close to 0
        assert!(window[0] < 0.01);
        assert!(window[7] < 0.01);
        // Middle value should be close to 1
        assert!((window[4] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_log_bin_edges() {
        let edges = FftProcessor::generate_log_bin_edges(10, 48000);
        assert_eq!(edges.len(), 11); // num_bins + 1
        // First edge should be close to 18 Hz
        assert!((edges[0] - 18.0).abs() < 1.0);
        // Edges should be increasing
        for i in 1..edges.len() {
            assert!(edges[i] > edges[i - 1]);
        }
    }

    #[test]
    fn test_fft_processor_creation() {
        let processor = FftProcessor::new(2048, 64, 48000);
        assert_eq!(processor.fft_size(), 2048);
    }

    #[test]
    fn test_process_sine_wave() {
        let mut processor = FftProcessor::new(2048, 64, 48000);

        // Generate a 440 Hz sine wave (A4 note)
        let sample_rate = 48000.0;
        let frequency = 440.0;
        let samples: Vec<f32> = (0..2048)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * PI * frequency * t).sin()
            })
            .collect();

        let (magnitudes, frequencies) = processor.process(&samples);

        assert_eq!(magnitudes.len(), 64);
        assert_eq!(frequencies.len(), 64);

        // Find the bin with maximum magnitude
        let max_idx = magnitudes
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(idx, _)| idx)
            .unwrap();

        // The peak should be near 440 Hz
        let peak_freq = frequencies[max_idx];
        assert!(
            (peak_freq - 440.0).abs() < 100.0,
            "Expected peak near 440 Hz, got {} Hz",
            peak_freq
        );
    }
}
