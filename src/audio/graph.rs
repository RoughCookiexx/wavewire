use std::collections::{HashMap, HashSet};

use super::eq::EqSettings;
use super::types::{DeviceId, DeviceType, PortId, PortInfo};

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub device_type: DeviceType,
    pub ports: Vec<PortInfo>,
    pub eq_settings: Option<EqSettings>,
}

impl DeviceInfo {
    pub fn new(id: DeviceId, name: String, device_type: DeviceType) -> Self {
        Self {
            id,
            name,
            device_type,
            ports: Vec::new(),
            eq_settings: None,
        }
    }
}

/// A connection between two ports
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Connection {
    pub source: PortId,
    pub destination: PortId,
}

impl Connection {
    pub fn new(source: PortId, destination: PortId) -> Self {
        Self {
            source,
            destination,
        }
    }
}

/// Graph tracking all audio devices and connections
pub struct RoutingGraph {
    /// All known devices (physical and virtual)
    devices: HashMap<DeviceId, DeviceInfo>,
    /// All active connections between ports
    connections: HashSet<Connection>,
    /// Counter for generating unique device IDs
    next_device_id: u64,
    /// Counter for generating unique port IDs
    next_port_id: u64,
}

impl RoutingGraph {
    /// Create a new empty routing graph
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            connections: HashSet::new(),
            next_device_id: 1,
            next_port_id: 1,
        }
    }

    /// Generate a new unique device ID
    pub fn generate_device_id(&mut self) -> DeviceId {
        let id = DeviceId::new(self.next_device_id);
        self.next_device_id += 1;
        id
    }

    /// Generate a new unique port ID
    pub fn generate_port_id(&mut self) -> PortId {
        let id = PortId::new(self.next_port_id);
        self.next_port_id += 1;
        id
    }

    /// Add a device to the graph
    pub fn add_device(&mut self, device: DeviceInfo) {
        self.devices.insert(device.id, device);
    }

    /// Remove a device from the graph
    pub fn remove_device(&mut self, device_id: DeviceId) -> Option<DeviceInfo> {
        self.devices.remove(&device_id)
    }

    /// Get a device by ID
    pub fn get_device(&self, device_id: DeviceId) -> Option<&DeviceInfo> {
        self.devices.get(&device_id)
    }

    /// Get a mutable reference to a device by ID
    pub fn get_device_mut(&mut self, device_id: DeviceId) -> Option<&mut DeviceInfo> {
        self.devices.get_mut(&device_id)
    }

    /// List all devices
    pub fn list_devices(&self) -> Vec<&DeviceInfo> {
        self.devices.values().collect()
    }

    /// Add a connection to the graph
    pub fn add_connection(&mut self, connection: Connection) {
        self.connections.insert(connection);
    }

    /// Remove a connection from the graph
    pub fn remove_connection(&mut self, connection: &Connection) -> bool {
        self.connections.remove(connection)
    }

    /// Get all connections for a specific port
    pub fn get_connections_for_port(&self, port_id: PortId) -> Vec<&Connection> {
        self.connections
            .iter()
            .filter(|conn| conn.source == port_id || conn.destination == port_id)
            .collect()
    }

    /// List all connections
    pub fn list_connections(&self) -> Vec<&Connection> {
        self.connections.iter().collect()
    }

    /// Find a port by its PipeWire port name
    pub fn find_port_by_name(&self, port_name: &str) -> Option<PortId> {
        self.devices
            .values()
            .flat_map(|device| &device.ports)
            .find(|port| port.pipewire_port_name == port_name)
            .map(|port| port.id)
    }

    /// Find a port name by its PortId
    pub fn find_port_name(&self, port_id: PortId) -> Option<&str> {
        self.devices
            .values()
            .flat_map(|device| &device.ports)
            .find(|port| port.id == port_id)
            .map(|port| port.pipewire_port_name.as_str())
    }
}

impl Default for RoutingGraph {
    fn default() -> Self {
        Self::new()
    }
}
