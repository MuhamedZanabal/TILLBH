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

fn legacy_key(key: &str) -> Option<&str> {
    match key {
        "tillbh.hub.master_secret" => Some("amwapos.hub.master_secret"),
        "tillbh.sync.device_key" => Some("amwapos.sync.device_key"),
        // These slot names did not change, but their OS service namespace did.
        tillbh_core::ai::SECRET_API_KEY | tillbh_core::product_images::SECRET_GOOGLE_KEY => Some(key),
        _ if tillbh_core::ai::PROVIDERS
            .iter()
            .any(|p| key == tillbh_core::ai::secret_key_slot(p) || key == tillbh_core::ai::secret_header_slot(p)) =>
        {
            Some(key)
        }
        _ => None,
    }
}

fn delete(service: &str, key: &str) -> AppResult<()> {
    match keyring::Entry::new(service, key).map_err(err)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(err(e)),
    }
}

impl SecretStore for OsSecretStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        if let Some(value) = read(SERVICE, key)? {
            return Ok(Some(value));
        }

        // Only migrate explicitly known credentials. Never fall back
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
        // Remove the fallback first. If deletion fails, keep the current value
        // and return the error; never report success while it can reappear.
        if let Some(old_key) = legacy_key(key) {
            delete(LEGACY_SERVICE, old_key)?;
        }
        delete(SERVICE, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

    // Keyring's built-in mock persists only within one Entry. This backend
    // models service/key persistence across Entries without touching OS secrets.
    type Values = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;
    struct TestBuilder(Values);
    struct TestCredential {
        values: Values,
        slot: (String, String),
    }

    impl keyring::credential::CredentialBuilderApi for TestBuilder {
        fn build(&self, _target: Option<&str>, service: &str, user: &str) -> keyring::Result<Box<keyring::credential::Credential>> {
            Ok(Box::new(TestCredential { values: self.0.clone(), slot: (service.into(), user.into()) }))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl keyring::credential::CredentialApi for TestCredential {
        fn set_secret(&self, value: &[u8]) -> keyring::Result<()> {
            self.values.lock().unwrap().insert(self.slot.clone(), value.to_vec());
            Ok(())
        }
        fn get_secret(&self) -> keyring::Result<Vec<u8>> {
            self.values.lock().unwrap().get(&self.slot).cloned().ok_or(keyring::Error::NoEntry)
        }
        fn delete_credential(&self) -> keyring::Result<()> {
            self.values.lock().unwrap().remove(&self.slot).map(|_| ()).ok_or(keyring::Error::NoEntry)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn test_store() -> MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        static VALUES: OnceLock<Values> = OnceLock::new();
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let values = VALUES.get_or_init(|| {
            let values = Values::default();
            keyring::set_default_credential_builder(Box::new(TestBuilder(values.clone())));
            values
        });
        values.lock().unwrap().clear();
        guard
    }

    fn put(service: &str, key: &str, value: &str) {
        keyring::Entry::new(service, key).unwrap().set_password(value).unwrap();
    }

    #[test]
    fn only_known_renamed_credentials_have_legacy_fallbacks() {
        assert_eq!(legacy_key("tillbh.hub.master_secret"), Some("amwapos.hub.master_secret"));
        assert_eq!(legacy_key("tillbh.sync.device_key"), Some("amwapos.sync.device_key"));
        assert_eq!(legacy_key("unrelated.secret"), None);
    }

    #[test]
    fn migrated_credential_is_verified_and_current_value_takes_precedence() {
        let _guard = test_store();
        let key = "tillbh.hub.master_secret";
        let old = legacy_key(key).unwrap();
        put(LEGACY_SERVICE, old, "legacy-test-value");
        assert_eq!(OsSecretStore.get(key).unwrap().as_deref(), Some("legacy-test-value"));
        assert_eq!(read(SERVICE, key).unwrap().as_deref(), Some("legacy-test-value"));
        assert_eq!(read(LEGACY_SERVICE, old).unwrap().as_deref(), Some("legacy-test-value"));
        OsSecretStore.set(key, "current-test-value").unwrap();
        assert_eq!(OsSecretStore.get(key).unwrap().as_deref(), Some("current-test-value"));
    }

    #[test]
    fn deleting_migrated_credential_does_not_resurrect_legacy_alias() {
        let _guard = test_store();
        let key = "tillbh.sync.device_key";
        put(LEGACY_SERVICE, legacy_key(key).unwrap(), "legacy-test-value");
        assert!(OsSecretStore.get(key).unwrap().is_some());
        OsSecretStore.delete(key).unwrap();
        assert_eq!(OsSecretStore.get(key).unwrap(), None);
        OsSecretStore.delete(key).unwrap();
    }

    #[test]
    fn known_provider_and_image_secrets_survive_the_service_namespace_rename() {
        let _guard = test_store();
        let mut keys = vec!["ai.api_key".to_string(), "catalog.image_search.google_key".to_string()];
        for provider in tillbh_core::ai::PROVIDERS {
            keys.push(tillbh_core::ai::secret_key_slot(provider));
            keys.push(tillbh_core::ai::secret_header_slot(provider));
        }
        for key in keys {
            put(LEGACY_SERVICE, &key, "legacy-test-value");
            assert_eq!(OsSecretStore.get(&key).unwrap().as_deref(), Some("legacy-test-value"), "slot={key}");
            OsSecretStore.delete(&key).unwrap();
            assert_eq!(OsSecretStore.get(&key).unwrap(), None, "deleted slot={key}");
        }
        put(LEGACY_SERVICE, "unrelated.secret", "must-not-migrate");
        assert_eq!(OsSecretStore.get("unrelated.secret").unwrap(), None);
    }
}
