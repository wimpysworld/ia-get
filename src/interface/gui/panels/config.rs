//! Configuration panel for the GUI

use crate::infrastructure::config::{Config, FilterPreset};
use egui::Ui;

#[derive(Default)]
pub struct ConfigPanel {
    // UI state for config editing
    new_preset_name: String,
    new_preset_description: String,
    new_preset_include: String,
    new_preset_exclude: String,
    new_preset_max_size: String,
}

impl ConfigPanel {
    pub fn new(_config: Config) -> Self {
        Self::default()
    }

    pub fn render(&mut self, ui: &mut Ui, config: &mut Config) {
        ui.heading("Application Settings");

        ui.add_space(10.0);

        // Download settings
        ui.group(|ui| {
            ui.label("Download Settings");

            ui.horizontal(|ui| {
                ui.label("Max Concurrent Downloads:");
                ui.add(egui::Slider::new(&mut config.concurrent_downloads, 1..=10));
                // Match CLI range
            });

            ui.horizontal(|ui| {
                ui.label("Max Retries:");
                ui.add(egui::Slider::new(&mut config.max_retries, 1..=20)); // Match CLI range
            });

            ui.horizontal(|ui| {
                ui.label("HTTP Timeout (seconds):");
                ui.add(egui::Slider::new(&mut config.http_timeout, 5..=300));
            });
        });

        ui.add_space(10.0);

        // Default behavior settings
        ui.group(|ui| {
            ui.label("Default Behavior");

            ui.checkbox(&mut config.default_resume, "Resume downloads by default");
            ui.checkbox(&mut config.default_verbose, "Verbose output by default");
            ui.checkbox(
                &mut config.default_log_hash_errors,
                "Log hash errors by default",
            );
            ui.checkbox(&mut config.default_dry_run, "Dry run mode by default");
            ui.checkbox(
                &mut config.default_compress,
                "Enable HTTP compression by default",
            );
            ui.checkbox(
                &mut config.default_decompress,
                "Auto-decompress files by default",
            );
        });

        ui.add_space(10.0);

        // Compression settings
        if config.default_decompress {
            ui.group(|ui| {
                ui.label("Decompression Settings");

                ui.horizontal(|ui| {
                    ui.label("Default decompress formats:");
                    let mut formats = config
                        .default_decompress_formats
                        .clone()
                        .unwrap_or_default();
                    if ui.text_edit_singleline(&mut formats).changed() {
                        config.default_decompress_formats = if formats.is_empty() {
                            None
                        } else {
                            Some(formats)
                        };
                    }
                    ui.label("(e.g., gzip,bzip2,xz)");
                });
            });

            ui.add_space(10.0);
        }

        ui.add_space(10.0);

        // User agent override
        ui.group(|ui| {
            ui.label("Advanced Settings");

            ui.horizontal(|ui| {
                ui.label("User Agent Override:");
                let mut user_agent = config.user_agent_override.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut user_agent).changed() {
                    config.user_agent_override = if user_agent.is_empty() {
                        None
                    } else {
                        Some(user_agent)
                    };
                }
            });
        });

        ui.add_space(10.0);

        // Filter presets management
        ui.group(|ui| {
            ui.label("Filter Presets");

            // List existing presets
            let mut to_remove: Option<usize> = None;
            for (i, preset) in config.filter_presets.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(&preset.name);
                    ui.label(&preset.description);
                    if ui.small_button("Remove").clicked() {
                        to_remove = Some(i);
                    }
                });
            }

            if let Some(index) = to_remove {
                config.filter_presets.remove(index);
            }

            ui.separator();

            // Add new preset
            ui.label("Add New Preset:");
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.new_preset_name);
            });
            ui.horizontal(|ui| {
                ui.label("Description:");
                ui.text_edit_singleline(&mut self.new_preset_description);
            });
            ui.horizontal(|ui| {
                ui.label("Include formats:");
                ui.text_edit_singleline(&mut self.new_preset_include);
            });
            ui.horizontal(|ui| {
                ui.label("Exclude formats:");
                ui.text_edit_singleline(&mut self.new_preset_exclude);
            });
            ui.horizontal(|ui| {
                ui.label("Max file size:");
                ui.text_edit_singleline(&mut self.new_preset_max_size);
            });

            if ui.button("Add Preset").clicked() && !self.new_preset_name.is_empty() {
                let preset = FilterPreset {
                    name: self.new_preset_name.clone(),
                    description: self.new_preset_description.clone(),
                    include_ext: if self.new_preset_include.is_empty() {
                        None
                    } else {
                        Some(self.new_preset_include.clone())
                    },
                    exclude_ext: if self.new_preset_exclude.is_empty() {
                        None
                    } else {
                        Some(self.new_preset_exclude.clone())
                    },
                    max_file_size: if self.new_preset_max_size.is_empty() {
                        None
                    } else {
                        Some(self.new_preset_max_size.clone())
                    },
                };

                config.filter_presets.push(preset);

                // Clear input fields
                self.new_preset_name.clear();
                self.new_preset_description.clear();
                self.new_preset_include.clear();
                self.new_preset_exclude.clear();
                self.new_preset_max_size.clear();
            }
        });
    }
}
