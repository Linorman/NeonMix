use crate::{
    MediaError,
    protocol::{self, Feedback, PacketStats, RtpTimeline, SendPolicy},
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::{AppSink, AppSrc};
use gstreamer_rtp::RTPBuffer;
use neonmix_control::Session;
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity, MAX_BLOCK_FRAMES, queue::BlockProducer,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{
            AtomicBool,
            Ordering::{Acquire, Release},
        },
    },
    time::Instant,
};

fn native(error: impl std::fmt::Display) -> MediaError {
    MediaError::Native(error.to_string())
}
pub fn certificate_fingerprint(pem: &str) -> Result<String, MediaError> {
    let cert = rustls_pemfile::certs(&mut Cursor::new(pem.as_bytes()))
        .next()
        .ok_or(MediaError::CertificateMismatch)?
        .map_err(native)?;
    Ok(Sha256::digest(cert.as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub fn runtime_probe() -> Result<serde_runtime::Runtime, MediaError> {
    gst::init().map_err(native)?;
    let names = [
        "appsrc",
        "appsink",
        "opusenc",
        "opusdec",
        "rtpopuspay",
        "rtpopusdepay",
        "rtpjitterbuffer",
        "dtlssrtpenc",
        "dtlssrtpdec",
        "srtpenc",
        "srtpdec",
    ];
    let mut plugins = Vec::new();
    for name in names {
        let factory = gst::ElementFactory::find(name)
            .ok_or_else(|| native(format!("missing GStreamer element {name}")))?;
        let plugin = factory.plugin().ok_or_else(|| native("missing plugin"))?;
        plugins.push((
            name.to_string(),
            plugin.version().to_string(),
            plugin.license().to_string(),
        ));
    }
    Ok(serde_runtime::Runtime {
        gstreamer: gst::version_string().to_string(),
        plugins,
    })
}
mod serde_runtime {
    #[derive(serde::Serialize)]
    pub struct Runtime {
        pub gstreamer: String,
        pub plugins: Vec<(String, String, String)>,
    }
}

/// Shared authorization gate is checked before depayload/decode and again
/// before handing PCM to the mixer. Revocation cannot be undone by DTLS notify.
#[derive(Clone)]
pub struct MediaGate {
    born: Instant,
    ttl: std::time::Duration,
    enabled: Arc<AtomicBool>,
    verified: Arc<AtomicBool>,
    mismatch: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
}
impl MediaGate {
    pub fn revoke(&self) {
        self.enabled.store(false, Release);
    }
    pub fn authorized(&self) -> bool {
        self.ready.load(Acquire)
            && self.enabled.load(Acquire)
            && self.born.elapsed() < self.ttl
            && self.verified.load(Acquire)
            && !self.mismatch.load(Acquire)
    }
}
struct Endpoint {
    scheduling: Arc<crate::scheduling::Scheduling>,
    pipeline: gst::Pipeline,
    wire_in: AppSrc,
    wire_out: AppSink,
    rtcp_in: AppSrc,
    rtcp_out: AppSink,
    gate: MediaGate,
}
impl Endpoint {
    fn new(
        session: &Session,
        pem: &str,
        expected_fingerprint: &str,
        client: bool,
    ) -> Result<Self, MediaError> {
        gst::init().map_err(native)?;
        session.offer.validate().map_err(native)?;
        if !session.status.active() {
            return Err(MediaError::Inactive);
        }
        if session.media_ttl_seconds == 0
            || session.media_ttl_seconds > neonmix_control::MEDIA_TTL_SECONDS
        {
            return Err(MediaError::Inactive);
        }
        // Different local IDs even when both endpoints run in one process.
        let connection = format!("{}-{}", session.media_context, uuid::Uuid::new_v4());
        let pipeline = gst::Pipeline::new();
        let scheduling = crate::scheduling::Scheduling::install(&pipeline);
        let wire_in = gst::ElementFactory::make("appsrc")
            .name("wire_in")
            .property("is-live", true)
            .property("format", gst::Format::Time)
            .property("do-timestamp", true)
            .property("block", false)
            .property("max-buffers", 32u64)
            .property("max-bytes", 131072u64)
            .property("leaky-type", gstreamer_app::AppLeakyType::Downstream)
            .build()
            .map_err(native)?
            .downcast::<AppSrc>()
            .map_err(|_| native("appsrc type"))?;
        // Decoder must be constructed before its paired encoder (plugin contract).
        let dec = gst::ElementFactory::make("dtlssrtpdec")
            .name("secure_in")
            .property("connection-id", &connection)
            .property("pem", pem)
            .build()
            .map_err(native)?;
        let enc = gst::ElementFactory::make("dtlssrtpenc")
            .name("secure_out")
            .property("connection-id", &connection)
            .property("is-client", client)
            .build()
            .map_err(native)?;
        let wire_out = make_sink("wire_out", 8)?;
        let rtcp_out = make_sink("rtcp_out", 8)?;
        let rtcp_in = gst::ElementFactory::make("appsrc")
            .name("rtcp_in")
            .property("is-live", true)
            .property("format", gst::Format::Time)
            .property("do-timestamp", true)
            .property("block", false)
            .property("max-buffers", 8u64)
            .property("max-bytes", 9600u64)
            .property("leaky-type", gstreamer_app::AppLeakyType::Downstream)
            .build()
            .map_err(native)?
            .downcast::<AppSrc>()
            .map_err(|_| native("appsrc type"))?;
        rtcp_in.set_caps(Some(&gst::Caps::builder("application/x-rtcp").build()));
        pipeline
            .add_many([
                wire_in.upcast_ref(),
                &dec,
                &enc,
                wire_out.upcast_ref(),
                rtcp_in.upcast_ref(),
                rtcp_out.upcast_ref(),
            ])
            .map_err(native)?;
        wire_in.link(&dec).map_err(native)?;
        enc.link(&wire_out).map_err(native)?;
        dec.link_pads(Some("rtcp_src"), &rtcp_out, Some("sink"))
            .map_err(native)?;
        rtcp_in
            .link_pads(Some("src"), &enc, Some("rtcp_sink_0"))
            .map_err(native)?;
        let gate = MediaGate {
            born: Instant::now(),
            ttl: std::time::Duration::from_secs(u64::from(session.media_ttl_seconds)),
            enabled: Arc::new(AtomicBool::new(true)),
            verified: Arc::new(AtomicBool::new(false)),
            mismatch: Arc::new(AtomicBool::new(false)),
            ready: Arc::new(AtomicBool::new(false)),
        };
        let notify_gate = gate.clone();
        let fingerprint = expected_fingerprint.to_owned();
        dec.connect_notify(Some("peer-pem"), move |element, _| {
            if let Some(pem) = element.property::<Option<String>>("peer-pem") {
                match certificate_fingerprint(&pem) {
                    Ok(actual) if actual == fingerprint => {
                        notify_gate.verified.store(true, Release)
                    }
                    _ => {
                        notify_gate.mismatch.store(true, Release);
                        notify_gate.verified.store(false, Release);
                    }
                }
            }
        });
        let ready_gate = gate.clone();
        dec.connect_notify(Some("connection-state"), move |element, _| {
            let state = element.property_value("connection-state");
            let connected = gst::glib::EnumValue::from_value(&state)
                .is_some_and(|(_, value)| value.nick() == "connected");
            let was_ready = ready_gate
                .ready
                .swap(connected, std::sync::atomic::Ordering::AcqRel);
            let failed = gst::glib::EnumValue::from_value(&state)
                .is_some_and(|(_, value)| value.nick() == "failed");
            if failed || (was_ready && !connected) {
                ready_gate.revoke();
            }
        });
        Ok(Self {
            scheduling,
            pipeline,
            wire_in,
            wire_out,
            rtcp_in,
            rtcp_out,
            gate,
        })
    }
    fn ingest(&self, bytes: &[u8]) -> Result<(), MediaError> {
        if !self.gate.enabled.load(Acquire) || self.gate.born.elapsed() >= self.gate.ttl {
            return Err(MediaError::Inactive);
        }
        protocol::validate_datagram(bytes)?;
        self.wire_in
            .push_buffer(gst::Buffer::from_mut_slice(bytes.to_vec()))
            .map_err(native)?;
        Ok(())
    }
    fn outgoing(&self) -> Vec<Vec<u8>> {
        drain_bytes(&self.wire_out, 32)
    }
    fn check(&self) -> Result<(), MediaError> {
        if self.gate.born.elapsed() >= self.gate.ttl {
            return Err(MediaError::Inactive);
        }
        if !self.gate.ready.load(Acquire)
            && self.gate.born.elapsed() >= std::time::Duration::from_secs(5)
        {
            return Err(MediaError::HandshakeTimeout);
        }
        if self.gate.mismatch.load(Acquire) {
            return Err(MediaError::CertificateMismatch);
        }
        if !self.gate.enabled.load(Acquire) {
            return Err(MediaError::Inactive);
        }
        if let Some(bus) = self.pipeline.bus() {
            for _ in 0..32 {
                let Some(message) = bus.pop() else {
                    break;
                };
                if let gst::MessageView::Error(e) = message.view() {
                    return Err(native(e.error()));
                }
            }
        }
        Ok(())
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.gate.revoke();
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
fn make_sink(name: &str, capacity: u32) -> Result<AppSink, MediaError> {
    gst::ElementFactory::make("appsink")
        .name(name)
        .property("sync", false)
        .property("async", false)
        .property("max-buffers", capacity)
        .property("drop", true)
        .property("enable-last-sample", false)
        .property("wait-on-eos", false)
        .build()
        .map_err(native)?
        .downcast::<AppSink>()
        .map_err(|_| native("appsink type"))
}
fn drain_bytes(sink: &AppSink, bound: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for _ in 0..bound {
        let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) else {
            break;
        };
        if let Some(buffer) = sample.buffer()
            && let Ok(map) = buffer.map_readable()
        {
            out.push(map.as_slice().to_vec());
        }
    }
    out
}

fn notify_samples(sink: &AppSink, wake: Arc<dyn Fn() + Send + Sync>) {
    sink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |_| {
                // Do not pull, decode or wait on the application here. Ownership of
                // samples remains with the bounded sink and its media worker.
                wake();
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
}

#[derive(Default, Clone, Serialize)]
pub struct ReceiveStats {
    pub native_scheduling: crate::SchedulingSnapshot,
    pub wire_to_authenticated_max_ns: u64,
    pub authenticated_to_jitter_max_ns: u64,
    pub pipeline_clock_ns: Option<u64>,
    pub pipeline_base_time_ns: Option<u64>,
    pub last_pcm_pts_ns: u64,
    pub last_source_position: u64,
    pub jitter_to_pcm_max_ns: u64,
    pub last_buffer_peak: f32,
    pub last_buffer_rms: f64,
    pub pcm_timing_gaps: Vec<(u64, u64, u32)>,
    pub pcm_timing_gap_count: u64,
    pub last_pcm_timing_gap: Option<(u64, u64, u32)>,
    pub authenticated: bool,
    pub packets: PacketStats,
    pub lost_packets: u64,
    pub late_packets: u64,
    pub plc_samples: u64,
    pub retimed_loss_events: u64,
    pub overflow_plc_packets: u64,
    pub last_loss_duration_ns: u64,
    pub pcm_frames: u64,
    pub pcm_peak: f32,
    pub silent_pcm_frames: u64,
    pub queue_drops: u64,
    pub pcm_sink_dropped: u64,
}
pub struct Receiver {
    pcm_queue_trace: Arc<crate::queue_trace::QueueTrace>,
    wire_to_authenticated_max_ns: Arc<std::sync::atomic::AtomicU64>,
    authenticated_to_jitter_max_ns: Arc<std::sync::atomic::AtomicU64>,
    last_pcm_pts_ns: u64,
    jitter_to_pcm_max_ns: u64,
    last_buffer_peak: f32,
    last_buffer_rms: f64,
    source_anchors: Arc<Mutex<SourceAnchors>>,
    pcm_timing_gaps: Vec<(u64, u64, u32)>,
    pcm_timing_gap_count: u64,
    last_pcm_timing_gap: Option<(u64, u64, u32)>,
    retimed_loss_events: Arc<std::sync::atomic::AtomicU64>,
    overflow_plc_packets: Arc<std::sync::atomic::AtomicU64>,
    last_loss_duration_ns: Arc<std::sync::atomic::AtomicU64>,
    endpoint: Endpoint,
    pcm: AppSink,
    jitter: gst::Element,
    decoder: gst::Element,
    timeline: Arc<Mutex<RtpTimeline>>,
    session: Session,
    next_position: u64,
    origin: Instant,
    pcm_frames: u64,
    pump_exhausted: bool,
    pcm_peak: f32,
    silent_pcm_frames: u64,
    queue_drops: u64,
    last_lost: u64,
    last_received: u64,
}
struct SourceAnchors {
    first: Option<u64>,
    highest: u64,
    pts_origin_ns: Option<u64>,
    next_position: u64,
    points: VecDeque<(u64, u64, u64)>,
}
impl SourceAnchors {
    fn observe(&mut self, timestamp: u32, pts: u64, now: u64) -> (u64, Option<(u64, u64)>) {
        let extended = if self.first.is_some() {
            protocol::extend(u64::from(timestamp), self.highest, 32)
        } else {
            u64::from(timestamp)
        };
        self.highest = self.highest.max(extended);
        let first = *self.first.get_or_insert(extended);
        if self.points.len() == 128 {
            self.points.pop_front();
        }
        let position = extended.saturating_sub(first);
        let origin = *self.pts_origin_ns.get_or_insert(pts);
        let missing = (position > self.next_position).then(|| {
            (
                origin.saturating_add(neonmix_core::clock::frames_to_ns(self.next_position, 48000)),
                position - self.next_position,
            )
        });
        let normalized = origin.saturating_add(neonmix_core::clock::frames_to_ns(position, 48000));
        self.points.push_back((normalized, position, now));
        self.next_position = position.saturating_add(480);
        (normalized, missing)
    }
    fn position(&self, pts: u64) -> Option<u64> {
        // Real decoded packets retain their PTS anchor. PLC buffers have no RTP
        // anchor: advance by the actual decoded frame count, not an extrapolated
        // receiver-clock PTS whose skew correction may differ by a few samples.
        self.points
            .iter()
            .rev()
            .find(|(p, _, _)| p.abs_diff(pts) <= 1000)
            .map(|(_, source, _)| *source)
    }
    fn arrival(&self, pts: u64) -> Option<u64> {
        self.points
            .iter()
            .rev()
            .find(|(p, _, _)| p.abs_diff(pts) <= 1000)
            .map(|(_, _, time)| *time)
    }
}
impl Receiver {
    pub fn notify_ready(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        notify_samples(&self.pcm, wake.clone());
        notify_samples(&self.endpoint.wire_out, wake);
    }
    pub fn new(session: Session, pem: &str, origin: Instant) -> Result<Self, MediaError> {
        let endpoint = Endpoint::new(&session, pem, &session.offer.certificate_sha256, false)?;
        let jitter = gst::ElementFactory::make("rtpjitterbuffer")
            .name("jitter")
            .property("latency", 40u32)
            .property("drop-on-latency", true)
            .property("do-lost", true)
            .property("do-retransmission", false)
            .property_from_str("mode", "slave")
            .build()
            .map_err(native)?;
        let filter = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("application/x-rtp")
                    .field("media", "audio")
                    .field("encoding-name", "OPUS")
                    .field("clock-rate", 48000i32)
                    .field("payload", 96i32)
                    .build(),
            )
            .build()
            .map_err(native)?;
        let depay = gst::ElementFactory::make("rtpopusdepay")
            .build()
            .map_err(native)?;
        let decoder = gst::ElementFactory::make("opusdec")
            .name("decoder")
            .property("plc", true)
            .property("use-inband-fec", false)
            .build()
            .map_err(native)?;
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(native)?;
        let pcm = make_sink("pcm", 8)?;
        // The jitterbuffer owns packet deadlines. Deliver decoded PCM promptly;
        // waiting at this sink blocks its upstream streaming thread and can
        // overflow the jitterbuffer despite a healthy Rust consumer. The
        // hardware-clock-driven Mixer owns presentation and drift correction.
        pcm.set_property("sync", false);
        pcm.set_property("processing-deadline", 0u64);
        // Independent limits bound both accumulated time and variable PLC buffers.
        pcm.set_property("max-time", 80_000_000u64);
        pcm.set_property("max-bytes", 8u64 * 5760 * 8);
        pcm.set_caps(Some(
            &gst::Caps::builder("audio/x-raw")
                .field("format", "F32LE")
                .field("rate", 48000i32)
                .field("channels", 2i32)
                .field("layout", "interleaved")
                .build(),
        ));
        let pcm_queue_trace = Arc::new(crate::queue_trace::QueueTrace::new());
        let tagging = pcm_queue_trace.clone();
        let gate = endpoint.gate.clone();
        pcm.static_pad("sink")
            .ok_or_else(|| native("PCM sink pad"))?
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                let Some(buffer) = info.buffer_mut() else {
                    return gst::PadProbeReturn::Drop;
                };
                if tagging.tag(buffer.make_mut()).is_err() {
                    gate.revoke();
                    return gst::PadProbeReturn::Drop;
                }
                gst::PadProbeReturn::Ok
            });
        endpoint
            .pipeline
            .add_many([
                &filter,
                &jitter,
                &depay,
                &decoder,
                &convert,
                pcm.upcast_ref(),
            ])
            .map_err(native)?;
        let dec = endpoint
            .pipeline
            .by_name("secure_in")
            .ok_or_else(|| native("secure_in"))?;
        dec.link_pads(Some("rtp_src"), &filter, Some("sink"))
            .map_err(native)?;
        gst::Element::link_many([
            &filter,
            &jitter,
            &depay,
            &decoder,
            &convert,
            pcm.upcast_ref(),
        ])
        .map_err(native)?;
        let source_anchors = Arc::new(Mutex::new(SourceAnchors {
            first: None,
            highest: 0,
            pts_origin_ns: None,
            next_position: 0,
            points: VecDeque::with_capacity(128),
        }));
        let anchors = source_anchors.clone();
        let authenticated_arrivals =
            Arc::new(Mutex::new(VecDeque::<(u32, u64)>::with_capacity(128)));
        let arrivals = authenticated_arrivals.clone();
        let wire_to_authenticated_max_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let authenticated_to_jitter_max_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let retimed_loss_events = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let last_loss_duration_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let overflow_plc_packets = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let overflow_packets = overflow_plc_packets.clone();
        let loss_anchors = source_anchors.clone();
        let retimed_events = retimed_loss_events.clone();
        let last_loss = last_loss_duration_ns.clone();
        jitter
            .static_pad("src")
            .ok_or_else(|| native("jitter src"))?
            .add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                if let Some(event) = info.event_mut()
                    && let Some(structure) = event.structure()
                    && structure.name() == "GstRTPPacketLost"
                {
                    if structure.get::<bool>("neonmix-retimed").unwrap_or(false) {
                        return gst::PadProbeReturn::Ok;
                    }
                    // The offer fixes one Opus RTP packet at 480 frames/10 ms.
                    // Jitterbuffer estimates missing duration from arrival PTS;
                    // that estimate can be 2.5 ms for one missing 10 ms packet.
                    // Keep aggregate loss events but align them to packet time.
                    let original = structure.get::<u64>("duration").unwrap_or(0);
                    last_loss.store(original, std::sync::atomic::Ordering::Relaxed);
                    let packets = original.saturating_add(5_000_000) / 10_000_000;
                    let duration = packets.max(1).saturating_mul(10_000_000);
                    let timestamp = if let Ok(mut anchors) = loss_anchors.lock() {
                        let timestamp = anchors.pts_origin_ns.map(|origin| {
                            origin.saturating_add(neonmix_core::clock::frames_to_ns(
                                anchors.next_position,
                                48000,
                            ))
                        });
                        anchors.next_position =
                            anchors.next_position.saturating_add(packets.max(1) * 480);
                        timestamp
                    } else {
                        return gst::PadProbeReturn::Drop;
                    };
                    let structure = event.make_mut().structure_mut();
                    structure.set("duration", duration);
                    if let Some(timestamp) = timestamp {
                        structure.set("timestamp", timestamp);
                    }
                    if duration != original {
                        retimed_events.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                gst::PadProbeReturn::Ok
            });
        let jitter_time = authenticated_to_jitter_max_ns.clone();
        jitter
            .static_pad("src")
            .ok_or_else(|| native("jitter src"))?
            .add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
                let Some(buffer) = info.buffer_mut() else {
                    return gst::PadProbeReturn::Drop;
                };
                let Ok(rtp) = RTPBuffer::from_buffer_readable(buffer) else {
                    return gst::PadProbeReturn::Drop;
                };
                let timestamp = rtp.timestamp();
                let sequence = rtp.seq();
                drop(rtp);
                let Some(pts) = buffer.pts() else {
                    return gst::PadProbeReturn::Drop;
                };
                let now = origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                if let Some(arrival) = arrivals.lock().ok().and_then(|a| {
                    a.iter()
                        .rev()
                        .find(|(ts, _)| *ts == timestamp)
                        .map(|(_, time)| *time)
                }) {
                    jitter_time.fetch_max(
                        now.saturating_sub(arrival),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                let (normalized, missing) = match anchors.lock() {
                    Ok(mut anchors) => anchors.observe(timestamp, pts.nseconds(), now),
                    Err(_) => return gst::PadProbeReturn::Drop,
                };
                // Decoder PTS uses exact source samples. Receiver-clock skew must
                // not make PLC overlap the next packet or shift the PCM anchor.
                buffer
                    .make_mut()
                    .set_pts(gst::ClockTime::from_nseconds(normalized));
                // drop-on-latency can evict queued packets without emitting a
                // GstRTPPacketLost event. Conceal only a bounded missing interval
                // and never duplicate PLC already accounted by native loss events.
                if let Some((timestamp, frames)) = missing {
                    let packets = frames / 480;
                    if frames <= 5760 && frames.is_multiple_of(480) {
                        let event = gst::event::CustomDownstream::new(
                            gst::Structure::builder("GstRTPPacketLost")
                                .field("seqnum", u32::from(sequence.wrapping_sub(packets as u16)))
                                .field("timestamp", timestamp)
                                .field("duration", neonmix_core::clock::frames_to_ns(frames, 48000))
                                .field("retry", 0u32)
                                .field("neonmix-retimed", true)
                                .build(),
                        );
                        if !pad.push_event(event) {
                            return gst::PadProbeReturn::Drop;
                        }
                        overflow_packets.fetch_add(packets, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                gst::PadProbeReturn::Ok
            });
        let gate = endpoint.gate.clone();
        let ssrc = session.offer.ssrc;
        let timeline = Arc::new(Mutex::new(RtpTimeline::default()));
        let tracked = timeline.clone();
        let wire_time = wire_to_authenticated_max_ns.clone();
        let pipeline = endpoint.pipeline.downgrade();
        dec.static_pad("rtp_src")
            .ok_or_else(|| native("rtp_src"))?
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if !gate.authorized() {
                    return gst::PadProbeReturn::Drop;
                }
                let Some(buffer) = info.buffer() else {
                    return gst::PadProbeReturn::Drop;
                };
                let Ok(rtp) = RTPBuffer::from_buffer_readable(buffer) else {
                    return gst::PadProbeReturn::Drop;
                };
                let Ok(mut t) = tracked.lock() else {
                    return gst::PadProbeReturn::Drop;
                };
                if rtp.ssrc() != ssrc
                    || rtp.payload_type() != 96
                    || rtp.payload().ok().and_then(protocol::opus_packet_frames) != Some(480)
                {
                    t.stats.invalid += 1;
                    return gst::PadProbeReturn::Drop;
                }
                if !t.observe(rtp.seq(), rtp.timestamp()) {
                    return gst::PadProbeReturn::Drop;
                }
                let now = origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                if let Ok(mut arrivals) = authenticated_arrivals.lock() {
                    if arrivals.len() == 128 {
                        arrivals.pop_front();
                    }
                    arrivals.push_back((rtp.timestamp(), now));
                }
                if let Some(pipeline) = pipeline.upgrade()
                    && let Some(running) = pipeline.current_running_time()
                    && let Some(dts) = buffer.dts()
                {
                    wire_time.fetch_max(
                        running.nseconds().saturating_sub(dts.nseconds()),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                gst::PadProbeReturn::Ok
            });
        endpoint
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(native)?;
        Ok(Self {
            pcm_queue_trace,
            wire_to_authenticated_max_ns,
            authenticated_to_jitter_max_ns,
            last_pcm_pts_ns: 0,
            jitter_to_pcm_max_ns: 0,
            last_buffer_peak: 0.0,
            last_buffer_rms: 0.0,
            pcm_timing_gaps: Vec::with_capacity(8),
            pcm_timing_gap_count: 0,
            last_pcm_timing_gap: None,
            retimed_loss_events,
            overflow_plc_packets,
            last_loss_duration_ns,
            source_anchors,
            endpoint,
            pcm,
            jitter,
            decoder,
            timeline,
            session,
            next_position: 0,
            origin,
            pcm_frames: 0,
            pump_exhausted: false,
            pcm_peak: 0.0,
            silent_pcm_frames: 0,
            queue_drops: 0,
            last_lost: 0,
            last_received: 0,
        })
    }
    pub fn gate(&self) -> MediaGate {
        self.endpoint.gate.clone()
    }
    pub fn ingest(&self, bytes: &[u8]) -> Result<(), MediaError> {
        self.endpoint.ingest(bytes)
    }
    pub fn outgoing(&self) -> Vec<Vec<u8>> {
        self.endpoint.outgoing()
    }
    pub fn check(&self) -> Result<(), MediaError> {
        self.endpoint.check()
    }
    pub fn pump_budget_exhausted(&self) -> bool {
        self.pump_exhausted
    }
    pub fn pump_pcm(&mut self, producer: &mut BlockProducer) -> Result<usize, MediaError> {
        self.check()?;
        let mut count = 0;
        self.pump_exhausted = true;
        for _ in 0..4 {
            let Some(sample) = self.pcm.try_pull_sample(gst::ClockTime::ZERO) else {
                self.pump_exhausted = false;
                break;
            };
            if !self.endpoint.gate.authorized() {
                continue;
            }
            let buffer = sample.buffer().ok_or(MediaError::InvalidPacket)?;
            self.pcm_queue_trace.observe(buffer)?;
            let map = buffer.map_readable().map_err(native)?;
            if map.len() % 8 != 0 || map.len() > 5760 * 8 {
                return Err(MediaError::InvalidPacket);
            }
            let pts = buffer.pts().ok_or(MediaError::InvalidPacket)?.nseconds();
            self.last_pcm_pts_ns = pts;
            let now = self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            if let Some(arrival) = self.source_anchors.lock().ok().and_then(|a| a.arrival(pts)) {
                self.jitter_to_pcm_max_ns =
                    self.jitter_to_pcm_max_ns.max(now.saturating_sub(arrival));
            }
            self.last_buffer_peak = 0.0;
            let mut energy = 0.0f64;
            let mapped = self
                .source_anchors
                .lock()
                .ok()
                .and_then(|anchors| anchors.position(pts))
                .unwrap_or(self.next_position);
            let base = if mapped.abs_diff(self.next_position) <= 1 {
                self.next_position
            } else {
                mapped
            };
            for (offset, samples) in map.as_slice().chunks(MAX_BLOCK_FRAMES * 8).enumerate() {
                let position = base + (offset * MAX_BLOCK_FRAMES) as u64;
                if self.pcm_frames > 0 && position != self.next_position {
                    let gap = (self.next_position, position, buffer.flags().bits());
                    self.pcm_timing_gap_count += 1;
                    self.last_pcm_timing_gap = Some(gap);
                    if self.pcm_timing_gaps.len() < 8 {
                        self.pcm_timing_gaps.push(gap);
                    }
                }
                let mut block = AudioBlock::empty(self.session.stream_id, AudioFormat::INTERNAL);
                block.header.stream_epoch = self.session.offer.stream_epoch;
                block.header.source_sample_position = position;
                block.header.frame_count = (samples.len() / 8) as u16;
                block.header.discontinuity_flags = if self.pcm_frames == 0 {
                    Discontinuity::START
                } else if position != self.next_position {
                    Discontinuity::GAP
                } else {
                    Discontinuity::NONE
                };
                block.header.arrival_ns =
                    self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                // Remote native capture clock is not transmitted by RTP.
                block.header.capture_timestamp_ns = 0;
                for (dst, raw) in block.pcm.iter_mut().zip(samples.chunks_exact(8)) {
                    dst[0] = f32::from_le_bytes(
                        raw[0..4]
                            .try_into()
                            .map_err(|_| MediaError::InvalidPacket)?,
                    );
                    dst[1] = f32::from_le_bytes(
                        raw[4..8]
                            .try_into()
                            .map_err(|_| MediaError::InvalidPacket)?,
                    );
                }
                self.next_position = position + u64::from(block.header.frame_count);
                self.pcm_frames += u64::from(block.header.frame_count);
                for frame in block.frames() {
                    self.last_buffer_peak = self
                        .last_buffer_peak
                        .max(frame[0].abs())
                        .max(frame[1].abs());
                    energy += f64::from(frame[0]).powi(2) + f64::from(frame[1]).powi(2);
                    self.pcm_peak = self.pcm_peak.max(frame[0].abs()).max(frame[1].abs());
                    if frame[0] == 0.0 && frame[1] == 0.0 {
                        self.silent_pcm_frames += 1;
                    }
                }
                if !producer.push(block) {
                    self.queue_drops += u64::from(block.header.frame_count);
                }
                count += usize::from(block.header.frame_count);
            }
            self.last_buffer_rms = (energy / (map.len() / 4).max(1) as f64).sqrt();
        }
        Ok(count)
    }
    pub fn stats(&self) -> ReceiveStats {
        let jitter = self.jitter.property::<gst::Structure>("stats");
        let decoder = self.decoder.property::<gst::Structure>("stats");
        ReceiveStats {
            native_scheduling: self.endpoint.scheduling.snapshot(),
            wire_to_authenticated_max_ns: self
                .wire_to_authenticated_max_ns
                .load(std::sync::atomic::Ordering::Relaxed),
            authenticated_to_jitter_max_ns: self
                .authenticated_to_jitter_max_ns
                .load(std::sync::atomic::Ordering::Relaxed),
            pipeline_clock_ns: self
                .endpoint
                .pipeline
                .clock()
                .and_then(|c| c.time())
                .map(|t| t.nseconds()),
            pipeline_base_time_ns: self.endpoint.pipeline.base_time().map(|t| t.nseconds()),
            last_pcm_pts_ns: self.last_pcm_pts_ns,
            last_source_position: self.next_position,
            jitter_to_pcm_max_ns: self.jitter_to_pcm_max_ns,
            last_buffer_peak: self.last_buffer_peak,
            last_buffer_rms: self.last_buffer_rms,
            pcm_timing_gaps: self.pcm_timing_gaps.clone(),
            pcm_timing_gap_count: self.pcm_timing_gap_count,
            last_pcm_timing_gap: self.last_pcm_timing_gap,
            authenticated: self.endpoint.gate.authorized(),
            packets: self
                .timeline
                .lock()
                .map(|t| t.stats.clone())
                .unwrap_or_default(),
            lost_packets: jitter.get::<u64>("num-lost").unwrap_or(0)
                + self
                    .overflow_plc_packets
                    .load(std::sync::atomic::Ordering::Relaxed),
            late_packets: jitter.get::<u64>("num-late").unwrap_or(0),
            plc_samples: decoder.get::<u64>("plc-num-samples").unwrap_or(0),
            retimed_loss_events: self
                .retimed_loss_events
                .load(std::sync::atomic::Ordering::Relaxed),
            overflow_plc_packets: self
                .overflow_plc_packets
                .load(std::sync::atomic::Ordering::Relaxed),
            last_loss_duration_ns: self
                .last_loss_duration_ns
                .load(std::sync::atomic::Ordering::Relaxed),
            pcm_frames: self.pcm_frames,
            pcm_peak: self.pcm_peak,
            silent_pcm_frames: self.silent_pcm_frames,
            queue_drops: self.queue_drops,
            pcm_sink_dropped: self.pcm_queue_trace.dropped(),
        }
    }
    pub fn authenticated_packets(&self) -> u64 {
        self.timeline.lock().map(|t| t.stats.received).unwrap_or(0)
    }
    pub fn send_feedback(&mut self, queue_frames: u32) -> Result<(), MediaError> {
        if !self.endpoint.gate.authorized() {
            return Ok(());
        }
        let stats = self.stats();
        let lost = stats.lost_packets.saturating_sub(self.last_lost);
        let received = stats.packets.received.saturating_sub(self.last_received);
        let feedback = Feedback {
            loss_fraction: lost as f64 / (lost + received).max(1) as f64,
            late_packets: stats.late_packets,
            queue_frames,
            receiving: received > 0,
        };
        self.last_lost = stats.lost_packets;
        self.last_received = stats.packets.received;
        let report = protocol::receiver_report(
            self.session.stream_id as u32,
            self.session.offer.ssrc,
            stats.packets.highest_sequence as u32,
            stats.lost_packets as u32,
            feedback,
        );
        self.endpoint
            .rtcp_in
            .push_buffer(gst::Buffer::from_mut_slice(report))
            .map_err(native)?;
        Ok(())
    }
}

pub struct Sender {
    audio_queue_trace: Arc<crate::queue_trace::QueueTrace>,
    endpoint: Endpoint,
    audio: AppSrc,
    encoder: gst::Element,
    pub policy: SendPolicy,
    source_position: u64,
    ssrc: u32,
    feedback_reports: u64,
    last_feedback: Option<Feedback>,
}
impl Sender {
    pub fn notify_ready(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        notify_samples(&self.endpoint.rtcp_out, wake.clone());
        notify_samples(&self.endpoint.wire_out, wake);
    }
    pub fn new(session: &Session, pem: &str, hub_fingerprint: &str) -> Result<Self, MediaError> {
        Self::new_with_rtp_origin(session, pem, hub_fingerprint, None)
    }
    /// Fixed origins allow native rollover tests; every instance still creates
    /// a fresh DTLS association and keys, even when counters are identical.
    pub fn new_with_rtp_origin(
        session: &Session,
        pem: &str,
        hub_fingerprint: &str,
        origin: Option<(u16, u32)>,
    ) -> Result<Self, MediaError> {
        let endpoint = Endpoint::new(session, pem, hub_fingerprint, true)?;
        let audio = gst::ElementFactory::make("appsrc")
            .name("audio")
            .property("is-live", true)
            .property("format", gst::Format::Time)
            .property("block", false)
            .property("max-buffers", 4u64)
            .property("max-bytes", 15360u64)
            .property("leaky-type", gstreamer_app::AppLeakyType::Downstream)
            .build()
            .map_err(native)?
            .downcast::<AppSrc>()
            .map_err(|_| native("appsrc type"))?;
        audio.set_caps(Some(
            &gst::Caps::builder("audio/x-raw")
                .field("format", "F32LE")
                .field("rate", 48000i32)
                .field("channels", 2i32)
                .field("layout", "interleaved")
                .build(),
        ));
        let audio_queue_trace = Arc::new(crate::queue_trace::QueueTrace::new());
        let observing = audio_queue_trace.clone();
        let gate = endpoint.gate.clone();
        audio
            .static_pad("src")
            .ok_or_else(|| native("audio source pad"))?
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if info
                    .buffer()
                    .is_none_or(|buffer| observing.observe_contiguous(buffer).is_err())
                {
                    // The encoder/payloader need not preserve appsrc's DISCONT
                    // flag. Close before encoding after a real queue gap, so the
                    // normalized RTP clock cannot compress missing source time.
                    gate.revoke();
                    return gst::PadProbeReturn::Drop;
                }
                gst::PadProbeReturn::Ok
            });
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(native)?;
        let encoder = gst::ElementFactory::make("opusenc")
            .property("perfect-timestamp", true)
            .property("bitrate", 192000i32)
            .property_from_str("frame-size", "10")
            .property_from_str("audio-type", "generic")
            .property("inband-fec", false)
            .build()
            .map_err(native)?;
        let pay_builder = gst::ElementFactory::make("rtpopuspay")
            .name("pay")
            .property("perfect-rtptime", false)
            .property("pt", 96u32)
            .property("ssrc", session.offer.ssrc)
            .property("mtu", 1100u32);
        let pay = if let Some((seq, ts)) = origin {
            pay_builder
                .property("seqnum-offset", i32::from(seq))
                .property("timestamp-offset", ts)
                .build()
        } else {
            // Lower-half initial SEQ prevents SRTP ROC ambiguity if startup
            // packets are lost near rollover (RFC 4568 section 6.1).
            pay_builder
                .property(
                    "seqnum-offset",
                    i32::from((uuid::Uuid::new_v4().as_u128() as u16) & 0x7fff),
                )
                .build()
        }
        .map_err(native)?;
        let clock = Arc::new(Mutex::new((origin.map(|(_, ts)| ts), 0u64)));
        let gate = endpoint.gate.clone();
        pay.static_pad("src")
            .ok_or_else(|| native("pay src"))?
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                let Some(buffer) = info.buffer_mut() else {
                    return gst::PadProbeReturn::Drop;
                };
                let discontinuity = buffer.flags().contains(gst::BufferFlags::DISCONT);
                let Ok(mut clock) = clock.lock() else {
                    return gst::PadProbeReturn::Drop;
                };
                if discontinuity && clock.1 != 0 {
                    gate.revoke();
                    return gst::PadProbeReturn::Drop;
                }
                let Ok(mut rtp) = RTPBuffer::from_buffer_writable(buffer.make_mut()) else {
                    return gst::PadProbeReturn::Drop;
                };
                let base = *clock.0.get_or_insert(rtp.timestamp());
                // Each encoded packet represents 480 source frames. Gst opusenc's
                // priming offsets are not a second source clock. A discontinuity
                // closes this context rather than silently compressing source time.
                rtp.set_timestamp(base.wrapping_add(clock.1 as u32));
                let Some(next) = clock.1.checked_add(480) else {
                    gate.revoke();
                    return gst::PadProbeReturn::Drop;
                };
                clock.1 = next;
                gst::PadProbeReturn::Ok
            });
        let discard = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("async", false)
            .build()
            .map_err(native)?;
        endpoint
            .pipeline
            .add_many([audio.upcast_ref(), &convert, &encoder, &pay, &discard])
            .map_err(native)?;
        gst::Element::link_many([audio.upcast_ref(), &convert, &encoder, &pay]).map_err(native)?;
        pay.link_pads(
            Some("src"),
            &endpoint
                .pipeline
                .by_name("secure_out")
                .ok_or_else(|| native("secure_out"))?,
            Some("rtp_sink_0"),
        )
        .map_err(native)?;
        endpoint
            .pipeline
            .by_name("secure_in")
            .ok_or_else(|| native("secure_in"))?
            .link_pads(Some("rtp_src"), &discard, Some("sink"))
            .map_err(native)?;
        // Block unverified plaintext RTCP before delivery to the feedback worker.
        let gate = endpoint.gate.clone();
        endpoint
            .rtcp_out
            .static_pad("sink")
            .ok_or_else(|| native("rtcp sink"))?
            .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                if gate.authorized() {
                    gst::PadProbeReturn::Ok
                } else {
                    gst::PadProbeReturn::Drop
                }
            });
        endpoint
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(native)?;
        Ok(Self {
            audio_queue_trace,
            endpoint,
            audio,
            encoder,
            policy: SendPolicy::default(),
            source_position: 0,
            ssrc: session.offer.ssrc,
            feedback_reports: 0,
            last_feedback: None,
        })
    }
    pub fn ingest(&self, bytes: &[u8]) -> Result<(), MediaError> {
        self.endpoint.ingest(bytes)
    }
    pub fn outgoing(&self) -> Vec<Vec<u8>> {
        self.endpoint.outgoing()
    }
    pub fn gate(&self) -> MediaGate {
        self.endpoint.gate.clone()
    }
    pub fn check(&self) -> Result<(), MediaError> {
        self.endpoint.check()
    }
    pub fn push_frames(&mut self, frames: &[[f32; 2]]) -> Result<(), MediaError> {
        self.check()?;
        if frames.len() != MAX_BLOCK_FRAMES {
            return Err(MediaError::InvalidPacket);
        }
        let position = self.source_position;
        self.source_position = self
            .source_position
            .checked_add(frames.len() as u64)
            .ok_or(MediaError::Inactive)?;
        if !self.endpoint.gate.authorized() {
            return Ok(());
        }
        let bytes: Vec<u8> = frames
            .iter()
            .flat_map(|f| {
                f.iter().flat_map(|s| {
                    if self.policy.paused {
                        0.0f32.to_le_bytes()
                    } else {
                        s.to_le_bytes()
                    }
                })
            })
            .collect();
        let mut buffer = gst::Buffer::from_mut_slice(bytes);
        let b = buffer.get_mut().ok_or(MediaError::InvalidPacket)?;
        b.set_pts(gst::ClockTime::from_nseconds(
            neonmix_core::clock::frames_to_ns(position, 48000),
        ));
        b.set_duration(gst::ClockTime::from_mseconds(10));
        self.audio_queue_trace.tag(b)?;
        self.audio.push_buffer(buffer).map_err(native)?;
        Ok(())
    }
    pub fn poll_feedback(&mut self) {
        for bytes in drain_bytes(&self.endpoint.rtcp_out, 8) {
            if let Some(feedback) = protocol::parse_feedback(&bytes, self.ssrc) {
                self.feedback_reports += 1;
                self.last_feedback = Some(feedback);
                self.policy.update(feedback);
            }
        }
        self.encoder
            .set_property("bitrate", self.policy.bitrate as i32);
        self.encoder.set_property("dtx", self.policy.paused);
    }
    pub fn native_scheduling(&self) -> crate::SchedulingSnapshot {
        self.endpoint.scheduling.snapshot()
    }
    pub fn last_feedback(&self) -> Option<Feedback> {
        self.last_feedback
    }
    pub fn feedback_reports(&self) -> u64 {
        self.feedback_reports
    }
    pub fn audio_queue_dropped(&self) -> u64 {
        self.audio_queue_trace.dropped()
    }
    pub fn audio_queue_buffers(&self) -> u64 {
        self.audio.property::<u64>("current-level-buffers")
    }
    pub fn encoder_bitrate(&self) -> i32 {
        self.encoder.property("bitrate")
    }
    pub fn encoder_dtx(&self) -> bool {
        self.encoder.property("dtx")
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;
    use neonmix_control::{MediaOffer, SessionStatus};
    use neonmix_core::{queue::block_queue, stats::AudioStats};
    use std::time::Duration;

    fn pem() -> String {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        format!("{}{}", cert.pem(), signing_key.serialize_pem())
    }

    #[test]
    fn authenticated_twenty_ms_opus_cannot_enter_a_ten_ms_stream() {
        let sender_pem = pem();
        let hub_pem = pem();
        let session = Session {
            created_revision: 0,
            media_ttl_seconds: neonmix_control::MEDIA_TTL_SECONDS,
            id: uuid::Uuid::new_v4(),
            device_id: uuid::Uuid::new_v4(),
            stream_id: 1,
            media_context: uuid::Uuid::new_v4(),
            status: SessionStatus::Buffering,
            offer: MediaOffer {
                version: 1,
                codec: "opus".into(),
                rate: 48000,
                channels: 2,
                packet_frames: 480,
                payload_type: 96,
                ssrc: 123,
                stream_epoch: 1,
                udp_port: 5000,
                certificate_sha256: certificate_fingerprint(&sender_pem).unwrap(),
            },
        };
        let mut receiver = Receiver::new(session.clone(), &hub_pem, Instant::now()).unwrap();
        let mut sender = Sender::new(
            &session,
            &sender_pem,
            &certificate_fingerprint(&hub_pem).unwrap(),
        )
        .unwrap();
        // Fault injection occurs before SRTP protection, so these malformed
        // negotiated-format packets still carry authentic context keys.
        sender.encoder.set_property_from_str("frame-size", "20");
        let start = Instant::now();
        while !sender.gate().authorized() || !receiver.gate().authorized() {
            assert!(start.elapsed() < Duration::from_secs(3));
            for packet in sender.outgoing() {
                receiver.ingest(&packet).unwrap();
            }
            for packet in receiver.outgoing() {
                sender.ingest(&packet).unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let (mut producer, _consumer) = block_queue(16, Arc::new(AudioStats::default())).unwrap();
        for _ in 0..12 {
            sender.push_frames(&[[0.01; 2]; 480]).unwrap();
            std::thread::sleep(Duration::from_millis(10));
            for packet in sender.outgoing() {
                receiver.ingest(&packet).unwrap();
            }
            assert_eq!(receiver.pump_pcm(&mut producer).unwrap(), 0);
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(receiver.pump_pcm(&mut producer).unwrap(), 0);
        let stats = receiver.stats();
        assert!(stats.authenticated);
        assert!(stats.packets.invalid >= 3);
        assert_eq!(stats.packets.received, 0);
        assert_eq!(stats.pcm_frames, 0);
    }
}
