//! Configuration persistence with priority handling
//!
//! Manages configuration from multiple sources with proper priority:
//! CLI args > saved preferences/config file > defaults (for CLI)
//! One-time options > saved preferences/config file > defaults (for GUI)

use crate::{error::IaGetError, infrastructure::config::Config, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Source of a configuration value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigSource {
    /// Value from command line arguments (highest priority for CLI)
    CommandLine,
    /// Value from one-time GUI selection (highest priority for GUI)
    GuiOneTime,
    /// Value from saved configuration file
    ConfigFile,
    /// Default value built into the application
    Default,
}

/// Configuration priority manager
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigPriority {
    /// Highest priority - CLI args or GUI one-time
    High = 3,
    /// Medium priority - saved configuration
    Medium = 2,
    /// Low priority - application defaults  
    Low = 1,
}

impl ConfigSource {
    /// Get the priority level for this source in CLI context
    pub fn cli_priority(&self) -> ConfigPriority {
        match self {
            ConfigSource::CommandLine => ConfigPriority::High,
            ConfigSource::ConfigFile => ConfigPriority::Medium,
            ConfigSource::GuiOneTime | ConfigSource::Default => ConfigPriority::Low,
        }
    }

    /// Get the priority level for this source in GUI context
    pub fn gui_priority(&self) -> ConfigPriority {
        match self {
            ConfigSource::GuiOneTime => ConfigPriority::High,
            ConfigSource::ConfigFile => ConfigPriority::Medium,
            ConfigSource::CommandLine | ConfigSource::Default => ConfigPriority::Low,
        }
    }
}

/// A configuration value with its source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValue<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> ConfigValue<T> {
    pub fn new(value: T, source: ConfigSource) -> Self {
        Self { value, source }
    }

    /// Check if this value should override another based on context
    pub fn should_override(&self, other: &Self, is_gui: bool) -> bool {
        let self_priority = if is_gui {
            self.source.gui_priority()
        } else {
            self.source.cli_priority()
        };

        let other_priority = if is_gui {
            other.source.gui_priority()
        } else {
            other.source.cli_priority()
        };

        self_priority > other_priority
    }
}

/// Enhanced configuration that tracks sources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigWithSources {
    pub default_output_path: ConfigValue<Option<String>>,
    pub concurrent_downloads: ConfigValue<usize>,
    pub max_retries: ConfigValue<usize>,
    pub default_include_ext: ConfigValue<Option<String>>,
    pub default_exclude_ext: ConfigValue<Option<String>>,
    pub default_min_file_size: ConfigValue<Option<String>>,
    pub default_max_file_size: ConfigValue<Option<String>>,
    pub default_resume: ConfigValue<bool>,
    pub default_verbose: ConfigValue<bool>,
    pub default_log_hash_errors: ConfigValue<bool>,
    pub default_dry_run: ConfigValue<bool>,
    pub default_compress: ConfigValue<bool>,
    pub default_decompress: ConfigValue<bool>,
    pub default_decompress_formats: ConfigValue<Option<String>>,
    pub http_timeout: ConfigValue<u64>,
    pub user_agent_override: ConfigValue<Option<String>>,
}

impl Default for ConfigWithSources {
    fn default() -> Self {
        let default_config = Config::default();
        Self {
            default_output_path: ConfigValue::new(
                default_config.default_output_path,
                ConfigSource::Default,
            ),
            concurrent_downloads: ConfigValue::new(
                default_config.concurrent_downloads,
                ConfigSource::Default,
            ),
            max_retries: ConfigValue::new(default_config.max_retries, ConfigSource::Default),
            default_include_ext: ConfigValue::new(
                default_config.default_include_ext,
                ConfigSource::Default,
            ),
            default_exclude_ext: ConfigValue::new(
                default_config.default_exclude_ext,
                ConfigSource::Default,
            ),
            default_min_file_size: ConfigValue::new(
                default_config.default_min_file_size,
                ConfigSource::Default,
            ),
            default_max_file_size: ConfigValue::new(
                default_config.default_max_file_size,
                ConfigSource::Default,
            ),
            default_resume: ConfigValue::new(default_config.default_resume, ConfigSource::Default),
            default_verbose: ConfigValue::new(
                default_config.default_verbose,
                ConfigSource::Default,
            ),
            default_log_hash_errors: ConfigValue::new(
                default_config.default_log_hash_errors,
                ConfigSource::Default,
            ),
            default_dry_run: ConfigValue::new(
                default_config.default_dry_run,
                ConfigSource::Default,
            ),
            default_compress: ConfigValue::new(
                default_config.default_compress,
                ConfigSource::Default,
            ),
            default_decompress: ConfigValue::new(
                default_config.default_decompress,
                ConfigSource::Default,
            ),
            default_decompress_formats: ConfigValue::new(
                default_config.default_decompress_formats,
                ConfigSource::Default,
            ),
            http_timeout: ConfigValue::new(default_config.http_timeout, ConfigSource::Default),
            user_agent_override: ConfigValue::new(
                default_config.user_agent_override,
                ConfigSource::Default,
            ),
        }
    }
}

impl ConfigWithSources {
    /// Convert to a regular Config for use in the application
    pub fn to_config(&self) -> Config {
        Config {
            default_output_path: self.default_output_path.value.clone(),
            concurrent_downloads: self.concurrent_downloads.value,
            max_retries: self.max_retries.value,
            default_include_ext: self.default_include_ext.value.clone(),
            default_exclude_ext: self.default_exclude_ext.value.clone(),
            default_min_file_size: self.default_min_file_size.value.clone(),
            default_max_file_size: self.default_max_file_size.value.clone(),
            default_resume: self.default_resume.value,
            default_verbose: self.default_verbose.value,
            default_log_hash_errors: self.default_log_hash_errors.value,
            default_dry_run: self.default_dry_run.value,
            default_compress: self.default_compress.value,
            default_decompress: self.default_decompress.value,
            default_decompress_formats: self.default_decompress_formats.value.clone(),
            http_timeout: self.http_timeout.value,
            user_agent_override: self.user_agent_override.value.clone(),
            // These fields aren't tracked with sources yet but use defaults
            recent_urls: Vec::new(),
            max_recent_urls: 10,
            filter_presets: Vec::new(),
        }
    }

    /// Apply values from another config with proper priority handling
    pub fn apply_from(&mut self, other: &ConfigWithSources, is_gui: bool) {
        macro_rules! apply_if_higher_priority {
            ($field:ident) => {
                if other.$field.should_override(&self.$field, is_gui) {
                    self.$field = other.$field.clone();
                }
            };
        }

        apply_if_higher_priority!(default_output_path);
        apply_if_higher_priority!(concurrent_downloads);
        apply_if_higher_priority!(max_retries);
        apply_if_higher_priority!(default_include_ext);
        apply_if_higher_priority!(default_exclude_ext);
        apply_if_higher_priority!(default_min_file_size);
        apply_if_higher_priority!(default_max_file_size);
        apply_if_higher_priority!(default_resume);
        apply_if_higher_priority!(default_verbose);
        apply_if_higher_priority!(default_log_hash_errors);
        apply_if_higher_priority!(default_dry_run);
        apply_if_higher_priority!(default_compress);
        apply_if_higher_priority!(default_decompress);
        apply_if_higher_priority!(default_decompress_formats);
        apply_if_higher_priority!(http_timeout);
        apply_if_higher_priority!(user_agent_override);
    }
}

/// Configuration persistence manager
pub struct ConfigPersistence {
    config_dir: PathBuf,
    config_file: PathBuf,
    conf_file: PathBuf, // Support for ia-get.conf
}

impl ConfigPersistence {
    /// Create a new configuration persistence manager
    pub fn new() -> Result<Self> {
        let config_dir = crate::infrastructure::config::ConfigManager::get_config_directory()?;
        let config_file = config_dir.join("config.toml");
        let conf_file = config_dir.join("ia-get.conf");

        Ok(Self {
            config_dir,
            config_file,
            conf_file,
        })
    }

    /// Load configuration from file (supports both .toml and .conf)
    pub fn load_config(&self) -> Result<Config> {
        // Try ia-get.conf first (as specified in requirements), then config.toml
        for config_path in [&self.conf_file, &self.config_file] {
            if config_path.exists() {
                return self.load_config_from_file(config_path);
            }
        }

        // No config file exists, return defaults
        Ok(Config::default())
    }

    /// Load configuration from a specific file
    fn load_config_from_file<P: AsRef<Path>>(&self, path: P) -> Result<Config> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| IaGetError::Config(format!("Failed to read config file: {}", e)))?;

        let config: Config = toml::from_str(&content)
            .map_err(|e| IaGetError::Config(format!("Failed to parse config file: {}", e)))?;

        Ok(config)
    }

    /// Save configuration to file (uses ia-get.conf format)
    pub fn save_config(&self, config: &Config) -> Result<()> {
        // Ensure config directory exists
        fs::create_dir_all(&self.config_dir)
            .map_err(|e| IaGetError::Config(format!("Failed to create config directory: {}", e)))?;

        let content = toml::to_string_pretty(config)
            .map_err(|e| IaGetError::Config(format!("Failed to serialize config: {}", e)))?;

        // Save as ia-get.conf (as specified in requirements)
        fs::write(&self.conf_file, content)
            .map_err(|e| IaGetError::Config(format!("Failed to write config file: {}", e)))?;

        Ok(())
    }

    /// Get the path to the preferred config file (ia-get.conf)
    pub fn get_config_file_path(&self) -> &Path {
        &self.conf_file
    }

    /// Get the config directory path
    pub fn get_config_directory(&self) -> &Path {
        &self.config_dir
    }

    /// Check if a config file exists
    pub fn config_exists(&self) -> bool {
        self.conf_file.exists() || self.config_file.exists()
    }

    /// Migrate from old config.toml to new ia-get.conf format
    pub fn migrate_config(&self) -> Result<bool> {
        if self.config_file.exists() && !self.conf_file.exists() {
            // Load from old format
            let config = self.load_config_from_file(&self.config_file)?;

            // Save in new format
            self.save_config(&config)?;

            // Optionally remove old file (for clean migration)
            if let Err(e) = fs::remove_file(&self.config_file) {
                eprintln!("Warning: Could not remove old config.toml: {}", e);
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Build final configuration with proper priority handling
    pub fn build_final_config(
        &self,
        cli_config: Option<Config>,
        gui_overrides: Option<Config>,
        is_gui: bool,
    ) -> Result<Config> {
        // Start with defaults
        let mut final_config = ConfigWithSources::default();

        // Apply saved config file
        if let Ok(saved_config) = self.load_config() {
            let saved_with_sources = self.config_to_sources(saved_config, ConfigSource::ConfigFile);
            final_config.apply_from(&saved_with_sources, is_gui);
        }

        // Apply CLI arguments (if any)
        if let Some(cli_config) = cli_config {
            let cli_with_sources = self.config_to_sources(cli_config, ConfigSource::CommandLine);
            final_config.apply_from(&cli_with_sources, is_gui);
        }

        // Apply GUI overrides (if any)
        if let Some(gui_config) = gui_overrides {
            let gui_with_sources = self.config_to_sources(gui_config, ConfigSource::GuiOneTime);
            final_config.apply_from(&gui_with_sources, is_gui);
        }

        Ok(final_config.to_config())
    }

    /// Convert a Config to ConfigWithSources
    fn config_to_sources(&self, config: Config, source: ConfigSource) -> ConfigWithSources {
        ConfigWithSources {
            default_output_path: ConfigValue::new(config.default_output_path, source.clone()),
            concurrent_downloads: ConfigValue::new(config.concurrent_downloads, source.clone()),
            max_retries: ConfigValue::new(config.max_retries, source.clone()),
            default_include_ext: ConfigValue::new(config.default_include_ext, source.clone()),
            default_exclude_ext: ConfigValue::new(config.default_exclude_ext, source.clone()),
            default_min_file_size: ConfigValue::new(config.default_min_file_size, source.clone()),
            default_max_file_size: ConfigValue::new(config.default_max_file_size, source.clone()),
            default_resume: ConfigValue::new(config.default_resume, source.clone()),
            default_verbose: ConfigValue::new(config.default_verbose, source.clone()),
            default_log_hash_errors: ConfigValue::new(
                config.default_log_hash_errors,
                source.clone(),
            ),
            default_dry_run: ConfigValue::new(config.default_dry_run, source.clone()),
            default_compress: ConfigValue::new(config.default_compress, source.clone()),
            default_decompress: ConfigValue::new(config.default_decompress, source.clone()),
            default_decompress_formats: ConfigValue::new(
                config.default_decompress_formats,
                source.clone(),
            ),
            http_timeout: ConfigValue::new(config.http_timeout, source.clone()),
            user_agent_override: ConfigValue::new(config.user_agent_override, source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_priority_cli() {
        let cli_value = ConfigValue::new(5, ConfigSource::CommandLine);
        let config_value = ConfigValue::new(3, ConfigSource::ConfigFile);

        // In CLI context, command line should override config file
        assert!(cli_value.should_override(&config_value, false));
        assert!(!config_value.should_override(&cli_value, false));
    }

    #[test]
    fn test_config_priority_gui() {
        let gui_value = ConfigValue::new(5, ConfigSource::GuiOneTime);
        let config_value = ConfigValue::new(3, ConfigSource::ConfigFile);

        // In GUI context, one-time selection should override config file
        assert!(gui_value.should_override(&config_value, true));
        assert!(!config_value.should_override(&gui_value, true));
    }

    #[test]
    fn test_config_with_sources_apply() {
        let mut base_config = ConfigWithSources::default();
        base_config.concurrent_downloads = ConfigValue::new(3, ConfigSource::Default);

        let mut override_config = ConfigWithSources::default();
        override_config.concurrent_downloads = ConfigValue::new(5, ConfigSource::CommandLine);

        base_config.apply_from(&override_config, false);

        assert_eq!(base_config.concurrent_downloads.value, 5);
        assert_eq!(
            base_config.concurrent_downloads.source,
            ConfigSource::CommandLine
        );
    }

    #[test]
    fn test_config_persistence() -> Result<()> {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().to_path_buf();

        let persistence = ConfigPersistence {
            config_dir: config_dir.clone(),
            config_file: config_dir.join("config.toml"),
            conf_file: config_dir.join("ia-get.conf"),
        };

        let mut config = Config::default();
        config.concurrent_downloads = 5;
        config.default_verbose = true;

        persistence.save_config(&config)?;

        let loaded_config = persistence.load_config()?;
        assert_eq!(loaded_config.concurrent_downloads, 5);
        assert_eq!(loaded_config.default_verbose, true);

        Ok(())
    }
}
