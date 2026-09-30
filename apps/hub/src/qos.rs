//! Process activity lifetime for explicitly running audio.
#![allow(unsafe_code)] // End the owned Foundation activity token.
/// Process-scoped activity while a Sender or receiver is live. Thread QoS alone
/// does not classify the process as user-requested audio work for App Nap.
/// Keep idle sleep allowed: an explicit suspend still follows recovery policy.
pub struct AudioActivity {
    #[cfg(target_os = "macos")]
    token:
        objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
}
impl AudioActivity {
    pub fn begin() -> Self {
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
            let token = objc2::rc::autoreleasepool(|_| {
                NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
                        | NSActivityOptions::LatencyCritical,
                    &NSString::from_str("NeonMix live audio transport"),
                )
            });
            Self { token }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self {}
        }
    }
}
impl Drop for AudioActivity {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: This is the retained token returned by beginActivity above,
            // ended once on its owning non-realtime thread before releasing it.
            unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.token) };
        }
    }
}
