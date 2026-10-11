//! Regression tests for stable identifiers that must survive the AMWAPOS → TILLBH rename.
//!
//! These are compatibility identifiers, not user-visible branding. Protocol v2 was
//! already deployed using the AMWAPOS cryptographic domain. Changing that domain
//! without incrementing the protocol version makes existing paired terminals and
//! hubs unable to authenticate/decrypt each other.

use std::sync::Arc;

use tempfile::tempdir;
use tillbh_core::channel::{aad, device_keys};
use tillbh_core::db::Db;
use tillbh_core::service::{AppCore, MemorySecretStore, DB_FILE, MARKER_FILE};

#[test]
fn protocol_v2_preserves_deployed_crypto_domain() {
    let request_aad = aad("POST", "/sync/push", "device-1", 123, "nonce-1");
    assert_eq!(
        request_aad,
        b"amwapos/2\nPOST\n/sync/push\ndevice-1\n123\nnonce-1",
        "protocol v2 AAD is a deployed cryptographic compatibility constant",
    );

    let (terminal_to_hub, hub_to_terminal) = device_keys("legacy-device-key");
    assert_eq!(
        hex::encode(terminal_to_hub),
        "54849c23e6285dc87026b8961997e1d19abf31c4bbe0beeeab3f4bf7a523c16c",
        "protocol v2 terminal→hub key derivation must stay compatible",
    );
    assert_eq!(
        hex::encode(hub_to_terminal),
        "031059aec7e5edaa266b319680795fcc59733fe45db3a76974cbddb0f6f1c6eb",
        "protocol v2 hub→terminal key derivation must stay compatible",
    );
}

#[test]
fn existing_legacy_database_is_copied_safely_to_tillbh_name() {
    let dir = tempdir().unwrap();
    let legacy_path = dir.path().join("amwapos.db");
    let (legacy, _) = Db::open(&legacy_path, true).unwrap();
    legacy
        .write(|tx| {
            tx.execute_batch("CREATE TABLE legacy_probe(value TEXT NOT NULL); INSERT INTO legacy_probe(value) VALUES ('preserved');")?;
            Ok(())
        })
        .unwrap();
    drop(legacy);
    std::fs::write(dir.path().join(MARKER_FILE), b"legacy initialized\n").unwrap();

    let core = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
    let probe: String = core
        .db
        .read(|c| Ok(c.query_row("SELECT value FROM legacy_probe", [], |row| row.get(0))?))
        .unwrap();

    assert_eq!(probe, "preserved");
    assert_eq!(core.db.path(), dir.path().join(DB_FILE));
    assert!(legacy_path.exists(), "the legacy database is retained as rollback evidence");
}

#[test]
fn existing_tillbh_database_always_takes_precedence_over_legacy_database() {
    let dir = tempdir().unwrap();
    let current_path = dir.path().join(DB_FILE);
    let legacy_path = dir.path().join("amwapos.db");

    let (current, _) = Db::open(&current_path, true).unwrap();
    current
        .write(|tx| {
            tx.execute_batch("CREATE TABLE precedence_probe(value TEXT NOT NULL); INSERT INTO precedence_probe(value) VALUES ('current');")?;
            Ok(())
        })
        .unwrap();
    drop(current);

    let (legacy, _) = Db::open(&legacy_path, true).unwrap();
    legacy
        .write(|tx| {
            tx.execute_batch("CREATE TABLE precedence_probe(value TEXT NOT NULL); INSERT INTO precedence_probe(value) VALUES ('legacy');")?;
            Ok(())
        })
        .unwrap();
    drop(legacy);
    std::fs::write(dir.path().join(MARKER_FILE), b"initialized\n").unwrap();

    let core = AppCore::open(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
    let probe: String = core
        .db
        .read(|c| Ok(c.query_row("SELECT value FROM precedence_probe", [], |row| row.get(0))?))
        .unwrap();

    assert_eq!(probe, "current");
    assert!(legacy_path.exists(), "legacy source must never overwrite the current TILLBH database");
}
