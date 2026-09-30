use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig},
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, time::Instant};
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
#[test]
fn dynamic_ratio_commands_and_stream_reuse_allocate_nothing_in_output() {
    let (mut mixer, mut control, mut inputs, _) = Mixer::new(Instant::now()).unwrap();
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
        mixer.render_block(&mut frames);
    }
    let measured = region.change();
    assert_eq!(measured.allocations, 0);
    assert_eq!(measured.reallocations, 0);
    assert_eq!(measured.deallocations, 0);
}
