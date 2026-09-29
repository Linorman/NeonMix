//! One process-wide allocator prevents a reopened stream from reusing an epoch reached by reset.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Epochs are opaque identifiers, not per-stream reset counts. E02 must additionally bind
/// them to a fresh session across process restarts. Exhaustion fails instead of wrapping.
pub fn reserve_epoch_after(previous: u64) -> Option<u64> {
    let minimum = previous.checked_add(1)?;
    NEXT_EPOCH
        .fetch_update(Relaxed, Relaxed, |next| next.max(minimum).checked_add(1))
        .ok()
        .map(|next| next.max(minimum))
}
