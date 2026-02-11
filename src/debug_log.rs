use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

/// Simple file logger for debugging
/// Logs to wavewire-debug.log in the current directory
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Initialize the debug log file
pub fn init_log() {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("wavewire-debug.log")
        .expect("Failed to open debug log file");

    *LOG_FILE.lock().unwrap() = Some(file);
    log("=== Wavewire Debug Log Started ===");
}

/// Write a line to the debug log
pub fn log(msg: &str) {
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(file) = guard.as_mut() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let _ = writeln!(file, "[{}] {}", timestamp, msg);
        }
    }
}

/// Log with formatting
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        $crate::debug_log::log(&format!($($arg)*))
    };
}
