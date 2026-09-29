use neonmix_core::{
    capture::CaptureBridge, clock::*, queue::block_queue, resample::*, signal::*, stats::*, *,
};
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[test]
fn format_rejects_invalid_rate_and_layout() {
    for rate in [0, 8000, u32::MAX] {
        assert!(AudioFormat::new(rate, 2).is_err());
    }
    for channels in [0, 3, 6, u16::MAX] {
        assert!(AudioFormat::new(48000, channels).is_err());
    }
    assert!(TestSignal::new(SignalKind::Sine, 48000, f64::NAN, -24.0).is_err());
    assert!(TestSignal::new(SignalKind::Sine, 48000, 440.0, f32::NAN).is_err());
}
#[test]
fn timestamp_respects_source_rate_and_arbitrary_periods() {
    for rate in [44100, 48000, 96000] {
        let stats = Arc::new(AudioStats::default());
        let (p, mut c) = block_queue(8, stats.clone()).unwrap();
        let mut bridge = CaptureBridge::new(42, 7, AudioFormat::new(rate, 2).unwrap(), p, stats);
        let mut expected = 0;
        for frames in [1, 127, 441, 480, 511, 1024, 2048] {
            bridge.ingest(frames, frames_to_ns(expected, rate), 1_000, |_| {
                [0.25, -0.25]
            });
            let mut received = 0;
            while let Some(b) = c.pop_fresh(1_001, 100_000_000) {
                assert_eq!(b.header.source_sample_position, expected);
                assert_eq!(b.header.stream_id, 42);
                assert_eq!(b.header.stream_epoch, 7);
                assert_eq!(b.header.format.sample_rate, rate);
                assert!(b.frames().iter().all(|&f| f == [0.25, -0.25]));
                expected += u64::from(b.header.frame_count);
                received += usize::from(b.header.frame_count);
            }
            assert_eq!(received, frames);
        }
    }
}
#[test]
fn overflow_and_age_never_replay_backlog() {
    let stats = Arc::new(AudioStats::default());
    let (mut p, mut c) = block_queue(2, stats.clone()).unwrap();
    let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
    b.header.frame_count = 480;
    b.header.arrival_ns = 10;
    assert!(p.push(b));
    assert!(p.push(b));
    assert!(!p.push(b));
    assert!(c.pop_fresh(10, 100).is_none());
    assert_eq!(stats.stale_frames.load(Relaxed), 960);
    b.header.source_sample_position = 1440;
    assert!(p.push(b));
    let fresh = c.pop_fresh(10, 100).unwrap();
    assert_eq!(fresh.header.source_sample_position, 1440);
    assert!(
        fresh
            .header
            .discontinuity_flags
            .contains(Discontinuity::GAP)
    );
    assert!(p.push(b));
    assert!(c.pop_fresh(111, 100).is_none());
}
#[test]
fn silence_is_data_but_idle_is_not() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(8, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats.clone());
    let mut monitor = ActivityMonitor::default();
    assert!(monitor.tick(&stats));
    bridge.ingest(128, 0, 10, |_| [0.0; 2]);
    stats.callback(128, 10);
    assert!(!monitor.tick(&stats));
    assert!(c.pop_fresh(10, 100).unwrap().is_silent());
    assert_eq!(stats.silent_frames.load(Relaxed), 128);
    assert!(monitor.tick(&stats));
    assert_eq!(stats.no_data_intervals.load(Relaxed), 2);
}
#[test]
fn reset_and_format_change_replace_epoch_and_flush() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(8, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, 8, AudioFormat::INTERNAL, p, stats);
    bridge.ingest(480, 0, 1, |_| [0.1; 2]);
    bridge.reset(
        AudioFormat::new(44100, 1).unwrap(),
        Discontinuity::FORMAT_CHANGED,
    );
    assert!(c.pop_fresh(1, 100).is_none());
    bridge.ingest(441, 5_000_000, 2, |_| [0.2; 2]);
    let b = c.pop_fresh(2, 100).unwrap();
    assert!(b.header.stream_epoch > 8);
    assert_eq!(b.header.source_sample_position, 0);
    assert_eq!(b.header.format.channel_layout, ChannelLayout::Mono);
    assert!(
        b.header
            .discontinuity_flags
            .contains(Discontinuity::FORMAT_CHANGED)
    );
}
#[test]
fn no_activity_gap_skips_time_and_invalidates_pending_audio() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(8, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats);
    bridge.ingest(480, 0, 0, |_| [0.1; 2]);
    assert!(c.pop_fresh(0, 1).is_some());
    bridge.ingest(480, 1_000_000_000, 1_000_000_000, |_| [0.2; 2]);
    let resumed = c.pop_fresh(1_000_000_000, 100).unwrap();
    assert_eq!(resumed.header.source_sample_position, 48000);
    assert!(
        resumed
            .header
            .discontinuity_flags
            .contains(Discontinuity::GAP)
    );
    bridge.ingest(480, 1_010_000_000, 1_010_000_000, |_| [0.3; 2]);
    assert_eq!(
        c.pop_fresh(1_010_000_000, 100)
            .unwrap()
            .header
            .source_sample_position,
        48480
    );
}
#[test]
fn mute_advances_signal_position() {
    let mut signal = TestSignal::new(SignalKind::Sine, 48000, 1000.0, -24.0).unwrap();
    signal.set_muted(true);
    for _ in 0..1001 {
        assert_eq!(signal.next_frame(), [0.0; 2]);
    }
    signal.set_muted(false);
    assert_eq!(signal.position(), 1001);
    assert_ne!(signal.next_frame(), [0.0; 2]);
}
#[test]
fn output_clock_rate_conversion_and_reopen() {
    assert_eq!(frames_at_rate(44100 * 3600, 44100, 48000), 48000 * 3600);
    let mut clock = OutputClock::new(44100, 3);
    for i in 0..100 {
        let p = clock.observe(i * 10_000_000, i * 10_000_000 + 20_000_000, 441);
        assert_eq!(p.submitted_frames, (i + 1) * 441);
    }
    let reset = clock.observe(0, 20_000_000, 127);
    assert!(reset.reset);
    assert!(reset.epoch > 3);
    assert_eq!(reset.presented_frames, 0);
    let p = OutputClock::new(48000, 5).observe(0, 0, 256);
    assert_eq!(p.submitted_frames, 256);
    assert_eq!(p.epoch, 5);
}
#[test]
fn resampling_preserves_tone_at_44100_48000_96000() {
    for (input, output) in [
        (44100, 48000),
        (48000, 44100),
        (96000, 48000),
        (48000, 96000),
        (48000, 48000),
    ] {
        let signal = TestSignal::new(SignalKind::Sine, input, 1000.0, -24.0).unwrap();
        let mut source = RateConverter::new(signal, input, output).unwrap();
        let mut count = 0;
        let mut previous = 0.0;
        let mut sum = 0.0;
        for i in 0..output * 2 {
            let frame = source.next_frame();
            assert!(frame[0].is_finite());
            assert_eq!(frame[0], frame[1]);
            if i > output / 2 {
                if previous <= 0.0 && frame[0] > 0.0 {
                    count += 1;
                }
                sum += frame[0] * frame[0];
            }
            previous = frame[0];
        }
        assert!((1498..=1502).contains(&count), "{input}->{output}: {count}");
        let rms = (sum / (output as f32 * 1.5)).sqrt();
        assert!((rms - 0.0446154).abs() < 0.001, "rms={rms}");
        assert!(!source.failed());
    }
    assert_eq!(stereo_to_mono([1.0, -1.0]), 0.0);
}
#[test]
fn malicious_pcm_is_sanitized() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(1, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats);
    bridge.ingest(1, 0, 0, |_| [f32::NAN, f32::INFINITY]);
    assert_eq!(c.pop_fresh(0, 1).unwrap().frames(), &[[0.0, 0.0]]);
}

#[test]
fn raw_device_clock_is_independent_of_submitted_audio_and_resets() {
    let mut clock = OutputClock::new(44100, 1);
    let first = clock.observe_native(0, 10_000_000, 441, Some(-441));
    assert_eq!(first.native_stream_frames, Some(0));
    let second = clock.observe_native(20_000_000, 30_000_000, 441, Some(441));
    // Device advanced 882 frames while application submitted only 441 since the first callback.
    assert_eq!(second.native_stream_frames, Some(882));
    assert_eq!(second.native_internal_frames, Some(960));
    assert_eq!(second.native_device_frame_position, Some(441));
    let reset = clock.observe_native(30_000_000, 40_000_000, 441, Some(0));
    assert!(reset.reset);
    assert!(reset.epoch > 1);
    assert_eq!(reset.native_stream_frames, Some(0));
    let missing = clock.observe_native(40_000_000, 50_000_000, 441, None);
    assert_eq!(missing.native_device_frame_position, None);
    assert_eq!(missing.native_internal_frames, None);
}

#[test]
fn resume_drops_old_generation_but_preserves_first_new_block() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(8, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats);
    bridge.ingest(127, 0, 0, |_| [0.1; 2]);
    bridge.reset(AudioFormat::INTERNAL, Discontinuity::RESUMED);
    bridge.ingest(127, 1_000_000_000, 1_000_000_000, |_| [0.2; 2]);
    let block = c.pop_fresh(1_000_000_000, 100_000_000).unwrap();
    assert!(block.header.stream_epoch > 1);
    assert_eq!(block.header.source_sample_position, 0);
    assert!(
        block
            .header
            .discontinuity_flags
            .contains(Discontinuity::RESUMED)
    );
    assert_eq!(block.frames()[0], [0.2; 2]);
}

#[test]
fn a_complete_large_native_callback_is_not_invalidated_by_its_own_tail() {
    for frames in [4096, 8192] {
        let stats = Arc::new(AudioStats::default());
        let (p, mut c) = block_queue(CAPTURE_QUEUE_BLOCKS, stats.clone()).unwrap();
        let mut bridge = CaptureBridge::new(1, 1, AudioFormat::INTERNAL, p, stats.clone());
        bridge.ingest(frames, 0, 0, |_| [0.25, -0.25]);
        let mut received = 0;
        while let Some(block) = c.pop_fresh(1, 100_000_000) {
            assert_eq!(block.header.source_sample_position, received);
            received += u64::from(block.header.frame_count);
        }
        assert_eq!(received, frames as u64);
        assert_eq!(stats.dropped_frames.load(Relaxed), 0);
        assert_eq!(stats.stale_frames.load(Relaxed), 0);
    }
}

#[test]
fn a_zero_length_callback_does_not_count_as_received_audio() {
    let stats = AudioStats::default();
    let mut activity = ActivityMonitor::default();
    stats.callback(0, 100);
    assert!(activity.tick(&stats));
    stats.callback(128, 100);
    assert!(!activity.tick(&stats));
}

#[test]
fn reopening_cannot_reuse_the_epoch_of_a_reset_clock() {
    let first = neonmix_core::epoch::reserve_epoch_after(0).unwrap();
    let mut clock = OutputClock::new(48000, first);
    clock.observe_native(0, 0, 480, Some(480));
    let reset = clock.observe_native(1, 1, 480, Some(0));
    assert!(reset.epoch > first);
    let reopened = neonmix_core::epoch::reserve_epoch_after(0).unwrap();
    assert!(reopened > reset.epoch);
    assert_eq!(
        OutputClock::new(48000, reopened)
            .observe(0, 0, 127)
            .presented_frames,
        0
    );
}

#[test]
fn epoch_exhaustion_stops_capture_instead_of_replaying_an_identity() {
    let stats = Arc::new(AudioStats::default());
    let (p, mut c) = block_queue(2, stats.clone()).unwrap();
    let mut bridge = CaptureBridge::new(1, u64::MAX, AudioFormat::INTERNAL, p, stats.clone());
    bridge.reset(AudioFormat::INTERNAL, Discontinuity::RESUMED);
    bridge.ingest(127, 0, 0, |_| [0.5; 2]);
    assert!(c.pop_fresh(0, 1).is_none());
    assert_eq!(stats.last_error.load(Relaxed), 10);
    let mut output = OutputClock::new(48000, u64::MAX);
    output.observe_native(0, 0, 480, Some(480));
    assert!(output.observe_native(1, 1, 480, Some(0)).epoch_exhausted);
}

#[test]
fn channel_test_patterns_expose_mono_mapping_errors() {
    for (pattern, expected) in [
        (ChannelPattern::Left, [1.0, 0.0]),
        (ChannelPattern::Right, [0.0, 1.0]),
        (ChannelPattern::AntiPhase, [1.0, -1.0]),
    ] {
        let mut source = TestSignal::new(SignalKind::Impulse, 48000, 440.0, 0.0)
            .unwrap()
            .with_pattern(pattern);
        let frame = source.next_frame();
        assert_eq!(frame, expected);
        assert_eq!(stereo_to_mono(frame), (expected[0] + expected[1]) / 2.0);
    }
}

#[test]
fn callback_budget_excess_is_distinct_from_a_native_xrun() {
    let stats = AudioStats::default();
    stats.callback_at_rate(480, 48000, 9_000_000);
    stats.callback_at_rate(480, 48000, 11_000_000);
    assert_eq!(stats.snapshot().callback_over_budget, 1);
    assert_eq!(stats.snapshot().errors, 0);
}
