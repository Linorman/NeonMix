#![cfg(target_os = "linux")]
//! Explicit native PipeWire host. Never silently fall back to ALSA or PulseAudio.
use neonmix_core::AudioError;
use neonmix_io::NativeBackend;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

/// One stable desktop sink; changing room names or Hub addresses cannot duplicate it.
mod owned_sink;
static ACTIVE_WORKER: AtomicBool = AtomicBool::new(false);

pub const VIRTUAL_NODE_NAME: &str = "neonmix.sink.default";

/// Owns the PipeWire connection for the local output owner, independently of Sender/Hub.
pub struct ManagedSink {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    failure: Arc<Mutex<Option<String>>>,
    rename: mpsc::SyncSender<owned_sink::Rename>,
}

impl ManagedSink {
    pub fn start() -> Result<Self, String> {
        Self::start_named("NeonMix")
    }
    pub fn start_named(name: &str) -> Result<Self, String> {
        validate_name(name)?;
        if ACTIVE_WORKER
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("previous virtual output worker is still retiring".into());
        }
        struct WorkerLease;
        impl Drop for WorkerLease {
            fn drop(&mut self) {
                ACTIVE_WORKER.store(false, Ordering::Release);
            }
        }
        let lease = WorkerLease;
        let name = name.to_owned();
        let (rename, rename_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (ready, response) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let worker_failure = failure.clone();
        let worker = thread::Builder::new()
            .name("virtual-output".into())
            .spawn(move || {
                let _lease = lease;
                let result = owned_sink::run(name, worker_stop, ready.clone(), rename_rx);
                if let Err(error) = result {
                    *worker_failure.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some(error.to_string());
                    let _ = ready.send(Err(error.to_string()));
                }
            })
            .map_err(|error| error.to_string())?;
        match response.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
                failure,
                rename,
            }),
            Ok(Err(error)) => {
                stop.store(true, Ordering::Release);
                retire_worker(worker);
                Err(error)
            }
            Err(error) => {
                stop.store(true, Ordering::Release);
                retire_worker(worker);
                Err(format!("virtual sink readiness timed out: {error}"))
            }
        }
    }
    /// Control-thread request; the owning native loop changes metadata in place.
    pub fn set_name(&self, name: &str) -> Result<(), String> {
        validate_name(name)?;
        let (response, result) = mpsc::sync_channel(1);
        self.rename
            .try_send((name.to_owned(), response))
            .map_err(|e| e.to_string())?;
        result
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?
    }
    pub fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }
    pub fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Drop for ManagedSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            retire_worker(worker);
        }
    }
}

fn retire_worker(worker: thread::JoinHandle<()>) {
    // Stop timers run every 100ms. Control-thread teardown stays bounded even when
    // a native server stalls; closing the owner process also closes its connection.
    let deadline = std::time::Instant::now() + Duration::from_millis(250);
    while !worker.is_finished() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
}
pub fn backend() -> Result<NativeBackend, AudioError> {
    cpal::host_from_id(cpal::HostId::PipeWire)
        .map(|host| NativeBackend::new(host, "pipewire", true).with_device_filter(|device| {
            matches!(device.as_inner(), cpal::platform::DeviceInner::PipeWire(native) if native.is_audio_endpoint())
        }))
        .map_err(|error| AudioError::Backend(error.to_string()))
}

/// E01 user-session sink fixture. Stable node.name; no root, system routing or persistence.
/// The adapter proxy is held until timeout and the server removes it on disconnect.
pub fn run_sink(room: &str, seconds: u32, channels: u16) -> Result<(), Box<dyn std::error::Error>> {
    run_sink_inner(room, Some(seconds), channels, None, None)
}

fn run_sink_inner(
    room: &str,
    seconds: Option<u32>,
    channels: u16,
    stop: Option<Arc<AtomicBool>>,
    ready: Option<mpsc::Sender<Result<(), String>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    use pipewire as pw;
    use std::{cell::RefCell, rc::Rc};
    if room.is_empty()
        || room.len() > 64
        || !room
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("room must be 1..64 ASCII letters, numbers, '-' or '_'".into());
    }
    if seconds.is_some_and(|value| !(1..=86400).contains(&value)) {
        return Err("seconds must be 1..86400".into());
    }
    neonmix_core::AudioFormat::new(48_000, channels)?;
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let errors = Rc::new(RefCell::new(None));
    let errors_clone = errors.clone();
    let loop_clone = mainloop.clone();
    let error_ready = ready.clone();
    let _listener = core
        .add_listener_local()
        .error(move |_, _, res, message| {
            let error = format!("PipeWire {res}: {message}");
            if let Some(tx) = &error_ready {
                let _ = tx.send(Err(error.clone()));
            }
            *errors_clone.borrow_mut() = Some(error);
            loop_clone.quit();
        })
        .register();
    let node_name = format!("neonmix.sink.{room}");
    let node = core.create_object::<pw::node::Node>(
        "adapter",
        &pw::properties::properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => node_name.clone(),
            "node.description" => format!("NeonMix — {room}"),
            "media.class" => "Audio/Sink",
            "audio.channels" => channels.to_string(),
            "audio.position" => if channels == 1 { "[ MONO ]" } else { "[ FL FR ]" },
            "audio.rate" => "48000",
            "node.driver" => "true",
            "node.pause-on-idle" => "false",
            "object.linger" => "false",
            "monitor.channel-volumes" => "true"
        },
    )?;
    // Deletion of our node must end this connection even when the core itself is
    // still alive (for example a session-manager policy reload removes the node).
    use pw::proxy::ProxyT;
    let removed_errors = errors.clone();
    let removed_loop = mainloop.clone();
    let proxy_errors = errors.clone();
    let proxy_loop = mainloop.clone();
    let _node_listener = node
        .upcast_ref()
        .add_listener_local()
        .removed(move || {
            *removed_errors.borrow_mut() =
                Some("selected PipeWire virtual node was removed".into());
            removed_loop.quit();
        })
        .error(move |_, res, message| {
            *proxy_errors.borrow_mut() = Some(format!("PipeWire virtual node {res}: {message}"));
            proxy_loop.quit();
        })
        .register();
    let loop_clone = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        if stop
            .as_ref()
            .is_none_or(|flag| flag.load(Ordering::Acquire))
        {
            loop_clone.quit();
        }
    });
    match seconds {
        Some(value) => timer
            .update_timer(Some(Duration::from_secs(u64::from(value))), None)
            .into_result()?,
        None => timer
            .update_timer(
                Some(Duration::from_millis(100)),
                Some(Duration::from_millis(100)),
            )
            .into_result()?,
    };
    // Creation is asynchronous. Announce readiness only after server roundtrip.
    let pending = core.sync(0)?;
    let _ready = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                if let Some(tx) = &ready {
                    let _ = tx.send(Ok(()));
                } else {
                    println!("{{\"event\":\"sink_ready\",\"device_id\":\"pipewire:{node_name}\"}}");
                }
            }
        })
        .register();
    mainloop.run();
    if let Some(error) = errors.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        Err("invalid virtual output name".into())
    } else {
        Ok(())
    }
}

/// Same-user control socket: only the local owner mutates its native node.
pub fn set_virtual_output_name(name: &str) -> Result<(), String> {
    use std::{
        io::{Read, Write},
        os::{
            linux::net::SocketAddrExt,
            unix::{
                fs::MetadataExt,
                net::{SocketAddr, UnixStream},
            },
        },
    };
    validate_name(name)?;
    let uid = std::fs::metadata("/proc/self")
        .map_err(|e| e.to_string())?
        .uid();
    let address =
        SocketAddr::from_abstract_name(format!("neonmix.virtual-output.owner.{uid}").as_bytes())
            .map_err(|e| e.to_string())?;
    let mut stream = UnixStream::connect_addr(&address).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(name.as_bytes())
        .map_err(|e| e.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream
        .take(513)
        .read_to_end(&mut response)
        .map_err(|e| e.to_string())?;
    if response == b"ok" {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&response).into_owned())
    }
}
