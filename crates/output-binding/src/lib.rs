//! Local output identity and permission state. File I/O is control-thread-only.
pub mod owner;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod owner_linux;

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use uuid::Uuid;
const MAX_BYTES: u64 = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Neonmix,
    Blackhole,
}
impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Neonmix => "neonmix",
            Self::Blackhole => "blackhole",
        })
    }
}
impl FromStr for Provider {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "neonmix" => Ok(Self::Neonmix),
            "blackhole" => Ok(Self::Blackhole),
            _ => Err("provider must be neonmix or blackhole".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputBinding {
    pub schema_version: u16,
    pub revision: u64,
    pub output_id: Uuid,
    pub hub_id: Uuid,
    pub provider: Provider,
    pub device_id: String,
    pub display_name: String,
    pub enabled: bool,
    /// Changes on disable, so a missed disable/enable window still stops an old Sender.
    pub authorization_epoch: u64,
}

impl OutputBinding {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema_version != 1
            || self.revision == 0
            || self.output_id.is_nil()
            || self.hub_id.is_nil()
            || self.authorization_epoch == 0
        {
            return Err(Error::Invalid("invalid binding schema or identity"));
        }
        validate_name(&self.display_name)?;
        if self.device_id.len() > 1024
            || self.device_id.chars().any(char::is_control)
            || !["coreaudio:", "pipewire:", "wasapi:"].iter().any(|prefix| {
                self.device_id.starts_with(prefix) && self.device_id.len() > prefix.len()
            })
        {
            return Err(Error::Invalid("invalid stable device ID"));
        }
        if self.provider == Provider::Blackhole && self.device_id != "coreaudio:BlackHole2ch_UID" {
            return Err(Error::Invalid(
                "BlackHole binding requires its exact stereo UID",
            ));
        }
        Ok(())
    }
    pub fn permits(&self, original: &Self) -> bool {
        self.enabled
            && self.output_id == original.output_id
            && self.hub_id == original.hub_id
            && self.provider == original.provider
            && self.device_id == original.device_id
            && self.authorization_epoch == original.authorization_epoch
    }
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "output name must be nonempty, at most 256 UTF-8 bytes and contain no control characters",
        ));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("invalid output binding: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("revision conflict (current {0})")]
    Conflict(u64),
    #[error("output_object_replaced")]
    ObjectReplaced,
    #[error(
        "one output is already bound; explicitly remove it before selecting another Hub/device"
    )]
    AlreadyBound,
}

#[derive(Clone)]
pub struct Store {
    directory: PathBuf,
}
impl Store {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn load(&self) -> Result<OutputBinding, Error> {
        reject_symlink(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(&self.directory)?.permissions().mode() & 0o077 != 0 {
                return Err(Error::Invalid("binding directory is no longer private"));
            }
        }
        let path = self.directory.join("binding.json");
        reject_symlink(&path)?;
        let file = File::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata()?.permissions().mode() & 0o077 != 0 {
                return Err(Error::Invalid("binding file is no longer private"));
            }
        }
        let mut bytes = Vec::with_capacity(MAX_BYTES as usize + 1);
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES as usize {
            return Err(Error::Invalid("binding exceeds the size limit"));
        }
        let value: OutputBinding = serde_json::from_slice(&bytes)?;
        value.validate()?;
        Ok(value)
    }
    fn lock(&self) -> Result<File, Error> {
        reject_symlink(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.directory)?;
            if fs::metadata(&self.directory)?.permissions().mode() & 0o077 != 0 {
                return Err(Error::Invalid(
                    "binding directory must be private (mode 0700)",
                ));
            }
        }
        #[cfg(not(unix))]
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join("binding.lock");
        reject_symlink(&path)?;
        let file = private_options().create(true).truncate(false).open(path)?;
        file.lock()?;
        Ok(file)
    }
    pub fn add(
        &self,
        hub_id: Uuid,
        provider: Provider,
        device_id: String,
        display_name: String,
    ) -> Result<OutputBinding, Error> {
        let _lock = self.lock()?;
        match self.load() {
            Ok(old) => {
                if old.hub_id == hub_id && old.provider == provider && old.device_id == device_id {
                    return Ok(old);
                }
                return Err(Error::AlreadyBound);
            }
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let binding = OutputBinding {
            schema_version: 1,
            revision: 1,
            output_id: Uuid::new_v4(),
            hub_id,
            provider,
            device_id,
            display_name,
            enabled: true,
            authorization_epoch: 1,
        };
        binding.validate()?;
        self.commit(&binding)?;
        Ok(binding)
    }
    pub fn rename(
        &self,
        expected_output_id: Uuid,
        revision: u64,
        name: String,
    ) -> Result<OutputBinding, Error> {
        validate_name(&name)?;
        self.edit(expected_output_id, revision, |binding| {
            binding.display_name = name
        })
    }
    pub fn set_enabled(
        &self,
        expected_output_id: Uuid,
        revision: u64,
        enabled: bool,
    ) -> Result<OutputBinding, Error> {
        self.edit_result(expected_output_id, revision, |binding| {
            if binding.enabled && !enabled {
                binding.authorization_epoch = binding
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or(Error::Invalid("authorization epoch exhausted"))?;
            }
            binding.enabled = enabled;
            Ok(())
        })
    }
    fn edit(
        &self,
        expected_output_id: Uuid,
        revision: u64,
        operation: impl FnOnce(&mut OutputBinding),
    ) -> Result<OutputBinding, Error> {
        self.edit_result(expected_output_id, revision, |value| {
            operation(value);
            Ok(())
        })
    }
    fn edit_result(
        &self,
        expected_output_id: Uuid,
        revision: u64,
        operation: impl FnOnce(&mut OutputBinding) -> Result<(), Error>,
    ) -> Result<OutputBinding, Error> {
        let _lock = self.lock()?;
        let mut binding = self.load()?;
        check_expected(&binding, expected_output_id, revision)?;
        let old = binding.clone();
        operation(&mut binding)?;
        if binding == old {
            return Ok(old);
        }
        binding.revision = binding
            .revision
            .checked_add(1)
            .ok_or(Error::Invalid("revision exhausted"))?;
        binding.validate()?;
        self.commit(&binding)?;
        Ok(binding)
    }
    pub fn remove(&self, expected_output_id: Uuid, revision: u64) -> Result<OutputBinding, Error> {
        let _lock = self.lock()?;
        let binding = self.load()?;
        check_expected(&binding, expected_output_id, revision)?;
        fs::remove_file(self.directory.join("binding.json"))?;
        self.sync_directory()?;
        Ok(binding)
    }
    /// Native name effects must validate the captured object and keep its
    /// management lock until the effect ends. Validation followed by an
    /// unlocked load/side effect could rename a replacement's native resource.
    pub fn with_expected<T, E: From<Error>>(
        &self,
        expected_output_id: Uuid,
        revision: u64,
        operation: impl FnOnce(&OutputBinding) -> Result<T, E>,
    ) -> Result<T, E> {
        let _lock = self.lock().map_err(E::from)?;
        let binding = self.load().map_err(E::from)?;
        check_expected(&binding, expected_output_id, revision).map_err(E::from)?;
        operation(&binding)
    }
    fn commit(&self, binding: &OutputBinding) -> Result<(), Error> {
        let bytes = serde_json::to_vec_pretty(binding)?;
        let temporary = self
            .directory
            .join(format!("binding-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = private_options().create_new(true).open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.directory.join("binding.json"))?;
            self.sync_directory()?;
            Ok(())
        })();
        let _ = fs::remove_file(temporary);
        result
    }
    fn sync_directory(&self) -> Result<(), Error> {
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn check_expected(binding: &OutputBinding, id: Uuid, revision: u64) -> Result<(), Error> {
    if id.is_nil() {
        return Err(Error::Invalid("expected output ID is required"));
    }
    if binding.output_id != id {
        return Err(Error::ObjectReplaced);
    }
    if binding.revision != revision {
        return Err(Error::Conflict(binding.revision));
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn reject_symlink(path: &Path) -> Result<(), Error> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::Invalid("binding files must not be symlinks"));
    }
    Ok(())
}
