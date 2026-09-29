use neonmix_core::{
    AudioFormat,
    capture::CaptureBridge,
    queue::block_queue,
    resample::RateConverter,
    signal::{SignalKind, StereoSource, TestSignal},
    stats::AudioStats,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, sync::Arc};
#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
#[test]
fn steady_state_capture_and_sinc_output_do_not_allocate_or_free() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(8, stats.clone()).unwrap();
    let mut capture = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats);
    let signal = TestSignal::new(SignalKind::Sine, 48000, 440.0, -24.0).unwrap();
    let mut resampler = RateConverter::new(signal, 48000, 44100).unwrap();
    let (mut publisher, reader) = neonmix_core::position::position_channel();
    let mut clock = neonmix_core::clock::OutputClock::new(44100, 1);
    let region = Region::new(ALLOCATOR);
    for i in 0..200 {
        publisher.publish(clock.observe_native(
            i * 10_000_000,
            i * 10_000_000 + 1_000_000,
            441,
            Some((i * 441) as i64),
        ));
        let _ = reader.latest();
        capture.ingest(480, i * 10_000_000, i * 10_000_000, |_| [0.1; 2]);
        let _ = c.pop_fresh(i * 10_000_000, 100_000_000);
        for _ in 0..441 {
            let _ = resampler.next_frame();
        }
    }
    let change = region.change();
    assert_eq!(change.allocations, 0);
    assert_eq!(change.reallocations, 0);
    assert_eq!(change.deallocations, 0);
}
