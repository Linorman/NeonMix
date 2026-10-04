//! Synthetic PCM coverage of the shared mixer boundary; this does not exercise
//! AirPlay transport, worker admission, CoreAudio, or real Apple source devices.
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig},
    signal::StereoSource,
};
use std::{
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};

fn lane(stream_id: u64, epoch: u64) -> LaneMix {
    LaneMix {
        stream_id,
        epoch,
        ..LaneMix::default()
    }
}

fn packet(stream: u64, epoch: u64, tick: u64, value: [f32; 2]) -> AudioBlock {
    let mut block = AudioBlock::empty(stream, AudioFormat::INTERNAL);
    block.header.stream_epoch = epoch;
    block.header.source_sample_position = tick * 480;
    block.header.frame_count = 480;
    block.header.arrival_ns = u64::MAX;
    block.header.presentation_time_ns = Some(1_000_000_000 + tick * 10_000_000);
    block.header.discontinuity_flags = Discontinuity::NONE;
    block.pcm.fill(value);
    block
}

fn render(mixer: &mut Mixer, origin: Instant, tick: u64) -> [[f32; 2]; 480] {
    mixer.set_presentation_time(origin + Duration::from_millis(1000 + tick * 10));
    let mut output = [[0.; 2]; 480];
    mixer.render_block(&mut output);
    output
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.0001,
        "actual {actual}, expected {expected}"
    );
}

fn tone(lane: usize, tick: u64, frame: usize) -> [f32; 2] {
    // Orthogonal block frequencies and per-block phase changes expose both
    // dropped lanes and replay of a previous muted/solo-excluded block.
    let phase =
        std::f32::consts::TAU * ((lane + 2) as f32 * frame as f32 / 480. + tick as f32 * 0.071);
    [0.045 * phase.sin(), 0.035 * phase.cos()]
}

#[test]
fn four_distinct_streams_mix_with_independent_gain_mute_and_solo() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    for (index, entry) in config.lanes[..4].iter_mut().enumerate() {
        *entry = lane(100 + index as u64, 1);
        entry.gain_db = [0., -3., -6., -9.][index];
    }
    // Baseline, each independently muted, each independently soloed, restore.
    for tick in 0..10_u64 {
        for (index, entry) in config.lanes[..4].iter_mut().enumerate() {
            entry.muted = (1..=4).contains(&tick) && index as u64 == tick - 1;
            entry.solo = (5..=8).contains(&tick) && index as u64 == tick - 5;
        }
        control.apply(config).unwrap();
        for (index, input) in inputs[..4].iter_mut().enumerate() {
            let mut block = packet(100 + index as u64, 1, tick, [0.; 2]);
            for (frame, sample) in block.pcm.iter_mut().enumerate() {
                *sample = tone(index, tick, frame);
            }
            input.push(block);
        }
        let output = render(&mut mixer, origin, tick);
        for (frame, actual) in output.iter().enumerate().skip(240) {
            let mut expected = [0.; 2];
            for index in 0..4 {
                let entry = config.lanes[index];
                if entry.muted || ((5..=8).contains(&tick) && !entry.solo) {
                    continue;
                }
                let gain = 10_f32.powf(entry.gain_db / 20.);
                let sample = tone(index, tick, frame);
                for channel in 0..2 {
                    expected[channel] += sample[channel] * gain;
                }
            }
            for channel in 0..2 {
                close(actual[channel], expected[channel]);
            }
        }
    }
    assert_eq!(stats.output_frames.load(Relaxed), 4800);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 0);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
    assert_eq!(stats.limited_frames.load(Relaxed), 0);
}

#[test]
fn flushing_a_held_future_head_and_stale_packets_does_not_restart_other_lane() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = lane(101, 1);
    config.lanes[1] = lane(202, 9);
    control.apply(config).unwrap();
    let (mut reference, mut reference_control, mut reference_inputs, _) =
        Mixer::new(origin).unwrap();
    let mut reference_config = config;
    reference_config.lanes[0] = LaneMix::default();
    reference_control.apply(reference_config).unwrap();
    // A is held until a future deadline while B is already audible.
    let mut future = packet(101, 1, 0, [0.8, 0.]);
    future.header.presentation_time_ns = Some(2_000_000_000);
    inputs[0].push(future);
    inputs[1].push(packet(202, 9, 0, [0., 0.2]));
    reference_inputs[1].push(packet(202, 9, 0, [0., 0.2]));
    render(&mut reference, origin, 0);
    let output = render(&mut mixer, origin, 0);
    close(output[479][0], 0.);
    close(output[479][1], 0.2);

    for tick in 1..=3 {
        config.lanes[0].epoch = tick + 1;
        control.apply(config).unwrap();
        // A's late old-generation packet would visibly poison the left output.
        inputs[0].push(packet(101, tick, tick, [0.8, 0.]));
        inputs[0].push(packet(101, tick + 1, tick, [0.1, 0.]));
        let b_value = 0.2 + tick as f32 * 0.03;
        inputs[1].push(packet(202, 9, tick, [0., b_value]));
        reference_inputs[1].push(packet(202, 9, tick, [0., b_value]));
        let reference_output = render(&mut reference, origin, tick);
        let output = render(&mut mixer, origin, tick);
        // Check every B sample, including A's reset fade. This catches an
        // accidental global reset even if the final samples later recover.
        for (sample, expected) in output.iter().zip(reference_output) {
            close(sample[1], expected[1]);
        }
        close(output[479][1], b_value);
        for sample in &output[240..] {
            close(sample[0], 0.1);
        }
        assert!(output.iter().all(|sample| sample[0] <= 0.1001));
    }
    assert_eq!(stats.rejected_blocks.load(Relaxed), 3);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}

#[test]
fn reusing_a_solo_lane_rejects_old_stream_and_keeps_other_stream_at_current_time() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = lane(11, 1);
    config.lanes[0].solo = true;
    config.lanes[1] = lane(22, 1);
    control.apply(config).unwrap();
    inputs[0].push(packet(11, 1, 0, [0.1, 0.]));
    inputs[1].push(packet(22, 1, 0, [0., 0.7]));
    let output = render(&mut mixer, origin, 0);
    close(output[479][0], 0.1);
    close(output[479][1], 0.);

    config.lanes[0] = LaneMix::default();
    control.apply(config).unwrap();
    inputs[1].push(packet(22, 1, 1, [0., 0.2]));
    let output = render(&mut mixer, origin, 1);
    for sample in &output[240..] {
        close(sample[0], 0.);
        close(sample[1], 0.2);
    }
    // Reuse the physical lane with a distinct stream, even reusing epoch=1.
    // The control owner explicitly initializes fresh session flags.
    config.lanes[0] = lane(33, 1);
    control.apply(config).unwrap();
    inputs[0].push(packet(11, 1, 2, [0.9, 0.]));
    inputs[0].push(packet(33, 1, 2, [0.3, 0.]));
    inputs[1].push(packet(22, 1, 2, [0., 0.25]));
    let output = render(&mut mixer, origin, 2);
    // The short sinc transition from B's preceding 0.2 samples is expected;
    // a reset fade or replay of its solo-excluded 0.7 block is not.
    for sample in &output {
        assert!(sample[1] > 0.20 && sample[1] < 0.27);
    }
    for sample in &output[240..] {
        close(sample[0], 0.3);
        close(sample[1], 0.25);
    }
    assert_eq!(stats.rejected_blocks.load(Relaxed), 1);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}
