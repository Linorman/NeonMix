//! Digital PCM quality through the public timed Mixer path, with no audio device.
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity,
    mixer::{LaneMix, Mixer, MixerConfig},
    signal::StereoSource,
};
use std::{
    f64::consts::TAU,
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Quality {
    recurrence_peak: f64,
    recurrence_rms: f64,
    phase_ns: Vec<f64>,
    late: u64,
    amplitude: Vec<f64>,
    crossing_count: usize,
}

fn render(frequency: f64, ppm: f64, jitter_ns: i64, seconds: usize) -> Quality {
    let origin = Instant::now();
    let (mut mixer, mut control, mut inputs, stats) = Mixer::new(origin).unwrap();
    let mut config = MixerConfig {
        master_db: 0.,
        ..MixerConfig::default()
    };
    config.lanes[0] = LaneMix {
        stream_id: 1,
        epoch: 1,
        ..LaneMix::default()
    };
    control.apply(config).unwrap();
    let rate = 48000. * (1. + ppm / 1_000_000.);
    let mut source = 0u64;
    let mut packet = 0;
    let mut output = 0u64;
    let mut last = [0.; 2];
    let mut recurrence_peak = 0f64;
    let mut squares = 0.;
    let mut measured = 0;
    let mut sine = 0.;
    let mut cosine = 0.;
    let mut phase_ns = Vec::new();
    let mut amplitude = Vec::new();
    let mut crossing_count = 0;
    let omega = TAU * frequency / 48000.;
    let mut frames = [[0.; 2]; 256];
    let total = seconds as u64 * 48000;
    while output < total {
        // Maintain a 50 ms supply horizon without enlarging the eight-block SPSC.
        let horizon = ((output + 2400) as f64 * rate / 48000.) as u64;
        while source < horizon && inputs[0].remaining_capacity() > 0 {
            let count = if packet % 2 == 0 { 480 } else { 383 };
            let mut b = AudioBlock::empty(1, AudioFormat::INTERNAL);
            b.header.stream_epoch = 1;
            b.header.frame_count = count;
            b.header.source_sample_position = source;
            b.header.presentation_time_ns =
                Some(1_000_000_000 + (source as f64 / rate * 1e9).round() as u64);
            b.header.arrival_ns = u64::MAX;
            b.header.discontinuity_flags = Discontinuity::NONE;
            for (i, f) in b.pcm[..usize::from(count)].iter_mut().enumerate() {
                let x = (TAU * frequency * (source + i as u64) as f64 / rate).sin() as f32 * 0.5;
                *f = [x; 2];
            }
            assert!(inputs[0].push(b));
            source += u64::from(count);
            packet += 1;
        }
        // Alternating jitter exercises exactly the hard callback re-anchoring seam.
        let jitter = if output == 0 {
            0
        } else if (output / 256).is_multiple_of(2) {
            jitter_ns
        } else {
            -jitter_ns
        };
        let at = (1_000_000_000i64 + (output as f64 / 48000. * 1e9).round() as i64 + jitter) as u64;
        mixer.set_presentation_time(origin + Duration::from_nanos(at));
        mixer.render_block(&mut frames);
        for f in frames {
            let x = f64::from(f[0]);
            if output >= 4800 && output < total {
                // A sine obeys this recurrence. Repeated/skipped source samples
                // create impulses here even if the frequency and RMS look right.
                if last[0] <= 0. && x > 0. {
                    crossing_count += 1;
                }
                let residual = x - 2. * omega.cos() * last[0] + last[1];
                recurrence_peak = recurrence_peak.max(residual.abs());
                squares += residual * residual;
                measured += 1;
                sine += x * (omega * output as f64).sin();
                cosine += x * (omega * output as f64).cos();
                if (output + 1).is_multiple_of(4800) {
                    amplitude.push(2. * sine.hypot(cosine) / 4800.);
                    phase_ns.push(cosine.atan2(sine) / (TAU * frequency) * 1e9);
                    sine = 0.;
                    cosine = 0.;
                }
            }
            last = [x, last[0]];
            output += 1;
        }
    }
    Quality {
        recurrence_peak,
        recurrence_rms: (squares / measured as f64).sqrt(),
        phase_ns,
        late: stats.timed_late_frames.load(Relaxed),
        amplitude,
        crossing_count,
    }
}

#[test]
fn callback_jitter_and_source_clock_error_do_not_create_pcm_spikes() {
    for frequency in [440., 1000.] {
        for ppm in [-500., -100., 100., 500.] {
            let baseline = render(frequency, ppm, 0, 3);
            for jitter in [100_000, 1_000_000] {
                let quality = render(frequency, ppm, jitter, 3);
                println!(
                    "frequency={frequency}, ppm={ppm}, jitter_ns={jitter}, recurrence_peak={}, rms={}, late={}",
                    quality.recurrence_peak, quality.recurrence_rms, quality.late
                );
                assert!(
                    quality.recurrence_peak < 0.0001,
                    "sample-repeat/drop spike: {quality:?}"
                );
                assert!(
                    quality.recurrence_rms < 0.00001,
                    "distorted waveform: {quality:?}"
                );
                assert!(baseline.recurrence_peak < 0.0001);
                assert!(quality.recurrence_peak < baseline.recurrence_peak + 0.00001);
                assert!(
                    quality.amplitude.iter().all(|a| (a - 0.5).abs() < 0.003)
                        && quality
                            .amplitude
                            .iter()
                            .skip(10)
                            .all(|a| (a - 0.5).abs() < 0.001),
                    "SRC passband gain: {quality:?}"
                );
                assert!((quality.crossing_count as f64 - frequency * 2.9).abs() < 2.);
                let phase_change = quality
                    .phase_ns
                    .iter()
                    .zip(&baseline.phase_ns)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0f64, f64::max);
                assert!(
                    phase_change < 25_000.,
                    "callback jitter changed the waveform phase by {phase_change} ns"
                );
                assert!(
                    quality.phase_ns.iter().all(|p| p.abs() < 1_500_000.),
                    "deadline phase is unbounded: {quality:?}"
                );
                assert_eq!(
                    quality.late, 0,
                    "jitter/rate conversion must not count as lost deadlines"
                );
            }
        }
    }
}

#[test]
#[ignore = "Run release ignored: accelerated two-minute absolute phase tracking"]
fn timed_source_clock_error_keeps_long_term_presentation_phase_bounded() {
    for ppm in [-500., -100., 100., 500.] {
        let q = render(440., ppm, 1_000_000, 120);
        assert!(q.recurrence_peak < 0.0001, "{q:?}");
        assert!(q.phase_ns.iter().all(|p| p.abs() < 1_500_000.), "{q:?}");
        let settled = &q.phase_ns[q.phase_ns.len() - 100..];
        let min = settled.iter().copied().fold(f64::INFINITY, f64::min);
        let max = settled.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max - min < 100_000.,
            "persistent phase drift at {ppm}ppm: {min}..{max}"
        );
        assert!(q.amplitude.iter().all(|a| (a - 0.5).abs() < 0.001));
        assert!((q.crossing_count as f64 - 440. * 119.9).abs() < 2.);
        println!(
            "ppm={ppm}, recurrence_peak={}, recurrence_rms={}, settled_phase_ns={min}..{max}, crossings={}",
            q.recurrence_peak, q.recurrence_rms, q.crossing_count
        );
        assert_eq!(q.late, 0);
    }
}

#[test]
fn fractional_src_preserves_upper_music_passband_after_clock_lock() {
    for frequency in [8000., 18000.] {
        for ppm in [-500., 500.] {
            let quality = render(frequency, ppm, 1_000_000, 4);
            // Measure steady tone amplitude after the clock servo acquires;
            // fast acquisition itself frequency-modulates a long Fourier window.
            let amplitudes = &quality.amplitude[25..];
            let min = amplitudes.iter().copied().fold(f64::INFINITY, f64::min);
            let max = amplitudes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert!(
                min > 0.499 && max < 0.501,
                "fractional SRC loses passband at {frequency} Hz / {ppm} ppm: {min}..{max}"
            );
            assert_eq!(quality.late, 0);
        }
    }
}
