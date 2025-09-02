//! File filtering panel

use crate::config::{Config, FilterPreset};
use egui::Ui;

#[derive(Default)]
pub struct FiltersPanel {
    // Filter settings
    include_formats: String,
    exclude_formats: String,
    max_file_size: String,
    min_file_size: String,
    selected_preset: Option<usize>,
}

impl FiltersPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn render(&mut self, ui: &mut Ui, config: &mut Config) {
        ui.heading("File Filters");

        ui.add_space(10.0);

        // Filter presets
        if !config.filter_presets.is_empty() {
            ui.group(|ui| {
                ui.label("Quick Presets");

                ui.horizontal_wrapped(|ui| {
                    for (i, preset) in config.filter_presets.iter().enumerate() {
                        if ui
                            .selectable_label(self.selected_preset == Some(i), &preset.name)
                            .clicked()
                        {
                            self.selected_preset = Some(i);
                            self.apply_preset(preset);
                        }
                    }
                });

                if let Some(index) = self.selected_preset {
                    if let Some(preset) = config.filter_presets.get(index) {
                        ui.label(&preset.description);
                    }
                }
            });

            ui.add_space(10.0);
        }

        // Manual filter settings
        ui.group(|ui| {
            ui.label("Manual Filters");

            ui.horizontal(|ui| {
                ui.label("Include formats:");
                ui.text_edit_singleline(&mut self.include_formats);
                ui.label("(comma-separated, e.g., pdf,txt,jpg)");
            });

            ui.horizontal(|ui| {
                ui.label("Exclude formats:");
                ui.text_edit_singleline(&mut self.exclude_formats);
                ui.label("(comma-separated)");
            });

            ui.horizontal(|ui| {
                ui.label("Min file size:");
                ui.text_edit_singleline(&mut self.min_file_size);
                ui.label("(e.g., 1MB, 500KB)");
            });

            ui.horizontal(|ui| {
                ui.label("Max file size:");
                ui.text_edit_singleline(&mut self.max_file_size);
                ui.label("(e.g., 100MB, 2GB)");
            });
        });

        ui.add_space(10.0);

        // Apply filters button
        ui.horizontal(|ui| {
            if ui.button("Apply Filters").clicked() {
                self.apply_filters_to_config(config);
            }

            if ui.button("Clear All Filters").clicked() {
                self.clear_filters();
                self.selected_preset = None;
            }
        });

        ui.add_space(10.0);

        // Current filter summary
        ui.group(|ui| {
            ui.label("Current Filter Settings");

            if !self.include_formats.is_empty() {
                ui.label(format!("Include: {}", self.include_formats));
            }
            if !self.exclude_formats.is_empty() {
                ui.label(format!("Exclude: {}", self.exclude_formats));
            }
            if !self.min_file_size.is_empty() {
                ui.label(format!("Min size: {}", self.min_file_size));
            }
            if !self.max_file_size.is_empty() {
                ui.label(format!("Max size: {}", self.max_file_size));
            }

            if self.include_formats.is_empty()
                && self.exclude_formats.is_empty()
                && self.min_file_size.is_empty()
                && self.max_file_size.is_empty()
            {
                ui.label("No filters applied - all files will be downloaded");
            }
        });
    }

    /// Apply a preset to the current filter settings
    fn apply_preset(&mut self, preset: &FilterPreset) {
        self.include_formats = preset.include_ext.clone().unwrap_or_default();
        self.exclude_formats = preset.exclude_ext.clone().unwrap_or_default();
        self.max_file_size = preset.max_file_size.clone().unwrap_or_default();
        // Presets don't typically have min size, so keep current value
    }

    /// Apply current filter settings to the configuration
    fn apply_filters_to_config(&self, config: &mut Config) {
        config.default_include_ext = if self.include_formats.is_empty() {
            None
        } else {
            Some(self.include_formats.clone())
        };

        config.default_exclude_ext = if self.exclude_formats.is_empty() {
            None
        } else {
            Some(self.exclude_formats.clone())
        };

        config.default_min_file_size = if self.min_file_size.is_empty() {
            None
        } else {
            Some(self.min_file_size.clone())
        };

        config.default_max_file_size = if self.max_file_size.is_empty() {
            None
        } else {
            Some(self.max_file_size.clone())
        };
    }

    /// Clear all filter settings
    fn clear_filters(&mut self) {
        self.include_formats.clear();
        self.exclude_formats.clear();
        self.max_file_size.clear();
        self.min_file_size.clear();
    }

    /// Get current filter settings as strings
    pub fn get_filter_settings(&self) -> (String, String, String, String) {
        (
            self.include_formats.clone(),
            self.exclude_formats.clone(),
            self.min_file_size.clone(),
            self.max_file_size.clone(),
        )
    }
}
