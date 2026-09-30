//! Scheduling for native streaming threads, separate from Rust socket pumps.
// Applies only to dedicated GStreamer streaming workers in the media process.
use gst::prelude::ElementExt;
use gstreamer as gst;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::Relaxed},
};

#[derive(Default)]
pub struct Scheduling {
    entered: AtomicU64,
    configured: AtomicU64,
    failed: AtomicU64,
    min_base_priority: AtomicU64,
    max_base_priority: AtomicU64,
}
#[derive(Clone, Default, serde::Serialize)]
pub struct SchedulingSnapshot {
    pub entered: u64,
    pub configured: u64,
    pub failed: u64,
    pub min_base_priority: u64,
    pub max_base_priority: u64,
}
impl Scheduling {
    pub fn snapshot(&self) -> SchedulingSnapshot {
        SchedulingSnapshot {
            entered: self.entered.load(Relaxed),
            configured: self.configured.load(Relaxed),
            failed: self.failed.load(Relaxed),
            min_base_priority: self.min_base_priority.load(Relaxed),
            max_base_priority: self.max_base_priority.load(Relaxed),
        }
    }
    pub fn install(pipeline: &gst::Pipeline) -> Arc<Self> {
        let stats = Arc::new(Self::default());
        let observe = stats.clone();
        if let Some(bus) = pipeline.bus() {
            bus.set_sync_handler(move |_, message| {
                if let gst::MessageView::StreamStatus(status) = message.view() {
                    match status.get().0 {
                        gst::StreamStatusType::Enter => {
                            observe.entered.fetch_add(1, Relaxed);
                            let configured = enter();
                            if let Some(priority) = configured.base_priority {
                                let priority = priority.max(0) as u64;
                                let _ = observe
                                    .min_base_priority
                                    .compare_exchange(0, priority, Relaxed, Relaxed);
                                observe.min_base_priority.fetch_min(priority, Relaxed);
                                observe.max_base_priority.fetch_max(priority, Relaxed);
                            }
                            if configured.configured {
                                observe.configured.fetch_add(1, Relaxed);
                            } else {
                                observe.failed.fetch_add(1, Relaxed);
                            }
                        }
                        gst::StreamStatusType::Leave => leave(),
                        _ => {}
                    }
                }
                gst::BusSyncReply::Pass
            });
        }
        stats
    }
}
std::thread_local! {
    static PRIORITY: std::cell::RefCell<Option<crate::ThreadPriority>> = const { std::cell::RefCell::new(None) };
}
fn enter() -> crate::ThreadPrioritySnapshot {
    PRIORITY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(guard) = slot.as_ref() {
            return guard.snapshot();
        }
        let guard = crate::ThreadPriority::enter();
        let snapshot = guard.snapshot();
        *slot = Some(guard);
        snapshot
    })
}
fn leave() {
    PRIORITY.with(|slot| {
        slot.borrow_mut().take();
    });
}
