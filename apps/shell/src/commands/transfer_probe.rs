// SPDX-License-Identifier: Apache-2.0
//! Diagnostic probe of the owner control path and system memory while a V7
//! tracked write is open. It only observes: the write itself goes through the
//! same typed SDK steps as `replace-pattern-v7`, and the supervisor query is the
//! owner's ordinary `INFO`.
use super::*;
use rustic_sdk::abi::{
    files::operation::Replacement, files::workspace::Operation, supervisor as sv,
};

/// Round trips of `BUCKETS - 1` ticks or more share the last bucket.
const BUCKETS: usize = 64;

/// What the probe observed across one transfer.
pub(super) struct Samples {
    pub every: u64,
    pub count: u32,
    /// Sum of all round trips, so a mean below one tick is recoverable.
    pub total: u64,
    pub max: u64,
    buckets: [u32; BUCKETS],
    pub free_frames: (u64, u64),
    pub heap_pages: (u64, u64),
}

impl Samples {
    fn new(every: u64) -> Self {
        Self {
            every,
            count: 0,
            total: 0,
            max: 0,
            buckets: [0; BUCKETS],
            free_frames: (u64::MAX, 0),
            heap_pages: (u64::MAX, 0),
        }
    }

    fn record(&mut self, ticks: u64, free_frames: u64, heap_pages: u64) {
        self.count = self.count.saturating_add(1);
        self.total = self.total.saturating_add(ticks);
        self.max = self.max.max(ticks);
        let bucket = usize::try_from(ticks).map_or(BUCKETS - 1, |t| t.min(BUCKETS - 1));
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.free_frames = (
            self.free_frames.0.min(free_frames),
            self.free_frames.1.max(free_frames),
        );
        self.heap_pages = (
            self.heap_pages.0.min(heap_pages),
            self.heap_pages.1.max(heap_pages),
        );
    }

    /// Median round trip in ticks, from the bucketed samples.
    pub fn p50(&self) -> u64 {
        let half = self.count.div_ceil(2);
        let mut seen = 0u32;
        for (ticks, count) in self.buckets.iter().enumerate() {
            seen = seen.saturating_add(*count);
            if seen >= half && seen > 0 {
                return ticks as u64;
            }
        }
        0
    }
}

/// Stream `size` bytes, issuing one owner `INFO` before every `every`-th chunk
/// and once more before the commit, then commit and return the verified
/// receipt with the samples. Any chunk or query failure aborts the transfer.
pub(super) fn probe(
    s: &mut Session,
    request: Replacement,
    size: u32,
    every: u64,
    fill: impl Fn(u32, &mut [u8]) -> Result<(), rustic_sdk::files::Error> + Copy,
) -> Result<(Operation, Samples), Error> {
    if every == 0 {
        return Err(Error::Usage);
    }
    let mut samples = Samples::new(every);
    let mut transfer = s.files.workspace_open(request, size)?;
    let mut chunk = 0u64;
    loop {
        let due = chunk.is_multiple_of(every) || transfer.offset() == transfer.size();
        if due && let Err(error) = sample(s, &mut samples) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error);
        }
        if transfer.offset() == transfer.size() {
            break;
        }
        if let Err(error) = s.files.workspace_chunk(&mut transfer, fill) {
            let _ = s.files.workspace_abort(transfer);
            return Err(error.into());
        }
        chunk += 1;
    }
    let operation = s.files.workspace_commit(transfer)?;
    Ok((operation, samples))
}

/// One owner `INFO` round trip: guest ticks, free frames and heap pages.
fn sample(s: &mut Session, samples: &mut Samples) -> Result<(), Error> {
    let started = rustic_sdk::runtime::clock();
    let r = s.request([sv::INFO, 0, 0, 0, 0, 0, 0, 0])?;
    let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
    if r[0] != 0 {
        return Err(Error::Service(4));
    }
    samples.record(ticks, r[2], r[7]);
    Ok(())
}
