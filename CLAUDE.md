# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

wavewire is a TUI-based audio visualization, routing, and processing tool built with Rust. It provides real-time frequency spectrum visualization, virtual audio devices, and a graph-based routing system for connecting physical and virtual audio devices.

## Build Commands

```bash
# Build the project
cargo build

# Build with optimizations
cargo build --release

# Run the application
cargo run

# Run with release optimizations
cargo run --release

# Run tests
cargo test

# Run a specific test
cargo test test_name

# Check code without building
cargo check

# Format code
cargo fmt

# Lint with clippy
cargo clippy
```

## Architecture

### Technology Stack
- **Audio Backend**: PipeWire via JACK API (using `jack` crate)
- **TUI Framework**: ratatui with termion backend (immediate-mode, lightweight)
- **Data Flow**: Real-time streams (no buffering)
- **Visualization**: Custom FFT/spectrum analysis (user-implemented)

### Planned Architecture

**Graph-Based Routing System**: Audio devices (both physical and virtual) will be represented as nodes in a routing graph. Connections between nodes define audio flow paths.

**Virtual Audio Devices**: The application will create virtual input/output devices that can be routed to/from physical devices or other virtual devices.

**Real-Time Audio Processing**: Audio filters (noise gate, EQ, etc.) will be implemented as DSP nodes that can be inserted into routing paths.

**Visualization**: Real-time frequency spectrum visualization using custom FFT analysis on audio streams. Terminal-based rendering via ratatui (immediate-mode) for maximum responsiveness.

### Design Considerations

- **Sample Rate Handling**: Strategy needed for handling sample rate mismatches between devices
- **Session Persistence**: Plan to implement save/load functionality for routing configurations
- **Performance**: Target is instant startup and lightweight operation with real-time audio processing. Using termion for minimal terminal overhead.
