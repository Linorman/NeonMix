//! Realtime-safe log sink with an off-thread drainer.
//!
//! Realtime code cannot log via `tracing` or `log` — both allocate
//! and may block. The pattern from `docs/architecture.md`
//! § Logging is: the realtime `IOProc` path enqueues events into a
//! lock-free queue, and a separate non-realtime thread drains them —
//! formats, writes to stderr, forwards to the standard `log` crate,
//! whatever. [`LogSink`] packages that pattern so driver authors do
//! not have to thread together a [`Producer`], a [`Consumer`], a
//! shutdown signal, and a [`JoinHandle`] themselves.
//!
//! # Realtime properties
//!
//! [`LogSink::log`] forwards directly to [`Producer::try_push`]:
//! two relaxed/acquire/release atomic ops plus a `T`-sized memcpy.
//! No allocation, no syscall. The event is dropped on the floor if
//! the queue is full ([`LogSink::log`] returns `false`).
//!
//! Construction and destruction allocate and spawn / join a thread —
//! both run outside the realtime path (typically in the driver's
//! `Initialize` and the CFPlugIn `Release`).
//!
//! # Drainer behaviour
//!
//! - The drainer thread polls the consumer at a fixed interval
//!   ([`DRAINER_POLL_INTERVAL`]). Latency is bounded by that
//!   interval plus whatever `drain_one` takes per event.
//! - When [`LogSink`]'s destructor runs it signals shutdown and
//!   joins the thread. The drainer performs a final pass to flush
//!   any events pushed between its last poll and the shutdown
//!   signal.
//! - The user-supplied `drain_one` closure runs on the drainer
//!   thread. If it panics, the drainer thread terminates; subsequent
//!   `log` calls still succeed until the queue fills, and the
//!   driver's `IOProc` keeps working — a failing log sink does not
//!   crash audio.
//!
//! # Example
//!
//! ```rust
//! use tympan_aspl::realtime::log::LogSink;
//!
//! // Capacity 1024; drainer prints each event to stderr.
//! let logger = LogSink::<&'static str>::new(1024, |event| {
//!     eprintln!("driver: {event}");
//! });
//!
//! // From the realtime path (inside the IO callback):
//! assert!(logger.log("input buffer underran"));
//!
//! // When `logger` drops the drainer thread is joined; any
//! // remaining events are flushed first.
//! drop(logger);
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::ring::{self, Consumer, Producer};

/// How often the drainer thread checks for new events when none are
/// queued. One millisecond is well under typical audio buffer
/// durations (256 frames at 48 kHz = 5.3 ms) and small enough that
/// the shutdown delay on drop is unnoticeable.
pub const DRAINER_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// A realtime-safe log queue with a non-realtime drainer thread.
///
/// `T` is the event type. Driver authors typically define an enum
/// covering the diagnostic events their driver emits from the
/// realtime path.
pub struct LogSink<T: Send + 'static> {
    tx: Producer<T>,
    shutdown: Arc<AtomicBool>,
    drainer: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> LogSink<T> {
    /// Construct a new log sink.
    ///
    /// `capacity` is the bounded queue size. Events pushed when the
    /// queue is full are dropped silently — [`log`](Self::log)
    /// returns `false` in that case.
    ///
    /// `drain_one` is invoked once per event in the order they were
    /// pushed. It runs on the drainer thread, **not** the realtime
    /// thread, so it may allocate, syscall, or block freely. Every
    /// event is delivered exactly once before the destructor
    /// returns.
    ///
    /// # Panics
    ///
    /// Panics if `capacity == 0` (forwarded from
    /// [`ring::channel`]).
    pub fn new<F>(capacity: usize, mut drain_one: F) -> Self
    where
        F: FnMut(T) + Send + 'static,
    {
        let (tx, rx) = ring::channel::<T>(capacity);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);

        let drainer = thread::Builder::new()
            .name("tympan-aspl-log-drainer".into())
            .spawn(move || {
                drainer_loop(&rx, &shutdown_for_thread, &mut drain_one);
            })
            .expect("spawning the LogSink drainer thread failed");

        Self {
            tx,
            shutdown,
            drainer: Some(drainer),
        }
    }

    /// Push an event into the queue. Realtime-safe.
    ///
    /// Returns `true` if the event was enqueued, `false` if the
    /// queue was full (the event is dropped in that case). The
    /// caller can use the return value to bump a "dropped log
    /// events" counter if accounting matters.
    pub fn log(&self, event: T) -> bool {
        self.tx.try_push(event).is_ok()
    }

    /// Queue capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.tx.capacity()
    }
}

impl<T: Send + 'static> Drop for LogSink<T> {
    fn drop(&mut self) {
        // Signal shutdown. Release-store so the drainer's
        // Acquire-load picks up every queued event written before
        // this point.
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.drainer.take() {
            // If the drainer thread panicked (e.g. inside the
            // user's `drain_one`) `join` returns `Err(payload)`; we
            // deliberately discard it so dropping `LogSink` never
            // propagates a remote panic. The remote panic is a bug
            // in the user's closure, not a state the audio path
            // needs to handle.
            let _ = handle.join();
        }
    }
}

// The drainer worker runs on a dedicated, non-realtime background
// thread spawned by `LogSink`. By construction it is *not* on the
// audio engine's realtime thread — its job is to drain the ring on
// behalf of the realtime producer, off the audio thread. So
// `thread::sleep` here is the natural backoff between polls, and
// the realtime-path lints (which target the audio thread) do not
// apply to it.
#[allow(clippy::disallowed_methods)]
fn drainer_loop<T, F>(rx: &Consumer<T>, shutdown: &AtomicBool, drain_one: &mut F)
where
    T: Send + 'static,
    F: FnMut(T),
{
    loop {
        let mut drained = false;
        while let Some(event) = rx.try_pop() {
            drain_one(event);
            drained = true;
        }
        // Acquire pairs with the Release in `LogSink::drop`'s
        // `shutdown.store`, so observing `shutdown == true` here
        // implies every prior push is now visible.
        if shutdown.load(Ordering::Acquire) {
            // One last drain pass to flush events that landed after
            // our previous `try_pop` but before the shutdown store.
            while let Some(event) = rx.try_pop() {
                drain_one(event);
            }
            break;
        }
        if !drained {
            thread::sleep(DRAINER_POLL_INTERVAL);
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests legitimately need to verify the realtime drainer's
    // contract from a non-realtime test thread: a `Mutex` collects
    // drained events for assertion, and `thread::sleep` gives the
    // drainer time to make progress. Both are forbidden in the
    // realtime path itself (this module's outer
    // `#![deny(clippy::disallowed_methods, clippy::disallowed_types)]`)
    // but are the natural test-harness primitives here.
    #![allow(clippy::disallowed_methods, clippy::disallowed_types)]

    use super::*;
    use std::sync::Mutex;

    /// Collect drained events into a shared `Vec` for assertion.
    fn collector() -> (Arc<Mutex<Vec<u32>>>, impl FnMut(u32) + Send + 'static) {
        let store = Arc::new(Mutex::new(Vec::new()));
        let store_for_drainer = Arc::clone(&store);
        let drain = move |v: u32| {
            store_for_drainer.lock().unwrap().push(v);
        };
        (store, drain)
    }

    #[test]
    fn events_pushed_before_drop_are_all_drained() {
        let (store, drain) = collector();
        let sink = LogSink::<u32>::new(64, drain);
        for i in 0..20 {
            assert!(sink.log(i), "queue should not be full at item {i}");
        }
        // Dropping the sink shuts the drainer down after flushing.
        drop(sink);
        let collected = store.lock().unwrap();
        assert_eq!(*collected, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn capacity_matches_constructor_argument() {
        let sink = LogSink::<u32>::new(256, |_| {});
        assert_eq!(sink.capacity(), 256);
    }

    #[test]
    fn log_returns_false_when_queue_is_full() {
        // Drainer that blocks until we say it's OK, so the queue
        // genuinely fills up.
        let release = Arc::new(AtomicBool::new(false));
        let release_for_drainer = Arc::clone(&release);
        let drain = move |_v: u32| {
            while !release_for_drainer.load(Ordering::Acquire) {
                thread::sleep(Duration::from_micros(100));
            }
        };

        let sink = LogSink::<u32>::new(4, drain);

        // Fill the queue. The drainer may consume one item before it
        // starts blocking, so spin until we observe a `false`.
        let mut successes = 0;
        let mut saw_full = false;
        for i in 0..1_000 {
            if sink.log(i) {
                successes += 1;
            } else {
                saw_full = true;
                break;
            }
        }
        assert!(
            saw_full,
            "queue never reported full after {successes} pushes"
        );

        // Let the drainer go so dropping the sink can complete.
        release.store(true, Ordering::Release);
        drop(sink);
    }

    #[test]
    fn drop_flushes_events_pushed_after_the_last_poll() {
        // We can't easily synchronise "drainer is between polls"
        // from the outside, so this test pushes a steady stream of
        // events and then drops the sink immediately. Every event
        // pushed must appear in the collected output.
        let (store, drain) = collector();
        let sink = LogSink::<u32>::new(1024, drain);
        for i in 0..500 {
            assert!(sink.log(i));
        }
        drop(sink);
        let collected = store.lock().unwrap();
        assert_eq!(collected.len(), 500, "every pushed event must be drained");
        // Order is preserved (FIFO from the underlying ring).
        for (i, &v) in collected.iter().enumerate() {
            assert_eq!(v, i as u32);
        }
    }

    #[test]
    fn drainer_panic_is_swallowed_by_drop() {
        // A closure that panics on every event. The drainer thread
        // dies after the first event; subsequent pushes return
        // `true` until the queue fills (the queue itself is fine),
        // and dropping the sink does not propagate the panic.
        let sink = LogSink::<u32>::new(16, |_v| panic!("drainer suicide"));
        let _ = sink.log(0);
        // Generously yield so the drainer has a chance to receive
        // and panic.
        thread::sleep(Duration::from_millis(20));
        drop(sink);
    }
}
