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
    pub nice: Option<i32>,
}

/// !Send/!Sync: configuration and restoration belong to the same worker.
/// Explicit pthread scheduling opts out of QoS on macOS. Use only on dedicated
/// media workers; restore the prior scheduling policy/priority before a native
/// worker returns to GStreamer's pool. Do not apply to a UI/Dispatch executor.
pub struct ThreadPriority {
    snapshot: ThreadPrioritySnapshot,
    #[cfg(target_os = "macos")]
    previous: Option<(i32, libc::sched_param)>,
    #[cfg(target_os = "linux")]
    previous_nice: Option<i32>,
    _thread: PhantomData<Rc<()>>,
}
impl ThreadPriority {
    pub fn enter() -> Self {
        let mut guard = Self {
            snapshot: ThreadPrioritySnapshot::default(),
            #[cfg(target_os = "macos")]
            previous: None,
            #[cfg(target_os = "linux")]
            previous_nice: None,
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
        #[cfg(target_os = "linux")]
        {
            let before = linux_nice();
            match before {
                Ok(before) => {
                    let wanted = before.min(-10);
                    // Linux nice is per-thread. Prefer an existing RLIMIT_NICE
                    // grant, then the desktop's RTKit broker; keep SCHED_OTHER.
                    // SAFETY: who=0 addresses only the calling thread, and nice
                    // is in range because it comes from getpriority or -10.
                    let configured = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, wanted) }
                        == 0
                        || linux_rtkit_nice(wanted);
                    if configured {
                        guard.previous_nice = Some(before);
                    }
                    guard.snapshot = linux_current();
                    guard.snapshot.configured = configured && guard.snapshot.nice == Some(wanted);
                    if !guard.snapshot.configured && guard.snapshot.error.is_none() {
                        guard.snapshot.error = Some(libc::EPERM);
                    }
                }
                Err(error) => guard.snapshot.error = Some(error),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
        #[cfg(target_os = "linux")]
        if let Some(previous) = self.previous_nice.take() {
            // SAFETY: restoring a less privileged nice value on this !Send
            // guard's own thread needs no privilege and cannot change another.
            let _ = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, previous) };
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
        nice: None,
    }
}

#[cfg(target_os = "linux")]
fn linux_nice() -> Result<i32, i32> {
    // SAFETY: errno is thread-local; -1 is also a valid nice value.
    unsafe {
        *libc::__errno_location() = 0;
        let nice = libc::getpriority(libc::PRIO_PROCESS, 0);
        let error = *libc::__errno_location();
        if error == 0 { Ok(nice) } else { Err(error) }
    }
}

#[cfg(target_os = "linux")]
fn linux_current() -> ThreadPrioritySnapshot {
    let mut params = libc::sched_param { sched_priority: 0 };
    // SAFETY: current-thread queries with a valid writable output structure.
    let policy = unsafe { libc::sched_getscheduler(0) };
    // SAFETY: query this thread into the initialized, writable sched_param.
    let status = unsafe { libc::sched_getparam(0, &mut params) };
    let nice = linux_nice();
    ThreadPrioritySnapshot {
        policy: (policy >= 0).then_some(policy),
        base_priority: (status == 0).then_some(params.sched_priority),
        nice: nice.as_ref().ok().copied(),
        error: nice.err(),
        configured: false,
    }
}

#[cfg(target_os = "linux")]
fn linux_rtkit_nice(priority: i32) -> bool {
    let Ok(connection) = dbus::blocking::Connection::new_system() else {
        return false;
    };
    let proxy = connection.with_proxy(
        "org.freedesktop.RealtimeKit1",
        "/org/freedesktop/RealtimeKit1",
        std::time::Duration::from_secs(1),
    );
    // SAFETY: Linux gettid has no pointer arguments or side effects.
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u64;
    let result: Result<(), _> = proxy.method_call(
        "org.freedesktop.RealtimeKit1",
        "MakeThreadHighPriority",
        (tid, priority),
    );
    result.is_ok()
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    #[test]
    fn nice_guard_reports_real_permission_and_restores_the_thread() {
        std::thread::spawn(|| {
            let before = linux_current();
            let guard = ThreadPriority::enter();
            let live = guard.snapshot();
            assert_eq!(live.policy, before.policy);
            assert_eq!(live.nice, linux_nice().ok());
            if live.configured {
                assert!(live.nice.unwrap() <= -10);
            } else {
                assert!(live.error.is_some());
                assert_eq!(live.nice, before.nice);
            }
            drop(guard);
            assert_eq!(linux_current().nice, before.nice);
        })
        .join()
        .unwrap();
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
