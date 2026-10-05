use std::sync::Arc;

use spake2::{Ed25519Group, Identity, Password, Spake2};
use tillbh_core::channel::{aad, derive, device_keys, TerminalPairing};
use tillbh_core::error::ErrorCode;
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
fn startup_refuses_to_guess_when_both_database_names_exist() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join(DB_FILE);
    let legacy = dir.path().join(concat!("amwa", "pos.db"));

    {
        let core = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        assert!(core.db.schema_version().unwrap() > 0);
    }
    std::fs::copy(&current, &legacy).unwrap();

    let err = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default()))
        .err()
        .expect("two possible financial databases must fail closed");

    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(current.exists());
    assert!(legacy.exists());
}

#[test]
fn hub_secret_is_recovered_from_legacy_credential_key() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let core = AppCore::open(dir.path(), secrets.clone()).unwrap();
    core.setup_initialize(common::setup_request()).unwrap();
    let owner = core.login_users().unwrap().into_iter().next().unwrap();
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

#[test]
fn protocol_v2_device_channel_keeps_its_original_wire_domain() {
    let device_key = "already-paired-device-key";
    let salt = concat!("amwa", "pos/2/device").as_bytes();
    let (t2h, h2t) = device_keys(device_key);

    assert_eq!(t2h, derive(device_key.as_bytes(), salt, "t2h"));
    assert_eq!(h2t, derive(device_key.as_bytes(), salt, "h2t"));

    let expected_aad = format!("{}/2\nPOST\n/sync/push\ndev-1\n123\nnonce-1", concat!("amwa", "pos"));
    assert_eq!(aad("POST", "/sync/push", "dev-1", 123, "nonce-1"), expected_aad.into_bytes());
}

#[test]
fn protocol_v2_pairing_keeps_its_original_spake_identities() {
    let code = "12345678";
    let code_hash = tillbh_core::auth::sha256_hex(code);
    let terminal = TerminalPairing::start(code);
    let terminal_message = terminal.message.clone();
    let pairing_id = terminal.pairing_id.clone();
    let terminal_id = Identity::new(concat!("amwa", "pos-terminal").as_bytes());
    let hub_id = Identity::new(concat!("amwa", "pos-hub").as_bytes());
    let (legacy_hub, hub_message) =
        Spake2::<Ed25519Group>::start_b(&Password::new(code_hash.as_bytes()), &terminal_id, &hub_id);

    let terminal_keys = terminal.finish(&hub_message).expect("protocol-2 pairing identity must remain compatible");
    let shared = legacy_hub.finish(&terminal_message).expect("legacy hub and current terminal must agree");

    assert_eq!(terminal_keys.request, derive(&shared, pairing_id.as_bytes(), "pair-t2h"));
    assert_eq!(terminal_keys.response, derive(&shared, pairing_id.as_bytes(), "pair-h2t"));
}
