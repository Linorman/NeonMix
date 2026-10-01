#![cfg_attr(target_os = "linux", allow(unsafe_code))]
//! Local virtual-output owner: one process owns the node, Sender only reads it.
use std::path::Path;
#[cfg(any(target_os = "linux", test))]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
};

#[cfg(any(target_os = "linux", test))]
struct OwnerLock {
    _file: File,
}
#[cfg(any(target_os = "linux", test))]
impl OwnerLock {
    fn acquire(directory: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)?;
            let mode =
                std::os::unix::fs::PermissionsExt::mode(&fs::metadata(directory)?.permissions());
            if mode & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "state directory must be private to its owner (mode 0700)",
                ));
            }
        }
        #[cfg(not(unix))]
        fs::create_dir_all(directory)?;
        if fs::symlink_metadata(directory).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "state directory must not be a symlink",
            ));
        }
        let path = directory.join("virtual-output.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "virtual output lock must not be a symlink",
            ));
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "owner lock must be a regular file",
            ));
        }
        #[cfg(unix)]
        if std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "owner lock must be private (mode 0600)",
            ));
        }
        file.try_lock().map_err(|error| {
            io::Error::other(format!("another local output owner is active: {error}"))
        })?;
        // Acquire before truncating: a rejected second owner cannot overwrite live metadata.
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { _file: file })
    }
}

pub fn run(
    directory: &Path,
    binding_directory: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, binding_directory);
        Err("virtual-output owner requires a Linux PipeWire user session; macOS/Windows devices are managed by the OS".into())
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::net::{SocketAddr, UnixListener};
        use std::os::{linux::net::SocketAddrExt, unix::fs::MetadataExt};
        use std::time::{Duration, Instant};
        // An abstract socket makes ownership atomic for this user and node identity,
        // even if two callers choose different state directories. It requires no file
        // outside the project during development and is released automatically on crash.
        let uid = std::fs::metadata("/proc/self")?.uid();
        if uid == 0 {
            return Err("virtual output requires the logged-in PipeWire user, not root".into());
        }
        let address = SocketAddr::from_abstract_name(
            format!("neonmix.virtual-output.owner.{uid}").as_bytes(),
        )?;
        let exclusive_node = UnixListener::bind_addr(&address)?;
        exclusive_node.set_nonblocking(true)?;
        let _owner = OwnerLock::acquire(directory)?;
        let store = binding_directory.map(neonmix_output_binding::Store::new);
        let read_name = || -> Result<String, Box<dyn std::error::Error>> {
            let Some(store) = &store else {
                return Ok("NeonMix".into());
            };
            let binding = match store.load() {
                Ok(binding) => binding,
                Err(neonmix_output_binding::Error::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    return Ok("NeonMix".into());
                }
                Err(error) => return Err(error.into()),
            };
            if binding.provider != neonmix_output_binding::Provider::Neonmix
                || binding.device_id != "pipewire:neonmix.sink.default"
            {
                return Err("owner binding requires the exact NeonMix PipeWire node".into());
            }
            Ok(binding.display_name)
        };
        let mut name = read_name()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let mut termination=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let interrupt=tokio::signal::ctrl_c();tokio::pin!(interrupt);
            let mut sink:Option<neonmix_linux::ManagedSink>=None;
            let mut retry=Instant::now();
            let mut delay=Duration::from_millis(250);
            let mut healthy_since=Instant::now();
            loop {
                if sink.as_ref().is_some_and(|node|!node.is_running()) {
                    crate::emit(serde_json::json!({"event":"virtual_output_lost","device_id":"pipewire:neonmix.sink.default","reason":sink.as_ref().and_then(|node|node.failure())}))?;
                    sink.take();retry=Instant::now()+delay;delay=(delay*2).min(Duration::from_secs(5));
                }
                if sink.is_none() && Instant::now()>=retry {
                    match neonmix_linux::ManagedSink::start_named(&name) {
                        Ok(node)=>{sink=Some(node);healthy_since=Instant::now();crate::emit(serde_json::json!({"event":"virtual_output_ready","device_id":"pipewire:neonmix.sink.default"}))?;},
                        Err(error)=>{crate::emit(serde_json::json!({"event":"virtual_output_unavailable","retry_ms":delay.as_millis(),"reason":error}))?;retry=Instant::now()+delay;delay=(delay*2).min(Duration::from_secs(5));},
                    }
                }
                if let Some(node) = &sink {
                    if healthy_since.elapsed() >= Duration::from_secs(10) { delay=Duration::from_millis(250); }
                    // Recover persisted names after a service restart; an unavailable
                    // binding never removes the local node or reauthorizes a Sender.
                    if store.is_some() && let Ok(current) = read_name() && current != name {
                        node.set_name(&current)?; name=current;
                    }
                }
                if let Ok((mut stream,_))=exclusive_node.accept() {
                    use std::{io::{Read,Write}, os::fd::AsRawFd};
                    // SAFETY: SO_PEERCRED writes a fixed-sized Linux ucred to the
                    // correctly sized buffer while the accepted socket remains live.
                    let same_user=unsafe {
                        let mut peer:libc::ucred=std::mem::zeroed();
                        let mut size=std::mem::size_of_val(&peer) as libc::socklen_t;
                        libc::getsockopt(stream.as_raw_fd(),libc::SOL_SOCKET,libc::SO_PEERCRED,(&mut peer as *mut libc::ucred).cast(),&mut size)==0 && size as usize==std::mem::size_of_val(&peer) && peer.uid==uid
                    };
                    if same_user {
                        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
                        stream.set_write_timeout(Some(Duration::from_millis(200)))?;
                        let mut bytes=Vec::new();
                        let result=(&mut stream).take(257).read_to_end(&mut bytes)
                            .map_err(|e|e.to_string()).and_then(|_|String::from_utf8(bytes).map_err(|e|e.to_string()))
                            .and_then(|requested| {
                                let node=sink.as_ref().ok_or("local virtual output is recovering")?;
                                if store.is_some() && read_name().map_err(|e|e.to_string())? != requested { return Err("name does not match persisted binding".into()); }
                                node.set_name(&requested)?;name=requested;Ok(())
                            });
                        let reply=result.err().unwrap_or_else(||"ok".into());
                        let _=stream.write_all(reply.as_bytes());
                    }
                }
                tokio::select! {
                    result=&mut interrupt=>{result?;break;},
                    _=termination.recv()=>break,
                    _=tokio::time::sleep(Duration::from_millis(100))=>{},
                }
            }
            drop(sink);
            crate::emit(serde_json::json!({"event":"virtual_output_stopped","device_id":"pipewire:neonmix.sink.default"}))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek};

    fn owner_metadata(owner: &mut OwnerLock) -> Vec<u8> {
        // Windows locks reject reads through another handle. Read through the
        // owning handle so the assertion also exercises mandatory file locks.
        owner._file.rewind().unwrap();
        let mut bytes = Vec::new();
        owner._file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn rejected_second_owner_preserves_live_lock_and_drop_releases_it() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/tmp")
            .join(format!("e06-lock-{}", std::process::id()));
        let mut owner = OwnerLock::acquire(&directory).unwrap();
        let before = owner_metadata(&mut owner);
        assert!(OwnerLock::acquire(&directory).is_err());
        assert_eq!(owner_metadata(&mut owner), before);
        drop(owner);
        assert_eq!(
            fs::read(directory.join("virtual-output.lock")).unwrap(),
            before
        );
        drop(OwnerLock::acquire(&directory).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn owner_rejects_symlinked_state_and_public_lock_without_overwriting_it() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/tmp")
            .join(format!("e06-lock-unsafe-{}", std::process::id()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let alias = directory.with_extension("alias");
        symlink(&directory, &alias).unwrap();
        assert!(OwnerLock::acquire(&alias).is_err());
        let lock = directory.join("virtual-output.lock");
        fs::write(&lock, b"existing owner metadata").unwrap();
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(OwnerLock::acquire(&directory).is_err());
        assert_eq!(fs::read(&lock).unwrap(), b"existing owner metadata");
        fs::remove_file(alias).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
