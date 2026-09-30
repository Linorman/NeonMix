use crate::{Result, emit};
use neonmix_control::{MediaOffer, Session, SessionStatus};
use neonmix_core::{
    MAX_BLOCK_FRAMES,
    mixer::{LaneMix, Mixer, MixerConfig},
    signal::{SignalKind, StereoSource, TestSignal},
};
use neonmix_media::{Receiver, Sender};
use std::{
    net::UdpSocket,
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};
use uuid::Uuid;
pub fn run(
    seconds: u32,
    streams: usize,
    output: Option<String>,
    drop_every: u32,
    wrong: bool,
    replay: bool,
) -> Result<()> {
    if !(1..=86400).contains(&seconds) || !(1..=2).contains(&streams) {
        return Err("seconds 1..86400, streams 1..2".into());
    }
    let _activity = crate::qos::AudioActivity::begin();
    neonmix_media::runtime_probe()?;
    let origin = Instant::now();
    let (mixer, mut control, mut inputs, stats) = Mixer::new(origin)?;
    let (hub_pem, hub_cert, _) = crate::certificate()?;
    let hub_fp = neonmix_media::certificate_fingerprint(&hub_cert)?;
    let mut pairs = Vec::new();
    let mut config = MixerConfig::default();
    for i in 0..streams {
        let (pem, cert, _) = crate::certificate()?;
        let send_socket = UdpSocket::bind("127.0.0.1:0")?;
        let receive_socket = UdpSocket::bind("127.0.0.1:0")?;
        send_socket.connect(receive_socket.local_addr()?)?;
        receive_socket.connect(send_socket.local_addr()?)?;
        send_socket.set_nonblocking(true)?;
        receive_socket.set_nonblocking(true)?;
        let session = Session {
            created_revision: 0,
            media_ttl_seconds: neonmix_control::MEDIA_TTL_SECONDS,
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            stream_id: (i + 1) as u64,
            media_context: Uuid::new_v4(),
            status: SessionStatus::Buffering,
            offer: MediaOffer {
                version: 1,
                codec: "opus".into(),
                rate: 48000,
                channels: 2,
                packet_frames: 480,
                payload_type: 96,
                ssrc: (i + 101) as u32,
                stream_epoch: 1,
                udp_port: send_socket.local_addr()?.port(),
                certificate_sha256: if wrong {
                    "0".repeat(64)
                } else {
                    neonmix_media::certificate_fingerprint(&cert)?
                },
            },
        };
        let receiver = Receiver::new(session.clone(), &hub_pem, origin)?;
        let sender = Sender::new(&session, &pem, &hub_fp)?;
        config.lanes[i] = LaneMix {
            stream_id: session.stream_id,
            epoch: 1,
            ..LaneMix::default()
        };
        pairs.push((
            sender,
            receiver,
            send_socket,
            receive_socket,
            TestSignal::new(SignalKind::Sine, 48000, 437.0 + i as f64 * 222.0, -36.0)?,
            0u64,
        ));
    }
    control.apply(config)?;
    let (mut offline, running) = if let Some(id) = output {
        let running = crate::backend()?.open_output(
            &id,
            neonmix_io::OpenOptions {
                sample_rate: Some(48000),
                period_frames: Some(256),
            },
            mixer,
        )?;
        running.play()?;
        (None, Some(running))
    } else {
        (Some(mixer), None)
    };
    let mut frames = [[0.0; 2]; MAX_BLOCK_FRAMES];
    let mut bytes = [0u8; 4097];
    let mut next = Instant::now();
    let mut feedback = Instant::now() + Duration::from_secs(1);
    let mut peak = 0.0f32;
    let mut energy = 0.0f64;
    let mut measured = 0u64;
    let mut rejected = 0u64;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(u64::from(seconds)) {
        for (i, (sender, receiver, send_socket, receive_socket, _, sent)) in
            pairs.iter_mut().enumerate()
        {
            for packet in sender.outgoing() {
                let media = packet.first().is_some_and(|b| b & 0xc0 == 0x80);
                if media {
                    *sent += 1;
                    if drop_every > 0 && sent.is_multiple_of(u64::from(drop_every)) {
                        continue;
                    }
                }
                let _ = send_socket.send(&packet);
                if replay && media {
                    let _ = send_socket.send(&packet);
                }
            }
            for _ in 0..32 {
                match receive_socket.recv(&mut bytes) {
                    Ok(n) => {
                        let _ = receiver.ingest(&bytes[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            match receiver.pump_pcm(&mut inputs[i]) {
                Ok(_) => {}
                Err(neonmix_media::MediaError::CertificateMismatch) if wrong => {
                    rejected += 1;
                }
                Err(e) => return Err(e.into()),
            }
            for packet in receiver.outgoing() {
                let _ = receive_socket.send(&packet);
            }
            for _ in 0..32 {
                match send_socket.recv(&mut bytes) {
                    Ok(n) => {
                        let _ = sender.ingest(&bytes[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            sender.check()?;
        }
        if Instant::now() >= next {
            for (sender, _, _, _, signal, _) in &mut pairs {
                for f in &mut frames {
                    *f = signal.next_frame();
                }
                sender.push_frames(&frames)?;
            }
            if let Some(mixer) = &mut offline {
                mixer.render_block(&mut frames);
                for f in frames {
                    peak = peak.max(f[0].abs());
                    energy += f64::from(f[0]).powi(2);
                    measured += 1;
                }
            }
            next += Duration::from_millis(10);
        }
        if Instant::now() >= feedback {
            for (i, (sender, receiver, _, _, _, _)) in pairs.iter_mut().enumerate() {
                receiver.send_feedback(stats.filtered_queue_frames[i].load(Relaxed) as u32)?;
                sender.poll_feedback();
            }
            feedback += Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let receivers: Vec<_> = pairs.iter().map(|p| p.1.stats()).collect();
    emit(
        serde_json::json!({"platform":std::env::consts::OS,"streams":streams,"seconds":seconds,"wrong_fingerprint":wrong,"replay":replay,"drop_every":drop_every,"rejected":rejected,"receivers":receivers,"peak":peak,"rms":(energy/measured.max(1) as f64).sqrt(),"output_frames":stats.output_frames.load(Relaxed),"underrun_frames":stats.underrun_frames.load(Relaxed),"limited_frames":stats.limited_frames.load(Relaxed),"output":running.as_ref().map(|o| &o.info),"output_stats":running.as_ref().map(|o| o.stats.snapshot())}),
    )?;
    if wrong {
        if receivers.iter().any(|r| r.pcm_frames > 0) || rejected == 0 {
            return Err("untrusted media was not rejected".into());
        }
    } else {
        if receivers
            .iter()
            .any(|r| !r.authenticated || r.pcm_frames < 48000)
        {
            return Err("insufficient authenticated PCM".into());
        }
        if receivers.iter().any(|r| r.pcm_timing_gap_count > 0) {
            return Err("decoded source timeline is discontinuous".into());
        }
        if stats.underrun_frames.load(Relaxed) > 0 {
            return Err("mixer underrun".into());
        }
        if drop_every > 0 && receivers.iter().any(|r| r.plc_samples == 0) {
            return Err("loss did not reach decoder PLC".into());
        }
        if offline.is_some() && peak < 0.001 {
            return Err("mixer output remained silent".into());
        }
        if running
            .as_ref()
            .is_some_and(|o| o.stats.snapshot().errors > 0)
        {
            return Err("native output error".into());
        }
    }
    Ok(())
}
