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
    block: AudioBlock,
}
pub struct BlockProducer {
    inner: Producer<QueuedBlock>,
    generation: Arc<AtomicU64>,
    stats: Arc<AudioStats>,
}
pub struct BlockConsumer {
    inner: Consumer<QueuedBlock>,
    generation: Arc<AtomicU64>,
    seen: u64,
    stats: Arc<AudioStats>,
    capacity: usize,
    mark_gap: bool,
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
    Ok((
        BlockProducer {
            inner: p,
            generation: generation.clone(),
            stats: stats.clone(),
        },
        BlockConsumer {
            inner: c,
            generation,
            seen: 0,
            stats,
            capacity,
            mark_gap: false,
        },
    ))
}
impl BlockProducer {
    pub fn push(&mut self, block: AudioBlock) -> bool {
        let frames = u64::from(block.header.frame_count);
        let entry = QueuedBlock {
            generation: self.generation.load(Acquire),
            block,
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
}
impl BlockConsumer {
    pub fn pop_fresh(&mut self, now_ns: u64, max_age_ns: u64) -> Option<AudioBlock> {
        for _ in 0..self.capacity {
            let entry = self.inner.pop().ok()?;
            let generation = self.generation.load(Acquire);
            if generation != self.seen {
                self.seen = generation;
                self.mark_gap = true;
            }
            let mut block = entry.block;
            if entry.generation != generation
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
