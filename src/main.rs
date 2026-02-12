use anyhow::Result;
use ratatui::{backend::TermionBackend, Terminal};
use std::io;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use termion::{async_stdin, event::Key, input::TermRead, raw::IntoRawMode};

mod audio;
mod ui;
mod debug_log;

use audio::{AudioEngine, AudioEvent};
use ui::App;

/// Target frames per second for the UI
const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / TARGET_FPS);

fn main() -> Result<()> {
    // Run the application and get the exit status
    let result = run_app();

    // Force exit to avoid waiting for background threads
    // (PipeWire event loop thread can't be gracefully shut down with MainLoopRc)
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_app() -> Result<()> {
    // Initialize debug logging
    debug_log::init_log();
    debug_log!("Application starting");

    // Initialize terminal
    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    // Initialize audio engine
    let mut audio_engine = AudioEngine::new()?;
    audio_engine.start()?;

    // Initialize UI app
    let mut app = App::new();

    // Set up non-blocking input handling
    let input_rx = spawn_input_thread();

    // Track frame timing
    let mut last_frame = Instant::now();

    // Main application loop
    while app.running {
        let now = Instant::now();
        let elapsed = now.duration_since(last_frame);

        // Poll audio events and update app state
        let audio_events = audio_engine.poll_events();
        let has_device_events = audio_events.iter().any(|e| {
            matches!(e, AudioEvent::DeviceAdded { .. } | AudioEvent::DeviceRemoved { .. })
        });
        app.handle_audio_events(&audio_events);

        // Refresh device list if device events occurred
        if has_device_events {
            let _ = app.refresh_devices(&audio_engine);
        }

        // Handle keyboard input
        loop {
            match input_rx.try_recv() {
                Ok(key) => {
                    app.handle_input(key, &mut audio_engine)?;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    eprintln!("Input thread disconnected");
                    app.running = false;
                    break;
                }
            }
        }

        // Render UI if enough time has passed
        if elapsed >= FRAME_DURATION {
            terminal.draw(|frame| {
                app.render(frame, &audio_engine);
            })?;
            last_frame = now;
        } else {
            // Sleep for remaining time to target FPS
            let sleep_time = FRAME_DURATION.saturating_sub(elapsed);
            if sleep_time > Duration::from_millis(1) {
                thread::sleep(sleep_time);
            }
        }
    }

    // Cleanup - restore terminal to normal mode
    terminal.show_cursor()?;
    terminal.clear()?;

    // Stop audio engine
    audio_engine.stop()?;

    // Explicitly drop terminal to restore terminal settings
    // This is critical - it runs the Drop handler that restores from raw mode
    drop(terminal);

    Ok(())
}

/// Spawn a thread to handle keyboard input asynchronously
fn spawn_input_thread() -> Receiver<Key> {
    let (tx, rx) = channel();

    thread::spawn(move || {
        let mut stdin = async_stdin().keys();
        loop {
            if let Some(Ok(key)) = stdin.next() {
                if tx.send(key).is_err() {
                    // Main thread has dropped the receiver, exit
                    break;
                }
            }
            // Small sleep to prevent busy-waiting
            thread::sleep(Duration::from_millis(10));
        }
    });

    rx
}
