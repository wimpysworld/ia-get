//! Main GUI application for ia-get
//!
//! Provides the main application window and state management for the GUI interface.

use crate::{
    core::download::download_service::{
        DownloadRequest, DownloadResult, DownloadService, ProgressUpdate,
    },
    core::session::metadata_storage::sanitize_filename_for_filesystem,
    infrastructure::config::{Config, ConfigManager},
};
use egui::{Context, Ui};
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::panels::{
    ArchiveHealthPanel, ConfigPanel, DownloadPanel, FileBrowserPanel, FiltersPanel,
};

/// Main application state
#[derive(Default)]
pub struct IaGetApp {
    // Core state
    config_manager: Option<ConfigManager>,
    config: Config,

    // UI state
    current_tab: AppTab,

    // Download state
    archive_identifier: String,
    output_directory: String,

    // Progress tracking
    download_progress: f32,
    download_status: String,
    is_downloading: bool,

    // Progress receiver for updates from download controller
    progress_rx: Option<mpsc::UnboundedReceiver<ProgressUpdate>>,

    // Completion receiver for download results
    completion_rx: Option<mpsc::UnboundedReceiver<DownloadResult>>,

    // Recent operations
    recent_downloads: Vec<String>,

    // Error handling
    error_message: Option<String>,
    success_message: Option<String>,

    // Async runtime handle for background operations
    rt_handle: Option<tokio::runtime::Handle>,

    // Panels
    archive_health_panel: ArchiveHealthPanel,
    config_panel: ConfigPanel,
    download_panel: DownloadPanel,
    file_browser_panel: FileBrowserPanel,
    filters_panel: FiltersPanel,

    // Dialog state
    show_about_dialog: bool,
    show_open_dialog: bool,

    // Mode switching state
    switch_to_cli: bool,
}

#[derive(Default, PartialEq)]
enum AppTab {
    #[default]
    Download,
    FileBrowser,
    Filters,
    Config,
    History,
    ArchiveHealth,
}

impl IaGetApp {
    /// Create a new GUI application
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load configuration
        let config_manager = ConfigManager::new().ok();
        let config = config_manager
            .as_ref()
            .and_then(|cm| cm.load_config().ok())
            .unwrap_or_default();

        // Set up the GUI theme
        Self::setup_theme(&cc.egui_ctx);

        // Get tokio runtime handle
        let rt_handle = tokio::runtime::Handle::try_current().ok();

        Self {
            config_manager,
            output_directory: config.default_output_path.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            }),
            rt_handle,
            archive_health_panel: ArchiveHealthPanel::new(),
            config_panel: ConfigPanel::new(config.clone()),
            download_panel: DownloadPanel::new(),
            file_browser_panel: FileBrowserPanel::new(),
            filters_panel: FiltersPanel::new(),
            config,
            switch_to_cli: false,
            ..Default::default()
        }
    }

    /// Check if the user wants to switch to CLI mode
    pub fn should_switch_to_cli(&self) -> bool {
        self.switch_to_cli
    }

    /// Set up the application theme
    fn setup_theme(ctx: &Context) {
        let mut style = (*ctx.style()).clone();

        // Use dark theme with custom colors
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(egui::Color32::from_rgb(220, 220, 220));
        style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(40, 40, 40);
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(50, 50, 50);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(60, 60, 60);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 70, 70);

        ctx.set_style(style);
    }

    /// Handle download operation
    fn start_download(&mut self, ctx: &Context) {
        if self.archive_identifier.is_empty() {
            self.error_message = Some("Please enter an Archive.org identifier or URL".to_string());
            return;
        }

        // Clear previous messages
        self.error_message = None;
        self.success_message = None;

        // Set downloading state
        self.is_downloading = true;
        self.download_status = "Initializing download...".to_string();
        self.download_progress = 0.0;

        // Add to recent downloads
        if !self.recent_downloads.contains(&self.archive_identifier) {
            self.recent_downloads
                .insert(0, self.archive_identifier.clone());
            if self.recent_downloads.len() > 10 {
                self.recent_downloads.truncate(10);
            }
        }

        // Create progress channel
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        self.progress_rx = Some(progress_rx);

        // Create completion channel
        let (completion_tx, completion_rx) = mpsc::unbounded_channel();
        self.completion_rx = Some(completion_rx);

        // Create download service
        let service = match DownloadService::new() {
            Ok(s) => s,
            Err(e) => {
                self.error_message = Some(format!("Failed to create download service: {}", e));
                self.is_downloading = false;
                return;
            }
        };

        // Get download parameters
        let raw_identifier = self.archive_identifier.clone();

        // Normalize the identifier - extract just the identifier portion if it's a URL
        let identifier = match crate::url_processing::normalize_archive_identifier(&raw_identifier)
        {
            Ok(id) => id,
            Err(e) => {
                self.error_message = Some(format!("Invalid archive identifier: {}", e));
                self.is_downloading = false;
                return;
            }
        };
        let mut output_dir = PathBuf::from(&self.output_directory);

        // Create archive-specific subdirectory like CLI does
        let sanitized_identifier = sanitize_filename_for_filesystem(&identifier);
        output_dir.push(sanitized_identifier);

        let (include_formats, exclude_formats, min_size, max_size) =
            self.filters_panel.get_filter_settings();
        let include_formats: Vec<String> = if include_formats.is_empty() {
            vec![]
        } else {
            include_formats
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };
        let exclude_formats: Vec<String> = if exclude_formats.is_empty() {
            vec![]
        } else {
            exclude_formats
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };
        let decompress_formats: Vec<String> = self
            .config
            .default_decompress_formats
            .as_ref()
            .map(|formats| formats.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        // Create unified download request using from_config
        let mut request = DownloadRequest::from_config(&self.config, identifier, output_dir);
        request.include_formats = include_formats;
        request.exclude_formats = exclude_formats;
        request.min_file_size = min_size;
        request.max_file_size = if max_size.is_empty() {
            None
        } else {
            Some(max_size)
        };
        request.decompress_formats = decompress_formats;
        request.source_types = self.filters_panel.get_source_types();

        // Create progress callback
        let progress_callback = Box::new(move |update: ProgressUpdate| {
            let _ = progress_tx.send(update);
        });

        // Start download in background
        if let Some(handle) = &self.rt_handle {
            let ctx_clone = ctx.clone();
            handle.spawn(async move {
                let result = service.download(request, Some(progress_callback)).await;

                // Send completion result
                if let Ok(download_result) = result {
                    let _ = completion_tx.send(download_result);
                } else if let Err(e) = result {
                    let _ = completion_tx.send(DownloadResult::Error(e.to_string()));
                }

                // Request repaint when done
                ctx_clone.request_repaint();
            });
        }
    }

    /// Render the main UI
    fn render_main_ui(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Top menu bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open Archive...").clicked() {
                        self.show_open_dialog = true;
                    }
                    if ui.button("Settings").clicked() {
                        self.current_tab = AppTab::Config;
                    }
                    ui.separator();
                    if ui.button("Switch to CLI Mode").clicked() {
                        self.switch_to_cli = true;
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about_dialog = true;
                    }
                });
            });
        });

        // Status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.is_downloading {
                    ui.spinner();
                    ui.label(&self.download_status);
                    if self.download_progress > 0.0 {
                        ui.add(
                            egui::ProgressBar::new(self.download_progress / 100.0)
                                .show_percentage(),
                        );
                    }
                } else {
                    ui.label("Ready");
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("ia-get v{}", env!("CARGO_PKG_VERSION")));
                });
            });
        });

        // Main content area with tabs
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, AppTab::Download, "Download");
                ui.selectable_value(&mut self.current_tab, AppTab::FileBrowser, "File Browser");
                ui.selectable_value(&mut self.current_tab, AppTab::Filters, "Filters");
                ui.selectable_value(&mut self.current_tab, AppTab::Config, "Settings");
                ui.selectable_value(&mut self.current_tab, AppTab::History, "History");
                ui.selectable_value(&mut self.current_tab, AppTab::ArchiveHealth, "API Health");
            });

            ui.separator();

            // Add scrollable area for all tab content to prevent UI elements from being hidden
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| match self.current_tab {
                    AppTab::Download => self.render_download_tab(ui, ctx),
                    AppTab::FileBrowser => self.render_file_browser_tab(ui),
                    AppTab::Filters => self.render_filters_tab(ui),
                    AppTab::Config => self.render_config_tab(ui),
                    AppTab::History => self.render_history_tab(ui),
                    AppTab::ArchiveHealth => self.render_archive_health_tab(ui),
                });
        });

        // Error/Success messages
        let show_error = self.error_message.is_some();
        if show_error {
            let error_msg = self.error_message.clone().unwrap();
            egui::Window::new("❌ Error")
                .collapsible(false)
                .resizable(true)
                .default_width(400.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&error_msg).color(egui::Color32::LIGHT_RED));
                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("OK").clicked() {
                                self.error_message = None;
                            }
                        });
                    });
                });
        }

        let show_success = self.success_message.is_some();
        if show_success {
            let success_msg = self.success_message.clone().unwrap();
            egui::Window::new("✅ Success")
                .collapsible(false)
                .resizable(true)
                .default_width(400.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&success_msg).color(egui::Color32::LIGHT_GREEN));
                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("OK").clicked() {
                                self.success_message = None;
                            }
                        });
                    });
                });
        }
    }

    /// Render the download tab
    fn render_download_tab(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.heading("Download from Archive.org");

        ui.add_space(10.0);

        // Archive identifier input
        ui.horizontal(|ui| {
            ui.label("Archive Identifier/URL:");
            ui.text_edit_singleline(&mut self.archive_identifier);
        });

        ui.add_space(5.0);

        // Output directory selection
        ui.horizontal(|ui| {
            ui.label("Output Directory:");
            ui.text_edit_singleline(&mut self.output_directory);
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.output_directory = path.to_string_lossy().to_string();
                }
            }
        });

        ui.add_space(10.0);

        // Download options
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.default_verbose, "Verbose output");
            ui.checkbox(&mut self.config.default_resume, "Resume downloads");
            ui.checkbox(&mut self.config.default_dry_run, "Dry run (preview only)");
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.default_compress, "Enable HTTP compression");
            ui.checkbox(&mut self.config.default_decompress, "Auto-decompress files");
            ui.checkbox(&mut self.config.default_log_hash_errors, "Log hash errors");
        });

        ui.add_space(10.0);

        // Concurrent downloads setting
        ui.horizontal(|ui| {
            ui.label("Concurrent Downloads:");
            ui.add(egui::Slider::new(
                &mut self.config.concurrent_downloads,
                1..=10, // Match CLI range
            ));
        });

        ui.horizontal(|ui| {
            ui.label("Max Retries:");
            ui.add(egui::Slider::new(
                &mut self.config.max_retries,
                1..=20, // Match CLI range
            ));
        });

        // Decompress formats setting
        if self.config.default_decompress {
            ui.horizontal(|ui| {
                ui.label("Decompress Formats:");
                let mut formats = self
                    .config
                    .default_decompress_formats
                    .clone()
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut formats).changed() {
                    self.config.default_decompress_formats = if formats.is_empty() {
                        None
                    } else {
                        Some(formats)
                    };
                }
                ui.label("(e.g., gzip,bzip2,xz)");
            });
        }

        ui.add_space(20.0);

        // Download button
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new("📥 Download").min_size(egui::vec2(120.0, 32.0)))
                .clicked()
                && !self.is_downloading
            {
                self.start_download(ctx);
            }

            if self.is_downloading && ui.button("❌ Cancel").clicked() {
                self.is_downloading = false;
                self.download_status = "Cancelled".to_string();
            }
        });

        // Download progress area
        if self.is_downloading || !self.download_status.is_empty() {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Use the download panel for progress display
            self.download_panel.render(ui);
        }
    }

    /// Render the file browser tab
    fn render_file_browser_tab(&mut self, ui: &mut Ui) {
        self.file_browser_panel.render(ui, &self.config);
    }

    /// Render the filters tab
    fn render_filters_tab(&mut self, ui: &mut Ui) {
        self.filters_panel.render(ui, &mut self.config);
    }

    /// Render the config tab
    fn render_config_tab(&mut self, ui: &mut Ui) {
        self.config_panel.render(ui, &mut self.config);

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(10.0);

        // Save configuration button
        ui.horizontal(|ui| {
            if ui.button("Save Configuration").clicked() {
                if let Some(manager) = &self.config_manager {
                    match manager.save_config(&self.config) {
                        Ok(()) => {
                            self.success_message =
                                Some("Configuration saved successfully!".to_string());
                        }
                        Err(e) => {
                            self.error_message =
                                Some(format!("Failed to save configuration: {}", e));
                        }
                    }
                }
            }

            if ui.button("Reset to Defaults").clicked() {
                self.config = Config::default();
                self.success_message = Some("Configuration reset to defaults".to_string());
            }
        });
    }

    /// Render the history tab
    fn render_history_tab(&mut self, ui: &mut Ui) {
        ui.heading("Recent Downloads");

        if self.recent_downloads.is_empty() {
            ui.label("No recent downloads");
        } else {
            let mut to_remove: Option<usize> = None;

            for (i, download) in self.recent_downloads.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button("📁").clicked() {
                        self.archive_identifier = download.clone();
                        self.current_tab = AppTab::Download;
                    }
                    ui.label(download);

                    if ui.small_button("❌").clicked() {
                        to_remove = Some(i);
                    }
                });
            }

            if let Some(index) = to_remove {
                self.recent_downloads.remove(index);
            }
        }

        ui.add_space(20.0);

        if ui.button("Clear History").clicked() {
            self.recent_downloads.clear();
        }
    }

    fn render_archive_health_tab(&mut self, ui: &mut Ui) {
        self.archive_health_panel.show(ui);
    }

    /// Render dialogs
    fn render_dialogs(&mut self, ctx: &Context) {
        // About dialog
        if self.show_about_dialog {
            egui::Window::new("About Internet Archive Helper")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("Internet Archive Helper");
                        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                        ui.add_space(10.0);
                        ui.label("Your comprehensive companion for accessing Internet Archive content");
                        ui.add_space(5.0);
                        ui.label("Maintained by Gameaday");
                        ui.add_space(10.0);
                        ui.hyperlink_to(
                            "GitHub Repository",
                            "https://github.com/Gameaday/ia-get-cli",
                        );
                        ui.add_space(5.0);
                        ui.label("Built with Rust and egui • Made with ❤️ for the Internet Archive community");
                        ui.add_space(20.0);

                        if ui.button("Close").clicked() {
                            self.show_about_dialog = false;
                        }
                    });
                });
        }

        // Open Archive dialog
        if self.show_open_dialog {
            egui::Window::new("Open Archive")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label("Enter Internet Archive identifier or URL:");
                        ui.add_space(10.0);

                        let mut temp_identifier = self.archive_identifier.clone();
                        ui.add(egui::TextEdit::singleline(&mut temp_identifier).hint_text(
                            "e.g., commute_test or https://archive.org/details/commute_test",
                        ));

                        ui.add_space(10.0);
                        ui.label("Examples:");
                        ui.label("• commute_test");
                        ui.label("• https://archive.org/details/commute_test");
                        ui.label("• https://archive.org/download/commute_test/");

                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            if ui.button("Open").clicked() {
                                self.archive_identifier = temp_identifier;
                                self.current_tab = AppTab::Download;
                                self.show_open_dialog = false;
                            }

                            if ui.button("Cancel").clicked() {
                                self.show_open_dialog = false;
                            }
                        });
                    });
                });
        }
    }
}

impl eframe::App for IaGetApp {
    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        // Handle progress updates
        if let Some(rx) = &mut self.progress_rx {
            while let Ok(update) = rx.try_recv() {
                self.download_panel.update_progress(
                    update.current_file,
                    update.total_files,
                    update.completed_files,
                    update.failed_files,
                    update.current_speed,
                    update.eta,
                );
                self.download_status = update.status;
                if update.total_files > 0 {
                    self.download_progress =
                        (update.completed_files as f32 / update.total_files as f32) * 100.0;
                }
            }
        }

        // Handle completion results
        if let Some(rx) = &mut self.completion_rx {
            while let Ok(result) = rx.try_recv() {
                self.is_downloading = false;
                match result {
                    DownloadResult::Success(session, _api_stats, is_dry_run) => {
                        if is_dry_run {
                            // This was a dry run - display the results
                            let file_count = session.archive_metadata.files.len();
                            let total_size: u64 = session
                                .archive_metadata
                                .files
                                .iter()
                                .map(|f| f.size.unwrap_or(0))
                                .sum();

                            self.success_message = Some(format!(
                                "Dry run completed: {} files found (total size: {} bytes)",
                                file_count, total_size
                            ));
                            self.download_status =
                                format!("Dry run completed - {} files found", file_count);
                        } else {
                            // This was a real download
                            let progress = session.get_progress_summary();
                            if progress.completed_files == progress.total_files {
                                self.success_message = Some(format!(
                                    "Download completed successfully! {} files downloaded",
                                    progress.completed_files
                                ));
                                self.download_status =
                                    "Download completed successfully".to_string();
                            } else {
                                let failed_files = progress.total_files - progress.completed_files;
                                self.error_message = Some(format!(
                                    "Download completed with {} failed files. {} of {} files downloaded successfully",
                                    failed_files, progress.completed_files, progress.total_files
                                ));
                                self.download_status =
                                    format!("Completed with {} failed files", failed_files);
                            }
                        }
                        // Reset progress panel
                        self.download_panel.reset();
                    }
                    DownloadResult::Error(error_msg) => {
                        self.error_message = Some(format!("Download failed: {}", error_msg));
                        self.download_status = "Download failed".to_string();
                        // Reset progress panel
                        self.download_panel.reset();
                    }
                }
            }
        }

        self.render_main_ui(ctx, frame);

        // Handle dialogs
        self.render_dialogs(ctx);
    }
}
