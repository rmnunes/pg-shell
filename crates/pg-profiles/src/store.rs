use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type ProfileId = String;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<Profile>,
}

#[derive(Debug, Error)]
pub enum ProfileStoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unable to resolve app data directory")]
    NoAppDataDir,
    #[error("profile not found: {0}")]
    NotFound(String),
}

pub struct ProfileStore {
    path: PathBuf,
    inner: RwLock<ProfilesFile>,
}

impl ProfileStore {
    pub fn open_default() -> Result<Self, ProfileStoreError> {
        let dirs = ProjectDirs::from("dev", "pg-shell", "pg-shell")
            .ok_or(ProfileStoreError::NoAppDataDir)?;
        let dir = dirs.config_dir().to_path_buf();
        fs::create_dir_all(&dir)?;
        Self::open_at(dir.join("profiles.json"))
    }

    pub fn open_at(path: PathBuf) -> Result<Self, ProfileStoreError> {
        let inner = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            if raw.trim().is_empty() {
                ProfilesFile::default()
            } else {
                serde_json::from_str(&raw)?
            }
        } else {
            ProfilesFile::default()
        };
        Ok(Self {
            path,
            inner: RwLock::new(inner),
        })
    }

    pub fn list(&self) -> Vec<Profile> {
        self.inner.read().profiles.clone()
    }

    pub fn get(&self, id: &str) -> Option<Profile> {
        self.inner
            .read()
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn create(&self, input: ProfileInput) -> Result<Profile, ProfileStoreError> {
        let profile = Profile {
            id: Uuid::new_v4().to_string(),
            name: input.name,
            host: input.host,
            port: input.port,
            database: input.database,
            user: input.user,
            ssl_mode: input.ssl_mode,
            app_name: input.app_name,
            group: input.group,
        };
        {
            let mut guard = self.inner.write();
            guard.profiles.push(profile.clone());
        }
        self.persist()?;
        Ok(profile)
    }

    pub fn update(&self, id: &str, input: ProfileInput) -> Result<Profile, ProfileStoreError> {
        let updated = {
            let mut guard = self.inner.write();
            let slot = guard
                .profiles
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ProfileStoreError::NotFound(id.to_string()))?;
            slot.name = input.name;
            slot.host = input.host;
            slot.port = input.port;
            slot.database = input.database;
            slot.user = input.user;
            slot.ssl_mode = input.ssl_mode;
            slot.app_name = input.app_name;
            slot.group = input.group;
            slot.clone()
        };
        self.persist()?;
        Ok(updated)
    }

    pub fn delete(&self, id: &str) -> Result<(), ProfileStoreError> {
        let removed = {
            let mut guard = self.inner.write();
            let before = guard.profiles.len();
            guard.profiles.retain(|p| p.id != id);
            before != guard.profiles.len()
        };
        if !removed {
            return Err(ProfileStoreError::NotFound(id.to_string()));
        }
        self.persist()?;
        Ok(())
    }

    fn persist(&self) -> Result<(), ProfileStoreError> {
        let snapshot: ProfilesFile = ProfilesFile {
            profiles: self.inner.read().profiles.clone(),
        };
        let json = serde_json::to_string_pretty(&snapshot)?;
        write_atomic(&self.path, json.as_bytes())?;
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.to_path_buf();
    let original_name = tmp
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "profiles.json".into());
    tmp.set_file_name(format!("{original_name}.tmp"));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_create_update_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let store = ProfileStore::open_at(path.clone()).unwrap();
        assert!(store.list().is_empty());
        let created = store
            .create(ProfileInput {
                name: "local".into(),
                host: "localhost".into(),
                port: 5432,
                database: "postgres".into(),
                user: "postgres".into(),
                ssl_mode: SslMode::Prefer,
                app_name: Some("pg-shell".into()),
                group: None,
            })
            .unwrap();
        assert_eq!(store.list().len(), 1);
        let updated = store
            .update(
                &created.id,
                ProfileInput {
                    name: "local-updated".into(),
                    host: "localhost".into(),
                    port: 5433,
                    database: "postgres".into(),
                    user: "postgres".into(),
                    ssl_mode: SslMode::Require,
                    app_name: None,
                    group: Some("dev".into()),
                },
            )
            .unwrap();
        assert_eq!(updated.name, "local-updated");
        assert_eq!(updated.port, 5433);
        // reopen from disk
        let store2 = ProfileStore::open_at(path).unwrap();
        assert_eq!(store2.list().len(), 1);
        assert_eq!(store2.get(&created.id).unwrap().port, 5433);
        store.delete(&created.id).unwrap();
        assert!(store.list().is_empty());
    }
}
