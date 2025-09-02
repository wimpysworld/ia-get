//! Configuration management for ia-get
//!
//! Handles loading, saving, and managing user configurations with support for
//! both CLI and future GUI integration.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::{error::IaGetError, Result};

/// Application configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default download directory
    pub default_output_path: Option<String>,

    /// Maximum concurrent downloads
    pub concurrent_downloads: usize,

    /// Maximum retry attempts
    pub max_retries: usize,

    /// Default file extensions to include (comma-separated)
    pub default_include_ext: Option<String>,

    /// Default file extensions to exclude (comma-separated)
    pub default_exclude_ext: Option<String>,

    /// Default minimum file size filter
    pub default_min_file_size: Option<String>,

    /// Default maximum file size filter
    pub default_max_file_size: Option<String>,

    /// Whether to resume downloads by default
    pub default_resume: bool,

    /// Whether to enable verbose output by default
    pub default_verbose: bool,

    /// Whether to log hash errors by default
    pub default_log_hash_errors: bool,

    /// Whether to enable dry run by default
    pub default_dry_run: bool,

    /// Whether to enable HTTP compression by default
    pub default_compress: bool,

    /// Whether to enable auto decompression by default
    pub default_decompress: bool,

    /// Default compression formats to decompress (comma-separated)
    pub default_decompress_formats: Option<String>,

    /// HTTP timeout in seconds
    pub http_timeout: u64,

    /// User agent string override
    pub user_agent_override: Option<String>,

    /// Recently used archive URLs (for quick access)
    pub recent_urls: Vec<String>,

    /// Maximum number of recent URLs to keep
    pub max_recent_urls: usize,

    /// Saved filter presets
    pub filter_presets: Vec<FilterPreset>,
}

/// Filter preset for quick configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    pub description: String,
    pub include_ext: Option<String>,
    pub exclude_ext: Option<String>,
    pub max_file_size: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_output_path: None,
            concurrent_downloads: 3, // Conservative default following Archive.org recommendations
            max_retries: 3,
            default_include_ext: None,
            default_exclude_ext: None,
            default_min_file_size: None,
            default_max_file_size: None,
            default_resume: false,
            default_verbose: false,
            default_log_hash_errors: false,
            default_dry_run: false,
            default_compress: true, // Enable compression by default as in CLI
            default_decompress: false,
            default_decompress_formats: None,
            http_timeout: 30,
            user_agent_override: None,
            recent_urls: Vec::new(),
            max_recent_urls: 10,
            filter_presets: vec![
                FilterPreset {
                    name: "Documents".to_string(),
                    description: "Common document formats".to_string(),
                    include_ext: Some("pdf,doc,docx,txt,rtf,odt".to_string()),
                    exclude_ext: None,
                    max_file_size: Some("100MB".to_string()),
                },
                FilterPreset {
                    name: "Images".to_string(),
                    description: "Image files only".to_string(),
                    include_ext: Some("jpg,jpeg,png,gif,bmp,tiff,webp".to_string()),
                    exclude_ext: None,
                    max_file_size: Some("50MB".to_string()),
                },
                FilterPreset {
                    name: "Audio".to_string(),
                    description: "Audio files only".to_string(),
                    include_ext: Some("mp3,flac,wav,ogg,m4a,aac".to_string()),
                    exclude_ext: None,
                    max_file_size: Some("500MB".to_string()),
                },
                FilterPreset {
                    name: "Small Files".to_string(),
                    description: "Files under 10MB".to_string(),
                    include_ext: None,
                    exclude_ext: Some("avi,mkv,mp4,mov,wmv,iso,dmg".to_string()),
                    max_file_size: Some("10MB".to_string()),
                },
            ],
        }
    }
}

/// Configuration manager with file I/O and validation
pub struct ConfigManager {
    config_dir: PathBuf,
    config_file: PathBuf,
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new() -> Result<Self> {
        let config_dir = Self::get_config_directory()?;
        let config_file = config_dir.join("config.toml");

        Ok(Self {
            config_dir,
            config_file,
        })
    }

    /// Get the application configuration directory
    pub fn get_config_directory() -> Result<PathBuf> {
        let config_dir = if cfg!(windows) {
            // Windows: %APPDATA%\ia-get
            std::env::var("APPDATA")
                .map_err(|_| {
                    IaGetError::Config("APPDATA environment variable not found".to_string())
                })?
                .into()
        } else {
            // Unix-like: ~/.config/ia-get
            let home = std::env::var("HOME").map_err(|_| {
                IaGetError::Config("HOME environment variable not found".to_string())
            })?;
            PathBuf::from(home).join(".config")
        };

        let ia_get_config_dir = config_dir.join("ia-get");

        // Create directory if it doesn't exist
        if !ia_get_config_dir.exists() {
            fs::create_dir_all(&ia_get_config_dir).map_err(|e| {
                IaGetError::Config(format!("Failed to create config directory: {}", e))
            })?;
        }

        Ok(ia_get_config_dir)
    }

    /// Load configuration from file or create default
    pub fn load_config(&self) -> Result<Config> {
        if self.config_file.exists() {
            let content = fs::read_to_string(&self.config_file)
                .map_err(|e| IaGetError::Config(format!("Failed to read config file: {}", e)))?;

            toml::from_str(&content)
                .map_err(|e| IaGetError::Config(format!("Failed to parse config file: {}", e)))
        } else {
            // Create default config and save it
            let config = Config::default();
            self.save_config(&config)?;
            Ok(config)
        }
    }

    /// Save configuration to file
    pub fn save_config(&self, config: &Config) -> Result<()> {
        let content = toml::to_string_pretty(config)
            .map_err(|e| IaGetError::Config(format!("Failed to serialize config: {}", e)))?;

        fs::write(&self.config_file, content)
            .map_err(|e| IaGetError::Config(format!("Failed to write config file: {}", e)))?;

        Ok(())
    }

    /// Get the path to the configuration file
    pub fn config_file_path(&self) -> &Path {
        &self.config_file
    }

    /// Get the configuration directory path
    pub fn config_directory(&self) -> &Path {
        &self.config_dir
    }

    /// Add a URL to recent URLs list
    pub fn add_recent_url(&self, config: &mut Config, url: String) {
        // Remove if already exists
        config.recent_urls.retain(|u| u != &url);

        // Add to front
        config.recent_urls.insert(0, url);

        // Trim to max size
        config.recent_urls.truncate(config.max_recent_urls);
    }

    /// Validate configuration values
    pub fn validate_config(config: &Config) -> Result<()> {
        if config.concurrent_downloads == 0 || config.concurrent_downloads > 20 {
            return Err(IaGetError::Config(
                "Concurrent downloads must be between 1 and 20".to_string(),
            ));
        }

        if config.max_retries > 10 {
            return Err(IaGetError::Config(
                "Max retries cannot exceed 10".to_string(),
            ));
        }

        if config.http_timeout == 0 || config.http_timeout > 300 {
            return Err(IaGetError::Config(
                "HTTP timeout must be between 1 and 300 seconds".to_string(),
            ));
        }

        if config.max_recent_urls > 100 {
            return Err(IaGetError::Config(
                "Max recent URLs cannot exceed 100".to_string(),
            ));
        }

        // Validate filter presets
        for preset in &config.filter_presets {
            if preset.name.is_empty() {
                return Err(IaGetError::Config(
                    "Filter preset names cannot be empty".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Create a backup of the current configuration
    pub fn backup_config(&self) -> Result<PathBuf> {
        if !self.config_file.exists() {
            return Err(IaGetError::Config("No config file to backup".to_string()));
        }

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_file = self
            .config_dir
            .join(format!("config_backup_{}.toml", timestamp));

        fs::copy(&self.config_file, &backup_file)
            .map_err(|e| IaGetError::Config(format!("Failed to create backup: {}", e)))?;

        Ok(backup_file)
    }

    /// List available configuration backups
    pub fn list_backups(&self) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(&self.config_dir)
            .map_err(|e| IaGetError::Config(format!("Failed to read config directory: {}", e)))?;

        let mut backups = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| {
                IaGetError::Config(format!("Failed to read directory entry: {}", e))
            })?;
            let path = entry.path();

            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with("config_backup_") && file_name.ends_with(".toml") {
                    backups.push(path);
                }
            }
        }

        backups.sort();
        Ok(backups)
    }
}

/// Configuration endpoints for external access (future GUI integration)
pub mod endpoints {
    use super::*;

    /// Configuration endpoint result
    #[derive(Debug, Serialize)]
    pub struct ConfigResponse {
        pub success: bool,
        pub message: String,
        pub config: Option<Config>,
    }

    /// Get current configuration
    pub fn get_config() -> ConfigResponse {
        match ConfigManager::new().and_then(|manager| manager.load_config()) {
            Ok(config) => ConfigResponse {
                success: true,
                message: "Configuration loaded successfully".to_string(),
                config: Some(config),
            },
            Err(e) => ConfigResponse {
                success: false,
                message: format!("Failed to load configuration: {}", e),
                config: None,
            },
        }
    }

    /// Update configuration
    pub fn update_config(config: Config) -> ConfigResponse {
        match ConfigManager::validate_config(&config) {
            Ok(()) => match ConfigManager::new().and_then(|manager| manager.save_config(&config)) {
                Ok(()) => ConfigResponse {
                    success: true,
                    message: "Configuration saved successfully".to_string(),
                    config: Some(config),
                },
                Err(e) => ConfigResponse {
                    success: false,
                    message: format!("Failed to save configuration: {}", e),
                    config: None,
                },
            },
            Err(e) => ConfigResponse {
                success: false,
                message: format!("Configuration validation failed: {}", e),
                config: None,
            },
        }
    }

    /// Reset configuration to defaults
    pub fn reset_config() -> ConfigResponse {
        let config = Config::default();
        update_config(config)
    }

    /// Get available filter presets
    pub fn get_filter_presets() -> Vec<FilterPreset> {
        get_config()
            .config
            .map(|c| c.filter_presets)
            .unwrap_or_default()
    }

    /// Get recent URLs
    pub fn get_recent_urls() -> Vec<String> {
        get_config()
            .config
            .map(|c| c.recent_urls)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.concurrent_downloads, 3);
        assert_eq!(config.max_retries, 3);
        assert!(!config.filter_presets.is_empty());
    }

    #[test]
    fn test_config_validation() {
        let mut config = Config::default();
        assert!(ConfigManager::validate_config(&config).is_ok());

        config.concurrent_downloads = 0;
        assert!(ConfigManager::validate_config(&config).is_err());

        config.concurrent_downloads = 25;
        assert!(ConfigManager::validate_config(&config).is_err());
    }

    #[test]
    fn test_filter_presets() {
        let config = Config::default();
        assert!(config.filter_presets.iter().any(|p| p.name == "Documents"));
        assert!(config.filter_presets.iter().any(|p| p.name == "Images"));
    }
}
