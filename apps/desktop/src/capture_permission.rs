//! Ask only for an explicit Sender start, before the background's start deadline.
//! The frozen lifecycle ticket still rejects a start cancelled during the prompt.
use neonmix_desktop_service::Request;

pub(crate) fn before_action(request: &Request) -> Result<(), &'static str> {
    authorize_sender_start(request, authorize)
}

fn authorize_sender_start(
    request: &Request,
    authorize: impl FnOnce() -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let request = match request {
        Request::LifecycleStart { request, .. } => request.as_ref(),
        other => other,
    };
    if matches!(request, Request::SenderStart { .. }) {
        authorize()?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn authorize() -> Result<(), &'static str> {
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn authorize() -> Result<(), &'static str> {
    use objc2_avf_audio::{AVAudioApplication, AVAudioApplicationRecordPermission as Permission};
    // SAFETY: Available since macOS 14, below our deployment target. The getter
    // and completion API are used on the desktop's non-realtime worker thread.
    let permission = unsafe { AVAudioApplication::sharedInstance().recordPermission() };
    if permission == Permission::Granted {
        return Ok(());
    }
    if permission != Permission::Undetermined {
        return Err("capture_permission_denied");
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let completion = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
        // A late callback only reports permission; it can never start capture.
        let _ = tx.send(granted.as_bool());
    });
    // SAFETY: AVFoundation copies the block; all captures are owned and Send.
    // No audio device is opened by this request. Only macOS can grant access.
    unsafe { AVAudioApplication::requestRecordPermissionWithCompletionHandler(&completion) };
    permission_result(rx.recv_timeout(std::time::Duration::from_secs(60)))
}

#[cfg(any(target_os = "macos", test))]
fn permission_result(
    result: Result<bool, std::sync::mpsc::RecvTimeoutError>,
) -> Result<(), &'static str> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err("capture_permission_denied"),
        Err(_) => Err("capture_permission_pending"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neonmix_desktop_service::SenderOptions;

    fn start() -> Request {
        Request::SenderStart {
            options: SenderOptions {
                credential: "profiles/sender.json".into(),
                hub: None,
                output_binding: "output".into(),
            },
        }
    }

    #[test]
    fn only_explicit_sender_starts_request_permission() {
        for request in [
            Request::Status,
            Request::Discover { seconds: 3 },
            Request::HubStart,
            Request::SenderStop,
            Request::Shutdown,
        ] {
            assert!(authorize_sender_start(&request, || panic!("unexpected prompt")).is_ok());
        }
        let ticket = Request::LifecycleStart {
            instance_generation: uuid::Uuid::new_v4(),
            expected_stop_generation: 7,
            request: Box::new(start()),
        };
        for request in [start(), ticket] {
            assert_eq!(
                authorize_sender_start(&request, || Err("capture_permission_denied")),
                Err("capture_permission_denied")
            );
        }
    }

    #[test]
    fn denial_and_unanswered_prompt_never_allow_dispatch() {
        assert_eq!(permission_result(Ok(true)), Ok(()));
        assert_eq!(
            permission_result(Ok(false)),
            Err("capture_permission_denied")
        );
        assert_eq!(
            permission_result(Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
            Err("capture_permission_pending")
        );
        assert_eq!(
            permission_result(Err(std::sync::mpsc::RecvTimeoutError::Disconnected)),
            Err("capture_permission_pending")
        );
    }
}
