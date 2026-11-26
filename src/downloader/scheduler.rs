//! Elastic permit scheduler for archive.org-compliant concurrent downloads.
//!
//! Small files (`< 8 MiB`) use 1 permit (up to 4 concurrent), medium files
//! (`8 MiB..64 MiB`) use 2 permits (up to 2 concurrent), and large or
//! unknown-size files (`>= 64 MiB` or `None`) use all 4 permits so they run
//! sequentially after smaller transfers drain. Reactive backpressure halves the
//! effective concurrency ceiling (`4 -> 2 -> 1`) on HTTP 429/503 responses and
//! shares a host cooldown across all tasks, recovering (`1 -> 2 -> 4`) after
//! four clean network completions per step.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::time::Instant;

use super::retry::{await_interruptible, interrupted_error, sleep_interruptible, ShutdownSignal};
use crate::Result;

/// Maximum concurrent download permits compliant with Internet Archive guidance.
pub const MAX_CONCURRENT_PERMITS: usize = 4;

/// Exclusive upper bound for small files (8 MiB): `< 8 MiB` costs 1 permit (up to 4 concurrent).
pub const SMALL_FILE_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Exclusive upper bound for medium files (64 MiB): `8 MiB..64 MiB` costs 2 permits (up to 2 concurrent).
pub const MEDIUM_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Clean network completions required while throttled before doubling `effective_ceiling`.
pub const BACKPRESSURE_RECOVERY_SUCCESSES: usize = 4;

/// Returns the base permit cost for a file given its expected size in bytes.
///
/// - `< 8 MiB`: 1 permit (up to 4 concurrent)
/// - `8 MiB..64 MiB`: 2 permits (up to 2 concurrent)
/// - `>= 64 MiB` or `None`: 4 permits (sequential)
pub fn base_permit_cost(expected_size: Option<u64>) -> usize {
    match expected_size {
        Some(size) if size < SMALL_FILE_MAX_BYTES => 1,
        Some(size) if size < MEDIUM_FILE_MAX_BYTES => 2,
        _ => MAX_CONCURRENT_PERMITS,
    }
}

/// Returns the sort key ordering downloads ascending by `expected_size`,
/// placing `None` at the tail and preserving `original_index` as tie-breaker.
pub fn download_sort_key(expected_size: Option<u64>, original_index: usize) -> (bool, u64, usize) {
    (
        expected_size.is_none(),
        expected_size.unwrap_or(u64::MAX),
        original_index,
    )
}

/// Sorts `(original_index, item)` entries in place ascending by `expected_size`
/// with `None` placed last and `original_index` breaking ties.
pub fn sort_planned_downloads<T, F>(items: &mut [(usize, T)], mut expected_size: F)
where
    F: FnMut(&T) -> Option<u64>,
{
    items.sort_by_key(|(original_index, item)| {
        download_sort_key(expected_size(item), *original_index)
    });
}

#[derive(Debug)]
struct SchedulerState {
    in_flight_units: usize,
    effective_ceiling: usize,
    clean_successes: usize,
    cooldown_until: Option<Instant>,
}

impl SchedulerState {
    fn new() -> Self {
        Self {
            in_flight_units: 0,
            effective_ceiling: MAX_CONCURRENT_PERMITS,
            clean_successes: 0,
            cooldown_until: None,
        }
    }

    fn effective_cost(&self, expected_size: Option<u64>) -> usize {
        let base_cost = base_permit_cost(expected_size);
        let floor_cost = MAX_CONCURRENT_PERMITS / self.effective_ceiling;
        base_cost.max(floor_cost)
    }
}

#[derive(Debug)]
struct SchedulerInner {
    /// Serialises permit acquisition in strict FIFO order so large files drain
    /// cleanly without starvation from later small files.
    fifo_gate: AsyncMutex<()>,
    state: StdMutex<SchedulerState>,
    notify: Notify,
}

impl SchedulerInner {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, SchedulerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Shared elastic funnel scheduler managing download concurrency and reactive backpressure.
#[derive(Clone, Debug)]
pub struct ElasticScheduler {
    inner: Arc<SchedulerInner>,
}

impl Default for ElasticScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl ElasticScheduler {
    /// Creates a scheduler starting at the full 4-permit concurrency ceiling.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                fifo_gate: AsyncMutex::new(()),
                state: StdMutex::new(SchedulerState::new()),
                notify: Notify::new(),
            }),
        }
    }

    /// Computes the current effective permit cost for `expected_size`.
    ///
    /// Always returns at most [`MAX_CONCURRENT_PERMITS`], guaranteeing that large
    /// files can never deadlock even when `effective_ceiling == 1`.
    pub fn effective_cost(&self, expected_size: Option<u64>) -> usize {
        self.inner.lock_state().effective_cost(expected_size)
    }

    /// Returns the current effective concurrency ceiling (`1`, `2`, or `4`).
    pub fn effective_ceiling(&self) -> usize {
        self.inner.lock_state().effective_ceiling
    }

    /// Returns the number of permit units currently held by in-flight transfers.
    pub fn in_flight_units(&self) -> usize {
        self.inner.lock_state().in_flight_units
    }

    /// Returns the count of clean network completions accumulated toward ceiling recovery.
    pub fn clean_successes(&self) -> usize {
        self.inner.lock_state().clean_successes
    }

    /// Returns the remaining shared cooldown duration, if a host cooldown is active.
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        let mut state = self.inner.lock_state();
        let now = Instant::now();
        match state.cooldown_until {
            Some(until) if until > now => Some(until.saturating_duration_since(now)),
            Some(_) => {
                state.cooldown_until = None;
                None
            }
            None => None,
        }
    }

    /// Waits until any active host cooldown has elapsed, waking immediately on `shutdown`.
    pub async fn wait_for_cooldown(&self, shutdown: &ShutdownSignal) -> Result<()> {
        loop {
            if !shutdown.is_running() {
                return Err(interrupted_error());
            }
            match self.cooldown_remaining() {
                Some(delay) => sleep_interruptible(delay, shutdown).await?,
                None => return Ok(()),
            }
        }
    }

    /// Acquires permit units for a file in FIFO dispatch order, waiting for any active
    /// host cooldown and sufficient permit capacity, cancellable by `shutdown`.
    pub async fn acquire(
        &self,
        expected_size: Option<u64>,
        shutdown: &ShutdownSignal,
    ) -> Result<SchedulerPermit> {
        let _fifo_guard = await_interruptible(self.inner.fifo_gate.lock(), shutdown).await?;

        loop {
            if !shutdown.is_running() {
                return Err(interrupted_error());
            }

            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let cooldown_wait = {
                let mut state = self.inner.lock_state();
                let now = Instant::now();
                if let Some(until) = state.cooldown_until {
                    if until > now {
                        Some(until.saturating_duration_since(now))
                    } else {
                        state.cooldown_until = None;
                        let cost = state.effective_cost(expected_size);
                        if state.in_flight_units + cost <= MAX_CONCURRENT_PERMITS {
                            state.in_flight_units += cost;
                            return Ok(SchedulerPermit {
                                inner: Arc::clone(&self.inner),
                                units: cost,
                            });
                        }
                        None
                    }
                } else {
                    let cost = state.effective_cost(expected_size);
                    if state.in_flight_units + cost <= MAX_CONCURRENT_PERMITS {
                        state.in_flight_units += cost;
                        return Ok(SchedulerPermit {
                            inner: Arc::clone(&self.inner),
                            units: cost,
                        });
                    }
                    None
                }
            };

            match cooldown_wait {
                Some(delay) => sleep_interruptible(delay, shutdown).await?,
                None => await_interruptible(notified, shutdown).await?,
            }
        }
    }

    /// Records a host throttle event (HTTP 429 or 503), halving `effective_ceiling`
    /// down to a floor of 1, resetting `clean_successes`, and advancing the shared
    /// cooldown to at least `now + delay`.
    ///
    /// Returns `(previous_ceiling, new_ceiling)`.
    pub fn record_throttle(&self, delay: Duration) -> (usize, usize) {
        let (prev_ceiling, new_ceiling) = {
            let mut state = self.inner.lock_state();
            let prev = state.effective_ceiling;
            state.effective_ceiling = (state.effective_ceiling / 2).max(1);
            state.clean_successes = 0;

            let now = Instant::now();
            if let Some(candidate) = now.checked_add(delay) {
                state.cooldown_until = Some(match state.cooldown_until {
                    Some(existing) => existing.max(candidate),
                    None => candidate,
                });
            }

            (prev, state.effective_ceiling)
        };

        self.inner.notify.notify_waiters();
        (prev_ceiling, new_ceiling)
    }

    /// Records a clean network download completion (never called for local skips).
    ///
    /// When `effective_ceiling` is reduced below [`MAX_CONCURRENT_PERMITS`], every
    /// [`BACKPRESSURE_RECOVERY_SUCCESSES`] clean completions doubles the ceiling
    /// up to [`MAX_CONCURRENT_PERMITS`].
    ///
    /// Returns `(previous_ceiling, new_ceiling)`.
    pub fn record_network_success(&self) -> (usize, usize) {
        let (prev_ceiling, new_ceiling, stepped_up) = {
            let mut state = self.inner.lock_state();
            let prev = state.effective_ceiling;
            if state.effective_ceiling < MAX_CONCURRENT_PERMITS {
                state.clean_successes += 1;
                if state.clean_successes >= BACKPRESSURE_RECOVERY_SUCCESSES {
                    state.effective_ceiling =
                        (state.effective_ceiling * 2).min(MAX_CONCURRENT_PERMITS);
                    state.clean_successes = 0;
                    (prev, state.effective_ceiling, true)
                } else {
                    (prev, prev, false)
                }
            } else {
                (prev, prev, false)
            }
        };

        if stepped_up {
            self.inner.notify.notify_waiters();
        }
        (prev_ceiling, new_ceiling)
    }
}

/// RAII permit guard returned by [`ElasticScheduler::acquire`].
///
/// Releases the exact number of permit units acquired and wakes waiting tasks on drop.
#[derive(Debug)]
pub struct SchedulerPermit {
    inner: Arc<SchedulerInner>,
    units: usize,
}

impl SchedulerPermit {
    /// Returns the number of permit units held by this guard.
    pub fn units(&self) -> usize {
        self.units
    }
}

impl Drop for SchedulerPermit {
    fn drop(&mut self) {
        {
            let mut state = self.inner.lock_state();
            state.in_flight_units = state.in_flight_units.saturating_sub(self.units);
        }
        self.inner.notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_permit_cost_size_tiers() {
        assert_eq!(base_permit_cost(Some(0)), 1);
        assert_eq!(base_permit_cost(Some(1024)), 1);
        assert_eq!(base_permit_cost(Some(SMALL_FILE_MAX_BYTES - 1)), 1);

        assert_eq!(base_permit_cost(Some(SMALL_FILE_MAX_BYTES)), 2);
        assert_eq!(base_permit_cost(Some(8 * 1024 * 1024)), 2);
        assert_eq!(base_permit_cost(Some(MEDIUM_FILE_MAX_BYTES - 1)), 2);

        assert_eq!(base_permit_cost(Some(MEDIUM_FILE_MAX_BYTES)), 4);
        assert_eq!(base_permit_cost(Some(1024 * 1024 * 1024)), 4);
        assert_eq!(base_permit_cost(Some(u64::MAX)), 4);
        assert_eq!(base_permit_cost(None), 4);
    }

    #[test]
    fn test_sort_planned_downloads_ascending_with_none_tail_and_stable_ties() {
        let mut entries: Vec<(usize, (&str, Option<u64>))> = vec![
            (0, ("unknown_first", None)),
            (1, ("large", Some(20 * 1024 * 1024))),
            (2, ("small_b", Some(500))),
            (3, ("max_u64", Some(u64::MAX))),
            (4, ("small_a", Some(500))),
            (5, ("tiny", Some(0))),
            (6, ("unknown_second", None)),
            (7, ("medium", Some(2 * 1024 * 1024))),
        ];

        sort_planned_downloads(&mut entries, |(_, size)| *size);

        let ordered_names: Vec<&str> = entries.iter().map(|(_, (name, _))| *name).collect();
        assert_eq!(
            ordered_names,
            vec![
                "tiny",
                "small_b",
                "small_a",
                "medium",
                "large",
                "max_u64",
                "unknown_first",
                "unknown_second",
            ]
        );
    }

    #[tokio::test]
    async fn test_funnel_drain_four_small_then_large_fifo_without_starvation() {
        let scheduler = ElasticScheduler::new();
        let signal = ShutdownSignal::new();

        // Acquire 4 small permits (fills all 4 units).
        let p1 = scheduler.acquire(Some(100), &signal).await.unwrap();
        let p2 = scheduler.acquire(Some(200), &signal).await.unwrap();
        let p3 = scheduler.acquire(Some(300), &signal).await.unwrap();
        let p4 = scheduler.acquire(Some(400), &signal).await.unwrap();
        assert_eq!(p1.units(), 1);
        assert_eq!(scheduler.in_flight_units(), 4);

        // Queue a large file (cost 4) while all 4 small permits are held.
        let large_sched = scheduler.clone();
        let large_sig = signal.clone();
        let large_task = tokio::spawn(async move {
            large_sched
                .acquire(Some(MEDIUM_FILE_MAX_BYTES), &large_sig)
                .await
                .unwrap()
        });

        tokio::task::yield_now().await;
        assert!(!large_task.is_finished());

        // Queue a 5th small file behind the large file to verify FIFO order (no starvation).
        let late_small_sched = scheduler.clone();
        let late_small_sig = signal.clone();
        let late_small_task =
            tokio::spawn(async move { late_small_sched.acquire(Some(50), &late_small_sig).await });

        // Dropping 3 of 4 small permits is not enough for the large file, and the late small
        // file must not jump ahead of the queued large file.
        drop(p1);
        drop(p2);
        drop(p3);
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert_eq!(scheduler.in_flight_units(), 1);
        assert!(!large_task.is_finished());
        assert!(!late_small_task.is_finished());

        // Dropping the 4th small permit drains in-flight to 0 and immediately grants the large file.
        drop(p4);
        let large_permit = tokio::time::timeout(Duration::from_secs(1), large_task)
            .await
            .expect("large file should acquire once funnel drains")
            .unwrap();
        assert_eq!(large_permit.units(), 4);
        assert_eq!(scheduler.in_flight_units(), 4);
        assert!(!late_small_task.is_finished());

        // Dropping the large permit lets the late small file proceed.
        drop(large_permit);
        let late_permit = tokio::time::timeout(Duration::from_secs(1), late_small_task)
            .await
            .expect("late small file should proceed after large file drops")
            .unwrap()
            .unwrap();
        assert_eq!(late_permit.units(), 1);
        assert_eq!(scheduler.in_flight_units(), 1);

        drop(late_permit);
        assert_eq!(scheduler.in_flight_units(), 0);
    }

    #[tokio::test]
    async fn test_backpressure_step_down_deadlock_freedom_and_recovery() {
        let scheduler = ElasticScheduler::new();
        let signal = ShutdownSignal::new();

        let medium_size = SMALL_FILE_MAX_BYTES;
        let large_size = MEDIUM_FILE_MAX_BYTES;

        assert_eq!(scheduler.effective_ceiling(), 4);
        assert_eq!(scheduler.effective_cost(Some(100)), 1);
        assert_eq!(scheduler.effective_cost(Some(medium_size)), 2);
        assert_eq!(scheduler.effective_cost(Some(large_size)), 4);
        assert_eq!(scheduler.effective_cost(None), 4);

        // First throttle: 4 -> 2.
        assert_eq!(scheduler.record_throttle(Duration::ZERO), (4, 2));
        assert_eq!(scheduler.effective_ceiling(), 2);
        assert_eq!(scheduler.effective_cost(Some(100)), 2);
        assert_eq!(scheduler.effective_cost(Some(medium_size)), 2);
        assert_eq!(scheduler.effective_cost(Some(large_size)), 4);
        assert_eq!(scheduler.effective_cost(None), 4);

        // Second throttle: 2 -> 1.
        assert_eq!(scheduler.record_throttle(Duration::ZERO), (2, 1));
        assert_eq!(scheduler.effective_ceiling(), 1);
        assert_eq!(scheduler.effective_cost(Some(100)), 4);
        assert_eq!(scheduler.effective_cost(Some(medium_size)), 4);
        assert_eq!(scheduler.effective_cost(Some(large_size)), 4);
        assert_eq!(scheduler.effective_cost(None), 4);

        // Third throttle stays floored at 1.
        assert_eq!(scheduler.record_throttle(Duration::ZERO), (1, 1));
        assert_eq!(scheduler.effective_ceiling(), 1);

        // Deadlock freedom at effective_ceiling == 1: both large and unknown-size files acquire cleanly.
        let large_permit = scheduler
            .acquire(Some(100 * 1024 * 1024), &signal)
            .await
            .unwrap();
        assert_eq!(large_permit.units(), 4);
        drop(large_permit);

        let unknown_permit = scheduler.acquire(None, &signal).await.unwrap();
        assert_eq!(unknown_permit.units(), 4);
        drop(unknown_permit);

        // Recovery requires 4 clean network completions per step; throttle resets counter.
        for expected_clean in 1..=3 {
            assert_eq!(scheduler.record_network_success(), (1, 1));
            assert_eq!(scheduler.clean_successes(), expected_clean);
            assert_eq!(scheduler.effective_ceiling(), 1);
        }
        // Throttle mid-recovery resets clean_successes to 0.
        assert_eq!(scheduler.record_throttle(Duration::ZERO), (1, 1));
        assert_eq!(scheduler.clean_successes(), 0);

        // 4 clean network completions raise ceiling 1 -> 2.
        for _ in 0..3 {
            assert_eq!(scheduler.record_network_success(), (1, 1));
        }
        assert_eq!(scheduler.record_network_success(), (1, 2));
        assert_eq!(scheduler.effective_ceiling(), 2);
        assert_eq!(scheduler.clean_successes(), 0);

        // 4 more clean network completions raise ceiling 2 -> 4.
        for _ in 0..3 {
            assert_eq!(scheduler.record_network_success(), (2, 2));
        }
        assert_eq!(scheduler.record_network_success(), (2, 4));
        assert_eq!(scheduler.effective_ceiling(), 4);
        assert_eq!(scheduler.clean_successes(), 0);

        // Further completions stay capped at 4 and do not accumulate clean_successes.
        assert_eq!(scheduler.record_network_success(), (4, 4));
        assert_eq!(scheduler.clean_successes(), 0);
    }

    #[tokio::test]
    async fn test_cooldown_gating_monotonic_extension_and_shutdown_cancellation() {
        let scheduler = ElasticScheduler::new();
        let signal = ShutdownSignal::new();

        scheduler.record_throttle(Duration::from_millis(80));
        // Shorter throttle must not shorten an existing longer cooldown.
        scheduler.record_throttle(Duration::from_millis(10));
        let started = std::time::Instant::now();
        let permit = scheduler.acquire(Some(100), &signal).await.unwrap();
        let elapsed = started.elapsed();
        drop(permit);

        assert!(
            elapsed >= Duration::from_millis(60),
            "acquire must wait for shared cooldown, elapsed={elapsed:?}"
        );

        // Long cooldown cancels promptly on ShutdownSignal.
        scheduler.record_throttle(Duration::from_secs(30));
        let cancel_signal = ShutdownSignal::new();
        let trigger_signal = cancel_signal.clone();
        let trigger_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            trigger_signal.trigger();
        });

        let cancel_started = std::time::Instant::now();
        let err = scheduler
            .acquire(Some(100), &cancel_signal)
            .await
            .unwrap_err();
        let cancel_elapsed = cancel_started.elapsed();
        trigger_task.await.unwrap();

        assert!(
            cancel_elapsed < Duration::from_secs(1),
            "shutdown must cancel cooldown wait promptly, took {cancel_elapsed:?}"
        );
        assert!(err.to_string().contains("interrupted"));
        assert_eq!(scheduler.in_flight_units(), 0);
    }

    #[tokio::test]
    async fn test_medium_files_allow_two_concurrent_and_block_third() {
        let scheduler = ElasticScheduler::new();
        let signal = ShutdownSignal::new();

        let m1 = scheduler
            .acquire(Some(SMALL_FILE_MAX_BYTES), &signal)
            .await
            .unwrap();
        let m2 = scheduler
            .acquire(Some(SMALL_FILE_MAX_BYTES + 2 * 1024 * 1024), &signal)
            .await
            .unwrap();
        assert_eq!(m1.units(), 2);
        assert_eq!(m2.units(), 2);
        assert_eq!(scheduler.in_flight_units(), 4);

        let sched_clone = scheduler.clone();
        let sig_clone = signal.clone();
        let m3_task = tokio::spawn(async move {
            sched_clone
                .acquire(Some(SMALL_FILE_MAX_BYTES + 1024 * 1024), &sig_clone)
                .await
                .unwrap()
        });

        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(!m3_task.is_finished());

        drop(m1);
        let m3 = tokio::time::timeout(Duration::from_secs(1), m3_task)
            .await
            .expect("third medium file should acquire once one finishes")
            .unwrap();
        assert_eq!(m3.units(), 2);
        assert_eq!(scheduler.in_flight_units(), 4);

        drop(m2);
        drop(m3);
        assert_eq!(scheduler.in_flight_units(), 0);
    }
}
