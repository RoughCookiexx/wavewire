use std::fmt;
use std::time::Instant;

use super::eq::EqSettings;
use super::volume::VolumeSettings;

/// Unique identifier for an audio device
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId(pub u64);

impl DeviceId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Device({})", self.0)
    }
}

/// Unique identifier for an audio port
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(pub u64);

impl PortId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Port({})", self.0)
    }
}

/// Type of audio device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    /// Physical hardware device discovered via PipeWire
    Physical,
    /// Virtual device created by wavewire
    Virtual,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceType::Physical => write!(f, "Physical"),
            DeviceType::Virtual => write!(f, "Virtual"),
        }
    }
}

/// Direction of audio flow for a port
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// Input port (receives audio data)
    Input,
    /// Output port (sends audio data)
    Output,
}

impl fmt::Display for PortDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortDirection::Input => write!(f, "Input"),
            PortDirection::Output => write!(f, "Output"),
        }
    }
}

/// Information about an audio port
#[derive(Debug, Clone)]
pub struct PortInfo {
    /// Unique identifier for this port
    pub id: PortId,
    /// Display name (just the port part, not the full PipeWire name)
    pub name: String,
    /// Direction of audio flow
    pub direction: PortDirection,
    /// Full PipeWire port name (format: "node_name:port_name")
    pub pipewire_port_name: String,
}

impl PortInfo {
    pub fn new(id: PortId, name: String, direction: PortDirection, pipewire_port_name: String) -> Self {
        Self {
            id,
            name,
            direction,
            pipewire_port_name,
        }
    }
}

/// Commands sent from UI thread to audio thread
#[derive(Debug)]
pub enum AudioCommand {
    /// Create a new virtual device
    CreateVirtualDevice {
        name: String,
        num_inputs: usize,
        num_outputs: usize,
    },
    /// Destroy a virtual device
    DestroyVirtualDevice { device_id: DeviceId },
    /// Connect two ports
    Connect {
        source_port: String,
        dest_port: String,
    },
    /// Disconnect two ports
    Disconnect {
        source_port: String,
        dest_port: String,
    },
    /// Start visualization for a device
    StartVisualization {
        device_id: DeviceId,
        port_id: PortId,
    },
    /// Stop visualization for a device
    StopVisualization {
        device_id: DeviceId,
    },
    /// Enable EQ for a device
    EnableEq {
        device_id: DeviceId,
        settings: EqSettings,
    },
    /// Disable EQ for a device
    DisableEq {
        device_id: DeviceId,
    },
    /// Update a single EQ band
    SetEqBand {
        device_id: DeviceId,
        band_index: usize,
        gain_db: f32,
        q_value: f32,
    },
    /// Update all EQ bands at once
    SetEqSettings {
        device_id: DeviceId,
        settings: EqSettings,
    },
    /// Toggle EQ bypass
    SetEqBypass {
        device_id: DeviceId,
        bypass: bool,
    },
    /// Reset EQ to flat (all gains = 0 dB)
    ResetEq {
        device_id: DeviceId,
    },
    /// Set volume for a device
    SetVolume {
        device_id: DeviceId,
        settings: VolumeSettings,
    },
}

/// Events sent from audio thread to UI thread
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// A new device was discovered or created
    DeviceAdded {
        device_id: DeviceId,
        name: String,
        device_type: DeviceType,
    },
    /// A device was removed or destroyed
    DeviceRemoved { device_id: DeviceId },
    /// A connection was established
    ConnectionEstablished {
        source: String,
        destination: String,
    },
    /// A connection was broken
    ConnectionBroken {
        source: String,
        destination: String,
    },
    /// PipeWire buffer underrun or overrun occurred
    Xrun,
    /// An error occurred
    Error { message: String },
    /// Visualization started for a device
    VisualizationStarted {
        device_id: DeviceId,
        port_id: PortId,
    },
    /// Visualization stopped for a device
    VisualizationStopped {
        device_id: DeviceId,
    },
    /// Spectrum data update from FFT processing
    SpectrumUpdate {
        device_id: DeviceId,
        data: SpectrumData,
    },
    /// EQ was enabled for a device
    EqEnabled {
        device_id: DeviceId,
        settings: EqSettings,
    },
    /// EQ was disabled for a device
    EqDisabled {
        device_id: DeviceId,
    },
    /// EQ settings were updated
    EqUpdated {
        device_id: DeviceId,
        settings: EqSettings,
    },
    /// Volume was updated for a device
    VolumeUpdated {
        device_id: DeviceId,
        settings: VolumeSettings,
    },
}

/// Frequency spectrum data for visualization
#[derive(Debug, Clone)]
pub struct SpectrumData {
    /// Frequency bin magnitudes in dB (typically 64-128 bins)
    pub bins: Vec<f32>,
    /// Corresponding frequencies in Hz for each bin
    pub frequencies: Vec<f32>,
    /// Sample rate of the audio source
    pub sample_rate: u32,
    /// Timestamp when this data was processed
    pub timestamp: Instant,
}
