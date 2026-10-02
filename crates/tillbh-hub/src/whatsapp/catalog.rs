//! WhatsApp Business catalogue worker: publishes the POS catalogue through
//! the same linked WhatsApp session the service already runs (no second
//! connection, no Meta API). It runs next to the message I/O worker, only
//! where the session lives (hub / standalone), one product at a time.
//!
//! * Capability comes from the live connection (`catalog_capability`):
//!   personal account, Business account without a readable catalogue,
//!   supported, or unavailable right now. Nothing is written unless it is
//!   `supported` and an administrator started publishing for this account.
//! * Work is claimed from the core (`wa_catalog_claim`), which decides the
//!   action from current POS data and scopes everything to the linked
//!   account, so a new account never reuses another account's remote ids.
//! * Before creating a product without a mapping, the account's catalogue is
//!   read once per pass and a remote product carrying exactly this retailer
//!   id (the POS product code) that no other POS product owns is adopted
//!   instead: a crash between "created" and "recorded" never duplicates.
//! * POS writes never wait for this; a disconnect only leaves work queued.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tillbh_core::wa_catalog::{account_key, CatalogAction, CatalogJob, CatalogOutcome};
use tillbh_core::{AppError, AppResult};
use serde::Serialize;

use super::adapter::*;
use super::service::WhatsAppService;

const TICK: Duration = Duration::from_secs(5);
/// Re-check the capability this often while connected.
const RECHECK: Duration = Duration::from_secs(30 * 60);
const DETECT_TIMEOUT: Duration = Duration::from_secs(40);
const OP_TIMEOUT: Duration = Duration::from_secs(60);
/// Products per claim; one remote change at a time, spaced out.
const BATCH: usize = 5;
const BETWEEN_OPS: Duration = Duration::from_millis(1200);
/// Catalogue pages read for reconciliation at most (50 per page).
const MAX_LIST_PAGES: usize = 40;
/// Full POS ↔ WhatsApp comparison at least this often (auto-sync).
const SCAN_EVERY: Duration = Duration::from_secs(60);

/// What the admin screen shows about the catalogue connection.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct CatalogInfo {
    /// disconnected | checking | personal | business_no_catalog | supported |
    /// unavailable | unsupported | terminal
    pub capability: String,
    pub detail: Option<String>,
    /// The linked account's number (digits) the capability refers to.
    pub account: Option<String>,
    pub checked_at: Option<String>,
    /// Collections (POS categories) can be written through this client.
    pub collections: bool,
    /// Products written in the current / last pass.
    pub last_pass_at: Option<String>,
    pub last_pass_done: u32,
    pub last_error: Option<String>,
}

fn now() -> String {
    tillbh_core::time::now_str()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::internal(format!("worker failed: {e}")))?
}

fn capability_name(c: &CatalogCapability) -> (&'static str, Option<String>) {
    match c {
        CatalogCapability::Personal => ("personal", None),
        CatalogCapability::BusinessNoCatalog(m) => ("business_no_catalog", Some(m.clone())),
        CatalogCapability::Supported => ("supported", None),
        CatalogCapability::Unavailable(m) => ("unavailable", Some(m.clone())),
        CatalogCapability::Unsupported => ("unsupported", None),
    }
}

fn outcome_of(e: AdapterError) -> CatalogOutcome {
    if e.permanent {
        CatalogOutcome::Failed { error: e.message }
    } else {
        CatalogOutcome::Transient { error: e.message, retry_after_s: e.retry_after_s }
    }
}

/// Remote products of this account by retailer id (for adoption), read once
/// per pass. `None`: the catalogue could not be read.
async fn remote_index(session: &Arc<dyn AdapterSession>) -> Option<HashMap<String, Vec<String>>> {
    let mut by_retailer: HashMap<String, Vec<String>> = HashMap::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_LIST_PAGES {
        let (items, next) = tokio::time::timeout(OP_TIMEOUT, session.catalog_list(cursor.as_deref())).await.ok()?.ok()?;
        for p in items {
            if let Some(r) = p.retailer_id {
                by_retailer.entry(r).or_default().push(p.id);
            }
        }
        match next {
            Some(c) if cursor.as_deref() != Some(c.as_str()) => cursor = Some(c),
            _ => break,
        }
    }
    Some(by_retailer)
}

async fn process(
    svc: &Arc<WhatsAppService>,
    session: &Arc<dyn AdapterSession>,
    job: CatalogJob,
    index: &mut Option<Option<HashMap<String, Vec<String>>>>,
) -> CatalogOutcome {
    let core = svc.core().clone();
    match job.action {
        CatalogAction::Delete => {
            let Some(id) = job.remote_id.clone() else { return CatalogOutcome::Deleted };
            match tokio::time::timeout(OP_TIMEOUT, session.catalog_delete(&[id])).await {
                Ok(Ok(_)) => CatalogOutcome::Deleted,
                Ok(Err(e)) if e.not_found => CatalogOutcome::Deleted,
                Ok(Err(e)) => outcome_of(e),
                Err(_) => CatalogOutcome::Transient { error: "WhatsApp did not answer in time.".into(), retry_after_s: None },
            }
        }
        CatalogAction::Upsert | CatalogAction::Hide => {
            let Some(item) = job.item.clone() else { return CatalogOutcome::Failed { error: "Nothing to publish.".into() } };
            // The POS-managed picture: re-used while unchanged, else uploaded.
            let image_url = match (&job.image_url, job.image_jpeg) {
                (Some(u), _) => Some(u.clone()),
                (None, Some(bytes)) => match tokio::time::timeout(OP_TIMEOUT, session.catalog_upload_image(bytes)).await {
                    Ok(Ok(u)) => Some(u),
                    Ok(Err(e)) => return outcome_of(e),
                    Err(_) => return CatalogOutcome::Transient { error: "The picture upload timed out.".into(), retry_after_s: None },
                },
                (None, None) => None,
            };
            let product = CatalogProduct {
                name: item.name.clone(),
                description: item.description.clone(),
                price_1000: item.price_1000,
                currency: item.currency.clone(),
                retailer_id: item.retailer_id.clone(),
                image_url: image_url.clone(),
                hidden: item.hidden,
            };
            let hide = job.action == CatalogAction::Hide;
            let (target, adopted) = match &job.remote_id {
                Some(id) => (Some(id.clone()), false),
                None => {
                    if index.is_none() {
                        *index = Some(remote_index(session).await);
                    }
                    let Some(Some(idx)) = index.as_ref() else {
                        // Without the catalogue we cannot rule out a duplicate: retry later.
                        return CatalogOutcome::Transient {
                            error: "The WhatsApp catalogue could not be read to check for an existing copy.".into(),
                            retry_after_s: None,
                        };
                    };
                    let mut adopt = None;
                    if let Some(ids) = idx.get(&item.retailer_id).filter(|ids| ids.len() == 1 && !item.retailer_id.is_empty()) {
                        let (c, acc, rid) = (core.clone(), job.account.clone(), ids[0].clone());
                        if blocking(move || c.wa_catalog_remote_owner(&acc, &rid)).await.ok().flatten().is_none() {
                            adopt = Some(ids[0].clone());
                        }
                    }
                    let adopted = adopt.is_some();
                    (adopt, adopted)
                }
            };
            let res = match &target {
                Some(id) => tokio::time::timeout(OP_TIMEOUT, session.catalog_update(id, &product)).await,
                None => tokio::time::timeout(OP_TIMEOUT, session.catalog_create(&product)).await,
            };
            match res {
                Ok(Ok(remote)) => {
                    let what = match (&target, hide) {
                        (_, true) => "hidden",
                        (Some(_), _) if adopted => "adopted and updated",
                        (Some(_), _) => "updated",
                        (None, _) => "created",
                    };
                    tracing::info!(product_id = %job.product_id, remote_id = %remote.id, what, "WhatsApp catalogue product");
                    CatalogOutcome::Published { remote_id: remote.id, item, image_url, adopted }
                }
                Ok(Err(e)) if e.not_found && hide => CatalogOutcome::Deleted,
                Ok(Err(e)) if e.not_found => CatalogOutcome::RemoteMissing { error: "The product was deleted on WhatsApp.".into() },
                Ok(Err(e)) => outcome_of(e),
                Err(_) => CatalogOutcome::Transient { error: "WhatsApp did not answer in time.".into(), retry_after_s: None },
            }
        }
    }
}

/// Runs for the life of the WhatsApp service.
pub(super) async fn catalog_worker(svc: Arc<WhatsAppService>) {
    let mut rx = svc.session_watch();
    let mut detected: Option<(String, Instant)> = None;
    let mut last_session: Option<usize> = None;
    let mut last_scan: Option<Instant> = None;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(TICK) => {}
            _ = svc.catalog_poke.notified() => {}
            _ = rx.changed() => {}
        }
        let session = rx.borrow().clone();
        let Some(session) = session.filter(|s| s.connected()) else {
            svc.set_catalog(|i| {
                i.capability = "disconnected".into();
                i.detail = None;
            });
            detected = None;
            continue;
        };
        // A new client (restart, relink) is re-checked.
        let sid = Arc::as_ptr(&session) as *const () as usize;
        if last_session != Some(sid) {
            last_session = Some(sid);
            detected = None;
        }
        let core = svc.core().clone();
        if core.device().is_some_and(|d| d.mode == "terminal") {
            svc.set_catalog(|i| i.capability = "terminal".into());
            continue;
        }
        let Some(account) = svc.status().account.as_deref().and_then(account_key) else {
            svc.set_catalog(|i| i.capability = "checking".into());
            continue;
        };
        let force = svc.take_catalog_recheck();
        let stale = force || detected.as_ref().map(|(a, t)| a != &account || t.elapsed() > RECHECK).unwrap_or(true);
        if stale {
            svc.set_catalog(|i| {
                i.capability = "checking".into();
                i.account = Some(account.clone());
            });
            let cap = match tokio::time::timeout(DETECT_TIMEOUT, session.catalog_capability()).await {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => CatalogCapability::Unavailable(e.message),
                Err(_) => CatalogCapability::Unavailable("WhatsApp did not answer in time.".into()),
            };
            let (name, detail) = capability_name(&cap);
            tracing::info!(capability = name, "WhatsApp catalogue capability detected");
            let collections = session.catalog_collections_writable();
            svc.set_catalog(|i| {
                i.capability = name.into();
                i.detail = detail.clone();
                i.account = Some(account.clone());
                i.checked_at = Some(now());
                i.collections = collections;
            });
            // Unavailable is re-checked on the next tick; the others hold.
            detected = (name != "unavailable").then(|| (account.clone(), Instant::now()));
        }
        if svc.catalog().capability != "supported" {
            continue;
        }
        let (c, a) = (core.clone(), account.clone());
        let published = blocking(move || c.wa_catalog_published(&a)).await.unwrap_or(false);
        if !published {
            continue;
        }
        // Compare POS and WhatsApp after a catalogue change, and at least
        // once a minute (changes made outside a command, e.g. a picture the
        // image worker found). Unchanged products produce no remote write.
        let changed = svc.take_catalog_dirty();
        let due_scan = changed || last_scan.is_none_or(|t| t.elapsed() > SCAN_EVERY);
        let (c, a) = (core.clone(), account.clone());
        if due_scan && blocking(move || c.wa_catalog_auto(&a)).await.unwrap_or(false) {
            last_scan = Some(Instant::now());
            let (c, a) = (core.clone(), account.clone());
            if let Err(e) = blocking(move || c.wa_catalog_scan(&a)).await {
                tracing::warn!(error = %e.message, "WhatsApp catalogue scan");
            }
        }
        let (c, a) = (core.clone(), account.clone());
        let jobs = match blocking(move || c.wa_catalog_claim(&a, BATCH)).await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e.message, "WhatsApp catalogue claim");
                continue;
            }
        };
        if jobs.is_empty() {
            continue;
        }
        let mut index = None;
        let mut done = 0u32;
        for job in jobs {
            if !session.connected() {
                // Stop the pass; claimed rows are re-queued by the core after
                // the claim timeout, or complete as transient here.
                let (c, a, p) = (core.clone(), job.account.clone(), job.product_id.clone());
                let _ = blocking(move || {
                    c.wa_catalog_complete(&a, &p, CatalogOutcome::Transient { error: "WhatsApp disconnected.".into(), retry_after_s: None })
                })
                .await;
                continue;
            }
            let pid = job.product_id.clone();
            let acc = job.account.clone();
            let outcome = process(&svc, &session, job, &mut index).await;
            if let CatalogOutcome::Failed { error } | CatalogOutcome::Transient { error, .. } = &outcome {
                tracing::warn!(product_id = %pid, error = %error, "WhatsApp catalogue product not synchronised");
                let e = error.clone();
                svc.set_catalog(|i| i.last_error = Some(e));
            }
            let c = core.clone();
            match blocking(move || c.wa_catalog_complete(&acc, &pid, outcome)).await {
                Ok(state) => {
                    if state != "retry" && state != "failed" {
                        done += 1;
                    }
                }
                Err(e) => tracing::warn!(error = %e.message, "WhatsApp catalogue: outcome not recorded"),
            }
            tokio::time::sleep(BETWEEN_OPS).await;
        }
        svc.set_catalog(|i| {
            i.last_pass_at = Some(now());
            i.last_pass_done = done;
        });
        // More may be due: go again without waiting for the tick.
        svc.catalog_poke.notify_one();
    }
}
