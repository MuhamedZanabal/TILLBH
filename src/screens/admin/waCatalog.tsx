// WhatsApp → Catalogue: publish the POS catalogue to the linked WhatsApp
// Business account, through the same WhatsApp link that sends receipts.
// The POS stays the source of truth; nothing is published until an owner
// presses "Sync catalogue" for the linked number.
import { useEffect, useState } from "react";
import { RefreshCw, Store } from "lucide-react";
import { api } from "../../api";
import type { WaCatalogCapability, WaCatalogItemStatus, WaCatalogProductState, WaCatalogStatus } from "../../api/types";
import { Banner, Button, Checkbox, Chip, Skeleton } from "../../components/ui";
import { useFeature } from "../../components/FeatureGate";
import { useSession } from "../../state/session";
import { useToast } from "../../components/toast";
import { Confirm } from "./common";
import { formatDateTime } from "../../lib/time";
import { t, tb } from "../../i18n";

export function capabilityText(c: WaCatalogCapability): string {
  switch (c) {
    case "supported":
      return t("WhatsApp Business account with a catalogue");
    case "personal":
      return t("Personal WhatsApp account: catalogues need WhatsApp Business");
    case "business_no_catalog":
      return t("WhatsApp Business account, but its catalogue could not be read");
    case "unavailable":
      return t("Could not check the catalogue right now");
    case "unsupported":
      return t("This WhatsApp client cannot manage catalogues");
    case "terminal":
      return t("Catalogue publishing runs on the hub computer");
    case "checking":
      return t("Checking the linked account…");
    default:
      return t("WhatsApp is not connected");
  }
}

export function itemStatusText(s: WaCatalogItemStatus | null | undefined): string {
  switch (s) {
    case "synced":
      return t("On WhatsApp");
    case "queued":
    case "syncing":
      return t("Waiting to publish");
    case "hidden":
      return t("Hidden on WhatsApp");
    case "removed":
      return t("Removed from WhatsApp");
    case "failed":
      return t("Could not publish");
    case "remote_missing":
      return t("Deleted on WhatsApp");
    default:
      return t("Not on WhatsApp");
  }
}

export function WaCatalog() {
  const toast = useToast();
  const { has } = useSession();
  const canManage = has("whatsapp.manage") && has("products.manage");
  const [st, setSt] = useState<WaCatalogStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "sync" | "retry" | "auto" | "check">(null);
  const [confirm, setConfirm] = useState(false);
  const load = () =>
    api.whatsapp
      .catalogStatus()
      .then((s) => (setSt(s), setError(null)))
      .catch((e: Error) => setError(e.message));
  useEffect(() => {
    void load();
    const id = setInterval(() => void load(), 4000);
    return () => clearInterval(id);
  }, []);
  const run = async (kind: "sync" | "retry" | "auto" | "check", fn: () => Promise<unknown>, ok?: string) => {
    setBusy(kind);
    try {
      await fn();
      if (ok) toast("success", ok);
      await load();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  if (!st) return error ? <Banner tone="danger">{tb(error)}</Banner> : <Skeleton />;
  const cap = st.capability;
  const cat = st.catalog;
  const supported = cap.capability === "supported";
  const c = cat.counts;
  const pending = (c.queued ?? 0) + (c.syncing ?? 0);
  return (
    <div className="col gap-16" data-testid="wa-catalog" data-capability={cap.capability}>
      <div className="card card-pad col gap-12">
        <div className="row">
          <Store size={20} aria-hidden />
          <h3 className="grow">{t("WhatsApp catalogue")}</h3>
          <Chip tone={supported ? "success" : "default"}>{capabilityText(cap.capability)}</Chip>
        </div>
        <div className="tiny">
          {t(
            "Publishes this shop's products to the WhatsApp Business catalogue of the linked number, through the same WhatsApp link that sends receipts. AMWAPOS stays the source of truth: prices, names, descriptions and pictures flow from AMWAPOS to WhatsApp, never back.",
          )}
        </div>
        {cap.detail ? <div className="tiny muted">{tb(cap.detail)}</div> : null}
        <dl className="kv" data-testid="wa-catalog-capability">
          <dt>{t("Linked number")}</dt>
          <dd>{cap.account ? `+${cap.account}` : "—"}</dd>
          <dt>{t("Products")}</dt>
          <dd>{supported ? t("Supported") : t("Not available")}</dd>
          <dt>{t("Collections (categories)")}</dt>
          <dd>
            {cap.collections
              ? t("Supported")
              : t("Not supported by this WhatsApp link: products are published without collections.")}
          </dd>
          {cap.checked_at ? (
            <>
              <dt>{t("Checked")}</dt>
              <dd>{formatDateTime(cap.checked_at)}</dd>
            </>
          ) : null}
        </dl>
        {canManage ? (
          <div className="row wrap">
            <Button
              icon={<RefreshCw size={16} />}
              loading={busy === "check"}
              disabled={!!busy}
              onClick={() => run("check", () => api.whatsapp.catalogRecheck())}
            >
              {t("Check again")}
            </Button>
          </div>
        ) : null}
      </div>

      {supported ? (
        <div className="card card-pad col gap-12">
          {!cat.published ? (
            <Banner tone="info" title={t("Not published yet")}>
              {t(
                "Nothing is sent to WhatsApp until you start. The first sync publishes every active product with a price; archived products and products without a price are left out.",
              )}
            </Banner>
          ) : null}
          <dl className="kv" data-testid="wa-catalog-counts">
            <dt>{t("Products that can be published")}</dt>
            <dd className="num">{cat.publishable}</dd>
            <dt>{t("On WhatsApp")}</dt>
            <dd className="num">{c.synced ?? 0}</dd>
            <dt>{t("Waiting to publish")}</dt>
            <dd className="num">{pending}</dd>
            <dt>{t("Hidden on WhatsApp")}</dt>
            <dd className="num">{c.hidden ?? 0}</dd>
            <dt>{t("Could not publish")}</dt>
            <dd className="num">{(c.failed ?? 0) + (c.remote_missing ?? 0)}</dd>
            <dt>{t("Left out (no price)")}</dt>
            <dd className="num">{cat.not_publishable.no_price ?? 0}</dd>
            <dt>{t("Last published")}</dt>
            <dd>{cat.last_synced_at ? formatDateTime(cat.last_synced_at) : "—"}</dd>
          </dl>
          {cat.failures.length ? (
            <div className="col gap-8" data-testid="wa-catalog-failures">
              <h4>{t("Could not publish")}</h4>
              {cat.failures.map((f) => (
                <div key={f.product_id} className="row">
                  <span className="grow ellipsis">
                    <strong>{f.name ?? f.product_id}</strong> · {itemStatusText(f.status as WaCatalogItemStatus)}
                    {f.error ? ` · ${tb(f.error)}` : ""}
                  </span>
                  {canManage ? (
                    <Button size="sm" onClick={() => run("retry", () => api.whatsapp.catalogRetry(f.product_id))}>
                      {t("Retry")}
                    </Button>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
          {canManage ? (
            <>
              <Checkbox
                label={t("Keep the WhatsApp catalogue synchronised automatically")}
                checked={cat.auto_sync}
                disabled={!!busy}
                onChange={(v) => run("auto", () => api.whatsapp.catalogConfigure(v), t("Settings saved"))}
              />
              <div className="row wrap">
                <Button variant="primary" loading={busy === "sync"} disabled={!!busy} onClick={() => setConfirm(true)}>
                  {cat.published ? t("Sync catalogue now") : t("Sync catalogue to WhatsApp")}
                </Button>
                {(c.failed ?? 0) + (c.remote_missing ?? 0) > 0 ? (
                  <Button
                    loading={busy === "retry"}
                    disabled={!!busy}
                    onClick={() => run("retry", () => api.whatsapp.catalogRetry())}
                  >
                    {t("Retry failed")}
                  </Button>
                ) : null}
              </div>
            </>
          ) : null}
        </div>
      ) : null}
      {error ? <Banner tone="danger">{tb(error)}</Banner> : null}
      {confirm ? (
        <Confirm
          title={cat.published ? t("Sync catalogue now") : t("Sync catalogue to WhatsApp")}
          confirmLabel={t("Start")}
          onCancel={() => setConfirm(false)}
          onConfirm={() => {
            setConfirm(false);
            void run("sync", () => api.whatsapp.catalogSync(), t("Catalogue sync started"));
          }}
        >
          {t(
            "{0} products will be published or updated in the WhatsApp catalogue of +{1}. Products you created directly in WhatsApp are not changed.",
            cat.publishable,
            cap.account ?? "",
          )}
        </Confirm>
      ) : null}
    </div>
  );
}

/** Product editor: one line about the product's WhatsApp catalogue copy. */
export function WaCatalogProductLine({ productId }: { productId: string }) {
  const on = useFeature("whatsapp.enabled");
  const { has } = useSession();
  const [st, setSt] = useState<WaCatalogProductState | null>(null);
  useEffect(() => {
    if (!on || !has("whatsapp.manage")) return;
    api.whatsapp
      .catalogProduct(productId)
      .then(setSt)
      .catch(() => undefined);
  }, [on, productId, has]);
  if (!st || !st.published) return null;
  return (
    <div className="tiny muted" data-testid="wa-catalog-product">
      {t("WhatsApp catalogue")}: {itemStatusText(st.status)}
      {st.last_error ? ` · ${tb(st.last_error)}` : ""}
    </div>
  );
}
