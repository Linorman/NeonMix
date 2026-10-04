use crate::{AudioError, AudioFormat};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Sine,
    Impulse,
    Silence,
}
#[derive(Debug, Clone, Copy, Default)]
pub enum ChannelPattern {
    #[default]
    Both,
    Left,
    Right,
    AntiPhase,
}
/// Source advances on demand from the output callback, independent of callback block size.
pub trait StereoSource: Send {
    /// Predicted speaker presentation time of the next output frame. This is
    /// an in-process Instant, never an independently started worker clock.
    fn set_presentation_time(&mut self, _at: std::time::Instant) {}
    fn next_frame(&mut self) -> [f32; 2];
}
pub struct TestSignal {
    kind: SignalKind,
    rate: u32,
    frequency: f64,
    gain: f32,
    position: u64,
    muted: bool,
    pattern: ChannelPattern,
}
impl TestSignal {
    pub fn new(
        kind: SignalKind,
        rate: u32,
        frequency: f64,
        gain_db: f32,
    ) -> Result<Self, AudioError> {
        AudioFormat::new(rate, 2)?;
        if !frequency.is_finite() || frequency <= 0.0 || frequency >= f64::from(rate) / 2.0 {
            return Err(AudioError::InvalidArgument(
                "frequency must be finite and below Nyquist".into(),
            ));
        }
        if !gain_db.is_finite() || !(-96.0..=0.0).contains(&gain_db) {
            return Err(AudioError::InvalidArgument("gain must be -96..0 dB".into()));
        }
        Ok(Self {
            kind,
            rate,
            frequency,
            gain: 10.0f32.powf(gain_db / 20.0),
            position: 0,
            muted: false,
            pattern: ChannelPattern::Both,
        })
    }
    pub fn with_pattern(mut self, pattern: ChannelPattern) -> Self {
        self.pattern = pattern;
        self
    }
    pub fn position(&self) -> u64 {
        self.position
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
}
impl StereoSource for TestSignal {
    fn next_frame(&mut self) -> [f32; 2] {
        let sample = match self.kind {
            SignalKind::Silence => 0.0,
            SignalKind::Impulse => {
                if self.position.is_multiple_of(u64::from(self.rate)) {
                    self.gain
                } else {
                    0.0
                }
            }
            SignalKind::Sine => {
                let phase = (self.position as f64 * self.frequency / f64::from(self.rate)).fract();
                (phase * std::f64::consts::TAU).sin() as f32 * self.gain
            }
        };
        self.position = self.position.saturating_add(1);
        if self.muted {
            return [0.0; 2];
        }
        match self.pattern {
            ChannelPattern::Both => [sample; 2],
            ChannelPattern::Left => [sample, 0.0],
            ChannelPattern::Right => [0.0, sample],
            ChannelPattern::AntiPhase => [sample, -sample],
        }
    }
}
