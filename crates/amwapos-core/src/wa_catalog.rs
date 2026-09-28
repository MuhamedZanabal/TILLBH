//! WhatsApp Business catalogue publishing: the POS catalogue is the source of
//! truth and WhatsApp receives a copy (one direction, POS → WhatsApp).
//!
//! This module owns the durable state; the network part is the hub's
//! WhatsApp service (the same linked session that sends receipts), which
//! claims work here and reports outcomes back. Nothing here talks to
//! WhatsApp, and no product, price or checkout write ever waits for it.
//!
//! * Mappings (`wa_catalog_products`) are keyed by (account, product_id):
//!   the linked number, then the POS product. Product names are never
//!   identifiers. A different linked account starts with no mappings, so a
//!   remote id from account A is never used with account B.
//! * What WhatsApp should show for a product is a deterministic
//!   representation (`CatalogItem`); its SHA-256 `fingerprint` is stored when
//!   a write succeeds, so an unchanged product causes no remote write.
//! * Publishing starts only when an administrator runs the first sync for the
//!   linked account ("published"); after that, with "keep synchronised" on,
//!   the worker's scan queues whatever changed.
//! * Lifecycle per product: `queued → syncing → synced | hidden | removed |
//!   failed | remote_missing`; transient errors go back to `queued` with a
//!   delay, at most `MAX_ATTEMPTS` times.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::audit;
use crate::catalog::PRICE_SQL;
use crate::error::{AppError, AppResult};
use crate::service::AppCore;
use crate::settings;
use crate::time;
use crate::validate;

pub const KEY_WA_CATALOG: &str = "whatsapp.catalog";
/// Conservative AMWAPOS limits (not published by WhatsApp): longer text is
/// cut deterministically at a character boundary with "…".
pub const NAME_MAX_CHARS: usize = 150;
pub const DESCRIPTION_MAX_CHARS: usize = 1000;
pub const RETAILER_ID_MAX_CHARS: usize = 100;
/// Transient failures are retried at most this many times, then `failed`.
pub const MAX_ATTEMPTS: i64 = 5;
/// A claim older than this is treated as abandoned (the worker stopped).
pub const STALE_CLAIM_MINUTES: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WaCatalogSettings {
    /// Keep the WhatsApp catalogue synchronised after the first sync.
    pub auto_sync: bool,
    /// Linked accounts (digits) an administrator started publishing to.
    pub published_accounts: Vec<String>,
}

impl Default for WaCatalogSettings {
    fn default() -> Self {
        Self { auto_sync: true, published_accounts: vec![] }
    }
}

/// The linked account's key: the phone number digits of its JID
/// (`97330000000:12@s.whatsapp.net` → `97330000000`).
pub fn account_key(account: &str) -> Option<String> {
    let user = account.split('@').next()?.split(':').next()?;
    let d: String = user.chars().filter(|c| c.is_ascii_digit()).collect();
    (8..=15).contains(&d.len()).then_some(d)
}

/// WhatsApp catalogue prices are integers in thousandths of the currency
/// unit (the protocol's `priceAmount1000`). POS prices are integers in the
/// currency's minor unit; BHD has 3 decimals, so fils map 1:1. No floating
/// point is involved.
pub fn to_wa_price(minor: i64, currency_digits: u32) -> AppResult<i64> {
    if currency_digits > 3 {
        return Err(AppError::validation("WhatsApp catalogue prices support at most 3 decimals."));
    }
    minor.checked_mul(10i64.pow(3 - currency_digits)).ok_or_else(|| AppError::validation("The price is too large for WhatsApp."))
}

/// Cut `s` to at most `max` characters, deterministically, marking the cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

/// What WhatsApp should show for one product. Field order is fixed, so the
/// JSON (and the fingerprint) is deterministic.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CatalogItem {
    pub name: String,
    pub description: Option<String>,
    /// Thousandths of the currency unit; None only for a hidden product
    /// without a usable price.
    pub price_1000: Option<i64>,
    pub currency: String,
    /// The POS product code (SKU) as WhatsApp's retailer id.
    pub retailer_id: String,
    /// The POS-managed picture (content hash); never an outside URL.
    pub image_hash: Option<String>,
    pub hidden: bool,
}

impl CatalogItem {
    pub fn fingerprint(&self) -> String {
        hex::encode(Sha256::digest(serde_json::to_vec(self).unwrap_or_default()))
    }
}

struct ProductData {
    product_id: String,
    sku: String,
    name: String,
    name_ar: Option<String>,
    description: Option<String>,
    active: bool,
    price: Option<i64>,
    image_hash: Option<String>,
}

fn load_products(c: &Connection, only: Option<&str>) -> AppResult<Vec<ProductData>> {
    let sql = format!(
        "SELECT p.product_id, p.sku, p.name, p.name_ar, p.description, p.active, {PRICE_SQL}, p.image_hash
         FROM products p {}",
        if only.is_some() { "WHERE p.product_id = ?1" } else { "" }
    );
    let mut st = c.prepare(&sql)?;
    let map = |r: &rusqlite::Row| {
        Ok(ProductData {
            product_id: r.get(0)?,
            sku: r.get(1)?,
            name: r.get(2)?,
            name_ar: r.get(3)?,
            description: r.get(4)?,
            active: r.get::<_, i64>(5)? == 1,
            price: r.get(6)?,
            image_hash: r.get(7)?,
        })
    };
    let rows = match only {
        Some(id) => st.query_map([id], map)?.collect::<Result<Vec<_>, _>>()?,
        None => st.query_map([], map)?.collect::<Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

/// Why a product is not published (None: it is publishable).
fn not_publishable_reason(p: &ProductData) -> Option<&'static str> {
    if !p.active {
        Some("archived")
    } else if p.name.trim().is_empty() {
        Some("no_name")
    } else if p.price.unwrap_or(0) <= 0 {
        Some("no_price")
    } else {
        None
    }
}

/// The representation of `p`. Description: the merchant's description, or
/// else the Arabic name the merchant entered, or none (nothing is invented).
fn item_for(p: &ProductData, currency: &str, digits: u32, hidden: bool) -> AppResult<CatalogItem> {
    let description = p
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .or_else(|| p.name_ar.as_deref().map(str::trim).filter(|d| !d.is_empty()))
        .map(|d| truncate_chars(d, DESCRIPTION_MAX_CHARS));
    let price_1000 = match p.price.filter(|v| *v > 0) {
        Some(v) => Some(to_wa_price(v, digits)?),
        None => None,
    };
    Ok(CatalogItem {
        name: truncate_chars(&p.name, NAME_MAX_CHARS),
        description,
        price_1000,
        currency: currency.to_string(),
        retailer_id: truncate_chars(&p.sku, RETAILER_ID_MAX_CHARS),
        image_hash: p.image_hash.clone(),
        hidden,
    })
}

/// What the worker must do for one claimed product.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogAction {
    /// Create (no remote id yet) or update the remote product, visible.
    Upsert,
    /// Update the remote product to hidden (archived / not publishable).
    Hide,
    /// Delete the POS-owned remote product (the POS product is gone).
    Delete,
}

#[derive(Debug, Clone)]
pub struct CatalogJob {
    pub account: String,
    pub product_id: String,
    pub action: CatalogAction,
    pub item: Option<CatalogItem>,
    pub remote_id: Option<String>,
    /// WhatsApp URL of the already uploaded picture, when it is unchanged.
    pub image_url: Option<String>,
    /// The POS-managed JPEG to upload (only when the picture changed).
    pub image_jpeg: Option<Vec<u8>>,
    pub attempt: i64,
}

#[derive(Debug, Clone)]
pub enum CatalogOutcome {
    /// The remote product now matches `item` (visible or hidden).
    Published {
        remote_id: String,
        item: CatalogItem,
        image_url: Option<String>,
        adopted: bool,
    },
    Deleted,
    /// The mapped remote product no longer exists on WhatsApp.
    RemoteMissing {
        error: String,
    },
    /// Disconnected, timeout, rate limit, server error: retried (bounded).
    Transient {
        error: String,
        retry_after_s: Option<u64>,
    },
    /// Validation or capability error: terminal until the product changes
    /// or an administrator retries.
    Failed {
        error: String,
    },
}

fn backoff_minutes(attempt: i64) -> i64 {
    match attempt {
        1 => 1,
        2 => 5,
        3 => 30,
        _ => 120,
    }
}

fn load_settings(c: &Connection) -> AppResult<WaCatalogSettings> {
    settings::get(c, KEY_WA_CATALOG)
}

fn is_published(c: &Connection, account: &str) -> AppResult<bool> {
    Ok(load_settings(c)?.published_accounts.iter().any(|a| a == account))
}

fn valid_account(account: &str) -> AppResult<String> {
    account_key(account).ok_or_else(|| AppError::conflict("WhatsApp is not linked to a number yet."))
}

struct Mapping {
    status: String,
    fingerprint: Option<String>,
    attempt_fingerprint: Option<String>,
    remote_id: Option<String>,
}

/// Queue every product whose WhatsApp representation differs from the last
/// synchronised one (and the POS-owned remote products whose POS product is
/// gone). Unchanged products are left alone. Returns how many were queued.
fn scan(c: &Connection, account: &str, currency: &str, digits: u32) -> AppResult<usize> {
    let products = load_products(c, None)?;
    let mut maps: HashMap<String, Mapping> = HashMap::new();
    {
        let mut st =
            c.prepare("SELECT product_id, status, fingerprint, remote_id, attempt_fingerprint FROM wa_catalog_products WHERE account=?1")?;
        for r in st.query_map([account], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Mapping { status: r.get(1)?, fingerprint: r.get(2)?, remote_id: r.get(3)?, attempt_fingerprint: r.get(4)? },
            ))
        })? {
            let (k, v) = r?;
            maps.insert(k, v);
        }
    }
    let now = time::now_str();
    let mut queued = 0usize;
    let queue = |pid: &str| -> AppResult<usize> {
        Ok(c.execute(
            "UPDATE wa_catalog_products SET status='queued', attempts=0, next_at=NULL, last_error=NULL, updated_at=?3
             WHERE account=?1 AND product_id=?2 AND status NOT IN ('queued','syncing')",
            params![account, pid, now],
        )?)
    };
    let mut seen: HashSet<&str> = HashSet::new();
    for p in &products {
        seen.insert(p.product_id.as_str());
        let m = maps.get(&p.product_id);
        match (not_publishable_reason(p), m) {
            (None, None) => {
                c.execute(
                    "INSERT INTO wa_catalog_products(account, product_id, status, created_at, updated_at) VALUES (?1,?2,'queued',?3,?3)",
                    params![account, p.product_id, now],
                )?;
                queued += 1;
            }
            (None, Some(m)) => {
                if matches!(m.status.as_str(), "queued" | "syncing" | "remote_missing") {
                    continue;
                }
                let fp = item_for(p, currency, digits, false)?.fingerprint();
                let changed = match m.status.as_str() {
                    // Failed: only a different representation is worth another try.
                    "failed" => m.attempt_fingerprint.as_deref() != Some(fp.as_str()),
                    "synced" => m.fingerprint.as_deref() != Some(fp.as_str()),
                    // hidden (re-activated), not_synced (now publishable), removed.
                    _ => true,
                };
                if changed {
                    queued += queue(&p.product_id)?;
                }
            }
            (Some(_), Some(m)) => {
                if matches!(m.status.as_str(), "queued" | "syncing" | "remote_missing" | "hidden") {
                    continue;
                }
                if m.status == "failed" {
                    let hidden_fp = item_for(p, currency, digits, true)?.fingerprint();
                    if m.attempt_fingerprint.as_deref() == Some(hidden_fp.as_str()) {
                        continue;
                    }
                }
                if m.remote_id.is_some() {
                    queued += queue(&p.product_id)?; // hide it
                } else if m.status != "not_synced" {
                    c.execute(
                        "UPDATE wa_catalog_products SET status='not_synced', last_error=NULL, updated_at=?3 WHERE account=?1 AND product_id=?2",
                        params![account, p.product_id, now],
                    )?;
                }
            }
            (Some(_), None) => {}
        }
    }
    // POS products that no longer exist: delete their POS-owned remote copy.
    for (pid, m) in &maps {
        if !seen.contains(pid.as_str()) && m.remote_id.is_some() && !matches!(m.status.as_str(), "queued" | "syncing" | "removed") {
            queued += queue(pid)?;
        }
    }
    // Categories → collections: recorded as unsupported (the linked client
    // cannot write collections; see docs/STATUS.md). Never re-created.
    c.execute(
        "INSERT OR IGNORE INTO wa_catalog_collections(account, category_id, status, last_error, updated_at)
         SELECT ?1, category_id, 'unsupported', 'Collections cannot be written through the linked WhatsApp client.', ?2 FROM categories",
        params![account, now],
    )?;
    if queued > 0 {
        tracing::info!(queued, "WhatsApp catalogue: changes queued");
    }
    Ok(queued)
}

impl AppCore {
    fn require_catalog_admin(&self, token: &str) -> AppResult<crate::auth::Session> {
        let s = self.session(token)?;
        s.require("whatsapp.manage")?;
        s.require("products.manage")?;
        Ok(s)
    }

    /// Worker: publishing was started for this account and auto-sync is on.
    pub fn wa_catalog_auto(&self, account: &str) -> AppResult<bool> {
        let Some(acc) = account_key(account) else { return Ok(false) };
        self.db.read(|c| {
            let s = load_settings(c)?;
            Ok(s.auto_sync && s.published_accounts.contains(&acc))
        })
    }

    /// Worker: an administrator started publishing to this account.
    pub fn wa_catalog_published(&self, account: &str) -> AppResult<bool> {
        let Some(acc) = account_key(account) else { return Ok(false) };
        self.db.read(|c| is_published(c, &acc))
    }

    /// Worker: queue what changed since the last sync (auto-sync).
    pub fn wa_catalog_scan(&self, account: &str) -> AppResult<usize> {
        let acc = valid_account(account)?;
        self.db.write(|tx| {
            if !is_published(tx, &acc)? {
                return Ok(0);
            }
            let (cur, digits) = self.currency(tx)?;
            scan(tx, &acc, &cur, digits)
        })
    }

    /// First sync (and later full reconciliations): an administrator starts
    /// publishing the catalogue to the linked account. Queues every product
    /// that differs from WhatsApp; the worker does the remote writes.
    pub fn wa_catalog_start(&self, token: &str, account: &str) -> AppResult<Value> {
        let s = self.require_catalog_admin(token)?;
        self.require_back_office_writable()?;
        let acc = valid_account(account)?;
        let actor = self.actor(&s, None);
        let queued = self.db.write(|tx| {
            let mut st = load_settings(tx)?;
            let first = !st.published_accounts.contains(&acc);
            if first {
                st.published_accounts.push(acc.clone());
                settings::put(tx, KEY_WA_CATALOG, &st, Some(&s.user_id))?;
            }
            let (cur, digits) = self.currency(tx)?;
            let n = scan(tx, &acc, &cur, digits)?;
            audit::record(
                tx,
                &actor,
                "whatsapp.catalog_sync",
                "whatsapp",
                None,
                None,
                Some(&json!({ "account": acc, "queued": n, "first": first })),
            )?;
            Ok(n)
        })?;
        tracing::info!(queued, "WhatsApp catalogue: full sync started");
        self.wa_catalog_overview(token, Some(account))
    }

    /// Retry failed and remote-missing products (all, or one).
    pub fn wa_catalog_retry(&self, token: &str, account: &str, product_id: Option<&str>) -> AppResult<Value> {
        let s = self.require_catalog_admin(token)?;
        let acc = valid_account(account)?;
        let pid = product_id.map(|p| validate::id(p, "Product")).transpose()?;
        let actor = self.actor(&s, None);
        let n = self.db.write(|tx| {
            let now = time::now_str();
            let n = match &pid {
                Some(p) => tx.execute(
                    "UPDATE wa_catalog_products SET status='queued', attempts=0, next_at=NULL, last_error=NULL,
                        remote_id=CASE WHEN status='remote_missing' THEN NULL ELSE remote_id END, updated_at=?3
                     WHERE account=?1 AND product_id=?2 AND status IN ('failed','remote_missing','not_synced','synced','hidden')",
                    params![acc, p, now],
                )?,
                None => tx.execute(
                    "UPDATE wa_catalog_products SET status='queued', attempts=0, next_at=NULL, last_error=NULL,
                        remote_id=CASE WHEN status='remote_missing' THEN NULL ELSE remote_id END, updated_at=?2
                     WHERE account=?1 AND status IN ('failed','remote_missing')",
                    params![acc, now],
                )?,
            };
            audit::record(tx, &actor, "whatsapp.catalog_retry", "whatsapp", pid.as_deref(), None, Some(&json!({ "queued": n })))?;
            Ok(n)
        })?;
        Ok(json!({ "queued": n }))
    }

    pub fn wa_catalog_configure(&self, token: &str, auto_sync: bool) -> AppResult<Value> {
        let s = self.require_catalog_admin(token)?;
        let actor = self.actor(&s, None);
        self.db.write(|tx| {
            let mut st = load_settings(tx)?;
            st.auto_sync = auto_sync;
            settings::put(tx, KEY_WA_CATALOG, &st, Some(&s.user_id))?;
            audit::record(tx, &actor, "settings.whatsapp_catalog", "settings", None, None, Some(&json!({ "auto_sync": auto_sync })))?;
            Ok(())
        })?;
        Ok(json!({ "auto_sync": auto_sync }))
    }

    /// Counts and state for the admin screen (for the linked account).
    pub fn wa_catalog_overview(&self, token: &str, account: Option<&str>) -> AppResult<Value> {
        let s = self.session(token)?;
        s.require("whatsapp.manage")?;
        let acc = account.and_then(account_key);
        self.db.read(|c| {
            let st = load_settings(c)?;
            let products = load_products(c, None)?;
            let mut publishable = 0i64;
            let mut not_publishable: HashMap<&str, i64> = HashMap::new();
            for p in &products {
                match not_publishable_reason(p) {
                    None => publishable += 1,
                    Some(r) => *not_publishable.entry(r).or_default() += 1,
                }
            }
            let (mut counts, mut last_synced, mut failures, mut collections) = (HashMap::new(), None::<String>, vec![], 0i64);
            if let Some(a) = &acc {
                let mut q = c.prepare("SELECT status, COUNT(*) FROM wa_catalog_products WHERE account=?1 GROUP BY status")?;
                counts = q.query_map([a], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<HashMap<_, _>, _>>()?;
                last_synced = c.query_row("SELECT MAX(last_synced_at) FROM wa_catalog_products WHERE account=?1", [a], |r| r.get(0))?;
                let mut q = c.prepare(
                    "SELECT m.product_id, p.name, m.status, m.last_error FROM wa_catalog_products m LEFT JOIN products p ON p.product_id=m.product_id
                     WHERE m.account=?1 AND m.status IN ('failed','remote_missing') ORDER BY m.updated_at DESC LIMIT 20",
                )?;
                failures = q
                    .query_map([a], |r| {
                        Ok(json!({ "product_id": r.get::<_, String>(0)?, "name": r.get::<_, Option<String>>(1)?,
                                   "status": r.get::<_, String>(2)?, "error": r.get::<_, Option<String>>(3)? }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                collections = c.query_row("SELECT COUNT(*) FROM wa_catalog_collections WHERE account=?1", [a], |r| r.get(0))?;
            }
            Ok(json!({
                "account": acc,
                "published": acc.as_ref().is_some_and(|a| st.published_accounts.contains(a)),
                "auto_sync": st.auto_sync,
                "publishable": publishable,
                "not_publishable": not_publishable,
                "counts": counts,
                "last_synced_at": last_synced,
                "failures": failures,
                "categories": c.query_row("SELECT COUNT(*) FROM categories", [], |r| r.get::<_, i64>(0))?,
                "collections_recorded": collections,
            }))
        })
    }

    /// One product's catalogue state (product editor).
    pub fn wa_catalog_product_state(&self, token: &str, account: Option<&str>, product_id: &str) -> AppResult<Value> {
        let s = self.session(token)?;
        s.require("products.view")?;
        let pid = validate::id(product_id, "Product")?;
        let Some(acc) = account.and_then(account_key) else { return Ok(json!({ "status": null, "published": false })) };
        self.db.read(|c| {
            let published = is_published(c, &acc)?;
            let row: Option<(String, Option<String>, Option<String>, Option<String>)> = c
                .query_row(
                    "SELECT status, remote_id, last_synced_at, last_error FROM wa_catalog_products WHERE account=?1 AND product_id=?2",
                    params![acc, pid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let reason = load_products(c, Some(&pid))?.first().and_then(not_publishable_reason);
            Ok(match row {
                Some((status, remote, synced, err)) => json!({ "published": published, "status": status, "on_whatsapp": remote.is_some(),
                    "last_synced_at": synced, "last_error": err, "not_publishable": reason }),
                None => json!({ "published": published, "status": "not_synced", "on_whatsapp": false, "not_publishable": reason }),
            })
        })
    }

    /// Worker: the remote id is already mapped to a POS product (adoption guard).
    pub fn wa_catalog_remote_owner(&self, account: &str, remote_id: &str) -> AppResult<Option<String>> {
        let acc = valid_account(account)?;
        self.db.read(|c| {
            Ok(c.query_row("SELECT product_id FROM wa_catalog_products WHERE account=?1 AND remote_id=?2", params![acc, remote_id], |r| {
                r.get(0)
            })
            .optional()?)
        })
    }

    /// Worker: claim up to `limit` due products. The claim is a conditional
    /// update, so two workers never take the same product; the action and
    /// payload are computed from the POS data at claim time.
    pub fn wa_catalog_claim(&self, account: &str, limit: usize) -> AppResult<Vec<CatalogJob>> {
        let acc = valid_account(account)?;
        let now = time::now_str();
        let stale = time::fmt(time::now() - chrono::Duration::minutes(STALE_CLAIM_MINUTES));
        let due: bool = self.db.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM wa_catalog_products WHERE account=?1 AND
                   ((status='queued' AND (next_at IS NULL OR next_at <= ?2)) OR (status='syncing' AND claimed_at < ?3)))",
                params![acc, now, stale],
                |r| r.get(0),
            )?)
        })?;
        if !due {
            return Ok(vec![]);
        }
        self.db.write(|tx| {
            if !is_published(tx, &acc)? {
                return Ok(vec![]);
            }
            let n = tx.execute(
                "UPDATE wa_catalog_products SET status='queued', claimed_at=NULL WHERE account=?1 AND status='syncing' AND claimed_at < ?2",
                params![acc, stale],
            )?;
            if n > 0 {
                tracing::warn!(n, "WhatsApp catalogue: abandoned claims re-queued");
            }
            let (cur, digits) = self.currency(tx)?;
            let pids: Vec<String> = {
                let mut st = tx.prepare(
                    "SELECT product_id FROM wa_catalog_products WHERE account=?1 AND status='queued' AND (next_at IS NULL OR next_at <= ?2)
                     ORDER BY updated_at LIMIT ?3",
                )?;
                let rows = st.query_map(params![acc, now, limit as i64], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
                rows
            };
            let mut jobs = vec![];
            for pid in pids {
                let claimed = tx.execute(
                    "UPDATE wa_catalog_products SET status='syncing', claimed_at=?3, last_attempt_at=?3, attempts=attempts+1, updated_at=?3
                     WHERE account=?1 AND product_id=?2 AND status='queued'",
                    params![acc, pid, now],
                )?;
                if claimed != 1 {
                    continue;
                }
                let (remote_id, image_hash, image_url, attempt): (Option<String>, Option<String>, Option<String>, i64) = tx.query_row(
                    "SELECT remote_id, image_hash, image_url, attempts FROM wa_catalog_products WHERE account=?1 AND product_id=?2",
                    params![acc, pid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                let product = load_products(tx, Some(&pid))?.into_iter().next();
                let (action, item) = match &product {
                    None => (CatalogAction::Delete, None),
                    Some(p) => match not_publishable_reason(p) {
                        None => (CatalogAction::Upsert, Some(item_for(p, &cur, digits, false)?)),
                        Some(_) => (CatalogAction::Hide, Some(item_for(p, &cur, digits, true)?)),
                    },
                };
                if remote_id.is_none() && action != CatalogAction::Upsert {
                    // Nothing on WhatsApp to hide or delete.
                    tx.execute(
                        "UPDATE wa_catalog_products SET status=?3, claimed_at=NULL, updated_at=?4 WHERE account=?1 AND product_id=?2",
                        params![acc, pid, if action == CatalogAction::Delete { "removed" } else { "not_synced" }, now],
                    )?;
                    continue;
                }
                if let Some(i) = &item {
                    tx.execute(
                        "UPDATE wa_catalog_products SET attempt_fingerprint=?3 WHERE account=?1 AND product_id=?2",
                        params![acc, pid, i.fingerprint()],
                    )?;
                }
                // Re-use the uploaded picture while it is unchanged; otherwise
                // upload the POS-managed JPEG (never an outside URL).
                let want_hash = item.as_ref().and_then(|i| i.image_hash.clone());
                let (mut reuse, mut jpeg) = (None, None);
                if let Some(h) = &want_hash {
                    if image_hash.as_deref() == Some(h.as_str()) && image_url.is_some() {
                        reuse = image_url.clone();
                    } else {
                        let data: Option<String> =
                            tx.query_row("SELECT data_b64 FROM product_images WHERE image_hash=?1", [h], |r| r.get(0)).optional()?;
                        jpeg = data.and_then(|d| crate::ids::b64_decode(&d));
                    }
                }
                jobs.push(CatalogJob {
                    account: acc.clone(),
                    product_id: pid,
                    action,
                    item,
                    remote_id,
                    image_url: reuse,
                    image_jpeg: jpeg,
                    attempt,
                });
            }
            Ok(jobs)
        })
    }

    /// Worker: record the outcome of a claimed product. Only a claimed row
    /// is written; a failed remote write is never recorded as synchronised.
    pub fn wa_catalog_complete(&self, account: &str, product_id: &str, outcome: CatalogOutcome) -> AppResult<String> {
        let acc = valid_account(account)?;
        let now = time::now_str();
        self.db.write(|tx| {
            let attempts: Option<i64> = tx
                .query_row(
                    "SELECT attempts FROM wa_catalog_products WHERE account=?1 AND product_id=?2 AND status='syncing'",
                    params![acc, product_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(attempts) = attempts else { return Ok("superseded".to_string()) };
            let state = match outcome {
                CatalogOutcome::Published { remote_id, item, image_url, adopted } => {
                    let owner: Option<String> = tx
                        .query_row(
                            "SELECT product_id FROM wa_catalog_products WHERE account=?1 AND remote_id=?2 AND product_id<>?3",
                            params![acc, remote_id, product_id],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if owner.is_some() {
                        tx.execute(
                            "UPDATE wa_catalog_products SET status='failed', claimed_at=NULL, next_at=NULL, updated_at=?3,
                                last_error='WhatsApp returned a product that is already linked to another POS product.'
                             WHERE account=?1 AND product_id=?2",
                            params![acc, product_id, now],
                        )?;
                        "failed"
                    } else {
                        let status = if item.hidden { "hidden" } else { "synced" };
                        tx.execute(
                            "UPDATE wa_catalog_products SET status=?3, remote_id=?4, fingerprint=?5, image_hash=?6, image_url=?7,
                                adopted=MAX(adopted, ?8), attempts=0, next_at=NULL, claimed_at=NULL, last_error=NULL,
                                last_synced_at=?9, updated_at=?9
                             WHERE account=?1 AND product_id=?2",
                            params![
                                acc,
                                product_id,
                                status,
                                remote_id,
                                item.fingerprint(),
                                item.image_hash,
                                image_url,
                                adopted as i64,
                                now
                            ],
                        )?;
                        status
                    }
                }
                CatalogOutcome::Deleted => {
                    tx.execute(
                        "UPDATE wa_catalog_products SET status='removed', remote_id=NULL, fingerprint=NULL, image_url=NULL, claimed_at=NULL,
                            attempts=0, last_error=NULL, last_synced_at=?3, updated_at=?3 WHERE account=?1 AND product_id=?2",
                        params![acc, product_id, now],
                    )?;
                    "removed"
                }
                CatalogOutcome::RemoteMissing { error } => {
                    tx.execute(
                        "UPDATE wa_catalog_products SET status='remote_missing', claimed_at=NULL, last_error=?3, updated_at=?4
                         WHERE account=?1 AND product_id=?2",
                        params![acc, product_id, error, now],
                    )?;
                    "remote_missing"
                }
                CatalogOutcome::Failed { error } => {
                    tx.execute(
                        "UPDATE wa_catalog_products SET status='failed', claimed_at=NULL, next_at=NULL, last_error=?3, updated_at=?4
                         WHERE account=?1 AND product_id=?2",
                        params![acc, product_id, error, now],
                    )?;
                    "failed"
                }
                CatalogOutcome::Transient { error, retry_after_s } => {
                    if attempts >= MAX_ATTEMPTS {
                        tx.execute(
                            "UPDATE wa_catalog_products SET status='failed', claimed_at=NULL, next_at=NULL, last_error=?3, updated_at=?4
                             WHERE account=?1 AND product_id=?2",
                            params![acc, product_id, error, now],
                        )?;
                        "failed"
                    } else {
                        let secs = (backoff_minutes(attempts) * 60).max(retry_after_s.unwrap_or(0) as i64);
                        let next = time::fmt(time::now() + chrono::Duration::seconds(secs));
                        tx.execute(
                            "UPDATE wa_catalog_products SET status='queued', claimed_at=NULL, next_at=?3, last_error=?4, updated_at=?5
                             WHERE account=?1 AND product_id=?2",
                            params![acc, product_id, next, error, now],
                        )?;
                        "retry"
                    }
                }
            };
            Ok(state.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bhd_prices_convert_exactly_to_thousandths() {
        // BHD has 3 decimals: fils are thousandths already.
        for (minor, want) in [(100, 100), (1000, 1000), (1250, 1250), (9990, 9990), (100_000, 100_000)] {
            assert_eq!(to_wa_price(minor, 3).unwrap(), want, "{minor} fils");
        }
        // Two-decimal and zero-decimal currencies scale up exactly.
        assert_eq!(to_wa_price(1999, 2).unwrap(), 19_990);
        assert_eq!(to_wa_price(5, 0).unwrap(), 5000);
        assert!(to_wa_price(1, 4).is_err(), "more than 3 decimals cannot be represented");
        assert!(to_wa_price(i64::MAX, 2).is_err());
    }

    #[test]
    fn accounts_are_keyed_by_number_without_device() {
        assert_eq!(account_key("97330000000:12@s.whatsapp.net").as_deref(), Some("97330000000"));
        assert_eq!(account_key("97330000000@s.whatsapp.net").as_deref(), Some("97330000000"));
        assert_eq!(account_key("+973 3000 0000").as_deref(), Some("97330000000"));
        assert!(account_key("12@lid").is_none());
    }

    #[test]
    fn truncation_is_deterministic_and_marked() {
        assert_eq!(truncate_chars("  Milk  ", 10), "Milk");
        let long = "حليب ".repeat(60);
        let t = truncate_chars(&long, NAME_MAX_CHARS);
        assert_eq!(t.chars().count(), NAME_MAX_CHARS.min(t.chars().count()));
        assert!(t.ends_with('…') && t.chars().count() <= NAME_MAX_CHARS);
        assert_eq!(t, truncate_chars(&long, NAME_MAX_CHARS));
    }

    #[test]
    fn fingerprint_changes_only_with_the_published_fields() {
        let base = CatalogItem {
            name: "Almarai Milk 1L".into(),
            description: None,
            price_1000: Some(850),
            currency: "BHD".into(),
            retailer_id: "100001".into(),
            image_hash: None,
            hidden: false,
        };
        let fp = base.fingerprint();
        assert_eq!(fp, base.clone().fingerprint());
        for changed in [
            CatalogItem { name: "Almarai Milk 2L".into(), ..base.clone() },
            CatalogItem { price_1000: Some(900), ..base.clone() },
            CatalogItem { description: Some("Fresh".into()), ..base.clone() },
            CatalogItem { image_hash: Some("ab".repeat(32)), ..base.clone() },
            CatalogItem { hidden: true, ..base.clone() },
        ] {
            assert_ne!(changed.fingerprint(), fp);
        }
    }
}
