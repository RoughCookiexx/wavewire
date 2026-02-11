use anyhow::Result;
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
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

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
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

    /// Clear all samples from the buffer
    pub fn clear(&mut self) {
        self.buffer.clear()
    }
}

/// Audio capture stream for visualization using pw-record subprocess
/// Captures audio from a specific port and buffers samples for FFT processing
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
    /// pw-record subprocess (must be kept alive)
    _subprocess: Option<Child>,
}

impl AudioCaptureStream {
    /// Create a new audio capture stream using pw-record subprocess
    pub fn new(
        _core: &pipewire::core::CoreRc,
        device_id: DeviceId,
        port_id: PortId,
        pw_node_id: Option<u32>,
        event_tx: Sender<AudioEvent>,
    ) -> Result<Self> {
        const DEFAULT_SAMPLE_RATE: u32 = 48000;
        const BUFFER_CAPACITY: usize = 8192;
        const FFT_SIZE: usize = 2048;
        const NUM_BINS: usize = 64;

        crate::debug_log!("[STREAM] Creating pw-record stream for device={:?}, port={:?}, node_id={:?}",
                  device_id, port_id, pw_node_id);

        // Create ring buffer (thread-safe with Arc<Mutex<>>)
        let sample_buffer = Arc::new(Mutex::new(RingBuffer::new(BUFFER_CAPACITY)));
        crate::debug_log!("[STREAM] Ring buffer created with capacity {}", BUFFER_CAPACITY);

        // Create FFT processor
        let fft_processor = FftProcessor::new(FFT_SIZE, NUM_BINS, DEFAULT_SAMPLE_RATE);
        crate::debug_log!("[STREAM] FFT processor created (size={}, bins={})", FFT_SIZE, NUM_BINS);

        // Spawn pw-record subprocess to capture audio
        let target = if let Some(node_id) = pw_node_id {
            node_id.to_string()
        } else {
            crate::debug_log!("[STREAM] WARNING: No node ID provided, using @DEFAULT_SINK@");
            "@DEFAULT_SINK@".to_string()
        };

        crate::debug_log!("[STREAM] Spawning: pw-record --target {} --format f32 --rate 48000 --channels 2 -", target);

        let mut child = Command::new("pw-record")
            .arg("--target")
            .arg(&target)
            .arg("--format")
            .arg("f32")
            .arg("--rate")
            .arg("48000")
            .arg("--channels")
            .arg("2")
            .arg("-") // Write to stdout
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        crate::debug_log!("[STREAM] pw-record subprocess spawned");

        // Get stdout handle
        let mut stdout = child.stdout.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to get stdout from pw-record")
        })?;

        // Clone buffer for reader thread
        let buffer_clone = Arc::clone(&sample_buffer);

        // Spawn thread to read from pw-record stdout
        crate::debug_log!("[STREAM] Starting reader thread");
        thread::spawn(move || {
            let mut first_data = true;
            let mut f32_buffer = [0f32; 2048]; // Read 2048 f32 samples at a time
            let byte_buffer_size = f32_buffer.len() * std::mem::size_of::<f32>();
            let mut byte_buffer = vec![0u8; byte_buffer_size];

            loop {
                match stdout.read_exact(&mut byte_buffer) {
                    Ok(()) => {
                        if first_data {
                            crate::debug_log!("[READER] *** FIRST DATA RECEIVED FROM PW-RECORD! ***");
                            first_data = false;
                        }

                        // Convert bytes to f32 samples
                        for (i, chunk) in byte_buffer.chunks_exact(4).enumerate() {
                            f32_buffer[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        }

                        // Convert stereo to mono (L,R,L,R -> mono)
                        let mut mono_samples = Vec::with_capacity(f32_buffer.len() / 2);
                        for chunk in f32_buffer.chunks_exact(2) {
                            let mono = (chunk[0] + chunk[1]) / 2.0;
                            mono_samples.push(mono);
                        }

                        // Push to ring buffer
                        buffer_clone.lock().unwrap().push(&mono_samples);

                        // Log occasionally
                        use std::cell::Cell;
                        thread_local! {
                            static READ_COUNT: Cell<u32> = Cell::new(0);
                        }
                        READ_COUNT.with(|count| {
                            let c = count.get() + 1;
                            count.set(c);
                            if c % 100 == 0 {
                                crate::debug_log!("[READER] Read {} chunks, buffer: {} samples",
                                                 c, buffer_clone.lock().unwrap().len());
                            }
                        });
                    }
                    Err(e) => {
                        crate::debug_log!("[READER] Read error: {}, exiting", e);
                        break;
                    }
                }
            }
        });

        crate::debug_log!("[STREAM] Reader thread started");

        // Send event that visualization started
        let _ = event_tx.send(AudioEvent::VisualizationStarted { device_id, port_id });

        Ok(Self {
            device_id,
            port_id,
            sample_buffer,
            sample_rate: DEFAULT_SAMPLE_RATE,
            fft_processor,
            event_tx,
            last_process_time: Instant::now(),
            _subprocess: Some(child),
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
        self.sample_buffer.lock().unwrap().has_enough_samples(fft_size)
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
            bins,
            frequencies,
            sample_rate: self.sample_rate,
            timestamp: Instant::now(),
        };

        // Send event
        let _ = self.event_tx.send(AudioEvent::SpectrumUpdate {
            device_id: self.device_id,
            data: spectrum_data,
        });
    }

    /// Update the stream (process FFT if enough time has passed)
    /// Should be called from the audio thread periodically
    /// Note: Audio data comes in via the reader thread
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
                crate::debug_log!("[UPDATE] Buffer: {}/{} samples, FFT needs {} samples",
                         buffer_len, 8192, fft_size);
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
        // Kill pw-record subprocess when stream is dropped
        if let Some(mut child) = self._subprocess.take() {
            crate::debug_log!("[STREAM] Killing pw-record subprocess for device {:?}", self.device_id);
            let _ = child.kill();
            let _ = child.wait();
        }
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
