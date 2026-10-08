//! Observation freshness belongs to a source, not to the room's latest GET.
use serde_json::Value;
use std::time::{Duration, Instant};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    NotObserved,
    ReadFailed,
    ServerUnavailable,
    RuntimeChanged,
    InvalidMetadata,
}
#[derive(Clone, Copy, Debug)]
pub struct Sample<T> {
    pub value: T,
    pub received_at: Instant,
    pub sample_age: Duration,
    pub runtime_epoch: Option<Uuid>,
}
#[derive(Clone, Copy, Debug)]
pub enum Measurement<T> {
    Available(Sample<T>),
    Unavailable {
        reason: UnavailableReason,
        last_good: Option<Sample<T>>,
    },
    Stale {
        last_good: Sample<T>,
        age: Duration,
    },
}
impl<T> Measurement<T> {
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Measurement<U> {
        fn sample<T, U>(sample: Sample<T>, map: impl FnOnce(T) -> U) -> Sample<U> {
            Sample {
                value: map(sample.value),
                received_at: sample.received_at,
                sample_age: sample.sample_age,
                runtime_epoch: sample.runtime_epoch,
            }
        }
        match self {
            Self::Available(value) => Measurement::Available(sample(value, map)),
            Self::Unavailable { reason, last_good } => Measurement::Unavailable {
                reason,
                last_good: last_good.map(|value| sample(value, map)),
            },
            Self::Stale { last_good, age } => Measurement::Stale {
                last_good: sample(last_good, map),
                age,
            },
        }
    }
    pub fn current(self) -> Option<T> {
        match self {
            Self::Available(sample) => Some(sample.value),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Stamp {
    received_at: Instant,
    sample_age: Duration,
    runtime_epoch: Option<Uuid>,
}
#[derive(Default, Debug)]
pub struct ObservationClock {
    last_good: Option<Stamp>,
    failure: Option<UnavailableReason>,
}
impl ObservationClock {
    pub fn success_with_age(
        &mut self,
        value: &Value,
        epoch: Option<Uuid>,
        now: Instant,
        age: Duration,
    ) {
        self.success(value, epoch, now);
        if self.failure.is_none()
            && let Some(sample) = self.last_good.as_mut()
        {
            sample.sample_age = age;
        }
    }
    pub fn success(&mut self, value: &Value, epoch: Option<Uuid>, now: Instant) {
        if value.get("available").is_some_and(|v| !v.is_boolean())
            || value
                .get("sample_age_ms")
                .is_some_and(|v| v.as_u64().is_none())
            || value
                .get("runtime_epoch")
                .is_some_and(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()).is_none())
        {
            self.failure = Some(UnavailableReason::InvalidMetadata);
            return;
        }
        if value["available"].as_bool() == Some(false) {
            self.failure = Some(UnavailableReason::ServerUnavailable);
            return;
        }
        self.last_good = Some(Stamp {
            received_at: now,
            sample_age: Duration::from_millis(value["sample_age_ms"].as_u64().unwrap_or(0)),
            runtime_epoch: value["runtime_epoch"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .or(epoch),
        });
        self.failure = None;
    }
    pub fn failed(&mut self) {
        self.failure = Some(UnavailableReason::ReadFailed);
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    #[cfg(test)]
    pub fn last_received(&self) -> Option<Instant> {
        self.last_good.map(|sample| sample.received_at)
    }
    pub fn inspect<'a>(
        &self,
        value: Option<&'a Value>,
        epoch: Option<Uuid>,
        now: Instant,
        period: Duration,
    ) -> Measurement<&'a Value> {
        let sample = value.zip(self.last_good).map(|(value, stamp)| Sample {
            value,
            received_at: stamp.received_at,
            sample_age: stamp.sample_age,
            runtime_epoch: stamp.runtime_epoch,
        });
        if let Some(reason) = self.failure {
            return Measurement::Unavailable {
                reason,
                last_good: sample,
            };
        }
        let Some(sample) = sample else {
            return Measurement::Unavailable {
                reason: UnavailableReason::NotObserved,
                last_good: None,
            };
        };
        if sample.runtime_epoch.zip(epoch).is_some_and(|(a, b)| a != b) {
            return Measurement::Unavailable {
                reason: UnavailableReason::RuntimeChanged,
                last_good: None,
            };
        }
        let age = now
            .saturating_duration_since(sample.received_at)
            .saturating_add(sample.sample_age);
        if age > period.saturating_mul(3) {
            Measurement::Stale {
                last_good: sample,
                age,
            }
        } else {
            Measurement::Available(sample)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_and_snapshot_activity_never_refresh_last_good_measurement() {
        let t = Instant::now();
        let epoch = Uuid::new_v4();
        let v = serde_json::json!({"available":true,"sample_age_ms":200,"errors":0});
        let mut c = ObservationClock::default();
        c.success(&v, Some(epoch), t);
        assert!(
            c.inspect(Some(&v), Some(epoch), t, Duration::from_secs(1))
                .current()
                .is_some()
        );
        c.failed();
        assert!(
            c.inspect(
                Some(&v),
                Some(epoch),
                t + Duration::from_secs(1),
                Duration::from_secs(1)
            )
            .current()
            .is_none()
        );
        assert_eq!(c.last_received(), Some(t));
        c.success(&v, Some(epoch), t);
        assert!(matches!(
            c.inspect(
                Some(&v),
                Some(epoch),
                t + Duration::from_secs(3),
                Duration::from_secs(1)
            ),
            Measurement::Stale { .. }
        ));
        assert!(
            c.inspect(Some(&v), Some(Uuid::new_v4()), t, Duration::from_secs(1))
                .current()
                .is_none()
        );
    }
    #[test]
    fn malformed_metadata_and_unavailable_status_do_not_refresh_the_sample() {
        let t = Instant::now();
        let v = serde_json::json!({"available":true,"frames":0});
        let mut c = ObservationClock::default();
        c.success(&v, None, t);
        for invalid in [
            serde_json::json!({"available":"true"}),
            serde_json::json!({"sample_age_ms":-1}),
            serde_json::json!({"runtime_epoch":"bad"}),
            serde_json::json!({"available":false}),
        ] {
            c.success_with_age(
                &invalid,
                None,
                t + Duration::from_secs(1),
                Duration::from_secs(20),
            );
            assert_eq!(c.last_received(), Some(t));
            assert!(
                c.inspect(Some(&v), None, t, Duration::from_secs(1))
                    .current()
                    .is_none()
            );
        }
    }
    #[test]
    fn explicit_unavailable_keeps_age_and_cannot_turn_into_zero() {
        let t = Instant::now();
        let v = serde_json::json!({"frames":0});
        let mut c = ObservationClock::default();
        c.success(&v, None, t);
        c.success(
            &serde_json::json!({"available":false}),
            None,
            t + Duration::from_secs(1),
        );
        assert_eq!(c.last_received(), Some(t));
        assert!(
            c.inspect(
                Some(&v),
                None,
                t + Duration::from_secs(2),
                Duration::from_secs(1)
            )
            .current()
            .is_none()
        );
    }
}

#[derive(Default)]
pub struct CounterHistory {
    previous: Option<Value>,
    current: Option<Value>,
}
impl CounterHistory {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn update(&mut self, value: &Value) {
        self.previous = self.current.take();
        self.current = Some(value.clone());
    }
    pub fn network_delta(&self) -> Option<(u64, u64)> {
        let current = self.current.as_ref()?["receivers"].as_array()?;
        let previous = self.previous.as_ref()?["receivers"].as_array()?;
        current
            .iter()
            .try_fold((0u64, 0u64), |(lost, late), value| {
                let stream = value["stream_id"].as_u64()?;
                let old = previous
                    .iter()
                    .find(|old| old["stream_id"].as_u64() == Some(stream))?;
                Some((
                    lost.saturating_add(
                        value["lost_packets"]
                            .as_u64()?
                            .checked_sub(old["lost_packets"].as_u64()?)?,
                    ),
                    late.saturating_add(
                        value["late_packets"]
                            .as_u64()?
                            .checked_sub(old["late_packets"].as_u64()?)?,
                    ),
                ))
            })
    }
    pub fn delta(&self, pointer: &str) -> Option<u64> {
        let now = self.current.as_ref()?.pointer(pointer)?.as_u64()?;
        let before = self.previous.as_ref()?.pointer(pointer)?.as_u64()?;
        now.checked_sub(before)
    }
}

impl crate::Desktop {
    pub(crate) fn diagnostic_measurement(&self) -> Measurement<&Value> {
        self.diagnostics_clock.inspect(
            self.diagnostics.as_ref(),
            self.snapshot.as_ref().map(|s| s.runtime_epoch),
            Instant::now(),
            self.poll_interval(),
        )
    }
    pub(crate) fn diagnostics_current(&self) -> Option<&Value> {
        self.diagnostic_measurement().current()
    }
    pub(crate) fn meter_measurement(&self) -> Measurement<&Value> {
        let measurement = self.diagnostic_measurement().map(|value| &value["meters"]);
        let Measurement::Available(mut sample) = measurement else {
            return measurement;
        };
        if sample.value.is_null() {
            return Measurement::Unavailable {
                reason: UnavailableReason::NotObserved,
                last_good: None,
            };
        }
        if let Some(age) = sample.value.get("sample_age_ms") {
            let Some(age) = age.as_u64() else {
                return Measurement::Unavailable {
                    reason: UnavailableReason::NotObserved,
                    last_good: None,
                };
            };
            sample.sample_age = sample.sample_age.saturating_add(Duration::from_millis(age));
        }
        let age = sample
            .received_at
            .elapsed()
            .saturating_add(sample.sample_age);
        if age > self.poll_interval().saturating_mul(3) {
            Measurement::Stale {
                last_good: sample,
                age,
            }
        } else {
            Measurement::Available(sample)
        }
    }
    pub(crate) fn meters_current(&self) -> Option<&Value> {
        self.meter_measurement().current()
    }
    pub(crate) fn sender_measurement(&self) -> Measurement<&Value> {
        self.sender_clock.inspect(
            self.status.as_ref().and_then(|s| s.sender.metrics.as_ref()),
            None,
            Instant::now(),
            Duration::from_secs(1),
        )
    }
    pub(crate) fn sender_metrics_current(&self) -> Option<&Value> {
        self.sender_measurement().current()
    }
    pub(crate) fn airplay_is_current(&self) -> bool {
        self.airplay_clock
            .inspect(
                self.airplay.as_ref(),
                self.snapshot.as_ref().map(|s| s.runtime_epoch),
                Instant::now(),
                self.poll_interval(),
            )
            .current()
            .is_some()
    }
    /// A telemetry deadline is not a permission change. Session/configuration
    /// conditions stay bound in the command and are checked by the server;
    /// stale meters alone must not disable a visible channel's controls.
    pub(crate) fn airplay_controls_available(&self) -> bool {
        if !self.writable() {
            return false;
        }
        let epoch = self.snapshot.as_ref().map(|s| s.runtime_epoch);
        match self.airplay_clock.inspect(
            self.airplay.as_ref(),
            epoch,
            Instant::now(),
            self.poll_interval(),
        ) {
            Measurement::Available(_) | Measurement::Stale { .. } => true,
            Measurement::Unavailable {
                reason: UnavailableReason::ReadFailed,
                last_good: Some(sample),
            } => {
                sample.runtime_epoch == epoch
                    && sample.received_at.elapsed() < Duration::from_secs(4)
            }
            _ => false,
        }
    }
    pub(crate) fn observation_note(&self, measurement: Measurement<&Value>) -> String {
        use crate::Message;
        match measurement {
            Measurement::Available(sample) => self.tr(&Message::DiagnosticsObservationAge {
                seconds: format!(
                    "{:.1}",
                    sample
                        .received_at
                        .elapsed()
                        .saturating_add(sample.sample_age)
                        .as_secs_f32()
                ),
            }),
            Measurement::Stale { age, last_good } => {
                let _ = last_good;
                self.tr(&Message::DiagnosticsObservationStale {
                    seconds: format!("{:.1}", age.as_secs_f32()),
                })
            }
            Measurement::Unavailable { reason, last_good } => {
                let _ = reason;
                if let Some(sample) = last_good {
                    self.tr(&Message::DiagnosticsObservationUnavailableLastGood {
                        seconds: format!(
                            "{:.1}",
                            sample
                                .received_at
                                .elapsed()
                                .saturating_add(sample.sample_age)
                                .as_secs_f32()
                        ),
                    })
                } else {
                    self.tr(&Message::DiagnosticsUnavailable)
                }
            }
        }
    }
}
