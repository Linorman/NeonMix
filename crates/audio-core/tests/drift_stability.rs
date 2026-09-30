//! Accelerated 10-minute source-clock injection through the actual sinc mixer.
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, MAX_WATER_FRAMES, Mixer, MixerConfig, TARGET_WATER_FRAMES},
    signal::{SignalKind, StereoSource, TestSignal},
};
use std::{sync::atomic::Ordering::Relaxed, time::Instant};
#[test]
#[ignore = "Run with --release --ignored: four accelerated 600-second drift cases"]
fn injected_clocks_keep_pcm_bounded_and_preserve_tone() {
    for ppm in [-500.0, -100.0, 100.0, 500.0] {
        let (mut mixer, mut control, mut producers, stats) = Mixer::new(Instant::now()).unwrap();
        let mut config = MixerConfig::default();
        config.lanes[0] = LaneMix {
            stream_id: 1,
            epoch: 1,
            ..LaneMix::default()
        };
        control.apply(config).unwrap();
        // Keep the physical tone at 437 Hz while changing its source clock.
        let mut signal = TestSignal::new(
            SignalKind::Sine,
            48000,
            437.0 / (1.0 + ppm / 1_000_000.0),
            -36.0,
        )
        .unwrap();
        let mut source = 0u64;
        let mut feed = |producer: &mut neonmix_core::queue::BlockProducer| {
            let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
            b.header.stream_epoch = 1;
            b.header.source_sample_position = source;
            b.header.frame_count = 480;
            b.header.arrival_ns = u64::MAX;
            b.header.discontinuity_flags = if source == 0 {
                Discontinuity::START
            } else {
                Discontinuity::NONE
            };
            for f in &mut b.pcm {
                *f = signal.next_frame();
            }
            source += 480;
            assert!(producer.push(b));
        };
        for _ in 0..4 {
            feed(&mut producers[0]);
        }
        let mut budget = 0.0;
        let mut frames = [[0.0; 2]; 480];
        let mut crossings = 0u64;
        let mut last = 0.0;
        let mut energy = 0.0;
        let mut measured = 0u64;
        let mut min_water = u64::MAX;
        let mut max_water = 0;
        for tick in 0..60_000 {
            budget += 480.0 * (1.0 + ppm / 1_000_000.0);
            while budget >= 480.0 {
                feed(&mut producers[0]);
                budget -= 480.0;
            }
            mixer.render_block(&mut frames);
            if tick > 1000 {
                let water = stats.queue_frames[0].load(Relaxed);
                min_water = min_water.min(water);
                max_water = max_water.max(water);
            }
            if tick >= 59_000 {
                for frame in frames {
                    if last <= 0.0 && frame[0] > 0.0 {
                        crossings += 1;
                    }
                    last = frame[0];
                    energy += f64::from(frame[0]).powi(2);
                    measured += 1;
                }
            }
        }
        let frequency = crossings as f64 / (measured as f64 / 48000.0);
        let rms = (energy / measured as f64).sqrt();
        let commanded = stats.drift_ppm_milli[0].load(Relaxed) as i64 as f64 / 1000.0;
        println!(
            "{{\"injected_ppm\":{ppm},\"duration_seconds\":600,\"commanded_ppm\":{commanded},\"min_queue_frames\":{min_water},\"max_queue_frames\":{max_water},\"frequency_hz\":{frequency},\"rms\":{rms},\"underrun_frames\":{}}}",
            stats.underrun_frames.load(Relaxed)
        );
        assert_eq!(stats.underrun_frames.load(Relaxed), 0);
        assert_eq!(stats.discontinuities.load(Relaxed), 0);
        assert!(max_water <= (TARGET_WATER_FRAMES + 960) as u64 && min_water >= 480);
        assert!((frequency - 437.0).abs() < 0.3);
        assert!((rms - 0.002814).abs() < 0.0001);
    }
}
#[test]
#[ignore = "Release-only accelerated over-limit drift degradation"]
fn unsupported_clocks_degrade_with_bounded_memory_and_finite_output() {
    for ppm in [-2000.0f64, -1000.0, 1000.0, 2000.0] {
        let (mut mixer, mut control, mut producers, stats) = Mixer::new(Instant::now()).unwrap();
        let mut config = MixerConfig::default();
        config.lanes[0] = LaneMix {
            stream_id: 1,
            epoch: 1,
            ..LaneMix::default()
        };
        control.apply(config).unwrap();
        let mut source = 0u64;
        let mut dropped = 0u64;
        let mut feed = |producer: &mut neonmix_core::queue::BlockProducer| {
            let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
            b.header.stream_epoch = 1;
            b.header.source_sample_position = source;
            b.header.frame_count = 480;
            b.header.arrival_ns = u64::MAX;
            b.header.discontinuity_flags = if source == 0 {
                Discontinuity::START
            } else {
                Discontinuity::NONE
            };
            b.pcm.fill([0.1; 2]);
            source += 480;
            if !producer.push(b) {
                dropped += 480;
            }
        };
        for _ in 0..4 {
            feed(&mut producers[0]);
        }
        let mut budget = 0.0;
        let mut frames = [[0.0; 2]; 480];
        let mut max_water = 0;
        for _ in 0..30_000 {
            budget += 480.0 * (1.0 + ppm / 1_000_000.0);
            while budget >= 480.0 {
                feed(&mut producers[0]);
                budget -= 480.0;
            }
            mixer.render_block(&mut frames);
            max_water = max_water.max(stats.queue_frames[0].load(Relaxed));
            assert!(max_water <= MAX_WATER_FRAMES as u64);
            assert!((stats.drift_ppm_milli[0].load(Relaxed) as i64).abs() <= 1_000_000);
            assert!(
                frames
                    .iter()
                    .flatten()
                    .all(|x| x.is_finite() && x.abs() <= 0.98)
            );
        }
        let underrun = stats.underrun_frames.load(Relaxed);
        let resets = stats.discontinuities.load(Relaxed);
        println!(
            "{{\"injected_ppm\":{ppm},\"seconds\":300,\"max_water\":{max_water},\"dropped_frames\":{dropped},\"underrun_frames\":{underrun},\"resets\":{resets}}}"
        );
        // ±1000 is the controller limit, not the supported-clock contract.
        // Record its measured boundary behavior; beyond it degradation must
        // be explicit rather than allowing unbounded latency or invalid PCM.
        if ppm.abs() > 1000.0 {
            assert!(dropped + underrun + resets > 0);
        }
    }
}

#[test]
#[ignore = "Release-only accelerated 300-second delivery-stall regression"]
fn sixty_ms_delivery_stall_preserves_both_timelines_without_pcm_concealment() {
    fn block(lane: usize, position: u64) -> AudioBlock {
        let mut b = AudioBlock::empty(lane as u64 + 1, AudioFormat::INTERNAL);
        b.header.stream_epoch = 1;
        b.header.source_sample_position = position;
        b.header.frame_count = 480;
        b.header.arrival_ns = u64::MAX;
        b.header.discontinuity_flags = if position == 0 {
            Discontinuity::START
        } else {
            Discontinuity::NONE
        };
        for (i, frame) in b.pcm.iter_mut().enumerate() {
            let value = (std::f64::consts::TAU
                * (437.0 + lane as f64 * 222.0)
                * (position + i as u64) as f64
                / 48000.0)
                .sin() as f32
                * 0.01;
            *frame = [value; 2];
        }
        b
    }
    let (mut mixer, mut control, mut producers, stats) = Mixer::new(Instant::now()).unwrap();
    let mut config = MixerConfig::default();
    for (i, lane) in config.lanes.iter_mut().take(2).enumerate() {
        *lane = LaneMix {
            stream_id: i as u64 + 1,
            epoch: 1,
            ..LaneMix::default()
        };
    }
    control.apply(config).unwrap();
    let mut positions = [0u64; 2];
    let mut held = [Vec::new(), Vec::new()];
    for (lane, producer) in producers.iter_mut().take(2).enumerate() {
        for _ in 0..7 {
            assert!(producer.push(block(lane, positions[lane])));
            positions[lane] += 480;
        }
    }
    let mut frames = [[0.0; 2]; 480];
    // Settle the controller for 100 seconds before the six-period delivery
    // stall. Source clocks continue; restore real samples, never silence/PLC.
    for tick in 0..30000 {
        for (lane, producer) in producers.iter_mut().take(2).enumerate() {
            let b = block(lane, positions[lane]);
            positions[lane] += 480;
            if (10000..10006).contains(&tick) {
                held[lane].push(b);
            } else {
                for b in held[lane].drain(..) {
                    assert!(producer.push(b));
                }
                assert!(producer.push(b));
            }
        }
        mixer.render_block(&mut frames);
        if tick > 10 {
            assert!(
                frames.iter().any(|f| f[0].abs() > 0.0001),
                "silent output at tick {tick}, underrun={}",
                stats.underrun_frames.load(Relaxed)
            );
        }
    }
    assert_eq!(stats.underrun_frames.load(Relaxed), 0);
    assert_eq!(stats.discontinuities.load(Relaxed), 0);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 0);
}
