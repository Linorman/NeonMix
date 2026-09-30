//! Atomic state machine for the AudioServerPlugin device lifecycle.
//!
//! The Core Audio HAL drives a plug-in's device through a small
//! finite lifecycle:
//!
//! ```text
//!  Uninitialized ──Initialize──▶ Initialized ──StartIO──▶ Running
//!                                    ▲                       │
//!                                    └────────StopIO─────────┘
//! ```
//!
//! The realtime `IOProc` callbacks (`BeginIOOperation`,
//! `DoIOOperation`, `EndIOOperation`) are callable only while the
//! state is [`State::Running`]. Several of the framework's
//! invariants (e.g. "the negotiated stream format is pinned for the
//! duration of an IO cycle") follow from the HAL obeying this
//! ordering, but the framework still verifies it: every transition
//! goes through [`StateCell`]'s atomic compare-exchange, and bad
//! transitions surface as [`TransitionError`] rather than silently
//! corrupting state.

use core::sync::atomic::{AtomicU8, Ordering};

/// AudioServerPlugin device lifecycle state.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum State {
    /// The object exists but the HAL has not yet called
    /// `Initialize` on the plug-in.
    Uninitialized = 0,
    /// `Initialize` has succeeded; `StartIO` has not been called
    /// (or has been undone by `StopIO`).
    Initialized = 1,
    /// `StartIO` has succeeded; the realtime `IOProc` callbacks are
    /// callable.
    Running = 2,
}

impl State {
    /// Returns `true` if the realtime `IOProc` callbacks are legal
    /// in this state.
    #[inline]
    #[must_use]
    pub const fn allows_io(self) -> bool {
        matches!(self, State::Running)
    }

    #[inline]
    const fn from_u8(value: u8) -> Self {
        // Writers go through `StateCell`, which only ever stores
        // values produced by `State as u8`. The match is exhaustive
        // for the legal byte set; the panic arm guards against a
        // future variant reaching this fn without the match being
        // updated.
        match value {
            0 => State::Uninitialized,
            1 => State::Initialized,
            2 => State::Running,
            _ => panic!("StateCell stored an invalid State byte"),
        }
    }
}

/// The outcome of a failed transition.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TransitionError {
    /// The state the caller assumed the cell was in.
    pub expected: State,
    /// The state the caller tried to transition into.
    pub attempted: State,
    /// The state actually observed in the cell.
    pub actual: State,
}

/// Atomic carrier for [`State`].
///
/// Holds the current lifecycle state and serialises transitions via
/// `compare_exchange`. Cheap to load from the realtime path
/// ([`Self::load`] is an `Acquire` load — no allocation, no kernel
/// involvement), and safe to share between threads.
#[derive(Debug)]
pub struct StateCell {
    state: AtomicU8,
}

impl StateCell {
    /// Construct a fresh cell in [`State::Uninitialized`].
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(State::Uninitialized as u8),
        }
    }

    /// Load the current state with `Acquire` ordering. Safe to call
    /// from the realtime path.
    #[inline]
    #[must_use]
    pub fn load(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    /// `true` iff [`Self::load`] currently returns
    /// [`State::Running`].
    #[inline]
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.load() == State::Running
    }

    /// Transition `Uninitialized → Initialized` (the HAL's
    /// `Initialize`).
    #[inline]
    pub fn initialize(&self) -> Result<(), TransitionError> {
        self.transition(State::Uninitialized, State::Initialized)
    }

    /// Transition `Initialized → Running` (the HAL's `StartIO`).
    #[inline]
    pub fn start(&self) -> Result<(), TransitionError> {
        self.transition(State::Initialized, State::Running)
    }

    /// Transition `Running → Initialized` (the HAL's `StopIO`).
    #[inline]
    pub fn stop(&self) -> Result<(), TransitionError> {
        self.transition(State::Running, State::Initialized)
    }

    /// Unconditional reset to [`State::Uninitialized`]. Returns the
    /// state observed before the reset.
    ///
    /// Used by the CFPlugIn `Release` path once the final reference
    /// is dropped, and by destructors that need to put the cell into
    /// a known terminal state regardless of where it was.
    #[inline]
    pub fn reset(&self) -> State {
        State::from_u8(
            self.state
                .swap(State::Uninitialized as u8, Ordering::AcqRel),
        )
    }

    fn transition(&self, from: State, to: State) -> Result<(), TransitionError> {
        match self
            .state
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            Err(actual_byte) => Err(TransitionError {
                expected: from,
                attempted: to,
                actual: State::from_u8(actual_byte),
            }),
        }
    }
}

impl Default for StateCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    assert_impl_all!(StateCell: Send, Sync);

    #[test]
    fn new_cell_starts_uninitialized() {
        let s = StateCell::new();
        assert_eq!(s.load(), State::Uninitialized);
        assert!(!s.is_running());
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(StateCell::default().load(), State::Uninitialized);
    }

    #[test]
    fn initialize_uninitialized_to_initialized() {
        let s = StateCell::new();
        assert!(s.initialize().is_ok());
        assert_eq!(s.load(), State::Initialized);
    }

    #[test]
    fn initialize_when_already_initialized_errors() {
        let s = StateCell::new();
        s.initialize().unwrap();
        let err = s.initialize().unwrap_err();
        assert_eq!(err.expected, State::Uninitialized);
        assert_eq!(err.attempted, State::Initialized);
        assert_eq!(err.actual, State::Initialized);
    }

    #[test]
    fn start_requires_initialized() {
        let s = StateCell::new();
        let err = s.start().unwrap_err();
        assert_eq!(err.expected, State::Initialized);
        assert_eq!(err.attempted, State::Running);
        assert_eq!(err.actual, State::Uninitialized);

        s.initialize().unwrap();
        assert!(s.start().is_ok());
        assert!(s.is_running());
        assert!(s.load().allows_io());
    }

    #[test]
    fn start_when_already_running_errors() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.start().unwrap();
        let err = s.start().unwrap_err();
        assert_eq!(err.actual, State::Running);
    }

    #[test]
    fn stop_requires_running() {
        let s = StateCell::new();
        let err = s.stop().unwrap_err();
        assert_eq!(err.expected, State::Running);
        assert_eq!(err.attempted, State::Initialized);
        assert_eq!(err.actual, State::Uninitialized);
    }

    #[test]
    fn stop_returns_to_initialized() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.start().unwrap();
        s.stop().unwrap();
        assert_eq!(s.load(), State::Initialized);
    }

    #[test]
    fn reset_from_any_state() {
        let s = StateCell::new();
        assert_eq!(s.reset(), State::Uninitialized);

        s.initialize().unwrap();
        assert_eq!(s.reset(), State::Initialized);
        assert_eq!(s.load(), State::Uninitialized);

        s.initialize().unwrap();
        s.start().unwrap();
        assert_eq!(s.reset(), State::Running);
        assert_eq!(s.load(), State::Uninitialized);
    }

    #[test]
    fn full_lifecycle_round_trip() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.start().unwrap();
        s.stop().unwrap();
        s.start().unwrap();
        s.stop().unwrap();
        let prior = s.reset();
        assert_eq!(prior, State::Initialized);
        assert_eq!(s.load(), State::Uninitialized);
    }

    #[test]
    fn allows_io_only_in_running() {
        assert!(!State::Uninitialized.allows_io());
        assert!(!State::Initialized.allows_io());
        assert!(State::Running.allows_io());
    }

    #[test]
    fn concurrent_initialize_has_exactly_one_winner() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 8;
        let s = Arc::new(StateCell::new());
        let wins = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let s = Arc::clone(&s);
                let wins = Arc::clone(&wins);
                thread::spawn(move || {
                    if s.initialize().is_ok() {
                        wins.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(wins.load(Ordering::Relaxed), 1);
        assert_eq!(s.load(), State::Initialized);
    }
}
