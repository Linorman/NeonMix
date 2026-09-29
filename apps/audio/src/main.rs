mod measurement;
use clap::{Args, Parser, Subcommand, ValueEnum};
use neonmix_core::{
    device_changes,
    resample::RateConverter,
    signal::{SignalKind, StereoSource, TestSignal},
    stats::ActivityMonitor,
};
use neonmix_io::{CaptureKind, NativeBackend, OpenOptions};
use serde_json::json;
use std::{
    io::{self, Write},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "NeonMix E00/E01 native audio laboratory. JSONL diagnostics; no audio is recorded."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Enumerate concrete devices with stable IDs, native formats and period ranges.
    Devices,
    /// Report device add/remove/format events. Control-thread polling, 1 Hz.
    Watch {
        #[arg(long, default_value_t = 10)]
        seconds: u32,
    },
    /// Play a low-level test signal to one explicitly selected output.
    Play {
        #[command(flatten)]
        device: DeviceArgs,
        #[command(flatten)]
        lifecycle: Lifecycle,
        #[arg(long, value_enum, default_value = "sine")]
        signal: Signal,
        #[arg(long, value_enum, default_value = "both")]
        channel: TestChannel,
        #[arg(long, default_value_t = 440.0)]
        frequency: f64,
        #[arg(long, default_value_t = -36.0, allow_hyphen_values = true)]
        gain_db: f32,
    },
    /// Read an installed virtual device or an explicitly selected loopback endpoint.
    Capture {
        #[command(flatten)]
        device: DeviceArgs,
        #[command(flatten)]
        lifecycle: Lifecycle,
        #[arg(long, value_enum, default_value = "virtual-device")]
        mode: CaptureMode,
        #[arg(long, default_value_t = 1)]
        stream_id: u64,
    },
    /// Deterministic offline source/rate conversion probe; does not open a device.
    Simulate {
        #[arg(long, default_value_t = 44100)]
        input_rate: u32,
        #[arg(long, default_value_t = 48000)]
        output_rate: u32,
        #[arg(long, default_value_t = 2)]
        seconds: u32,
    },
    /// Build and runtime identity. No private host/device information.
    Version,
    /// Create a user-session PipeWire sink for E01 capture experiments (Linux).
    Sink {
        #[arg(long, default_value = "lab")]
        room: String,
        #[arg(long, default_value_t = 2)]
        channels: u16,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
    },
}
#[derive(Args)]
struct DeviceArgs {
    #[arg(long)]
    device: String,
    #[arg(long)]
    rate: Option<u32>,
    #[arg(long)]
    period: Option<u32>,
    #[arg(long, default_value_t = 3)]
    seconds: u32,
}
impl DeviceArgs {
    fn options(&self) -> OpenOptions {
        OpenOptions {
            sample_rate: self.rate,
            period_frames: self.period,
        }
    }
}
#[derive(Args, Default)]
struct Lifecycle {
    #[arg(long)]
    mute_at: Option<u32>,
    #[arg(long)]
    unmute_at: Option<u32>,
    #[arg(long)]
    pause_at: Option<u32>,
    #[arg(long)]
    resume_at: Option<u32>,
}
#[derive(Clone, Copy, ValueEnum)]
enum Signal {
    Sine,
    Impulse,
    Silence,
}
impl From<Signal> for SignalKind {
    fn from(s: Signal) -> Self {
        match s {
            Signal::Sine => Self::Sine,
            Signal::Impulse => Self::Impulse,
            Signal::Silence => Self::Silence,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum TestChannel {
    Both,
    Left,
    Right,
    AntiPhase,
}
impl From<TestChannel> for neonmix_core::signal::ChannelPattern {
    fn from(value: TestChannel) -> Self {
        match value {
            TestChannel::Both => Self::Both,
            TestChannel::Left => Self::Left,
            TestChannel::Right => Self::Right,
            TestChannel::AntiPhase => Self::AntiPhase,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum CaptureMode {
    VirtualDevice,
    Loopback,
}
fn backend() -> Result<NativeBackend, neonmix_core::AudioError> {
    #[cfg(target_os = "macos")]
    {
        neonmix_macos::backend()
    }
    #[cfg(target_os = "windows")]
    {
        neonmix_windows::backend()
    }
    #[cfg(target_os = "linux")]
    {
        neonmix_linux::backend()
    }
}
fn emit(value: impl serde::Serialize) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
fn duration(seconds: u32) -> Result<Duration, Box<dyn std::error::Error>> {
    if !(1..=86400).contains(&seconds) {
        return Err("seconds must be 1..86400".into());
    }
    Ok(Duration::from_secs(u64::from(seconds)))
}
fn validate_lifecycle(
    l: &Lifecycle,
    seconds: u32,
    capture: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    for n in [l.mute_at, l.unmute_at, l.pause_at, l.resume_at]
        .into_iter()
        .flatten()
    {
        if n >= seconds {
            return Err("lifecycle event must precede the end of the probe".into());
        }
    }
    if !capture && (l.pause_at.is_some() || l.resume_at.is_some()) {
        return Err("pause/resume are capture-probe options; use mute/unmute for output".into());
    }
    if l.resume_at
        .is_some_and(|r| l.pause_at.is_none_or(|p| r <= p))
    {
        return Err("resume-at must be after pause-at".into());
    }
    if l.unmute_at
        .is_some_and(|r| l.mute_at.is_none_or(|p| r <= p))
    {
        return Err("unmute-at must be after mute-at".into());
    }
    Ok(())
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Version => emit(
            json!({"application":"neonmix-audio", "version":env!("CARGO_PKG_VERSION"),
            "os":std::env::consts::OS,"arch":std::env::consts::ARCH,"rust_toolchain":"1.95.0", "cpal":"0.18.2", "cpal_patch":"neonmix-native-io-3", "protocol_version":null}),
        )?,
        Command::Devices => emit(backend()?.devices()?)?,
        Command::Watch { seconds } => {
            let until = Instant::now() + duration(seconds)?;
            let mut previous = Vec::new();
            while Instant::now() < until {
                let current = backend()?.devices()?;
                for event in device_changes(&previous, &current) {
                    emit(event)?;
                }
                previous = current;
                thread::sleep(
                    Duration::from_secs(1).min(until.saturating_duration_since(Instant::now())),
                );
            }
        }
        Command::Play {
            device,
            lifecycle,
            signal,
            channel,
            frequency,
            gain_db,
        } => {
            let duration = duration(device.seconds)?;
            validate_lifecycle(&lifecycle, device.seconds, false)?;
            let source = TestSignal::new(signal.into(), 48_000, frequency, gain_db)?
                .with_pattern(channel.into());
            let output = backend()?.open_output(&device.device, device.options(), source)?;
            emit(
                json!({"event":"output_opened","info":output.info,"position_basis":"native_device_clock_plus_separate_presentation_estimate"}),
            )?;
            output.play()?;
            let start = Instant::now();
            let mut reported = 0;
            let mut activity = ActivityMonitor::default();
            while start.elapsed() < duration {
                let second = start.elapsed().as_secs() as u32;
                output.control.set_muted(
                    lifecycle.mute_at.is_some_and(|t| second >= t)
                        && lifecycle.unmute_at.is_none_or(|t| second < t),
                );
                if second > reported {
                    activity.tick(&output.stats);
                    emit(
                        json!({"event":"output_stats","stats":output.stats.snapshot(),"clock":output.latest_position()}),
                    )?;
                    reported = second;
                }
                let snapshot = output.stats.snapshot();
                if snapshot.errors > 0 {
                    emit(
                        json!({"event":"output_fault","info":output.info,"stats":snapshot,"clock":output.latest_position()}),
                    )?;
                    return Err(format!("output failed: error code {}", snapshot.last_error).into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            output.control.stop();
            thread::sleep(Duration::from_millis(20));
            let clock_reader = output.position_reader();
            let info = output.info.clone();
            let stats = output.stats.clone();
            drop(output);
            let snapshot = stats.snapshot();
            emit(
                json!({"event":"output_complete","info":info,"stats":snapshot,"clock":clock_reader.latest()}),
            )?;
            if snapshot.callbacks == 0 || snapshot.errors > 0 {
                return Err("output produced no callbacks or reported a device fault".into());
            }
        }
        Command::Capture {
            device,
            lifecycle,
            mode,
            stream_id,
        } => {
            let duration = duration(device.seconds)?;
            validate_lifecycle(&lifecycle, device.seconds, true)?;
            let kind = match mode {
                CaptureMode::VirtualDevice => CaptureKind::VirtualDevice,
                CaptureMode::Loopback => CaptureKind::Loopback,
            };
            let mut capture =
                backend()?.open_capture(&device.device, kind, device.options(), stream_id)?;
            emit(json!({"event":"capture_opened","info":capture.info}))?;
            capture.play()?;
            let start = Instant::now();
            let mut reported = 0;
            let mut activity = ActivityMonitor::default();
            let mut paused = false;
            let mut last_epoch = None;
            let mut measurement = measurement::CaptureMeasurement::default();
            while start.elapsed() < duration {
                let second = start.elapsed().as_secs() as u32;
                let should_pause = lifecycle.pause_at.is_some_and(|t| second >= t)
                    && lifecycle.resume_at.is_none_or(|t| second < t);
                if paused != should_pause {
                    if should_pause {
                        capture.pause()?;
                    } else {
                        capture.resume()?;
                    }
                    paused = should_pause;
                    emit(json!({"event":if paused {"capture_paused"} else {"capture_resumed"}}))?;
                }
                capture.control.set_muted(
                    lifecycle.mute_at.is_some_and(|t| second >= t)
                        && lifecycle.unmute_at.is_none_or(|t| second < t),
                );
                while let Some(block) = capture.pop() {
                    measurement.observe(&block);
                    if last_epoch != Some(block.header.stream_epoch)
                        || block.header.discontinuity_flags != neonmix_core::Discontinuity::NONE
                    {
                        emit(
                            json!({"event":"capture_block","header":block.header,"silent":block.is_silent()}),
                        )?;
                        last_epoch = Some(block.header.stream_epoch);
                    }
                }
                if second > reported {
                    let no_data = activity.tick(&capture.stats);
                    emit(
                        json!({"event":"capture_stats","no_data":no_data,"paused":paused,"stats":capture.stats.snapshot(),"measurement":measurement.snapshot()}),
                    )?;
                    reported = second;
                }
                let snapshot = capture.stats.snapshot();
                if snapshot.errors > 0 {
                    emit(
                        json!({"event":"capture_fault","info":capture.info,"stats":snapshot,"measurement":measurement.snapshot()}),
                    )?;
                    return Err(
                        format!("capture failed: error code {}", snapshot.last_error).into(),
                    );
                }
                // Only diagnostics/queue draining sleep. Native callbacks drive all audio.
                thread::sleep(Duration::from_millis(5));
            }
            capture.control.stop();
            let stats = capture.stats.clone();
            drop(capture);
            let snapshot = stats.snapshot();
            emit(
                json!({"event":"capture_complete","stats":snapshot,"measurement":measurement.snapshot()}),
            )?;
            if snapshot.errors > 0 {
                return Err(format!("capture failed: error code {}", snapshot.last_error).into());
            }
        }
        Command::Simulate {
            input_rate,
            output_rate,
            seconds,
        } => {
            duration(seconds)?;
            let source = TestSignal::new(SignalKind::Sine, input_rate, 1000.0, -24.0)?;
            let mut converted = RateConverter::new(source, input_rate, output_rate)?;
            let count = u64::from(output_rate) * u64::from(seconds);
            let mut square = 0.0;
            let mut peak = 0.0f32;
            let mut crossings = 0u64;
            let mut previous = 0.0;
            let mut frames = 0u64;
            let periods = [1, 127, 256, 441, 480, 511, 1024];
            let mut callbacks = 0usize;
            while frames < count {
                let period = periods[callbacks % periods.len()].min(count - frames);
                for _ in 0..period {
                    let frame = converted.next_frame();
                    square += f64::from(frame[0]).powi(2);
                    peak = peak.max(frame[0].abs());
                    if previous <= 0.0 && frame[0] > 0.0 {
                        crossings += 1;
                    }
                    previous = frame[0];
                }
                frames += period;
                callbacks += 1;
            }
            if converted.failed() {
                return Err("resampler failed".into());
            }
            emit(
                json!({"event":"simulation_complete","input_rate":input_rate,"output_rate":output_rate,
                "frames":frames,"callbacks":callbacks,"periods":periods,"peak":peak,"rms":(square / count as f64).sqrt(),
                "positive_zero_crossings":crossings,"resampler_delay_frames":converted.delay_frames()}),
            )?;
        }
        Command::Sink {
            room,
            seconds,
            channels,
        } => {
            duration(seconds)?;
            #[cfg(target_os = "linux")]
            {
                neonmix_linux::run_sink(&room, seconds, channels)?;
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (room, channels);
                return Err("sink creation requires a Linux PipeWire session".into());
            }
        }
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "{}",
                json!({"event":"error","version":env!("CARGO_PKG_VERSION"),"message":error.to_string()})
            );
            ExitCode::FAILURE
        }
    }
}
