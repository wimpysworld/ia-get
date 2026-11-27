//! Pure, terminal-independent state model for the archival inline instrument.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::downloader::SchedulerSnapshot;

/// Fixed number of throughput samples displayed in the header sparkline.
pub const SPARKLINE_SAMPLES: usize = 12;

/// Sampling window for each sparkline bar and EWMA aggregate throughput update.
const SPARKLINE_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// Frame interval for the animated MD5 half-circle spinner (~8 fps).
const MD5_SPINNER_INTERVAL: Duration = Duration::from_millis(125);

/// Angular frequency (rad/s) for critically damped bar easing (~250ms settle, zero overshoot).
const SPRING_OMEGA: f64 = 14.0;

/// Pending file awaiting a scheduler permit in the live queue preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedFile {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub permits: usize,
}

/// Active `-f/--filter` summary passed to `--list`, `--check`, and download receipts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterContext {
    /// Raw `-f/--filter` glob patterns supplied by the user.
    pub patterns: Vec<String>,
    /// Number of files matched after filtering.
    pub matched: usize,
    /// Total number of target files before `-f/--filter` was applied.
    pub total: usize,
}

impl FilterContext {
    /// Returns `true` when at least one `-f/--filter` pattern was supplied.
    pub fn is_active(&self) -> bool {
        !self.patterns.is_empty()
    }

    /// Formats `filter: all` or `filter: <patterns> (<matched> of <total>)`.
    pub fn label(&self) -> String {
        if self.patterns.is_empty() {
            "filter: all".to_string()
        } else {
            format!(
                "filter: {} ({} of {})",
                self.patterns.join(", "),
                self.matched,
                self.total
            )
        }
    }
}

/// Active transfer stage displayed as a high-visibility geometric glyph on each slot row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStage {
    /// Fresh HTTP GET streaming from offset 0.
    Get,
    /// HTTP Range resume from an existing `.part` offset.
    Resume,
    /// Following HTTP redirect(s) to an archive.org storage mirror.
    Redirect,
    /// Computing local MD5 digest for integrity verification.
    Md5,
    /// Host throttle / cooldown (HTTP 429 or 503).
    RateLimited,
}

/// Reason an existing file was skipped without network transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Local file existed with matching size and verified MD5 digest.
    AlreadyVerified,
    /// Local file existed with matching size (or no size/MD5 in metadata).
    AlreadyExists,
}

/// Outcome payload for a completed file slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionKind {
    /// File downloaded, verified (if MD5 available), and atomically promoted.
    Downloaded {
        bytes: u64,
        duration: Duration,
        verified_md5: bool,
    },
    /// File already existed locally and was skipped.
    Skipped { reason: SkipReason, bytes: u64 },
    /// Downloaded `.part` file failed MD5 verification and was kept unpromoted.
    HashMismatch { got: String, expected: String },
    /// Hard network, HTTP, containment, or filesystem error.
    Failed { message: String },
}

/// Permanent single-line completion record emitted above the live inline box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRecord {
    pub slot_id: u64,
    pub file_path: String,
    pub kind: CompletionKind,
    pub finished_index: usize,
    pub total_files: usize,
}

/// Structured item listed in the post-run `Needs attention` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionItem {
    /// Downloaded `.part` file's MD5 did not match archive.org metadata.
    HashMismatch {
        file_path: String,
        got: String,
        expected: String,
    },
    /// Private archive.org files skipped because no cookies were supplied.
    SkippedPrivate { count: usize },
    /// Hard transfer or filesystem error preventing file completion.
    TransferError { file_path: String, message: String },
}

/// Per-file active transfer slot shown inside the live instrument (up to 4).
#[derive(Debug, Clone)]
pub struct ActiveSlot {
    pub slot_id: u64,
    pub file_path: String,
    pub permit_cost: usize,
    pub stage: TransferStage,
    pub mirror_host: Option<String>,
    pub is_resuming: bool,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub counted_planned_bytes: u64,
    pub start_offset: u64,
    pub transfer_started_at: Instant,
    pub hash_bytes: u64,
    pub hash_total: Option<u64>,
    pub target_ratio: f64,
    pub display_ratio: f64,
    pub ratio_velocity: f64,
    pub speed_bps: f64,
    pub last_sample_bytes: u64,
    pub last_sample_at: Instant,
    pub cooldown_remaining: Option<Duration>,
    pub retry_attempt: Option<(u32, u32)>,
}

impl ActiveSlot {
    fn new(
        slot_id: u64,
        file_path: String,
        permit_cost: usize,
        total_bytes: Option<u64>,
        mirror_host: Option<String>,
        now: Instant,
    ) -> Self {
        Self {
            slot_id,
            file_path,
            permit_cost,
            stage: TransferStage::Get,
            mirror_host,
            is_resuming: false,
            downloaded_bytes: 0,
            total_bytes,
            counted_planned_bytes: total_bytes.unwrap_or(0),
            start_offset: 0,
            transfer_started_at: now,
            hash_bytes: 0,
            hash_total: None,
            target_ratio: 0.0,
            display_ratio: 0.0,
            ratio_velocity: 0.0,
            speed_bps: 0.0,
            last_sample_bytes: 0,
            last_sample_at: now,
            cooldown_remaining: None,
            retry_attempt: None,
        }
    }

    /// Returns the active display percentage (`0..=100`) derived from `display_ratio`.
    pub fn display_percent(&self) -> u8 {
        (self.display_ratio.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// Returns bytes transferred over the network during the current attempt (excluding resumed prefix).
    pub fn session_bytes(&self) -> u64 {
        self.downloaded_bytes.saturating_sub(self.start_offset)
    }

    /// Returns the estimated remaining seconds for this slot based on its post-resume baseline.
    pub fn eta_secs(&self) -> Option<u64> {
        let total = self.total_bytes?;
        let remaining = total.saturating_sub(self.downloaded_bytes);
        if remaining == 0 {
            return Some(0);
        }
        if self.speed_bps < 1.0 {
            return None;
        }
        Some((remaining as f64 / self.speed_bps).ceil() as u64)
    }
}

/// Non-blocking telemetry events emitted by download tasks to the UI task.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Initialises the ordered pending queue and formatted destination directory for the batch.
    BatchStarted {
        queue: Vec<QueuedFile>,
        output_dir: String,
    },
    /// Updated snapshot of scheduler concurrency and backpressure state.
    SchedulerUpdate(SchedulerSnapshot),
    /// A file acquired its scheduler permit and entered an active slot.
    SlotStarted {
        slot_id: u64,
        file_path: String,
        permit_cost: usize,
        total_bytes: Option<u64>,
        mirror_host: Option<String>,
    },
    /// A slot transitioned stage (e.g. Redirect, Md5, RateLimited, Get, Resume).
    SlotStage {
        slot_id: u64,
        stage: TransferStage,
        mirror_host: Option<String>,
        cooldown: Option<Duration>,
        retry: Option<(u32, u32)>,
    },
    /// Resets a slot's rate/ETA baseline at the start of a fresh or `.part`-resumed stream.
    SlotResumeBaseline {
        slot_id: u64,
        start_offset: u64,
        total_bytes: Option<u64>,
    },
    /// Network chunk progress for an active slot.
    SlotProgress {
        slot_id: u64,
        current_bytes: u64,
        total_bytes: Option<u64>,
        delta_network_bytes: u64,
    },
    /// Local MD5 verification progress for an active slot.
    SlotHashProgress {
        slot_id: u64,
        hashed_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// Slot completed with a permanent outcome (emits a completion line and updates counters).
    SlotFinished(CompletionRecord),
    /// Slot aborted due to user interruption (removed without emitting a completion line).
    SlotCancelled { slot_id: u64 },
    /// Records private files skipped before batch download due to missing cookies.
    SkippedPrivate { count: usize },
    /// Records specific private filenames skipped before batch download due to missing cookies.
    SkippedPrivateFiles(Vec<String>),
    /// Marks the batch as interrupted by user signal (Ctrl+C).
    Interrupted,
    /// Signals the render loop to flush remaining events, clear the inline box, and print summary.
    Finish,
}

/// Advances `value` smoothly toward `target` using an analytical critically damped spring.
///
/// Guarantees monotonic, overshoot-free convergence in ~250ms at 30 fps.
pub fn step_critically_damped(value: &mut f64, velocity: &mut f64, target: f64, dt_secs: f64) {
    let target = target.clamp(0.0, 1.0);
    let dt = dt_secs.clamp(0.0, 0.25);
    if dt == 0.0 {
        return;
    }

    let delta = *value - target;
    let exp = (-SPRING_OMEGA * dt).exp();
    let new_delta = (delta + (*velocity + SPRING_OMEGA * delta) * dt) * exp;
    let new_vel = (*velocity - (*velocity + SPRING_OMEGA * delta) * SPRING_OMEGA * dt) * exp;

    *value = (target + new_delta).clamp(0.0, 1.0);
    *velocity = new_vel;

    if (*value - target).abs() < 0.0005 && velocity.abs() < 0.001 {
        *value = target;
        *velocity = 0.0;
    }
}

/// Terminal-independent state of a live `ia-get` batch transfer session.
#[derive(Debug, Clone)]
pub struct UiState {
    pub identifier: String,
    pub output_dir: String,
    pub total_files: usize,
    pub planned_total_bytes: u64,
    pub finished_bytes: u64,
    pub failed_planned_bytes: u64,
    pub session_network_bytes: u64,
    pub downloaded_bytes_total: u64,
    pub ok_count: usize,
    pub skip_count: usize,
    pub warn_count: usize,
    pub fail_count: usize,
    pub skipped_private_count: usize,
    pub skipped_private_files: Vec<String>,
    pub filter: Option<FilterContext>,
    pub limit_rate: Option<u64>,
    pub scheduler: SchedulerSnapshot,
    pub active_slots: Vec<ActiveSlot>,
    pub pending_queue: VecDeque<QueuedFile>,
    pub attention_items: Vec<AttentionItem>,
    pub interrupted: bool,
    pub started_at: Instant,
    pub last_tick_at: Instant,
    pub finished_at: Option<Instant>,
    pub total_display_ratio: f64,
    pub total_ratio_velocity: f64,
    pub sparkline: VecDeque<u64>,
    pub run_speed_samples: Vec<u64>,
    pub aggregate_speed_bps: f64,
    pub peak_bytes_per_sec: f64,
    pub md5_spinner_frame: usize,
    md5_spinner_elapsed: Duration,
    spark_window_bytes: u64,
    spark_window_elapsed: Duration,
    pub eta_secs: Option<u64>,
    dirty: bool,
}

impl UiState {
    /// Creates a new session state for an archive.org batch download.
    pub fn new(
        identifier: impl Into<String>,
        total_files: usize,
        planned_total_bytes: u64,
        limit_rate: Option<u64>,
        now: Instant,
    ) -> Self {
        let mut sparkline = VecDeque::with_capacity(SPARKLINE_SAMPLES);
        sparkline.resize(SPARKLINE_SAMPLES, 0);
        Self {
            identifier: identifier.into(),
            output_dir: ".".to_string(),
            total_files,
            planned_total_bytes,
            finished_bytes: 0,
            failed_planned_bytes: 0,
            session_network_bytes: 0,
            downloaded_bytes_total: 0,
            ok_count: 0,
            skip_count: 0,
            warn_count: 0,
            fail_count: 0,
            skipped_private_count: 0,
            skipped_private_files: Vec::new(),
            filter: None,
            limit_rate,
            scheduler: SchedulerSnapshot::default(),
            active_slots: Vec::with_capacity(4),
            pending_queue: VecDeque::new(),
            attention_items: Vec::new(),
            interrupted: false,
            started_at: now,
            last_tick_at: now,
            finished_at: None,
            total_display_ratio: 0.0,
            total_ratio_velocity: 0.0,
            sparkline,
            run_speed_samples: Vec::new(),
            aggregate_speed_bps: 0.0,
            peak_bytes_per_sec: 0.0,
            md5_spinner_frame: 0,
            md5_spinner_elapsed: Duration::ZERO,
            spark_window_bytes: 0,
            spark_window_elapsed: Duration::ZERO,
            eta_secs: None,
            dirty: true,
        }
    }

    /// Returns whether the rendered frame needs to be redrawn.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after rendering a frame.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Returns `true` if any bar easing, MD5 spinner, or active cooldown requires further frame ticks.
    pub fn is_animating(&self) -> bool {
        let total_diff = (self.total_display_ratio - self.total_target_ratio()).abs();
        if total_diff >= 0.0005 || self.total_ratio_velocity.abs() >= 0.001 {
            return true;
        }
        if self.scheduler.cooldown_remaining.is_some() {
            return true;
        }
        self.active_slots.iter().any(|slot| {
            (slot.display_ratio - slot.target_ratio).abs() >= 0.0005
                || slot.ratio_velocity.abs() >= 0.001
                || slot.cooldown_remaining.is_some()
                || slot.stage == TransferStage::Md5
        })
    }

    /// Returns elapsed session duration (frozen at `finished_at` once finished).
    pub fn elapsed(&self) -> Duration {
        self.finished_at
            .unwrap_or(self.last_tick_at)
            .saturating_duration_since(self.started_at)
    }

    /// Returns total bytes downloaded over the network in this session (with fallback to completed byte counts).
    pub fn total_bytes_downloaded(&self) -> u64 {
        if self.session_network_bytes > 0 {
            self.session_network_bytes
        } else {
            self.downloaded_bytes_total
        }
    }

    /// Returns full-run average transfer speed in bytes/sec.
    pub fn average_speed_bps(&self) -> f64 {
        let bytes = self.total_bytes_downloaded();
        let secs = self.elapsed().as_secs_f64();
        if bytes == 0 || secs <= 0.0 {
            0.0
        } else {
            bytes as f64 / secs
        }
    }

    /// Returns the number of MD5 hash mismatches recorded (excluding skipped private warnings).
    pub fn hash_mismatch_count(&self) -> usize {
        self.warn_count.saturating_sub(self.skipped_private_count)
    }

    /// Downsamples (or stretches for short runs) the full-run throughput history to 12 buckets.
    pub fn downsampled_run_sparkline(&self) -> [u64; SPARKLINE_SAMPLES] {
        let n = self.run_speed_samples.len();
        if n == 0 {
            return [0; SPARKLINE_SAMPLES];
        }
        let mut out = [0u64; SPARKLINE_SAMPLES];
        if n == SPARKLINE_SAMPLES {
            out.copy_from_slice(&self.run_speed_samples);
        } else if n > SPARKLINE_SAMPLES {
            for (i, bucket) in out.iter_mut().enumerate() {
                let start = i * n / SPARKLINE_SAMPLES;
                let end = ((i + 1) * n / SPARKLINE_SAMPLES).max(start + 1).min(n);
                let sum: u128 = self.run_speed_samples[start..end]
                    .iter()
                    .map(|&v| u128::from(v))
                    .sum();
                *bucket = (sum / (end - start) as u128) as u64;
            }
        } else {
            for (i, bucket) in out.iter_mut().enumerate() {
                *bucket = self.run_speed_samples[i * n / SPARKLINE_SAMPLES];
            }
        }
        out
    }

    /// Returns total bytes currently accounted for (completed + skipped + active slots).
    pub fn aggregate_current_bytes(&self) -> u64 {
        let active: u64 = self
            .active_slots
            .iter()
            .map(|slot| slot.downloaded_bytes)
            .sum();
        self.finished_bytes.saturating_add(active)
    }

    /// Returns the total byte denominator across the batch (never less than current bytes).
    pub fn aggregate_total_bytes(&self) -> u64 {
        self.planned_total_bytes.max(self.aggregate_current_bytes())
    }

    /// Returns the target progress ratio (`0.0..=1.0`) for the `TOTAL` bar.
    pub fn total_target_ratio(&self) -> f64 {
        let total = self.aggregate_total_bytes();
        if total > 0 {
            let accounted = self
                .aggregate_current_bytes()
                .saturating_add(self.failed_planned_bytes);
            (accounted as f64 / total as f64).clamp(0.0, 1.0)
        } else if self.total_files > 0 {
            let hash_mismatches = self.warn_count.saturating_sub(self.skipped_private_count);
            let resolved = self.ok_count + self.skip_count + self.fail_count + hash_mismatches;
            (resolved as f64 / self.total_files as f64).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Returns the smoothed display percentage (`0..=100`) for the `TOTAL` bar.
    pub fn total_display_percent(&self) -> u8 {
        (self.total_display_ratio.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// Pushes one throughput sample (bytes/sec) into the 12-sample sparkline ring buffer
    /// and the full-run throughput history, updating peak speed.
    pub fn record_spark_sample(&mut self, bytes_per_sec: u64) {
        if self.sparkline.len() >= SPARKLINE_SAMPLES {
            self.sparkline.pop_front();
        }
        self.sparkline.push_back(bytes_per_sec);
        self.run_speed_samples.push(bytes_per_sec);
        if (bytes_per_sec as f64) > self.peak_bytes_per_sec {
            self.peak_bytes_per_sec = bytes_per_sec as f64;
        }
        self.dirty = true;
    }

    fn remove_from_pending_queue(&mut self, file_path: &str) {
        if let Some(pos) = self.pending_queue.iter().position(|q| q.name == file_path) {
            self.pending_queue.remove(pos);
        }
    }

    fn adjust_planned_total_for_slot(&mut self, slot_idx: usize, new_total: u64) {
        let slot = &mut self.active_slots[slot_idx];
        if new_total > slot.counted_planned_bytes {
            let extra = new_total - slot.counted_planned_bytes;
            self.planned_total_bytes = self.planned_total_bytes.saturating_add(extra);
            slot.counted_planned_bytes = new_total;
        }
    }

    /// Applies one telemetry event at `now`.
    pub fn on_event(&mut self, event: UiEvent, now: Instant) {
        match event {
            UiEvent::BatchStarted { queue, output_dir } => {
                self.pending_queue = queue
                    .into_iter()
                    .filter(|item| !self.active_slots.iter().any(|s| s.file_path == item.name))
                    .collect();
                if !output_dir.is_empty() {
                    self.output_dir = output_dir;
                }
                self.dirty = true;
            }
            UiEvent::SchedulerUpdate(snapshot) => {
                if self.scheduler != snapshot {
                    self.scheduler = snapshot;
                    self.dirty = true;
                }
            }
            UiEvent::SlotStarted {
                slot_id,
                file_path,
                permit_cost,
                total_bytes,
                mirror_host,
            } => {
                self.remove_from_pending_queue(&file_path);
                if let Some(existing) = self
                    .active_slots
                    .iter_mut()
                    .find(|slot| slot.slot_id == slot_id)
                {
                    existing.file_path = file_path;
                    existing.permit_cost = permit_cost;
                    existing.total_bytes = total_bytes;
                    existing.mirror_host = mirror_host;
                } else {
                    self.active_slots.push(ActiveSlot::new(
                        slot_id,
                        file_path,
                        permit_cost,
                        total_bytes,
                        mirror_host,
                        now,
                    ));
                }
                self.dirty = true;
            }
            UiEvent::SlotStage {
                slot_id,
                stage,
                mirror_host,
                cooldown,
                retry,
            } => {
                if let Some(slot) = self
                    .active_slots
                    .iter_mut()
                    .find(|slot| slot.slot_id == slot_id)
                {
                    let prev_stage = slot.stage;
                    slot.stage = stage;
                    if mirror_host.is_some() {
                        slot.mirror_host = mirror_host;
                    }
                    slot.cooldown_remaining = cooldown;
                    slot.retry_attempt = retry;
                    if stage == TransferStage::Md5 && prev_stage != TransferStage::Md5 {
                        slot.hash_bytes = 0;
                        slot.hash_total = slot.total_bytes.or(Some(slot.downloaded_bytes));
                        slot.target_ratio = 0.0;
                        slot.display_ratio = 0.0;
                        slot.ratio_velocity = 0.0;
                    }
                    self.dirty = true;
                }
            }
            UiEvent::SlotResumeBaseline {
                slot_id,
                start_offset,
                total_bytes,
            } => {
                if let Some(idx) = self
                    .active_slots
                    .iter()
                    .position(|slot| slot.slot_id == slot_id)
                {
                    if let Some(total) = total_bytes {
                        self.adjust_planned_total_for_slot(idx, total);
                    }
                    let slot = &mut self.active_slots[idx];
                    slot.start_offset = start_offset;
                    slot.downloaded_bytes = start_offset;
                    slot.total_bytes = total_bytes.or(slot.total_bytes);
                    slot.is_resuming = start_offset > 0;
                    slot.stage = if start_offset > 0 {
                        TransferStage::Resume
                    } else {
                        TransferStage::Get
                    };
                    slot.cooldown_remaining = None;
                    // Reset ETA and rate baselines so pre-existing .part bytes do not distort speed.
                    slot.transfer_started_at = now;
                    slot.last_sample_bytes = 0;
                    slot.last_sample_at = now;
                    slot.speed_bps = 0.0;
                    let ratio = match slot.total_bytes {
                        Some(total) if total > 0 => {
                            (start_offset as f64 / total as f64).clamp(0.0, 1.0)
                        }
                        _ => 0.0,
                    };
                    slot.target_ratio = ratio;
                    slot.display_ratio = ratio;
                    slot.ratio_velocity = 0.0;
                    self.dirty = true;
                }
            }
            UiEvent::SlotProgress {
                slot_id,
                current_bytes,
                total_bytes,
                delta_network_bytes,
            } => {
                self.session_network_bytes = self
                    .session_network_bytes
                    .saturating_add(delta_network_bytes);
                self.spark_window_bytes =
                    self.spark_window_bytes.saturating_add(delta_network_bytes);

                if let Some(idx) = self
                    .active_slots
                    .iter()
                    .position(|slot| slot.slot_id == slot_id)
                {
                    let effective_total = total_bytes.or(self.active_slots[idx].total_bytes);
                    if let Some(total) = effective_total {
                        self.adjust_planned_total_for_slot(idx, total.max(current_bytes));
                    }
                    let slot = &mut self.active_slots[idx];
                    slot.downloaded_bytes = current_bytes;
                    slot.total_bytes = effective_total;
                    if slot.stage == TransferStage::Redirect
                        || slot.stage == TransferStage::RateLimited
                    {
                        slot.stage = if slot.is_resuming {
                            TransferStage::Resume
                        } else {
                            TransferStage::Get
                        };
                        slot.cooldown_remaining = None;
                    }
                    let session_bytes = slot.session_bytes();
                    let sample_dt = now.saturating_duration_since(slot.last_sample_at);
                    if sample_dt >= Duration::from_millis(200) {
                        let delta_since_sample =
                            session_bytes.saturating_sub(slot.last_sample_bytes);
                        let inst_bps = delta_since_sample as f64 / sample_dt.as_secs_f64();
                        slot.speed_bps = if slot.speed_bps <= 0.0 {
                            inst_bps
                        } else {
                            0.35 * inst_bps + 0.65 * slot.speed_bps
                        };
                        slot.last_sample_bytes = session_bytes;
                        slot.last_sample_at = now;
                    } else if slot.speed_bps <= 0.0 {
                        let elapsed = now.saturating_duration_since(slot.transfer_started_at);
                        if elapsed >= Duration::from_millis(50) {
                            slot.speed_bps = session_bytes as f64 / elapsed.as_secs_f64();
                        }
                    }

                    if let Some(total) = slot.total_bytes.filter(|&t| t > 0) {
                        let new_ratio = (current_bytes as f64 / total as f64).clamp(0.0, 1.0);
                        if new_ratio < slot.target_ratio {
                            slot.display_ratio = new_ratio;
                            slot.ratio_velocity = 0.0;
                        }
                        slot.target_ratio = new_ratio;
                    }
                    self.dirty = true;
                }
            }
            UiEvent::SlotHashProgress {
                slot_id,
                hashed_bytes,
                total_bytes,
            } => {
                if let Some(slot) = self
                    .active_slots
                    .iter_mut()
                    .find(|slot| slot.slot_id == slot_id)
                {
                    slot.stage = TransferStage::Md5;
                    slot.hash_bytes = hashed_bytes;
                    slot.hash_total = total_bytes.or(slot.hash_total).or(slot.total_bytes);
                    if let Some(total) = slot.hash_total.filter(|&t| t > 0) {
                        slot.target_ratio = (hashed_bytes as f64 / total as f64).clamp(0.0, 1.0);
                    }
                    self.dirty = true;
                }
            }
            UiEvent::SlotFinished(record) => {
                let removed = self
                    .active_slots
                    .iter()
                    .position(|slot| slot.slot_id == record.slot_id)
                    .map(|idx| self.active_slots.remove(idx));
                if removed.is_none() {
                    self.remove_from_pending_queue(&record.file_path);
                }

                let counted_planned = removed
                    .as_ref()
                    .map_or(0, |slot| slot.counted_planned_bytes);
                match record.kind {
                    CompletionKind::Downloaded { bytes, .. } => {
                        self.ok_count += 1;
                        self.downloaded_bytes_total =
                            self.downloaded_bytes_total.saturating_add(bytes);
                        let final_bytes = bytes.max(counted_planned);
                        if final_bytes > counted_planned {
                            self.planned_total_bytes = self
                                .planned_total_bytes
                                .saturating_add(final_bytes - counted_planned);
                        }
                        self.finished_bytes = self.finished_bytes.saturating_add(final_bytes);
                    }
                    CompletionKind::Skipped { bytes, .. } => {
                        self.skip_count += 1;
                        let final_bytes = bytes.max(counted_planned);
                        if final_bytes > counted_planned {
                            self.planned_total_bytes = self
                                .planned_total_bytes
                                .saturating_add(final_bytes - counted_planned);
                        }
                        self.finished_bytes = self.finished_bytes.saturating_add(final_bytes);
                    }
                    CompletionKind::HashMismatch { got, expected } => {
                        self.warn_count += 1;
                        self.failed_planned_bytes =
                            self.failed_planned_bytes.saturating_add(counted_planned);
                        self.attention_items.push(AttentionItem::HashMismatch {
                            file_path: record.file_path,
                            got,
                            expected,
                        });
                    }
                    CompletionKind::Failed { message } => {
                        self.fail_count += 1;
                        self.failed_planned_bytes =
                            self.failed_planned_bytes.saturating_add(counted_planned);
                        self.attention_items.push(AttentionItem::TransferError {
                            file_path: record.file_path,
                            message,
                        });
                    }
                }
                self.dirty = true;
            }
            UiEvent::SlotCancelled { slot_id } => {
                if let Some(idx) = self
                    .active_slots
                    .iter()
                    .position(|slot| slot.slot_id == slot_id)
                {
                    self.active_slots.remove(idx);
                    self.dirty = true;
                }
            }
            UiEvent::SkippedPrivate { count } => {
                if count > 0 {
                    self.skipped_private_count += count;
                    self.warn_count += count;
                    if let Some(AttentionItem::SkippedPrivate { count: existing }) = self
                        .attention_items
                        .iter_mut()
                        .find(|item| matches!(item, AttentionItem::SkippedPrivate { .. }))
                    {
                        *existing += count;
                    } else {
                        self.attention_items
                            .push(AttentionItem::SkippedPrivate { count });
                    }
                    self.dirty = true;
                }
            }
            UiEvent::SkippedPrivateFiles(files) => {
                let count = files.len();
                if count > 0 {
                    self.skipped_private_files.extend(files);
                    self.skipped_private_count += count;
                    self.warn_count += count;
                    if let Some(AttentionItem::SkippedPrivate { count: existing }) = self
                        .attention_items
                        .iter_mut()
                        .find(|item| matches!(item, AttentionItem::SkippedPrivate { .. }))
                    {
                        *existing += count;
                    } else {
                        self.attention_items
                            .push(AttentionItem::SkippedPrivate { count });
                    }
                    self.dirty = true;
                }
            }
            UiEvent::Interrupted => {
                self.interrupted = true;
                self.active_slots.clear();
                self.dirty = true;
            }
            UiEvent::Finish => {
                self.finished_at = Some(now);
                let tail_elapsed = self
                    .spark_window_elapsed
                    .saturating_add(now.saturating_duration_since(self.last_tick_at));
                if self.spark_window_bytes > 0 {
                    if tail_elapsed >= SPARKLINE_SAMPLE_INTERVAL / 2 {
                        let secs = tail_elapsed.as_secs_f64();
                        let bps = (self.spark_window_bytes as f64 / secs).round() as u64;
                        self.record_spark_sample(bps);
                    }
                    self.spark_window_bytes = 0;
                    self.spark_window_elapsed = Duration::ZERO;
                }
                let avg = self.average_speed_bps();
                if avg > self.peak_bytes_per_sec {
                    self.peak_bytes_per_sec = avg;
                }
                if self.run_speed_samples.is_empty() && avg >= 1.0 {
                    self.record_spark_sample(avg.round() as u64);
                }
                self.dirty = true;
            }
        }
    }

    /// Advances smooth bar easing, MD5 spinner, sparkline throughput sampling, and whole-second ETA.
    pub fn tick(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.last_tick_at);
        self.last_tick_at = now;
        let dt_secs = dt.as_secs_f64();

        if self
            .active_slots
            .iter()
            .any(|slot| slot.stage == TransferStage::Md5)
        {
            self.md5_spinner_elapsed = self.md5_spinner_elapsed.saturating_add(dt);
            if self.md5_spinner_elapsed >= MD5_SPINNER_INTERVAL {
                let interval_ms = MD5_SPINNER_INTERVAL.as_millis();
                let elapsed_ms = self.md5_spinner_elapsed.as_millis();
                let steps = (elapsed_ms / interval_ms) as usize;
                self.md5_spinner_frame = (self.md5_spinner_frame + steps.max(1)) % 4;
                self.md5_spinner_elapsed = Duration::from_millis((elapsed_ms % interval_ms) as u64);
                self.dirty = true;
            }
        }

        let total_target = self.total_target_ratio();
        let prev_total = self.total_display_ratio;
        step_critically_damped(
            &mut self.total_display_ratio,
            &mut self.total_ratio_velocity,
            total_target,
            dt_secs,
        );
        if (self.total_display_ratio - prev_total).abs() >= 0.0005 {
            self.dirty = true;
        }

        for slot in &mut self.active_slots {
            let prev = slot.display_ratio;
            step_critically_damped(
                &mut slot.display_ratio,
                &mut slot.ratio_velocity,
                slot.target_ratio,
                dt_secs,
            );
            if (slot.display_ratio - prev).abs() >= 0.0005 {
                self.dirty = true;
            }
            if let Some(cd) = slot.cooldown_remaining {
                let remaining = cd.saturating_sub(dt);
                slot.cooldown_remaining = if remaining.is_zero() {
                    None
                } else {
                    Some(remaining)
                };
                self.dirty = true;
            }
        }

        self.spark_window_elapsed = self.spark_window_elapsed.saturating_add(dt);
        if self.spark_window_elapsed >= SPARKLINE_SAMPLE_INTERVAL {
            let secs = self.spark_window_elapsed.as_secs_f64();
            let window_bps = if secs > 0.0 {
                self.spark_window_bytes as f64 / secs
            } else {
                0.0
            };
            self.spark_window_bytes = 0;
            self.spark_window_elapsed = Duration::ZERO;

            self.aggregate_speed_bps = if self.aggregate_speed_bps <= 0.0 {
                window_bps
            } else if window_bps == 0.0 && self.active_slots.is_empty() {
                0.0
            } else {
                0.35 * window_bps + 0.65 * self.aggregate_speed_bps
            };

            self.record_spark_sample(window_bps.round() as u64);

            let remaining = self
                .aggregate_total_bytes()
                .saturating_sub(self.aggregate_current_bytes())
                .saturating_sub(self.failed_planned_bytes);
            let new_eta = if remaining == 0 {
                Some(0)
            } else if self.aggregate_speed_bps >= 1.0 {
                Some((remaining as f64 / self.aggregate_speed_bps).ceil() as u64)
            } else {
                None
            };
            if new_eta != self.eta_secs {
                self.eta_secs = new_eta;
                self.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critically_damped_spring_converges_smoothly_without_overshoot() {
        let mut value = 0.0;
        let mut velocity = 0.0;
        let target = 0.75;

        for _ in 0..30 {
            let prev = value;
            step_critically_damped(&mut value, &mut velocity, target, 1.0 / 30.0);
            assert!(
                value >= prev,
                "spring must increase monotonically: {prev} -> {value}"
            );
            assert!(
                value <= target + 1e-6,
                "critically damped spring must not overshoot target {target}: got {value}"
            );
        }
        assert!(
            (value - target).abs() < 0.01,
            "spring should settle within ~1s: value={value}"
        );
    }

    #[test]
    fn sparkline_ring_buffer_keeps_fixed_12_samples() {
        let now = Instant::now();
        let mut state = UiState::new("item", 1, 1000, None, now);
        assert_eq!(state.sparkline.len(), SPARKLINE_SAMPLES);

        for i in 1..=20u64 {
            state.record_spark_sample(i * 100);
        }
        assert_eq!(state.sparkline.len(), SPARKLINE_SAMPLES);
        let samples: Vec<u64> = state.sparkline.iter().copied().collect();
        assert_eq!(
            samples,
            vec![900, 1000, 1100, 1200, 1300, 1400, 1500, 1600, 1700, 1800, 1900, 2000]
        );
    }

    #[test]
    fn resume_baseline_resets_per_slot_speed_and_eta_without_counting_prefix_as_network_rate() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 1, 1000, None, t0);
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "archive.pdf".to_string(),
                permit_cost: 2,
                total_bytes: Some(1000),
                mirror_host: Some("ia801504".to_string()),
            },
            t0,
        );

        // Resuming from 600 of 1000 bytes snaps display ratio to 0.6 and resets speed baseline.
        state.on_event(
            UiEvent::SlotResumeBaseline {
                slot_id: 1,
                start_offset: 600,
                total_bytes: Some(1000),
            },
            t0,
        );
        let slot = &state.active_slots[0];
        assert!(slot.is_resuming);
        assert_eq!(slot.stage, TransferStage::Resume);
        assert_eq!(slot.downloaded_bytes, 600);
        assert_eq!(slot.session_bytes(), 0);
        assert_eq!(slot.speed_bps, 0.0);
        assert!((slot.display_ratio - 0.6).abs() < 1e-6);
        assert_eq!(state.session_network_bytes, 0);

        // 1 second later, 100 new bytes arrive -> speed is 100 B/s (not 700 B/s), ETA is 3s for remaining 300B.
        let t1 = t0 + Duration::from_secs(1);
        state.on_event(
            UiEvent::SlotProgress {
                slot_id: 1,
                current_bytes: 700,
                total_bytes: Some(1000),
                delta_network_bytes: 100,
            },
            t1,
        );
        let slot = &state.active_slots[0];
        assert_eq!(slot.session_bytes(), 100);
        assert!((slot.speed_bps - 100.0).abs() < 1.0);
        assert_eq!(slot.eta_secs(), Some(3));
        assert_eq!(state.session_network_bytes, 100);
    }

    #[test]
    fn counters_and_attention_items_classify_ok_skip_warn_and_fail() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 4, 400, None, t0);

        state.on_event(UiEvent::SkippedPrivate { count: 2 }, t0);
        assert_eq!(state.warn_count, 2);
        assert_eq!(state.fail_count, 0);

        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 1,
                file_path: "good.bin".to_string(),
                kind: CompletionKind::Downloaded {
                    bytes: 100,
                    duration: Duration::from_millis(250),
                    verified_md5: true,
                },
                finished_index: 1,
                total_files: 4,
            }),
            t0,
        );
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 2,
                file_path: "cached.bin".to_string(),
                kind: CompletionKind::Skipped {
                    reason: SkipReason::AlreadyVerified,
                    bytes: 100,
                },
                finished_index: 2,
                total_files: 4,
            }),
            t0,
        );
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 3,
                file_path: "stale.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: "aaaa".to_string(),
                    expected: "bbbb".to_string(),
                },
                finished_index: 3,
                total_files: 4,
            }),
            t0,
        );
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 4,
                file_path: "missing.bin".to_string(),
                kind: CompletionKind::Failed {
                    message: "HTTP error 404".to_string(),
                },
                finished_index: 4,
                total_files: 4,
            }),
            t0,
        );

        assert_eq!(state.ok_count, 1);
        assert_eq!(state.skip_count, 1);
        assert_eq!(state.warn_count, 3); // 2 skipped private + 1 hash mismatch
        assert_eq!(state.fail_count, 1);
        assert_eq!(state.attention_items.len(), 3);
    }

    #[test]
    fn pending_queue_removes_files_on_start_and_finish_and_md5_spinner_advances() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 3, 3000, None, t0);
        state.on_event(
            UiEvent::BatchStarted {
                queue: vec![
                    QueuedFile {
                        name: "a.jpg".to_string(),
                        size_bytes: Some(1000),
                        permits: 1,
                    },
                    QueuedFile {
                        name: "b.pdf".to_string(),
                        size_bytes: Some(2000),
                        permits: 2,
                    },
                    QueuedFile {
                        name: "c.zip".to_string(),
                        size_bytes: None,
                        permits: 4,
                    },
                ],
                output_dir: "~/Downloads/item".to_string(),
            },
            t0,
        );
        assert_eq!(state.output_dir, "~/Downloads/item");
        assert_eq!(state.pending_queue.len(), 3);

        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "a.jpg".to_string(),
                permit_cost: 1,
                total_bytes: Some(1000),
                mirror_host: None,
            },
            t0,
        );
        assert_eq!(state.pending_queue.len(), 2);
        assert_eq!(state.pending_queue[0].name, "b.pdf");

        state.on_event(
            UiEvent::SlotStage {
                slot_id: 1,
                stage: TransferStage::Md5,
                mirror_host: None,
                cooldown: None,
                retry: None,
            },
            t0,
        );
        assert!(state.is_animating());
        assert_eq!(state.md5_spinner_frame, 0);
        state.tick(t0 + Duration::from_millis(125));
        assert_eq!(state.md5_spinner_frame, 1);
        state.tick(t0 + Duration::from_millis(250));
        assert_eq!(state.md5_spinner_frame, 2);
        state.tick(t0 + Duration::from_millis(375));
        assert_eq!(state.md5_spinner_frame, 3);
        state.tick(t0 + Duration::from_millis(500));
        assert_eq!(state.md5_spinner_frame, 0);

        // Finishing a file directly also removes it from pending_queue if still present.
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 2,
                file_path: "b.pdf".to_string(),
                kind: CompletionKind::Skipped {
                    reason: SkipReason::AlreadyVerified,
                    bytes: 2000,
                },
                finished_index: 1,
                total_files: 3,
            }),
            t0 + Duration::from_millis(600),
        );
        assert_eq!(state.pending_queue.len(), 1);
        assert_eq!(state.pending_queue[0].name, "c.zip");
    }

    #[test]
    fn full_run_sparkline_downsamples_long_runs_and_tracks_peak_speed() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 2, 2_000_000, None, t0);
        for i in 1..=24u64 {
            state.record_spark_sample(i * 100_000);
        }
        assert_eq!(state.peak_bytes_per_sec, 2_400_000.0);
        let downsampled = state.downsampled_run_sparkline();
        assert_eq!(downsampled.len(), SPARKLINE_SAMPLES);
        assert_eq!(downsampled[0], 150_000);
        assert_eq!(downsampled[11], 2_350_000);
    }

    #[test]
    fn finish_ignores_sub_quarter_second_tail_flush_to_prevent_fake_peak_spike() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 1, 2_000_000, None, t0);
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "scan.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(2_000_000),
                mirror_host: None,
            },
            t0,
        );

        // Two normal 500ms windows at 1 MB/s each (500 KB per 500ms).
        state.on_event(
            UiEvent::SlotProgress {
                slot_id: 1,
                current_bytes: 500_000,
                total_bytes: Some(2_000_000),
                delta_network_bytes: 500_000,
            },
            t0 + Duration::from_millis(500),
        );
        state.tick(t0 + Duration::from_millis(500));

        state.on_event(
            UiEvent::SlotProgress {
                slot_id: 1,
                current_bytes: 1_000_000,
                total_bytes: Some(2_000_000),
                delta_network_bytes: 500_000,
            },
            t0 + Duration::from_millis(1000),
        );
        state.tick(t0 + Duration::from_millis(1000));

        // 2ms later, a 100 KB final buffer flush arrives right before Finish (would be 50 MB/s!).
        state.on_event(
            UiEvent::SlotProgress {
                slot_id: 1,
                current_bytes: 1_100_000,
                total_bytes: Some(2_000_000),
                delta_network_bytes: 100_000,
            },
            t0 + Duration::from_millis(1002),
        );
        state.on_event(UiEvent::Finish, t0 + Duration::from_millis(1002));

        assert_eq!(state.run_speed_samples.len(), 2);
        assert!(
            state.peak_bytes_per_sec < 1_200_000.0,
            "2ms tail flush must not create a fake peak spike: {}",
            state.peak_bytes_per_sec
        );
    }

    #[test]
    fn slot_finished_for_active_slot_does_not_evict_duplicate_pending_filename() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 2, 2000, None, t0);
        state.on_event(
            UiEvent::BatchStarted {
                queue: vec![
                    QueuedFile {
                        name: "dup.bin".to_string(),
                        size_bytes: Some(1000),
                        permits: 1,
                    },
                    QueuedFile {
                        name: "dup.bin".to_string(),
                        size_bytes: Some(1000),
                        permits: 1,
                    },
                ],
                output_dir: ".".to_string(),
            },
            t0,
        );
        assert_eq!(state.pending_queue.len(), 2);

        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "dup.bin".to_string(),
                permit_cost: 1,
                total_bytes: Some(1000),
                mirror_host: None,
            },
            t0,
        );
        assert_eq!(state.pending_queue.len(), 1);

        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 1,
                file_path: "dup.bin".to_string(),
                kind: CompletionKind::Downloaded {
                    bytes: 1000,
                    duration: Duration::from_millis(100),
                    verified_md5: true,
                },
                finished_index: 1,
                total_files: 2,
            }),
            t0 + Duration::from_millis(100),
        );
        assert_eq!(
            state.pending_queue.len(),
            1,
            "finishing an active slot must not evict a second still-queued duplicate filename"
        );
    }
}
