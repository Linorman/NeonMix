//! Shared implementation over native CPAL hosts. OS-specific policy stays in adapters/.
use cpal::{
    Device, Host, SampleFormat, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use neonmix_core::{
    AudioError, AudioFormat, DeviceInfo, FormatInfo, StreamInfo,
    capture::CaptureBridge,
    clock::OutputClock,
    queue::{BlockConsumer, block_queue},
    resample::{RateConverter, stereo_to_mono},
    signal::{SignalKind, StereoSource, TestSignal},
    stats::AudioStats,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{
            AtomicBool, AtomicU64,
            Ordering::{Acquire, Relaxed, Release},
        },
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum CaptureKind {
    VirtualDevice,
    Loopback,
}
#[derive(Clone, Copy, Default)]
pub struct OpenOptions {
    pub sample_rate: Option<u32>,
    pub period_frames: Option<u32>,
}
pub struct NativeBackend {
    host: Host,
    name: &'static str,
    allow_loopback: bool,
    device_filter: fn(&Device) -> bool,
    capture_preflight: fn() -> Result<(), AudioError>,
}
impl NativeBackend {
    pub fn new(host: Host, name: &'static str, allow_loopback: bool) -> Self {
        Self {
            host,
            name,
            allow_loopback,
            device_filter: |_| true,
            capture_preflight: || Ok(()),
        }
    }
    pub fn with_device_filter(mut self, filter: fn(&Device) -> bool) -> Self {
        self.device_filter = filter;
        self
    }
    /// Platform permission checks run on the control thread before opening capture.
    pub fn with_capture_preflight(mut self, check: fn() -> Result<(), AudioError>) -> Self {
        self.capture_preflight = check;
        self
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn devices(&self) -> Result<Vec<DeviceInfo>, AudioError> {
        let mut devices = Vec::new();
        for device in self.host.devices().map_err(native_error)? {
            if !(self.device_filter)(&device) {
                continue;
            }
            let id = device.id().map_err(native_error)?.to_string();
            // PipeWire synthetic defaults are movable aliases, not stable device identities.
            if id.ends_with(":input_default")
                || id.ends_with(":output_default")
                || id.ends_with(":sink_default")
            {
                continue;
            }
            let input = device.default_input_config().ok().map(format_info);
            let output = device.default_output_config().ok().map(format_info);
            let supported_input = device
                .supported_input_configs()
                .map(|it| configs(it.collect()))
                .unwrap_or_default();
            let supported_output = device
                .supported_output_configs()
                .map(|it| configs(it.collect()))
                .unwrap_or_default();
            devices.push(DeviceInfo {
                id,
                name: device.description().map_err(native_error)?.name().into(),
                backend: self.name.into(),
                input,
                output,
                supported_input,
                supported_output,
            });
        }
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }
    fn find(&self, id: &str) -> Result<Device, AudioError> {
        // Enumerate concrete devices rather than creating a movable default alias.
        self.host
            .devices()
            .map_err(native_error)?
            .find(|d| (self.device_filter)(d) && d.id().is_ok_and(|v| v.to_string() == id))
            .filter(|_| {
                !id.ends_with(":input_default")
                    && !id.ends_with(":output_default")
                    && !id.ends_with(":sink_default")
            })
            .ok_or_else(|| AudioError::DeviceUnavailable(id.into()))
    }
    pub fn open_test_output(
        &self,
        id: &str,
        options: OpenOptions,
        kind: SignalKind,
        frequency: f64,
        gain_db: f32,
    ) -> Result<RunningOutput, AudioError> {
        let source = TestSignal::new(kind, 48_000, frequency, gain_db)?;
        self.open_output(id, options, source)
    }

    /// Pull 48 kHz stereo PCM from a preallocated source at the native device cadence.
    pub fn open_output<S: StereoSource + 'static>(
        &self,
        id: &str,
        options: OpenOptions,
        source: S,
    ) -> Result<RunningOutput, AudioError> {
        let device = self.find(id)?;
        let supported = select_config(&device, false, options)?;
        let format = AudioFormat::new(supported.sample_rate(), supported.channels())?;
        let mut source = RateConverter::new(source, 48_000, format.sample_rate)?;
        let delay = source.delay_frames();
        let stats = Arc::new(AudioStats::default());
        let control = Arc::new(StreamControl::default());
        let callback_stats = stats.clone();
        let error_stats = stats.clone();
        let callback_control = control.clone();
        let error_control = control.clone();
        let epoch = neonmix_core::epoch::reserve_epoch_after(0)
            .ok_or_else(|| AudioError::Backend("stream epoch space exhausted".into()))?;
        let mut clock = OutputClock::new(format.sample_rate, epoch);
        let (mut position_publisher, position_reader) = neonmix_core::position::position_channel();
        let channels = format.channel_layout.channels();
        let config = stream_config(supported, options);
        let start = Instant::now();
        let mut gain = 0.0f32;
        let mut poisoned = false;
        let stream = device
            .build_output_stream_raw(
                config,
                supported.sample_format(),
                move |data, info| {
                    let began = Instant::now();
                    let frames = data.len() / channels;
                    if poisoned {
                        zero_output(data);
                        callback_stats.callback_at_rate(frames, format.sample_rate, elapsed(began));
                        return;
                    }
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let target = if callback_control.muted.load(Acquire)
                            || callback_control.stopped.load(Acquire)
                            || poisoned
                        {
                            0.0
                        } else {
                            1.0
                        };
                        let stamp = info.timestamp();
                        let playback_lead = ns(stamp.playback).saturating_sub(ns(stamp.callback));
                        source.set_presentation_time(
                            began
                                .checked_add(Duration::from_nanos(playback_lead))
                                .unwrap_or(began),
                        );
                        let mut peak = 0.0f32;
                        let mut silent_frames = 0u64;
                        let mut next = || {
                            let frame = source.next_frame();
                            gain += (target - gain).clamp(-1.0 / 240.0, 1.0 / 240.0);
                            let frame = frame.map(|v| {
                                if v.is_finite() {
                                    (v * gain).clamp(-1.0, 1.0)
                                } else {
                                    0.0
                                }
                            });
                            peak = peak.max(frame[0].abs()).max(frame[1].abs());
                            if frame == [0.0; 2] {
                                silent_frames += 1;
                            }
                            frame
                        };
                        let ok = write_samples(data, channels, &mut next);
                        callback_stats
                            .peak_bits
                            .store(u64::from(peak.to_bits()), Relaxed);
                        callback_stats
                            .silent_frames
                            .fetch_add(silent_frames, Relaxed);
                        if !ok || source.failed() {
                            callback_stats.error(3);
                        }
                        let stamp = info.timestamp();
                        let position = clock.observe_native(
                            ns(stamp.callback),
                            ns(stamp.playback),
                            frames,
                            info.native_frame_position(),
                        );
                        position_publisher.publish(position);
                        if position.epoch_exhausted {
                            callback_stats.error(10);
                            zero_output(data);
                        }
                        callback_stats
                            .native_position_valid
                            .store(position.native_device_frame_position.is_some(), Relaxed);
                        if let Some(raw) = position.native_device_frame_position {
                            callback_stats.native_position.store(raw, Relaxed);
                        }
                        if let Some(relative) = position.native_stream_frames {
                            callback_stats.native_stream_frames.store(relative, Relaxed);
                        }
                        if let Some(internal) = position.native_internal_frames {
                            callback_stats
                                .native_internal_frames
                                .store(internal, Relaxed);
                        }
                        callback_stats
                            .last_device_ns
                            .store(position.device_timestamp_ns, Relaxed);
                        callback_stats
                            .submitted_frames
                            .store(position.submitted_frames, Relaxed);
                        callback_stats
                            .presented_frames
                            .store(position.presented_frames, Relaxed);
                        callback_stats.clock_epoch.store(position.epoch, Relaxed);
                        if position.reset {
                            callback_stats.discontinuities.fetch_add(1, Relaxed);
                        }
                        callback_stats
                            .last_arrival_ns
                            .store(elapsed(start), Relaxed);
                    }));
                    if result.is_err() {
                        poisoned = true;
                        callback_stats.error(4);
                        zero_output(data);
                    }
                    callback_stats.callback_at_rate(frames, format.sample_rate, elapsed(began));
                },
                move |error| {
                    // PipeWire emits Unconnected while intentionally destroying a stream.
                    // Only suppress teardown events; live removal still latches a fault.
                    let expected_teardown = error_control.closing.load(Acquire)
                        && matches!(
                            error.kind(),
                            cpal::ErrorKind::DeviceNotAvailable
                                | cpal::ErrorKind::StreamInvalidated
                        );
                    if !expected_teardown {
                        error_stats.error(error_code(&error));
                    }
                },
                Some(Duration::from_secs(3)),
            )
            .map_err(native_error)?;
        let actual_period_frames = stream.buffer_size().ok();
        let info = StreamInfo {
            device_id: id.into(),
            backend: self.name.into(),
            format,
            sample_type: supported.sample_format().to_string(),
            requested_period_frames: options.period_frames,
            actual_period_frames,
            stream_epoch: epoch,
            resampler_delay_frames: delay,
        };
        Ok(RunningOutput {
            position_reader,
            stream,
            info,
            stats,
            control,
        })
    }
    pub fn open_capture(
        &self,
        id: &str,
        kind: CaptureKind,
        options: OpenOptions,
        stream_id: u64,
    ) -> Result<RunningCapture, AudioError> {
        (self.capture_preflight)()?;
        let loopback = matches!(kind, CaptureKind::Loopback) || self.name == "wasapi";
        if loopback && !self.allow_loopback {
            return Err(AudioError::InvalidArgument("Core Audio capture requires the input side of a virtual device; process taps are not the virtual-device bridge".into()));
        }
        let device = self.find(id)?;
        let supported = select_config(&device, !loopback, options)?;
        let format = AudioFormat::new(supported.sample_rate(), supported.channels())?;
        let stats = Arc::new(AudioStats::default());
        let (producer, consumer) = block_queue(neonmix_core::CAPTURE_QUEUE_BLOCKS, stats.clone())?;
        let epoch = neonmix_core::epoch::reserve_epoch_after(0)
            .ok_or_else(|| AudioError::Backend("stream epoch space exhausted".into()))?;
        let mut bridge = CaptureBridge::new(stream_id, epoch, format, producer, stats.clone());
        let callback_stats = stats.clone();
        let error_stats = stats.clone();
        let control = Arc::new(StreamControl::default());
        let callback_control = control.clone();
        let error_control = control.clone();
        let mut generation = 0;
        let config = stream_config(supported, options);
        let start = Instant::now();
        let channels = format.channel_layout.channels();
        let mut poisoned = false;
        let stream = device
            .build_input_stream_raw(
                config,
                supported.sample_format(),
                move |data, info| {
                    let began = Instant::now();
                    let frames = data.len() / channels;
                    if poisoned || callback_control.stopped.load(Acquire) {
                        return;
                    }
                    if catch_unwind(AssertUnwindSafe(|| {
                        let current = callback_control.generation.load(Acquire);
                        if generation != current {
                            bridge.reset(format, neonmix_core::Discontinuity::RESUMED);
                            generation = current;
                        }
                        let muted = callback_control.muted.load(Acquire);
                        let capture_ns = ns(info.timestamp().capture);
                        let arrival = elapsed(start);
                        let ok = read_samples(
                            data,
                            channels,
                            |i, f| {
                                // This closure is used by the bulk dispatcher below, not one ingest per frame.
                                (i, if muted { [0.0; 2] } else { f })
                            },
                            &mut bridge,
                            capture_ns,
                            arrival,
                        );
                        if !ok {
                            callback_stats.error(3);
                        }
                    }))
                    .is_err()
                    {
                        poisoned = true;
                        callback_stats.error(4);
                    }
                    callback_stats.callback_at_rate(frames, format.sample_rate, elapsed(began));
                },
                move |error| {
                    // PipeWire emits Unconnected while intentionally destroying a stream.
                    // Only suppress teardown events; live removal still latches a fault.
                    let expected_teardown = error_control.closing.load(Acquire)
                        && matches!(
                            error.kind(),
                            cpal::ErrorKind::DeviceNotAvailable
                                | cpal::ErrorKind::StreamInvalidated
                        );
                    if !expected_teardown {
                        error_stats.error(error_code(&error));
                    }
                },
                Some(Duration::from_secs(3)),
            )
            .map_err(native_error)?;
        let info = StreamInfo {
            device_id: id.into(),
            backend: self.name.into(),
            format,
            sample_type: supported.sample_format().to_string(),
            requested_period_frames: options.period_frames,
            actual_period_frames: stream.buffer_size().ok(),
            stream_epoch: epoch,
            resampler_delay_frames: 0,
        };
        Ok(RunningCapture {
            stream,
            info,
            stats,
            control,
            consumer,
            start,
        })
    }
}
fn configs(ranges: Vec<cpal::SupportedStreamConfigRange>) -> Vec<FormatInfo> {
    let mut configs = Vec::new();
    for range in ranges {
        for rate in [44_100, 48_000, 96_000] {
            if range.min_sample_rate() <= rate && range.max_sample_rate() >= rate {
                configs.push(format_info(range.with_sample_rate(rate)));
            }
        }
    }
    configs
}
fn format_info(config: SupportedStreamConfig) -> FormatInfo {
    let (min, max) = match config.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => (Some(*min), Some(*max)),
        _ => (None, None),
    };
    FormatInfo {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
        sample_type: config.sample_format().to_string(),
        period_min_frames: min,
        period_max_frames: max,
    }
}
fn select_config(
    device: &Device,
    input: bool,
    options: OpenOptions,
) -> Result<SupportedStreamConfig, AudioError> {
    if options.period_frames.is_some_and(|n| n == 0 || n > 8192) {
        return Err(AudioError::InvalidArgument(
            "period must be 1..8192 frames".into(),
        ));
    }
    let default = if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .map_err(native_error)?;
    let selected = if let Some(rate) = options.sample_rate {
        let ranges: Vec<_> = if input {
            device
                .supported_input_configs()
                .map_err(native_error)?
                .collect()
        } else {
            device
                .supported_output_configs()
                .map_err(native_error)?
                .collect()
        };
        ranges
            .into_iter()
            .filter(|r| {
                r.channels() == default.channels()
                    && r.min_sample_rate() <= rate
                    && r.max_sample_rate() >= rate
            })
            .find(|r| r.sample_format() == default.sample_format())
            .map(|r| r.with_sample_rate(rate))
            .ok_or_else(|| {
                AudioError::UnsupportedFormat(format!(
                    "device does not support requested rate {rate}"
                ))
            })?
    } else {
        default
    };
    AudioFormat::new(selected.sample_rate(), selected.channels())?;
    if !matches!(
        selected.sample_format(),
        SampleFormat::F32
            | SampleFormat::F64
            | SampleFormat::I16
            | SampleFormat::I32
            | SampleFormat::U16
    ) {
        return Err(AudioError::UnsupportedFormat(format!(
            "sample representation {}",
            selected.sample_format()
        )));
    }
    if let (Some(requested), cpal::SupportedBufferSize::Range { min, max }) =
        (options.period_frames, selected.buffer_size())
        && (requested < *min || requested > *max)
    {
        return Err(AudioError::UnsupportedFormat(format!(
            "period {requested} outside {min}..{max}"
        )));
    }
    Ok(selected)
}
fn stream_config(supported: SupportedStreamConfig, options: OpenOptions) -> cpal::StreamConfig {
    cpal::StreamConfig {
        buffer_size: options
            .period_frames
            .map_or(cpal::BufferSize::Default, cpal::BufferSize::Fixed),
        ..supported.config()
    }
}
fn native_error(error: cpal::Error) -> AudioError {
    AudioError::Backend(error.to_string())
}
fn ns(value: cpal::StreamInstant) -> u64 {
    value.as_nanos().min(u128::from(u64::MAX)) as u64
}
fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
fn error_code(error: &cpal::Error) -> u64 {
    match error.kind() {
        cpal::ErrorKind::DeviceNotAvailable => 1,
        cpal::ErrorKind::UnsupportedConfig => 3,
        cpal::ErrorKind::Xrun => 5,
        cpal::ErrorKind::DeviceChanged => 6,
        cpal::ErrorKind::StreamInvalidated => 7,
        cpal::ErrorKind::PermissionDenied => 8,
        cpal::ErrorKind::RealtimeDenied => 9,
        _ => 2,
    }
}
fn write_samples(
    data: &mut cpal::Data,
    channels: usize,
    next: &mut impl FnMut() -> [f32; 2],
) -> bool {
    macro_rules! write {
        ($ty:ty, $convert:expr) => {{
            let Some(samples) = data.as_slice_mut::<$ty>() else {
                return false;
            };
            for frame in samples.chunks_exact_mut(channels) {
                let input = next();
                if channels == 1 {
                    frame[0] = ($convert)(stereo_to_mono(input));
                } else {
                    frame[0] = ($convert)(input[0]);
                    frame[1] = ($convert)(input[1]);
                }
            }
            true
        }};
    }
    match data.sample_format() {
        SampleFormat::F32 => write!(f32, |s: f32| s),
        SampleFormat::F64 => write!(f64, f64::from),
        SampleFormat::I16 => write!(i16, |s: f32| (s * 32767.0) as i16),
        SampleFormat::I32 => write!(i32, |s: f32| (f64::from(s) * 2147483647.0) as i32),
        SampleFormat::U16 => write!(u16, |s: f32| ((s + 1.0) * 32767.5).round() as u16),
        _ => false,
    }
}
fn zero_output(data: &mut cpal::Data) {
    let _ = write_samples(data, 1, &mut || [0.0; 2]);
}
fn read_samples(
    data: &cpal::Data,
    channels: usize,
    mut transform: impl FnMut(usize, [f32; 2]) -> (usize, [f32; 2]),
    bridge: &mut CaptureBridge,
    capture_ns: u64,
    arrival_ns: u64,
) -> bool {
    macro_rules! read {
        ($ty:ty, $convert:expr) => {{
            let Some(samples) = data.as_slice::<$ty>() else {
                return false;
            };
            bridge.ingest(samples.len() / channels, capture_ns, arrival_ns, |i| {
                let left = ($convert)(samples[i * channels]);
                let right = if channels == 1 {
                    left
                } else {
                    ($convert)(samples[i * channels + 1])
                };
                transform(i, [left, right]).1
            });
            true
        }};
    }
    match data.sample_format() {
        SampleFormat::F32 => read!(f32, |s: f32| s),
        SampleFormat::F64 => read!(f64, |s: f64| s as f32),
        SampleFormat::I16 => read!(i16, |s: i16| f32::from(s) / 32768.0),
        SampleFormat::I32 => read!(i32, |s: i32| (f64::from(s) / 2147483648.0) as f32),
        SampleFormat::U16 => read!(u16, |s: u16| (f32::from(s) - 32768.0) / 32768.0),
        _ => false,
    }
}
#[derive(Default)]
pub struct StreamControl {
    closing: AtomicBool,
    muted: AtomicBool,
    stopped: AtomicBool,
    generation: AtomicU64,
}
impl StreamControl {
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Release);
    }
    pub fn stop(&self) {
        self.stopped.store(true, Release);
    }
}
pub struct RunningOutput {
    // Drop the native stream first, keeping the reader's Arc alive through callback teardown.
    stream: cpal::Stream,
    position_reader: neonmix_core::position::PositionReader,
    pub info: StreamInfo,
    pub stats: Arc<AudioStats>,
    pub control: Arc<StreamControl>,
}
impl RunningOutput {
    pub fn position_reader(&self) -> neonmix_core::position::PositionReader {
        self.position_reader.clone()
    }
    pub fn latest_position(&self) -> Option<neonmix_core::clock::OutputPosition> {
        self.position_reader.latest()
    }
    pub fn play(&self) -> Result<(), AudioError> {
        self.stream.play().map_err(native_error)
    }
}
pub struct RunningCapture {
    stream: cpal::Stream,
    pub info: StreamInfo,
    pub stats: Arc<AudioStats>,
    pub control: Arc<StreamControl>,
    consumer: BlockConsumer,
    start: Instant,
}
impl RunningCapture {
    pub fn play(&self) -> Result<(), AudioError> {
        self.stream.play().map_err(native_error)
    }
    pub fn pause(&self) -> Result<(), AudioError> {
        self.stream.pause().map_err(native_error)
    }
    pub fn resume(&self) -> Result<(), AudioError> {
        self.control.generation.fetch_add(1, Release);
        self.stream.play().map_err(native_error)
    }
    pub fn pop(&mut self) -> Option<neonmix_core::AudioBlock> {
        self.consumer.pop_fresh(elapsed(self.start), 100_000_000)
    }
}

impl Drop for RunningOutput {
    fn drop(&mut self) {
        self.control.closing.store(true, Release);
    }
}
impl Drop for RunningCapture {
    fn drop(&mut self) {
        self.control.closing.store(true, Release);
    }
}
