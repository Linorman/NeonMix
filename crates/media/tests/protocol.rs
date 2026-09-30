use neonmix_media::protocol::*;
#[test]
fn rtp_sequence_timestamp_wrap_duplicate_and_reorder() {
    let mut t = RtpTimeline::default();
    assert!(t.observe(65534, u32::MAX - 479));
    assert!(t.observe(0, 480));
    assert!(t.observe(65535, 0));
    assert!(!t.observe(65535, 0));
    assert_eq!(t.stats.highest_sequence, 65536);
    assert_eq!(t.stats.highest_timestamp, (1u64 << 32) + 480);
    assert_eq!(t.stats.reordered, 1);
    assert_eq!(t.stats.duplicate, 1);
}
#[test]
fn packet_quotas_and_feedback_congestion_pause_recovery() {
    assert!(validate_datagram(&[]).is_err());
    assert!(validate_datagram(&vec![0x80; 1201]).is_err());
    assert!(validate_datagram(&vec![22; 4097]).is_err());
    let f = Feedback {
        loss_fraction: 0.4,
        late_packets: 3,
        queue_frames: 1920,
        receiving: true,
    };
    let bytes = receiver_report(10, 11, 123, 8, f);
    let parsed = parse_feedback(&bytes, 11).unwrap();
    assert_eq!(parsed.queue_frames, 1920);
    assert!(parse_feedback(&bytes, 12).is_none());
    let mut p = SendPolicy::default();
    for _ in 0..24 {
        p.update(f);
    }
    assert_eq!(p.bitrate, 64000);
    assert!(p.paused);
    for _ in 0..5 {
        p.update(Feedback {
            loss_fraction: 0.0,
            queue_frames: 960,
            ..f
        });
    }
    assert!(!p.paused);
}
#[test]
fn authenticated_rtp_with_wrong_time_step_is_rejected() {
    let mut t = RtpTimeline::default();
    assert!(t.observe(1, 0));
    assert!(!t.observe(2, 960));
    assert!(t.observe(2, 480));
    assert_eq!(t.stats.timestamp_step_errors, 1);
}

#[test]
fn reordered_wrong_timestamp_does_not_poison_clock_or_consume_sequence() {
    let mut t = RtpTimeline::default();
    assert!(t.observe(65534, u32::MAX - 479));
    assert!(t.observe(0, 480));
    assert!(!t.observe(65535, 960));
    assert_eq!(t.stats.highest_timestamp, (1u64 << 32) + 480);
    // Correcting the rejected packet must still be accepted exactly once.
    assert!(t.observe(65535, 0));
    assert!(!t.observe(65535, 0));
    assert!(t.observe(1, 960));
    assert_eq!(t.stats.timestamp_step_errors, 1);
    assert_eq!(t.stats.reordered, 1);
    assert_eq!(t.stats.received, 4);
}

#[test]
fn initial_reordering_across_timestamp_zero_preserves_forward_anchor() {
    let mut t = RtpTimeline::default();
    assert!(t.observe(102, 480));
    assert!(t.observe(100, u32::MAX - 479));
    assert_eq!(t.stats.highest_sequence, 102);
    assert_eq!(t.stats.highest_timestamp, 480);
    assert!(t.observe(101, 0));
    assert!(t.observe(103, 960));
    assert_eq!(t.stats.timestamp_step_errors, 0);
    assert_eq!(t.stats.reordered, 2);
}

#[test]
fn opus_packet_duration_matches_negotiated_clock_for_all_toc_modes() {
    // SILK/Hybrid/CELT 10 ms and aggregates of 2.5/5 ms frames all
    // represent the same 480-sample packet clock, irrespective of stereo bit.
    for config in [0, 4, 8, 12, 14, 18, 22, 26, 30] {
        for stereo in [0, 4] {
            assert_eq!(opus_packet_frames(&[(config << 3) | stereo]), Some(480));
        }
    }
    assert_eq!(opus_packet_frames(&[(17 << 3) | 1, 0, 0]), Some(480));
    assert_eq!(opus_packet_frames(&[(16 << 3) | 3, 4, 0]), Some(480));
    assert_eq!(opus_packet_frames(&[19 << 3]), Some(960));
    assert_eq!(opus_packet_frames(&[11 << 3]), Some(2880));
    for packet in [
        &[][..],
        &[(16 << 3) | 2][..],
        &[(16 << 3) | 3][..],
        &[(16 << 3) | 3, 0][..],
        &[(19 << 3) | 3, 7][..],
    ] {
        assert_eq!(opus_packet_frames(packet), None);
    }
}
