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
            mixer.render_block(&mut rendered);
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
