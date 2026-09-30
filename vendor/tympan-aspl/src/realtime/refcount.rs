//! CFPlugIn-style atomic reference counter.
//!
//! An AudioServerPlugin is a CFPlugIn instance: the HAL holds it
//! through the COM-style `IUnknown` contract, with `AddRef` and
//! `Release` returning the new `UInt32` reference count. The
//! framework's plug-in and driver wrappers implement those by
//! delegating to a [`Refcount`] field; the cross-platform definition
//! here lets the lock-free behaviour be unit-tested on any host.
//!
//! The counter is wait-free, allocation-free, and safe to share
//! between threads.

use core::sync::atomic::{AtomicU32, Ordering};

/// Atomic reference counter.
///
/// Holds the count of outstanding strong references to a CFPlugIn
/// object. The two write paths — [`Self::add_ref`] and
/// [`Self::release`] — return the new count, matching the
/// `IUnknown::AddRef` / `IUnknown::Release` ABI. Read access via
/// [`Self::count`] is realtime-safe.
///
/// A fresh counter starts at `1`: CFPlugIn factory functions hand
/// back an object that the caller already owns one reference to.
#[derive(Debug)]
pub struct Refcount(AtomicU32);

impl Refcount {
    /// Construct a counter at `1` — the initial owning reference a
    /// CFPlugIn factory hands back to its caller.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU32::new(1))
    }

    /// Construct a counter at zero.
    ///
    /// Useful for framework objects whose lifetime is managed
    /// separately from the CFPlugIn `IUnknown` contract and which
    /// therefore start unreferenced.
    #[inline]
    #[must_use]
    pub const fn new_unowned() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Atomically increment and return the new count.
    ///
    /// Equivalent to `IUnknown::AddRef`.
    #[inline]
    pub fn add_ref(&self) -> u32 {
        let prev = self.0.fetch_add(1, Ordering::AcqRel);
        debug_assert!(
            prev < u32::MAX,
            "Refcount::add_ref overflowed u32 — runaway reference leak"
        );
        prev + 1
    }

    /// Atomically decrement and return the new count.
    ///
    /// Equivalent to `IUnknown::Release`. The CFPlugIn convention is
    /// that the caller frees the underlying object when this returns
    /// zero.
    ///
    /// # Panics (debug)
    ///
    /// Panics in debug builds if the counter was already zero —
    /// that always indicates a release-after-free bug. In release
    /// builds the counter wraps via `AtomicU32::fetch_sub`'s defined
    /// wrap behaviour; the caller is then responsible for not
    /// double-freeing the object.
    #[inline]
    pub fn release(&self) -> u32 {
        let prev = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(
            prev > 0,
            "Refcount::release called when count was already 0"
        );
        prev.wrapping_sub(1)
    }

    /// Realtime-safe load of the current count (`Acquire` ordering).
    #[inline]
    #[must_use]
    pub fn count(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }

    /// `true` iff [`Self::count`] returns 0.
    #[inline]
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.count() == 0
    }
}

impl Default for Refcount {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    assert_impl_all!(Refcount: Send, Sync);

    #[test]
    fn new_starts_at_one() {
        let rc = Refcount::new();
        assert_eq!(rc.count(), 1);
        assert!(!rc.is_zero());
    }

    #[test]
    fn new_unowned_starts_at_zero() {
        let rc = Refcount::new_unowned();
        assert_eq!(rc.count(), 0);
        assert!(rc.is_zero());
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(Refcount::default().count(), 1);
    }

    #[test]
    fn add_ref_returns_incremented_count() {
        let rc = Refcount::new_unowned();
        assert_eq!(rc.add_ref(), 1);
        assert_eq!(rc.add_ref(), 2);
        assert_eq!(rc.add_ref(), 3);
        assert_eq!(rc.count(), 3);
        assert!(!rc.is_zero());
    }

    #[test]
    fn release_returns_decremented_count() {
        let rc = Refcount::new_unowned();
        rc.add_ref();
        rc.add_ref();
        rc.add_ref();
        assert_eq!(rc.release(), 2);
        assert_eq!(rc.release(), 1);
        assert_eq!(rc.release(), 0);
        assert!(rc.is_zero());
    }

    #[test]
    fn owning_refcount_releases_to_zero() {
        // A factory-minted object: starts at 1, the HAL's single
        // Release drops it to 0.
        let rc = Refcount::new();
        assert_eq!(rc.release(), 0);
        assert!(rc.is_zero());
    }

    #[test]
    #[should_panic(expected = "Refcount::release called when count was already 0")]
    fn release_when_zero_panics_in_debug() {
        let rc = Refcount::new_unowned();
        rc.release();
    }

    #[test]
    fn many_round_trips_preserve_invariants() {
        let rc = Refcount::new_unowned();
        for i in 1..=100 {
            assert_eq!(rc.add_ref(), i);
        }
        for i in (0..100).rev() {
            assert_eq!(rc.release(), i);
        }
        assert!(rc.is_zero());
    }

    #[test]
    fn concurrent_add_ref_release_balance_to_zero() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 8;
        const OPS_PER_THREAD: u32 = 10_000;

        let rc = Arc::new(Refcount::new_unowned());

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let rc = Arc::clone(&rc);
                thread::spawn(move || {
                    for _ in 0..OPS_PER_THREAD {
                        rc.add_ref();
                    }
                    for _ in 0..OPS_PER_THREAD {
                        rc.release();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(rc.count(), 0);
    }

    #[test]
    fn concurrent_add_ref_only_yields_total_count() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 8;
        const ADDS_PER_THREAD: u32 = 10_000;

        let rc = Arc::new(Refcount::new_unowned());

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let rc = Arc::clone(&rc);
                thread::spawn(move || {
                    for _ in 0..ADDS_PER_THREAD {
                        rc.add_ref();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(rc.count(), THREADS as u32 * ADDS_PER_THREAD);
    }
}
