use neonmix_airplay_adapter::{Context, Ingress, IngressError};
use neonmix_airplay_ipc::{PcmHeader, PcmPacket};
use neonmix_core::{queue::block_queue, stats::AudioStats};
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn context() -> Context {
    Context {
        session_id: 1,
        stream_id: 2,
        stream_epoch: 3,
        format_epoch: 4,
        mapping_id: 5,
    }
}
fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_000_000)
}

#[test]
fn low_latency_removes_protocol_prefetch_without_modulating_packet_timing() {
    use neonmix_airplay_adapter::{control::PlaybackMode, ingress::LOW_LATENCY_HEADROOM_NS};
    let now = Instant::now();
    let mut ingress = Ingress::new_at(now, context(), now, wall());
    ingress.set_playback_mode(PlaybackMode::LowLatency);
    // Simulate the classic sender's two-second lead, with burst arrival and jitter.
    for seq in 0..30 {
        let arrival = Duration::from_millis(seq * 10 + (seq % 3));
        ingress
            .push_at(
                packet(seq, 2_000_000_000 + seq * 10_000_000),
                now + arrival,
                wall() + arrival,
            )
            .unwrap();
        assert_eq!(ingress.stats().latency_advance_ns, 1_880_000_000);
    }
    let (mut producer, mut consumer) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
    for seq in 0..30 {
        let release_ns = 60_000_000 + seq * 10_000_000;
        assert_eq!(
            ingress.release_due(now + Duration::from_nanos(release_ns), &mut producer),
            1
        );
        let block = consumer.pop_fresh(release_ns, 80_000_000).unwrap();
        assert_eq!(
            block.header.presentation_time_ns,
            Some(LOW_LATENCY_HEADROOM_NS + seq * 10_000_000)
        );
        assert_eq!(block.pcm[0], [0.25; 2]);
    }
    assert_eq!(ingress.stats().late_packets, 0);
    assert_eq!(ingress.stats().playout_lead_ns, 118_000_000);
    ingress.clear();
    ingress
        .push_at(packet(31, 500_000_000), now, wall())
        .unwrap();
    assert_eq!(ingress.stats().latency_advance_ns, 380_000_000);
    assert_eq!(ingress.stats().playout_lead_ns, 120_000_000);
}

#[test]
fn low_latency_keeps_bounds_rejects_late_packets_and_preserves_short_deadlines() {
    use neonmix_airplay_adapter::control::PlaybackMode;
    let now = Instant::now();
    let mut ingress = Ingress::new_at(now, context(), now, wall());
    ingress.set_playback_mode(PlaybackMode::LowLatency);
    assert_eq!(
        ingress.push_at(packet(0, 5_000_000_000), now, wall()),
        Err(IngressError::TooFar)
    );
    ingress.push_at(packet(1, 50_000_000), now, wall()).unwrap();
    assert_eq!(ingress.stats().latency_advance_ns, 0);
    assert_eq!(ingress.stats().playout_lead_ns, 50_000_000);
    ingress.clear();
    ingress
        .push_at(packet(0, 2_000_000_000), now, wall())
        .unwrap();
    let late = Duration::from_millis(200);
    assert_eq!(
        ingress.push_at(packet(1, 2_010_000_000), now + late, wall() + late),
        Err(IngressError::Late)
    );
    assert_eq!(ingress.stats().latency_advance_ns, 1_880_000_000);
    ingress.set_playback_mode(PlaybackMode::Synchronized);
    ingress
        .push_at(packet(0, 2_000_000_000), now, wall())
        .unwrap();
    assert_eq!(ingress.stats().last_presentation_time_ns, 2_000_000_000);
    assert_eq!(ingress.stats().latency_advance_ns, 0);
}
fn packet(seq: u64, target_ns: u64) -> PcmPacket {
    PcmPacket {
        header: PcmHeader {
            flags: 0,
            session_id: 1,
            stream_id: 2,
            stream_epoch: 3,
            format_epoch: 4,
            sequence: seq,
            source_sample_position: seq * 441,
            normalized_sample_position: seq * 480,
            presentation_time_ns: 1_000_000_000_000_000 + target_ns,
            mapping_id: 5,
            uncertainty_ns: 1000,
            source_rate: 44100,
            frame_count: 480,
            protocol_gain: 0.5,
            gain_applied: false,
        },
        samples: vec![0.5; 960],
    }
}
#[test]
fn unix_mapping_preserves_source_domain_and_release_obeys_device_budget() {
    let now = Instant::now();
    let mut i = Ingress::new_at(now, context(), now, wall());
    i.push_at(packet(1, 2_000_000_000), now, wall()).unwrap();
    let (mut p, mut c) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
    assert_eq!(i.release_due(now + Duration::from_millis(1900), &mut p), 0);
    i.set_output_latency_ns(40_000_000);
    assert_eq!(i.release_due(now + Duration::from_millis(1900), &mut p), 1);
    let block = c.pop_fresh(1_900_000_000, 80_000_000).unwrap();
    assert_eq!(block.header.presentation_time_ns, Some(2_000_000_000));
    assert_eq!(block.header.arrival_ns, 1_900_000_000);
    assert_eq!(block.pcm[0], [0.25; 2]);
    assert_eq!(i.stats().last_source_rate, 44100);
    assert_eq!(i.stats().last_source_sample_position, 441);
}
#[test]
fn old_epoch_duplicates_clock_jump_and_bad_payload_fail_closed() {
    let now = Instant::now();
    let mut i = Ingress::new_at(now, context(), now, wall());
    let mut old = packet(1, 1_000_000_000);
    old.header.stream_epoch = 2;
    assert_eq!(i.push_at(old, now, wall()), Err(IngressError::Identity));
    i.push_at(packet(1, 1_000_000_000), now, wall()).unwrap();
    assert_eq!(
        i.push_at(packet(1, 1_000_000_000), now, wall()),
        Err(IngressError::Timeline)
    );
    let mut bad = packet(2, 1_010_000_000);
    bad.samples[0] = f32::NAN;
    assert_eq!(i.push_at(bad, now, wall()), Err(IngressError::Malformed));
    assert_eq!(
        i.push_at(
            packet(2, 1_010_000_000),
            now,
            wall() + Duration::from_millis(30)
        ),
        Err(IngressError::ClockReset)
    );
    assert_eq!(i.stats().pending_frames, 0);
    assert_eq!(
        i.push_at(packet(2, 1_010_000_000), now, wall()),
        Err(IngressError::ClockReset)
    );
    let mut new_context = context();
    new_context.stream_epoch += 1;
    new_context.mapping_id += 1;
    i.reset_at(new_context, now, wall());
    assert_eq!(
        i.push_at(packet(2, 1_010_000_000), now, wall()),
        Err(IngressError::Identity)
    );
}
#[test]
fn four_second_and_byte_limits_hold_for_large_and_tiny_packets() {
    let now = Instant::now();
    let mut i = Ingress::new_at(now, context(), now, wall());
    assert_eq!(
        i.push_at(packet(1, 4_000_000_000), now, wall()),
        Err(IngressError::TooFar)
    );
    for seq in 0..400 {
        i.push_at(packet(seq, seq * 10_000_000), now, wall())
            .unwrap();
    }
    assert_eq!(i.stats().pending_frames, 192000);
    assert!(i.stats().pending_bytes <= 2 * 1024 * 1024);
    assert_eq!(
        i.push_at(packet(400, 3_995_000_000), now, wall()),
        Err(IngressError::TooFar)
    );
    i.clear();
    let mut rejected = false;
    for seq in 1..2000 {
        let mut b = packet(seq, 1_000_000_000 + seq * 21_000);
        b.header.normalized_sample_position = seq;
        b.header.source_sample_position = seq * 44100 / 48000;
        b.header.frame_count = 1;
        b.samples.truncate(2);
        if i.push_at(b, now, wall()) == Err(IngressError::Full) {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert!(i.stats().pending_bytes <= 2 * 1024 * 1024);
}
#[test]
fn full_short_queue_never_invalidates_already_released_pcm() {
    let now = Instant::now();
    let mut i = Ingress::new_at(now, context(), now, wall());
    let (mut p, mut c) = block_queue(2, Arc::new(AudioStats::default())).unwrap();
    for seq in 1..=3 {
        i.push_at(packet(seq, seq * 10_000_000), now, wall())
            .unwrap();
    }
    assert_eq!(i.release_due(now, &mut p), 2);
    assert_eq!(i.stats().pending_frames, 480);
    assert!(c.pop_fresh(0, 80_000_000).is_some());
    assert_eq!(i.release_due(now, &mut p), 1);
    assert!(c.pop_fresh(0, 80_000_000).is_some());
    assert!(c.pop_fresh(0, 80_000_000).is_some());
}
#[test]
fn already_applied_protocol_gain_is_not_multiplied_twice_and_late_packet_drops() {
    let now = Instant::now();
    let mut i = Ingress::new_at(now, context(), now, wall());
    let mut b = packet(1, 10_000_000);
    b.header.gain_applied = true;
    i.push_at(b, now, wall()).unwrap();
    let (mut p, mut c) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
    assert_eq!(i.release_due(now, &mut p), 1);
    assert_eq!(c.pop_fresh(0, 80_000_000).unwrap().pcm[0], [0.5; 2]);
    assert_eq!(
        i.push_at(
            packet(2, 0),
            now + Duration::from_millis(20),
            wall() + Duration::from_millis(20)
        ),
        Err(IngressError::Late)
    );
}

#[test]
fn low_latency_preserves_pcm_through_deep_device_callbacks() {
    use neonmix_airplay_adapter::control::PlaybackMode;
    use neonmix_core::{
        clock::frames_to_ns,
        mixer::{LaneMix, Mixer, MixerConfig},
        signal::StereoSource,
    };
    use std::sync::atomic::Ordering::Relaxed;
    // Ubuntu's report: 1024-frame callbacks and 106.645833 ms playback lead.
    // Include shallower macOS/Windows and larger callback shapes as controls.
    for (period, latency_ns) in [
        (256usize, 10_000_000u64),
        (1024, 106_645_833),
        (2048, 160_000_000),
    ] {
        let origin = Instant::now();
        let mut ingress = Ingress::new_at(origin, context(), origin, wall());
        ingress.set_playback_mode(PlaybackMode::LowLatency);
        ingress.set_output_latency_ns(latency_ns);
        let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
        let mut config = MixerConfig {
            master_db: 0.,
            ..MixerConfig::default()
        };
        config.lanes[0] = LaneMix {
            stream_id: 2,
            epoch: 3,
            playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
            ..LaneMix::default()
        };
        control.apply(config).unwrap();
        let mut rendered = vec![[0.; 2]; period];
        let mut sequence = 0;
        let mut bad_samples = 0;
        for output in (0..48000 * 3).step_by(period) {
            let now_ns = frames_to_ns(output as u64, 48000);
            // All source packets produced by this callback's wall time have
            // arrived. No artificial network loss or CPU scheduling stall.
            while sequence * 10_000_000 <= now_ns {
                let arrival = Duration::from_nanos(sequence * 10_000_000);
                ingress
                    .push_at(
                        packet(sequence, 2_000_000_000 + sequence * 10_000_000),
                        origin + arrival,
                        wall() + arrival,
                    )
                    .unwrap();
                sequence += 1;
            }
            ingress.release_due(origin + Duration::from_nanos(now_ns), &mut inputs[0]);
            mixer.set_presentation_time(origin + Duration::from_nanos(now_ns + latency_ns));
            mixer.render_block_at(origin + Duration::from_nanos(now_ns), &mut rendered);
            if output > 24000 {
                bad_samples += rendered
                    .iter()
                    .filter(|f| (f[0] - 0.25).abs() > 0.001)
                    .count();
            }
        }
        println!(
            "period={period} latency_ns={latency_ns} skipped={} bad_samples={bad_samples}",
            stats.timed_late_frames.load(Relaxed)
        );
        assert_eq!(ingress.stats().late_packets, 0);
        assert_eq!(
            stats.timed_late_frames.load(Relaxed),
            0,
            "device period={period}, latency={latency_ns}"
        );
        assert_eq!(bad_samples, 0, "steady PCM must be continuous");
        assert_eq!(stats.underrun_frames.load(Relaxed), 0);
    }
}

#[test]
fn prefetched_gap_keeps_the_first_tail_and_the_second_deadline() {
    use neonmix_core::{
        mixer::{LaneMix, Mixer, MixerConfig, PlaybackKind},
        signal::StereoSource,
    };
    for period in [1usize, 127, 480, 1024] {
        let origin = Instant::now();
        let mut ingress = Ingress::new_at(origin, context(), origin, wall());
        let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
        let mut config = MixerConfig {
            master_db: 0.,
            ..Default::default()
        };
        config.lanes[0] = LaneMix {
            stream_id: 2,
            epoch: 3,
            playback_kind: PlaybackKind::Timed,
            ..Default::default()
        };
        control.apply(config).unwrap();
        let mut first = packet(0, 100_000_000);
        first.samples.fill(0.4);
        let mut second = packet(2, 120_000_000);
        second.samples.fill(-0.6);
        ingress.push_at(first, origin, wall()).unwrap();
        ingress.push_at(second, origin, wall()).unwrap();
        // Both descriptors reach the real Mixer before A's first sample.
        assert_eq!(
            ingress.release_due(origin + Duration::from_millis(60), &mut inputs[0]),
            2
        );
        let mut rendered = Vec::new();
        for frame in (0..1920).step_by(period) {
            let count = period.min(1920 - frame);
            let time = Duration::from_nanos(
                90_000_000 + neonmix_core::clock::frames_to_ns(frame as u64, 48000),
            );
            mixer.set_presentation_time(origin + time);
            let mut output = vec![[0.; 2]; count];
            mixer.render_block_at(origin + Duration::from_millis(60), &mut output);
            rendered.extend(output);
        }
        assert!(
            rendered[800..950]
                .iter()
                .all(|f| (f[0] - 0.2).abs() < 0.001),
            "period={period}: A tail lost during prefetch"
        );
        assert!(
            rendered[..1440].iter().all(|f| f[0] >= -0.00001),
            "period={period}: B appeared before 120ms"
        );
        assert!(
            rendered[960..1440].iter().all(|f| f[0].abs() < 0.00001),
            "gap must stay silent"
        );
        assert!(rendered[1800..].iter().all(|f| (f[0] + 0.3).abs() < 0.001));
        assert_eq!(
            stats
                .timed_late_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }
}

#[test]
fn normalized_coordinates_keep_gaps_after_rejection_and_check_pts_and_source() {
    let now = Instant::now();
    let mut ingress = Ingress::new_at(now, context(), now, wall());
    ingress
        .push_at(packet(0, 100_000_000), now, wall())
        .unwrap();
    let mut bad = packet(1, 110_000_000);
    bad.samples[0] = f32::NAN;
    assert_eq!(
        ingress.push_at(bad, now, wall()),
        Err(IngressError::Malformed)
    );
    ingress
        .push_at(packet(2, 120_000_000), now, wall())
        .unwrap();
    assert_eq!(ingress.stats().source_gap_frames, 480);
    assert_eq!(ingress.stats().last_normalized_sample_position, 960);
    let mut conflicting = packet(3, 140_000_000);
    assert_eq!(
        ingress.push_at(conflicting.clone(), now, wall()),
        Err(IngressError::Timeline)
    );
    assert_eq!(ingress.stats().pts_rejections, 1);
    conflicting.header.presentation_time_ns = 1_000_000_130_000_000;
    conflicting.header.source_sample_position += 441;
    assert_eq!(
        ingress.push_at(conflicting, now, wall()),
        Err(IngressError::Timeline)
    );
    assert_eq!(ingress.stats().coordinate_rejections, 1);
    let mut reversed = packet(3, 130_000_000);
    reversed.header.normalized_sample_position = 959;
    assert_eq!(
        ingress.push_at(reversed, now, wall()),
        Err(IngressError::Timeline)
    );
    // Rejections leave the fixed anchor and accepted tail untouched.
    ingress
        .push_at(packet(3, 130_000_000), now, wall())
        .unwrap();
    let (mut producer, mut consumer) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
    assert_eq!(
        ingress.release_due(now + Duration::from_millis(80), &mut producer),
        3
    );
    for expected in [0, 960, 1440] {
        assert_eq!(
            consumer
                .pop_fresh(80_000_000, 80_000_000)
                .unwrap()
                .header
                .source_sample_position,
            expected
        );
    }
    let mut replacement = context();
    replacement.mapping_id += 1;
    replacement.stream_epoch += 1;
    ingress.reset_at(replacement, now, wall());
    let mut fresh = packet(0, 100_000_000);
    fresh.header.mapping_id += 1;
    fresh.header.stream_epoch += 1;
    ingress.push_at(fresh, now, wall()).unwrap();
    assert_eq!(ingress.stats().last_normalized_sample_position, 0);
}

#[test]
fn variable_src_chunks_preserve_the_same_media_grid_without_rounding_drift() {
    for rate in [44_100, 48_000] {
        let now = Instant::now();
        let mut ingress = Ingress::new_at(now, context(), now, wall());
        let (mut producer, mut consumer) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
        let mut position = 0u64;
        for sequence in 0..2000 {
            let frames = [1usize, 127, 352, 480, 17][sequence as usize % 5];
            let mut media = packet(
                sequence,
                100_000_000 + neonmix_core::clock::frames_to_ns(position, 48_000),
            );
            media.header.source_rate = rate;
            media.header.source_sample_position =
                4_294_967_040 + position * u64::from(rate) / 48_000;
            media.header.normalized_sample_position = position;
            media.header.frame_count = frames as u16;
            media.samples.resize(frames * 2, 0.25);
            let pts = media.header.presentation_time_ns - 1_000_000_000_000_000;
            let arrival = Duration::from_nanos(pts - 40_000_000);
            ingress
                .push_at(media, now + arrival, wall() + arrival)
                .unwrap();
            assert_eq!(ingress.release_due(now + arrival, &mut producer), 1);
            let delivered = consumer.pop_fresh(pts - 40_000_000, 80_000_000).unwrap();
            assert_eq!(delivered.header.source_sample_position, position);
            assert_eq!(delivered.header.presentation_time_ns, Some(pts));
            position += frames as u64;
        }
        assert_eq!(ingress.stats().source_gap_frames, 0);
        assert_eq!(ingress.stats().coordinate_rejections, 0);
        assert_eq!(ingress.stats().pts_rejections, 0);
    }
}
