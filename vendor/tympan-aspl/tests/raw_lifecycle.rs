//! In-process lifecycle harness — drives the framework through its
//! actual `AudioServerPlugInDriverInterface` vtable, the C-ABI
//! surface `coreaudiod` calls.
//!
//! [`realtime_safety`](realtime_safety) guards the *safe* API
//! (`DriverInstance::process_io`). This harness goes one layer
//! lower: it builds a plug-in with the real
//! [`driver_factory_dispatch`] factory, then calls **through the
//! function pointers in the `'static` vtable** —
//! `Initialize → AddDeviceClient → StartIO → GetZeroTimeStamp →
//! WillDoIOOperation → (BeginIOOperation → DoIOOperation →
//! EndIOOperation) × N → StopIO → RemoveDeviceClient → Release`.
//! That exercises the hand-written `raw` ABI layer end-to-end: the
//! vtable wiring, the `DriverObject` recovery, the marshalling, the
//! `DoIOOperation` stack-scratch-buffer bridge, and the device
//! ring — none of which the safe-API harness touches.
//!
//! The `DoIOOperation` cycles run inside an
//! [`assert_no_alloc::assert_no_alloc`] guard: the test crate and
//! the framework rlib are the same link unit, so the global
//! allocator hook intercepts any allocation on the realtime data
//! path and aborts. (`GetZeroTimeStamp` is deliberately *outside*
//! the guard — it is a clock/timing callback that locks the device
//! state, not part of the `DoIOOperation` data path whose
//! alloc-freedom this asserts.)
//!
//! The `raw` layer is cross-platform — only `raw::cf` is macOS-only
//! and it is not on this path — so the harness runs on any host,
//! in Tier 1, on every pull request. It does not `dlopen` the built
//! `minimal-loopback` bundle: a separately linked cdylib has its
//! own allocator symbols (the guard could not see into it), and the
//! `coreaudiod` bundle-load path is already Tier 3's job. The
//! `IUnknown` preamble (`QueryInterface`/`AddRef`/`Release`) is
//! covered by the unit tests in `raw`.

use core::ffi::c_void;
use std::sync::Arc;

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

use tympan_aspl::driver::{AnyDriver, DriverInstance};
use tympan_aspl::io::{IoBuffer, IoOperation};
use tympan_aspl::raw::abi::{
    AudioObjectID, AudioServerPlugInDriverInterface, AudioServerPlugInDriverRef,
    AudioServerPlugInIOCycleInfo, AudioTimeStamp, Boolean, Float64, UInt32, UInt64,
};
use tympan_aspl::raw::driver_factory_dispatch;
use tympan_aspl::{DeviceSpec, Driver, RealtimeContext, StreamFormat, StreamSpec};

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// Nominal sample rate and channel count of the fixture device.
const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: usize = 2;
/// Frames per IO cycle, and the resulting interleaved sample count.
const FRAMES: usize = 256;
const SAMPLES: usize = FRAMES * CHANNELS;
/// IO cycles driven through the vtable inside the alloc guard.
const ITERATIONS: usize = 32;

/// Arbitrary device / stream / client identifiers — the entry
/// points validate the driver ref but otherwise ignore these.
const DEVICE_ID: AudioObjectID = 2;
const STREAM_ID: AudioObjectID = 3;
const CLIENT_ID: UInt32 = 0;

/// A stereo loopback driver — the fixture whose `process_io` is the
/// identity copy, so what one IO cycle writes the next reads back.
struct RawLoopback;

impl Driver for RawLoopback {
    const NAME: &'static str = "tympan-aspl raw-lifecycle fixture";
    const MANUFACTURER: &'static str = "tympan-aspl";
    const VERSION: &'static str = "0.0.0";

    fn new() -> Self {
        Self
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(SAMPLE_RATE, CHANNELS as u32);
        DeviceSpec::new(
            "com.tympan.test.raw-lifecycle",
            "Raw Lifecycle",
            Self::MANUFACTURER,
        )
        .with_input(StreamSpec::input(format))
        .with_output(StreamSpec::output(format))
    }

    fn process_io(&self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        let n = buffer.output.len().min(buffer.input.len());
        buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        buffer.output[n..].fill(0.0);
    }
}

fn create() -> Arc<dyn AnyDriver> {
    Arc::new(DriverInstance::<RawLoopback>::new())
}

/// An `AudioServerPlugInIOCycleInfo` whose input and output
/// timelines both sit at `sample_time` — so a `WriteMix` at
/// `sample_time` and a `ReadInput` at the same `sample_time`
/// address the same slot of the device ring.
fn cycle_at(sample_time: f64) -> AudioServerPlugInIOCycleInfo {
    let ts = AudioTimeStamp {
        mSampleTime: sample_time,
        ..Default::default()
    };
    AudioServerPlugInIOCycleInfo {
        mInputTime: ts,
        mOutputTime: ts,
        ..Default::default()
    }
}

#[test]
fn raw_factory_drives_a_full_io_lifecycle_through_the_vtable() {
    // The factory hands back the single owning reference, exactly as
    // `coreaudiod` receives it.
    // SAFETY: the factory dereferences neither pointer argument, so
    // null is sound for the allocator and the requested type UUID.
    let object = unsafe { driver_factory_dispatch(core::ptr::null(), core::ptr::null(), create) };
    assert!(!object.is_null(), "factory returned a null driver object");
    let driver_ref: AudioServerPlugInDriverRef = object.cast();

    // The vtable is the object's first word; recover it once and
    // call every entry point through its function pointers.
    // SAFETY: `driver_ref` came straight from the factory, so its
    // first word is the framework's `'static` vtable pointer.
    let vtable: &AudioServerPlugInDriverInterface = unsafe { &**driver_ref };

    let initialize = vtable.Initialize.expect("Initialize slot is wired");
    let add_client = vtable
        .AddDeviceClient
        .expect("AddDeviceClient slot is wired");
    let start_io = vtable.StartIO.expect("StartIO slot is wired");
    let stop_io = vtable.StopIO.expect("StopIO slot is wired");
    let remove_client = vtable
        .RemoveDeviceClient
        .expect("RemoveDeviceClient slot is wired");
    let get_zero_time_stamp = vtable
        .GetZeroTimeStamp
        .expect("GetZeroTimeStamp slot is wired");
    let will_do = vtable
        .WillDoIOOperation
        .expect("WillDoIOOperation slot is wired");
    let begin = vtable
        .BeginIOOperation
        .expect("BeginIOOperation slot is wired");
    let do_io = vtable.DoIOOperation.expect("DoIOOperation slot is wired");
    let end = vtable.EndIOOperation.expect("EndIOOperation slot is wired");
    let release = vtable.Release.expect("Release slot is wired");

    // --- bring the device up -------------------------------------
    // SAFETY: `driver_ref` is the live object the factory built; a
    // null host ref is explicitly permitted by `Initialize`.
    let initialized = unsafe { initialize(driver_ref, core::ptr::null_mut()) };
    assert_eq!(initialized, 0, "Initialize failed");
    let client = tympan_aspl::raw::abi::AudioServerPlugInClientInfo {
        mClientID: CLIENT_ID,
        mProcessID: 1,
        mIsNativeEndian: 1,
        mBundleID: core::ptr::null(),
    };
    // SAFETY: live driver ref and readable client record matching the SDK contract.
    let client_added = unsafe { add_client(driver_ref, DEVICE_ID, &client) };
    assert_eq!(client_added, 0, "AddDeviceClient failed");
    // SAFETY: live driver ref.
    let io_started = unsafe { start_io(driver_ref, DEVICE_ID, CLIENT_ID) };
    assert_eq!(io_started, 0, "StartIO failed");

    // `GetZeroTimeStamp` — the IO clock callback. Driven outside the
    // alloc guard to separate clock setup from
    // the `DoIOOperation` data path.
    let mut sample_time: Float64 = -1.0;
    let mut host_time: UInt64 = 0;
    let mut seed: UInt64 = 0;
    // SAFETY: live driver ref; the three out-pointers are valid,
    // writable, and outlive the call.
    let zts = unsafe {
        get_zero_time_stamp(
            driver_ref,
            DEVICE_ID,
            CLIENT_ID,
            &mut sample_time,
            &mut host_time,
            &mut seed,
        )
    };
    assert_eq!(zts, 0, "GetZeroTimeStamp failed");
    assert!(
        sample_time.is_finite() && sample_time >= 0.0,
        "GetZeroTimeStamp reported a nonsensical sample time: {sample_time}"
    );

    // `WillDoIOOperation` — the framework claims the two
    // data-movement operations and declines a whole-cycle marker.
    for (operation, expected) in [
        (IoOperation::WRITE_MIX, 1u8),
        (IoOperation::READ_INPUT, 1),
        (IoOperation::CYCLE, 0),
    ] {
        let mut will: Boolean = 0xFF;
        let mut in_place: Boolean = 0xFF;
        // SAFETY: live driver ref; both out-pointers are valid and
        // writable.
        let status = unsafe {
            will_do(
                driver_ref,
                DEVICE_ID,
                CLIENT_ID,
                operation.code().as_u32(),
                &mut will,
                &mut in_place,
            )
        };
        assert_eq!(status, 0, "WillDoIOOperation({operation:?}) failed");
        assert_eq!(
            will, expected,
            "WillDoIOOperation({operation:?}) reported the wrong handling"
        );
    }

    // --- drive the IO cycles under the alloc guard ---------------
    // Buffers and result slots are pre-allocated *outside* the
    // guard; the realtime invariant is about the per-cycle entry
    // points, not the harness setup.
    let input: [f32; SAMPLES] = core::array::from_fn(|i| (i as f32 / SAMPLES as f32) * 2.0 - 1.0);
    let mut readback = [0.0_f32; SAMPLES];
    let mut write_status = [i32::MIN; ITERATIONS];
    let mut read_status = [i32::MIN; ITERATIONS];
    let mut roundtrip_ok = [false; ITERATIONS];
    let write_op = IoOperation::WRITE_MIX.code().as_u32();
    let read_op = IoOperation::READ_INPUT.code().as_u32();

    assert_no_alloc(|| {
        for iter in 0..ITERATIONS {
            // Each cycle uses a distinct sample time so the ring
            // round-trips through a fresh slot.
            let cycle = cycle_at((iter * FRAMES) as f64);

            // WriteMix: the HAL buffer holds the client's output;
            // the framework runs it through `process_io` and stores
            // the result in the device ring.
            // SAFETY: live driver ref; `&cycle` is a valid cycle
            // info; `input` is `SAMPLES` readable f32s, the size
            // `WriteMix` reads for a `FRAMES`-frame stereo cycle.
            unsafe {
                begin(
                    driver_ref,
                    DEVICE_ID,
                    CLIENT_ID,
                    write_op,
                    FRAMES as UInt32,
                    &cycle,
                );
                write_status[iter] = do_io(
                    driver_ref,
                    DEVICE_ID,
                    4,
                    CLIENT_ID,
                    write_op,
                    FRAMES as UInt32,
                    &cycle,
                    input.as_ptr() as *mut c_void,
                    core::ptr::null_mut(),
                );
                end(
                    driver_ref,
                    DEVICE_ID,
                    CLIENT_ID,
                    write_op,
                    FRAMES as UInt32,
                    &cycle,
                );
            }

            // ReadInput: the framework reads the device ring at the
            // same sample time and writes it into the HAL buffer.
            readback.fill(0.0);
            // SAFETY: live driver ref; `&cycle` is valid; `readback`
            // is `SAMPLES` writable f32s, the size `ReadInput`
            // writes for a `FRAMES`-frame stereo cycle.
            unsafe {
                begin(
                    driver_ref,
                    DEVICE_ID,
                    CLIENT_ID,
                    read_op,
                    FRAMES as UInt32,
                    &cycle,
                );
                read_status[iter] = do_io(
                    driver_ref,
                    DEVICE_ID,
                    STREAM_ID,
                    CLIENT_ID,
                    read_op,
                    FRAMES as UInt32,
                    &cycle,
                    readback.as_mut_ptr() as *mut c_void,
                    core::ptr::null_mut(),
                );
                end(
                    driver_ref,
                    DEVICE_ID,
                    CLIENT_ID,
                    read_op,
                    FRAMES as UInt32,
                    &cycle,
                );
            }

            // The loopback round-trip: what WriteMix stored, ReadInput
            // returns bit-for-bit. Compared in-loop without
            // allocating; the verdict is asserted after the guard.
            roundtrip_ok[iter] = readback
                .iter()
                .zip(input.iter())
                .all(|(out, inp)| out.to_bits() == inp.to_bits());
        }
    });

    // Verdicts are asserted outside the guard so the assertion
    // machinery is free to allocate on failure.
    for iter in 0..ITERATIONS {
        assert_eq!(
            write_status[iter], 0,
            "DoIOOperation(WriteMix) cycle {iter}"
        );
        assert_eq!(
            read_status[iter], 0,
            "DoIOOperation(ReadInput) cycle {iter}"
        );
        assert!(
            roundtrip_ok[iter],
            "cycle {iter}: ReadInput did not return what WriteMix stored"
        );
    }

    // --- tear the device down ------------------------------------
    // SAFETY: live driver ref.
    let io_stopped = unsafe { stop_io(driver_ref, DEVICE_ID, CLIENT_ID) };
    assert_eq!(io_stopped, 0, "StopIO failed");
    // SAFETY: live driver ref and the same readable client record.
    let client_removed = unsafe { remove_client(driver_ref, DEVICE_ID, &client) };
    assert_eq!(client_removed, 0, "RemoveDeviceClient failed");
    // Mirror the HAL's final `Release`: the count returns to zero
    // and the driver object is freed.
    // SAFETY: `object` is the single owning reference the factory
    // produced; after this call it must not be touched again.
    let remaining = unsafe { release(object) };
    assert_eq!(remaining, 0, "final Release should free the object");
}
