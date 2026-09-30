//! The top-level `Driver` trait and its framework-side wrapper.
//!
//! A *driver* is what a consumer of `tympan-aspl` implements: a type
//! carrying the per-plug-in state of an AudioServerPlugin. The
//! framework's CFPlugIn harness (the `raw` module, landing in a
//! later PR) constructs an instance via [`Driver::new`], drives the
//! `Initialize → StartIO → IOProc → StopIO` lifecycle, and routes
//! the HAL's calls into the corresponding trait methods.
//!
//! The framework wraps every user `Driver` in a [`DriverInstance<T>`]
//! — the object that owns the user state, tracks the lifecycle
//! [`State`], and holds the CFPlugIn [`Refcount`]. The CFPlugIn
//! bridge dispatches through the type-erased [`AnyDriver`] trait so
//! it never has to name the user's concrete `T`.
//!
//! This module is cross-platform: `DriverInstance<T>` and its state
//! machine are exercisable by unit tests on any host, which is how
//! the lifecycle invariants are verified without a `coreaudiod`
//! round-trip.

use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::device::DeviceSpec;
use crate::error::OsStatus;
use crate::io::IoBuffer;
use crate::realtime::{RealtimeContext, Refcount, State, StateCell};

/// A snapshot of a driver type's identity constants.
///
/// Built from a `T: Driver`'s associated constants with
/// [`DriverInfo::of`]. The framework's `bundle` module consumes one
/// of these to generate the `.driver` bundle's `Info.plist`, and the
/// CFPlugIn factory uses it to answer the HAL's plug-in property
/// queries.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct DriverInfo {
    /// Human-readable plug-in name (`T::NAME`).
    pub name: &'static str,
    /// Plug-in manufacturer / vendor string (`T::MANUFACTURER`).
    pub manufacturer: &'static str,
    /// Plug-in version string (`T::VERSION`), e.g. `"0.1.0"`.
    pub version: &'static str,
}

impl DriverInfo {
    /// Capture the identity constants of a `T: Driver`.
    #[must_use]
    pub const fn of<T: Driver>() -> Self {
        Self {
            name: T::NAME,
            manufacturer: T::MANUFACTURER,
            version: T::VERSION,
        }
    }
}

/// A user-implemented AudioServerPlugin driver.
///
/// Each implementor represents one plug-in with a distinct name,
/// manufacturer, and the audio device(s) it exposes. The framework
/// instantiates the type via [`Self::new`], answers the HAL's
/// property queries from [`Self::device`] and the
/// [`DriverInfo`] constants, drives the
/// `Initialize → StartIO → IOProc → StopIO` lifecycle, and routes
/// each IO cycle into [`Self::process_io`].
///
/// ## Realtime safety
///
/// [`Self::process_io`] runs on the Core Audio realtime thread and
/// takes a [`RealtimeContext`] witness. Any helper reachable from it
/// should also take `&RealtimeContext`, which makes the constraint
/// visible at compile time. The realtime path must be
/// allocation-free, lock-free, and free of blocking syscalls, per
/// `CLAUDE.md` prohibitions 1 and 2. [`Self::new`],
/// [`Self::initialize`], and [`Self::start_io`] run on non-realtime
/// threads and may allocate freely — pre-allocate processing
/// buffers there.
pub trait Driver: Sized + Send + Sync {
    /// Human-readable plug-in name. Surfaced in the bundle
    /// `Info.plist` and in the HAL's plug-in property protocol.
    const NAME: &'static str;

    /// Plug-in manufacturer / vendor string.
    const MANUFACTURER: &'static str;

    /// Plug-in version string, e.g. `"0.1.0"`. Written to the
    /// bundle `Info.plist` as `CFBundleVersion`.
    const VERSION: &'static str;

    /// Construct a fresh driver instance.
    ///
    /// Called by the framework's CFPlugIn factory once per plug-in
    /// activation. Heap allocation is allowed here; it is *not*
    /// allowed inside [`Self::process_io`].
    fn new() -> Self;

    /// Describe the audio device this driver exposes.
    ///
    /// Called by the framework while answering the HAL's
    /// `kAudioPlugInPropertyDeviceList` and the per-device property
    /// queries. The returned [`DeviceSpec`] is expected to be stable
    /// for the lifetime of the driver instance.
    fn device(&self) -> DeviceSpec;

    /// Prepare the driver for use.
    ///
    /// Called once by the HAL via the plug-in's `Initialize` entry
    /// point, before any device IO. Returning an [`OsStatus`]
    /// failure aborts plug-in initialisation.
    ///
    /// The default implementation succeeds without doing anything.
    fn initialize(&self) -> Result<(), OsStatus> {
        Ok(())
    }

    /// Begin device IO.
    ///
    /// Called by the HAL via the device's `StartIO` entry point
    /// when the first client starts the device. This is where a
    /// driver pre-allocates the buffers [`Self::process_io`] will
    /// use — allocation here is allowed, allocation in
    /// `process_io` is not. Returning an [`OsStatus`] failure
    /// aborts the start and leaves the device idle.
    ///
    /// The default implementation succeeds without doing anything.
    fn start_io(&self) -> Result<(), OsStatus> {
        Ok(())
    }

    /// End device IO.
    ///
    /// Called by the HAL via the device's `StopIO` entry point when
    /// the last client stops the device. Always paired with a prior
    /// successful [`Self::start_io`]; releasing the resources it
    /// acquired is allowed here.
    ///
    /// The default implementation does nothing.
    fn stop_io(&self) {}

    /// Process one IO cycle.
    ///
    /// Realtime-critical: must be allocation-free, lock-free, and
    /// must not call into the kernel. Reachable helpers should take
    /// `&RealtimeContext` to keep the constraint visible throughout
    /// the call graph.
    ///
    /// `buffer` carries the cycle's [`Timestamp`](crate::io::Timestamp),
    /// the [`IoOperation`](crate::io::IoOperation) that produced it,
    /// the input samples (empty for an output-only device), and the
    /// output slice to write into (empty for an input-only device).
    /// A loopback device's implementation copies `buffer.input`
    /// into `buffer.output`.
    fn process_io(&self, rt: &RealtimeContext, buffer: &mut IoBuffer<'_>);
}

/// A type-erased view of a [`DriverInstance<T>`].
///
/// The framework's CFPlugIn bridge wires the HAL's vtable to a
/// `dyn AnyDriver` so it never has to name the user's concrete
/// `T: Driver`. The trait surfaces every method `DriverInstance<T>`
/// exposes, dispatched through `Arc<dyn AnyDriver>`.
pub trait AnyDriver: Send + Sync {
    /// CFPlugIn `IUnknown::AddRef`.
    fn add_ref(&self) -> u32;
    /// CFPlugIn `IUnknown::Release`.
    fn release(&self) -> u32;
    /// Current CFPlugIn reference count.
    fn refcount(&self) -> u32;
    /// Current lifecycle [`State`].
    fn state(&self) -> State;

    /// Lifecycle: `Uninitialized → Initialized`, forwarding to the
    /// user's [`Driver::initialize`].
    fn initialize(&self) -> Result<(), OsStatus>;
    /// Lifecycle: `Initialized → Running`, forwarding to the user's
    /// [`Driver::start_io`].
    fn start_io(&self) -> Result<(), OsStatus>;
    /// Lifecycle: `Running → Initialized`, forwarding to the user's
    /// [`Driver::stop_io`].
    fn stop_io(&self) -> Result<(), OsStatus>;

    /// Realtime: drive one IO cycle through the user's
    /// [`Driver::process_io`]. Fails with [`OsStatus::NOT_RUNNING`]
    /// when the instance is not currently in [`State::Running`].
    fn process_io(&self, rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) -> Result<(), OsStatus>;

    /// The device this driver exposes (`T::device`).
    fn device(&self) -> DeviceSpec;
    /// The driver's identity constants.
    fn info(&self) -> DriverInfo;
}

/// The framework-side wrapper around a user `T: Driver`.
///
/// Owns the user's driver instance and tracks its lifecycle
/// ([`StateCell`]) and CFPlugIn ownership ([`Refcount`]).
/// Constructed by the framework's CFPlugIn factory and handed to the
/// HAL as the plug-in object. Users do not interact with this type
/// directly — they implement [`Driver`] and the framework wraps it.
///
/// ## Threading
/// Drivers expose shared, Sync callbacks: input/output may overlap without creating
/// aliased mutable references. Lifecycle calls serialize on a control-thread mutex.
/// The high gate bit closes admission; StopIO drains admitted callbacks off realtime.
/// IO admission makes at most eight atomic attempts and never waits on the mutex.
pub struct DriverInstance<T: Driver> {
    inner: T,
    state: StateCell,
    refcount: Refcount,
    lifecycle: Mutex<()>,
    io_gate: AtomicU32,
}

const IO_CLOSED: u32 = 1 << 31;
struct IoLease<'a>(&'a AtomicU32);
impl Drop for IoLease<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl<T: Driver> DriverInstance<T> {
    /// Construct a fresh instance in [`State::Uninitialized`] with
    /// CFPlugIn refcount 1.
    ///
    /// Calls [`Driver::new`] to materialise the user's driver state;
    /// heap allocation is permitted here.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: T::new(),
            lifecycle: Mutex::new(()),
            io_gate: AtomicU32::new(IO_CLOSED),
            state: StateCell::new(),
            refcount: Refcount::new(),
        }
    }

    /// Current lifecycle [`State`].
    #[inline]
    #[must_use]
    pub fn state(&self) -> State {
        self.state.load()
    }

    /// Current CFPlugIn reference count.
    #[inline]
    #[must_use]
    pub fn refcount(&self) -> u32 {
        self.refcount.count()
    }

    /// Increment the CFPlugIn reference count, returning the new
    /// value. Delegates to [`Refcount::add_ref`].
    #[inline]
    pub fn add_ref(&self) -> u32 {
        self.refcount.add_ref()
    }

    /// Decrement the CFPlugIn reference count, returning the new
    /// value. Delegates to [`Refcount::release`].
    #[inline]
    pub fn release(&self) -> u32 {
        self.refcount.release()
    }

    /// Transition `Uninitialized → Initialized` and call
    /// [`Driver::initialize`] on the user.
    ///
    /// Rolls the state machine back to `Uninitialized` if the
    /// user's `initialize` fails, so the HAL can retry. Surfaces
    /// [`OsStatus::ILLEGAL_OPERATION`] when the instance is not in
    /// [`State::Uninitialized`].
    pub fn initialize(&self) -> Result<(), OsStatus> {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        self.state
            .initialize()
            .map_err(|_| OsStatus::ILLEGAL_OPERATION)?;
        if let Err(status) = self.inner.initialize() {
            let _ = self.state.reset();
            return Err(status);
        }
        Ok(())
    }

    /// Publish a fresh run only after the driver's preparation is complete.
    pub fn start_io(&self) -> Result<(), OsStatus> {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        self.state
            .start()
            .map_err(|_| OsStatus::ILLEGAL_OPERATION)?;
        if let Err(status) = self.inner.start_io() {
            let _ = self.state.stop();
            return Err(status);
        }
        self.io_gate.store(0, Ordering::Release);
        Ok(())
    }

    /// Close admission before draining callbacks; only this control thread waits.
    pub fn stop_io(&self) -> Result<(), OsStatus> {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        if !self.state.is_running() {
            return Err(OsStatus::ILLEGAL_OPERATION);
        }
        self.io_gate.fetch_or(IO_CLOSED, Ordering::AcqRel);
        while self.io_gate.load(Ordering::Acquire) != IO_CLOSED {
            std::thread::yield_now();
        }
        self.inner.stop_io();
        self.state.stop().map_err(|_| OsStatus::ILLEGAL_OPERATION)?;
        Ok(())
    }

    /// Shared realtime dispatch. Concurrent input/output are admitted together;
    /// closed or contended admission returns silence without blocking or allocating.
    pub fn process_io(
        &self,
        rt: &RealtimeContext,
        buffer: &mut IoBuffer<'_>,
    ) -> Result<(), OsStatus> {
        let mut gate = self.io_gate.load(Ordering::Acquire);
        for _ in 0..8 {
            if gate >= IO_CLOSED - 1 {
                buffer.silence_output();
                return Err(OsStatus::NOT_RUNNING);
            }
            match self.io_gate.compare_exchange_weak(
                gate,
                gate + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let _lease = IoLease(&self.io_gate);
                    self.inner.process_io(rt, buffer);
                    return Ok(());
                }
                Err(current) => gate = current,
            }
        }
        buffer.silence_output();
        Err(OsStatus::NOT_READY)
    }

    /// Device metadata is shared immutable state.
    #[must_use]
    pub fn device(&self) -> DeviceSpec {
        self.inner.device()
    }

    /// The driver's identity constants.
    #[inline]
    #[must_use]
    pub fn info(&self) -> DriverInfo {
        DriverInfo::of::<T>()
    }
}

impl<T: Driver> Default for DriverInstance<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Driver> AnyDriver for DriverInstance<T> {
    #[inline]
    fn add_ref(&self) -> u32 {
        Self::add_ref(self)
    }
    #[inline]
    fn release(&self) -> u32 {
        Self::release(self)
    }
    #[inline]
    fn refcount(&self) -> u32 {
        Self::refcount(self)
    }
    #[inline]
    fn state(&self) -> State {
        Self::state(self)
    }
    #[inline]
    fn initialize(&self) -> Result<(), OsStatus> {
        Self::initialize(self)
    }
    #[inline]
    fn start_io(&self) -> Result<(), OsStatus> {
        Self::start_io(self)
    }
    #[inline]
    fn stop_io(&self) -> Result<(), OsStatus> {
        Self::stop_io(self)
    }
    #[inline]
    fn process_io(&self, rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) -> Result<(), OsStatus> {
        Self::process_io(self, rt, buffer)
    }
    #[inline]
    fn device(&self) -> DeviceSpec {
        Self::device(self)
    }
    #[inline]
    fn info(&self) -> DriverInfo {
        Self::info(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceSpec;
    use crate::format::StreamFormat;
    use crate::io::{IoOperation, Timestamp};
    use crate::stream::StreamSpec;
    struct Counter(core::sync::atomic::AtomicU32);
    impl Counter {
        fn new(value: u32) -> Self {
            Self(core::sync::atomic::AtomicU32::new(value))
        }
        fn get(&self) -> u32 {
            self.0.load(Ordering::Relaxed)
        }
        fn set(&self, value: u32) {
            self.0.store(value, Ordering::Relaxed);
        }
    }
    struct Flag(core::sync::atomic::AtomicBool);
    impl Flag {
        fn new(value: bool) -> Self {
            Self(core::sync::atomic::AtomicBool::new(value))
        }
        fn get(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
        fn set(&self, value: bool) {
            self.0.store(value, Ordering::Relaxed);
        }
    }
    use static_assertions::assert_impl_all;

    /// Reference driver: a stereo loopback that copies input to
    /// output. Carries atomic counters so tests can observe which
    /// lifecycle hooks fired.
    struct Loopback {
        initialize_seen: Counter,
        start_seen: Counter,
        stop_seen: Counter,
        process_seen: Counter,
        start_should_fail: Flag,
    }

    impl Driver for Loopback {
        const NAME: &'static str = "tympan-aspl loopback (test)";
        const MANUFACTURER: &'static str = "tympan-aspl";
        const VERSION: &'static str = "0.0.0";

        fn new() -> Self {
            Self {
                initialize_seen: Counter::new(0),
                start_seen: Counter::new(0),
                stop_seen: Counter::new(0),
                process_seen: Counter::new(0),
                start_should_fail: Flag::new(false),
            }
        }

        fn device(&self) -> DeviceSpec {
            DeviceSpec::new(
                "com.tympan.test.loopback",
                "Test Loopback",
                Self::MANUFACTURER,
            )
            .with_input(StreamSpec::input(StreamFormat::float32(48_000.0, 2)))
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 2)))
        }

        fn initialize(&self) -> Result<(), OsStatus> {
            self.initialize_seen.set(self.initialize_seen.get() + 1);
            Ok(())
        }

        fn start_io(&self) -> Result<(), OsStatus> {
            if self.start_should_fail.get() {
                return Err(OsStatus::NOT_READY);
            }
            self.start_seen.set(self.start_seen.get() + 1);
            Ok(())
        }

        fn stop_io(&self) {
            self.stop_seen.set(self.stop_seen.get() + 1);
        }

        fn process_io(&self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
            self.process_seen.set(self.process_seen.get() + 1);
            let n = buffer.output.len().min(buffer.input.len());
            buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        }
    }

    assert_impl_all!(DriverInstance<Loopback>: Sync, AnyDriver);

    fn rt() -> RealtimeContext {
        // Safety: a pure-logic unit test is case (2) of the
        // `RealtimeContext::new_unchecked` contract.
        unsafe { RealtimeContext::new_unchecked() }
    }

    #[test]
    fn concurrent_callbacks_are_shared_and_stop_drains_before_restart() {
        use std::sync::{Arc, Barrier};
        struct Overlap {
            entered: Barrier,
            finish: Barrier,
            stopped: core::sync::atomic::AtomicBool,
        }
        impl Driver for Overlap {
            const NAME: &'static str = "overlap test";
            const MANUFACTURER: &'static str = "test";
            const VERSION: &'static str = "0";
            fn new() -> Self {
                Self {
                    entered: Barrier::new(3),
                    finish: Barrier::new(3),
                    stopped: core::sync::atomic::AtomicBool::new(false),
                }
            }
            fn device(&self) -> DeviceSpec {
                DeviceSpec::new("overlap", "overlap", "test")
            }
            fn process_io(&self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
                // Deliberately parked mock callbacks make lifecycle overlap
                // deterministic. These barriers are never used by the product driver.
                self.entered.wait();
                self.finish.wait();
                buffer.silence_output();
            }
            fn stop_io(&self) {
                self.stopped.store(true, Ordering::Release);
            }
        }
        let driver = Arc::new(DriverInstance::<Overlap>::new());
        driver.initialize().unwrap();
        driver.start_io().unwrap();
        let mut callbacks = Vec::new();
        for _ in 0..2 {
            let driver = driver.clone();
            callbacks.push(std::thread::spawn(move || {
                let mut output = [1.0; 2];
                driver
                    .process_io(
                        &rt(),
                        &mut IoBuffer::new(
                            Timestamp::ZERO,
                            IoOperation::READ_INPUT,
                            &[],
                            &mut output,
                        ),
                    )
                    .unwrap();
                assert_eq!(output, [0.0; 2]);
            }));
        }
        driver.inner.entered.wait();
        let stopping = driver.clone();
        let stop = std::thread::spawn(move || stopping.stop_io().unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while driver.io_gate.load(Ordering::Acquire) & IO_CLOSED == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!driver.inner.stopped.load(Ordering::Acquire));
        let mut output = [1.0; 2];
        assert_eq!(
            driver.process_io(
                &rt(),
                &mut IoBuffer::new(Timestamp::ZERO, IoOperation::READ_INPUT, &[], &mut output)
            ),
            Err(OsStatus::NOT_RUNNING)
        );
        assert_eq!(output, [0.0; 2]);
        driver.inner.finish.wait();
        for callback in callbacks {
            callback.join().unwrap();
        }
        stop.join().unwrap();
        assert!(driver.inner.stopped.load(Ordering::Acquire));
        driver.start_io().unwrap();
        driver.stop_io().unwrap();
    }

    #[test]
    fn new_starts_uninitialized_with_refcount_one() {
        let d = DriverInstance::<Loopback>::new();
        assert_eq!(d.state(), State::Uninitialized);
        assert_eq!(d.refcount(), 1);
    }

    #[test]
    fn default_matches_new() {
        let d: DriverInstance<Loopback> = DriverInstance::default();
        assert_eq!(d.state(), State::Uninitialized);
        assert_eq!(d.refcount(), 1);
    }

    #[test]
    fn add_ref_release_delegate_to_refcount() {
        let d = DriverInstance::<Loopback>::new();
        assert_eq!(d.add_ref(), 2);
        assert_eq!(d.add_ref(), 3);
        assert_eq!(d.release(), 2);
        assert_eq!(d.release(), 1);
    }

    #[test]
    fn info_reflects_associated_constants() {
        let d = DriverInstance::<Loopback>::new();
        let info = d.info();
        assert_eq!(info.name, "tympan-aspl loopback (test)");
        assert_eq!(info.manufacturer, "tympan-aspl");
        assert_eq!(info.version, "0.0.0");
    }

    #[test]
    fn device_is_forwarded_from_the_user_driver() {
        let d = DriverInstance::<Loopback>::new();
        let spec = d.device();
        assert_eq!(spec.uid(), "com.tympan.test.loopback");
        assert!(spec.is_loopback());
    }

    #[test]
    fn initialize_transitions_and_forwards_to_user() {
        let d = DriverInstance::<Loopback>::new();
        assert!(d.initialize().is_ok());
        assert_eq!(d.state(), State::Initialized);
        // Safety: no `&mut` alias is in flight in this single-
        // threaded test.
        let inner = &d.inner;
        assert_eq!(inner.initialize_seen.get(), 1);
    }

    #[test]
    fn double_initialize_is_illegal() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        assert_eq!(d.initialize(), Err(OsStatus::ILLEGAL_OPERATION));
    }

    #[test]
    fn start_requires_initialized() {
        let d = DriverInstance::<Loopback>::new();
        assert_eq!(d.start_io(), Err(OsStatus::ILLEGAL_OPERATION));
        assert_eq!(d.state(), State::Uninitialized);
    }

    #[test]
    fn start_io_transitions_and_forwards_to_user() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        d.start_io().unwrap();
        assert_eq!(d.state(), State::Running);
        // SAFETY: no `&mut` alias is in flight in this single-
        // threaded test.
        let inner = &d.inner;
        assert_eq!(inner.start_seen.get(), 1);
    }

    #[test]
    fn start_failure_rolls_state_back_to_initialized() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        // Safety: single-threaded test, no alias in flight.
        d.inner.start_should_fail.set(true);
        assert_eq!(d.start_io(), Err(OsStatus::NOT_READY));
        // Rolled back so the HAL can retry from Initialized.
        assert_eq!(d.state(), State::Initialized);
    }

    #[test]
    fn stop_io_returns_to_initialized() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        d.start_io().unwrap();
        d.stop_io().unwrap();
        assert_eq!(d.state(), State::Initialized);
        // SAFETY: no `&mut` alias is in flight in this single-
        // threaded test.
        let inner = &d.inner;
        assert_eq!(inner.stop_seen.get(), 1);
    }

    #[test]
    fn stop_without_start_is_illegal() {
        let d = DriverInstance::<Loopback>::new();
        assert_eq!(d.stop_io(), Err(OsStatus::ILLEGAL_OPERATION));
    }

    #[test]
    fn process_io_requires_running_state() {
        let d = DriverInstance::<Loopback>::new();
        let input = [0.0_f32; 4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        assert_eq!(d.process_io(&rt, &mut buffer), Err(OsStatus::NOT_RUNNING));

        d.initialize().unwrap();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        assert_eq!(d.process_io(&rt, &mut buffer), Err(OsStatus::NOT_RUNNING));
    }

    #[test]
    fn process_io_after_start_copies_input_to_output() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        d.start_io().unwrap();

        let input = [0.1_f32, -0.2, 0.3, -0.4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        d.process_io(&rt, &mut buffer).unwrap();
        assert_eq!(output, input);

        // SAFETY: no `&mut` alias is in flight in this single-
        // threaded test.
        let inner = &d.inner;
        assert_eq!(inner.process_seen.get(), 1);
    }

    #[test]
    fn full_lifecycle_round_trip() {
        let d = DriverInstance::<Loopback>::new();
        d.initialize().unwrap();
        d.start_io().unwrap();

        let input = [0.5_f32; 4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        for _ in 0..3 {
            let mut buffer = IoBuffer::new(
                Timestamp::ZERO,
                IoOperation::PROCESS_OUTPUT,
                &input,
                &mut output,
            );
            d.process_io(&rt, &mut buffer).unwrap();
        }
        d.stop_io().unwrap();
        assert_eq!(d.state(), State::Initialized);

        // Start / stop can repeat.
        d.start_io().unwrap();
        d.stop_io().unwrap();
        assert_eq!(d.state(), State::Initialized);

        // SAFETY: no `&mut` alias is in flight in this single-
        // threaded test.
        let inner = &d.inner;
        assert_eq!(inner.process_seen.get(), 3);
        assert_eq!(inner.start_seen.get(), 2);
        assert_eq!(inner.stop_seen.get(), 2);
    }

    #[test]
    fn type_erased_dispatch_drives_full_lifecycle() {
        use std::sync::Arc;
        let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<Loopback>::new());

        assert_eq!(driver.state(), State::Uninitialized);
        assert_eq!(driver.refcount(), 1);
        assert_eq!(driver.add_ref(), 2);

        driver.initialize().unwrap();
        driver.start_io().unwrap();
        assert_eq!(driver.state(), State::Running);

        let input = [0.25_f32, -0.5, 0.75, -1.0];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        driver.process_io(&rt, &mut buffer).unwrap();
        assert_eq!(output, input);

        driver.stop_io().unwrap();
        assert_eq!(driver.state(), State::Initialized);
        assert_eq!(driver.release(), 1);

        // The type-erased view still answers identity queries.
        assert_eq!(driver.info().name, "tympan-aspl loopback (test)");
        assert!(driver.device().is_loopback());
    }
}
