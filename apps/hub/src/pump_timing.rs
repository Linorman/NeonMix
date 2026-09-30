//! Bounded wall/CPU timing on non-realtime media workers. A long loop alone
//! cannot distinguish codec work, a blocked native call and a late wakeup.
#![allow(unsafe_code)] // Read only the current thread's CPU clock on macOS.
use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Copy, Default, Serialize)]
pub struct Sample {
    pub phase: &'static str,
    pub at_ns: u64,
    pub wall_ns: u64,
    pub cpu_ns: Option<u64>,
    pub requested_wait_ns: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub receive: Sample,
    pub source: Sample,
    pub outgoing: Sample,
    pub feedback: Sample,
    pub report: Sample,
    pub wait: Sample,
    pub max_wait_lateness_ns: u64,
    pub slow_count: u64,
    pub recent_slow: [Option<Sample>; 8],
}

pub struct Timing {
    origin: Instant,
    mark: Instant,
    cpu: Option<u64>,
    pub snapshot: Snapshot,
}
impl Timing {
    pub fn new() -> Self {
        let origin = Instant::now();
        Self {
            origin,
            mark: origin,
            cpu: thread_cpu_ns(),
            snapshot: Snapshot::default(),
        }
    }
    pub fn finish(&mut self, phase: &'static str) {
        self.record(phase, None);
    }
    pub fn finish_wait(&mut self, requested: std::time::Duration) {
        self.record(
            "wait",
            Some(requested.as_nanos().min(u128::from(u64::MAX)) as u64),
        );
    }
    fn record(&mut self, phase: &'static str, requested_wait_ns: Option<u64>) {
        let now = Instant::now();
        let cpu = thread_cpu_ns();
        let sample = Sample {
            phase,
            at_ns: self
                .mark
                .duration_since(self.origin)
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
            wall_ns: now
                .duration_since(self.mark)
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
            cpu_ns: cpu
                .zip(self.cpu)
                .map(|(end, start)| end.saturating_sub(start)),
            requested_wait_ns,
        };
        let peak = match phase {
            "receive" => &mut self.snapshot.receive,
            "source" => &mut self.snapshot.source,
            "outgoing" => &mut self.snapshot.outgoing,
            "feedback" => &mut self.snapshot.feedback,
            "report" => &mut self.snapshot.report,
            "wait" => &mut self.snapshot.wait,
            _ => unreachable!("fixed media pump phase"),
        };
        if sample.wall_ns > peak.wall_ns {
            *peak = sample;
        }
        let excess = sample
            .wall_ns
            .saturating_sub(requested_wait_ns.unwrap_or(0));
        if requested_wait_ns.is_some() {
            self.snapshot.max_wait_lateness_ns = self.snapshot.max_wait_lateness_ns.max(excess);
        }
        if excess >= 10_000_000 {
            let index = self.snapshot.slow_count as usize % self.snapshot.recent_slow.len();
            self.snapshot.recent_slow[index] = Some(sample);
            self.snapshot.slow_count = self.snapshot.slow_count.saturating_add(1);
        }
        self.mark = now;
        self.cpu = cpu;
    }
}

fn thread_cpu_ns() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: valid writable timespec; reads only this thread's CPU time.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } != 0 {
            return None;
        }
        Some(
            (ts.tv_sec as u64)
                .saturating_mul(1_000_000_000)
                .saturating_add(ts.tv_nsec as u64),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
