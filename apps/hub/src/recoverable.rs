use neonmix_core::{mixer::Mixer, signal::StereoSource};
use std::sync::mpsc::SyncSender;
/// CPAL destroys the source with the native stream on its owner/control thread.
/// Returning ownership preserves preallocated lanes across same-device reopens.
pub struct RecoverableMixer {
    pub mixer: Option<Mixer>,
    pub returned: SyncSender<Mixer>,
}
impl StereoSource for RecoverableMixer {
    fn next_frame(&mut self) -> [f32; 2] {
        self.mixer
            .as_mut()
            .map_or([0.0; 2], StereoSource::next_frame)
    }
}
impl Drop for RecoverableMixer {
    fn drop(&mut self) {
        if let Some(mixer) = self.mixer.take() {
            let _ = self.returned.try_send(mixer);
        }
    }
}
