use neonmix_core::{clock::frames_to_ns, resample::RateConverter, signal::StereoSource};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Recorder {
    position: u64,
    hooks: Arc<Mutex<Vec<(u64, Instant)>>>,
}
impl StereoSource for Recorder {
    fn set_presentation_time(&mut self, at: Instant) {
        self.hooks.lock().unwrap().push((self.position, at));
    }
    fn next_frame(&mut self) -> [f32; 2] {
        self.position += 1;
        [0.; 2]
    }
}
#[test]
fn callback_hooks_include_pending_frames_and_retained_sinc_lookahead() {
    for rate in [44100, 96000] {
        let hooks = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder {
            position: 0,
            hooks: hooks.clone(),
        };
        let mut source = RateConverter::new(recorder, 48000, rate).unwrap();
        let at = Instant::now();
        let mut output = 0;
        for period in [100, 127, 441, 513, 128, 256, 100, 127, 441, 513] {
            source.set_presentation_time(at + Duration::from_nanos(frames_to_ns(output, rate)));
            for _ in 0..period {
                source.next_frame();
            }
            output += period;
        }
        let hooks = hooks.lock().unwrap();
        assert!(hooks.len() > 2);
        let (first_pos, first_at) = hooks[0];
        assert_eq!(first_pos, 0);
        assert_eq!(first_at, at);
        for (position, presentation) in hooks.iter().copied() {
            let expected = frames_to_ns(position, 48000);
            let observed = (presentation - first_at).as_nanos() as u64;
            assert!(
                expected.abs_diff(observed) <= frames_to_ns(2, rate),
                "rate {rate}, source position {position}, expected {expected}, observed {observed}"
            );
        }
    }
}

struct Impulse {
    position: usize,
    at: usize,
}
impl StereoSource for Impulse {
    fn next_frame(&mut self) -> [f32; 2] {
        let sample = if self.position == self.at { 1.0 } else { 0.0 };
        self.position += 1;
        [sample; 2]
    }
}
#[test]
fn converter_impulse_tracks_source_time_without_adding_output_delay_twice() {
    for rate in [44100, 96000] {
        for input in [0, 256, 512] {
            let mut converter = RateConverter::new(
                Impulse {
                    position: 0,
                    at: input,
                },
                48000,
                rate,
            )
            .unwrap();
            let mut peak = (0, 0.0f32);
            for frame in 0..2000 {
                let sample = converter.next_frame()[0].abs();
                if sample > peak.1 {
                    peak = (frame, sample);
                }
            }
            let expected = input as u64 * u64::from(rate) / 48000;
            assert!((peak.0 as u64).abs_diff(expected) <= 1);
            assert!(peak.1 > 0.7);
        }
    }
}
