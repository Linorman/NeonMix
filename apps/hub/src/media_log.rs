//! Bounded diagnostics on an ordinary writer thread. A stopped UI reader or
//! slow filesystem must never hold the Sender's paced media thread.
use serde_json::Value;
use std::{
    io::Write,
    sync::mpsc::{SyncSender, sync_channel},
};

pub struct MediaLog {
    send: SyncSender<Value>,
    pub dropped: u64,
}
impl MediaLog {
    pub fn new() -> std::io::Result<Self> {
        let (send, receive) = sync_channel::<Value>(2);
        std::thread::Builder::new()
            .name("media-diagnostics".into())
            .spawn(move || {
                while let Ok(value) = receive.recv() {
                    let mut output = std::io::stdout().lock();
                    if serde_json::to_writer(&mut output, &value).is_err()
                        || output.write_all(b"\n").is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { send, dropped: 0 })
    }
    pub fn publish(&mut self, value: Value) {
        if self.send.try_send(value).is_err() {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}
