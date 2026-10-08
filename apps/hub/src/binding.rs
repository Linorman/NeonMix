//! Control-thread persistence and a bounded fail-closed permission watcher.
use crate::{OutputCommand, Result};
use neonmix_output_binding::{OutputBinding, Store};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub async fn execute(command: OutputCommand) -> Result<()> {
    match command {
        OutputCommand::SyncName {
            directory,
            expected_output_id,
            expected_revision,
        } => {
            let binding = Store::new(directory).with_expected(
                expected_output_id,
                expected_revision,
                |binding| -> Result<_> {
                    sync_native_name(binding)?;
                    Ok(binding.clone())
                },
            )?;
            crate::emit(binding)
        }
        OutputCommand::Add {
            directory,
            credential,
            hub,
            provider,
            device,
            name,
        } => {
            let mut credential = crate::identity::credential(&credential)?;
            let hub = crate::identity::endpoint(&mut credential, hub).await?;
            let snapshot: neonmix_control::Snapshot = crate::sender::client(&credential)?
                .get(format!("{hub}/v1/hub"))
                .bearer_auth(&credential.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            crate::identity::check_hub(&credential, snapshot.hub_id)?;
            let device = match device {
                Some(id) => {
                    crate::virtual_output::resolve_id(provider, &id, crate::backend()?.devices()?)?
                }
                None => crate::virtual_output::resolve(provider, crate::backend()?.devices()?)?,
            };
            let stored =
                Store::new(directory).add(snapshot.hub_id, provider, device.device.id, name)?;
            crate::emit(stored)
        }
        OutputCommand::Show { directory } => crate::emit(Store::new(directory).load()?),
        OutputCommand::Rename {
            directory,
            expected_revision,
            expected_output_id,
            name,
        } => crate::emit(Store::new(directory).rename(
            expected_output_id,
            expected_revision,
            name,
        )?),
        OutputCommand::Enable {
            directory,
            expected_revision,
            expected_output_id,
        } => crate::emit(Store::new(directory).set_enabled(
            expected_output_id,
            expected_revision,
            true,
        )?),
        OutputCommand::Disable {
            directory,
            expected_revision,
            expected_output_id,
        } => crate::emit(Store::new(directory).set_enabled(
            expected_output_id,
            expected_revision,
            false,
        )?),
        OutputCommand::Remove {
            directory,
            expected_revision,
            expected_output_id,
        } => crate::emit(Store::new(directory).remove(expected_output_id, expected_revision)?),
    }
}

pub fn sync_native_name(binding: &OutputBinding) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if binding.provider != neonmix_output_binding::Provider::Neonmix
            || binding.device_id != "coreaudio:com.neonmix.audio.virtual-output"
        {
            return Err(
                "native name sync only applies to the owned NeonMix HAL, not BlackHole".into(),
            );
        }
        neonmix_macos::set_virtual_output_name(&binding.display_name)?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        if binding.provider != neonmix_output_binding::Provider::Neonmix
            || binding.device_id != "pipewire:neonmix.sink.default"
        {
            return Err("native name sync requires the owned NeonMix PipeWire sink".into());
        }
        neonmix_linux::set_virtual_output_name(&binding.display_name)?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        if binding.provider != neonmix_output_binding::Provider::Neonmix {
            return Err("native name sync requires the owned NeonMix render endpoint".into());
        }
        neonmix_windows::set_virtual_output_name(&binding.device_id, &binding.display_name)?;
        Ok(())
    }
}

pub struct Guard {
    allowed: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Guard {
    pub fn new(store: Store, original: OutputBinding) -> Result<Self> {
        let allowed = Arc::new(AtomicBool::new(true));
        let reason = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_allowed, worker_reason, worker_stop) =
            (allowed.clone(), reason.clone(), stop.clone());
        let worker=thread::Builder::new().name("output-binding".into()).spawn(move|| {
            while !worker_stop.load(Ordering::Acquire) {
                let failure=match store.load() {
                    Ok(current) if current.permits(&original)=>None,
                    Ok(_)=>Some("local output was disabled, replaced or rebound; explicit restart required".to_string()),
                    Err(error)=>Some(format!("local output binding is unavailable: {error}")),
                };
                if let Some(failure)=failure {
                    *worker_reason.lock().unwrap_or_else(|p|p.into_inner())=Some(failure);
                    worker_allowed.store(false,Ordering::Release);break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        })?;
        Ok(Self {
            allowed,
            reason,
            stop,
            worker: Some(worker),
        })
    }
    pub fn permits(&self) -> bool {
        self.allowed.load(Ordering::Acquire)
    }
    pub fn reason(&self) -> Option<String> {
        self.reason
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(250);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod cas_tests {
    use super::*;
    #[tokio::test]
    async fn dispatch_rejects_old_object_for_every_mutation_before_native_effects() {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let directory = project
            .join(".local/tmp")
            .join(format!("binding-dispatch-{}", uuid::Uuid::new_v4()));
        let store = Store::new(&directory);
        let a = store
            .add(
                uuid::Uuid::new_v4(),
                neonmix_output_binding::Provider::Blackhole,
                "coreaudio:BlackHole2ch_UID".into(),
                "A".into(),
            )
            .unwrap();
        store.remove(a.output_id, a.revision).unwrap();
        let b = store
            .add(a.hub_id, a.provider, a.device_id, "B".into())
            .unwrap();
        for command in [
            OutputCommand::Rename {
                directory: directory.clone(),
                expected_output_id: a.output_id,
                expected_revision: a.revision,
                name: "old".into(),
            },
            OutputCommand::Enable {
                directory: directory.clone(),
                expected_output_id: a.output_id,
                expected_revision: a.revision,
            },
            OutputCommand::Disable {
                directory: directory.clone(),
                expected_output_id: a.output_id,
                expected_revision: a.revision,
            },
            OutputCommand::Remove {
                directory: directory.clone(),
                expected_output_id: a.output_id,
                expected_revision: a.revision,
            },
            OutputCommand::SyncName {
                directory: directory.clone(),
                expected_output_id: a.output_id,
                expected_revision: a.revision,
            },
        ] {
            let rejected = execute(command).await.unwrap_err();
            assert_eq!(rejected.to_string(), "output_object_replaced");
            assert_eq!(store.load().unwrap(), b);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
