use anyhow::Result;
use ratatui::{
    backend::TermionBackend,
    Terminal,
};
use std::io;
use std::time::Duration;
use termion::{
    event::Key,
    input::TermRead,
    raw::IntoRawMode,
};

mod audio;
mod ui;

use audio::AudioEngine;
use ui::App;

fn main() -> Result<()> {
    // Initialize terminal
    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Initialize audio engine
    let mut audio_engine = AudioEngine::new()?;
    audio_engine.start()?;

    // Initialize UI app
    let mut app = App::new();

    // Get stdin for input handling
    let stdin = io::stdin();
    let mut keys = stdin.keys();

    // Main application loop
    while app.running {
        // Render UI
        terminal.draw(|frame| {
            app.render(frame);
        })?;

        // Handle input (non-blocking)
        if let Some(Ok(key)) = keys.next() {
            match key {
                Key::Char('q') | Key::Esc => {
                    app.running = false;
                }
                _ => {}
            }
        }

        // Small sleep to prevent busy-waiting
        std::thread::sleep(Duration::from_millis(16)); // ~60 FPS
    }

    // Cleanup
    audio_engine.stop()?;
    terminal.clear()?;

    Ok(())
}
