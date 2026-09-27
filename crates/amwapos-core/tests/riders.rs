//! Rider cash custody: cash taken at the door sits with the rider (in no
//! drawer) until a cashier counts it in at a hand-over.
mod common;

use amwapos_core::auth::{ROLE_CASHIER, ROLE_DELIVERY};
use amwapos_core::pricing::TenderInput;
use amwapos_core::riders::HandoverRequest;
use amwapos_core::sales::{FinalizeRequest, Fulfilment};
use amwapos_core::tickets::RecordPayment;
use amwapos_core::ErrorCode;
use common::*;
use serde_json::json;

fn count(e: &Env, sql: &str) -> i64 {
    e.core.db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn expected_cash(e: &Env, t: &str) -> i64 {
    let id = e.core.shift_current(t).unwrap().unwrap().shift_id;
    e.core.db.read(|c| amwapos_core::shifts::shift_summary(c, &id)).unwrap().expected_cash_minor
}

/// A pay-on-delivery Send sale, dispatched with the rider. Returns (delivery_id, total).
fn pod_drop(e: &Env, ct: &str, cu: &str, rider: &str) -> (String, i64) {
    e.core.pos_scan(ct, "7001", Some(1000)).unwrap();
    let cart = e.core.pos_set_customer(ct, Some(cu.into())).unwrap();
    let total = cart.totals.total_minor;
    let s = e
        .core
        .pos_finalize(
            ct,
            FinalizeRequest {
                cart_id: cart.cart_id.unwrap(),
                operation_id: op(),
                tenders: vec![TenderInput { method: "pay_on_delivery".into(), amount_minor: total, reference: None }],
                approval_token: None,
                expected_total_minor: Some(total),
                fulfilment: Some(Fulfilment { mode: "send".into(), address: Some("Villa 7".into()), ..Default::default() }),
            },
        )
        .unwrap();
    let did = s.delivery_id.unwrap();
    e.core.delivery_update(&e.owner_token, &did, Some("dispatched".into()), Some(rider.into()), None, None).unwrap();
    (did, total)
}

fn door_cash(did: &str) -> RecordPayment {
    RecordPayment { delivery_id: did.into(), method: "cash".into(), amount_minor: None, reference: None, operation_id: op() }
}

fn handover(rider: &str, collect: Vec<String>, counted: i64, note: Option<&str>) -> HandoverRequest {
    HandoverRequest { rider_user_id: rider.into(), collect, counted_minor: counted, note: note.map(Into::into), operation_id: op() }
}

#[test]
fn door_cash_is_held_by_the_rider_until_the_hand_over_counts_it_in() {
    let e = env();
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = e
        .core
        .customer_save(&e.owner_token, None, serde_json::from_value(json!({ "name": "Ahmed", "phone": "33334444" })).unwrap())
        .unwrap()
        .customer_id;
    let (rider, rt) = e.user("Ali Rider", ROLE_DELIVERY, "1357");
    let (_c, ct) = e.user("Cashier", ROLE_CASHIER, "2468");
    e.open_shift(&ct, 10_000);
    let (d1, t1) = pod_drop(&e, &ct, &cu, &rider);
    let (d2, t2) = pod_drop(&e, &ct, &cu, &rider);
    let before = expected_cash(&e, &ct);

    // The rider records cash at the door for the first drop: paid, with the rider, in no drawer.
    let t = e.core.ticket_record_payment(&rt, door_cash(&d1)).unwrap();
    assert_eq!((t.pay_state.as_str(), t.outstanding_minor, t.cash_with.as_deref()), ("paid", 0, Some("Ali Rider")));
    assert_eq!(expected_cash(&e, &ct), before, "door cash is not in any drawer");
    assert_eq!(count(&e, "SELECT COUNT(*) FROM sale_collections WHERE held_by IS NOT NULL AND shift_id IS NULL"), 1);
    // Someone who is not the rider on the drop, without a shift, cannot take cash.
    let (_o, other) = e.user("Other Rider", ROLE_DELIVERY, "9753");
    assert_eq!(e.core.ticket_record_payment(&other, door_cash(&d2)).unwrap_err().code, ErrorCode::ShiftRequired);

    // Hand-over list: Ali holds the first drop and still owes the second.
    let list = e.core.rider_cash_list(&ct).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].held_minor, list[0].uncollected_minor), (t1, t2));
    // A rider never counts their own cash in.
    assert_eq!(e.core.rider_handover(&rt, handover(&rider, vec![], t1, None)).unwrap_err().code, ErrorCode::Forbidden);

    // Short count without a note is refused; with a note it is kept as variance.
    let err = e.core.rider_handover(&ct, handover(&rider, vec![d2.clone()], t1 + t2 - 100, None)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM rider_handovers"), 0, "nothing half-done");
    let req = handover(&rider, vec![d2.clone()], t1 + t2 - 100, Some("100 fils short"));
    let h = e.core.rider_handover(&ct, req.clone()).unwrap();
    assert_eq!(
        (h["expected_minor"].as_i64(), h["counted_minor"].as_i64(), h["variance_minor"].as_i64()),
        (Some(t1 + t2), Some(t1 + t2 - 100), Some(-100))
    );
    assert_eq!(h["drops"], 2);
    // Replay-safe.
    assert_eq!(e.core.rider_handover(&ct, req).unwrap()["handover_id"], h["handover_id"]);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM rider_handovers"), 1);
    // The drawer now expects exactly what was counted in.
    assert_eq!(expected_cash(&e, &ct), before + t1 + t2 - 100);
    // Both tickets are paid and nobody holds cash any more.
    let t = e.core.ticket_get(&ct, &d2).unwrap();
    assert_eq!((t["ticket"]["pay_state"].as_str(), t["ticket"]["cash_with"].is_null()), (Some("paid"), true));
    assert!(e.core.rider_cash_list(&ct).unwrap().is_empty());
    assert_eq!(e.core.rider_handover(&ct, handover(&rider, vec![], 0, None)).unwrap_err().code, ErrorCode::Conflict);
    assert!(count(&e, "SELECT COUNT(*) FROM audit_logs WHERE event_type='rider.handover'") == 1);
}

#[test]
fn end_of_day_lists_cash_still_with_riders() {
    let e = env();
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = e
        .core
        .customer_save(&e.owner_token, None, serde_json::from_value(json!({ "name": "Sara", "phone": "33335555" })).unwrap())
        .unwrap()
        .customer_id;
    let (rider, rt) = e.user("Faisal", ROLE_DELIVERY, "1122");
    let (_c, ct) = e.user("Cashier", ROLE_CASHIER, "3344");
    e.open_shift(&ct, 0);
    let (d1, t1) = pod_drop(&e, &ct, &cu, &rider);
    e.core.ticket_record_payment(&rt, door_cash(&d1)).unwrap();
    let pack = e.core.eod_pack(&e.owner_token, None, None).unwrap();
    assert_eq!(pack.rider_cash_held.len(), 1);
    assert_eq!((pack.rider_cash_held[0]["name"].as_str(), pack.rider_cash_held[0]["amount_minor"].as_i64()), (Some("Faisal"), Some(t1)));
    // A hand-over needs an open shift (cash goes into a drawer).
    e.core.rider_handover(&ct, handover(&rider, vec![], t1, None)).unwrap();
    assert!(e.core.eod_pack(&e.owner_token, None, None).unwrap().rider_cash_held.is_empty());
}
