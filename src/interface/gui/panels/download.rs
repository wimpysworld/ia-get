//! Download panel for real-time progress tracking

use egui::Ui;

#[derive(Default)]
pub struct DownloadPanel {
    // Progress tracking
    current_file: String,
    total_files: usize,
    completed_files: usize,
    failed_files: usize,
    current_speed: f64,
    eta: String,
}

impl DownloadPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn render(&mut self, ui: &mut Ui) {
        ui.heading("Download Progress");

        if self.total_files > 0 {
            // Overall progress
            let progress = self.completed_files as f32 / self.total_files as f32;

            ui.add(
                egui::ProgressBar::new(progress)
                    .show_percentage()
                    .text(format!(
                        "{}/{} files",
                        self.completed_files, self.total_files
                    )),
            );

            ui.add_space(10.0);

            // Current file
            if !self.current_file.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Current file:");
                    ui.label(&self.current_file);
                });
            }

            // Statistics
            ui.group(|ui| {
                ui.label("Statistics");

                ui.horizontal(|ui| {
                    ui.label(format!("Completed: {}", self.completed_files));
                    ui.label(format!("Failed: {}", self.failed_files));
                    ui.label(format!(
                        "Remaining: {}",
                        self.total_files - self.completed_files - self.failed_files
                    ));
                });

                if self.current_speed > 0.0 {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Speed: {:.2} MB/s",
                            self.current_speed / 1_000_000.0
                        ));
                        if !self.eta.is_empty() {
                            ui.label(format!("ETA: {}", self.eta));
                        }
                    });
                }
            });
        } else {
            ui.label("No active downloads");
        }
    }

    /// Update download progress
    pub fn update_progress(
        &mut self,
        current_file: String,
        total_files: usize,
        completed_files: usize,
        failed_files: usize,
        current_speed: f64,
        eta: String,
    ) {
        self.current_file = current_file;
        self.total_files = total_files;
        self.completed_files = completed_files;
        self.failed_files = failed_files;
        self.current_speed = current_speed;
        self.eta = eta;
    }

    /// Reset progress tracking
    pub fn reset(&mut self) {
        self.current_file.clear();
        self.total_files = 0;
        self.completed_files = 0;
        self.failed_files = 0;
        self.current_speed = 0.0;
        self.eta.clear();
    }
}
