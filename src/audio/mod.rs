mod client;
mod device;
mod eq;
mod fft;
mod graph;
mod stream;
mod types;
mod volume;

pub use eq::{EqBandParams, EqSettings, GRAPHIC_EQ_BANDS};
pub use graph::DeviceInfo;
pub use types::{AudioCommand, AudioEvent, DeviceId, DeviceType, PortDirection, PortId, PortInfo, SpectrumData};
pub use volume::{VolumeSettings, VolumeProcessor, update_volume_settings};

use anyhow::Result;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

use client::PipeWireClient;

/// Main audio engine managing PipeWire client and routing
pub struct AudioEngine {
    /// PipeWire client wrapper
    pipewire_client: Option<PipeWireClient>,
    /// Channel for receiving events from audio thread
    event_rx: Receiver<AudioEvent>,
    /// Channel for sending commands to audio thread
    command_tx: Sender<AudioCommand>,
}

impl AudioEngine {
    /// Create a new audio engine
    pub fn new() -> Result<Self> {
        // Create channels for communication between UI and audio threads
        let (event_tx, event_rx) = unbounded(); // Events from audio → UI
        let (command_tx, command_rx) = bounded(100); // Commands from UI → audio

        // Create PipeWire client with event and command channels
        let pipewire_client = PipeWireClient::new(event_tx, command_rx)?;

        Ok(Self {
            pipewire_client: Some(pipewire_client),
            event_rx,
            command_tx,
        })
    }

    /// Start the audio engine and connect to PipeWire
    pub fn start(&mut self) -> Result<()> {
        if let Some(ref mut pipewire_client) = self.pipewire_client {
            pipewire_client.activate()?;
        } else {
            anyhow::bail!("PipeWire client not initialized");
        }

        Ok(())
    }

    /// Stop the audio engine and disconnect from PipeWire
    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut pipewire_client) = self.pipewire_client.take() {
            pipewire_client.deactivate()?;
        }
        Ok(())
    }

    /// Poll for events from the audio thread (non-blocking)
    pub fn poll_events(&self) -> Vec<AudioEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Send a command to the audio thread
    pub fn send_command(&self, command: AudioCommand) -> Result<()> {
        self.command_tx
            .send(command)
            .map_err(|e| anyhow::anyhow!("Failed to send command: {}", e))
    }

    /// List all discovered audio devices
    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        if let Some(ref pipewire_client) = self.pipewire_client {
            let graph = pipewire_client.routing_graph().read().unwrap();
            Ok(graph.list_devices().into_iter().cloned().collect())
        } else {
            anyhow::bail!("PipeWire client not initialized")
        }
    }

    /// Create a new virtual audio device
    pub fn create_virtual_device(
        &mut self,
        name: String,
        num_inputs: usize,
        num_outputs: usize,
    ) -> Result<DeviceId> {
        if let Some(ref mut pipewire_client) = self.pipewire_client {
            pipewire_client.create_virtual_device(name, num_inputs, num_outputs)
        } else {
            anyhow::bail!("PipeWire client not initialized")
        }
    }

    /// Destroy a virtual audio device
    pub fn destroy_virtual_device(&mut self, device_id: DeviceId) -> Result<()> {
        if let Some(ref mut pipewire_client) = self.pipewire_client {
            pipewire_client.destroy_virtual_device(device_id)
        } else {
            anyhow::bail!("PipeWire client not initialized")
        }
    }
}
