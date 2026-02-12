use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
};
use termion::event::Key;

use crate::audio::{AudioEngine, AudioEvent, DeviceInfo};
use crate::audio::{DeviceId, SpectrumData};
use std::collections::{HashMap, HashSet};

/// Minimum terminal height for full layout (with device list and tabs)
/// Below this threshold, only spectrum is displayed
const MIN_HEIGHT_FOR_FULL_LAYOUT: u16 = 24;

/// Height reserved for the spectrum visualization in full layout
const SPECTRUM_HEIGHT: u16 = 24;

/// Width of the left navigation panel (device list)
const DEVICE_LIST_WIDTH: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTab {
    Routing,
    Filters,
}

impl DeviceTab {
    pub fn title(&self) -> &str {
        match self {
            DeviceTab::Routing => "Routing",
            DeviceTab::Filters => "Filters",
        }
    }

    pub fn all() -> Vec<DeviceTab> {
        vec![DeviceTab::Routing, DeviceTab::Filters]
    }

    pub fn next(&self) -> DeviceTab {
        match self {
            DeviceTab::Routing => DeviceTab::Filters,
            DeviceTab::Filters => DeviceTab::Routing,
        }
    }

    pub fn previous(&self) -> DeviceTab {
        match self {
            DeviceTab::Routing => DeviceTab::Filters,
            DeviceTab::Filters => DeviceTab::Routing,
        }
    }
}

pub struct App {
    pub running: bool,
    current_tab: DeviceTab,
    devices: Vec<DeviceInfo>,
    selected_device: usize,
    status_message: String,
    /// Devices currently being visualized
    visualized_devices: HashSet<DeviceId>,
    /// Latest spectrum data per device
    spectrum_data: HashMap<DeviceId, SpectrumData>,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            current_tab: DeviceTab::Routing,
            devices: Vec::new(),
            selected_device: 0,
            status_message: String::from("Starting up..."),
            visualized_devices: HashSet::new(),
            spectrum_data: HashMap::new(),
        }
    }

    pub fn handle_input(&mut self, key: Key, audio_engine: &mut AudioEngine) -> Result<()> {
        match key {
            // Global keys
            Key::Char('q') | Key::Esc | Key::Ctrl('c') => {
                self.running = false;
            }
            Key::Char('\t') => {
                // Cycle through device tabs
                self.current_tab = self.current_tab.next();
            }
            Key::BackTab => {
                // Cycle through device tabs in reverse
                self.current_tab = self.current_tab.previous();
            }
            Key::Up | Key::Char('k') => {
                // Navigate device list
                if self.selected_device > 0 {
                    self.selected_device -= 1;
                }
            }
            Key::Down | Key::Char('j') => {
                // Navigate device list
                if !self.devices.is_empty() && self.selected_device + 1 < self.devices.len() {
                    self.selected_device += 1;
                }
            }
            Key::Char('r') => {
                // Refresh device list
                self.refresh_devices(audio_engine)?;
                self.status_message = String::from("Refreshed device list");
            }
            Key::Char('n') => {
                // Create new virtual device (placeholder)
                self.status_message = String::from("Virtual device creation not yet implemented");
            }
            Key::Char(' ') => {
                // Toggle visualization for selected device
                self.toggle_visualization(audio_engine)?;
            }
            // Tab-specific input handling
            _ => {
                self.handle_tab_input(key, audio_engine)?;
            }
        }

        Ok(())
    }

    fn handle_tab_input(&mut self, key: Key, _audio_engine: &mut AudioEngine) -> Result<()> {
        match self.current_tab {
            DeviceTab::Routing => {
                // Placeholder for routing tab input
                match key {
                    _ => {}
                }
            }
            DeviceTab::Filters => {
                // Placeholder for filters tab input
                match key {
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn handle_audio_events(&mut self, events: &[AudioEvent]) {
        for event in events {
            match event {
                AudioEvent::DeviceAdded {
                    device_id: _,
                    name,
                    device_type,
                } => {
                    self.status_message = format!("Device added: {} ({:?})", name, device_type);
                }
                AudioEvent::DeviceRemoved { device_id } => {
                    self.status_message = format!("Device removed: {:?}", device_id);
                }
                AudioEvent::ConnectionEstablished {
                    source,
                    destination,
                } => {
                    self.status_message = format!("Connected: {} -> {}", source, destination);
                }
                AudioEvent::ConnectionBroken {
                    source,
                    destination,
                } => {
                    self.status_message = format!("Disconnected: {} -> {}", source, destination);
                }
                AudioEvent::Xrun => {
                    self.status_message = String::from("Audio buffer xrun occurred");
                }
                AudioEvent::Error { message } => {
                    self.status_message = format!("Error: {}", message);
                }
                AudioEvent::VisualizationStarted { device_id, port_id } => {
                    self.visualized_devices.insert(*device_id);
                    self.status_message = format!(
                        "Visualization started for device {:?}, port {:?}",
                        device_id, port_id
                    );
                }
                AudioEvent::VisualizationStopped { device_id } => {
                    self.visualized_devices.remove(device_id);
                    self.spectrum_data.remove(device_id);
                    self.status_message =
                        format!("Visualization stopped for device {:?}", device_id);
                }
                AudioEvent::SpectrumUpdate { device_id, data } => {
                    crate::debug_log!(
                        "[UI] SpectrumUpdate received: device={:?}, bins={}, samples=[{:.2}, {:.2}, {:.2}]",
                        device_id,
                        data.bins.len(),
                        data.bins.get(0).unwrap_or(&-60.0),
                        data.bins.get(32).unwrap_or(&-60.0),
                        data.bins.get(63).unwrap_or(&-60.0)
                    );
                    self.spectrum_data.insert(*device_id, data.clone());
                }
            }
        }
    }

    pub fn refresh_devices(&mut self, audio_engine: &AudioEngine) -> Result<()> {
        self.devices = audio_engine.list_devices()?;
        if self.selected_device >= self.devices.len() && !self.devices.is_empty() {
            self.selected_device = self.devices.len() - 1;
        }
        Ok(())
    }

    fn toggle_visualization(&mut self, audio_engine: &AudioEngine) -> Result<()> {
        if self.devices.is_empty() {
            self.status_message = String::from("No devices available");
            return Ok(());
        }

        let device = &self.devices[self.selected_device];
        let device_id = device.id;

        if self.visualized_devices.contains(&device_id) {
            // Stop visualization
            use crate::audio::AudioCommand;
            audio_engine.send_command(AudioCommand::StopVisualization { device_id })?;
            self.status_message = format!("Stopping visualization for {}", device.name);
        } else {
            // Start visualization - pick the first output port (monitor port if available)
            let port_to_visualize = device.ports.iter().find(|p| {
                use crate::audio::PortDirection;
                p.direction == PortDirection::Output
            });

            if let Some(port) = port_to_visualize {
                use crate::audio::AudioCommand;
                audio_engine.send_command(AudioCommand::StartVisualization {
                    device_id,
                    port_id: port.id,
                })?;
                self.status_message = format!("Starting visualization for {}", device.name);
            } else {
                self.status_message = format!("No output port found for {}", device.name);
            }
        }

        Ok(())
    }

    pub fn render(&mut self, frame: &mut Frame, audio_engine: &AudioEngine) {
        // Note: Device list is refreshed via events and manual refresh ('r' key), not on every render

        let terminal_height = frame.area().height;

        // Responsive layout based on terminal height
        if terminal_height < MIN_HEIGHT_FOR_FULL_LAYOUT {
            // Minimal view: spectrum only
            self.render_minimal_layout(frame);
        } else {
            // Full view: device list + tabs + spectrum
            self.render_full_layout(frame);
        }
    }

    fn render_minimal_layout(&self, frame: &mut Frame) {
        // In minimal mode, just show spectrum filling the entire screen
        self.render_spectrum(frame, frame.area(), false);
    }

    fn render_full_layout(&mut self, frame: &mut Frame) {
        // Main vertical split: content area + spectrum at bottom + status bar
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),                  // Content area (device list + main content)
                Constraint::Length(SPECTRUM_HEIGHT), // Spectrum visualization
                Constraint::Length(3),               // Status bar
            ])
            .split(frame.area());

        // Horizontal split: device list on left + main content on right
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(DEVICE_LIST_WIDTH), // Device list
                Constraint::Min(0),                    // Main content area
            ])
            .split(main_chunks[0]);

        // Render device list on the left
        self.render_device_list(frame, content_chunks[0]);

        // Render main content area on the right (with tabs)
        self.render_main_content(frame, content_chunks[1]);

        // Render spectrum at the bottom
        self.render_spectrum(frame, main_chunks[1], true);

        // Render status bar at the very bottom
        self.render_status_bar(frame, main_chunks[2]);
    }

    fn render_device_list(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .devices
            .iter()
            .map(|device| {
                let device_type = format!("{:?}", device.device_type);
                let is_visualized = self.visualized_devices.contains(&device.id);
                let indicator = if is_visualized { "[x]" } else { "[ ]" };
                let line = Line::from(vec![
                    Span::styled(
                        indicator,
                        Style::default().fg(if is_visualized {
                            Color::Cyan
                        } else {
                            Color::DarkGray
                        }),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        &device.name,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("({})", device_type),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Devices")
                    .title_alignment(Alignment::Left),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        frame.render_stateful_widget(
            list,
            area,
            &mut ratatui::widgets::ListState::default().with_selected(Some(self.selected_device)),
        );
    }

    fn render_main_content(&self, frame: &mut Frame, area: Rect) {
        // Split main content into tab bar and content area
        let content_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Tab bar
                Constraint::Min(0),    // Tab content
            ])
            .split(area);

        // Render tab bar
        self.render_tabs(frame, content_chunks[0]);

        // Render current tab content
        match self.current_tab {
            DeviceTab::Routing => self.render_routing_tab(frame, content_chunks[1]),
            DeviceTab::Filters => self.render_filters_tab(frame, content_chunks[1]),
        }
    }

    fn render_tabs(&self, frame: &mut Frame, area: Rect) {
        let tabs = DeviceTab::all();
        let titles: Vec<_> = tabs.iter().map(|t| t.title()).collect();

        let selected_index = match self.current_tab {
            DeviceTab::Routing => 0,
            DeviceTab::Filters => 1,
        };

        let device_name = if !self.devices.is_empty() && self.selected_device < self.devices.len() {
            &self.devices[self.selected_device].name
        } else {
            "No device selected"
        };

        let tabs_widget = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("wavewire - {}", device_name)),
            )
            .select(selected_index)
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(tabs_widget, area);
    }

    fn render_routing_tab(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Routing")
            .title_alignment(Alignment::Left);

        let content = if self.devices.is_empty() {
            "No devices available\n\nPress 'r' to refresh device list"
        } else {
            "Routing configuration\n\nConfigure input/output connections for this device"
        };

        let paragraph = Paragraph::new(content)
            .block(block)
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn render_filters_tab(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Filters")
            .title_alignment(Alignment::Left);

        let content = if self.devices.is_empty() {
            "No devices available\n\nPress 'r' to refresh device list"
        } else {
            "Audio filters and processing\n\nConfigure DSP filters for this device"
        };

        let paragraph = Paragraph::new(content)
            .block(block)
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn render_spectrum(&self, frame: &mut Frame, area: Rect, show_borders: bool) {
        // If no devices are being visualized, show a message
        if self.visualized_devices.is_empty() {
            if show_borders {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Frequency Spectrum - Combined View")
                    .title_alignment(Alignment::Left);

                let paragraph = Paragraph::new(
                    "No devices visualized\n\nPress Space on a device to start visualization",
                )
                .block(block)
                .alignment(Alignment::Center);

                frame.render_widget(paragraph, area);
            } else {
                // In minimal mode, just show a simple message without borders
                let paragraph = Paragraph::new(
                    "No devices visualized\n\nPress Space on a device to start visualization",
                )
                .alignment(Alignment::Center);

                frame.render_widget(paragraph, area);
            }
            return;
        }

        // Get all visualized devices in a consistent order
        let mut device_ids: Vec<DeviceId> = self.visualized_devices.iter().copied().collect();
        device_ids.sort_by_key(|id| id.0); // Sort for consistent ordering

        // Check if we have spectrum data for any device
        let has_data = device_ids
            .iter()
            .any(|id| self.spectrum_data.contains_key(id));
        if !has_data {
            if show_borders {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Frequency Spectrum - Waiting for data...")
                    .title_alignment(Alignment::Left);

                frame.render_widget(block, area);
            }
            return;
        }

        // Build the title with device names and colors
        let device_info: Vec<String> = device_ids
            .iter()
            .enumerate()
            .map(|(idx, &device_id)| {
                let name = self
                    .devices
                    .iter()
                    .find(|d| d.id == device_id)
                    .map(|d| d.name.as_str())
                    .unwrap_or("Unknown");
                let color_name = Self::get_device_color_name(idx);
                format!("[{}] {}", color_name, name)
            })
            .collect();

        let title = format!("Frequency Spectrum - {}", device_info.join(" | "));

        // Render the combined spectrum
        self.render_combined_spectrum(frame, area, &title, &device_ids, show_borders);
    }

    fn get_device_color(idx: usize) -> Color {
        match idx % 6 {
            0 => Color::Cyan,
            1 => Color::Yellow,
            2 => Color::Magenta,
            3 => Color::Green,
            4 => Color::Red,
            5 => Color::Blue,
            _ => Color::White,
        }
    }

    fn get_device_color_name(idx: usize) -> &'static str {
        match idx % 6 {
            0 => "C", // Cyan
            1 => "Y", // Yellow
            2 => "M", // Magenta
            3 => "G", // Green
            4 => "R", // Red
            5 => "B", // Blue
            _ => "W",
        }
    }

    fn render_combined_spectrum(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        device_ids: &[DeviceId],
        show_borders: bool,
    ) {
        let num_devices = device_ids.len();

        // Get first device's spectrum to determine number of bins
        let first_spectrum = device_ids
            .iter()
            .find_map(|id| self.spectrum_data.get(id))
            .unwrap();

        let total_bins = first_spectrum.bins.len();

        // Calculate the actual inner width the same way render_custom_bars does
        let available_width = if show_borders {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_alignment(Alignment::Left);
            block.inner(area).width as usize
        } else {
            area.width as usize
        };

        if available_width == 0 {
            return;
        }

        let bars_per_group = num_devices;

        // Calculate how many frequency groups we can show and how many times to repeat each
        let max_possible_groups = available_width / bars_per_group;
        let num_frequency_groups = max_possible_groups.min(total_bins);

        if num_frequency_groups == 0 {
            return;
        }

        // Calculate base repetition per group and extra bars
        let base_repetition = available_width / (num_frequency_groups * bars_per_group);
        let total_with_base = num_frequency_groups * bars_per_group * base_repetition;
        let extra_bars = available_width - total_with_base;

        // Build bar chart data
        let mut bars_data: Vec<(&str, u64)> = Vec::new();
        let mut bar_styles: Vec<Style> = Vec::new();

        // Helper function to get magnitude for a frequency range
        let get_magnitude = |group_idx: usize, device_id: DeviceId| -> f32 {
            let bin_start = (group_idx * total_bins) / num_frequency_groups;
            let bin_end = ((group_idx + 1) * total_bins) / num_frequency_groups;

            if let Some(spectrum) = self.spectrum_data.get(&device_id) {
                let mut max_mag: f32 = -60.0;
                for bin_idx in bin_start..bin_end {
                    if bin_idx < spectrum.bins.len() {
                        max_mag = max_mag.max(spectrum.bins[bin_idx]);
                    }
                }
                max_mag
            } else {
                -60.0
            }
        };

        // Track extra bars distributed
        let mut extra_bars_used = 0;

        // For each frequency group
        for group_idx in 0..num_frequency_groups {
            // Calculate repetitions for this group (distribute extra bars evenly)
            let extra_for_this_group = if extra_bars_used < extra_bars {
                extra_bars_used += 1;
                1
            } else {
                0
            };
            let repetitions_for_this_group = base_repetition + extra_for_this_group;

            // Repeat this frequency group's bars the calculated number of times
            for _rep in 0..repetitions_for_this_group {
                for (device_idx, &device_id) in device_ids.iter().enumerate() {
                    let magnitude = get_magnitude(group_idx, device_id);

                    let normalized = (magnitude + 60.0_f32).max(0.0_f32).min(60.0_f32);
                    let amplified = (normalized * 2.0_f32).min(60.0_f32);
                    let display_value = amplified as u64;

                    bars_data.push(("", display_value));
                    bar_styles.push(Style::default().fg(Self::get_device_color(device_idx)));
                }
            }
        }

        // Debug logging
        crate::debug_log!(
            "[SPECTRUM] area.width={}, available_width={}, bars_per_group={}, num_frequency_groups={}, base_rep={}, extra_bars={}, total_bars={}",
            area.width,
            available_width,
            bars_per_group,
            num_frequency_groups,
            base_repetition,
            extra_bars,
            bars_data.len()
        );

        // We need to render bars with individual colors, but BarChart only has one style
        // Workaround: render the spectrum using custom rendering
        self.render_custom_bars(frame, area, title, &bars_data, &bar_styles, show_borders);
    }

    fn render_custom_bars(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        bars: &[(&str, u64)],
        bar_styles: &[Style],
        show_borders: bool,
    ) {
        let inner = if show_borders {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_alignment(Alignment::Left);

            let inner = block.inner(area);
            frame.render_widget(block, area);
            inner
        } else {
            // In minimal mode, use the full area without borders
            area
        };

        crate::debug_log!(
            "[RENDER_BARS] area.width={}, inner.width={}, bars.len()={}, show_borders={}",
            area.width,
            inner.width,
            bars.len(),
            show_borders
        );

        if bars.is_empty() || inner.width < 2 || inner.height < 3 {
            return;
        }

        // Use full height for bars
        let bar_height_area = inner.height;
        let _label_y = inner.y + bar_height_area;

        // Max height accounts for baseline (5) + amplified range (60)
        let max_height = 65.0; // Baseline (5) + max amplified dB (60)

        // With braille, each terminal row represents 4 vertical pixels
        let braille_pixels_per_row = 4;
        let total_vertical_pixels = bar_height_area as usize * braille_pixels_per_row;

        // Render each bar using braille characters for better vertical resolution
        for (i, ((_label, value), style)) in bars.iter().zip(bar_styles.iter()).enumerate() {
            if i >= inner.width as usize {
                break;
            }

            // Calculate height in braille pixels (4x resolution)
            let bar_height_pixels =
                (*value as f32 / max_height * total_vertical_pixels as f32) as usize;
            let bar_height_pixels = bar_height_pixels.min(total_vertical_pixels).max(1);

            if bar_height_pixels > 0 {
                let x = inner.x + i as u16;

                // Calculate how many full rows and remaining pixels
                let full_rows = bar_height_pixels / braille_pixels_per_row;
                let remaining_pixels = bar_height_pixels % braille_pixels_per_row;

                // Start from bottom
                let bottom_row = inner.y + bar_height_area - 1;

                // Render full rows with full braille character
                for row in 0..full_rows {
                    let y = bottom_row.saturating_sub(row as u16);
                    if y >= inner.y && y < inner.y + bar_height_area {
                        let cell = frame.buffer_mut().cell_mut((x, y)).unwrap();
                        // Full column: left column filled (dots 1,2,3,4)
                        cell.set_symbol(Self::braille_char(0b1111));
                        cell.set_style(*style);
                    }
                }

                // Render partial row at top if needed
                if remaining_pixels > 0 && full_rows < bar_height_area as usize {
                    let y = bottom_row.saturating_sub(full_rows as u16);
                    if y >= inner.y && y < inner.y + bar_height_area {
                        let cell = frame.buffer_mut().cell_mut((x, y)).unwrap();
                        // Partial column: fill from bottom
                        let pattern = match remaining_pixels {
                            1 => 0b0001, // Bottom dot only
                            2 => 0b0011, // Bottom 2 dots
                            3 => 0b0111, // Bottom 3 dots
                            _ => 0b1111, // All dots
                        };
                        cell.set_symbol(Self::braille_char(pattern));
                        cell.set_style(*style);
                    }
                }
            }
        }
    }

    /// Convert a 4-bit pattern to a braille character (both columns filled)
    /// Bit 0 = level 1 (bottom), bit 1 = level 2, bit 2 = level 3, bit 3 = level 4 (top)
    /// Braille layout: 1 4
    ///                 2 5
    ///                 3 6
    ///                 7 8
    fn braille_char(pattern: u8) -> &'static str {
        match pattern {
            0b0000 => "⠀", // blank
            0b0001 => "⣀", // bottom row only (dots 7,8)
            0b0011 => "⣤", // bottom 2 rows (dots 3,6,7,8)
            0b0111 => "⣶", // bottom 3 rows (dots 2,3,5,6,7,8)
            0b1111 => "⣿", // all 4 rows (full block)
            _ => "⠀",      // default to blank for other patterns
        }
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let status_text = vec![Line::from(vec![
            Span::styled(
                "Status: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&self.status_message, Style::default().fg(Color::White)),
            Span::raw("  |  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(": quit  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(": switch tab  "),
            Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
            Span::raw(": select device  "),
            Span::styled("r", Style::default().fg(Color::Cyan)),
            Span::raw(": refresh  "),
            Span::styled("Space", Style::default().fg(Color::Cyan)),
            Span::raw(": toggle viz  "),
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(": new device"),
        ])];

        let paragraph = Paragraph::new(status_text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, area);
    }
}
