//! Desired targets and actual callback acknowledgments; no wall-clock sleeps.
use neonmix_core::mixer::{Mixer, MixerConfig};
use std::time::{Duration, Instant};
#[test]
fn full_queue_keeps_newest_desired_and_retry_does_not_allocate_a_second_sequence() {
    let origin = Instant::now();
    let (mut mixer, mut control, _, stats) = Mixer::new(origin).unwrap();
    let first = MixerConfig::default();
    for _ in 0..8 {
        control.apply(first).unwrap();
    }
    let next = MixerConfig {
        master_db: -6.,
        ..first
    };
    assert!(control.apply(next).is_err());
    let pending = stats.config_progress();
    assert_eq!(pending.desired_config_sequence, 2);
    assert_eq!(pending.applied_config_sequence, 0);
    assert!(pending.pending());
    assert_eq!(pending.queue_rejections, 1);
    assert!(control.apply(next).is_err());
    assert_eq!(stats.config_progress().desired_config_sequence, 2);
    mixer.render_block(&mut [[0.; 2]; 480]);
    let old = stats.config_progress();
    assert_eq!(old.applied_config_sequence, 1);
    assert!(
        old.pending(),
        "old backlog cannot acknowledge the newest desired target"
    );
    control.apply(next).unwrap();
    assert_eq!(stats.config_progress().desired_config_sequence, 2);
    mixer.render_block(&mut [[0.; 2]; 480]);
    let applied = stats.config_progress();
    assert_eq!(applied.applied_config_sequence, 2);
    assert!(!applied.pending());
    assert!(
        control
            .pending_for(origin + Duration::from_secs(10))
            .is_none()
    );
}
#[test]
fn control_thread_discard_does_not_impersonate_output_callback_application() {
    let origin = Instant::now();
    let (mut mixer, mut control, _, stats) = Mixer::new(origin).unwrap();
    control.apply(MixerConfig::default()).unwrap();
    mixer.discard_backlog();
    assert_eq!(stats.config_progress().applied_config_sequence, 0);
    assert!(stats.config_progress().pending());
    assert!(
        control
            .pending_for(origin + Duration::from_secs(3))
            .unwrap()
            >= Duration::from_secs(2)
    );
    mixer.render_block(&mut [[0.; 2]; 1]);
    assert_eq!(stats.config_progress().applied_config_sequence, 1);
}
#[test]
fn invalid_config_never_changes_desired_or_callback_sequence() {
    let origin = Instant::now();
    let (_, mut control, _, stats) = Mixer::new(origin).unwrap();
    let bad = MixerConfig {
        master_db: f32::NAN,
        ..Default::default()
    };
    assert!(control.apply(bad).is_err());
    assert_eq!(stats.config_progress().desired_config_sequence, 0);
    assert_eq!(stats.config_progress().applied_config_sequence, 0);
}
