use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig},
    signal::StereoSource,
};
use std::{
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};
fn packet(epoch: u64, target: u64, value: f32) -> AudioBlock {
    let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
    b.header.stream_epoch = epoch;
    b.header.frame_count = 480;
    b.header.arrival_ns = u64::MAX;
    b.header.presentation_time_ns = Some(target);
    b.header.discontinuity_flags = Discontinuity::NONE;
    b.pcm.fill([value; 2]);
    b
}
#[test]
fn single_block_waits_until_target_without_native_start_water() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.2));
    mixer.set_presentation_time(origin + Duration::from_millis(930));
    let mut before = [[0.; 2]; 480];
    mixer.render_block(&mut before);
    assert!(before.iter().all(|f| *f == [0.; 2]));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    let mut at = [[0.; 2]; 480];
    mixer.render_block(&mut at);
    assert!(at[0][0] > 0. && at[479][0] > 0.19);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}
#[test]
fn callback_mid_block_selects_source_offset_and_epoch_flush_rejects_stale() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 2,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.8));
    let mut b = packet(2, 1_000_000_000, 0.1);
    b.pcm[240] = [0.8; 2];
    inputs[0].push(b);
    mixer.set_presentation_time(origin + Duration::from_millis(1005));
    let first = mixer.next_frame();
    let second = mixer.next_frame();
    assert!(first[0] > second[0]);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 1);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 240);
    assert_eq!(stats.timed_late_frames_by_lane[0].load(Relaxed), 240);
    assert_eq!(stats.last_timed_late_stream_id[0].load(Relaxed), 1);
    assert_eq!(stats.last_timed_late_epoch[0].load(Relaxed), 2);
    assert_eq!(stats.last_timed_late_output_frame[0].load(Relaxed), 0);
    assert_eq!(
        stats.last_timed_late_target_ns[0].load(Relaxed),
        1_000_000_000
    );
    assert_eq!(
        stats.last_timed_late_presentation_ns[0].load(Relaxed),
        1_005_000_000
    );
}

#[test]
fn late_deadlines_are_attributed_to_stream_epochs_across_lane_reuse() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 2,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    config.lanes[1] = LaneMix {
        stream_id: 2,
        epoch: 5,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(2, 1_000_000_000, 0.1));
    let mut survivor = packet(5, 1_005_000_000, 0.2);
    survivor.header.stream_id = 2;
    inputs[1].push(survivor);
    let mut frames = [[0.; 2]; 240];
    mixer.set_presentation_time(origin + Duration::from_millis(1005));
    mixer.render_block(&mut frames);
    mixer.set_presentation_time(origin + Duration::from_millis(1010));
    mixer.render_block(&mut frames);

    // Reuse the physical lane with an unrelated stream and an older PTS.
    // Its lifetime count accumulates, while the last event identifies the
    // current stream/epoch; the continuously fed survivor stays unchanged.
    config.lanes[0].stream_id = 3;
    config.lanes[0].epoch = 7;
    control.apply(config).unwrap();
    let mut reused = packet(7, 1_010_000_000, 0.1);
    reused.header.stream_id = 3;
    inputs[0].push(reused);
    let mut survivor = packet(5, 1_015_000_000, 0.2);
    survivor.header.stream_id = 2;
    survivor.header.source_sample_position = 480;
    inputs[1].push(survivor);
    mixer.set_presentation_time(origin + Duration::from_millis(1015));
    mixer.render_block(&mut frames);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 480);
    assert_eq!(stats.timed_late_frames_by_lane[0].load(Relaxed), 480);
    assert_eq!(stats.timed_late_frames_by_lane[1].load(Relaxed), 0);
    assert_eq!(stats.underrun_frames_by_lane[1].load(Relaxed), 0);
    assert_eq!(stats.last_timed_late_stream_id[0].load(Relaxed), 3);
    assert_eq!(stats.last_timed_late_epoch[0].load(Relaxed), 7);
    assert_eq!(stats.last_timed_late_output_frame[0].load(Relaxed), 480);
    assert_eq!(
        stats.last_timed_late_target_ns[0].load(Relaxed),
        1_010_000_000
    );
    assert_eq!(
        stats.last_timed_late_presentation_ns[0].load(Relaxed),
        1_015_000_000
    );
}
#[test]
fn muted_and_solo_lanes_consume_elapsed_audio_without_replay() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        muted: true,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.3));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    let mut frames = [[0.; 2]; 480];
    mixer.render_block(&mut frames);
    assert!(frames.iter().all(|f| *f == [0.; 2]));
    config.lanes[0].muted = false;
    control.apply(config).unwrap();
    mixer.set_presentation_time(origin + Duration::from_millis(1010));
    mixer.render_block(&mut frames);
    assert!(frames.iter().all(|f| *f == [0.; 2]));
}

#[test]
fn new_epoch_flushes_a_future_head_already_held_by_output() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.9));
    mixer.set_presentation_time(origin + Duration::from_millis(930));
    let mut frames = [[0.; 2]; 480];
    mixer.render_block(&mut frames);
    config.lanes[0].epoch = 2;
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.9));
    inputs[0].push(packet(2, 1_000_000_000, 0.1));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut frames);
    assert!(frames[479][0] > 0.09 && frames[479][0] < 0.11);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 1);
}
#[test]
fn solo_exclusion_consumes_timed_input_instead_of_replaying_it_later() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    config.lanes[1] = LaneMix {
        stream_id: 2,
        epoch: 1,
        solo: true,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(packet(1, 1_000_000_000, 0.9));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    let mut frames = [[0.; 2]; 480];
    mixer.render_block(&mut frames);
    assert!(frames.iter().all(|f| *f == [0.; 2]));
    config.lanes[1].solo = false;
    control.apply(config).unwrap();
    mixer.set_presentation_time(origin + Duration::from_millis(1010));
    mixer.render_block(&mut frames);
    assert!(frames.iter().all(|f| *f == [0.; 2]));
}

#[test]
fn timed_audio_recovers_after_a_track_gap_without_an_epoch_change() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let mut frames = [[0.; 2]; 480];
    inputs[0].push(packet(1, 1_000_000_000, 0.2));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut frames);
    assert!(frames[479][0] > 0.19);
    assert_eq!(stats.underrun_frames.load(Relaxed), 0);
    // The sender advances RTP and PTS during a gap, but ingress numbers only
    // decoded PCM frames. No FLUSH/SETUP or new stream epoch is required.
    for tick in 1..200 {
        mixer.set_presentation_time(origin + Duration::from_millis(1000 + tick * 10));
        mixer.render_block(&mut frames);
    }
    assert_eq!(stats.underrun_frames.load(Relaxed), 199 * 480);
    assert_eq!(stats.underrun_frames_by_lane[0].load(Relaxed), 199 * 480);
    assert_eq!(stats.underrun_frames_by_lane[1].load(Relaxed), 0);
    for tick in 0..10 {
        let mut next = packet(1, 3_000_000_000 + tick * 10_000_000, 0.2);
        next.header.source_sample_position = 480 + tick * 480;
        inputs[0].push(next);
        mixer.set_presentation_time(origin + Duration::from_millis(3000 + tick * 10));
        mixer.render_block(&mut frames);
        assert!(
            frames[479][0] > 0.19,
            "next track stayed silent at block {tick}"
        );
    }
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}

#[test]
fn starvation_recovery_waits_for_future_pts_and_skips_elapsed_audio() {
    for late in [false, true] {
        let origin = Instant::now();
        let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
        let mut config = MixerConfig {
            master_db: 0.,
            ..MixerConfig::default()
        };
        config.lanes[0] = LaneMix {
            stream_id: 1,
            epoch: 1,
            playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
            ..LaneMix::default()
        };
        control.apply(config).unwrap();
        let mut frames = [[0.; 2]; 480];
        inputs[0].push(packet(1, 1_000_000_000, 0.2));
        mixer.set_presentation_time(origin + Duration::from_secs(1));
        mixer.render_block(&mut frames);
        for tick in 1..200 {
            mixer.set_presentation_time(origin + Duration::from_millis(1000 + tick * 10));
            mixer.render_block(&mut frames);
        }
        let mut next = packet(1, if late { 2_995_000_000 } else { 3_050_000_000 }, 0.3);
        next.header.source_sample_position = 480;
        // Poison elapsed samples: they must never be replayed on recovery.
        if late {
            next.pcm[..240].fill([0.9; 2]);
        }
        inputs[0].push(next);
        mixer.set_presentation_time(origin + Duration::from_secs(3));
        mixer.render_block(&mut frames);
        if late {
            assert!(frames[239][0] > 0.29 && frames[239][0] < 0.31);
            assert!(frames.iter().all(|f| f[0] < 0.31));
            assert!(frames[300..].iter().all(|f| *f == [0.; 2]));
            assert_eq!(stats.timed_late_frames.load(Relaxed), 240);
        } else {
            assert!(frames.iter().all(|f| *f == [0.; 2]));
            mixer.set_presentation_time(origin + Duration::from_millis(3050));
            mixer.render_block(&mut frames);
            assert!(frames[479][0] > 0.29);
            assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
        }
    }
}

#[test]
fn an_early_first_packet_keeps_its_samples_across_callback_start_jitter() {
    // Windows stream14/epoch4 started 116,100 ns after its first PTS and
    // skipped five frames. Reproduce that boundary with PCM already waiting.
    let target = 42_026_092_734;
    for lifecycle in ["new", "epoch", "gap"] {
        for period in [256, 480, 512, 1056] {
            let render = |jitter_ns: u64| {
                let origin = Instant::now();
                let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
                let mut config = MixerConfig {
                    master_db: 0.,
                    ..MixerConfig::default()
                };
                config.lanes[1] = LaneMix {
                    stream_id: 14,
                    epoch: 4,
                    playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
                    ..LaneMix::default()
                };
                if lifecycle != "new" {
                    config.lanes[1].epoch = if lifecycle == "epoch" { 3 } else { 4 };
                    control.apply(config).unwrap();
                    let mut prior = packet(config.lanes[1].epoch, 40_000_000_000, 0.1);
                    prior.header.stream_id = 14;
                    assert!(inputs[1].push(prior));
                    mixer.set_presentation_time(origin + Duration::from_secs(40));
                    mixer.render_block(&mut [[0.; 2]; 480]);
                    if lifecycle == "gap" {
                        mixer.set_presentation_time(origin + Duration::from_millis(40_010));
                        mixer.render_block(&mut [[0.; 2]; 480]);
                    }
                    config.lanes[1].epoch = 4;
                }
                control.apply(config).unwrap();
                let mut block = packet(4, target, 0.1);
                block.header.stream_id = 14;
                if lifecycle == "gap" {
                    block.header.source_sample_position = 480;
                }
                // A distinctive prefix makes skipped PCM observable in the output,
                // independently of whether the diagnostic counter reports it.
                block.pcm[..8].fill([0.8; 2]);
                assert!(inputs[1].push(block));
                let next_at = target - 8_900;
                let first_at = next_at - neonmix_core::clock::frames_to_ns(period, 48_000);
                mixer.set_presentation_time(origin + Duration::from_nanos(first_at));
                let mut before = vec![[0.; 2]; period as usize];
                mixer.render_block(&mut before);
                assert!(before.iter().all(|f| *f == [0.; 2]));
                mixer.set_presentation_time(origin + Duration::from_nanos(next_at + jitter_ns));
                let mut output = [[0.; 2]; 64];
                mixer.render_block(&mut output);
                assert_eq!(stats.rejected_blocks.load(Relaxed), 0);
                assert!(output[7][0] > 0.01, "distinctive PCM prefix stayed silent");
                assert!(output[63][0] > 0.01, "recovered PCM stayed silent");
                (output, stats.timed_late_frames.load(Relaxed))
            };
            let (reference, _) = render(0);
            for jitter in [125_000, 1_000_000] {
                let (jittered, late) = render(jitter);
                println!(
                    "lifecycle={lifecycle}, period={period}, jitter_ns={jitter}, skipped={late}"
                );
                assert_eq!(late, 0, "early PCM skipped at callback period {period}");
                for (actual, expected) in jittered.iter().zip(reference) {
                    assert!(
                        (actual[0] - expected[0]).abs() < 0.00001,
                        "callback jitter changed the initial PCM: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn negative_callback_start_jitter_does_not_delay_buffered_pcm() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    assert!(inputs[0].push(packet(1, 1_000_000_000, 0.2)));
    mixer.set_presentation_time(origin + Duration::from_millis(990));
    mixer.render_block(&mut [[0.; 2]; 480]);
    mixer.set_presentation_time(origin + Duration::from_nanos(999_883_900));
    assert!(mixer.next_frame()[0] > 0., "buffered onset moved backwards");
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}

#[test]
fn genuinely_late_first_pcm_and_large_clock_jumps_still_skip_elapsed_samples() {
    for (early, jump, expected) in [(false, 116_100, 5), (true, 2_000_000, 96)] {
        let origin = Instant::now();
        let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
        let mut config = MixerConfig {
            master_db: 0.,
            ..MixerConfig::default()
        };
        config.lanes[0] = LaneMix {
            stream_id: 1,
            epoch: 1,
            playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
            ..LaneMix::default()
        };
        control.apply(config).unwrap();
        let mut block = packet(1, 1_000_000_000, 0.1);
        block.pcm[..expected].fill([0.8; 2]);
        if early {
            assert!(inputs[0].push(block));
            mixer.set_presentation_time(origin + Duration::from_millis(990));
            mixer.render_block(&mut [[0.; 2]; 480]);
        } else {
            assert!(inputs[0].push(block));
        }
        mixer.set_presentation_time(origin + Duration::from_nanos(1_000_000_000 + jump));
        let sample = mixer.next_frame();
        assert_eq!(stats.timed_late_frames.load(Relaxed), expected as u64);
        assert_eq!(
            stats.timed_late_frames_by_lane[0].load(Relaxed),
            expected as u64
        );
        assert!(sample[0] < 0.001, "elapsed prefix was replayed: {sample:?}");
    }
}
