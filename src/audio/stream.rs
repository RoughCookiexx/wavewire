use anyhow::Result;
use crossbeam_channel::Sender;
use jack::{AudioIn, Client, Port};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::fft::FftProcessor;
use super::types::{AudioEvent, DeviceId, PortId, SpectrumData};

/// Ring buffer for audio samples
/// Stores incoming audio samples in a circular buffer for FFT processing
pub struct RingBuffer {
    /// Internal buffer storage
    buffer: VecDeque<f32>,
    /// Maximum capacity of the buffer
    capacity: usize,
}

impl RingBuffer {
    /// Create a new ring buffer with the specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push samples into the buffer
    /// If capacity is exceeded, oldest samples are dropped
    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            if self.buffer.len() >= self.capacity {
                self.buffer.pop_front();
            }
            self.buffer.push_back(sample);
        }
    }

    /// Get the current number of samples in the buffer
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if the buffer has enough samples for processing
    pub fn has_enough_samples(&self, required: usize) -> bool {
        self.buffer.len() >= required
    }

    /// Get a slice of samples without removing them
    /// Returns the most recent `count` samples
    pub fn peek(&self, count: usize) -> Vec<f32> {
        let available = self.buffer.len().min(count);
        let start_index = self.buffer.len().saturating_sub(available);
        self.buffer.iter().skip(start_index).copied().collect()
    }
}

/// JACK audio processor for handling process callbacks
struct JackProcessor {
    /// Left channel input port
    in_left: Port<AudioIn>,
    /// Right channel input port
    in_right: Port<AudioIn>,
    /// Ring buffer for storing samples (shared with main thread)
    sample_buffer: Arc<Mutex<RingBuffer>>,
}

impl jack::ProcessHandler for JackProcessor {
    fn process(&mut self, _client: &jack::Client, ps: &jack::ProcessScope) -> jack::Control {
        use std::cell::Cell;
        thread_local! {
            static PROCESS_COUNT: Cell<u32> = Cell::new(0);
        }

        PROCESS_COUNT.with(|count| {
            let c = count.get() + 1;
            count.set(c);

            // Get audio slices from JACK ports
            let left_samples = self.in_left.as_slice(ps);
            let right_samples = self.in_right.as_slice(ps);

            // Log first few callbacks
            if c <= 5 {
                crate::debug_log!(
                    "[JACK PROCESS] Callback #{}: {} samples per channel",
                    c,
                    left_samples.len()
                );
            }

            // Convert stereo to mono and push to ring buffer
            let mut mono_samples = Vec::with_capacity(left_samples.len());
            for i in 0..left_samples.len() {
                let mono = (left_samples[i] + right_samples[i]) / 2.0;
                mono_samples.push(mono);
            }

            // Log audio statistics every 100 callbacks
            if c % 100 == 0 {
                let max_sample = mono_samples
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max);
                let min_sample = mono_samples
                    .iter()
                    .copied()
                    .fold(f32::INFINITY, f32::min);
                let avg_abs = mono_samples.iter().map(|s| s.abs()).sum::<f32>()
                    / mono_samples.len() as f32;
                crate::debug_log!(
                    "[JACK PROCESS] Callback #{}: {} samples, max={:.4}, min={:.4}, avg_abs={:.4}",
                    c,
                    mono_samples.len(),
                    max_sample,
                    min_sample,
                    avg_abs
                );
            }

            // Push to shared buffer
            if let Ok(mut buffer) = self.sample_buffer.lock() {
                buffer.push(&mono_samples);
            }
        });

        jack::Control::Continue
    }
}

/// Audio capture stream for visualization using JACK API
/// Captures audio from monitor ports and buffers samples for FFT processing
pub struct AudioCaptureStream {
    /// Device ID this stream is capturing from
    device_id: DeviceId,
    /// Port ID this stream is capturing from
    port_id: PortId,
    /// Ring buffer for incoming audio samples (thread-safe)
    sample_buffer: Arc<Mutex<RingBuffer>>,
    /// Sample rate of the stream
    sample_rate: u32,
    /// FFT processor for spectrum analysis
    fft_processor: FftProcessor,
    /// Event channel for sending spectrum updates
    event_tx: Sender<AudioEvent>,
    /// Last FFT processing timestamp
    last_process_time: Instant,
    /// JACK client (must be kept alive)
    _jack_client: jack::AsyncClient<(), JackProcessor>,
}

impl AudioCaptureStream {
    /// Create a new audio capture stream using JACK API
    pub fn new(
        _core: &(), // No longer need PipeWire core
        device_id: DeviceId,
        port_id: PortId,
        target_name: Option<String>,
        event_tx: Sender<AudioEvent>,
    ) -> Result<Self> {
        const BUFFER_CAPACITY: usize = 8192;
        const FFT_SIZE: usize = 2048;
        const NUM_BINS: usize = 64;

        let target = target_name.unwrap_or_else(|| {
            crate::debug_log!("[JACK] WARNING: No target provided");
            String::new()
        });

        crate::debug_log!(
            "[JACK] Creating capture stream for device={:?}, target={}",
            device_id,
            target
        );

        // Create ring buffer
        let sample_buffer = Arc::new(Mutex::new(RingBuffer::new(BUFFER_CAPACITY)));
        crate::debug_log!("[JACK] Ring buffer created with capacity {}", BUFFER_CAPACITY);

        // Create JACK client
        let client_name = format!("wavewire_{}", device_id.0);
        let (client, _status) =
            jack::Client::new(&client_name, jack::ClientOptions::NO_START_SERVER)?;

        let sample_rate = client.sample_rate();
        crate::debug_log!(
            "[JACK] Client created: {}, sample_rate={}Hz",
            client_name,
            sample_rate
        );

        // Create FFT processor with actual JACK sample rate
        let fft_processor = FftProcessor::new(FFT_SIZE, NUM_BINS, sample_rate as u32);

        // Register input ports (stereo)
        let in_left = client.register_port("capture_L", jack::AudioIn::default())?;
        let in_right = client.register_port("capture_R", jack::AudioIn::default())?;
        crate::debug_log!("[JACK] Registered input ports: capture_L, capture_R");

        // Create processor with shared buffer
        let processor = JackProcessor {
            in_left,
            in_right,
            sample_buffer: Arc::clone(&sample_buffer),
        };

        // Activate the client
        let async_client = client.activate_async((), processor)?;
        crate::debug_log!("[JACK] Client activated");

        // Connect to target ports if specified
        if !target.is_empty() {
            let client_ref = async_client.as_client();

            // Get all output ports (potential monitor sources)
            let all_ports = client_ref.ports(None, None, jack::PortFlags::IS_OUTPUT);

            crate::debug_log!("[JACK] Searching for monitor ports matching target: {}", target);

            // Find monitor ports that match our target
            // Look for ports containing the target name and "monitor"
            let mut left_port = None;
            let mut right_port = None;

            for port_name in all_ports.iter() {
                // Match ports that contain our target and monitor
                if port_name.contains("monitor") {
                    // For virtual sinks like "virtual_out_1", match "virtual_out_1 Audio/Sink sink:monitor_"
                    // For ALSA devices, the JACK name is different (e.g., "Elgato Wave XLR Analog Stereo")
                    let matches_target = if target.starts_with("virtual_") || target.starts_with("obs_") {
                        // Virtual sinks: look for exact prefix match
                        port_name.starts_with(&target)
                    } else if target.starts_with("alsa_output") || target.starts_with("alsa_input") {
                        // ALSA devices: they have friendly names, so just check if it contains "monitor"
                        // and is output port (we already filtered for output ports above)
                        true
                    } else {
                        port_name.contains(&target)
                    };

                    if matches_target {
                        if port_name.ends_with("monitor_FL") {
                            left_port = Some(port_name.clone());
                            crate::debug_log!("[JACK] Found left monitor port: {}", port_name);
                        } else if port_name.ends_with("monitor_FR") {
                            right_port = Some(port_name.clone());
                            crate::debug_log!("[JACK] Found right monitor port: {}", port_name);
                        }
                    }
                }
            }

            // Try to connect if we found both ports
            match (&left_port, &right_port) {
                (Some(left), Some(right)) => {
                    crate::debug_log!("[JACK] Attempting to connect to {} and {}", left, right);

                    match client_ref.connect_ports_by_name(left, &format!("{}:capture_L", client_name)) {
                        Ok(_) => {
                            crate::debug_log!("[JACK] ✓ Connected left channel");
                            match client_ref.connect_ports_by_name(right, &format!("{}:capture_R", client_name)) {
                                Ok(_) => crate::debug_log!("[JACK] ✓ Connected right channel"),
                                Err(e) => crate::debug_log!("[JACK] ✗ Failed to connect right: {}", e),
                            }
                        }
                        Err(e) => crate::debug_log!("[JACK] ✗ Failed to connect left: {}", e),
                    }
                }
                _ => {
                    crate::debug_log!("[JACK] WARNING: Could not find monitor ports for target: {}", target);
                    crate::debug_log!("[JACK] Found left: {:?}, Found right: {:?}", left_port, right_port);
                    crate::debug_log!("[JACK] You may need to connect manually using Helvum or pw-link");
                }
            }
        }

        // Send event that visualization started
        let _ = event_tx.send(AudioEvent::VisualizationStarted { device_id, port_id });

        Ok(Self {
            device_id,
            port_id,
            sample_buffer,
            sample_rate: sample_rate as u32,
            fft_processor,
            event_tx,
            last_process_time: Instant::now(),
            _jack_client: async_client,
        })
    }

    /// Get the device ID for this stream
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Get the port ID for this stream
    pub fn port_id(&self) -> PortId {
        self.port_id
    }

    /// Get the sample rate of this stream
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get access to the sample buffer
    pub fn sample_buffer(&self) -> &Arc<Mutex<RingBuffer>> {
        &self.sample_buffer
    }

    /// Check if we have enough samples for FFT processing
    pub fn has_enough_samples(&self, fft_size: usize) -> bool {
        self.sample_buffer
            .lock()
            .unwrap()
            .has_enough_samples(fft_size)
    }

    /// Process buffered audio and send spectrum update
    /// Should be called periodically (e.g., 20-30 Hz)
    pub fn process_spectrum(&mut self) {
        // Check if we have enough samples
        let fft_size = self.fft_processor.fft_size();
        if !self.has_enough_samples(fft_size) {
            return;
        }

        // Get samples from buffer
        let samples = self.sample_buffer.lock().unwrap().peek(fft_size);

        // Run FFT
        let (bins, frequencies) = self.fft_processor.process(&samples);

        // Create spectrum data
        let spectrum_data = SpectrumData {
            bins: bins.clone(),
            frequencies,
            sample_rate: self.sample_rate,
            timestamp: Instant::now(),
        };

        // Diagnostic logging
        crate::debug_log!(
            "[SPECTRUM] Device {:?}: Sending {} bins, sample: [{:.2}, {:.2}, {:.2}]",
            self.device_id,
            bins.len(),
            bins.get(0).unwrap_or(&-60.0),
            bins.get(32).unwrap_or(&-60.0),
            bins.get(63).unwrap_or(&-60.0)
        );

        // Send event
        let send_result = self.event_tx.send(AudioEvent::SpectrumUpdate {
            device_id: self.device_id,
            data: spectrum_data,
        });

        if let Err(e) = send_result {
            crate::debug_log!("[SPECTRUM] Event send failed: {:?}", e);
        }
    }

    /// Update the stream (process FFT if enough time has passed)
    /// Should be called from the audio thread periodically
    pub fn update(&mut self) {
        // Process spectrum at ~30 Hz
        const PROCESS_INTERVAL_MS: u128 = 33; // ~30 Hz
        let elapsed = self.last_process_time.elapsed().as_millis();

        let buffer_len = self.sample_buffer.lock().unwrap().len();
        let fft_size = self.fft_processor.fft_size();

        // Log periodically (every ~1 second)
        use std::cell::RefCell;
        thread_local! {
            static LAST_LOG: RefCell<Option<Instant>> = RefCell::new(None);
        }
        LAST_LOG.with(|last_log| {
            let mut last = last_log.borrow_mut();
            if last.is_none() || last.unwrap().elapsed().as_secs() >= 1 {
                crate::debug_log!(
                    "[UPDATE] Buffer: {}/{} samples, FFT needs {} samples",
                    buffer_len,
                    8192,
                    fft_size
                );
                *last = Some(Instant::now());
            }
        });

        if elapsed >= PROCESS_INTERVAL_MS {
            if buffer_len >= fft_size {
                crate::debug_log!("[UPDATE] Processing spectrum (buffer has enough samples)");
            }
            self.process_spectrum();
            self.last_process_time = Instant::now();
        }
    }
}

impl Drop for AudioCaptureStream {
    fn drop(&mut self) {
        crate::debug_log!(
            "[JACK] Dropping audio capture stream for device {:?}",
            self.device_id
        );
        // JACK client will be automatically deactivated and cleaned up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_push() {
        let mut buffer = RingBuffer::new(10);
        buffer.push(&[1.0, 2.0, 3.0]);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let mut buffer = RingBuffer::new(5);
        buffer.push(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(buffer.len(), 5);

        // Push more samples - oldest should be dropped
        buffer.push(&[6.0, 7.0]);
        assert_eq!(buffer.len(), 5);

        let samples = buffer.peek(5);
        assert_eq!(samples, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn test_ring_buffer_peek() {
        let mut buffer = RingBuffer::new(10);
        buffer.push(&[1.0, 2.0, 3.0, 4.0, 5.0]);

        let samples = buffer.peek(3);
        assert_eq!(samples, vec![3.0, 4.0, 5.0]);

        // Peek should not remove samples
        assert_eq!(buffer.len(), 5);
    }
}
