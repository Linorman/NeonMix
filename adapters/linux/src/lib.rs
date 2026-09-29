#![cfg(target_os = "linux")]
//! Explicit native PipeWire host. Never silently fall back to ALSA or PulseAudio.
use neonmix_core::AudioError;
use neonmix_io::NativeBackend;
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
    use pipewire as pw;
    use std::{cell::RefCell, rc::Rc, time::Duration};
    if room.is_empty()
        || room.len() > 64
        || !room
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("room must be 1..64 ASCII letters, numbers, '-' or '_'".into());
    }
    if !(1..=86400).contains(&seconds) {
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
    let _listener = core
        .add_listener_local()
        .error(move |_, _, res, message| {
            *errors_clone.borrow_mut() = Some(format!("PipeWire {res}: {message}"));
            loop_clone.quit();
        })
        .register();
    let node_name = format!("neonmix.sink.{room}");
    let _node = core.create_object::<pw::node::Node>(
        "adapter",
        &pw::properties::properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => node_name.clone(),
            "node.description" => format!("NeonMix — {room}"),
            "media.class" => "Audio/Sink",
            "audio.channels" => channels.to_string(),
            "audio.position" => if channels == 1 { "[ MONO ]" } else { "[ FL FR ]" },
            "audio.rate" => "48000",
            "object.linger" => "false",
            "monitor.channel-volumes" => "true"
        },
    )?;
    let loop_clone = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| loop_clone.quit());
    timer
        .update_timer(Some(Duration::from_secs(u64::from(seconds))), None)
        .into_result()?;
    // Creation is asynchronous. Announce readiness only after server roundtrip.
    let pending = core.sync(0)?;
    let _ready = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                println!("{{\"event\":\"sink_ready\",\"device_id\":\"pipewire:{node_name}\"}}");
            }
        })
        .register();
    mainloop.run();
    if let Some(error) = errors.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}
