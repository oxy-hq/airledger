//! Local store unit tests — CRUD, tombstones, ordering, pending.

use airledger_engine::store::Store;

#[test]
fn open_creates_schema_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("airledger-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t1.db");
    let path_str = path.to_str().unwrap();
    {
        let store = Store::open(path_str).expect("first open");
        assert_eq!(
            store.meta_get("schema_version").unwrap().as_deref(),
            Some("1")
        );
    }
    // Reopen: no error, version unchanged.
    let store = Store::open(path_str).expect("reopen");
    assert_eq!(
        store.meta_get("schema_version").unwrap().as_deref(),
        Some("1")
    );
    std::fs::remove_file(&path).ok();
}
