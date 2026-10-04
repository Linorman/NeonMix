//! Real native DTLS/SRTP/SRTCP; no plaintext substitute or stored PCM.
use neonmix_control::{MediaOffer, Session, SessionStatus};
use neonmix_core::{queue::block_queue, stats::AudioStats};
use neonmix_media::{Receiver, Sender, certificate_fingerprint};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
fn pem() -> String {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    format!("{}{}", cert.pem(), signing_key.serialize_pem())
}
fn session(pem: &str) -> Session {
    Session {
        created_revision: 0,
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
            ssrc: 123,
            stream_epoch: 1,
            udp_port: 5000,
            certificate_sha256: certificate_fingerprint(pem).unwrap(),
        },
    }
}
fn handshake(sender: &Sender, receiver: &Receiver) {
    let start = Instant::now();
    while !sender.gate().authorized() || !receiver.gate().authorized() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "DTLS handshake timeout"
        );
        for bytes in sender.outgoing() {
            receiver.ingest(&bytes).unwrap();
        }
        for bytes in receiver.outgoing() {
            sender.ingest(&bytes).unwrap();
        }
        sender.check().unwrap();
        receiver.check().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn replay_corruption_revocation_and_fresh_key_context_isolation() {
    let sender_pem = pem();
    let hub_pem = pem();
    let old = session(&sender_pem);
    let mut receiver = Receiver::new(old.clone(), &hub_pem, Instant::now()).unwrap();
    let mut sender = Sender::new_with_rtp_origin(
        &old,
        &sender_pem,
        &certificate_fingerprint(&hub_pem).unwrap(),
        Some((65534, u32::MAX - 479)),
    )
    .unwrap();
    handshake(&sender, &receiver);
    let (mut producer, mut consumer) = block_queue(16, Arc::new(AudioStats::default())).unwrap();
    let mut saved = Vec::new();
    let frames = [[0.01; 2]; 480];
    for _ in 0..12 {
        sender.push_frames(&frames).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        for packet in sender.outgoing() {
            if packet.first().is_some_and(|b| b & 0xc0 == 0x80) {
                saved.push(packet.clone());
            }
            receiver.ingest(&packet).unwrap();
        }
        receiver.pump_pcm(&mut producer).unwrap();
        while consumer.pop_fresh(0, u64::MAX).is_some() {}
    }
    std::thread::sleep(Duration::from_millis(60));
    receiver.pump_pcm(&mut producer).unwrap();
    assert!(!saved.is_empty());
    assert!(
        receiver.stats().packets.highest_sequence >= 65536,
        "{:?}",
        receiver.stats().packets
    );
    assert!(receiver.stats().packets.highest_timestamp >= 1u64 << 32);
    assert_eq!(receiver.stats().packets.timestamp_step_errors, 0);
    let before = receiver.stats().packets.received;
    for packet in &saved {
        receiver.ingest(packet).unwrap();
        let mut damaged = packet.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0x80;
        receiver.ingest(&damaged).unwrap();
    }
    std::thread::sleep(Duration::from_millis(30));
    receiver.check().unwrap();
    assert_eq!(
        receiver.stats().packets.received,
        before,
        "replayed/corrupt SRTP passed authentication"
    );
    receiver.send_feedback(4000).unwrap();
    std::thread::sleep(Duration::from_millis(10));
    let report = receiver.outgoing();
    assert!(!report.is_empty());
    for bytes in &report {
        sender.ingest(bytes).unwrap();
        sender.ingest(bytes).unwrap();
    }
    std::thread::sleep(Duration::from_millis(20));
    sender.poll_feedback();
    assert_eq!(sender.feedback_reports(), 1, "SRTCP replay was accepted");
    receiver.gate().revoke();
    assert!(receiver.ingest(&saved[0]).is_err());
    assert!(receiver.pump_pcm(&mut producer).is_err());
    drop(receiver);
    drop(sender);
    let fresh = session(&sender_pem);
    assert_ne!(old.media_context, fresh.media_context);
    let receiver = Receiver::new(fresh.clone(), &hub_pem, Instant::now()).unwrap();
    let mut sender = Sender::new(
        &fresh,
        &sender_pem,
        &certificate_fingerprint(&hub_pem).unwrap(),
    )
    .unwrap();
    handshake(&sender, &receiver);
    for packet in &saved {
        receiver.ingest(packet).unwrap();
    }
    for bytes in &report {
        sender.ingest(bytes).unwrap();
    }
    std::thread::sleep(Duration::from_millis(30));
    receiver.check().unwrap();
    sender.poll_feedback();
    assert_eq!(
        receiver.stats().packets.received,
        0,
        "old session SRTP accepted under new keys"
    );
    assert_eq!(
        sender.feedback_reports(),
        0,
        "old SRTCP accepted under new keys"
    );
}
#[test]
fn negotiated_expiry_stops_both_decode_and_ingress() {
    let sender_pem = pem();
    let hub_pem = pem();
    let mut session = session(&sender_pem);
    session.media_ttl_seconds = 1;
    let receiver = Receiver::new(session.clone(), &hub_pem, Instant::now()).unwrap();
    let sender = Sender::new(
        &session,
        &sender_pem,
        &certificate_fingerprint(&hub_pem).unwrap(),
    )
    .unwrap();
    handshake(&sender, &receiver);
    std::thread::sleep(Duration::from_millis(1050));
    assert!(!receiver.gate().authorized());
    assert!(receiver.ingest(&[0x80; 22]).is_err());
    assert!(receiver.check().is_err());
}
#[test]
fn native_jitter_reorders_and_discards_late_packets_with_real_plc() {
    let sender_pem = pem();
    let hub_pem = pem();
    let s = session(&sender_pem);
    let mut receiver = Receiver::new(s.clone(), &hub_pem, Instant::now()).unwrap();
    let mut sender =
        Sender::new(&s, &sender_pem, &certificate_fingerprint(&hub_pem).unwrap()).unwrap();
    handshake(&sender, &receiver);
    let (mut producer, mut consumer) = block_queue(16, Arc::new(AudioStats::default())).unwrap();
    let mut next_pcm_position = 0;
    let mut count = 0;
    let mut reordered = None;
    let mut late = None;
    for _ in 0..24 {
        sender.push_frames(&[[0.01; 2]; 480]).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        for packet in sender.outgoing() {
            if packet.first().is_some_and(|b| b & 0xc0 == 0x80) {
                count += 1;
                if count == 4 {
                    reordered = Some(packet);
                    continue;
                }
                if count == 9 {
                    late = Some(packet);
                    continue;
                }
            }
            receiver.ingest(&packet).unwrap();
            if count == 5
                && let Some(packet) = reordered.take()
            {
                receiver.ingest(&packet).unwrap();
            }
        }
        receiver.pump_pcm(&mut producer).unwrap();
        while let Some(block) = consumer.pop_fresh(0, u64::MAX) {
            assert_eq!(block.header.source_sample_position, next_pcm_position);
            next_pcm_position += u64::from(block.header.frame_count);
        }
    }
    receiver
        .ingest(&late.expect("missing late packet"))
        .unwrap();
    assert!(receiver.ingest(&[0x80; 1201]).is_err());
    assert!(receiver.ingest(&[22; 4097]).is_err());
    std::thread::sleep(Duration::from_millis(100));
    receiver.pump_pcm(&mut producer).unwrap();
    while let Some(block) = consumer.pop_fresh(0, u64::MAX) {
        assert_eq!(block.header.source_sample_position, next_pcm_position);
        next_pcm_position += u64::from(block.header.frame_count);
    }
    let stats = receiver.stats();
    assert_eq!(next_pcm_position, stats.pcm_frames);
    assert!(stats.packets.reordered > 0);
    assert!(stats.late_packets > 0);
    assert!(stats.plc_samples >= 480);
    assert!(stats.pcm_frames > 4800);
    assert!(stats.pcm_timing_gaps.is_empty());
    assert_eq!(stats.jitter_queued_packets, 0);
    assert!(!stats.jitter_capacity_exhausted);
    #[cfg(target_os = "macos")]
    {
        assert!(stats.native_scheduling.entered > 0);
        assert_eq!(
            stats.native_scheduling.configured,
            stats.native_scheduling.entered
        );
        assert_eq!(stats.native_scheduling.failed, 0);
    }
}

#[test]
fn sub_deadline_network_batches_do_not_evict_authenticated_audio() {
    let sender_pem = pem();
    let hub_pem = pem();
    let s = session(&sender_pem);
    let mut receiver = Receiver::new(s.clone(), &hub_pem, Instant::now()).unwrap();
    let mut sender =
        Sender::new(&s, &sender_pem, &certificate_fingerprint(&hub_pem).unwrap()).unwrap();
    handshake(&sender, &receiver);
    let (mut producer, mut consumer) = block_queue(16, Arc::new(AudioStats::default())).unwrap();
    let start = Instant::now();
    let mut source_frames = 0u64;
    let mut next_delivery = Duration::from_millis(30);
    let mut pending = Vec::new();
    let mut next_pcm = 0;
    while start.elapsed() < Duration::from_millis(1500) {
        let elapsed = start.elapsed();
        if source_frames < 48000
            && elapsed.as_micros() >= u128::from(source_frames) * 1_000_000 / 48000
        {
            sender.push_frames(&[[0.01; 2]; 480]).unwrap();
            source_frames += 480;
        }
        pending.extend(sender.outgoing());
        if elapsed >= next_delivery {
            // A 30 ms receive burst is inside the negotiated 40 ms jitter
            // deadline. It must not be mistaken for a full/expired queue.
            for packet in pending.drain(..) {
                receiver.ingest(&packet).unwrap();
            }
            next_delivery += Duration::from_millis(30);
        }
        receiver.pump_pcm(&mut producer).unwrap();
        while let Some(block) = consumer.pop_fresh(0, u64::MAX) {
            assert_eq!(block.header.source_sample_position, next_pcm);
            next_pcm += u64::from(block.header.frame_count);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let stats = receiver.stats();
    println!(
        "received={} pcm={} lost={} overflow={} late={}",
        stats.packets.received,
        stats.pcm_frames,
        stats.lost_packets,
        stats.overflow_plc_packets,
        stats.late_packets
    );
    assert!(stats.packets.received >= 99);
    assert!(stats.pcm_frames >= 47520);
    assert_eq!(stats.lost_packets, 0);
    assert_eq!(stats.overflow_plc_packets, 0);
    assert_eq!(stats.jitter_queued_packets, 0);
    assert!(!stats.jitter_capacity_exhausted);
    assert_eq!(stats.pcm_sink_dropped, 0);
    assert_eq!(stats.queue_drops, 0);
    assert_eq!(stats.pcm_timing_gap_count, 0);
}
