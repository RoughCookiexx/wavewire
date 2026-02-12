use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::audio::{DeviceId, DeviceInfo};
use crate::debug_log;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub visualization: VisualizationConfig,
}

/// Configuration for spectrum visualization
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisualizationConfig {
    /// Device names to visualize (matched by name on restore)
    pub enabled_devices: Vec<String>,
}

impl Config {
    /// Create config from current app state
    /// Extracts device names for all visualized devices
    pub fn from_visualized_devices(
        visualized_ids: &HashSet<DeviceId>,
        all_devices: &[DeviceInfo],
    ) -> Self {
        let enabled_devices = all_devices
            .iter()
            .filter(|d| visualized_ids.contains(&d.id))
            .map(|d| d.name.clone())
            .collect();

        Config {
            visualization: VisualizationConfig { enabled_devices },
        }
    }
}

/// Manages configuration file loading and saving
pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    /// Create a new ConfigManager with XDG-compliant config directory
    pub fn new() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .context("Failed to get config directory")?
            .join("wavewire");

        // Create config directory if it doesn't exist
        fs::create_dir_all(&config_dir)
            .context("Failed to create config directory")?;

        let config_path = config_dir.join("config.toml");
        debug_log!("Config path: {}", config_path.display());

        Ok(Self { config_path })
    }

    /// Load configuration from disk
    /// Returns default config if file doesn't exist or is corrupted
    pub fn load(&self) -> Result<Config> {
        // If config file doesn't exist, return defaults
        if !self.config_path.exists() {
            debug_log!("Config file not found, using defaults");
            return Ok(Config::default());
        }

        // Try to read and parse the config file
        match fs::read_to_string(&self.config_path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    debug_log!("Config loaded successfully");
                    Ok(config)
                }
                Err(e) => {
                    // Config is corrupted, back it up and use defaults
                    debug_log!("Config parse error: {}, backing up and using defaults", e);
                    let backup_path = self.config_path.with_extension("toml.bak");
                    let _ = fs::rename(&self.config_path, &backup_path);
                    Ok(Config::default())
                }
            },
            Err(e) => {
                debug_log!("Failed to read config: {}, using defaults", e);
                Ok(Config::default())
            }
        }
    }

    /// Save configuration to disk
    /// Uses atomic write (write to temp file, then rename)
    pub fn save(&self, config: &Config) -> Result<()> {
        let toml_string = toml::to_string_pretty(config)
            .context("Failed to serialize config")?;

        // Write to temp file first
        let temp_path = self.config_path.with_extension("toml.tmp");
        fs::write(&temp_path, toml_string)
            .context("Failed to write config to temp file")?;

        // Atomic rename
        fs::rename(&temp_path, &self.config_path)
            .context("Failed to rename temp config file")?;

        debug_log!("Config saved successfully");
        Ok(())
    }
}
