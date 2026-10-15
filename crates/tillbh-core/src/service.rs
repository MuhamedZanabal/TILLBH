//! Application core: owns the database, sessions and device identity, and is
//! the single entry point used by every transport (Tauri IPC, dev HTTP bridge,
//! LAN hub). Each public operation authenticates, authorizes, validates and
//! runs its mutation in one transaction.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::audit::Actor;
use crate::auth::{Session, SessionStore};
use crate::db::{Db, MigrationReport};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::settings;

/// Secure storage for long-lived secrets (Windows Credential Manager in the
/// desktop build). Implementations must never log secret values.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;
}

/// In-memory secret store for tests and the development bridge only.
#[derive(Default)]
pub struct MemorySecretStore(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.0.lock().map_err(|_| AppError::internal("poisoned"))?.get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().map_err(|_| AppError::internal("poisoned"))?.insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> AppResult<()> {
        self.0.lock().map_err(|_| AppError::internal("poisoned"))?.remove(key);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub device_code: String,
    pub name: String,
    pub branch_id: String,
    /// standalone | hub | terminal
    pub mode: String,
}

pub struct AppCore {
    pub db: Db,
    pub sessions: SessionStore,
    pub data_dir: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
    device: RwLock<Option<DeviceIdentity>>,
    pub migration: MigrationReport,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

pub const DB_FILE: &str = "tillbh.db";
/// Compatibility-only source filename used by deployed AMWAPOS installations.
pub const LEGACY_DB_FILE: &str = "amwapos.db";
pub const MARKER_FILE: &str = "store.marker";

fn publish_legacy_snapshot(snapshot: tempfile::NamedTempFile, current: &Path) -> AppResult<()> {
    // persist_noclobber performs the final existence check in the filesystem
    // operation itself; a plain rename can overwrite a newly created DB on Unix.
    snapshot.persist_noclobber(current).map(|_| ()).map_err(|e| {
        AppError::new(ErrorCode::Io, format!("The verified legacy database copy could not be installed at {}: {e}", current.display()))
    })
}

/// Copy a deployed legacy database into the TILLBH filename using SQLite's
/// online backup API. The legacy source is deliberately retained as rollback
/// evidence. A pre-existing TILLBH database always wins and is never replaced.
fn migrate_legacy_database(data_dir: &Path) -> AppResult<()> {
    let current = data_dir.join(DB_FILE);
    if current.exists() {
        return Ok(());
    }
    let legacy = data_dir.join(LEGACY_DB_FILE);
    if !legacy.is_file() {
        return Ok(());
    }

    // Each attempt owns a securely created temporary file; concurrent starts
    // and stale files cannot redirect or delete another attempt's snapshot.
    let snapshot = tempfile::Builder::new().prefix("tillbh-migration-").tempfile_in(data_dir)?;

    let source = Connection::open_with_flags(&legacy, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let source_check: String = source.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if source_check != "ok" {
        return Err(AppError::new(
            ErrorCode::DatabaseCorrupt,
            "The legacy store database failed its integrity check. It was not migrated or modified.",
        )
        .with_details(serde_json::json!({ "quick_check": source_check, "legacy_path": legacy.to_string_lossy() })));
    }
    // SQLite reports an empty file as healthy. Do not mistake a truncated
    // predecessor database (or an unrelated SQLite file) for a new store.
    let schema = source.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row.get::<_, Option<i64>>(0));
    if !matches!(schema, Ok(Some(version)) if version > 0) {
        return Err(AppError::new(
            ErrorCode::DatabaseCorrupt,
            "The legacy file does not contain an initialized store database. No blank store was created; restore a verified backup.",
        ));
    }

    let mut destination = Connection::open(snapshot.path())?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
        match backup.step(-1)? {
            rusqlite::backup::StepResult::Done => {}
            other => {
                return Err(AppError::new(
                    ErrorCode::DatabaseBusy,
                    format!("The legacy database snapshot did not complete ({other:?}). No destination database was installed."),
                ));
            }
        }
    }
    destination.execute_batch("PRAGMA journal_mode=DELETE;")?;
    let destination_check: String = destination.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if destination_check != "ok" {
        return Err(AppError::new(
            ErrorCode::DatabaseCorrupt,
            "The migrated database copy failed its integrity check. The legacy source remains unchanged.",
        )
        .with_details(serde_json::json!({ "quick_check": destination_check })));
    }
    drop(destination);

    publish_legacy_snapshot(snapshot, &current)
}

#[cfg(test)]
mod migration_publication_tests {
    #[test]
    fn destination_created_while_snapshotting_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        std::fs::write(snapshot.path(), b"legacy snapshot").unwrap();
        // A destination appeared after discovery and before publication.
        let current = dir.path().join(super::DB_FILE);
        std::fs::write(&current, b"new current store").unwrap();

        assert!(super::publish_legacy_snapshot(snapshot, &current).is_err());
        assert_eq!(std::fs::read(current).unwrap(), b"new current store");
    }
}

impl AppCore {
    /// Open the store in `data_dir`. A missing database is created only when
    /// the data folder has never been initialized (no marker file); otherwise
    /// startup fails with a recovery error instead of creating an empty store.
    pub fn open(data_dir: &Path, secrets: Arc<dyn SecretStore>) -> AppResult<AppCore> {
        std::fs::create_dir_all(data_dir)?;
        migrate_legacy_database(data_dir)?;
        let db_path = data_dir.join(DB_FILE);
        let marker = data_dir.join(MARKER_FILE);
        let create = !db_path.exists() && !marker.exists();
        if !db_path.exists() && marker.exists() {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "The store database is missing from the data folder, although this installation was set up before. \
                 A blank store was NOT created. Restore a backup or check the data folder.",
            )
            .with_details(serde_json::json!({ "data_dir": data_dir.to_string_lossy(), "recovery": true })));
        }
        let (db, migration) = Db::open(&db_path, create)?;
        db.write(|tx| crate::auth::seed_roles(tx))?;
        let device = db.read(load_device)?;
        Ok(AppCore {
            db,
            sessions: SessionStore::default(),
            data_dir: data_dir.to_path_buf(),
            secrets,
            device: RwLock::new(device),
            migration,
            started_at: crate::time::now(),
        })
    }

    pub fn device(&self) -> Option<DeviceIdentity> {
        self.device.read().ok().and_then(|d| d.clone())
    }

    pub fn require_device(&self) -> AppResult<DeviceIdentity> {
        self.device().ok_or_else(|| AppError::new(ErrorCode::NotSetUp, "This terminal has not been set up yet."))
    }

    pub(crate) fn set_device(&self, d: Option<DeviceIdentity>) {
        if let Ok(mut w) = self.device.write() {
            *w = d;
        }
    }

    pub(crate) fn write_marker(&self) -> AppResult<()> {
        let marker = self.data_dir.join(MARKER_FILE);
        std::fs::write(&marker, format!("TILLBH store initialized {}\n", crate::time::now_str()))?;
        Ok(())
    }

    /// Resolve and validate a session token.
    pub fn session(&self, token: &str) -> AppResult<Session> {
        let idle =
            self.db.read(|c| settings::get::<settings::PosSettings>(c, settings::KEY_POS)).map(|p| p.idle_lock_minutes).unwrap_or(10);
        self.sessions.get_active(token, idle)
    }

    /// Authorize `perm` for the session, or via a manager approval token.
    /// Returns the approver's user id when an approval was consumed.
    pub fn authorize(&self, s: &Session, perm: &str, approval_token: Option<&str>, summary: &str) -> AppResult<Option<String>> {
        if s.has(perm) {
            return Ok(None);
        }
        if let Some(t) = approval_token {
            if let Some((approver, _)) = self.sessions.consume_approval(t, perm) {
                return Ok(Some(approver));
            }
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "The manager approval has expired or does not cover this action. Please approve again.",
            )
            .with_details(serde_json::json!({ "permission": perm, "summary": summary })));
        }
        Err(AppError::approval_required(perm, summary))
    }

    pub fn actor(&self, s: &Session, approved_by: Option<String>) -> Actor {
        Actor { user_id: Some(s.user_id.clone()), device_id: Some(s.device_id.clone()), branch_id: Some(s.branch_id.clone()), approved_by }
    }

    /// Refuse a command of an optional module whose feature flag is off.
    pub fn require_feature(&self, name: &str) -> AppResult<()> {
        let f: crate::settings::FeatureFlags = self.db.read(|c| crate::settings::get(c, crate::settings::KEY_FEATURES))?;
        if f.is_on(name) {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::Conflict,
                format!("The {name} module is not enabled. An owner can turn it on in Settings → Features."),
            )
            .with_details(serde_json::json!({ "kind": "feature_disabled", "feature": name })))
        }
    }

    pub fn features(&self) -> AppResult<crate::settings::FeatureFlags> {
        self.db.read(|c| crate::settings::get(c, crate::settings::KEY_FEATURES))
    }

    pub fn store_timezone(&self, c: &Connection) -> AppResult<String> {
        Ok(c.query_row("SELECT timezone FROM business LIMIT 1", [], |r| r.get(0)).optional()?.unwrap_or_else(|| "Asia/Bahrain".to_string()))
    }

    pub fn currency(&self, c: &Connection) -> AppResult<(String, u32)> {
        Ok(c.query_row("SELECT currency, currency_digits FROM business LIMIT 1", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })
        .optional()?
        .unwrap_or_else(|| ("BHD".to_string(), 3)))
    }

    /// Reject catalogue/back-office mutations on a terminal: in multi-terminal
    /// mode the hub is authoritative for that data.
    pub fn require_back_office_writable(&self) -> AppResult<()> {
        if let Some(d) = self.device() {
            if d.mode == "terminal" {
                return Err(AppError::conflict(
                    "This terminal is connected to a hub. Make catalogue and back-office changes on the hub computer.",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn load_device(c: &Connection) -> AppResult<Option<DeviceIdentity>> {
    let v = settings::get_raw(c, settings::KEY_DEVICE)?;
    match v {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}
