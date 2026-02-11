# wavewire

TUI audio visualization, routing, and processing tool

## Stack
- Rust
- PipeWire (via JACK API using `jack` crate)
- TUI: ratatui with termion backend (Unix-only, lightweight)
- Real-time data streams (no buffering)

## Core Features
- Real-time frequency spectrum visualization
- Virtual audio devices (ins/outs)
- Route physical/virtual devices together
- Audio filters (noise gate, EQ, etc.)
- Fast, lightweight, instant startup

## Technical TODO
- Set up dependencies (ratatui, termion, jack)
- JACK client connection and device enumeration
- Graph-based routing system
- Audio buffer management for real-time streaming
- Filter DSP implementations
- Custom audio visualization (FFT/spectrum analysis)

## Open Questions
- How to handle sample rate mismatches between devices
- Session persistence format (save/load routing configs)
- Virtual device creation API design
