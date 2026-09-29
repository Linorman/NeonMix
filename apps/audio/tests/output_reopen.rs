//! Explicit hardware acceptance. Normal CI does not open an ambient/default device.
use neonmix_core::{clock::OutputPosition, position::PositionReader, signal::SignalKind};
use neonmix_io::{NativeBackend, OpenOptions, RunningOutput};
use std::{
    thread,
    time::{Duration, Instant},
};

fn backend() -> NativeBackend {
    #[cfg(target_os = "macos")]
    return neonmix_macos::backend().unwrap();
    #[cfg(target_os = "windows")]
    return neonmix_windows::backend().unwrap();
    #[cfg(target_os = "linux")]
    return neonmix_linux::backend().unwrap();
}

fn wait_position(output: &RunningOutput) -> OutputPosition {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert_eq!(output.stats.snapshot().errors, 0, "native output fault");
        if let Some(position) = output.latest_position() {
            return position;
        }
        assert!(Instant::now() < deadline, "no output clock received");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
#[ignore = "requires explicit NEONMIX_TEST_OUTPUT_DEVICE and physical native audio hardware"]
fn same_process_reopen_rebuilds_output_clock() {
    let device = std::env::var("NEONMIX_TEST_OUTPUT_DEVICE")
        .expect("Set NEONMIX_TEST_OUTPUT_DEVICE to an explicit stable output ID");
    let backend = backend();
    let mut closed: Vec<(PositionReader, OutputPosition)> = Vec::new();
    for run in 0..3 {
        let output = backend
            .open_test_output(
                &device,
                OpenOptions {
                    sample_rate: Some(48_000),
                    period_frames: Some(256),
                },
                SignalKind::Silence,
                437.0,
                -42.0,
            )
            .unwrap();
        assert_eq!(output.info.device_id, device);
        assert!(
            output.latest_position().is_none(),
            "new stream exposes an old observation"
        );
        output.play().unwrap();
        let first = wait_position(&output);
        assert_eq!(first.epoch, output.info.stream_epoch);
        assert_eq!(first.sample_rate, 48_000);
        assert!(first.native_device_frame_position.is_some());
        assert!(
            first.native_stream_frames.unwrap() < 24_000,
            "new native origin was not reset"
        );
        assert!(
            first.submitted_frames < 24_000,
            "new submitted position was not reset"
        );
        for (_, previous) in &closed {
            assert!(
                first.epoch > previous.epoch,
                "reopen reused an old clock epoch"
            );
        }
        thread::sleep(Duration::from_millis(1200));
        let end = wait_position(&output);
        assert!(end.clock_timestamp_ns > first.clock_timestamp_ns);
        assert!(end.native_stream_frames > first.native_stream_frames);
        assert!(end.submitted_frames > 48_000);
        assert_eq!(
            end.epoch, first.epoch,
            "unexpected native clock discontinuity"
        );
        let reader = output.position_reader();
        let stats = output.stats.clone();
        output.control.stop();
        thread::sleep(Duration::from_millis(20));
        drop(output);
        let final_position = reader.latest().unwrap();
        let stats = stats.snapshot();
        println!(
            "{}",
            serde_json::json!({"event":"output_reopen_test", "run":run,
            "device_id":device, "first":first, "final":final_position, "stats":stats})
        );
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.callback_over_budget, 0);
        assert_eq!(stats.frames, stats.silent_frames);
        for (old_reader, previous) in &closed {
            assert_eq!(
                old_reader.latest(),
                Some(*previous),
                "new stream mutated closed stream state"
            );
        }
        closed.push((reader, final_position));
    }
}
