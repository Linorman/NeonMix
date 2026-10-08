//! Actual Mixer/queue regressions for slot reuse. No wall-clock sleeps.
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig, PlaybackKind},
    signal::StereoSource,
};
use std::{
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};

fn block(stream: u64, epoch: u64, position: u64, pts: Option<u64>, value: f32) -> AudioBlock {
    let mut block = AudioBlock::empty(stream, AudioFormat::INTERNAL);
    block.header.stream_epoch = epoch;
    block.header.source_sample_position = position;
    block.header.presentation_time_ns = pts;
    block.header.arrival_ns = u64::MAX;
    block.header.frame_count = 480;
    block.header.discontinuity_flags = Discontinuity::NONE;
    block.pcm.fill([value; 2]);
    block
}

#[test]
fn stale_timed_queue_head_cannot_choose_playback_for_new_native_binding() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 2,
        epoch: 2,
        ..Default::default()
    };
    control.apply(config).unwrap();
    // The old lane's timed head is invalidated but still occupies a ring slot.
    inputs[0].push(block(1, 1, 0, Some(1_000_000_000), 0.8));
    inputs[0].invalidate();
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut [[0.; 2]; 480]);
    for n in 0..8 {
        inputs[0].push(block(2, 2, n * 480, None, 0.2));
    }
    let mut rendered = [[0.; 2]; 1440];
    mixer.render_block(&mut rendered);
    assert!(
        rendered[960..].iter().any(|f| f[0] > 0.19),
        "new native binding remained silent"
    );
    assert_eq!(
        stats.rejected_blocks.load(Relaxed),
        0,
        "fresh native PCM must be accepted"
    );
}

#[test]
fn native_head_is_rejected_before_new_timed_stream_plays() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 2,
        epoch: 2,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, None, 0.8));
    inputs[0].push(block(2, 2, 0, Some(1_000_000_000), 0.2));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    let mut frames = [[0.; 2]; 480];
    mixer.render_block(&mut frames);
    assert!(frames[479][0] > 0.19);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 1);
}

#[test]
fn same_stream_wrong_kind_is_rejected_and_kind_change_resets_dsp() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, None, 0.9));
    inputs[0].push(block(1, 1, 0, Some(1_000_000_000), 0.3));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut [[0.; 2]; 480]);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 1);
    config.lanes[0].playback_kind = PlaybackKind::NativeAdaptive;
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 480, Some(1_010_000_000), 0.9));
    let mut priming = [[0.; 2]; 480];
    mixer.render_block(&mut priming);
    assert!(
        priming.iter().all(|f| *f == [0.; 2]),
        "old timed FIFO leaked into native binding"
    );
    assert_eq!(stats.rejected_blocks.load(Relaxed), 2);
    for n in 0..8 {
        inputs[0].push(block(1, 1, n * 480, None, 0.2));
    }
    let mut frames = [[0.; 2]; 1440];
    mixer.render_block(&mut frames);
    assert!(frames[960..].iter().any(|f| f[0] > 0.19));
}

#[test]
fn output_reopen_discards_old_timed_fifo_and_waits_for_new_deadline() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        output_epoch: 1,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, Some(1_000_000_000), 0.8));
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.next_frame();
    mixer.discard_backlog();
    config.output_epoch = 2;
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, Some(2_000_000_000), 0.2));
    mixer.set_presentation_time(origin + Duration::from_millis(1500));
    let mut before = [[0.; 2]; 480];
    mixer.render_block(&mut before);
    assert!(before.iter().all(|f| *f == [0.; 2]));
    mixer.set_presentation_time(origin + Duration::from_secs(2));
    mixer.render_block(&mut before);
    assert!(before[479][0] > 0.19);
    assert_eq!(stats.rejected_blocks.load(Relaxed), 0);
}

#[test]
fn revocation_cuts_off_buffered_pcm_even_when_config_queue_is_full_and_same_ids_are_reused() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    for n in 0..8 {
        assert!(inputs[0].push(block(
            1,
            1,
            n * 480,
            Some(1_000_000_000 + n * 10_000_000),
            0.8
        )));
    }
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut [[0.; 2]; 480]);
    while control.has_capacity() {
        control.apply(config).unwrap();
    }
    control.revoke_lane(0);
    let before = stats.underrun_frames.load(Relaxed);
    let mut closed = [[0.; 2]; 480];
    mixer.render_block(&mut closed);
    assert!(
        closed.iter().all(|frame| *frame == [0.; 2]),
        "revoked FIFO continued to render"
    );
    assert_eq!(
        stats.underrun_frames.load(Relaxed),
        before,
        "planned stop counted as starvation"
    );
    let new_binding = inputs[0].bind_next().unwrap();
    assert_eq!(new_binding, 2);
    config.lanes[0].binding_generation = new_binding;
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, Some(2_000_000_000), 0.2));
    mixer.set_presentation_time(origin + Duration::from_secs(2));
    mixer.render_block(&mut closed);
    assert!(
        closed[479][0] > 0.19 && closed[479][0] < 0.21,
        "same IDs replayed old lease PCM"
    );
}

#[test]
fn old_queue_binding_is_rejected_without_erasing_new_binding_and_mode() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    inputs[0].push(block(1, 1, 0, Some(1_000_000_000), 0.8));
    let binding = inputs[0].bind_next().unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        binding_generation: binding,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    inputs[0].push(block(1, 1, 0, Some(2_000_000_000), 0.2));
    mixer.set_presentation_time(origin + Duration::from_secs(2));
    let mut output = [[0.; 2]; 480];
    mixer.render_block(&mut output);
    assert!(output[479][0] > 0.19 && output[479][0] < 0.21);
    assert_eq!(stats.timed_late_frames.load(Relaxed), 0);
}

#[test]
fn revocation_after_inactive_config_cannot_continue_a_planned_retirement_tail() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut cfg = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    cfg.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..Default::default()
    };
    control.apply(cfg).unwrap();
    for index in 0..8 {
        inputs[0].push(block(1, 1, index * 480, None, 0.8));
    }
    let mut warm = [[0.; 2]; 1440];
    mixer.render_block(&mut warm);
    assert!(warm[1200][0] > 0.7);
    cfg.lanes[0] = LaneMix::default();
    control.apply(cfg).unwrap();
    mixer.render_block(&mut [[0.; 2]; 1]);
    control.revoke_lane(0);
    let mut stopped = [[0.; 2]; 240];
    mixer.render_block(&mut stopped);
    assert!(
        stopped.iter().all(|frame| *frame == [0.; 2]),
        "retirement tail escaped authorization"
    );
}
