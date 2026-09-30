//! Ordinary, preemptible media-worker scheduling. This is never applied to the
//! hardware callback and never requests Mach time-constraint scheduling.
#![allow(unsafe_code)] // Current-thread pthread policy and read-only Mach info.
use serde::Serialize;
use std::{marker::PhantomData, rc::Rc};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ThreadPrioritySnapshot {
    pub configured: bool,
    pub policy: Option<i32>,
    pub base_priority: Option<i32>,
    pub error: Option<i32>,
}

/// !Send/!Sync: configuration and restoration belong to the same worker.
/// Explicit pthread scheduling opts out of QoS on macOS. Use only on dedicated
/// media workers; restore the prior scheduling policy/priority before a native
/// worker returns to GStreamer's pool. Do not apply to a UI/Dispatch executor.
pub struct ThreadPriority {
    snapshot: ThreadPrioritySnapshot,
    #[cfg(target_os = "macos")]
    previous: Option<(i32, libc::sched_param)>,
    _thread: PhantomData<Rc<()>>,
}
impl ThreadPriority {
    pub fn enter() -> Self {
        let mut guard = Self {
            snapshot: ThreadPrioritySnapshot::default(),
            #[cfg(target_os = "macos")]
            previous: None,
            _thread: PhantomData,
        };
        #[cfg(target_os = "macos")]
        {
            let mut policy = 0;
            // SAFETY: plain C structures, initialized before use; all pthread
            // functions address this calling thread and valid out parameters.
            let mut previous: libc::sched_param = unsafe { std::mem::zeroed() };
            // SAFETY: both out parameters are writable; pthread_self is borrowed.
            let status = unsafe {
                libc::pthread_getschedparam(libc::pthread_self(), &mut policy, &mut previous)
            };
            if status != 0 {
                guard.snapshot.error = Some(status);
                return guard;
            }
            let mut desired = previous;
            // USER_INTERACTIVE's ordinary base band is 47. CLI task roles can
            // clamp requested QoS to 31 despite a successful setter. SCHED_OTHER
            // keeps timesharing enabled and verifies the actual resulting band.
            desired.sched_priority = 47;
            // SAFETY: SCHED_OTHER and 47 are valid macOS ordinary-thread values;
            // desired lives through the call and only this thread is changed.
            let status = unsafe {
                libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_OTHER, &desired)
            };
            if status != 0 {
                guard.snapshot.error = Some(status);
                return guard;
            }
            guard.previous = Some((policy, previous));
            guard.snapshot = current();
        }
        #[cfg(not(target_os = "macos"))]
        {
            guard.snapshot.configured = true;
        }
        guard
    }
    pub fn snapshot(&self) -> ThreadPrioritySnapshot {
        self.snapshot.clone()
    }
}
impl Drop for ThreadPriority {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some((policy, params)) = self.previous.take() {
            // SAFETY: this !Send guard is dropped on its configuring thread.
            let _ = unsafe { libc::pthread_setschedparam(libc::pthread_self(), policy, &params) };
        }
    }
}

#[cfg(target_os = "macos")]
fn current() -> ThreadPrioritySnapshot {
    // SAFETY: plain C output buffer with the SDK-defined count and current
    // thread's borrowed Mach port (no new send right is allocated).
    let mut info: libc::thread_extended_info = unsafe { std::mem::zeroed() };
    let mut count = libc::THREAD_EXTENDED_INFO_COUNT;
    // SAFETY: buffer and count match THREAD_EXTENDED_INFO; the borrowed thread
    // port remains valid throughout this call.
    let status = unsafe {
        libc::thread_info(
            libc::pthread_mach_thread_np(libc::pthread_self()),
            libc::THREAD_EXTENDED_INFO as u32,
            (&mut info as *mut libc::thread_extended_info).cast(),
            &mut count,
        )
    };
    ThreadPrioritySnapshot {
        configured: status == 0 && info.pth_priority == 47 && info.pth_policy == 1, // POLICY_TIMESHARE, SDK mach/policy.h.
        policy: (status == 0).then_some(info.pth_policy),
        base_priority: (status == 0).then_some(info.pth_priority),
        error: (status != 0).then_some(status),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn media_worker_uses_timesharing_and_restores_its_base_priority() {
        std::thread::spawn(|| {
            let before = current();
            let guard = ThreadPriority::enter();
            let live = guard.snapshot();
            assert!(live.configured, "{live:?}");
            drop(guard);
            let after = current();
            assert_eq!(after.base_priority, before.base_priority);
            assert_eq!(after.policy, before.policy);
        })
        .join()
        .unwrap();
    }
}
