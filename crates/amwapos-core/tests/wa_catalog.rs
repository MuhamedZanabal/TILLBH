//! WhatsApp catalogue publishing state in the core: nothing before an
//! administrator starts it, account-scoped mappings, fingerprints (no
//! rewrites when unchanged), the claim / outcome state machine with bounded
//! retries, deletion and remote-missing semantics, and permissions.
mod common;

use amwapos_core::auth::ROLE_CASHIER;
use amwapos_core::wa_catalog::{CatalogAction, CatalogOutcome, MAX_ATTEMPTS};
use amwapos_core::ErrorCode;
use common::*;
use serde_json::json;

const ACC: &str = "97330000000@s.whatsapp.net";
const ACC2: &str = "97339999999@s.whatsapp.net";

fn count(e: &Env, sql: &str) -> i64 {
    e.core.db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn published(e: &Env, pid: &str, remote: &str) {
    let job = e.core.wa_catalog_claim(ACC, 10).unwrap().into_iter().find(|j| j.product_id == pid).unwrap();
    let item = job.item.clone().unwrap();
    let st = e
        .core
        .wa_catalog_complete(ACC, pid, CatalogOutcome::Published { remote_id: remote.into(), item, image_url: None, adopted: false })
        .unwrap();
    assert_eq!(st, "synced");
}

#[test]
fn nothing_is_published_until_an_administrator_starts_it() {
    let e = env();
    let pid = e.product("Laban", "7001", 450, 300, 0);
    assert!(e.core.wa_catalog_claim(ACC, 10).unwrap().is_empty());
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 0, "the automatic scan needs the first sync");
    assert!(!e.core.wa_catalog_auto(ACC).unwrap());
    // A cashier can neither start nor see it.
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    assert_eq!(e.core.wa_catalog_start(&ct, ACC).unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(e.core.wa_catalog_overview(&ct, Some(ACC)).unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(e.core.wa_catalog_configure(&ct, false).unwrap_err().code, ErrorCode::Forbidden);
    // Start: queued, the job carries the exact price in thousandths.
    let o = e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    assert_eq!((o["published"].as_bool(), o["counts"]["queued"].as_i64()), (Some(true), Some(1)));
    let jobs = e.core.wa_catalog_claim(ACC, 10).unwrap();
    assert_eq!(jobs.len(), 1);
    let j = &jobs[0];
    assert_eq!((j.product_id.as_str(), &j.action), (pid.as_str(), &CatalogAction::Upsert));
    assert_eq!(j.item.as_ref().unwrap().price_1000, Some(450));
    assert_eq!(j.item.as_ref().unwrap().currency, "BHD");
    assert!(e.core.wa_catalog_claim(ACC, 10).unwrap().is_empty(), "claimed once");
    assert!(count(&e, "SELECT COUNT(*) FROM audit_logs WHERE event_type='whatsapp.catalog_sync'") == 1);
}

#[test]
fn unchanged_products_are_not_rewritten_and_changes_are_detected() {
    let e = env();
    let pid = e.product("Laban", "7001", 450, 300, 0);
    e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    published(&e, &pid, "5001");
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 0, "unchanged: nothing queued");
    e.core.product_price_update(&e.owner_token, &pid, 500, None, None).unwrap();
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 1);
    let j = e.core.wa_catalog_claim(ACC, 10).unwrap().remove(0);
    assert_eq!((j.remote_id.as_deref(), j.item.as_ref().unwrap().price_1000), (Some("5001"), Some(500)), "the same remote product");
    // Archive → hide; a hidden product is not re-written on later edits.
    let item = j.item.clone().unwrap();
    e.core
        .wa_catalog_complete(ACC, &pid, CatalogOutcome::Published { remote_id: "5001".into(), item, image_url: None, adopted: false })
        .unwrap();
    e.core.product_set_active(&e.owner_token, &pid, false).unwrap();
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 1);
    let j = e.core.wa_catalog_claim(ACC, 10).unwrap().remove(0);
    assert_eq!(j.action, CatalogAction::Hide);
    assert!(j.item.as_ref().unwrap().hidden);
    let item = j.item.clone().unwrap();
    assert_eq!(
        e.core
            .wa_catalog_complete(ACC, &pid, CatalogOutcome::Published { remote_id: "5001".into(), item, image_url: None, adopted: false })
            .unwrap(),
        "hidden"
    );
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 0);
}

#[test]
fn transient_errors_are_bounded_and_permanent_ones_wait_for_a_change_or_retry() {
    let e = env();
    let pid = e.product("Flaky", "7002", 300, 100, 0);
    e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    let due = || e.core.db.write(|c| Ok(c.execute("UPDATE wa_catalog_products SET next_at=NULL", [])?)).unwrap();
    for i in 1..=MAX_ATTEMPTS {
        let j = e.core.wa_catalog_claim(ACC, 10).unwrap();
        assert_eq!(j.len(), 1, "attempt {i}");
        let st =
            e.core.wa_catalog_complete(ACC, &pid, CatalogOutcome::Transient { error: "timeout".into(), retry_after_s: Some(120) }).unwrap();
        assert_eq!(st, if i < MAX_ATTEMPTS { "retry" } else { "failed" });
        if i == 1 {
            assert!(e.core.wa_catalog_claim(ACC, 10).unwrap().is_empty(), "waits for its delay");
        }
        due();
    }
    assert!(e.core.wa_catalog_claim(ACC, 10).unwrap().is_empty(), "failed is terminal");
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 0, "no loop while nothing changed");
    // A change re-queues it; so does an explicit retry.
    e.core.product_price_update(&e.owner_token, &pid, 350, None, None).unwrap();
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 1);
    let _ = e.core.wa_catalog_claim(ACC, 10).unwrap();
    e.core.wa_catalog_complete(ACC, &pid, CatalogOutcome::Failed { error: "400".into() }).unwrap();
    assert_eq!(e.core.wa_catalog_retry(&e.owner_token, ACC, None).unwrap()["queued"], 1);
    assert_eq!(e.core.wa_catalog_claim(ACC, 10).unwrap().len(), 1);
}

#[test]
fn a_stale_claim_is_recovered_and_a_superseded_outcome_is_dropped() {
    let e = env();
    let pid = e.product("Stale", "7003", 300, 100, 0);
    e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    let j = e.core.wa_catalog_claim(ACC, 10).unwrap().remove(0);
    e.core.db.write(|c| Ok(c.execute("UPDATE wa_catalog_products SET claimed_at='2000-01-01T00:00:00.000Z'", [])?)).unwrap();
    assert_eq!(e.core.wa_catalog_claim(ACC, 10).unwrap().len(), 1, "taken back after the claim timeout");
    let _ = j;
    // An outcome for a row that is not claimed any more is ignored.
    e.core.wa_catalog_complete(ACC, &pid, CatalogOutcome::Deleted).unwrap();
    assert_eq!(e.core.wa_catalog_complete(ACC, &pid, CatalogOutcome::Deleted).unwrap(), "superseded");
}

#[test]
fn remote_missing_and_pos_deletion_semantics() {
    let e = env();
    let pid = e.product("Gone there", "7004", 300, 100, 0);
    e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    published(&e, &pid, "5004");
    // Deleted on WhatsApp: never silently re-created.
    e.core.product_price_update(&e.owner_token, &pid, 320, None, None).unwrap();
    e.core.wa_catalog_scan(ACC).unwrap();
    let _ = e.core.wa_catalog_claim(ACC, 10).unwrap();
    e.core.wa_catalog_complete(ACC, &pid, CatalogOutcome::RemoteMissing { error: "deleted on WhatsApp".into() }).unwrap();
    e.core.product_price_update(&e.owner_token, &pid, 330, None, None).unwrap();
    assert_eq!(e.core.wa_catalog_scan(ACC).unwrap(), 0, "remote_missing waits for Retry");
    // Retry creates it again (the stale remote id is dropped).
    e.core.wa_catalog_retry(&e.owner_token, ACC, Some(&pid)).unwrap();
    let j = e.core.wa_catalog_claim(ACC, 10).unwrap().remove(0);
    assert_eq!((j.action, j.remote_id), (CatalogAction::Upsert, None));
    // A mapping whose POS product no longer exists deletes only its own remote product.
    e.core
        .db
        .write(|c| {
            Ok(c.execute(
                "INSERT INTO wa_catalog_products(account, product_id, remote_id, status, created_at, updated_at)
                 VALUES ('97330000000','01ARZ3NDEKTSV4RRFFQ69G5FAV','5099','synced','x','x')",
                [],
            )?)
        })
        .unwrap();
    e.core.wa_catalog_scan(ACC).unwrap();
    let j = e.core.wa_catalog_claim(ACC, 10).unwrap().into_iter().find(|j| j.remote_id.as_deref() == Some("5099")).unwrap();
    assert_eq!(j.action, CatalogAction::Delete);
    assert_eq!(e.core.wa_catalog_complete(ACC, &j.product_id, CatalogOutcome::Deleted).unwrap(), "removed");
}

#[test]
fn mappings_are_scoped_to_the_linked_account() {
    let e = env();
    let pid = e.product("Scoped", "7005", 300, 100, 0);
    e.core.wa_catalog_start(&e.owner_token, ACC).unwrap();
    published(&e, &pid, "5005");
    // Another linked number: not published, no jobs, no reuse of 5005.
    assert!(!e.core.wa_catalog_published(ACC2).unwrap());
    assert!(e.core.wa_catalog_claim(ACC2, 10).unwrap().is_empty());
    e.core.wa_catalog_start(&e.owner_token, ACC2).unwrap();
    let j = e.core.wa_catalog_claim(ACC2, 10).unwrap().remove(0);
    assert_eq!(j.remote_id, None, "a new account starts without remote ids");
    // A remote id already linked to another product is never accepted twice.
    let other = e.product("Other", "7006", 300, 100, 0);
    e.core.wa_catalog_scan(ACC).unwrap();
    let k = e.core.wa_catalog_claim(ACC, 10).unwrap().into_iter().find(|j| j.product_id == other).unwrap();
    let item = k.item.clone().unwrap();
    let st = e
        .core
        .wa_catalog_complete(ACC, &other, CatalogOutcome::Published { remote_id: "5005".into(), item, image_url: None, adopted: true })
        .unwrap();
    assert_eq!(st, "failed");
    let ov = e.core.wa_catalog_overview(&e.owner_token, Some(ACC)).unwrap();
    assert_eq!(ov["counts"]["synced"], 1);
    assert_eq!(ov["failures"][0]["product_id"], json!(other));
}
