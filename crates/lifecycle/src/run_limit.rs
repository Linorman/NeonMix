//! The media pump has an explicit lifetime; zero never means unlimited.
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunLimit {
    UntilStopped,
    Duration(Duration),
}
impl RunLimit {
    pub fn seconds(seconds: u32) -> Option<Self> {
        (1..=86400)
            .contains(&seconds)
            .then_some(Self::Duration(Duration::from_secs(u64::from(seconds))))
    }
    pub fn elapsed(self, elapsed: Duration) -> bool {
        match self {
            Self::UntilStopped => false,
            Self::Duration(limit) => elapsed >= limit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEndReason {
    UserStopped,
    DurationElapsed,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_survives_multiple_days_while_explicit_duration_has_an_exact_boundary() {
        for seconds in [0, 1, 86399, 86400, 86401, 172800, u64::MAX] {
            assert!(!RunLimit::UntilStopped.elapsed(Duration::from_secs(seconds)));
        }
        let timed = RunLimit::seconds(86400).unwrap();
        assert!(!timed.elapsed(Duration::from_secs(86400) - Duration::from_nanos(1)));
        assert!(timed.elapsed(Duration::from_secs(86400)));
        assert!(RunLimit::seconds(0).is_none());
        assert!(RunLimit::seconds(86401).is_none());
    }
}
