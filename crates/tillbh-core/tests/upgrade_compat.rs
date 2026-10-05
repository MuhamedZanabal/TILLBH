use std::sync::Arc;

use tillbh_core::service::{AppCore, MemorySecretStore, DB_FILE};

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
