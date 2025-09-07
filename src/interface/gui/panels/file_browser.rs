//! File browser panel for selecting specific files to download

use crate::{
    core::download::download_service::{DownloadRequest, DownloadService},
    core::session::metadata_storage::{ArchiveFile, ArchiveMetadata},
    infrastructure::config::Config,
    utilities::filters::format_size,
};
use egui::Ui;
use std::collections::{HashMap, HashSet};

/// File browser panel for archive exploration and selective downloading
#[derive(Default)]
pub struct FileBrowserPanel {
    // Input state
    archive_identifier: String,
    output_directory: String,

    // Metadata state
    archive_metadata: Option<ArchiveMetadata>,
    filtered_files: Vec<ArchiveFile>,

    // File selection state
    selected_files: HashSet<String>,
    file_tree: FileTreeNode,
    expanded_folders: HashSet<String>,

    // UI state
    is_loading: bool,
    error_message: Option<String>,
    success_message: Option<String>,
    show_file_details: bool,
    selected_file_for_details: Option<ArchiveFile>,

    // Search and filtering
    search_filter: String,
    format_filter: String,
    min_size_filter: String,
    max_size_filter: String,

    // Download state
    download_in_progress: bool,

    // Runtime handle for async operations
    rt_handle: Option<tokio::runtime::Handle>,
}

/// Represents a node in the file tree structure
#[derive(Default, Clone)]
pub struct FileTreeNode {
    name: String,
    full_path: String,
    is_file: bool,
    file_info: Option<ArchiveFile>,
    children: HashMap<String, FileTreeNode>,
    size: Option<u64>,
}

impl FileBrowserPanel {
    pub fn new() -> Self {
        Self {
            rt_handle: tokio::runtime::Handle::try_current().ok(),
            ..Default::default()
        }
    }

    pub fn render(&mut self, ui: &mut Ui, config: &Config) {
        ui.heading("File Browser & Selector");
        ui.separator();

        // Input section
        self.render_input_section(ui, config);

        ui.separator();

        if self.is_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Fetching archive metadata...");
            });
            return;
        }

        // Error display
        if let Some(ref error) = self.error_message {
            ui.colored_label(egui::Color32::RED, error);
            ui.separator();
        }

        // Success display
        if let Some(ref success) = self.success_message {
            ui.colored_label(egui::Color32::GREEN, success);
            ui.separator();
        }

        // File browser section
        if self.archive_metadata.is_some() {
            self.render_file_browser(ui);
        }
    }

    fn render_input_section(&mut self, ui: &mut Ui, config: &Config) {
        ui.group(|ui| {
            ui.label("Archive Information");

            ui.horizontal(|ui| {
                ui.label("Identifier:");
                ui.text_edit_singleline(&mut self.archive_identifier);

                if ui.button("Browse Archive").clicked()
                    && !self.archive_identifier.trim().is_empty()
                {
                    self.fetch_archive_metadata();
                }
            });

            ui.horizontal(|ui| {
                ui.label("Output Directory:");
                ui.text_edit_singleline(&mut self.output_directory);

                if ui.button("📁").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_directory = path.to_string_lossy().to_string();
                    }
                }
            });

            // Initialize output directory from config if empty
            if self.output_directory.is_empty() {
                self.output_directory = config
                    .default_output_path
                    .clone()
                    .unwrap_or_else(|| "./downloads".to_string());
            }
        });
    }

    fn render_file_browser(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("Archive Files");
                ui.separator();

                if let Some(ref _metadata) = self.archive_metadata {
                    let total_files = self.filtered_files.len();
                    let selected_count = self.selected_files.len();
                    let total_size: u64 = self
                        .filtered_files
                        .iter()
                        .filter(|f| self.selected_files.contains(&f.name))
                        .map(|f| f.size.unwrap_or(0))
                        .sum();

                    ui.label(format!(
                        "{} files total, {} selected ({})",
                        total_files,
                        selected_count,
                        format_size(total_size)
                    ));
                }
            });

            // Search and filter controls
            self.render_filter_controls(ui);

            ui.separator();

            // File selection controls
            ui.horizontal(|ui| {
                if ui.button("Select All").clicked() {
                    for file in &self.filtered_files {
                        self.selected_files.insert(file.name.clone());
                    }
                }

                if ui.button("Select None").clicked() {
                    self.selected_files.clear();
                }

                if ui.button("Invert Selection").clicked() {
                    let all_files: HashSet<_> =
                        self.filtered_files.iter().map(|f| f.name.clone()).collect();
                    let currently_selected = self.selected_files.clone();
                    self.selected_files =
                        all_files.difference(&currently_selected).cloned().collect();
                }
            });

            ui.separator();

            // File tree display
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    self.render_file_tree(ui, &self.file_tree.clone());
                });

            ui.separator();

            // Download controls
            self.render_download_controls(ui);
        });
    }

    fn render_filter_controls(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label("Filters");

            ui.horizontal(|ui| {
                ui.label("Search:");
                if ui.text_edit_singleline(&mut self.search_filter).changed() {
                    self.apply_filters();
                }

                ui.label("Format:");
                if ui.text_edit_singleline(&mut self.format_filter).changed() {
                    self.apply_filters();
                }
            });

            ui.horizontal(|ui| {
                ui.label("Min Size:");
                if ui.text_edit_singleline(&mut self.min_size_filter).changed() {
                    self.apply_filters();
                }

                ui.label("Max Size:");
                if ui.text_edit_singleline(&mut self.max_size_filter).changed() {
                    self.apply_filters();
                }
            });
        });
    }

    fn render_file_tree(&mut self, ui: &mut Ui, node: &FileTreeNode) {
        if node.is_file {
            // Render file entry
            if let Some(ref file_info) = node.file_info {
                ui.horizontal(|ui| {
                    let is_selected = self.selected_files.contains(&file_info.name);
                    let mut selected = is_selected;

                    if ui.checkbox(&mut selected, "").changed() {
                        if selected {
                            self.selected_files.insert(file_info.name.clone());
                        } else {
                            self.selected_files.remove(&file_info.name);
                        }
                    }

                    // File icon based on format
                    let icon = self.get_file_icon(&file_info.format);
                    ui.label(icon);

                    if ui.selectable_label(false, &file_info.name).clicked() {
                        self.selected_file_for_details = Some(file_info.clone());
                        self.show_file_details = true;
                    }

                    // File size
                    if let Some(size) = file_info.size {
                        ui.label(format_size(size));
                    }

                    // File format
                    if let Some(ref format) = file_info.format {
                        ui.label(format);
                    }
                });
            }
        } else {
            // Render folder entry
            let is_expanded = self.expanded_folders.contains(&node.full_path);

            ui.horizontal(|ui| {
                let folder_icon = if is_expanded { "📂" } else { "📁" };

                if ui
                    .button(format!("{} {}", folder_icon, node.name))
                    .clicked()
                {
                    if is_expanded {
                        self.expanded_folders.remove(&node.full_path);
                    } else {
                        self.expanded_folders.insert(node.full_path.clone());
                    }
                }

                // Show folder size if available
                if let Some(size) = node.size {
                    ui.label(format!("({})", format_size(size)));
                }
            });

            if is_expanded {
                ui.indent("folder_contents", |ui| {
                    let mut children: Vec<_> = node.children.values().collect();
                    children.sort_by(|a, b| {
                        // Folders first, then files, both alphabetically
                        match (a.is_file, b.is_file) {
                            (false, true) => std::cmp::Ordering::Less,
                            (true, false) => std::cmp::Ordering::Greater,
                            _ => a.name.cmp(&b.name),
                        }
                    });

                    for child in children {
                        self.render_file_tree(ui, child);
                    }
                });
            }
        }
    }

    fn render_download_controls(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let selected_count = self.selected_files.len();
            let can_download = selected_count > 0 && !self.download_in_progress;

            ui.add_enabled_ui(can_download, |ui| {
                if ui
                    .button(format!("Download {} Selected Files", selected_count))
                    .clicked()
                {
                    self.start_download();
                }
            });

            if self.download_in_progress {
                ui.spinner();
                ui.label("Download in progress...");
            }
        });
    }

    fn get_file_icon(&self, format: &Option<String>) -> &'static str {
        match format.as_ref().map(|s| s.as_str()) {
            Some("JPEG") | Some("PNG") | Some("GIF") | Some("TIFF") => "🖼️",
            Some("MP4") | Some("AVI") | Some("MKV") | Some("MOV") => "🎬",
            Some("MP3") | Some("FLAC") | Some("WAV") | Some("OGG") => "🎵",
            Some("PDF") => "📄",
            Some("ZIP") | Some("RAR") | Some("7Z") | Some("TAR") => "📦",
            Some("TXT") | Some("RTF") => "📝",
            _ => "📄",
        }
    }

    fn fetch_archive_metadata(&mut self) {
        if let Some(rt_handle) = &self.rt_handle {
            self.is_loading = true;
            self.error_message = None;
            self.success_message = None;

            let identifier = self.archive_identifier.clone();

            rt_handle.spawn(async move {
                let _result = Self::fetch_metadata_async(identifier).await;
                // TODO: Send result back to UI through a proper channel
                // For now, this is a placeholder implementation
            });
        }
    }

    async fn fetch_metadata_async(identifier: String) -> Result<ArchiveMetadata, String> {
        let download_service = DownloadService::new()
            .map_err(|e| format!("Failed to create download service: {}", e))?;

        let request = DownloadRequest {
            identifier,
            dry_run: true,
            ..Default::default()
        };

        match download_service.download(request, None).await {
            Ok(crate::core::download::download_service::DownloadResult::Success(session, _, _)) => {
                Ok(session.archive_metadata)
            }
            Ok(crate::core::download::download_service::DownloadResult::Error(e)) => Err(e),
            Err(e) => Err(format!("Download failed: {}", e)),
        }
    }

    fn apply_filters(&mut self) {
        if let Some(ref metadata) = self.archive_metadata {
            self.filtered_files = metadata
                .files
                .iter()
                .filter(|file| {
                    // Search filter
                    if !self.search_filter.is_empty() {
                        if !file
                            .name
                            .to_lowercase()
                            .contains(&self.search_filter.to_lowercase())
                        {
                            return false;
                        }
                    }

                    // Format filter
                    if !self.format_filter.is_empty() {
                        if let Some(ref format) = file.format {
                            if !format
                                .to_lowercase()
                                .contains(&self.format_filter.to_lowercase())
                            {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }

                    // Size filters
                    if let Some(size) = file.size {
                        if !self.min_size_filter.is_empty() {
                            if let Ok(min_size) = self.min_size_filter.parse::<u64>() {
                                if size < min_size {
                                    return false;
                                }
                            }
                        }

                        if !self.max_size_filter.is_empty() {
                            if let Ok(max_size) = self.max_size_filter.parse::<u64>() {
                                if size > max_size {
                                    return false;
                                }
                            }
                        }
                    }

                    true
                })
                .cloned()
                .collect();

            self.build_file_tree();
        }
    }

    fn build_file_tree(&mut self) {
        let mut new_tree = FileTreeNode::default();

        for file in &self.filtered_files {
            Self::add_file_to_tree_static(&mut new_tree, file);
        }

        self.file_tree = new_tree;
    }

    fn add_file_to_tree_static(root: &mut FileTreeNode, file: &ArchiveFile) {
        let path_parts: Vec<&str> = file.name.split('/').collect();
        let mut current_node = root;

        for (i, part) in path_parts.iter().enumerate() {
            let is_last = i == path_parts.len() - 1;
            let full_path = path_parts[..=i].join("/");

            if is_last {
                // This is the file itself
                current_node.children.insert(
                    part.to_string(),
                    FileTreeNode {
                        name: part.to_string(),
                        full_path,
                        is_file: true,
                        file_info: Some(file.clone()),
                        children: HashMap::new(),
                        size: file.size,
                    },
                );
            } else {
                // This is a directory
                current_node
                    .children
                    .entry(part.to_string())
                    .or_insert_with(|| FileTreeNode {
                        name: part.to_string(),
                        full_path: full_path.clone(),
                        is_file: false,
                        file_info: None,
                        children: HashMap::new(),
                        size: None,
                    });

                current_node = current_node.children.get_mut(&part.to_string()).unwrap();
            }
        }
    }

    fn start_download(&mut self) {
        if self.selected_files.is_empty() {
            self.error_message = Some("No files selected for download".to_string());
            return;
        }

        self.download_in_progress = true;
        self.error_message = None;

        // This would integrate with the existing download system
        // For now, just show a message
        self.success_message = Some(format!(
            "Starting download of {} files...",
            self.selected_files.len()
        ));
    }

    pub fn set_archive_metadata(&mut self, metadata: ArchiveMetadata) {
        self.archive_metadata = Some(metadata.clone());
        self.filtered_files = metadata.files;
        self.build_file_tree();
        self.is_loading = false;
        self.success_message = Some(format!(
            "Loaded {} files from archive",
            self.filtered_files.len()
        ));
    }

    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
        self.is_loading = false;
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }
}
