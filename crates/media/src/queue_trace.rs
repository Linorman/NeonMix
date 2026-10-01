//! Queue-loss accounting that also works on the GStreamer 1.24 baseline.
//! The `dropped` appsrc/appsink properties only exist in newer runtimes.
//! Private metadata follows each buffer across the queue, without changing
//! audio offsets, timestamps, format or the RTP sample clock.
use gstreamer as gst;
use std::sync::{
    Once,
    atomic::{AtomicU64, Ordering::Relaxed},
};

const META: &str = "NeonMixQueueSequenceMeta";
static REGISTER: Once = Once::new();

#[derive(Default)]
pub(crate) struct QueueTrace {
    produced: AtomicU64,
    next_seen: AtomicU64,
    dropped: AtomicU64,
}

impl QueueTrace {
    pub fn new() -> Self {
        REGISTER.call_once(|| gst::meta::CustomMeta::register(META, &[]));
        Self::default()
    }
    pub fn tag(&self, buffer: &mut gst::BufferRef) -> Result<(), crate::MediaError> {
        let index = self.produced.load(Relaxed);
        let next = index.checked_add(1).ok_or(crate::MediaError::Inactive)?;
        let mut meta = gst::meta::CustomMeta::add(buffer, META)
            .map_err(|error| crate::MediaError::Native(error.to_string()))?;
        meta.mut_structure().set("queue-index", index);
        self.produced.store(next, Relaxed);
        Ok(())
    }
    pub fn observe(&self, buffer: &gst::BufferRef) -> Result<(), crate::MediaError> {
        let meta = gst::meta::CustomMeta::from_buffer(buffer, META)
            .map_err(|error| crate::MediaError::Native(error.to_string()))?;
        let index = meta
            .structure()
            .get::<u64>("queue-index")
            .map_err(|error| crate::MediaError::Native(error.to_string()))?;
        let expected = self.next_seen.load(Relaxed);
        if index < expected {
            return Err(crate::MediaError::InvalidPacket);
        }
        self.dropped.fetch_add(index - expected, Relaxed);
        self.next_seen.store(
            index.checked_add(1).ok_or(crate::MediaError::Inactive)?,
            Relaxed,
        );
        Ok(())
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Relaxed)
    }
    pub fn observe_contiguous(&self, buffer: &gst::BufferRef) -> Result<(), crate::MediaError> {
        self.observe(buffer)?;
        if self.dropped() != 0 {
            return Err(crate::MediaError::QueueFull);
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn produced(&self) -> u64 {
        self.produced.load(Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gst::prelude::*;
    use gstreamer_app::{AppSink, AppSrc};
    use std::{
        sync::{Arc, Condvar, Mutex, mpsc},
        time::Duration,
    };

    fn pipeline(source_capacity: u64, sink_capacity: u32) -> (gst::Pipeline, AppSrc, AppSink) {
        gst::init().unwrap();
        let pipeline = gst::Pipeline::new();
        let source = gst::ElementFactory::make("appsrc")
            .property("is-live", true)
            .property("block", false)
            .property("format", gst::Format::Time)
            .property("max-buffers", source_capacity)
            .property("max-bytes", 0u64)
            .property("leaky-type", gstreamer_app::AppLeakyType::Downstream)
            .build()
            .unwrap()
            .downcast::<AppSrc>()
            .unwrap();
        let sink = gst::ElementFactory::make("appsink")
            .property("sync", false)
            .property("async", false)
            .property("max-buffers", sink_capacity)
            .property("drop", true)
            .property("enable-last-sample", false)
            .property("wait-on-eos", false)
            .build()
            .unwrap()
            .downcast::<AppSink>()
            .unwrap();
        pipeline
            .add_many([
                source.upcast_ref::<gst::Element>(),
                sink.upcast_ref::<gst::Element>(),
            ])
            .unwrap();
        source.link(&sink).unwrap();
        (pipeline, source, sink)
    }

    #[test]
    fn real_appsink_capacity_loss_matches_private_buffer_sequence() {
        let (pipeline, source, sink) = pipeline(32, 4);
        let trace = Arc::new(QueueTrace::new());
        let incoming = trace.clone();
        sink.static_pad("sink")
            .unwrap()
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                incoming.tag(info.buffer_mut().unwrap().make_mut()).unwrap();
                gst::PadProbeReturn::Ok
            });
        pipeline.set_state(gst::State::Playing).unwrap();
        for n in 0..10u8 {
            source
                .push_buffer(gst::Buffer::from_mut_slice(vec![n]))
                .unwrap();
        }
        source.end_of_stream().unwrap();
        pipeline
            .bus()
            .unwrap()
            .timed_pop_filtered(gst::ClockTime::from_seconds(3), &[gst::MessageType::Eos])
            .unwrap();
        assert_eq!(trace.produced(), 10);
        let mut received = Vec::new();
        while let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
            let buffer = sample.buffer().unwrap();
            trace.observe(buffer).unwrap();
            received.push(buffer.map_readable().unwrap()[0]);
        }
        assert_eq!(received, [6, 7, 8, 9]);
        assert_eq!(trace.dropped(), 6);
        if sink.find_property("dropped").is_some() {
            assert_eq!(sink.property::<u64>("dropped"), 6);
        }
        pipeline.set_state(gst::State::Null).unwrap();
    }

    #[test]
    fn real_appsrc_capacity_loss_quarantines_following_audio() {
        let (pipeline, source, sink) = pipeline(4, 32);
        let trace = Arc::new(QueueTrace::new());
        let outgoing = trace.clone();
        let blocked = Arc::new((Mutex::new(false), Condvar::new()));
        let waiting = blocked.clone();
        let (entered, notified) = mpsc::sync_channel(1);
        let first = std::sync::atomic::AtomicBool::new(true);
        source
            .static_pad("src")
            .unwrap()
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if outgoing.observe_contiguous(info.buffer().unwrap()).is_err() {
                    return gst::PadProbeReturn::Drop;
                }
                if first.swap(false, Relaxed) {
                    entered.send(()).unwrap();
                    let (mutex, condition) = &*waiting;
                    let ready = mutex.lock().unwrap();
                    let (ready, timeout) = condition
                        .wait_timeout_while(ready, Duration::from_secs(3), |ready| !*ready)
                        .unwrap();
                    assert!(*ready && !timeout.timed_out());
                }
                gst::PadProbeReturn::Ok
            });
        pipeline.set_state(gst::State::Playing).unwrap();
        let mut first_buffer = gst::Buffer::from_mut_slice(vec![0]);
        trace.tag(first_buffer.make_mut()).unwrap();
        source.push_buffer(first_buffer).unwrap();
        notified.recv_timeout(Duration::from_secs(2)).unwrap();
        for n in 1..10u8 {
            let mut buffer = gst::Buffer::from_mut_slice(vec![n]);
            trace.tag(buffer.make_mut()).unwrap();
            source.push_buffer(buffer).unwrap();
        }
        {
            let (mutex, condition) = &*blocked;
            *mutex.lock().unwrap() = true;
            condition.notify_one();
        }
        source.end_of_stream().unwrap();
        pipeline
            .bus()
            .unwrap()
            .timed_pop_filtered(gst::ClockTime::from_seconds(3), &[gst::MessageType::Eos])
            .unwrap();
        let mut received = Vec::new();
        while let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
            received.push(sample.buffer().unwrap().map_readable().unwrap()[0]);
        }
        assert_eq!(received, [0]);
        assert_eq!(trace.dropped(), 5);
        if source.find_property("dropped").is_some() {
            assert_eq!(source.property::<u64>("dropped"), 5);
        }
        pipeline.set_state(gst::State::Null).unwrap();
    }
}
