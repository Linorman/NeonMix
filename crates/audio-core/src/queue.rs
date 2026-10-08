//! SPSC preallocated block transport. A generation invalidates old backlog without losing new data.
use crate::{AudioBlock, Discontinuity, stats::AudioStats};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc,
    atomic::{
        AtomicU64,
        Ordering::{Acquire, Relaxed, Release},
    },
};
struct QueuedBlock {
    generation: u64,
    binding: u64,
    block: AudioBlock,
}
pub struct BlockProducer {
    inner: Producer<QueuedBlock>,
    generation: Arc<AtomicU64>,
    stats: Arc<AudioStats>,
    binding: Arc<AtomicU64>,
    lease: u64,
}
pub struct BlockConsumer {
    inner: Consumer<QueuedBlock>,
    generation: Arc<AtomicU64>,
    seen: u64,
    stats: Arc<AudioStats>,
    capacity: usize,
    mark_gap: bool,
    binding: Arc<AtomicU64>,
}
pub fn block_queue(
    capacity: usize,
    stats: Arc<AudioStats>,
) -> Result<(BlockProducer, BlockConsumer), crate::AudioError> {
    if !(1..=64).contains(&capacity) {
        return Err(crate::AudioError::InvalidArgument(
            "queue capacity must be 1..64 blocks".into(),
        ));
    }
    let (p, c) = RingBuffer::new(capacity);
    let generation = Arc::new(AtomicU64::new(0));
    let binding = Arc::new(AtomicU64::new(1));
    Ok((
        BlockProducer {
            inner: p,
            generation: generation.clone(),
            stats: stats.clone(),
            binding: binding.clone(),
            lease: 1,
        },
        BlockConsumer {
            inner: c,
            generation,
            seen: 0,
            stats,
            capacity,
            mark_gap: false,
            binding,
        },
    ))
}
impl BlockProducer {
    pub fn remaining_capacity(&self) -> usize {
        self.inner.slots()
    }
    pub fn push(&mut self, block: AudioBlock) -> bool {
        let frames = u64::from(block.header.frame_count);
        let entry = QueuedBlock {
            generation: self.generation.load(Acquire),
            block,
            binding: self.lease,
        };
        if self.inner.push(entry).is_err() {
            self.stats.dropped_frames.fetch_add(frames, Relaxed);
            self.invalidate();
            false
        } else {
            true
        }
    }
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Release);
    }
    /// New owner lease, independent of queue-loss/discontinuity generation.
    pub fn bind_next(&mut self) -> Result<u64, crate::AudioError> {
        self.lease = self
            .lease
            .checked_add(1)
            .ok_or_else(|| crate::AudioError::InvalidArgument("lane binding exhausted".into()))?;
        self.binding.store(self.lease, Release);
        self.invalidate();
        Ok(self.lease)
    }
    pub fn binding_generation(&self) -> u64 {
        self.lease
    }
    pub fn revoke_binding(&self) {
        self.binding.store(0, Release);
    }
    pub(crate) fn authorization(&self) -> Arc<AtomicU64> {
        self.binding.clone()
    }
}
impl BlockConsumer {
    pub fn binding_valid(&self, binding: u64) -> bool {
        binding != 0 && self.binding.load(Acquire) == binding
    }
    pub fn queued_blocks(&self) -> usize {
        self.inner.slots()
    }
    pub fn pop_fresh(&mut self, now_ns: u64, max_age_ns: u64) -> Option<AudioBlock> {
        let mut budget = self.capacity;
        self.pop_fresh_budget(now_ns, max_age_ns, None, &mut budget)
    }
    pub fn pop_fresh_budget(
        &mut self,
        now_ns: u64,
        max_age_ns: u64,
        expected_binding: Option<u64>,
        budget: &mut usize,
    ) -> Option<AudioBlock> {
        while *budget != 0 {
            if expected_binding.is_some_and(|binding| !self.binding_valid(binding)) {
                return None;
            }
            let entry = self.inner.pop().ok()?;
            *budget -= 1;
            let generation = self.generation.load(Acquire);
            if generation != self.seen {
                self.seen = generation;
                self.mark_gap = true;
            }
            let mut block = entry.block;
            if entry.generation != generation
                || entry.binding != self.binding.load(Acquire)
                || expected_binding.is_some_and(|binding| binding != entry.binding)
                || now_ns.saturating_sub(block.header.arrival_ns) > max_age_ns
            {
                self.stats
                    .stale_frames
                    .fetch_add(u64::from(block.header.frame_count), Relaxed);
                self.mark_gap = true;
                continue;
            }
            if self.mark_gap {
                block.header.discontinuity_flags.insert(Discontinuity::GAP);
                self.mark_gap = false;
            }
            return Some(block);
        }
        None
    }
}
