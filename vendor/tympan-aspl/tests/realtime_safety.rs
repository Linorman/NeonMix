//! Mechanical enforcement of `CLAUDE.md` prohibition #1 — the
//! `IOProc` realtime path and everything reachable from it must be
//! allocation-free.
//!
//! This integration test installs [`assert_no_alloc::AllocDisabler`]
//! as the test crate's global allocator and drives a
//! [`DriverInstance`] through a full lifecycle. The `process_io`
//! calls themselves run inside an [`assert_no_alloc::assert_no_alloc`]
//! guard, which aborts the test if any allocation traverses the
//! global allocator hook during that span.
//!
//! The framework's realtime layer is cross-platform, so this test
//! runs on any host — it does not need a macOS build or a
//! `coreaudiod` round-trip to be meaningful.

use assert_no_alloc::{assert_no_alloc, AllocDisabler};
use std::sync::Arc;

use tympan_aspl::driver::{AnyDriver, DriverInstance};
use tympan_aspl::io::{IoBuffer, IoOperation, Timestamp};
use tympan_aspl::{DeviceSpec, Driver, RealtimeContext, StreamFormat, StreamSpec};

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// A stereo loopback driver — the realtime fixture under test.
struct Loopback;

impl Driver for Loopback {
    const NAME: &'static str = "tympan-aspl realtime-safety fixture";
    const MANUFACTURER: &'static str = "tympan-aspl";
    const VERSION: &'static str = "0.0.0";

    fn new() -> Self {
        Self
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(48_000.0, 2);
        DeviceSpec::new("com.tympan.test.rt", "RT Fixture", Self::MANUFACTURER)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn process_io(&self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        let n = buffer.output.len().min(buffer.input.len());
        buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        buffer.output[n..].fill(0.0);
    }
}

#[test]
fn process_io_dispatch_is_allocation_free() {
    let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<Loopback>::new());
    driver.initialize().unwrap();
    driver.start_io().unwrap();

    // Pre-allocate the IO buffers *outside* the guarded span — the
    // realtime invariant is about the `process_io` dispatch, not
    // about test setup.
    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut output: Vec<f32> = vec![0.0; FRAMES];

    // Safety: a pure-logic integration test is case (2) of the
    // `RealtimeContext::new_unchecked` contract — it drives the
    // framework's `process_io` path in-process with full knowledge
    // that the allocation-free guarantee only covers that span.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    const ITERATIONS: usize = 64;
    assert_no_alloc(|| {
        for _ in 0..ITERATIONS {
            let mut buffer = IoBuffer::new(
                Timestamp::ZERO,
                IoOperation::PROCESS_OUTPUT,
                &input,
                &mut output,
            );
            driver.process_io(&rt, &mut buffer).unwrap();
        }
    });

    // Verify correctness outside the guard so the assertion helpers
    // are themselves free to allocate.
    for (&s_in, &s_out) in input.iter().zip(output.iter()) {
        assert!(s_out.is_finite());
        assert_eq!(s_out.to_bits(), s_in.to_bits());
    }

    driver.stop_io().unwrap();
}

#[test]
fn prepared_start_stop_are_allocation_free() {
    // Control-thread initialization may prepare platform mutex storage. Once
    // prepared, the no-contention start/stop dispatch remains allocation-free.
    let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<Loopback>::new());
    driver.initialize().unwrap();
    assert_no_alloc(|| {
        driver.start_io().unwrap();
        driver.stop_io().unwrap();
    });
}
