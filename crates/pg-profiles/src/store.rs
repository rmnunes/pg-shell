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

/// How a profile authenticates to the server.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// Classic password authentication; the secret lives in the OS keychain.
    #[default]
    Password,
    /// Microsoft Entra ID interactive sign-in (MFA-capable). The short-lived
    /// access token is sent as the Postgres password; only the refresh token
    /// is persisted, in the OS keychain.
    EntraMfa,
}

/// Entra-specific knobs. Everything is optional: the defaults target
/// work/school accounts through the Azure CLI public client, which every
/// tenant already trusts for Azure Database for PostgreSQL.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntraSettings {
    /// Tenant id or verified domain (e.g. `contoso.onmicrosoft.com`).
    /// `None` means `organizations`.
    #[serde(default)]
    pub tenant: Option<String>,
    /// App registration (public client) id. `None` uses the built-in default.
    #[serde(default)]
    pub client_id: Option<String>,
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
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub entra: Option<EntraSettings>,
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
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub entra: Option<EntraSettings>,
}

impl Profile {
    /// Materialize a profile from user input under the given id. Used both by
    /// the store and by callers that need a throwaway profile (e.g. testing
    /// connection parameters before saving).
    pub fn from_input(id: ProfileId, input: ProfileInput) -> Self {
        Self {
            id,
            name: input.name,
            host: input.host,
            port: input.port,
            database: input.database,
            user: input.user,
            ssl_mode: input.ssl_mode,
            app_name: input.app_name,
            group: input.group,
            auth_method: input.auth_method,
            entra: input.entra,
        }
    }
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
        let profile = Profile::from_input(Uuid::new_v4().to_string(), input);
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
            *slot = Profile::from_input(slot.id.clone(), input);
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

    fn password_input(name: &str, port: u16) -> ProfileInput {
        ProfileInput {
            name: name.into(),
            host: "localhost".into(),
            port,
            database: "postgres".into(),
            user: "postgres".into(),
            ssl_mode: SslMode::Prefer,
            app_name: Some("pg-shell".into()),
            group: None,
            auth_method: AuthMethod::Password,
            entra: None,
        }
    }

    #[test]
    fn roundtrip_create_update_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let store = ProfileStore::open_at(path.clone()).unwrap();
        assert!(store.list().is_empty());
        let created = store.create(password_input("local", 5432)).unwrap();
        assert_eq!(store.list().len(), 1);
        let updated = store
            .update(
                &created.id,
                ProfileInput {
                    ssl_mode: SslMode::Require,
                    app_name: None,
                    group: Some("dev".into()),
                    ..password_input("local-updated", 5433)
                },
            )
            .unwrap();
        assert_eq!(updated.name, "local-updated");
        assert_eq!(updated.port, 5433);
        assert_eq!(updated.id, created.id);
        // reopen from disk
        let store2 = ProfileStore::open_at(path).unwrap();
        assert_eq!(store2.list().len(), 1);
        assert_eq!(store2.get(&created.id).unwrap().port, 5433);
        store.delete(&created.id).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn entra_settings_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let store = ProfileStore::open_at(path.clone()).unwrap();
        let created = store
            .create(ProfileInput {
                user: "rodrigo@contoso.com".into(),
                ssl_mode: SslMode::Require,
                auth_method: AuthMethod::EntraMfa,
                entra: Some(EntraSettings {
                    tenant: Some("contoso.onmicrosoft.com".into()),
                    client_id: None,
                }),
                ..password_input("azure", 5432)
            })
            .unwrap();
        let store2 = ProfileStore::open_at(path).unwrap();
        let loaded = store2.get(&created.id).unwrap();
        assert_eq!(loaded.auth_method, AuthMethod::EntraMfa);
        assert_eq!(
            loaded.entra.as_ref().and_then(|e| e.tenant.as_deref()),
            Some("contoso.onmicrosoft.com")
        );
        assert!(loaded.entra.as_ref().unwrap().client_id.is_none());
    }

    #[test]
    fn legacy_profiles_default_to_password_auth() {
        // profiles.json written before `auth_method` existed must keep loading.
        let dir = tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        fs::write(
            &path,
            r#"{"profiles":[{"id":"abc","name":"old","host":"h","port":5432,"database":"d","user":"u"}]}"#,
        )
        .unwrap();
        let store = ProfileStore::open_at(path).unwrap();
        let p = store.get("abc").unwrap();
        assert_eq!(p.auth_method, AuthMethod::Password);
        assert!(p.entra.is_none());
        assert_eq!(p.ssl_mode, SslMode::Prefer);
    }
}
