//! Secrets in the operating system's credential store (Windows Credential
//! Manager on Windows). Secret values are never logged.

use tillbh_core::service::SecretStore;
use tillbh_core::{AppError, AppResult, ErrorCode};

const SERVICE: &str = "TILLBH";
/// Compatibility-only service name used by deployed AMWAPOS installations.
const LEGACY_SERVICE: &str = "AMWAPOS";

pub struct OsSecretStore;

fn err(e: keyring::Error) -> AppError {
    AppError::new(ErrorCode::Io, format!("The Windows credential store is unavailable ({e})."))
}

fn read(service: &str, key: &str) -> AppResult<Option<String>> {
    let entry = keyring::Entry::new(service, key).map_err(err)?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

fn legacy_key(key: &str) -> Option<&'static str> {
    match key {
        "tillbh.hub.master_secret" => Some("amwapos.hub.master_secret"),
        "tillbh.sync.device_key" => Some("amwapos.sync.device_key"),
        _ => None,
    }
}

impl SecretStore for OsSecretStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        if let Some(value) = read(SERVICE, key)? {
            return Ok(Some(value));
        }

        // Only migrate explicitly known long-lived credentials. Never fall back
        // arbitrary keys across product namespaces, and never log secret values.
        let Some(old_key) = legacy_key(key) else {
            return Ok(None);
        };
        let Some(value) = read(LEGACY_SERVICE, old_key)? else {
            return Ok(None);
        };

        let entry = keyring::Entry::new(SERVICE, key).map_err(err)?;
        entry.set_password(&value).map_err(err)?;
        match read(SERVICE, key)? {
            Some(written) if written == value => Ok(Some(value)),
            Some(_) => Err(AppError::new(ErrorCode::Io, "The migrated credential could not be verified.")),
            None => Err(AppError::new(ErrorCode::Io, "The migrated credential was not found after it was written.")),
        }
        // The legacy credential is intentionally retained for rollback. It can
        // be removed only by a future explicit cleanup after upgrade confidence.
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        keyring::Entry::new(SERVICE, key).map_err(err)?.set_password(value).map_err(err)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match keyring::Entry::new(SERVICE, key).map_err(err)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::legacy_key;

    #[test]
    fn only_known_renamed_credentials_have_legacy_fallbacks() {
        assert_eq!(legacy_key("tillbh.hub.master_secret"), Some("amwapos.hub.master_secret"));
        assert_eq!(legacy_key("tillbh.sync.device_key"), Some("amwapos.sync.device_key"));
        assert_eq!(legacy_key("unrelated.secret"), None);
    }
}
