//! The Send loop: PAY Here / Send, pay on delivery, the ticket ↔ drop link,
//! forward-only status, delivery notices, area lexicon and chat linking.
mod common;

use tillbh_core::auth::ROLE_CASHIER;
use tillbh_core::messaging::{Inbound, QueueRequest};
use tillbh_core::pricing::TenderInput;
use tillbh_core::sales::{FinalizeRequest, Fulfilment, SaleResult};
use tillbh_core::tickets::{RecordPayment, TicketFilter};
use tillbh_core::ErrorCode;
use common::*;
use serde_json::json;

fn count(e: &Env, sql: &str) -> i64 {
    e.core.db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn features(e: &Env, v: serde_json::Value) {
    e.core.settings_save(&e.owner_token, "features", v).unwrap();
}

/// A Send to building 12, block 905, with `landmark` as the rest of the address.
fn send(landmark: &str, area: Option<&str>) -> Option<Fulfilment> {
    Some(Fulfilment {
        mode: "send".into(),
        area: area.map(Into::into),
        address_parts: tillbh_core::address::AddressParts {
            building: Some("12".into()),
            block: Some("905".into()),
            landmark: Some(landmark.into()),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn cash(total: i64) -> Vec<TenderInput> {
    vec![TenderInput { method: "cash".into(), amount_minor: total, reference: None }]
}

fn pod(total: i64) -> Vec<TenderInput> {
    vec![TenderInput { method: "pay_on_delivery".into(), amount_minor: total, reference: None }]
}

/// Ring up one item for a customer and pay with the given tenders.
fn sell(
    e: &Env,
    t: &str,
    customer: Option<&str>,
    tenders: fn(i64) -> Vec<TenderInput>,
    f: Option<Fulfilment>,
) -> Result<SaleResult, tillbh_core::AppError> {
    e.core.pos_scan(t, "7001", Some(1000)).unwrap();
    let cart = e.core.pos_set_customer(t, customer.map(Into::into)).unwrap();
    let total = cart.totals.total_minor;
    e.core.pos_finalize(
        t,
        FinalizeRequest {
            cart_id: cart.cart_id.unwrap(),
            operation_id: op(),
            tenders: tenders(total),
            approval_token: None,
            expected_total_minor: Some(total),
            fulfilment: f,
        },
    )
}

fn customer(e: &Env, name: &str, phone: &str, address: Option<&str>) -> String {
    e.core
        .customer_save(
            &e.owner_token,
            None,
            serde_json::from_value(json!({ "name": name, "phone": phone, "whatsapp": phone, "address": address })).unwrap(),
        )
        .unwrap()
        .customer_id
}

/// The drawer's expected cash (read directly: a cashier may count blind).
fn expected_cash(e: &Env, t: &str) -> i64 {
    let id = e.core.shift_current(t).unwrap().unwrap().shift_id;
    e.core.db.read(|c| tillbh_core::shifts::shift_summary(c, &id)).unwrap().expected_cash_minor
}

#[test]
fn pay_here_makes_no_drop_and_send_makes_one_on_the_rail() {
    let e = env();
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = customer(&e, "Maryam", "33331111", Some("House 1203, Road 45"));
    let (_u, ct) = e.user("Cashier One", ROLE_CASHIER, "2580");
    e.open_shift(&ct, 0);
    // Here (default and explicit): sale and receipt, no drop.
    sell(&e, &ct, Some(&cu), cash, None).unwrap();
    let here = sell(&e, &ct, Some(&cu), cash, Some(Fulfilment { mode: "here".into(), ..Default::default() })).unwrap();
    assert!(here.delivery_id.is_none());
    assert_eq!(count(&e, "SELECT COUNT(*) FROM delivery_orders"), 0);
    // Send: sale + one drop, pending, paid at the till, on this till's rail.
    let s = sell(&e, &ct, Some(&cu), cash, send("1203/45", Some("Riffa"))).unwrap();
    let did = s.delivery_id.clone().expect("a drop");
    let rail = e.core.tickets_list(&ct, TicketFilter::default()).unwrap();
    assert_eq!(rail.len(), 1);
    let t0 = &rail[0];
    assert_eq!((t0.delivery_id.as_deref(), t0.sale_id.as_deref()), (Some(did.as_str()), Some(s.sale_id.as_str())));
    assert_eq!((t0.status.as_str(), t0.pay_state.as_str(), t0.area.as_deref()), ("pending", "paid", Some("Riffa")));
    assert_eq!(t0.number, s.receipt_number, "the ticket is the sale");
    assert_eq!(e.core.tickets_counts(&ct).unwrap()["badge"], 1);
    // One active drop per ticket.
    let again = e.core.delivery_create(&e.owner_token, serde_json::from_value(json!({ "sale_id": s.sale_id, "address": "x" })).unwrap());
    assert_eq!(again.unwrap_err().code, ErrorCode::Conflict);
    // "Save on customer" off: the customer's saved address is unchanged.
    let c = e.core.customer_get(&e.owner_token, &cu).unwrap();
    assert_eq!(c["customer"]["address"], "House 1203, Road 45");
    // Ticket sheet: lines read-only from the sale, legal next steps only.
    let sheet = e.core.ticket_get(&ct, &did).unwrap();
    assert_eq!(sheet["lines"].as_array().unwrap().len(), 1);
    assert_eq!(sheet["next"], json!(["preparing", "dispatched"]), "a cashier never cancels");
    // Send needs a customer, and a building and block (a free-text line is not enough).
    let err = sell(&e, &ct, None, cash, send("Road 1", None)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let line_only = Some(Fulfilment { mode: "send".into(), address: Some("Villa 7, Riffa".into()), ..Default::default() });
    let before = count(&e, "SELECT COUNT(*) FROM sales");
    let err = sell(&e, &ct, Some(&cu), cash, line_only).unwrap_err();
    assert_eq!((err.code, err.message.as_str()), (ErrorCode::Validation, "Enter the building and block to send to."));
    assert_eq!(count(&e, "SELECT COUNT(*) FROM sales"), before, "nothing committed");
}

#[test]
fn pay_on_delivery_leaves_the_ticket_unpaid_and_the_drawer_unchanged() {
    let e = env();
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = customer(&e, "Ali", "33332222", Some("Villa 7"));
    let (_u, ct) = e.user("Cashier Two", ROLE_CASHIER, "3690");
    e.open_shift(&ct, 5_000);
    let before = expected_cash(&e, &ct);
    // Pay on delivery is only for a sale that is sent; zero tender without it is refused.
    assert_eq!(sell(&e, &ct, Some(&cu), pod, None).unwrap_err().code, ErrorCode::Validation);
    assert_eq!(sell(&e, &ct, Some(&cu), |_| vec![], send("Villa 7", None)).unwrap_err().code, ErrorCode::Validation);
    let s = sell(&e, &ct, Some(&cu), pod, send("Villa 7", None)).unwrap();
    let did = s.delivery_id.unwrap();
    // A real sale (not a ghost cash sale): committed with the pay-on-delivery tender, drawer unchanged.
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM payments WHERE sale_id='{}' AND method='pay_on_delivery'", s.sale_id)), 1);
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM payments WHERE sale_id='{}' AND method='cash'", s.sale_id)), 0);
    assert_eq!(expected_cash(&e, &ct), before);
    let t = e.core.tickets_list(&ct, TicketFilter::default()).unwrap().remove(0);
    assert_eq!((t.pay_state.as_str(), t.outstanding_minor), ("unpaid", s.total_minor));
    assert!(count(&e, "SELECT COUNT(*) FROM audit_logs WHERE event_type='sale.completed'") >= 1);
    // Out while unpaid is a problem on the board.
    e.core.delivery_update(&ct, &did, Some("dispatched".into()), None, None, None).unwrap();
    let board = e.core.tickets_list(&e.owner_token, TicketFilter { tab: Some("board".into()), ..Default::default() }).unwrap();
    assert_eq!(board[0].problem.as_deref(), Some("unpaid_out"));
    // Record the cash at the door: once (replay-safe), then the drawer expects it.
    let op_id = op();
    let rec = |id: &str| RecordPayment {
        delivery_id: did.clone(),
        method: "cash".into(),
        amount_minor: None,
        reference: None,
        operation_id: id.into(),
    };
    let paid = e.core.ticket_record_payment(&ct, rec(&op_id)).unwrap();
    assert_eq!((paid.pay_state.as_str(), paid.outstanding_minor, paid.problem.as_deref()), ("paid", 0, None));
    assert_eq!(e.core.ticket_record_payment(&ct, rec(&op_id)).unwrap().pay_state, "paid");
    assert_eq!(count(&e, "SELECT COUNT(*) FROM sale_collections"), 1);
    assert_eq!(expected_cash(&e, &ct), before + s.total_minor);
    assert_eq!(e.core.ticket_record_payment(&ct, rec(&op())).unwrap_err().code, ErrorCode::Conflict);
    // The sale itself never changed.
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM payments WHERE sale_id='{}'", s.sale_id)), 1);
}

#[test]
fn drop_status_moves_forward_only_and_undo_is_the_one_way_back() {
    let e = env();
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = customer(&e, "Huda", "33333333", Some("Flat 2"));
    let (_u, ct) = e.user("Cashier Three", ROLE_CASHIER, "7410");
    e.open_shift(&ct, 0);
    let did = sell(&e, &ct, Some(&cu), cash, send("Flat 2", Some("Seef"))).unwrap().delivery_id.unwrap();
    let step = |to: &str| e.core.delivery_update(&ct, &did, Some(to.into()), None, None, None);
    step("preparing").unwrap();
    assert_eq!(step("pending").unwrap_err().code, ErrorCode::Conflict);
    step("dispatched").unwrap();
    assert_eq!(step("preparing").unwrap_err().code, ErrorCode::Conflict);
    // A cashier cannot cancel or assign a rider.
    assert_eq!(step("cancelled").unwrap_err().code, ErrorCode::Forbidden);
    step("delivered").unwrap();
    assert_eq!(step("dispatched").unwrap_err().code, ErrorCode::Conflict);
    assert!(e.core.tickets_list(&ct, TicketFilter { tab: Some("done".into()), ..Default::default() }).unwrap().len() == 1);
    // Undo (deliveries.manage) steps back exactly one move.
    e.core.delivery_revert(&e.owner_token, &did, "dispatched", None).unwrap();
    assert_eq!(e.core.tickets_list(&ct, TicketFilter { tab: Some("out".into()), ..Default::default() }).unwrap().len(), 1);
}

#[test]
fn ringing_up_a_digital_order_is_idempotent_and_sends_one_drop() {
    let e = env();
    let t = &e.owner_token;
    let pid = e.product("Laban 1L", "7001", 450, 300, 10_000);
    let cu = customer(&e, "Sara", "33334444", Some("Road 12, Manama"));
    e.open_shift(t, 0);
    features(&e, json!({ "orders.digital": true }));
    let o = e
        .core
        .order_save(
            t,
            None,
            serde_json::from_value(json!({ "channel": "phone", "customer_id": cu, "delivery_wanted": true, "address": "Road 12, Manama",
                "lines": [{ "product_id": pid, "qty_milli": 2000 }] }))
            .unwrap(),
        )
        .unwrap();
    // A draft waits on the rail as a ticket.
    let now = e.core.tickets_list(t, TicketFilter::default()).unwrap();
    assert_eq!((now[0].kind.as_str(), now[0].status.as_str()), ("order", "draft"));
    e.core.order_confirm(t, &o.order_id).unwrap();
    let op1 = op();
    let cart = e.core.order_convert(t, &o.order_id, &op1).unwrap();
    assert_eq!(e.core.order_convert(t, &o.order_id, &op1).unwrap().cart_id, cart.cart_id);
    // PAY sheet prefill: the order wants delivery to its address.
    assert_eq!(cart.order.as_ref().unwrap()["delivery_wanted"], true);
    let total = cart.totals.total_minor;
    let s = e
        .core
        .pos_finalize(
            t,
            FinalizeRequest {
                cart_id: cart.cart_id.unwrap(),
                operation_id: op(),
                tenders: cash(total),
                approval_token: None,
                expected_total_minor: Some(total),
                fulfilment: send("Road 12, Manama", None),
            },
        )
        .unwrap();
    let did = s.delivery_id.unwrap();
    let order = e.core.order_get(t, &o.order_id).unwrap();
    assert_eq!((order.status.as_str(), order.delivery_id.as_deref()), ("converted", Some(did.as_str())));
    assert_eq!(count(&e, "SELECT COUNT(*) FROM delivery_orders"), 1, "one drop per ticket");
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM delivery_orders WHERE order_id='{}' AND channel='phone'", o.order_id)), 1);
    // Area found in the address when none was typed.
    assert_eq!(e.core.ticket_get(t, &o.order_id).unwrap()["ticket"]["area"], "Manama");
    // Stock moved once.
    let qty = e.core.db.read(|c| tillbh_core::inventory::current_qty(c, &pid, &e.core.require_device().unwrap().branch_id)).unwrap();
    assert_eq!(qty, 10_000 - 2000);
    assert_eq!(e.core.order_convert(t, &o.order_id, &op1).unwrap_err().code, ErrorCode::Conflict);
}

#[test]
fn delivery_notice_carries_the_address_and_is_never_automatic_by_default() {
    let e = env();
    let t = &e.owner_token;
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = customer(&e, "Noor", "33335555", None);
    e.open_shift(t, 0);
    let did = sell(&e, t, Some(&cu), cash, send("1203/45", Some("Riffa"))).unwrap().delivery_id.unwrap();
    let notice = |kind: &str| -> QueueRequest {
        serde_json::from_value(json!({ "operation_id": op(), "kind": kind, "delivery_id": did, "lang": "en" })).unwrap()
    };
    // Flag off: no notice at all.
    assert_eq!(e.core.wa_queue(t, notice("dispatch")).unwrap_err().details.unwrap()["kind"], "feature_disabled");
    e.core.delivery_update(t, &did, Some("dispatched".into()), None, None, None).unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM wa_outbox"), 0);
    // Flag on: a person sends it (with the address); moving the drop sends nothing by itself.
    features(&e, json!({ "whatsapp.enabled": true, "whatsapp.delivery_notices": true }));
    let m = e.core.wa_queue(t, notice("dispatch")).unwrap();
    assert!(m.body.contains("1203/45") && m.body.contains("Riffa"), "{}", m.body);
    e.core.delivery_update(t, &did, Some("delivered".into()), None, None, None).unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM wa_outbox WHERE kind='delivered'"), 0);
    // The automatic notice is a setting, off by default.
    let wa = e.core.settings_get(t, "whatsapp").unwrap();
    assert_eq!(wa["auto_delivery_notice"], false);
}

#[test]
fn chat_links_to_a_customer_by_number_and_unmatched_stays_unmatched() {
    let e = env();
    let t = &e.owner_token;
    let cu = customer(&e, "Maryam 1203/45 Riffa", "33336666", None);
    // Manual save with the place in the address fills the area.
    let c = e.core.customer_get(t, &cu).unwrap();
    assert_eq!(c["customer"]["area"], serde_json::Value::Null, "no address, no area");
    let c2 = e
        .core
        .customer_save(
            t,
            None,
            serde_json::from_value(json!({ "name": "Fatima", "phone": "33337777", "address": "Villa 3 Isa Town" })).unwrap(),
        )
        .unwrap();
    assert_eq!(c2.info.area.as_deref(), Some("Isa Town"));
    let msg = |id: &str, chat: &str| Inbound {
        wa_id: id.into(),
        chat: chat.into(),
        ts: 1_790_000_000,
        kind: "text".into(),
        text: Some("hello".into()),
        push_name: Some("M".into()),
        ..Default::default()
    };
    e.core.wa_ingest(&[msg("A1", "97333336666@s.whatsapp.net"), msg("B1", "97339990000@s.whatsapp.net")]).unwrap();
    let known = e.core.wa_thread_context(t, "97333336666@s.whatsapp.net").unwrap();
    assert_eq!((known["match"].as_str(), known["customer"]["customer_id"].as_str()), (Some("number"), Some(cu.as_str())));

    // Simulate legacy data from before cross-field uniqueness was enforced.
    // Once two customers own the same number, neither a new inbound message,
    // the conversation list nor a WhatsApp-derived order may guess a customer.
    let legacy = op();
    e.core
        .db
        .write(|tx| {
            tx.execute(
                "INSERT INTO customers(customer_id, name, phone, whatsapp, active, created_at, updated_at)
                 VALUES (?1,'Legacy duplicate','+97331112222','+97333336666',1,'2026-10-01','2026-10-01')",
                rusqlite::params![legacy],
            )?;
            Ok(())
        })
        .unwrap();
    e.core.wa_ingest(&[msg("A2", "97333336666@s.whatsapp.net")]).unwrap();
    let ambiguous = e.core.wa_thread_context(t, "97333336666@s.whatsapp.net").unwrap();
    assert!(ambiguous["customer"].is_null());
    assert_eq!(ambiguous["match"], "ambiguous");
    let conversations = e.core.wa_conversations(t).unwrap();
    let conversation = conversations.iter().find(|x| x.chat == "97333336666@s.whatsapp.net").unwrap();
    assert!(conversation.customer_id.is_none(), "the list must not keep a stale historical auto-match");
    assert_eq!(
        count(&e, "SELECT COUNT(*) FROM wa_inbox WHERE wa_id='A2' AND customer_id IS NULL"),
        1,
        "new ambiguous messages stay unlinked"
    );
    features(&e, json!({ "whatsapp.enabled": true, "orders.digital": true }));
    let seq: i64 = e
        .core
        .db
        .read(|c| Ok(c.query_row("SELECT seq FROM wa_inbox WHERE wa_id='A2'", [], |r| r.get(0))?))
        .unwrap();
    let draft = e.core.order_from_inbox(t, seq).unwrap();
    assert!(draft.customer_id.is_none(), "an ambiguous WhatsApp message must create an unlinked draft");

    // A manual link is authoritative and resolves the ambiguity for the chat.
    let relinked = e.core.wa_link_customer(t, "97333336666@s.whatsapp.net", Some(cu.clone())).unwrap();
    assert_eq!((relinked["match"].as_str(), relinked["customer"]["customer_id"].as_str()), (Some("linked"), Some(cu.as_str())));

    let unknown = e.core.wa_thread_context(t, "97339990000@s.whatsapp.net").unwrap();
    assert!(unknown["customer"].is_null() && unknown["match"].is_null());
    // A person links it by hand; later messages in that chat follow the link.
    let linked = e.core.wa_link_customer(t, "97339990000@s.whatsapp.net", Some(c2.customer_id.clone())).unwrap();
    assert_eq!(linked["match"], "linked");
    e.core.wa_ingest(&[msg("B2", "97339990000@s.whatsapp.net")]).unwrap();
    assert_eq!(
        count(&e, &format!("SELECT COUNT(*) FROM wa_inbox WHERE chat='97339990000@s.whatsapp.net' AND customer_id='{}'", c2.customer_id)),
        2
    );
    // A cashier (no whatsapp.manage) cannot link chats or preview the phone's address book.
    let (_u, ct) = e.user("Cashier Four", ROLE_CASHIER, "8520");
    assert_eq!(e.core.wa_link_customer(&ct, "97339990000@s.whatsapp.net", None).unwrap_err().code, ErrorCode::Forbidden);
    features(&e, json!({ "whatsapp.enabled": true }));
    assert_eq!(e.core.wa_contacts_preview(&ct).unwrap_err().code, ErrorCode::Forbidden);
}

#[test]
fn whatsapp_order_parser_preserves_numbered_product_names_and_arabic_quantities() {
    let e = env();
    let coke = e.product("Coke 330", "8801", 300, 180, 50_000);
    let seven = e.product("7 Up", "8802", 350, 200, 50_000);
    let milk = e.product("Milk", "8803", 500, 300, 50_000);
    let bread = e.product("Bread", "8804", 250, 120, 50_000);

    let parsed = e
        .core
        .db
        .read(|c| tillbh_core::orders::suggest_lines(c, "Coke 330, 7 Up"))
        .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!((parsed[0].product_id.as_deref(), parsed[0].qty_milli), (Some(coke.as_str()), 1000));
    assert_eq!((parsed[1].product_id.as_deref(), parsed[1].qty_milli), (Some(seven.as_str()), 1000));

    let parsed = e
        .core
        .db
        .read(|c| tillbh_core::orders::suggest_lines(c, "أبغى ٢ Milk، Bread 3"))
        .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!((parsed[0].product_id.as_deref(), parsed[0].qty_milli), (Some(milk.as_str()), 2000));
    assert_eq!((parsed[1].product_id.as_deref(), parsed[1].qty_milli), (Some(bread.as_str()), 3000));

    let parsed = e
        .core
        .db
        .read(|c| tillbh_core::orders::suggest_lines(c, "please bring 2 x Milk; Bread لو سمحت"))
        .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!((parsed[0].product_id.as_deref(), parsed[0].qty_milli), (Some(milk.as_str()), 2000));
    assert_eq!((parsed[1].product_id.as_deref(), parsed[1].qty_milli), (Some(bread.as_str()), 1000));
}

#[test]
fn order_received_notice_is_manual_by_default_and_automatic_when_the_setting_is_on() {
    let e = env();
    let t = &e.owner_token;
    e.product("Laban 1L", "7001", 450, 300, 100_000);
    let cu = customer(&e, "Layla", "33338181", None);
    e.open_shift(t, 0);
    features(&e, json!({ "whatsapp.enabled": true, "whatsapp.delivery_notices": true }));
    // Off by default: taking a Send order queues nothing by itself.
    let did = sell(&e, t, Some(&cu), cash, send("Bldg 7, Road 12", Some("Saar"))).unwrap().delivery_id.unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM wa_outbox"), 0);
    // A person can send it: ticket, total and the address are in it.
    let m = e
        .core
        .wa_queue(t, serde_json::from_value(json!({ "operation_id": op(), "kind": "received", "delivery_id": did, "lang": "en" })).unwrap())
        .unwrap();
    assert!(m.body.contains("has your order") && m.body.contains("Bldg 7, Road 12"), "{}", m.body);
    // With the automatic notice on, the next Send order queues it once.
    let mut wa = e.core.settings_get(t, "whatsapp").unwrap();
    wa["auto_delivery_notice"] = json!(true);
    e.core.settings_save(t, "whatsapp", wa).unwrap();
    let d2 = sell(&e, t, Some(&cu), cash, send("Villa 2", Some("Saar"))).unwrap().delivery_id.unwrap();
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM wa_outbox WHERE kind='received' AND delivery_id='{d2}'")), 1);
}
