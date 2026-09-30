//! Main-loop-owned native adapter: exporting a local impl-node lets its name
//! change in place. No foreign user memory is shared with the audio callbacks.
#![allow(unsafe_code)]
use pipewire as pw;
use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, c_void},
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Events {
    mainloop: pw::main_loop::MainLoopRc,
    error: Rc<RefCell<Option<String>>>,
    bound: Rc<Cell<bool>>,
}
unsafe extern "C" fn removed(data: *mut c_void) {
    // SAFETY: boxed Events outlives the registered hook; invoked on the owning loop.
    let events = unsafe { &*(data.cast::<Events>()) };
    *events.error.borrow_mut() = Some("owned virtual node was removed".into());
    events.mainloop.quit();
}
unsafe extern "C" fn bound(data: *mut c_void, _id: u32) {
    // SAFETY: same boxed callback state and main-loop contract as removed.
    unsafe { &*(data.cast::<Events>()) }.bound.set(true);
}
unsafe extern "C" fn error(
    data: *mut c_void,
    _seq: i32,
    res: i32,
    message: *const std::ffi::c_char,
) {
    // SAFETY: callback state and PipeWire's NUL-terminated message live for this call.
    let events = unsafe { &*(data.cast::<Events>()) };
    let message = if message.is_null() {
        "unknown error".into()
    } else {
        // SAFETY: PipeWire supplies a readable C string for the callback duration.
        unsafe { CStr::from_ptr(message) }.to_string_lossy()
    };
    *events.error.borrow_mut() = Some(format!("owned virtual node {res}: {message}"));
    events.mainloop.quit();
}
static PROXY_EVENTS: pw::sys::pw_proxy_events = pw::sys::pw_proxy_events {
    version: 0,
    destroy: None,
    bound: Some(bound),
    removed: Some(removed),
    done: None,
    error: Some(error),
    bound_props: None,
};

struct OwnedNode {
    proxy: NonNull<pw::sys::pw_proxy>,
    node: NonNull<pw::sys::pw_impl_node>,
    hook: Box<pw::spa::sys::spa_hook>,
    _events: Box<Events>,
    _core: pw::core::CoreRc,
    _context: pw::context::ContextRc,
}
impl OwnedNode {
    fn new(
        context: &pw::context::ContextRc,
        core: &pw::core::CoreRc,
        events: Events,
        name: &str,
    ) -> Result<Self, String> {
        let properties = pw::properties::properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => super::VIRTUAL_NODE_NAME,
            "node.description" => name, "node.nick" => name,
            "media.class" => "Audio/Sink", "audio.channels" => "2",
            "audio.position" => "[ FL FR ]", "audio.rate" => "48000",
            "node.driver" => "true", "node.pause-on-idle" => "false",
            "object.linger" => "false", "monitor.channel-volumes" => "true"
        };
        // SAFETY: all objects belong to this loop. Module lifetime is owned by
        // context; the factory consumes properties even if creation fails.
        unsafe {
            let mut factory =
                pw::sys::pw_context_find_factory(context.as_raw_ptr(), c"adapter".as_ptr());
            if factory.is_null() {
                if pw::sys::pw_context_load_module(
                    context.as_raw_ptr(),
                    c"libpipewire-module-adapter".as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
                .is_null()
                {
                    return Err("cannot load local PipeWire adapter module".into());
                }
                factory =
                    pw::sys::pw_context_find_factory(context.as_raw_ptr(), c"adapter".as_ptr());
            }
            if factory.is_null() {
                return Err("local adapter factory is unavailable".into());
            }
            let node = NonNull::new(
                pw::sys::pw_impl_factory_create_object(
                    factory,
                    std::ptr::null_mut(),
                    c"PipeWire:Interface:Node".as_ptr(),
                    3,
                    properties.into_raw(),
                    0,
                )
                .cast::<pw::sys::pw_impl_node>(),
            )
            .ok_or("cannot create local virtual adapter")?;
            let proxy = pw::sys::pw_core_export(
                core.as_raw_ptr(),
                c"PipeWire:Interface:Node".as_ptr(),
                std::ptr::null(),
                node.as_ptr().cast(),
                0,
            );
            let Some(proxy) = NonNull::new(proxy) else {
                pw::sys::pw_impl_node_destroy(node.as_ptr());
                return Err("cannot export virtual adapter to user PipeWire session".into());
            };
            let mut hook = Box::new(std::mem::zeroed());
            let mut events = Box::new(events);
            pw::sys::pw_proxy_add_listener(
                proxy.as_ptr(),
                &mut *hook,
                &PROXY_EVENTS,
                (&mut *events as *mut Events).cast(),
            );
            Ok(Self {
                proxy,
                node,
                hook,
                _events: events,
                _core: core.clone(),
                _context: context.clone(),
            })
        }
    }
    fn set_name(&self, name: &str) -> Result<(), String> {
        let properties =
            pw::properties::properties! {"node.description" => name, "node.nick" => name};
        // SAFETY: local node remains owned here and updates occur only on its main loop.
        let result = unsafe {
            pw::sys::pw_impl_node_update_properties(
                self.node.as_ptr(),
                properties.dict().as_raw_ptr(),
            )
        };
        if result < 0 {
            Err(format!("native node rename failed: {result}"))
        } else {
            Ok(())
        }
    }
}
impl Drop for OwnedNode {
    fn drop(&mut self) {
        pw::spa::utils::hook::remove(*self.hook);
        // SAFETY: no registered callbacks remain. Destroy export before its
        // implementation, while the retained context/core are still live.
        unsafe {
            pw::sys::pw_proxy_destroy(self.proxy.as_ptr());
            pw::sys::pw_impl_node_destroy(self.node.as_ptr());
        }
    }
}

pub(super) type Rename = (String, mpsc::SyncSender<Result<(), String>>);
pub(super) fn run(
    name: String,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
    rename: mpsc::Receiver<Rename>,
) -> Result<(), String> {
    let execute = || -> Result<(), Box<dyn std::error::Error>> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let errors = Rc::new(RefCell::new(None));
        let bound = Rc::new(Cell::new(false));
        let errors_clone = errors.clone();
        let loop_clone = mainloop.clone();
        let _error = core
            .add_listener_local()
            .error(move |_, _, res, message| {
                *errors_clone.borrow_mut() = Some(format!("PipeWire {res}: {message}"));
                loop_clone.quit();
            })
            .register();
        let node = Rc::new(OwnedNode::new(
            &context,
            &core,
            Events {
                mainloop: mainloop.clone(),
                error: errors.clone(),
                bound: bound.clone(),
            },
            &name,
        )?);
        let pending = core.sync(0)?;
        let loop_ready = mainloop.clone();
        let _ready = core
            .add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    if bound.get() {
                        let _ = ready.send(Ok(()));
                    } else {
                        let _ = ready.send(Err("virtual node was not bound after export".into()));
                        loop_ready.quit();
                    }
                }
            })
            .register();
        let loop_timer = mainloop.clone();
        let node_timer = node.clone();
        let timer = mainloop.loop_().add_timer(move |_| {
            if stop.load(Ordering::Acquire) {
                loop_timer.quit();
                return;
            }
            // Bounded requests, on the control loop, never on audio callbacks.
            if let Ok((name, response)) = rename.try_recv() {
                let _ = response.send(node_timer.set_name(&name));
            }
        });
        timer
            .update_timer(
                Some(Duration::from_millis(100)),
                Some(Duration::from_millis(100)),
            )
            .into_result()?;
        mainloop.run();
        drop(timer);
        drop(node);
        if let Some(error) = errors.borrow_mut().take() {
            return Err(error.into());
        }
        Ok(())
    };
    execute().map_err(|error| error.to_string())
}
