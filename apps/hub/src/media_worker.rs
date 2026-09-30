//! Each stream owns its native pipeline and socket on a non-realtime thread.
//! Authority, disk I/O and other stream teardown never stop this media pump.
use neonmix_control::SessionStatus;
use neonmix_core::{mixer::MixerStats, queue::BlockProducer};
use neonmix_media::{MediaGate, ReceiveStats, Receiver};
use std::{
    net::UdpSocket,
    sync::{
        Arc, Mutex,
        atomic::Ordering::Relaxed,
        mpsc::{SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Report {
    pub receive: ReceiveStats,
    pub status: SessionStatus,
    pub error: Option<String>,
    pub budget_drops: u64,
    pub receive_throttles: u64,
    pub scheduling: neonmix_media::ThreadPrioritySnapshot,
    pub max_loop_gap_ns: u64,
    pub max_pump_ns: u64,
    pub max_report_ns: u64,
    pub timing: crate::pump_timing::Snapshot,
}
pub struct MediaWorker {
    pub port: u16,
    gate: MediaGate,
    report: Arc<Mutex<Report>>,
    thread: Option<JoinHandle<Option<BlockProducer>>>,
}
/// Created before the durable transaction. The thread waits without owning an
/// audio producer or accepting packets until commit; dropping rolls it back.
pub struct PreparedWorker {
    activation: Option<SyncSender<BlockProducer>>,
    worker: MediaWorker,
}
impl PreparedWorker {
    pub fn activate(mut self, producer: BlockProducer) -> MediaWorker {
        // The only code in the prepared thread is recv(). Its sender remains
        // owned here until activation, so it cannot disconnect before this send.
        self.activation
            .take()
            .expect("prepared activation owned")
            .send(producer)
            .unwrap_or_else(|_| unreachable!("prepared receiver remains alive"));
        self.worker
    }
}
impl MediaWorker {
    pub fn prepare(
        mut receiver: Receiver,
        socket: UdpSocket,
        index: usize,
        mixer: Arc<MixerStats>,
    ) -> std::io::Result<PreparedWorker> {
        let port = socket.local_addr()?.port();
        let mut media_wait = crate::media_wait::MediaWait::new(socket)?;
        let wake = media_wait.waker();
        receiver.notify_ready(Arc::new(move || {
            let _ = wake.wake();
        }));
        let gate = receiver.gate();
        let report = Arc::new(Mutex::new(Report {
            receive: receiver.stats(),
            status: SessionStatus::Buffering,
            error: None,
            budget_drops: 0,
            receive_throttles: 0,
            scheduling: neonmix_media::ThreadPrioritySnapshot::default(),
            max_loop_gap_ns: 0,
            max_pump_ns: 0,
            max_report_ns: 0,
            timing: crate::pump_timing::Snapshot::default(),
        }));
        let publish = report.clone();
        let (activate, awaiting) = sync_channel::<BlockProducer>(1);
        let worker = thread::Builder::new()
            .name(format!("media-{index}"))
            .spawn(move || {
                let Ok(mut producer) = awaiting.recv() else {
                    return None;
                };
                let _activity = crate::qos::AudioActivity::begin();
                let scheduling = neonmix_media::ThreadPriority::enter();
                let origin = Instant::now();
                let mut previous_loop = origin;
                let mut timing = crate::pump_timing::Timing::new();
                let mut max_loop_gap_ns = 0u64;
                let mut max_pump_ns = 0u64;
                let mut max_report_ns = 0u64;
                let mut last_packet = origin;
                let mut last_feedback = origin;
                let mut last_publish = origin;
                let mut degraded = origin;
                let mut packet_budget = 0u32;
                let mut budget_started = origin;
                let mut budget_drops = 0;
                let mut packets_seen = 0;
                let mut lost_seen = 0;
                let mut late_seen = 0;
                let mut status = SessionStatus::Buffering;
                let mut bytes = [0u8; 4097];
                let mut wait_error: Option<std::io::Error> = None;
                loop {
                    let loop_started = Instant::now();
                    max_loop_gap_ns = max_loop_gap_ns.max(
                        loop_started
                            .duration_since(previous_loop)
                            .as_nanos()
                            .min(u128::from(u64::MAX)) as u64,
                    );
                    previous_loop = loop_started;
                    let mut work_remaining = false;
                    let result: Result<(), Box<dyn std::error::Error>> = (|| {
                        if let Some(error) = wait_error.take() {
                            return Err(error.into());
                        }
                        if budget_started.elapsed() >= Duration::from_secs(1) {
                            packet_budget = 0;
                            budget_started = Instant::now();
                        }
                        work_remaining = true;
                        for _ in 0..32 {
                            match media_wait.recv(&mut bytes) {
                                Ok(n) => {
                                    if packet_budget < 200 {
                                        packet_budget += 1;
                                        let _ = receiver.ingest(&bytes[..n]);
                                    } else {
                                        budget_drops += 1;
                                    }
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    work_remaining = false;
                                    break;
                                }
                                Err(e) => return Err(e.into()),
                            }
                        }
                        timing.finish("receive");
                        let pump_started = Instant::now();
                        let frames = receiver.pump_pcm(&mut producer)?;
                        work_remaining |= receiver.pump_budget_exhausted();
                        max_pump_ns =
                            max_pump_ns
                                .max(pump_started.elapsed().as_nanos().min(u128::from(u64::MAX))
                                    as u64);
                        timing.finish("source");
                        let packets = receiver.authenticated_packets();
                        if packets > packets_seen {
                            packets_seen = packets;
                            last_packet = Instant::now();
                        }
                        for packet in receiver.outgoing() {
                            let _ = media_wait.send(&packet);
                        }
                        timing.finish("outgoing");
                        if last_feedback.elapsed() >= Duration::from_secs(1) {
                            let stats = receiver.stats();
                            if stats.lost_packets > lost_seen || stats.late_packets > late_seen {
                                degraded = Instant::now() + Duration::from_secs(2);
                            }
                            lost_seen = stats.lost_packets;
                            late_seen = stats.late_packets;
                            receiver.send_feedback(
                                mixer.filtered_queue_frames[index].load(Relaxed) as u32,
                            )?;
                            last_feedback = Instant::now();
                        }
                        timing.finish("feedback");
                        if last_packet.elapsed() > Duration::from_secs(5) {
                            return Err("media_timeout".into());
                        }
                        if frames > 0 {
                            status = if Instant::now() < degraded {
                                SessionStatus::NetworkDegraded
                            } else {
                                SessionStatus::Playing
                            };
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        receiver.gate().revoke();
                        producer.invalidate();
                        // Media has halted and authorization is closed. Publish the
                        // terminal state reliably; a busy diagnostic read must not
                        // leave the authority seeing a dead worker as Playing.
                        if let Ok(mut report) = publish.lock() {
                            *report = Report {
                                receive: receiver.stats(),
                                status: SessionStatus::NetworkInterrupted,
                                error: Some(error.to_string()),
                                budget_drops,
                                receive_throttles: media_wait.receive_throttles,
                                scheduling: scheduling.snapshot(),
                                max_loop_gap_ns,
                                max_pump_ns,
                                max_report_ns,
                                timing: timing.snapshot.clone(),
                            };
                        }
                        break;
                    }
                    if last_publish.elapsed() >= Duration::from_millis(20) {
                        let report_started = Instant::now();
                        if let Ok(mut report) = publish.try_lock() {
                            *report = Report {
                                receive: receiver.stats(),
                                status,
                                error: None,
                                budget_drops,
                                receive_throttles: media_wait.receive_throttles,
                                scheduling: scheduling.snapshot(),
                                max_loop_gap_ns,
                                max_pump_ns,
                                max_report_ns,
                                timing: timing.snapshot.clone(),
                            };
                        }
                        max_report_ns = max_report_ns.max(
                            report_started
                                .elapsed()
                                .as_nanos()
                                .min(u128::from(u64::MAX)) as u64,
                        );
                        last_publish = Instant::now();
                    }
                    timing.finish("report");
                    let wait = if work_remaining {
                        // Retry an exhausted edge in at most 1 ms; ingress
                        // still has its independent 200 datagrams/s budget.
                        Duration::from_millis(1)
                    } else {
                        Duration::from_millis(20).saturating_sub(last_publish.elapsed())
                    };
                    wait_error = media_wait.wait(wait).err();
                    timing.finish_wait(wait);
                }
                drop(receiver);
                Some(producer)
            })?;
        Ok(PreparedWorker {
            worker: Self {
                port,
                gate,
                report,
                thread: Some(worker),
            },
            activation: Some(activate),
        })
    }
    pub fn latest(&self) -> Option<Report> {
        self.report.lock().ok().map(|r| r.clone())
    }
    pub fn close(mut self) -> Option<BlockProducer> {
        self.gate.revoke();
        self.thread.take()?.join().ok().flatten()
    }
}
impl Drop for MediaWorker {
    fn drop(&mut self) {
        self.gate.revoke();
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neonmix_control::{MediaOffer, Session};
    use neonmix_core::{queue::block_queue, stats::AudioStats};
    use uuid::Uuid;
    #[test]
    fn terminal_report_survives_a_busy_diagnostic_reader() {
        let (pem, certificate, _) = crate::certificate().unwrap();
        let session = Session {
            created_revision: 1,
            media_ttl_seconds: neonmix_control::MEDIA_TTL_SECONDS,
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            stream_id: 1,
            media_context: Uuid::new_v4(),
            status: SessionStatus::Buffering,
            offer: MediaOffer {
                version: 1,
                codec: "opus".into(),
                rate: 48000,
                channels: 2,
                packet_frames: 480,
                payload_type: 96,
                ssrc: 1,
                stream_epoch: 1,
                udp_port: 54321,
                certificate_sha256: neonmix_media::certificate_fingerprint(&certificate).unwrap(),
            },
        };
        let receiver = Receiver::new(session, &pem, Instant::now()).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let (mut producer, mut consumer) = block_queue(8, Arc::new(AudioStats::default())).unwrap();
        let mut pending = neonmix_core::AudioBlock::empty(1, neonmix_core::AudioFormat::INTERNAL);
        pending.header.stream_epoch = 1;
        pending.header.frame_count = 480;
        pending.pcm.fill([0.01; 2]);
        assert!(producer.push(pending));
        let prepared =
            MediaWorker::prepare(receiver, socket, 0, Arc::new(MixerStats::default())).unwrap();
        let report = prepared.worker.report.clone();
        let reading = report.lock().unwrap();
        prepared.worker.gate.revoke();
        let worker = prepared.activate(producer);
        thread::sleep(Duration::from_millis(50));
        assert!(
            !worker.thread.as_ref().unwrap().is_finished(),
            "terminal state was discarded"
        );
        assert!(
            consumer.pop_fresh(0, u64::MAX).is_none(),
            "revoked PCM backlog remained playable"
        );
        drop(reading);
        let start = Instant::now();
        while !worker.thread.as_ref().unwrap().is_finished() {
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(1));
        }
        let terminal = worker.latest().unwrap();
        assert_eq!(terminal.status, SessionStatus::NetworkInterrupted);
        assert!(terminal.error.is_some());
        assert!(worker.close().is_some(), "producer ownership was lost");
    }
}
