use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig, PlaybackKind},
    signal::StereoSource,
};
use std::{
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};

#[test]
fn active_native_starvation_counts_every_internal_frame_after_dsp_reset() {
    for period in [1, 127, 480, 1024] {
        let (mut mixer, mut control, mut inputs, stats) = Mixer::new(Instant::now()).unwrap();
        let mut config = MixerConfig::default();
        config.lanes[0] = LaneMix {
            stream_id: 1,
            epoch: 1,
            ..Default::default()
        };
        control.apply(config).unwrap();
        mixer.render_block(&mut [[0.; 2]; 960]);
        assert_eq!(stats.underrun_frames.load(Relaxed), 0, "initial priming");
        let mut block = AudioBlock::empty(1, AudioFormat::INTERNAL);
        block.header.stream_epoch = 1;
        block.header.frame_count = 480;
        block.header.arrival_ns = u64::MAX;
        block.header.discontinuity_flags = Discontinuity::NONE;
        block.pcm.fill([0.2; 2]);
        for n in 0..8 {
            block.header.source_sample_position = n * 480;
            assert!(inputs[0].push(block));
        }
        // Consume valid PCM and go well beyond FIFO exhaustion.
        mixer.render_block(&mut vec![[0.; 2]; 9600]);
        let before = stats.underrun_frames.load(Relaxed);
        let mut output = vec![[0.; 2]; period];
        let mut remaining = 48000;
        while remaining != 0 {
            let frames = remaining.min(period);
            mixer.render_block(&mut output[..frames]);
            remaining -= frames;
        }
        assert_eq!(
            stats.underrun_frames.load(Relaxed) - before,
            48000,
            "period {period}"
        );
        assert_eq!(
            stats.underrun_frames_by_lane[0].load(Relaxed),
            stats.underrun_frames.load(Relaxed)
        );
        let missing = stats.underrun_frames.load(Relaxed);
        let consumed = stats.rendered_pcm_frames_by_lane[0].load(Relaxed);
        for n in 0..8 {
            block.header.source_sample_position = 52000 + n * 480;
            assert!(inputs[0].push(block));
        }
        mixer.render_block(&mut [[0.; 2]; 480]);
        assert_eq!(
            stats.underrun_frames.load(Relaxed),
            missing,
            "recovery must retain counters"
        );
        assert_eq!(
            stats.rendered_pcm_frames_by_lane[0].load(Relaxed) - consumed,
            480
        );
        assert_eq!(
            stats.render_state_by_lane[0].load(Relaxed),
            neonmix_core::mixer::LaneRenderState::Running as u64
        );
        config.lanes[0] = LaneMix::default();
        control.apply(config).unwrap();
        let stopped = stats.underrun_frames.load(Relaxed);
        mixer.render_block(&mut [[0.; 2]; 960]);
        assert_eq!(stats.underrun_frames.load(Relaxed), stopped);
    }
}

#[test]
fn active_timed_starvation_counts_one_second_and_stops_on_remove() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    let mut block = AudioBlock::empty(1, AudioFormat::INTERNAL);
    block.header.stream_epoch = 1;
    block.header.frame_count = 480;
    block.header.arrival_ns = u64::MAX;
    block.header.presentation_time_ns = Some(1_000_000_000);
    block.pcm.fill([0.2; 2]);
    inputs[0].push(block);
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut [[0.; 2]; 480]);
    let before = stats.underrun_frames.load(Relaxed);
    mixer.set_presentation_time(origin + Duration::from_millis(1010));
    mixer.render_block(&mut vec![[0.; 2]; 48000]);
    assert_eq!(stats.underrun_frames.load(Relaxed) - before, 48000);
    config.lanes[0] = LaneMix::default();
    control.apply(config).unwrap();
    let stopped = stats.underrun_frames.load(Relaxed);
    mixer.render_block(&mut [[0.; 2]; 480]);
    assert_eq!(stats.underrun_frames.load(Relaxed), stopped);
}

#[test]
fn valid_silent_muted_pcm_is_counted_as_running_instead_of_starved() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        muted: true,
        playback_kind: PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    let mut block = AudioBlock::empty(1, AudioFormat::INTERNAL);
    block.header.stream_epoch = 1;
    block.header.frame_count = 480;
    block.header.arrival_ns = u64::MAX;
    block.header.presentation_time_ns = Some(1_000_000_000);
    inputs[0].push(block);
    mixer.set_presentation_time(origin + Duration::from_secs(1));
    mixer.render_block(&mut [[0.; 2]; 480]);
    assert_eq!(stats.rendered_pcm_frames_by_lane[0].load(Relaxed), 480);
    assert_eq!(stats.underrun_frames.load(Relaxed), 0);
    assert_eq!(
        stats.render_state_by_lane[0].load(Relaxed),
        neonmix_core::mixer::LaneRenderState::Running as u64
    );
}
