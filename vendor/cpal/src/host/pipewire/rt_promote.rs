//! Background real-time thread promotion.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    thread::{self, JoinHandle, Thread},
};

use crate::{
    host::{emit_error, ErrorCallbackArc},
    Error, ErrorKind, FrameCount, SampleRate,
};

pub(crate) struct RtPromoter {
    pending_frames: Arc<AtomicU32>,
    shutdown: Arc<AtomicBool>,
    handle: Thread,
    join: Option<JoinHandle<()>>,
}

impl RtPromoter {
    /// Must be called on the thread that will run the audio callbacks: `get_current_thread_info`
    /// captures the calling thread's identity, to promote later from the background thread.
    pub(crate) fn spawn(error_callback: ErrorCallbackArc, rate: SampleRate) -> Option<Self> {
        // The callback runs on this main-loop thread. Capture its native TID
        // while alive; the promotion helper is joined before UserData is freed.
        // SAFETY: gettid has no pointer arguments and only reads caller identity.
        let callback_tid = unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t };
        let thread_info = match audio_thread_priority::get_current_thread_info() {
            Ok(info) => info,
            Err(e) => {
                emit_error(&error_callback, Error::from(e));
                return None;
            }
        };

        let pending_frames = Arc::new(AtomicU32::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let pending_frames_bg = pending_frames.clone();
        let shutdown_bg = shutdown.clone();
        let error_callback_bg = error_callback.clone();

        let join = thread::Builder::new()
            .name("cpal_rt_promote".to_owned())
            .spawn(move || loop {
                if shutdown_bg.load(Ordering::Relaxed) {
                    return;
                }
                let frames = pending_frames_bg.swap(0, Ordering::Relaxed);
                if frames != 0 {
                    // Honor an existing RLIMIT_RTPRIO/CAP_SYS_NICE grant before
                    // asking RTKit. Headless sessions may lack a polkit session
                    // even when the administrator granted audio scheduling.
                    let params = libc::sched_param { sched_priority: 10 };
                    // SAFETY: callback_tid remains live until this helper joins;
                    // params is initialized and the kernel enforces permission.
                    let direct = unsafe {
                        libc::sched_setscheduler(
                            callback_tid,
                            libc::SCHED_RR | libc::SCHED_RESET_ON_FORK,
                            &params,
                        )
                    };
                    if direct == 0 {
                        continue;
                    }
                    if let Err(e) = audio_thread_priority::promote_thread_to_real_time(
                        thread_info,
                        frames,
                        rate,
                    ) {
                        emit_error(&error_callback_bg, Error::from(e));
                    }
                    continue;
                }
                thread::park();
            });
        let join = match join {
            Ok(j) => j,
            Err(e) => {
                emit_error(
                    &error_callback,
                    Error::with_message(
                        ErrorKind::ResourceExhausted,
                        format!("failed to create real-time promotion thread: {e}"),
                    ),
                );
                return None;
            }
        };

        Some(Self {
            pending_frames,
            shutdown,
            handle: join.thread().clone(),
            join: Some(join),
        })
    }

    /// Hands a promotion request to the background thread; never blocks.
    pub(crate) fn request(&self, frames: FrameCount) {
        self.pending_frames.store(frames, Ordering::Relaxed);
        self.handle.unpark();
    }
}

impl Drop for RtPromoter {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.handle.unpark();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
