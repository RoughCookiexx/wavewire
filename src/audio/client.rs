use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, unbounded};
use pipewire::{
    context::ContextRc, link::Link, main_loop::MainLoopRc, node::Node, port::Port,
    types::ObjectType,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

use super::device::VirtualDevice;
use super::graph::{DeviceInfo, RoutingGraph};
use super::stream::AudioCaptureStream;
use super::types::{
    AudioCommand, AudioEvent, DeviceId, DeviceType, PortDirection, PortId, PortInfo,
};

// Thread-local storage at module level for PipeWire objects
// These must be at module level to be accessible from closures
thread_local! {
    static LINKS: RefCell<HashMap<u32, Link>> = RefCell::new(HashMap::new());
    static CONNECTION_TO_LINK: RefCell<HashMap<(PortId, PortId), u32>> = RefCell::new(HashMap::new());
    static CAPTURE_STREAMS: RefCell<HashMap<DeviceId, AudioCaptureStream>> = RefCell::new(HashMap::new());
}

/// PipeWire client wrapper managing audio processing
pub struct PipeWireClient {
    /// Routing graph tracking all devices and connections
    routing_graph: Arc<RwLock<RoutingGraph>>,
    /// Virtual devices created by wavewire
    virtual_devices: Arc<RwLock<HashMap<DeviceId, VirtualDevice>>>,
    /// Channel for sending events to UI thread
    event_tx: Option<Sender<AudioEvent>>,
    /// Channel for receiving commands from UI thread
    command_rx: Option<Receiver<AudioCommand>>,
    /// Thread handle for PipeWire event loop
    event_thread: Option<JoinHandle<()>>,
    /// Channel for signaling event loop thread to quit
    quit_tx: Option<Sender<()>>,
    /// Mapping from PipeWire global ID to our internal DeviceId
    pw_node_map: Arc<RwLock<HashMap<u32, DeviceId>>>,
    /// Mapping from PipeWire global ID to our internal PortId
    pw_port_map: Arc<RwLock<HashMap<u32, PortId>>>,
    /// Track if client is activated
    is_activated: bool,
}

impl PipeWireClient {
    /// Create a new PipeWire client
    pub fn new(event_tx: Sender<AudioEvent>, command_rx: Receiver<AudioCommand>) -> Result<Self> {
        Ok(Self {
            routing_graph: Arc::new(RwLock::new(RoutingGraph::new())),
            virtual_devices: Arc::new(RwLock::new(HashMap::new())),
            event_tx: Some(event_tx),
            command_rx: Some(command_rx),
            event_thread: None,
            quit_tx: None,
            pw_node_map: Arc::new(RwLock::new(HashMap::new())),
            pw_port_map: Arc::new(RwLock::new(HashMap::new())),
            is_activated: false,
        })
    }

    /// Activate the PipeWire client and start audio processing
    pub fn activate(&mut self) -> Result<()> {
        if self.is_activated {
            return Ok(());
        }

        // Create quit channel
        let (quit_tx, quit_rx) = unbounded();
        self.quit_tx = Some(quit_tx);

        // Clone necessary data for the event loop thread
        let routing_graph = Arc::clone(&self.routing_graph);
        let pw_node_map = Arc::clone(&self.pw_node_map);
        let pw_port_map = Arc::clone(&self.pw_port_map);
        let event_tx = self
            .event_tx
            .as_ref()
            .context("Event sender not initialized")?
            .clone();
        let command_rx = self
            .command_rx
            .take()
            .context("Command receiver not initialized")?;

        // Spawn thread to run the PipeWire event loop
        // All PipeWire objects must be created and owned by this thread
        // since they use Rc (not thread-safe)
        let event_thread = thread::spawn(move || {
            // Initialize PipeWire
            pipewire::init();

            // Create main loop
            let main_loop = match MainLoopRc::new(None) {
                Ok(ml) => ml,
                Err(e) => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Failed to create PipeWire main loop: {}", e),
                    });
                    return;
                }
            };

            // Create context
            let context = match ContextRc::new(&main_loop, None) {
                Ok(ctx) => ctx,
                Err(e) => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Failed to create PipeWire context: {}", e),
                    });
                    return;
                }
            };

            // Connect to PipeWire daemon
            let core = match context.connect_rc(None) {
                Ok(core) => core,
                Err(e) => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Failed to connect to PipeWire daemon: {}", e),
                    });
                    return;
                }
            };

            // Get registry for device discovery
            let registry = match core.get_registry_rc() {
                Ok(reg) => reg,
                Err(e) => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Failed to get registry: {}", e),
                    });
                    return;
                }
            };

            // Weak reference to registry for use in closures
            let registry_weak = registry.downgrade();

            // Clone for all handlers upfront (before creating any closures)
            let routing_graph_global = Arc::clone(&routing_graph);
            let pw_node_map_global = Arc::clone(&pw_node_map);
            let pw_port_map_global = Arc::clone(&pw_port_map);
            let event_tx_global = event_tx.clone();

            let routing_graph_remove = Arc::clone(&routing_graph);
            let pw_node_map_remove = Arc::clone(&pw_node_map);
            let pw_port_map_remove = Arc::clone(&pw_port_map);
            let event_tx_remove = event_tx.clone();

            let core_cmd = core.clone();
            let routing_graph_cmd = Arc::clone(&routing_graph);
            let pw_node_map_cmd = Arc::clone(&pw_node_map);
            let pw_port_map_cmd = Arc::clone(&pw_port_map);
            let event_tx_cmd = event_tx.clone();
            let main_loop_cmd = main_loop.clone();

            // Set up registry listener for device discovery
            let _registry_listener = registry
                .add_listener_local()
                .global(move |obj| {
                    if let Some(registry) = registry_weak.upgrade() {
                        Self::handle_registry_object(
                            &registry,
                            &routing_graph_global,
                            &pw_node_map_global,
                            &pw_port_map_global,
                            &event_tx_global,
                            obj,
                        );
                    }
                })
                .global_remove(move |id| {
                    Self::handle_registry_remove(
                        &routing_graph_remove,
                        &pw_node_map_remove,
                        &pw_port_map_remove,
                        &event_tx_remove,
                        id,
                    );
                })
                .register();

            // Set up a timer to poll for commands periodically
            // This allows us to process commands while the event loop is running

            let timer_source = main_loop
                .loop_()
                .add_timer(move |_expirations| {
                    // Update all active capture streams (generate test data and process FFT)
                    // Use the CAPTURE_STREAMS from the outer scope (line 184)
                    CAPTURE_STREAMS.with(|streams| {
                        let num_streams = streams.borrow().len();
                        if num_streams > 0 {
                            // Log occasionally to avoid spam
                            static mut TIMER_TICK: u32 = 0;
                            unsafe {
                                TIMER_TICK += 1;
                                if TIMER_TICK % 100 == 0 { // Every ~1 second (100 * 10ms)
                                    crate::debug_log!("[TIMER] Updating {} active stream(s)", num_streams);
                                }
                            }
                        }

                        for stream in streams.borrow_mut().values_mut() {
                            stream.update();
                        }
                    });

                    // Poll for commands (non-blocking)
                    match command_rx.try_recv() {
                        Ok(AudioCommand::Connect { source_port, dest_port }) => {
                            Self::handle_connect_command(
                                &core_cmd,
                                &routing_graph_cmd,
                                &pw_port_map_cmd,
                                &event_tx_cmd,
                                &source_port,
                                &dest_port,
                            );
                        }
                        Ok(AudioCommand::Disconnect { source_port, dest_port }) => {
                            Self::handle_disconnect_command(
                                &routing_graph_cmd,
                                &event_tx_cmd,
                                &source_port,
                                &dest_port,
                            );
                        }
                        Ok(AudioCommand::CreateVirtualDevice { .. }) => {
                            // TODO: Implement virtual device creation
                        }
                        Ok(AudioCommand::DestroyVirtualDevice { .. }) => {
                            // TODO: Implement virtual device destruction
                        }
                        Ok(AudioCommand::StartVisualization { device_id, port_id }) => {
                            Self::handle_start_visualization_command(
                                &core_cmd,
                                &routing_graph_cmd,
                                &pw_node_map_cmd,
                                &event_tx_cmd,
                                device_id,
                                port_id,
                            );
                        }
                        Ok(AudioCommand::StopVisualization { device_id }) => {
                            Self::handle_stop_visualization_command(
                                &event_tx_cmd,
                                device_id,
                            );
                        }
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            // Command channel closed, quit the loop
                            main_loop_cmd.quit();
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => {
                            // No commands, continue
                        }
                    }
                });

            // Arm the timer to fire every 10ms
            timer_source.update_timer(
                Some(std::time::Duration::from_millis(10)), // Initial delay
                Some(std::time::Duration::from_millis(10)), // Repeat interval
            );

            // Keep objects alive
            let _context = context;
            let _registry = registry;
            let _listener = _registry_listener;
            let _timer_source = timer_source;
            let _quit_rx = quit_rx; // Keep alive for future use

            // Run the main loop (blocks until quit is called)
            main_loop.run();

            // Cleanup (may not be reached if process exits abruptly)
            unsafe {
                pipewire::deinit();
            }
        });

        self.event_thread = Some(event_thread);
        self.is_activated = true;

        Ok(())
    }

    /// Deactivate the PipeWire client and stop audio processing
    pub fn deactivate(&mut self) -> Result<()> {
        if !self.is_activated {
            return Ok(());
        }

        // Drop the quit channel sender
        // The event loop thread will continue running but won't block process exit
        self.quit_tx.take();

        // Don't wait for the event thread - PipeWire's MainLoopRc can't be
        // safely signaled from another thread. The thread will be cleaned up
        // by the OS when the process exits.
        self.event_thread.take();

        self.is_activated = false;
        Ok(())
    }

    /// Handle a global object discovered via PipeWire registry
    fn handle_registry_object(
        registry: &pipewire::registry::RegistryRc,
        routing_graph: &Arc<RwLock<RoutingGraph>>,
        pw_node_map: &Arc<RwLock<HashMap<u32, DeviceId>>>,
        pw_port_map: &Arc<RwLock<HashMap<u32, PortId>>>,
        event_tx: &Sender<AudioEvent>,
        obj: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    ) {
        // Store objects in thread-local storage so they stay alive for the duration
        // of the registry listener
        use std::any::Any;
        thread_local! {
            static NODES: RefCell<Vec<Node>> = RefCell::new(Vec::new());
            static PORTS: RefCell<Vec<Port>> = RefCell::new(Vec::new());
            static LISTENERS: RefCell<Vec<Box<dyn Any>>> = RefCell::new(Vec::new());
        }

        match obj.type_ {
            ObjectType::Node => {
                // Bind the node proxy
                let node: Node = match registry.bind(obj) {
                    Ok(node) => node,
                    Err(_) => return,
                };

                // Clone for the listener callback
                let routing_graph = Arc::clone(routing_graph);
                let pw_node_map = Arc::clone(pw_node_map);
                let event_tx = event_tx.clone();
                let global_id = obj.id;

                // Add listener to get node info
                let listener = node
                    .add_listener_local()
                    .info(move |info| {
                        let props = info.props();

                        // Extract node name
                        let node_name = props
                            .and_then(|p| p.get("node.name"))
                            .or_else(|| props.and_then(|p| p.get("node.description")))
                            .unwrap_or("Unknown Node")
                            .to_string();

                        // Skip wavewire's own virtual devices to avoid duplicates
                        if node_name.starts_with("wavewire_virtual_") {
                            return;
                        }

                        // Create device info
                        let device_id = {
                            let mut graph = routing_graph.write().unwrap();
                            let device_id = graph.generate_device_id();

                            let device_info =
                                DeviceInfo::new(device_id, node_name.clone(), DeviceType::Physical);
                            graph.add_device(device_info);

                            device_id
                        };

                        // Track PipeWire node ID -> our DeviceId
                        {
                            let mut node_map = pw_node_map.write().unwrap();
                            node_map.insert(global_id, device_id);
                        }

                        // Send event to UI
                        let _ = event_tx.send(AudioEvent::DeviceAdded {
                            device_id,
                            name: node_name,
                            device_type: DeviceType::Physical,
                        });
                    })
                    .register();

                // Store node and listener to keep them alive
                NODES.with(|nodes| nodes.borrow_mut().push(node));
                LISTENERS.with(|listeners| listeners.borrow_mut().push(Box::new(listener)));
            }
            ObjectType::Port => {
                // Bind the port proxy
                let port: Port = match registry.bind(obj) {
                    Ok(port) => port,
                    Err(_) => return,
                };

                // Clone for the listener callback
                let routing_graph = Arc::clone(routing_graph);
                let pw_node_map = Arc::clone(pw_node_map);
                let pw_port_map = Arc::clone(pw_port_map);
                let global_id = obj.id;

                // Add listener to get port info
                let listener = port
                    .add_listener_local()
                    .info(move |info| {
                        let props = info.props();

                        // Get the parent node ID
                        let node_id: u32 = props
                            .and_then(|p| p.get("node.id"))
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);

                        // Find the device this port belongs to
                        let device_id = {
                            let node_map = pw_node_map.read().unwrap();
                            node_map.get(&node_id).copied()
                        };

                        if let Some(device_id) = device_id {
                            // Extract port information
                            let port_name = props
                                .and_then(|p| p.get("port.name"))
                                .unwrap_or("unknown")
                                .to_string();

                            let port_direction = props
                                .and_then(|p| p.get("port.direction"))
                                .map(|d| {
                                    if d == "in" {
                                        PortDirection::Input
                                    } else {
                                        PortDirection::Output
                                    }
                                })
                                .unwrap_or(PortDirection::Output);

                            // Generate full PipeWire port name
                            let node_name =
                                props.and_then(|p| p.get("node.name")).unwrap_or("unknown");
                            let pw_port_name = format!("{}:{}", node_name, port_name);

                            // Add port to device
                            let port_id = {
                                let mut graph = routing_graph.write().unwrap();
                                let port_id = graph.generate_port_id();

                                if let Some(device) = graph.get_device_mut(device_id) {
                                    device.ports.push(PortInfo::new(
                                        port_id,
                                        port_name,
                                        port_direction,
                                        pw_port_name,
                                    ));
                                }

                                port_id
                            };

                            // Track PipeWire port ID -> our PortId
                            let mut port_map = pw_port_map.write().unwrap();
                            port_map.insert(global_id, port_id);
                        }
                    })
                    .register();

                // Store port and listener to keep them alive
                PORTS.with(|ports| ports.borrow_mut().push(port));
                LISTENERS.with(|listeners| listeners.borrow_mut().push(Box::new(listener)));
            }
            ObjectType::Link => {
                // Bind the link proxy
                let link: Link = match registry.bind(obj) {
                    Ok(link) => link,
                    Err(_) => return,
                };

                // Clone for listener callback
                let routing_graph = Arc::clone(routing_graph);
                let pw_port_map = Arc::clone(pw_port_map);
                let event_tx = event_tx.clone();
                let global_id = obj.id;

                // Add listener to get link info
                let listener = link
                    .add_listener_local()
                    .info(move |info| {
                        // Extract port IDs from link info
                        let output_port_id = info.output_port_id();
                        let input_port_id = info.input_port_id();

                        // Map PipeWire port IDs to our PortId
                        let port_map = pw_port_map.read().unwrap();
                        let source_port_id = port_map.get(&output_port_id).copied();
                        let dest_port_id = port_map.get(&input_port_id).copied();

                        if let (Some(source), Some(dest)) = (source_port_id, dest_port_id) {
                            // Add connection to graph
                            {
                                let mut graph = routing_graph.write().unwrap();
                                graph.add_connection(super::graph::Connection::new(source, dest));
                            }

                            // Get port names for event
                            let (source_name, dest_name) = {
                                let graph = routing_graph.read().unwrap();
                                (
                                    graph.find_port_name(source).map(|s| s.to_string()),
                                    graph.find_port_name(dest).map(|s| s.to_string()),
                                )
                            };

                            if let (Some(s), Some(d)) = (source_name, dest_name) {
                                let _ = event_tx.send(AudioEvent::ConnectionEstablished {
                                    source: s,
                                    destination: d,
                                });
                            }

                            // Store link ID mapping for disconnection
                            thread_local! {
                                static LINKS: RefCell<HashMap<u32, Link>> = RefCell::new(HashMap::new());
                                static CONNECTION_TO_LINK: RefCell<HashMap<(PortId, PortId), u32>> = RefCell::new(HashMap::new());
                            }

                            CONNECTION_TO_LINK.with(|conn_map| {
                                conn_map.borrow_mut().insert((source, dest), global_id);
                            });
                        }
                    })
                    .register();

                // Store link and listener to keep them alive
                thread_local! {
                    static LINKS: RefCell<HashMap<u32, Link>> = RefCell::new(HashMap::new());
                }

                LINKS.with(|links| {
                    links.borrow_mut().insert(global_id, link);
                });
                LISTENERS.with(|listeners| listeners.borrow_mut().push(Box::new(listener)));
            }
            _ => {
                // Ignore other object types for now
            }
        }
    }

    /// Handle removal of a global object from PipeWire registry
    fn handle_registry_remove(
        routing_graph: &Arc<RwLock<RoutingGraph>>,
        pw_node_map: &Arc<RwLock<HashMap<u32, DeviceId>>>,
        pw_port_map: &Arc<RwLock<HashMap<u32, PortId>>>,
        event_tx: &Sender<AudioEvent>,
        id: u32,
    ) {
        // Check if it's a node being removed
        if let Some(device_id) = {
            let mut node_map = pw_node_map.write().unwrap();
            node_map.remove(&id)
        } {
            // Remove device from graph
            let mut graph = routing_graph.write().unwrap();
            graph.remove_device(device_id);

            // Send event to UI
            let _ = event_tx.send(AudioEvent::DeviceRemoved { device_id });
        }

        // Check if it's a port being removed
        if let Some(_port_id) = {
            let mut port_map = pw_port_map.write().unwrap();
            port_map.remove(&id)
        } {
            // TODO: Remove port from device in routing graph
            // This requires maintaining port -> device relationships
        }

        // Check if it's a link being removed
        thread_local! {
            static LINKS: RefCell<HashMap<u32, Link>> = RefCell::new(HashMap::new());
            static CONNECTION_TO_LINK: RefCell<HashMap<(PortId, PortId), u32>> = RefCell::new(HashMap::new());
        }

        LINKS.with(|links| {
            if links.borrow_mut().remove(&id).is_some() {
                // Link removed - find and remove corresponding connection
                CONNECTION_TO_LINK.with(|conn_map| {
                    let mut conn_map = conn_map.borrow_mut();
                    if let Some(connection_key) = conn_map
                        .iter()
                        .find(|&(_, link_id)| *link_id == id)
                        .map(|(k, _)| *k)
                    {
                        let (source, dest) = connection_key;
                        conn_map.remove(&connection_key);

                        // Remove from routing graph
                        {
                            let mut graph = routing_graph.write().unwrap();
                            graph.remove_connection(&super::graph::Connection::new(source, dest));
                        }

                        // Get port names for event
                        let (source_name, dest_name) = {
                            let graph = routing_graph.read().unwrap();
                            (
                                graph.find_port_name(source).map(|s| s.to_string()),
                                graph.find_port_name(dest).map(|s| s.to_string()),
                            )
                        };

                        if let (Some(s), Some(d)) = (source_name, dest_name) {
                            let _ = event_tx.send(AudioEvent::ConnectionBroken {
                                source: s,
                                destination: d,
                            });
                        }
                    }
                });
            }
        });
    }

    /// Handle connect command - create a link between two ports
    fn handle_connect_command(
        core: &pipewire::core::CoreRc,
        routing_graph: &Arc<RwLock<RoutingGraph>>,
        pw_port_map: &Arc<RwLock<HashMap<u32, PortId>>>,
        event_tx: &Sender<AudioEvent>,
        source_port: &str,
        dest_port: &str,
    ) {
        // Resolve port names to PortIds
        let (source_id, dest_id) = {
            let graph = routing_graph.read().unwrap();
            match (
                graph.find_port_by_name(source_port),
                graph.find_port_by_name(dest_port),
            ) {
                (Some(src), Some(dst)) => (src, dst),
                _ => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Ports not found: {} -> {}", source_port, dest_port),
                    });
                    return;
                }
            }
        };

        // Create link using PipeWire link-factory
        // Use the properties! macro to create the properties dict
        let props = &pipewire::properties::properties! {
            "link.output.port" => source_port,
            "link.input.port" => dest_port,
            "object.linger" => "1",
        };

        match core.create_object::<Link>("link-factory", props) {
            Ok(_link) => {
                // Link created successfully
                // Add connection to routing graph (tentatively)
                {
                    let mut graph = routing_graph.write().unwrap();
                    graph.add_connection(super::graph::Connection::new(source_id, dest_id));
                }

                // Send success event
                let _ = event_tx.send(AudioEvent::ConnectionEstablished {
                    source: source_port.to_string(),
                    destination: dest_port.to_string(),
                });

                // Note: Link object will be tracked when registry discovers it
                // The link's global ID and full lifecycle is managed by the registry listener
            }
            Err(e) => {
                let _ = event_tx.send(AudioEvent::Error {
                    message: format!(
                        "Failed to create link {} -> {}: {}",
                        source_port, dest_port, e
                    ),
                });
            }
        }
    }

    /// Handle disconnect command - destroy a link between two ports
    fn handle_disconnect_command(
        routing_graph: &Arc<RwLock<RoutingGraph>>,
        event_tx: &Sender<AudioEvent>,
        source_port: &str,
        dest_port: &str,
    ) {
        // Resolve port names to PortIds
        let (source_id, dest_id) = {
            let graph = routing_graph.read().unwrap();
            match (
                graph.find_port_by_name(source_port),
                graph.find_port_by_name(dest_port),
            ) {
                (Some(src), Some(dst)) => (src, dst),
                _ => {
                    let _ = event_tx.send(AudioEvent::Error {
                        message: format!("Ports not found: {} -> {}", source_port, dest_port),
                    });
                    return;
                }
            }
        };

        // Remove from routing graph
        {
            let mut graph = routing_graph.write().unwrap();
            let connection = super::graph::Connection::new(source_id, dest_id);
            if graph.remove_connection(&connection) {
                // Successfully removed from graph
                // Now remove the Link object from thread-local storage
                thread_local! {
                    static LINKS: RefCell<HashMap<u32, Link>> = RefCell::new(HashMap::new());
                    static CONNECTION_TO_LINK: RefCell<HashMap<(PortId, PortId), u32>> = RefCell::new(HashMap::new());
                }

                CONNECTION_TO_LINK.with(|conn_map| {
                    if let Some(link_id) = conn_map.borrow_mut().remove(&(source_id, dest_id)) {
                        LINKS.with(|links| {
                            // Removing the link from storage drops it, destroying the connection
                            links.borrow_mut().remove(&link_id);
                        });
                    }
                });

                let _ = event_tx.send(AudioEvent::ConnectionBroken {
                    source: source_port.to_string(),
                    destination: dest_port.to_string(),
                });
            } else {
                let _ = event_tx.send(AudioEvent::Error {
                    message: format!("Connection not found: {} -> {}", source_port, dest_port),
                });
            }
        }
    }

    /// Handle start visualization command - create an audio capture stream
    fn handle_start_visualization_command(
        core: &pipewire::core::CoreRc,
        _routing_graph: &Arc<RwLock<RoutingGraph>>,
        pw_node_map: &Arc<RwLock<HashMap<u32, DeviceId>>>,
        event_tx: &Sender<AudioEvent>,
        device_id: DeviceId,
        port_id: PortId,
    ) {
        crate::debug_log!("[DEBUG] Start visualization: device_id={:?}, port_id={:?}", device_id, port_id);

        // Find the PipeWire node ID for this device
        let pw_node_id = {
            let node_map = pw_node_map.read().unwrap();
            crate::debug_log!("[DEBUG] Node map contains {} entries", node_map.len());
            // Find the entry where the value matches our device_id
            let found = node_map.iter()
                .find(|&(_, &dev_id)| dev_id == device_id)
                .map(|(&pw_id, _)| pw_id);
            crate::debug_log!("[DEBUG] Found PipeWire node ID: {:?}", found);
            found
        };

        if pw_node_id.is_none() {
            crate::debug_log!("[ERROR] PipeWire node ID not found for device {:?}", device_id);
            let _ = event_tx.send(AudioEvent::Error {
                message: format!("PipeWire node ID not found for device {:?}", device_id),
            });
            return;
        }

        // Create the audio capture stream
        crate::debug_log!("[DEBUG] Creating AudioCaptureStream with node_id={:?}", pw_node_id);
        match AudioCaptureStream::new(
            core,
            device_id,
            port_id,
            pw_node_id,
            event_tx.clone(),
        ) {
            Ok(stream) => {
                crate::debug_log!("[DEBUG] AudioCaptureStream created successfully for device {:?}", device_id);
                // Store the stream in the SAME thread-local storage that the timer uses (line 184)
                CAPTURE_STREAMS.with(|streams| {
                    let mut streams_mut = streams.borrow_mut();
                    streams_mut.insert(device_id, stream);
                    let count = streams_mut.len();
                    crate::debug_log!("[DEBUG] Stream stored in thread-local storage. Total streams: {}", count);
                });
            }
            Err(e) => {
                crate::debug_log!("[ERROR] Failed to create stream: {}", e);
                let _ = event_tx.send(AudioEvent::Error {
                    message: format!(
                        "Failed to create visualization stream for {:?}: {}",
                        device_id, e
                    ),
                });
            }
        }
    }

    /// Handle stop visualization command - destroy an audio capture stream
    fn handle_stop_visualization_command(event_tx: &Sender<AudioEvent>, device_id: DeviceId) {
        // Use the CAPTURE_STREAMS from the outer scope (line 184)
        CAPTURE_STREAMS.with(|streams| {
            if let Some(_stream) = streams.borrow_mut().remove(&device_id) {
                // Stream dropped, PipeWire will clean up
                let _ = event_tx.send(AudioEvent::VisualizationStopped { device_id });
                //println!("Visualization stream stopped for device {:?}", device_id);
            } else {
                let _ = event_tx.send(AudioEvent::Error {
                    message: format!("No visualization stream found for device {:?}", device_id),
                });
            }
        });
    }

    /// Get a reference to the routing graph
    pub fn routing_graph(&self) -> &Arc<RwLock<RoutingGraph>> {
        &self.routing_graph
    }

    /// Create a new virtual device
    pub fn create_virtual_device(
        &mut self,
        name: String,
        num_inputs: usize,
        num_outputs: usize,
    ) -> Result<DeviceId> {
        if !self.is_activated {
            anyhow::bail!("PipeWire client not activated");
        }

        // Generate device ID
        let device_id = {
            let mut graph = self.routing_graph.write().unwrap();
            graph.generate_device_id()
        };

        // Create the virtual device
        let virtual_device = VirtualDevice::new(device_id, name.clone(), num_inputs, num_outputs)?;

        // Add to routing graph
        {
            let mut graph = self.routing_graph.write().unwrap();
            let mut device_info = DeviceInfo::new(device_id, name.clone(), DeviceType::Virtual);

            // Add input ports to device info
            for i in 0..num_inputs {
                let port_id = graph.generate_port_id();
                let port_name = format!("input_{}", i);
                let pw_port_name = format!("wavewire_virtual_{}:{}", name, port_name);
                device_info.ports.push(PortInfo::new(
                    port_id,
                    port_name,
                    PortDirection::Input,
                    pw_port_name,
                ));
            }

            // Add output ports to device info
            for i in 0..num_outputs {
                let port_id = graph.generate_port_id();
                let port_name = format!("output_{}", i);
                let pw_port_name = format!("wavewire_virtual_{}:{}", name, port_name);
                device_info.ports.push(PortInfo::new(
                    port_id,
                    port_name,
                    PortDirection::Output,
                    pw_port_name,
                ));
            }

            graph.add_device(device_info);
        }

        // Store the virtual device
        {
            let mut virtual_devices = self.virtual_devices.write().unwrap();
            virtual_devices.insert(device_id, virtual_device);
        }

        Ok(device_id)
    }

    /// Destroy a virtual device
    pub fn destroy_virtual_device(&mut self, device_id: DeviceId) -> Result<()> {
        // Remove from virtual devices map
        {
            let mut virtual_devices = self.virtual_devices.write().unwrap();
            virtual_devices
                .remove(&device_id)
                .context("Virtual device not found")?;
        }

        // Remove from routing graph
        {
            let mut graph = self.routing_graph.write().unwrap();
            graph.remove_device(device_id);
        }

        Ok(())
    }
}

impl Drop for PipeWireClient {
    fn drop(&mut self) {
        // Ensure clean shutdown
        let _ = self.deactivate();
    }
}
