use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig},
    signal::StereoSource,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{
    alloc::System,
    time::{Duration, Instant},
};
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
#[test]
fn dynamic_ratio_commands_and_stream_reuse_allocate_nothing_in_output() {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    let mut frames = [[0.0; 2]; 480];
    let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
    b.header.stream_epoch = 1;
    b.header.frame_count = 480;
    b.header.arrival_ns = u64::MAX;
    b.pcm.fill([0.1; 2]);
    control.apply(config).unwrap();
    inputs[0].push(b);
    b.header.source_sample_position = 480;
    b.header.discontinuity_flags = Discontinuity::NONE;
    inputs[0].push(b);
    mixer.render_block(&mut frames);
    let region = Region::new(GLOBAL);
    for n in 2..1200u64 {
        b.header.source_sample_position = n * 480;
        inputs[0].push(b);
        if n == 400 {
            config.lanes[0].muted = true;
            control.apply(config).unwrap();
        }
        if n == 600 {
            config.lanes[0].muted = false;
            control.apply(config).unwrap();
        }
        if n == 800 {
            config.lanes[0].epoch = 2;
            b.header.stream_epoch = 2;
            control.apply(config).unwrap();
        }
        mixer.render_block_at(origin + Duration::from_millis(n * 10), &mut frames);
    }
    let measured = region.change();
    assert_eq!(measured.allocations, 0);
    assert_eq!(measured.reallocations, 0);
    assert_eq!(measured.deallocations, 0);

    let origin = Instant::now();
    let (mut timed, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig::default();
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let region = Region::new(GLOBAL);
    for n in 0..1200u64 {
        b.header.stream_epoch = 1;
        b.header.source_sample_position = n * 480;
        b.header.presentation_time_ns = Some(1_000_000_000 + n * 10_000_000);
        inputs[0].push(b);
        timed.set_presentation_time(origin + Duration::from_nanos(1_000_000_000 + n * 10_000_000));
        if n == 400 {
            config.lanes[0].muted = true;
            control.apply(config).unwrap();
        }
        if n == 600 {
            config.lanes[0].muted = false;
            control.apply(config).unwrap();
        }
        if n == 900 {
            // Reopen between logical boundaries and apply the new binding
            // before the first new frame, with no allocation or destruction.
            timed.discard_backlog();
            config.output_epoch += 1;
            control.apply(config).unwrap();
        }
        timed.render_block_at(origin + Duration::from_millis(n * 10), &mut frames);
    }
    let measured = region.change();
    assert_eq!(measured.allocations, 0);
    assert_eq!(measured.reallocations, 0);
    assert_eq!(measured.deallocations, 0);

    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..Default::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
        ..Default::default()
    };
    control.apply(config).unwrap();
    let mut a = AudioBlock::empty(1, AudioFormat::INTERNAL);
    a.header.stream_epoch = 1;
    a.header.frame_count = 1;
    a.header.arrival_ns = u64::MAX;
    a.header.discontinuity_flags = Discontinuity::NONE;
    a.pcm.fill([0.2; 2]);
    let region = Region::new(GLOBAL);
    for tick in 0..64 {
        // Eight separate one-frame segments; every FIR boundary is isolated,
        // including a large missing time span. Descriptors are preallocated.
        for segment in 0..8 {
            a.header.source_sample_position = tick * 480 + segment * 2;
            a.header.presentation_time_ns = Some(
                1_000_000_000
                    + neonmix_core::clock::frames_to_ns(a.header.source_sample_position, 48000),
            );
            inputs[0].push(a);
        }
        mixer.set_presentation_time(
            origin + Duration::from_nanos(1_000_000_000 + tick * 10_000_000),
        );
        mixer.render_block_at(origin, &mut frames);
        if tick == 16 {
            while control.has_capacity() {
                control.apply(config).unwrap();
            }
            control.revoke_lane(0);
            mixer.render_block_at(origin, &mut frames);
            config.lanes[0].binding_generation = inputs[0].bind_next().unwrap();
            control.apply(config).unwrap();
        }
    }
    let measured = region.change();
    assert_eq!(measured.allocations, 0);
    assert_eq!(measured.reallocations, 0);
    assert_eq!(measured.deallocations, 0);
}
