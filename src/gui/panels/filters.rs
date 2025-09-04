//! File filtering panel

use crate::cli::SourceType;
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

    // Format category checkboxes
    include_documents: bool,
    include_images: bool,
    include_audio: bool,
    include_video: bool,
    include_software: bool,
    include_data: bool,
    include_web: bool,
    include_archives: bool,
    include_metadata_formats: bool,

    exclude_documents: bool,
    exclude_images: bool,
    exclude_audio: bool,
    exclude_video: bool,
    exclude_software: bool,
    exclude_data: bool,
    exclude_web: bool,
    exclude_archives: bool,
    exclude_metadata_formats: bool,

    // Source filtering
    include_original: bool,
    include_derivative: bool,
    include_metadata: bool,
}

impl FiltersPanel {
    pub fn new() -> Self {
        Self {
            include_original: true, // Default to including original files
            include_derivative: false,
            include_metadata: false,
            ..Default::default()
        }
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

        // Format category filters
        ui.group(|ui| {
            ui.label("Format Categories");
            ui.label("Select format categories to include or exclude:");

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Include Categories:");
                    ui.checkbox(&mut self.include_documents, "📄 Documents")
                        .on_hover_text("PDF, text, eBooks, office documents");
                    ui.checkbox(&mut self.include_images, "🖼️ Images")
                        .on_hover_text("Photos, graphics, artwork");
                    ui.checkbox(&mut self.include_audio, "🎵 Audio")
                        .on_hover_text("Music, recordings, podcasts");
                    ui.checkbox(&mut self.include_video, "🎬 Video")
                        .on_hover_text("Movies, TV shows, clips");
                    ui.checkbox(&mut self.include_software, "💾 Software")
                        .on_hover_text("Applications, games, installers");
                });

                ui.vertical(|ui| {
                    ui.label("Exclude Categories:");
                    ui.checkbox(&mut self.exclude_documents, "📄 Documents");
                    ui.checkbox(&mut self.exclude_images, "🖼️ Images");
                    ui.checkbox(&mut self.exclude_audio, "🎵 Audio");
                    ui.checkbox(&mut self.exclude_video, "🎬 Video");
                    ui.checkbox(&mut self.exclude_software, "💾 Software");
                });

                ui.vertical(|ui| {
                    ui.label("More Categories:");
                    ui.checkbox(&mut self.include_data, "📊 Data")
                        .on_hover_text("Datasets, databases, structured data");
                    ui.checkbox(&mut self.include_web, "🌐 Web")
                        .on_hover_text("Web pages, websites, archives");
                    ui.checkbox(&mut self.include_archives, "📦 Archives")
                        .on_hover_text("ZIP, RAR, compressed files");
                    ui.checkbox(&mut self.include_metadata_formats, "🏷️ Metadata")
                        .on_hover_text("Archive metadata, checksums");
                    ui.label(""); // Spacer
                });

                ui.vertical(|ui| {
                    ui.label("Exclude More:");
                    ui.checkbox(&mut self.exclude_data, "📊 Data");
                    ui.checkbox(&mut self.exclude_web, "🌐 Web");
                    ui.checkbox(&mut self.exclude_archives, "📦 Archives");
                    ui.checkbox(&mut self.exclude_metadata_formats, "🏷️ Metadata");
                    ui.label(""); // Spacer
                });
            });

            // Quick preset buttons
            ui.horizontal(|ui| {
                if ui.button("Documents Only").clicked() {
                    self.clear_format_categories();
                    self.include_documents = true;
                }
                if ui.button("Media Only").clicked() {
                    self.clear_format_categories();
                    self.include_images = true;
                    self.include_audio = true;
                    self.include_video = true;
                }
                if ui.button("No Metadata").clicked() {
                    self.exclude_metadata_formats = true;
                }
                if ui.button("Clear Categories").clicked() {
                    self.clear_format_categories();
                }
            });
        });

        ui.add_space(10.0);

        // Source type filtering
        ui.group(|ui| {
            ui.label("Source Types");
            ui.label("Select which types of files to include:");

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.include_original, "Original files")
                    .on_hover_text("Files uploaded by users to the Internet Archive");
                ui.checkbox(&mut self.include_derivative, "Derivative files")
                    .on_hover_text("Files generated from originals (e.g., lower quality versions)");
                ui.checkbox(&mut self.include_metadata, "Metadata files")
                    .on_hover_text("Archive-generated metadata files (e.g., XML, torrents)");
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

            // Show source types
            let mut source_types = Vec::new();
            if self.include_original {
                source_types.push("original");
            }
            if self.include_derivative {
                source_types.push("derivative");
            }
            if self.include_metadata {
                source_types.push("metadata");
            }
            if !source_types.is_empty() {
                ui.label(format!("Source types: {}", source_types.join(", ")));
            }

            if self.include_formats.is_empty()
                && self.exclude_formats.is_empty()
                && self.min_file_size.is_empty()
                && self.max_file_size.is_empty()
                && self.include_original
                && !self.include_derivative
                && !self.include_metadata
            {
                ui.label("Default filters: original files only");
            }
        });
    }

    /// Apply a preset to the current filter settings
    pub fn get_source_types(&self) -> Vec<SourceType> {
        let mut source_types = Vec::new();
        if self.include_original {
            source_types.push(SourceType::Original);
        }
        if self.include_derivative {
            source_types.push(SourceType::Derivative);
        }
        if self.include_metadata {
            source_types.push(SourceType::Metadata);
        }
        // Default to original if none selected
        if source_types.is_empty() {
            source_types.push(SourceType::Original);
        }
        source_types
    }

    fn apply_preset(&mut self, preset: &FilterPreset) {
        self.include_formats = preset.include_ext.clone().unwrap_or_default();
        self.exclude_formats = preset.exclude_ext.clone().unwrap_or_default();
        self.max_file_size = preset.max_file_size.clone().unwrap_or_default();
        // Presets don't typically have min size, so keep current value
    }

    /// Apply current filter settings to the configuration
    fn apply_filters_to_config(&self, config: &mut Config) {
        // Combine manual formats with category-based formats
        let mut combined_include = self.include_formats.clone();
        let mut combined_exclude = self.exclude_formats.clone();

        // Add category-based includes
        let include_categories = self.get_selected_include_categories();
        if !include_categories.is_empty() {
            if !combined_include.is_empty() {
                combined_include.push(',');
            }
            combined_include.push_str(&include_categories.join(","));
        }

        // Add category-based excludes
        let exclude_categories = self.get_selected_exclude_categories();
        if !exclude_categories.is_empty() {
            if !combined_exclude.is_empty() {
                combined_exclude.push(',');
            }
            combined_exclude.push_str(&exclude_categories.join(","));
        }

        config.default_include_ext = if combined_include.is_empty() {
            None
        } else {
            Some(combined_include)
        };

        config.default_exclude_ext = if combined_exclude.is_empty() {
            None
        } else {
            Some(combined_exclude)
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
        self.clear_format_categories();
    }

    /// Clear format category selections
    fn clear_format_categories(&mut self) {
        self.include_documents = false;
        self.include_images = false;
        self.include_audio = false;
        self.include_video = false;
        self.include_software = false;
        self.include_data = false;
        self.include_web = false;
        self.include_archives = false;
        self.include_metadata_formats = false;

        self.exclude_documents = false;
        self.exclude_images = false;
        self.exclude_audio = false;
        self.exclude_video = false;
        self.exclude_software = false;
        self.exclude_data = false;
        self.exclude_web = false;
        self.exclude_archives = false;
        self.exclude_metadata_formats = false;
    }

    /// Get file extensions for selected include categories
    fn get_selected_include_categories(&self) -> Vec<String> {
        use crate::file_formats::{FileFormats, FormatCategory};
        let file_formats = FileFormats::new();
        let mut extensions = Vec::new();

        if self.include_documents {
            extensions.extend(file_formats.get_formats(&FormatCategory::Documents));
        }
        if self.include_images {
            extensions.extend(file_formats.get_formats(&FormatCategory::Images));
        }
        if self.include_audio {
            extensions.extend(file_formats.get_formats(&FormatCategory::Audio));
        }
        if self.include_video {
            extensions.extend(file_formats.get_formats(&FormatCategory::Video));
        }
        if self.include_software {
            extensions.extend(file_formats.get_formats(&FormatCategory::Software));
        }
        if self.include_data {
            extensions.extend(file_formats.get_formats(&FormatCategory::Data));
        }
        if self.include_web {
            extensions.extend(file_formats.get_formats(&FormatCategory::Web));
        }
        if self.include_archives {
            extensions.extend(file_formats.get_formats(&FormatCategory::Archives));
        }
        if self.include_metadata_formats {
            extensions.extend(file_formats.get_formats(&FormatCategory::Metadata));
        }

        extensions
    }

    /// Get file extensions for selected exclude categories
    fn get_selected_exclude_categories(&self) -> Vec<String> {
        use crate::file_formats::{FileFormats, FormatCategory};
        let file_formats = FileFormats::new();
        let mut extensions = Vec::new();

        if self.exclude_documents {
            extensions.extend(file_formats.get_formats(&FormatCategory::Documents));
        }
        if self.exclude_images {
            extensions.extend(file_formats.get_formats(&FormatCategory::Images));
        }
        if self.exclude_audio {
            extensions.extend(file_formats.get_formats(&FormatCategory::Audio));
        }
        if self.exclude_video {
            extensions.extend(file_formats.get_formats(&FormatCategory::Video));
        }
        if self.exclude_software {
            extensions.extend(file_formats.get_formats(&FormatCategory::Software));
        }
        if self.exclude_data {
            extensions.extend(file_formats.get_formats(&FormatCategory::Data));
        }
        if self.exclude_web {
            extensions.extend(file_formats.get_formats(&FormatCategory::Web));
        }
        if self.exclude_archives {
            extensions.extend(file_formats.get_formats(&FormatCategory::Archives));
        }
        if self.exclude_metadata_formats {
            extensions.extend(file_formats.get_formats(&FormatCategory::Metadata));
        }

        extensions
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
