//! The backlog: intake requests no Poet has acknowledged yet, kept while
//! the Poets are unreachable and replayed when one answers again.
//!
//! The backlog is bounded by bytes, not by batch count: one all-namespace
//! batch of a small cluster is about 0.75 MB of JSON, so a count bound does
//! not bound memory. Each request is kept as deflate-compressed JSON
//! (about 20 times smaller), so the bound is the memory it costs.
//!
//! When a new batch would exceed the bound, the oldest data is thinned
//! first: of two neighbouring batches that each stand for the same number
//! of collection intervals, the newer is removed and the older one then
//! stands for both (it keeps every second, then every fourth, ... sample, up
//! to [`BacklogConfig::max_thinning`]). Only when nothing can be thinned
//! further is the oldest batch dropped. Batches that carry events are never
//! thinned, only dropped.
//!
//! Nothing is reduced silently. A thinned batch's availability states the
//! longer window it stands for, its coarser resolution and its lower
//! coverage, under [`THINNED_PRESSURE_POLICY`]; a dropped window has no
//! availability at all (unavailable, never a measured zero). Every
//! reduction is counted in a [`Loss`] that the Muse reports to Poet as an
//! event in the next batch it sends, until a Poet acknowledges one.

use std::collections::VecDeque;

use ih_muse_proto::GraphIntakeRequest;

/// Default byte bound, an eighth of the release's 128 MiB memory limit. On
/// k-lab (4 nodes, 19 pods, every namespace: 465 observations, 37 KiB per
/// compressed batch) it holds about 37 minutes at 5 s before thinning and
/// about 10 hours thinned before dropping; a 10 minute outage needs 4.5 MB.
pub const DEFAULT_BACKLOG_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Default largest thinning: keep every 16th sample (80 s at 5 s) before
/// dropping the oldest.
pub const DEFAULT_MAX_THINNING: u32 = 16;

/// Default replay pace: batches sent per collection interval after the
/// Poets come back. The backlog drains three intervals per interval, so a
/// 10 minute outage replays in about 3 to 4 minutes without flooding Poet.
pub const DEFAULT_REPLAY_BATCHES_PER_INTERVAL: usize = 4;

/// `pressure_policy` of the availability of a thinned batch.
pub const THINNED_PRESSURE_POLICY: &str = "ih.muse.backlog.thinned";

/// How much unsent data the backlog keeps and how far it thins it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BacklogConfig {
    /// Bound on the compressed bytes of all queued batches. The newest
    /// batch is always kept, even alone above the bound.
    pub max_bytes: usize,
    /// The most collection intervals one kept batch may stand for (a power
    /// of two); beyond it the oldest batch is dropped.
    pub max_thinning: u32,
}

impl Default for BacklogConfig {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_BACKLOG_MAX_BYTES,
            max_thinning: DEFAULT_MAX_THINNING,
        }
    }
}

/// What the backlog removed: collection intervals thinned (their sample is
/// gone, the window is marked partial) or dropped (the window is gone), and
/// the events lost with dropped batches, over the time window affected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loss {
    pub thinned_intervals: u64,
    pub dropped_intervals: u64,
    pub dropped_events: u64,
    /// Start of the earliest affected window (0 when nothing was lost).
    pub from_unix_nano: u64,
    /// End of the latest affected window.
    pub to_unix_nano: u64,
}

impl Loss {
    pub fn is_empty(&self) -> bool {
        self.thinned_intervals == 0 && self.dropped_intervals == 0 && self.dropped_events == 0
    }

    /// Adds `other` and widens the window to cover both.
    pub fn merge(&mut self, other: &Loss) {
        if other.is_empty() {
            return;
        }
        self.from_unix_nano = if self.is_empty() {
            other.from_unix_nano
        } else {
            self.from_unix_nano.min(other.from_unix_nano)
        };
        self.to_unix_nano = self.to_unix_nano.max(other.to_unix_nano);
        self.thinned_intervals += other.thinned_intervals;
        self.dropped_intervals += other.dropped_intervals;
        self.dropped_events += other.dropped_events;
    }

    fn window(from: u64, to: u64) -> Self {
        Self {
            from_unix_nano: from,
            to_unix_nano: to,
            ..Self::default()
        }
    }
}

/// One queued request, compressed, with the window it stands for.
struct Entry {
    packed: Vec<u8>,
    /// Start of the window: the batch's collection time minus one interval.
    from: u64,
    /// End of the window: the collection time of the newest batch thinned
    /// into this one (its own collection time until then).
    until: u64,
    /// Collection intervals this batch stands for (1 until thinned).
    intervals: u32,
    events: usize,
    /// Carries events: never thinned.
    pinned: bool,
}

/// Unacknowledged intake requests, oldest first, within a byte bound.
pub struct Backlog {
    config: BacklogConfig,
    interval_ns: u64,
    entries: VecDeque<Entry>,
    bytes: usize,
    /// Reductions no Poet has acknowledged a report of.
    unreported: Loss,
    /// Every reduction since start.
    total: Loss,
}

impl Backlog {
    pub fn new(config: BacklogConfig, interval_ns: u64) -> Self {
        Self {
            config,
            interval_ns,
            entries: VecDeque::new(),
            bytes: 0,
            unreported: Loss::default(),
            total: Loss::default(),
        }
    }

    pub fn config(&self) -> BacklogConfig {
        self.config
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Compressed bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Every reduction since start.
    pub fn total(&self) -> &Loss {
        &self.total
    }

    /// Takes the reductions no Poet has acknowledged a report of, to
    /// report them in the batch about to be sent. Give them back with
    /// [`Self::restore_unreported`] if that batch is not acknowledged.
    pub fn take_unreported(&mut self) -> Option<Loss> {
        let loss = std::mem::take(&mut self.unreported);
        (!loss.is_empty()).then_some(loss)
    }

    /// Returns a report taken by [`Self::take_unreported`] that no Poet
    /// acknowledged.
    pub fn restore_unreported(&mut self, loss: &Loss) {
        self.unreported.merge(loss);
    }

    /// Queues `request`, collected at `observed_at`, then thins or drops
    /// the oldest data until the bound holds. Returns what this call
    /// removed.
    pub fn push(&mut self, request: &GraphIntakeRequest, observed_at: u64) -> Loss {
        let before = self.total.clone();
        let from = observed_at.saturating_sub(self.interval_ns);
        match pack(request) {
            Some(packed) => {
                self.bytes += packed.len();
                self.entries.push_back(Entry {
                    packed,
                    from,
                    until: observed_at,
                    intervals: 1,
                    events: request.batch.events.len(),
                    // Dashboards are sent again until acknowledged, so a
                    // batch carrying them may be thinned.
                    pinned: !request.batch.events.is_empty(),
                });
            }
            // Unreachable for these plain data types; still counted.
            None => {
                let mut loss = Loss::window(from, observed_at);
                loss.dropped_intervals = 1;
                loss.dropped_events = request.batch.events.len() as u64;
                self.record(&loss);
            }
        }
        while self.bytes > self.config.max_bytes && self.entries.len() > 1 {
            if !self.thin_oldest() {
                self.drop_oldest();
            }
        }
        delta(&before, &self.total)
    }

    /// The oldest request, its availability rewritten if it was thinned.
    /// `None` when empty; an entry that cannot be read is dropped (counted).
    pub fn front(&mut self) -> Option<GraphIntakeRequest> {
        loop {
            let entry = self.entries.front()?;
            match unpack(&entry.packed) {
                Some(mut request) => {
                    if entry.intervals > 1 {
                        mark_thinned(&mut request, entry.from, entry.until, self.interval_ns);
                    }
                    return Some(request);
                }
                None => self.drop_oldest(),
            }
        }
    }

    /// Removes the oldest request after a Poet acknowledged it.
    pub fn pop_acknowledged(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.bytes -= entry.packed.len();
        }
    }

    /// Removes the oldest request after a Poet rejected it (any Poet would
    /// again); its window is counted as dropped.
    pub fn pop_rejected(&mut self) {
        self.drop_oldest();
    }

    fn record(&mut self, loss: &Loss) {
        self.unreported.merge(loss);
        self.total.merge(loss);
    }

    fn drop_oldest(&mut self) {
        let Some(entry) = self.entries.pop_front() else {
            return;
        };
        self.bytes -= entry.packed.len();
        let mut loss = Loss::window(entry.from, entry.until);
        loss.dropped_intervals = u64::from(entry.intervals);
        loss.dropped_events = entry.events as u64;
        self.record(&loss);
    }

    /// Thins the oldest pair of neighbouring unpinned batches of the
    /// smallest stride that can still double, never touching the newest
    /// batch, until the bound holds. Returns whether anything was thinned.
    fn thin_oldest(&mut self) -> bool {
        let mut strides: Vec<u32> = self
            .entries
            .iter()
            .take(self.entries.len().saturating_sub(1))
            .filter(|entry| !entry.pinned && entry.intervals * 2 <= self.config.max_thinning)
            .map(|entry| entry.intervals)
            .collect();
        strides.sort_unstable();
        strides.dedup();
        let mut thinned = false;
        for stride in strides {
            let mut index = 0;
            // `index + 2 < len`: the pair excludes the newest batch.
            while index + 2 < self.entries.len() {
                let pair = (&self.entries[index], &self.entries[index + 1]);
                if pair.0.pinned
                    || pair.1.pinned
                    || pair.0.intervals != stride
                    || pair.1.intervals != stride
                {
                    index += 1;
                    continue;
                }
                let Some(newer) = self.entries.remove(index + 1) else {
                    break;
                };
                self.bytes -= newer.packed.len();
                let older = &mut self.entries[index];
                older.intervals += newer.intervals;
                older.until = newer.until;
                let mut loss = Loss::window(newer.from, newer.until);
                loss.thinned_intervals = u64::from(newer.intervals);
                self.record(&loss);
                thinned = true;
                if self.bytes <= self.config.max_bytes {
                    return true;
                }
                index += 1;
            }
            if thinned {
                return true;
            }
        }
        thinned
    }
}

/// The reductions in `after` that are not in `before` (window: `after`'s).
fn delta(before: &Loss, after: &Loss) -> Loss {
    Loss {
        thinned_intervals: after.thinned_intervals - before.thinned_intervals,
        dropped_intervals: after.dropped_intervals - before.dropped_intervals,
        dropped_events: after.dropped_events - before.dropped_events,
        from_unix_nano: after.from_unix_nano,
        to_unix_nano: after.to_unix_nano,
    }
}

/// States that one sample stands for the window `[from, until]`: the
/// window, a resolution as long as it, and the share of it the sample
/// covers.
fn mark_thinned(request: &mut GraphIntakeRequest, from: u64, until: u64, interval_ns: u64) {
    let window = (until + 1).saturating_sub(from).max(1);
    let share = (interval_ns as f64 / window as f64).min(1.0);
    for availability in &mut request.batch.availability {
        availability.available_time.from_unix_nano =
            availability.available_time.from_unix_nano.min(from);
        availability.available_time.to_unix_nano = until + 1;
        availability.resolution_unix_nano = window;
        availability.coverage_fraction *= share;
        availability.pressure_policy = Some(THINNED_PRESSURE_POLICY.into());
    }
}

/// Deflate level 1: about 20 times smaller than the JSON at a few ms per
/// batch.
const DEFLATE_LEVEL: u8 = 1;

fn pack(request: &GraphIntakeRequest) -> Option<Vec<u8>> {
    let json = serde_json::to_vec(request).ok()?;
    let mut packed = miniz_oxide::deflate::compress_to_vec(&json, DEFLATE_LEVEL);
    packed.shrink_to_fit();
    Some(packed)
}

fn unpack(packed: &[u8]) -> Option<GraphIntakeRequest> {
    let json = miniz_oxide::inflate::decompress_to_vec(packed).ok()?;
    serde_json::from_slice(&json).ok()
}
