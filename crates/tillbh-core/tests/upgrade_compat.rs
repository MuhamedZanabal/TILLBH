use std::sync::Arc;

use tillbh_core::service::{AppCore, MemorySecretStore, SecretStore, DB_FILE};
use tillbh_core::sync::{SECRET_DEVICE_KEY, SECRET_HUB_MASTER};

mod common;

#[test]
fn existing_store_with_legacy_db_filename_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join(DB_FILE);
    let legacy = dir.path().join(concat!("amwa", "pos.db"));

    {
        let core = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        assert_eq!(core.db.path(), current.as_path());
        assert!(core.db.schema_version().unwrap() > 0);
    }

    std::fs::rename(&current, &legacy).unwrap();
    std::fs::write(dir.path().join("store.marker"), "existing store\n").unwrap();

    let core = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default()))
        .expect("a pre-rename store must remain readable after the product rename");

    assert_eq!(core.db.path(), legacy.as_path());
    assert!(core.db.schema_version().unwrap() > 0);
}

#[test]
fn hub_secret_is_recovered_from_legacy_credential_key() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let core = AppCore::open(dir.path(), secrets.clone()).unwrap();
    core.setup_initialize(common::setup_request()).unwrap();
    let owner = core.login_users().unwrap().remove(0);
    let token = core.login(&owner.user_id, common::OWNER_PIN).unwrap().token;
    core.sync_enable_hub(&token).unwrap();

    let expected = secrets.get(SECRET_HUB_MASTER).unwrap().unwrap();
    secrets.delete(SECRET_HUB_MASTER).unwrap();
    secrets.set(concat!("amwa", "pos.hub.master_secret"), &expected).unwrap();

    assert_eq!(core.hub_master_secret().unwrap(), expected);
    assert_eq!(secrets.get(SECRET_HUB_MASTER).unwrap().as_deref(), Some(expected.as_str()));
}

#[test]
fn terminal_secret_is_recovered_from_legacy_credential_key() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let core = AppCore::open(dir.path(), secrets.clone()).unwrap();
    let expected = "existing-device-key";
    secrets.set(concat!("amwa", "pos.sync.device_key"), expected).unwrap();

    assert_eq!(core.terminal_device_key().unwrap(), expected);
    assert_eq!(secrets.get(SECRET_DEVICE_KEY).unwrap().as_deref(), Some(expected));
}
