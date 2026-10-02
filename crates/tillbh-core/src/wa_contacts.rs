//! Customers from the linked WhatsApp phone's address book.
//!
//! WhatsApp's contact sync gives each saved contact the name the shop typed
//! on the phone ("825 - 3325 husband") and its number. The hub stores them in
//! `wa_contacts` as they arrive; an admin previews and imports them as POS
//! customers: the saved name becomes the customer name, the number the phone
//! and WhatsApp number, and the digits with their hyphens or slashes found in
//! the name become the address ("825 - 3325"). Import matches on the phone
//! number, so running it again never creates duplicates.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::ids::new_id;
use crate::service::AppCore;
use crate::time;

/// One contact as reported by the WhatsApp client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaContact {
    /// Chat address (phone JID or LID).
    pub jid: String,
    /// Phone JID or number when known (`97333001122@s.whatsapp.net` or digits).
    pub phone: Option<String>,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ImportRequest {
    /// Only these contacts (by jid); all importable ones when absent.
    pub jids: Option<Vec<String>>,
    /// Also rename existing customers (matched by phone) to the phone's name
    /// and fill their address when it is empty.
    pub update_existing: bool,
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{0660}'..='\u{0669}').contains(&c) || ('\u{06F0}'..='\u{06F9}').contains(&c)
}

/// "+97333001122" from a phone JID, a bare number or `None`.
pub fn phone_of(v: &str) -> Option<String> {
    let user = v.split('@').next().unwrap_or("");
    let user = user.split(':').next().unwrap_or(user);
    if v.contains('@') && !(v.ends_with("@s.whatsapp.net") || v.ends_with("@c.us")) {
        return None;
    }
    let digits: String = user.chars().filter(|c| c.is_ascii_digit()).collect();
    (7..=15).contains(&digits.len()).then(|| format!("+{digits}"))
}

/// The address hidden in a saved name: every run of digits with the spaces,
/// hyphens and slashes between them, e.g. "825 - 3325 husband" → "825 - 3325",
/// "Ali 12/4 rd 55" → "12/4 55". A name that is only the phone number gives none.
pub fn address_from_name(name: &str, phone: Option<&str>) -> Option<String> {
    let seps = |c: char| c == '-' || c == '/' || c == '\\' || c == '–' || c == '—' || c == ' ';
    let mut runs: Vec<String> = vec![];
    let mut cur = String::new();
    for c in name.chars() {
        if is_digit(c) || (seps(c) && !cur.is_empty()) {
            cur.push(c);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    let parts: Vec<String> = runs
        .into_iter()
        .map(|r| r.trim_matches(|c: char| seps(c)).split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|r| r.chars().any(is_digit))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let out = parts.join(" ");
    // A contact saved under its own number is not an address.
    let digits: String = out.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Some(p) = phone {
        let pd: String = p.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 7 && pd.ends_with(&digits) {
            return None;
        }
    }
    Some(out.chars().take(300).collect())
}

fn display_name(full: &Option<String>, first: &Option<String>) -> Option<String> {
    full.as_deref()
        .or(first.as_deref())
        .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(120).collect())
}

impl AppCore {
    /// Hub: store contacts from WhatsApp's contact sync (no session: the
    /// WhatsApp client runs as the store, not as a person).
    pub fn wa_contacts_store(&self, batch: &[WaContact]) -> AppResult<usize> {
        if batch.is_empty() {
            return Ok(0);
        }
        let now = time::now_str();
        self.db.write(|tx| {
            let mut n = 0;
            for c in batch {
                if c.jid.ends_with("@g.us") || c.jid.ends_with("@broadcast") || c.jid.ends_with("@newsletter") {
                    continue;
                }
                let phone = c.phone.as_deref().and_then(phone_of).or_else(|| phone_of(&c.jid));
                n += tx.execute(
                    "INSERT INTO wa_contacts(jid, phone, full_name, first_name, updated_at) VALUES (?1,?2,?3,?4,?5)
                     ON CONFLICT(jid) DO UPDATE SET phone=COALESCE(excluded.phone, wa_contacts.phone),
                       full_name=COALESCE(excluded.full_name, wa_contacts.full_name),
                       first_name=COALESCE(excluded.first_name, wa_contacts.first_name), updated_at=excluded.updated_at",
                    params![c.jid, phone, c.full_name, c.first_name, now],
                )?;
            }
            Ok(n)
        })
    }

    /// Admin: the phone's contacts with what an import would do to each.
    pub fn wa_contacts_preview(&self, token: &str) -> AppResult<Value> {
        let s = self.session(token)?;
        s.require("customers.manage")?;
        s.require("whatsapp.manage")?;
        self.require_feature("whatsapp.enabled")?;
        self.db.read(|c| {
            let mut st = c.prepare(
                "SELECT w.jid, w.phone, w.full_name, w.first_name, w.updated_at,
                        (SELECT cu.customer_id FROM customers cu WHERE w.phone IS NOT NULL AND (cu.phone=w.phone OR cu.whatsapp=w.phone) ORDER BY cu.active DESC LIMIT 1),
                        (SELECT cu.name FROM customers cu WHERE w.phone IS NOT NULL AND (cu.phone=w.phone OR cu.whatsapp=w.phone) ORDER BY cu.active DESC LIMIT 1)
                 FROM wa_contacts w ORDER BY COALESCE(w.full_name, w.first_name, w.phone) COLLATE NOCASE LIMIT 5000",
            )?;
            let rows = st
                .query_map([], |r| {
                    let (full, first): (Option<String>, Option<String>) = (r.get(2)?, r.get(3)?);
                    let phone: Option<String> = r.get(1)?;
                    let name = display_name(&full, &first);
                    let existing: Option<String> = r.get(5)?;
                    let status = if phone.is_none() {
                        "no_phone"
                    } else if name.is_none() {
                        "no_name"
                    } else if existing.is_some() {
                        "exists"
                    } else {
                        "new"
                    };
                    Ok(json!({
                        "jid": r.get::<_, String>(0)?,
                        "phone": phone,
                        "name": name,
                        "address": name.as_deref().and_then(|n| address_from_name(n, phone.as_deref())),
                        "area": name.as_deref().and_then(crate::customers::area_from_text),
                        "status": status,
                        "customer_id": existing,
                        "customer_name": r.get::<_, Option<String>>(6)?,
                        "updated_at": r.get::<_, String>(4)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let count = |k: &str| rows.iter().filter(|x| x["status"] == k).count();
            let last: Option<String> = c.query_row("SELECT MAX(updated_at) FROM wa_contacts", [], |r| r.get(0)).optional()?.flatten();
            Ok(json!({
                "contacts": rows,
                "counts": { "new": count("new"), "exists": count("exists"), "no_phone": count("no_phone"), "no_name": count("no_name") },
                "last_sync_at": last,
            }))
        })
    }

    /// Admin: create (and optionally update) customers from the phone's contacts.
    pub fn wa_contacts_import(&self, token: &str, req: ImportRequest) -> AppResult<Value> {
        let s = self.session(token)?;
        s.require("customers.manage")?;
        s.require("whatsapp.manage")?;
        self.require_feature("whatsapp.enabled")?;
        let actor = self.actor(&s, None);
        let preview = self.wa_contacts_preview(token)?;
        if preview["contacts"].as_array().is_none_or(|a| a.is_empty()) {
            return Err(AppError::validation("No phone contacts yet. Press Refresh from phone and wait for WhatsApp to send them."));
        }
        let wanted: Option<std::collections::HashSet<String>> = req.jids.map(|v| v.into_iter().collect());
        let (mut created, mut updated, mut skipped) = (0u32, 0u32, 0u32);
        self.db.write(|tx| {
            let now = time::now_str();
            let mut seen_phones = std::collections::HashSet::new();
            for row in preview["contacts"].as_array().into_iter().flatten() {
                let jid = row["jid"].as_str().unwrap_or_default();
                if wanted.as_ref().is_some_and(|w| !w.contains(jid)) {
                    continue;
                }
                let (Some(phone), Some(name)) = (row["phone"].as_str(), row["name"].as_str()) else {
                    skipped += 1;
                    continue;
                };
                // The same number saved twice (phone JID and LID): first one wins.
                if !seen_phones.insert(phone.to_string()) {
                    skipped += 1;
                    continue;
                }
                let address = row["address"].as_str();
                let area = row["area"].as_str();
                let existing: Option<(String, Option<String>)> = tx
                    .query_row(
                        "SELECT customer_id, address FROM customers WHERE phone=?1 OR whatsapp=?1 ORDER BY active DESC LIMIT 1",
                        [phone],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let cid = match existing {
                    Some((id, addr)) => {
                        if !req.update_existing {
                            skipped += 1;
                            continue;
                        }
                        let fill = addr.as_deref().is_none_or(|a| a.trim().is_empty());
                        tx.execute(
                            "UPDATE customers SET name=?2, whatsapp=COALESCE(whatsapp, ?3), address=CASE WHEN ?4 THEN COALESCE(?5, address) ELSE address END,
                                area=COALESCE(NULLIF(TRIM(area),''), ?7), updated_at=?6 WHERE customer_id=?1",
                            params![id, name, phone, fill, address, now, area],
                        )?;
                        updated += 1;
                        id
                    }
                    None => {
                        let id = new_id();
                        tx.execute(
                            "INSERT INTO customers(customer_id, name, phone, whatsapp, address, area, active, created_at, updated_at) VALUES (?1,?2,?3,?3,?4,?6,1,?5,?5)",
                            params![id, name, phone, address, now, area],
                        )?;
                        created += 1;
                        id
                    }
                };
                tx.execute(
                    "UPDATE wa_contacts SET imported_customer_id=?2, imported_at=?3 WHERE jid=?1",
                    params![jid, cid, now],
                )?;
            }
            audit::record(
                tx,
                &actor,
                "customers.imported_whatsapp",
                "customer",
                None,
                None,
                Some(&json!({ "created": created, "updated": updated, "skipped": skipped, "update_existing": req.update_existing })),
            )?;
            Ok(())
        })?;
        Ok(json!({ "created": created, "updated": updated, "skipped": skipped }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_the_digits_hyphens_and_slashes_in_the_name() {
        assert_eq!(address_from_name("825 - 3325 husband", None).as_deref(), Some("825 - 3325"));
        assert_eq!(address_from_name("Um Ali 1203/45 Riffa", None).as_deref(), Some("1203/45"));
        assert_eq!(address_from_name("Villa 12 Road 3405 Block 934", None).as_deref(), Some("12 3405 934"));
        assert_eq!(address_from_name("Ahmed", None), None);
        assert_eq!(address_from_name("- husband -", None), None);
        assert_eq!(address_from_name("٨٢٥-٣٣٢٥ زوج", None).as_deref(), Some("٨٢٥-٣٣٢٥"));
        // Saved under its own number: not an address.
        assert_eq!(address_from_name("+973 3300 1122", Some("+97333001122")), None);
        assert_eq!(address_from_name("3300 1122", Some("+97333001122")), None);
    }

    #[test]
    fn area_lexicon_takes_the_place_and_leaves_the_house_number() {
        use crate::customers::area_from_text;
        let name = "Maryam 1203/45 Riffa";
        assert_eq!(address_from_name(name, None).as_deref(), Some("1203/45"));
        assert_eq!(area_from_text(name), Some("Riffa"));
        assert_eq!(area_from_text("Villa 5, East Riffa"), Some("Riffa East"));
        assert_eq!(area_from_text("house 12 isa town"), Some("Isa Town"));
        assert_eq!(area_from_text("منزل 12 بالمحرق"), Some("Muharraq"));
        assert_eq!(area_from_text("Aali 44"), Some("A'ali"));
        // No known place: no area (never a guess), and parts of words do not count.
        assert_eq!(area_from_text("825 - 3325 husband"), None);
        assert_eq!(area_from_text("Seefood shop"), None);
        assert_eq!(crate::customers::strip_area("Maryam 1203/45 Riffa", "Riffa"), "Maryam 1203/45");
    }

    #[test]
    fn phone_from_jids() {
        assert_eq!(phone_of("97333001122@s.whatsapp.net").as_deref(), Some("+97333001122"));
        assert_eq!(phone_of("97333001122:12@s.whatsapp.net").as_deref(), Some("+97333001122"));
        assert_eq!(phone_of("1234567890123@lid"), None);
        assert_eq!(phone_of("120363@g.us"), None);
        assert_eq!(phone_of("97333001122").as_deref(), Some("+97333001122"));
    }
}
