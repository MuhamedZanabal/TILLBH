// The product editor's picture card: preview, upload / replace, remove, and a
// one-time automatic search. On a new product the chosen file is held and sent
// with "Save"; on an existing product each action is saved straight away.
import { useEffect, useRef, useState } from "react";
import { ImagePlus, Search, Trash2 } from "lucide-react";
import { api } from "../../api";
import type { AutoImageStatus, ImageOverview, ImageSearchSettings, ProductImageState } from "../../api/types";
import { Banner, Button, Checkbox, Chip, Field, Skeleton, TextInput } from "../../components/ui";
import { ProductImage } from "../../components/ProductImage";
import { useFeature } from "../../components/FeatureGate";
import { useToast } from "../../components/toast";
import { t } from "../../i18n";

export const IMAGE_TYPES = ["image/png", "image/jpeg", "image/webp", "image/gif"];
export const IMAGE_MAX_BYTES = 8 * 1024 * 1024;

/** Reads a picked file as a data URL after the same checks the server makes first. */
export function readImageFile(file: File): Promise<string> {
  if (!IMAGE_TYPES.includes(file.type)) {
    return Promise.reject(new Error(t("Choose a PNG, JPEG, WebP or GIF picture.")));
  }
  if (file.size > IMAGE_MAX_BYTES) {
    return Promise.reject(new Error(t("The picture is larger than 8 MB.")));
  }
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(new Error(t("The picture could not be read.")));
    r.readAsDataURL(file);
  });
}

export function autoStatusLabel(s: AutoImageStatus): string {
  switch (s) {
    case "pending":
      return t("Waiting to search");
    case "processing":
      return t("Searching…");
    case "found":
      return t("Found automatically");
    case "not_found":
      return t("No suitable picture found");
    case "failed":
      return t("Automatic search failed");
    case "skipped":
      return t("Not searched (picture set by hand)");
    default:
      return t("Not searched");
  }
}

/** New product: holds the chosen picture until the product is saved. */
export function NewProductImageField({
  name,
  value,
  onChange,
}: {
  name: string;
  value: string | null;
  onChange: (dataUrl: string | null) => void;
}) {
  const auto = useFeature("catalog.auto_images");
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="card card-pad col gap-12" data-testid="product-image-field">
      <h3>{t("Picture")}</h3>
      <div className="pimg-edit">
        {value ? (
          <span className="pimg xl has" data-testid="product-image" data-state="image">
            <img src={value} alt={name || t("New product")} />
          </span>
        ) : (
          <ProductImage name={name || "?"} size="xl" />
        )}
      </div>
      <PickButton
        label={value ? t("Replace picture") : t("Upload picture")}
        onPick={async (f) => {
          setError(null);
          try {
            onChange(await readImageFile(f));
          } catch (e) {
            setError((e as Error).message);
          }
        }}
      />
      {value ? (
        <Button variant="ghost" icon={<Trash2 size={16} />} onClick={() => onChange(null)}>
          {t("Remove picture")}
        </Button>
      ) : null}
      {error ? <Banner tone="danger">{error}</Banner> : null}
      <div className="tiny muted">
        {value
          ? t("The picture is saved with the product.")
          : auto
            ? t("Without a picture, one is searched for once after saving.")
            : t("PNG, JPEG, WebP or GIF up to 8 MB. It is shown on a white background.")}
      </div>
    </div>
  );
}

/** Existing product: every action is saved at once. */
export function ProductImageField({ productId, name, canEdit }: { productId: string; name: string; canEdit: boolean }) {
  const auto = useFeature("catalog.auto_images");
  const toast = useToast();
  const [state, setState] = useState<ProductImageState | null>(null);
  const [busy, setBusy] = useState<null | "upload" | "remove" | "find">(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api.products
      .imageState(productId)
      .then((s) => live && setState(s))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [productId]);

  // While a search is queued or running, check back now and then (no image traffic).
  const searching = state?.auto_image_status === "pending" || state?.auto_image_status === "processing";
  useEffect(() => {
    if (!searching) return;
    let n = 0;
    const id = setInterval(() => {
      if (++n > 40) return clearInterval(id);
      api.products
        .imageState(productId)
        .then(setState)
        .catch(() => undefined);
    }, 3000);
    return () => clearInterval(id);
  }, [searching, productId]);

  const run = async (kind: "upload" | "remove" | "find", fn: () => Promise<ProductImageState>, ok: string) => {
    setBusy(kind);
    setError(null);
    try {
      setState(await fn());
      toast("success", ok);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  };

  if (!state) {
    return (
      <div className="card card-pad col gap-12" data-testid="product-image-field">
        <h3>{t("Picture")}</h3>
        {error ? <Banner tone="danger">{error}</Banner> : <ProductImage name={name} size="xl" hash={null} />}
      </div>
    );
  }
  const has = !!state.image_hash;
  return (
    <div className="card card-pad col gap-12" data-testid="product-image-field">
      <div className="row">
        <h3 className="grow">{t("Picture")}</h3>
        {has ? (
          <Chip tone={state.image_source === "manual" ? undefined : "brand"}>
            {state.image_source === "manual" ? t("Uploaded") : t("Found automatically")}
          </Chip>
        ) : null}
      </div>
      <div className="pimg-edit">
        <ProductImage hash={state.image_hash} name={name} size="xl" />
      </div>
      {canEdit ? (
        <>
          <PickButton
            label={has ? t("Replace picture") : t("Upload picture")}
            loading={busy === "upload"}
            disabled={!!busy}
            onPick={async (f) => {
              let data: string;
              try {
                data = await readImageFile(f);
              } catch (e) {
                setError((e as Error).message);
                return;
              }
              await run("upload", () => api.products.imageUpload(productId, data), t("Picture saved"));
            }}
          />
          {has ? (
            <Button
              variant="ghost"
              icon={<Trash2 size={16} />}
              loading={busy === "remove"}
              disabled={!!busy}
              onClick={() => run("remove", () => api.products.imageRemove(productId), t("Picture removed"))}
            >
              {t("Remove picture")}
            </Button>
          ) : auto ? (
            <Button
              icon={<Search size={16} />}
              loading={busy === "find" || searching}
              disabled={!!busy || searching}
              onClick={() => run("find", () => api.products.imageFind(productId), t("Search queued"))}
            >
              {t("Find picture automatically")}
            </Button>
          ) : null}
        </>
      ) : null}
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {!has || state.image_source === "automatic" ? (
        <div className="tiny muted" data-testid="auto-image-status">
          {autoStatusLabel(state.auto_image_status)}
        </div>
      ) : null}
    </div>
  );
}

function PickButton({
  label,
  onPick,
  loading,
  disabled,
}: {
  label: string;
  onPick: (f: File) => void | Promise<void>;
  loading?: boolean;
  disabled?: boolean;
}) {
  const ref = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={ref}
        type="file"
        accept={IMAGE_TYPES.join(",")}
        hidden
        data-testid="product-image-input"
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) void onPick(f);
        }}
      />
      <Button icon={<ImagePlus size={16} />} loading={loading} disabled={disabled} onClick={() => ref.current?.click()}>
        {label}
      </Button>
    </>
  );
}

/** Settings → Product images: sources, market, counts and the one-off backfill. */
export function ProductImageSettings({ canManage }: { canManage: boolean }) {
  const toast = useToast();
  const auto = useFeature("catalog.auto_images");
  const [ov, setOv] = useState<ImageOverview | null>(null);
  const [cfg, setCfg] = useState<ImageSearchSettings | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<null | "save" | "backfill" | "clear">(null);
  const [error, setError] = useState<string | null>(null);
  const load = () =>
    api.products
      .imageOverview()
      .then((o) => (setOv(o), setCfg(o.settings)))
      .catch((e: Error) => setError(e.message));
  useEffect(() => {
    void load();
  }, []);
  const run = async (kind: "save" | "backfill" | "clear", fn: () => Promise<unknown>) => {
    setBusy(kind);
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  if (!ov || !cfg) return error ? <Banner tone="danger">{error}</Banner> : <Skeleton />;
  const c = ov.counts;
  const waiting = (c.pending ?? 0) + (c.processing ?? 0);
  return (
    <div className="card card-pad col gap-16" data-testid="image-settings">
      <div>
        <h3>{t("Product images")}</h3>
        <div className="tiny">
          {t(
            "Pictures are stored on this computer and shown only from there. An automatic search runs at most once per product, for new products without a picture, and never while the till is loading a screen.",
          )}
        </div>
      </div>
      {!auto ? (
        <Banner tone="info">
          {t("Automatic search is off. Switch on “Find product images automatically” in Settings → Features.")}
        </Banner>
      ) : null}
      <dl className="kv" data-testid="image-counts">
        <dt>{t("With a picture")}</dt>
        <dd className="num">{ov.with_image}</dd>
        <dt>{t("Never searched")}</dt>
        <dd className="num">{ov.never_searched}</dd>
        <dt>{t("Waiting or searching")}</dt>
        <dd className="num">{waiting}</dd>
        <dt>{t("No suitable picture found")}</dt>
        <dd className="num">{c.not_found ?? 0}</dd>
        <dt>{t("Automatic search failed")}</dt>
        <dd className="num">{c.failed ?? 0}</dd>
      </dl>
      <div className="col gap-8">
        <h4>{t("Where to look")}</h4>
        <Checkbox
          label={t("Open Food Facts (by barcode)")}
          checked={cfg.open_food_facts}
          disabled={!canManage}
          onChange={(v) => setCfg({ ...cfg, open_food_facts: v })}
        />
        <Checkbox
          label={t("Bing image search (no key needed)")}
          checked={cfg.bing}
          disabled={!canManage}
          onChange={(v) => setCfg({ ...cfg, bing: v })}
        />
        <Checkbox
          label={t("Google Programmable Search (needs a key)")}
          checked={cfg.google}
          disabled={!canManage}
          onChange={(v) => setCfg({ ...cfg, google: v })}
        />
        {cfg.google ? (
          <div className="form-grid">
            <TextInput
              label={t("Search engine id (cx)")}
              value={cfg.google_cx}
              disabled={!canManage}
              onChange={(e) => setCfg({ ...cfg, google_cx: e.target.value })}
            />
            <TextInput
              label={t("API key")}
              type="password"
              autoComplete="off"
              value={key}
              disabled={!canManage}
              placeholder={ov.google_key_set ? t("Saved — type to replace") : ""}
              hint={t("Kept in this computer's secure store. It is never shown again.")}
              onChange={(e) => setKey(e.target.value)}
            />
          </div>
        ) : null}
      </div>
      <div className="form-grid">
        <TextInput
          label={t("Market (country code)")}
          value={cfg.region}
          maxLength={2}
          disabled={!canManage}
          hint={t("bh for Bahrain; results from this market are preferred.")}
          onChange={(e) => setCfg({ ...cfg, region: e.target.value.toLowerCase() })}
        />
        <Field label={t("Search language")}>
          <select
            className="select"
            value={cfg.language}
            disabled={!canManage}
            onChange={(e) => setCfg({ ...cfg, language: e.target.value as "en" | "ar" })}
          >
            <option value="en">English</option>
            <option value="ar">العربية</option>
          </select>
        </Field>
      </div>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {canManage ? (
        <div className="row wrap">
          <Button
            variant="primary"
            loading={busy === "save"}
            disabled={!!busy}
            onClick={() =>
              run("save", async () => {
                await api.products.imageConfigure(cfg, key.trim() ? key.trim() : undefined);
                setKey("");
                toast("success", t("Settings saved"));
              })
            }
          >
            {t("Save")}
          </Button>
          {ov.google_key_set ? (
            <Button
              variant="ghost"
              loading={busy === "clear"}
              disabled={!!busy}
              onClick={() => run("clear", () => api.products.imageConfigure(cfg, ""))}
            >
              {t("Remove API key")}
            </Button>
          ) : null}
          <span className="grow" />
          <Button
            icon={<Search size={16} />}
            loading={busy === "backfill"}
            disabled={!!busy || !auto || ov.never_searched === 0}
            onClick={() =>
              run("backfill", async () => {
                const r = await api.products.imageBackfill(200);
                toast("success", t("{0} products queued for a picture search", r.queued));
              })
            }
          >
            {t("Find pictures for products without one")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
