mod common;

use common::*;
use serde_json::json;
use tillbh_core::credit::AddressInput;

fn customer(e: &Env) -> String {
    e.core
        .customer_save(
            &e.owner_token,
            None,
            serde_json::from_value(json!({
                "name": "Fatima",
                "phone": "33330000",
                "area": "Manama",
                "address": "Old address"
            }))
            .unwrap(),
        )
        .unwrap()
        .customer_id
}

#[test]
fn default_saved_address_becomes_the_operational_customer_address() {
    let e = env();
    let cid = customer(&e);

    e.core
        .customer_address_save(
            &e.owner_token,
            AddressInput {
                address_id: None,
                customer_id: cid.clone(),
                label: "Home".into(),
                area: Some("Amwaj".into()),
                address: "Villa 10, Road 5718, Block 257".into(),
                notes: Some("Blue gate".into()),
                is_default: true,
            },
        )
        .unwrap();

    let got = e.core.customer_get(&e.owner_token, &cid).unwrap();
    assert_eq!(got["customer"]["address"], "Villa 10, Road 5718, Block 257");
    assert_eq!(got["customer"]["area"], "Amwaj");
    assert_eq!(got["customer"]["address_parts"], json!({
        "flat": null,
        "building": null,
        "road": null,
        "block": null,
        "landmark": null
    }));
}

#[test]
fn changing_the_default_changes_what_pos_prefills() {
    let e = env();
    let cid = customer(&e);

    for (label, area, address, is_default) in [
        ("Home", "Amwaj", "Home address", true),
        ("Work", "Seef", "Work address", false),
    ] {
        e.core
            .customer_address_save(
                &e.owner_token,
                AddressInput {
                    address_id: None,
                    customer_id: cid.clone(),
                    label: label.into(),
                    area: Some(area.into()),
                    address: address.into(),
                    notes: None,
                    is_default,
                },
            )
            .unwrap();
    }

    let account = e.core.customer_account(&e.owner_token, &cid).unwrap();
    let work_id = account["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["label"] == "Work")
        .unwrap()["address_id"]
        .as_str()
        .unwrap()
        .to_string();

    e.core
        .customer_address_save(
            &e.owner_token,
            AddressInput {
                address_id: Some(work_id),
                customer_id: cid.clone(),
                label: "Work".into(),
                area: Some("Seef".into()),
                address: "Work address".into(),
                notes: None,
                is_default: true,
            },
        )
        .unwrap();

    let got = e.core.customer_get(&e.owner_token, &cid).unwrap();
    assert_eq!(got["customer"]["address"], "Work address");
    assert_eq!(got["customer"]["area"], "Seef");

    let account = e.core.customer_account(&e.owner_token, &cid).unwrap();
    let defaults = account["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["is_default"] == true)
        .count();
    assert_eq!(defaults, 1);
}
