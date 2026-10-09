//! Local window preferences; never sent to the audio service.
use neonmix_i18n::LanguagePreference;
use neonmix_identity::files;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

const VERSION: u64 = 2;
const LIMIT: usize = 16 * 1024;

/// Window appearance. `System` follows the OS light/dark setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn preference(self) -> eframe::egui::ThemePreference {
        match self {
            Self::System => eframe::egui::ThemePreference::System,
            Self::Dark => eframe::egui::ThemePreference::Dark,
            Self::Light => eframe::egui::ThemePreference::Light,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UiPreferences {
    #[serde(default = "current_version")]
    pub version: u64,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub language: LanguagePreference,
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}
fn current_version() -> u64 {
    VERSION
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            version: VERSION,
            reduce_motion: false,
            language: LanguagePreference::Auto,
            theme: ThemeChoice::System,
            extra: Default::default(),
        }
    }
}

pub(crate) struct Preferences {
    path: Option<PathBuf>,
    pub value: UiPreferences,
    pub pending: bool,
    pub read_failed: bool,
    language_dirty: bool,
    motion_dirty: bool,
    theme_dirty: bool,
}
impl Preferences {
    pub fn memory() -> Self {
        Self::load(None)
    }
    pub fn load(directory: Option<&Path>) -> Self {
        let path = directory.map(|d| d.join("ui-preferences.json"));
        let loaded = path.as_deref().map(read).transpose();
        let (value, read_failed) = match loaded {
            Ok(v) => (v.flatten().unwrap_or_default(), false),
            Err(_) => (UiPreferences::default(), true),
        };
        Self {
            path,
            value,
            pending: false,
            read_failed,
            language_dirty: false,
            motion_dirty: false,
            theme_dirty: false,
        }
    }
    pub fn language(&mut self, language: LanguagePreference) {
        self.value.language = language;
        self.language_dirty = true;
        self.pending = true;
    }
    pub fn motion(&mut self, on: bool) {
        self.value.reduce_motion = on;
        self.motion_dirty = true;
        self.pending = true;
    }
    pub fn theme(&mut self, theme: ThemeChoice) {
        self.value.theme = theme;
        self.theme_dirty = true;
        self.pending = true;
    }
    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            self.pending = false;
            return Ok(());
        };
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("preferences directory missing"))?;
        if let Some(parent) = directory.parent() {
            std::fs::create_dir_all(parent)?;
        }
        files::private_dir(directory)?;
        let _guard = files::lock(&directory.join("ui-preferences.lock"))?;
        let mut latest = match read(path) {
            Ok(v) => v.unwrap_or_default(),
            Err(error) if self.read_failed && error.kind() == io::ErrorKind::InvalidData => {
                // Explicit save preserves the damaged file for recovery.
                let first_backup = directory.join("ui-preferences.damaged.json");
                let backup = if first_backup.exists() {
                    directory.join(format!(
                        "ui-preferences.damaged-{}.json",
                        uuid::Uuid::new_v4()
                    ))
                } else {
                    first_backup
                };
                files::write_new(&backup, &files::read_private(path, LIMIT)?)?;
                self.value.clone()
            }
            Err(e) => return Err(e),
        };
        if latest.version > VERSION {
            return Err(io::Error::other("newer preferences version"));
        }
        if self.language_dirty {
            latest.language = self.value.language.clone();
        }
        if self.motion_dirty {
            latest.reduce_motion = self.value.reduce_motion;
        }
        if self.theme_dirty {
            latest.theme = self.value.theme;
        }
        latest.version = VERSION;
        let bytes = serde_json::to_vec_pretty(&latest).map_err(io::Error::other)?;
        files::replace(path, &bytes)?;
        self.value = latest;
        self.pending = false;
        self.read_failed = false;
        self.language_dirty = false;
        self.motion_dirty = false;
        self.theme_dirty = false;
        Ok(())
    }
}
fn read(path: &Path) -> io::Result<Option<UiPreferences>> {
    match files::read_private(path, LIMIT) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        #[cfg(unix)]
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => read_legacy(path).map(Some),
        Err(e) => Err(e),
    }
}

/// The old UI used std::fs::write, normally creating a 0644 file. Only its
/// single, non-secret boolean format is accepted inside a private state dir.
#[cfg(unix)]
#[allow(unsafe_code)]
fn read_legacy(path: &Path) -> io::Result<UiPreferences> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Legacy {
        reduce_motion: bool,
    }
    files::validate_private_dir(
        path.parent()
            .ok_or_else(|| io::Error::other("preferences parent missing"))?,
    )?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no pointers or preconditions; it only reads the
    // current process identity, matching the private-directory owner check.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o022 != 0
        || metadata.len() > LIMIT as u64
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe legacy preferences",
        ));
    }
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "preferences too large",
        ));
    }
    let legacy: Legacy = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "not legacy preferences"))?;
    Ok(UiPreferences {
        reduce_motion: legacy.reduce_motion,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        let p = std::env::temp_dir().join(format!("neonmix-preferences-{}", uuid::Uuid::new_v4()));
        files::private_dir(&p).unwrap();
        p
    }
    #[cfg(unix)]
    #[test]
    fn old_plain_writer_mode_is_migrated_without_relaxing_new_files_or_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = directory();
        let path = dir.join("ui-preferences.json");
        std::fs::write(&path, br#"{"reduce_motion":true}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut prefs = Preferences::load(Some(&dir));
        assert!(!prefs.read_failed);
        assert!(prefs.value.reduce_motion);
        assert_eq!(prefs.value.language, LanguagePreference::Auto);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        prefs.language(LanguagePreference::Explicit("en".into()));
        prefs.save().unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        files::replace(&path, br#"{"version":999,"reduce_motion":true}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Preferences::load(Some(&dir)).read_failed);
        std::fs::remove_file(&path).unwrap();
        let target = dir.join("outside.json");
        std::fs::write(&target, br#"{"reduce_motion":true}"#).unwrap();
        symlink(&target, &path).unwrap();
        assert!(Preferences::load(Some(&dir)).read_failed);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_unknown_language_and_interleaved_updates_preserve_fields() {
        let dir = directory();
        let path = dir.join("ui-preferences.json");
        files::write_new(&path, br#"{"reduce_motion":true,"other":42}"#).unwrap();
        let mut a = Preferences::load(Some(&dir));
        let mut b = Preferences::load(Some(&dir));
        assert_eq!(a.value.language, LanguagePreference::Auto);
        a.language(LanguagePreference::Explicit("future-XY".into()));
        a.save().unwrap();
        b.motion(false);
        b.save().unwrap();
        let saved = Preferences::load(Some(&dir));
        assert_eq!(
            saved.value.language,
            LanguagePreference::Explicit("future-XY".into())
        );
        assert!(!saved.value.reduce_motion);
        assert_eq!(saved.value.extra["other"], 42);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn newer_version_is_never_overwritten_and_memory_choice_survives_failure() {
        let dir = directory();
        let path = dir.join("ui-preferences.json");
        let bytes = br#"{"version":999,"language":"auto"}"#;
        files::write_new(&path, bytes).unwrap();
        let mut prefs = Preferences::load(Some(&dir));
        prefs.language(LanguagePreference::Explicit("en".into()));
        assert!(prefs.save().is_err());
        assert!(prefs.pending);
        assert_eq!(files::read_private(&path, LIMIT).unwrap(), bytes);
        assert_eq!(
            prefs.value.language,
            LanguagePreference::Explicit("en".into())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_and_oversized_files_do_not_get_overwritten_on_load() {
        let dir = directory();
        let path = dir.join("ui-preferences.json");
        files::write_new(&path, b"broken json").unwrap();
        let mut prefs = Preferences::load(Some(&dir));
        assert!(prefs.read_failed);
        assert_eq!(files::read_private(&path, LIMIT).unwrap(), b"broken json");
        prefs.motion(true);
        prefs.save().unwrap();
        assert_eq!(
            files::read_private(&dir.join("ui-preferences.damaged.json"), LIMIT).unwrap(),
            b"broken json"
        );
        files::replace(&path, &vec![b' '; LIMIT + 1]).unwrap();
        assert!(Preferences::load(Some(&dir)).read_failed);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
