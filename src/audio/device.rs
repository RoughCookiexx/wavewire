use anyhow::Result;

use super::types::DeviceId;

/// A virtual audio device created by wavewire
pub struct VirtualDevice {
    /// Unique identifier for this device
    pub id: DeviceId,
    /// Display name of the device
    pub name: String,
    /// Number of input ports
    pub num_inputs: usize,
    /// Number of output ports
    pub num_outputs: usize,
}

impl VirtualDevice {
    /// Create a new virtual device with the specified number of input and output ports
    pub fn new(
        id: DeviceId,
        name: String,
        num_inputs: usize,
        num_outputs: usize,
    ) -> Result<Self> {
        // For now, we just store the metadata
        // Actual PipeWire node/port creation will be implemented later
        Ok(Self {
            id,
            name,
            num_inputs,
            num_outputs,
        })
    }
}
