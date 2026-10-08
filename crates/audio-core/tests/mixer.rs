use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{DriftController, LaneMix, Mixer, MixerConfig},
    signal::StereoSource,
};
use std::{sync::atomic::Ordering::Relaxed, time::Instant};
fn block(id: u64, epoch: u64, position: u64, value: [f32; 2]) -> AudioBlock {
    let mut b = AudioBlock::empty(id, AudioFormat::INTERNAL);
    b.header.stream_epoch = epoch;
    b.header.source_sample_position = position;
    b.header.frame_count = 480;
    b.header.discontinuity_flags = if position == 0 {
        Discontinuity::START
    } else {
        Discontinuity::NONE
    };
    // Synthetic clock: disable wall-time eviction during accelerated DSP tests.
    b.header.arrival_ns = u64::MAX;
    b.pcm.fill(value);
    b
}
#[test]
fn gain_mute_multisolo_advance_all_lanes_and_limiter_reports_overload() {
    let (mut m, mut c, mut p, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    config.lanes[1] = LaneMix {
        stream_id: 2,
        epoch: 1,
        ..LaneMix::default()
    };
    c.apply(config).unwrap();
    let mut frames = [[0.0; 2]; 480];
    let mut position = 480;
    p[0].push(block(1, 1, 0, [0.2, 0.0]));
    p[1].push(block(2, 1, 0, [0.0, 0.3]));
    for _ in 0..2 {
        p[0].push(block(1, 1, position, [0.2, 0.0]));
        p[1].push(block(2, 1, position, [0.0, 0.3]));
        position += 480;
    }
    let mut render = |m: &mut Mixer, p: &mut Vec<neonmix_core::queue::BlockProducer>| {
        for _ in 0..2 {
            p[0].push(block(1, 1, position, [0.2, 0.0]));
            p[1].push(block(2, 1, position, [0.0, 0.3]));
            position += 480;
        }
        m.render_block(&mut frames);
        m.render_block(&mut frames);
        frames[479]
    };
    for _ in 0..4 {
        render(&mut m, &mut p);
    }
    let f = render(&mut m, &mut p);
    assert!((f[0] - 0.2).abs() < 0.001 && (f[1] - 0.3).abs() < 0.001);
    config.lanes[0].gain_db = -6.0206;
    c.apply(config).unwrap();
    let f = render(&mut m, &mut p);
    assert!((f[0] - 0.1).abs() < 0.001 && (f[1] - 0.3).abs() < 0.001);
    config.lanes[0].muted = true;
    c.apply(config).unwrap();
    let f = render(&mut m, &mut p);
    assert!(f[0].abs() < 0.001 && (f[1] - 0.3).abs() < 0.001);
    config.lanes[0].muted = false;
    config.lanes[0].solo = true;
    c.apply(config).unwrap();
    let f = render(&mut m, &mut p);
    assert!((f[0] - 0.1).abs() < 0.001 && f[1].abs() < 0.001);
    config.lanes[1].solo = true;
    c.apply(config).unwrap();
    let f = render(&mut m, &mut p);
    assert!((f[1] - 0.3).abs() < 0.001);
    assert_eq!(stats.underrun_frames.load(Relaxed), 0);
    config.master_db = 12.0;
    config.lanes[0].gain_db = 12.0;
    config.lanes[1].gain_db = 12.0;
    c.apply(config).unwrap();
    let f = render(&mut m, &mut p);
    assert!(f[0].abs() <= 0.98 && f[1].abs() <= 0.98);
    assert!(stats.limited_frames.load(Relaxed) > 0);
}
#[test]
fn inactive_identity_epoch_and_nan_never_leak_old_audio() {
    let (mut m, mut c, mut p, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 2,
        ..LaneMix::default()
    };
    c.apply(config).unwrap();
    p[0].push(block(1, 1, 0, [1.0; 2]));
    p[0].push(block(999, 2, 480, [1.0; 2]));
    for _ in 0..960 {
        assert_eq!(m.next_frame(), [0.0; 2]);
    }
    assert_eq!(stats.rejected_blocks.load(Relaxed), 2);
    p[0].push(block(1, 2, 0, [f32::NAN, f32::INFINITY]));
    p[0].push(block(1, 2, 480, [f32::NAN; 2]));
    for _ in 0..960 {
        assert_eq!(m.next_frame(), [0.0; 2]);
    }
}
#[test]
fn drift_estimator_filters_jitter_limits_slew_and_resets() {
    for injected in [-500.0, -100.0, 100.0, 500.0] {
        let mut d = DriftController::default();
        let mut previous = 0.0;
        for second in 0..600u64 {
            let output = second * 48000;
            let source = (output as f64 * (1.0 + injected / 1_000_000.0)) as u64;
            let commanded = d.observe(
                source,
                output,
                neonmix_core::mixer::TARGET_WATER_FRAMES,
                48000,
            );
            assert!((commanded - previous).abs() <= 50.0001);
            previous = commanded;
        }
        assert!((previous - injected).abs() < 20.0, "{injected}: {previous}");
        d.reset();
        assert_eq!(
            d.observe(0, 0, neonmix_core::mixer::TARGET_WATER_FRAMES, 480),
            0.0
        );
    }
}
#[test]
fn volume_zero_and_stream_removal_end_at_silence_with_a_short_tail() {
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let mut position = 0;
    let mut frames = [[0.0; 2]; 480];
    // Fill the bounded startup reservoir before checking audible gain.
    for _ in 0..neonmix_core::mixer::PCM_QUEUE_BLOCKS {
        inputs[0].push(block(1, 1, position, [0.2; 2]));
        position += 480;
    }
    for _ in 0..3 {
        mixer.render_block(&mut frames);
    }
    assert!(frames[479][0] > 0.19);
    config.lanes[0].gain_db = -96.0;
    control.apply(config).unwrap();
    mixer.render_block(&mut frames);
    assert_eq!(frames[479], [0.0; 2]);
    config.lanes[0].gain_db = 0.0;
    control.apply(config).unwrap();
    for _ in 0..3 {
        inputs[0].push(block(1, 1, position, [0.2; 2]));
        position += 480;
    }
    mixer.render_block(&mut frames);
    assert!(frames[479][0] > 0.19);
    config.lanes[0] = LaneMix::default();
    control.apply(config).unwrap();
    mixer.render_block(&mut frames);
    assert!(frames[0][0] > 0.19 && frames[239][0] < 0.001);
    assert!(frames[240..].iter().all(|f| *f == [0.0; 2]));
}

#[test]
fn every_gain_change_finishes_in_five_ms_including_low_and_high_levels() {
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let mut position = 0;
    for _ in 0..8 {
        assert!(inputs[0].push(block(1, 1, position, [0.2; 2])));
        position += 480;
    }
    let mut frames = [[0.0; 2]; 480];
    mixer.render_block(&mut frames);
    for _ in 0..4 {
        assert!(inputs[0].push(block(1, 1, position, [0.2; 2])));
        position += 480;
        mixer.render_block(&mut frames);
    }
    config.lanes[0].gain_db = -20.0;
    control.apply(config).unwrap();
    mixer.render_block(&mut frames);
    assert!(
        (frames[119][0] - 0.11).abs() < 0.0001,
        "wrong unity-to-low midpoint: {}",
        frames[119][0]
    );
    assert!((frames[239][0] - 0.02).abs() < 0.0001);
    config.lanes[0].muted = true;
    control.apply(config).unwrap();
    assert!(inputs[0].push(block(1, 1, position, [0.2; 2])));
    position += 480;
    mixer.render_block(&mut frames);
    assert!(
        (frames[119][0] - 0.01).abs() < 0.0001,
        "low-gain mute faded too early"
    );
    assert_eq!(frames[239], [0.0; 2]);
    config.lanes[0].muted = false;
    config.lanes[0].gain_db = 12.0;
    control.apply(config).unwrap();
    assert!(inputs[0].push(block(1, 1, position, [0.2; 2])));
    position += 480;
    mixer.render_block(&mut frames);
    let high = 0.2 * 10.0f32.powf(12.0 / 20.0);
    assert!((frames[119][0] - high / 2.0).abs() < 0.0001);
    assert!(
        (frames[239][0] - high).abs() < 0.0001,
        "+12 dB gain took longer than 5 ms"
    );
    config.master_db = -20.0;
    control.apply(config).unwrap();
    assert!(inputs[0].push(block(1, 1, position, [0.2; 2])));
    mixer.render_block(&mut frames);
    assert!((frames[119][0] - high * 0.55).abs() < 0.0001);
    assert!((frames[239][0] - high * 0.1).abs() < 0.0001);
}

#[test]
fn buffering_silence_does_not_consume_the_resume_fade() {
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        gain_db: -20.0,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    assert!(inputs[0].push(block(1, 1, 0, [0.2; 2])));
    let mut frames = [[0.0; 2]; 480];
    for _ in 0..10 {
        mixer.render_block(&mut frames);
        assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    }
    for n in 1..8 {
        assert!(inputs[0].push(block(1, 1, n * 480, [0.2; 2])));
    }
    mixer.render_block(&mut frames);
    assert!(
        (frames[119][0] - 0.01).abs() < 0.0001,
        "buffering completed the fade before PCM arrived: {}",
        frames[119][0]
    );
    assert!((frames[239][0] - 0.02).abs() < 0.0001);
    // Exercise the same envelope after a real FIFO exhaustion/reset, rather
    // than only the initial buffering state.
    for _ in 0..12 {
        mixer.render_block(&mut frames);
    }
    assert!(stats.underrun_frames.load(Relaxed) > 0);
    assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    for n in 8..16 {
        assert!(inputs[0].push(block(1, 1, n * 480, [0.2; 2])));
    }
    mixer.render_block(&mut frames);
    assert!(
        (frames[119][0] - 0.01).abs() < 0.0001,
        "underflow recovery skipped its fade"
    );
    assert!((frames[239][0] - 0.02).abs() < 0.0001);
}

#[test]
fn live_meters_follow_post_fader_samples_and_clear_after_mute_missing_pcm_and_reuse() {
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: -6.0206,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 42,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let mut frames = [[0.0; 2]; 480];
    let mut position = 0;
    for _ in 0..8 {
        assert!(inputs[0].push(block(42, 1, position, [0.2, 0.4])));
        position += 480;
    }
    let mut feed = |mixer: &mut Mixer, inputs: &mut Vec<neonmix_core::queue::BlockProducer>| {
        for _ in 0..15 {
            mixer.render_block(&mut frames);
            assert!(inputs[0].push(block(42, 1, position, [0.2, 0.4])));
            position += 480;
        }
    };
    feed(&mut mixer, &mut inputs);
    let lane = stats.lane_meters[0].snapshot();
    assert_eq!(lane.stream_id, 42);
    assert!((lane.peak - 0.4).abs() < 0.001);
    assert!((lane.rms - 0.1f32.sqrt()).abs() < 0.001);
    let output = stats.output_meter.snapshot();
    assert!((output.peak - 0.2).abs() < 0.001);
    assert!((output.rms - 0.1f32.sqrt() / 2.0).abs() < 0.001);
    assert_eq!(stats.lane_meters[1].snapshot().peak, 0.0);

    // Room mute belongs after the lane meters; the lane still consumes PCM.
    config.muted = true;
    control.apply(config).unwrap();
    feed(&mut mixer, &mut inputs);
    assert!(stats.lane_meters[0].snapshot().peak > 0.39);
    assert_eq!(stats.output_meter.snapshot().peak, 0.0);
    assert_eq!(stats.output_meter.snapshot().rms, 0.0);

    config.muted = false;
    config.lanes[0].muted = true;
    control.apply(config).unwrap();
    feed(&mut mixer, &mut inputs);
    assert_eq!(stats.lane_meters[0].snapshot().peak, 0.0);
    assert_eq!(stats.lane_meters[0].snapshot().rms, 0.0);
    config.lanes[0].muted = false;
    control.apply(config).unwrap();
    feed(&mut mixer, &mut inputs);
    assert!(stats.lane_meters[0].snapshot().peak > 0.39);

    config.lanes[1] = LaneMix {
        stream_id: 7,
        epoch: 1,
        solo: true,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    feed(&mut mixer, &mut inputs);
    assert_eq!(stats.lane_meters[0].snapshot().peak, 0.0);
    config.lanes[1] = LaneMix::default();
    control.apply(config).unwrap();
    feed(&mut mixer, &mut inputs);
    assert!(stats.lane_meters[0].snapshot().peak > 0.39);

    // Exhausting the PCM reservoir clears the latest window, rather than
    // leaving a historical maximum lit while the same stream is buffering.
    for _ in 0..30 {
        mixer.render_block(&mut [[0.0; 2]; 480]);
    }
    assert_eq!(stats.lane_meters[0].snapshot().stream_id, 42);
    assert_eq!(stats.lane_meters[0].snapshot().peak, 0.0);
    assert_eq!(stats.lane_meters[0].snapshot().rms, 0.0);
    assert_eq!(stats.output_meter.snapshot().peak, 0.0);
    feed(&mut mixer, &mut inputs);
    assert!(stats.lane_meters[0].snapshot().peak > 0.39);

    // A new occupant starts with zero immediately, before a window completes.
    config.lanes[0].stream_id = 99;
    control.apply(config).unwrap();
    mixer.next_frame();
    let reused = stats.lane_meters[0].snapshot();
    assert_eq!(reused.stream_id, 99);
    assert_eq!(reused.peak, 0.0);
    assert_eq!(reused.rms, 0.0);
    for _ in 0..30 {
        mixer.render_block(&mut [[0.0; 2]; 480]);
    }
    assert_eq!(stats.lane_meters[0].snapshot().peak, 0.0);
    assert_eq!(stats.output_meter.snapshot().peak, 0.0);
    // Same stream id under a new binding/output clock cannot retain old levels.
    let old = stats.lane_meters[0].snapshot();
    config.lanes[0].epoch = 2;
    config.lanes[0].binding_generation = inputs[0].bind_next().unwrap();
    config.output_epoch = 7;
    control.apply(config).unwrap();
    assert_eq!(
        stats.lane_meters[0].snapshot().binding_generation,
        old.binding_generation
    );
    mixer.render_block(&mut [[0.; 2]; 480]);
    let rebound = stats.lane_meters[0].snapshot();
    assert!(rebound.observed);
    assert_eq!(
        rebound.binding_generation,
        config.lanes[0].binding_generation
    );
    assert_eq!(rebound.stream_epoch, 2);
    assert_eq!(rebound.output_epoch, 7);
    assert_eq!(rebound.peak, 0.0);
    let reopened = stats.output_meter.snapshot();
    assert_eq!(reopened.output_epoch, 7);
    assert_eq!(reopened.peak, 0.0);
}

#[test]
fn live_output_meter_and_limiter_gain_recover_after_overload() {
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig {
        master_db: 0.0,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let mut position = 0;
    for _ in 0..8 {
        assert!(inputs[0].push(block(1, 1, position, [2.0; 2])));
        position += 480;
    }
    let mut frames = [[0.0; 2]; 480];
    for _ in 0..15 {
        mixer.render_block(&mut frames);
        assert!(inputs[0].push(block(1, 1, position, [2.0; 2])));
        position += 480;
    }
    assert!(stats.lane_meters[0].snapshot().peak > 1.99);
    assert!((stats.output_meter.snapshot().peak - 0.98).abs() < 0.001);
    assert!(f32::from_bits(stats.limiter_gain_bits.load(Relaxed) as u32) < 0.5);
    let limited = stats.limited_frames.load(Relaxed);
    config.lanes[0].muted = true;
    control.apply(config).unwrap();
    for _ in 0..100 {
        mixer.render_block(&mut frames);
        assert!(inputs[0].push(block(1, 1, position, [2.0; 2])));
        position += 480;
    }
    assert_eq!(stats.output_meter.snapshot().peak, 0.0);
    assert_eq!(stats.output_meter.snapshot().rms, 0.0);
    assert!(f32::from_bits(stats.limiter_gain_bits.load(Relaxed) as u32) > 0.999);
    assert!(stats.limited_frames.load(Relaxed) >= limited);
    mixer.discard_backlog();
    assert_eq!(stats.lane_meters[0].snapshot().peak, 0.0);
    assert_eq!(stats.output_meter.snapshot().peak, 0.0);
    assert_eq!(
        f32::from_bits(stats.limiter_gain_bits.load(Relaxed) as u32),
        1.0
    );
}

#[test]
fn meter_timestamp_only_advances_on_the_actual_mixer_publication_clock() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 42,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    for position in (0..3840).step_by(480) {
        assert!(inputs[0].push(block(42, 1, position, [0.1, 0.2])));
    }
    mixer.render_block_at(
        origin + std::time::Duration::from_millis(100),
        &mut [[0.; 2]; 2400],
    );
    let first = stats.output_meter.snapshot();
    assert!(first.observed);
    assert_eq!(first.sampled_at_ns, 100_000_000);
    assert!(first.peak > 0.);
    // Repeated readers cannot rejuvenate the stopped audio callback.
    assert_eq!(
        stats.output_meter.snapshot().sampled_at_ns,
        first.sampled_at_ns
    );
    mixer.render_block_at(
        origin + std::time::Duration::from_millis(500),
        &mut [[0.; 2]; 2400],
    );
    assert_eq!(stats.output_meter.snapshot().sampled_at_ns, 500_000_000);
}
