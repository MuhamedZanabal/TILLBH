// One product picture everywhere: the stored image (by content hash), a calm
// monogram placeholder when there is none, a shimmer while it loads, and the
// placeholder again if the image cannot be shown. Images come only from the
// POS's own store (`products.images`), never from an outside URL, and are
// requested in small batches and cached by hash (a hash never changes).
import { useEffect, useState } from "react";
import { api } from "../api";

const cache = new Map<string, string>();
const missing = new Set<string>();
const waiters = new Map<string, ((src: string | null) => void)[]>();
const queue: string[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;
const MAX_CACHED = 800;
const BATCH = 60;

function remember(hash: string, src: string) {
  if (cache.size >= MAX_CACHED) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(hash, src);
}

function settle(hash: string, src: string | null) {
  (waiters.get(hash) ?? []).forEach((w) => w(src));
  waiters.delete(hash);
}

async function flush() {
  timer = null;
  const batch = queue.splice(0, BATCH);
  if (queue.length) timer = setTimeout(() => void flush(), 0);
  if (!batch.length) return;
  try {
    const got = await api.products.images(batch);
    for (const h of batch) {
      const src = got[h] ?? null;
      if (src) remember(h, src);
      else missing.add(h);
      settle(h, src);
    }
  } catch {
    // Offline or not signed in: show the placeholder; try again on next mount.
    batch.forEach((h) => settle(h, null));
  }
}

/** The stored image for `hash` (null while loading or when there is none). */
export function loadProductImage(hash: string): Promise<string | null> {
  const hit = cache.get(hash);
  if (hit) return Promise.resolve(hit);
  if (missing.has(hash)) return Promise.resolve(null);
  return new Promise((resolve) => {
    const list = waiters.get(hash);
    if (list) {
      list.push(resolve);
      return;
    }
    waiters.set(hash, [resolve]);
    queue.push(hash);
    timer ??= setTimeout(() => void flush(), 16);
  });
}

/** Seed the cache with an image the page already has (e.g. just uploaded). */
export function primeProductImage(hash: string, dataUrl: string) {
  missing.delete(hash);
  remember(hash, dataUrl);
}

/** A calm, stable colour per product name (hue 0–359). */
export function productHue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

/** Two letters for the placeholder (first letters of the first two words). */
export function productMonogram(name: string): string {
  const w = name.trim().split(/\s+/).filter(Boolean);
  return ((w[0]?.[0] ?? "") + (w[1]?.[0] ?? w[0]?.[1] ?? "")).toUpperCase();
}

export type ProductImageSize = "xs" | "sm" | "md" | "lg" | "xl";

export function ProductImage({
  hash,
  name,
  size = "sm",
  className = "",
}: {
  hash?: string | null;
  name: string;
  size?: ProductImageSize;
  className?: string;
}) {
  const [src, setSrc] = useState<string | null>(() => (hash ? (cache.get(hash) ?? null) : null));
  const [loading, setLoading] = useState(() => !!hash && !cache.has(hash) && !missing.has(hash));
  const [broken, setBroken] = useState(false);
  useEffect(() => {
    setBroken(false);
    if (!hash) {
      setSrc(null);
      setLoading(false);
      return;
    }
    const hit = cache.get(hash);
    if (hit) {
      setSrc(hit);
      setLoading(false);
      return;
    }
    let live = true;
    setSrc(null);
    setLoading(!missing.has(hash));
    void loadProductImage(hash).then((s) => {
      if (!live) return;
      setSrc(s);
      setLoading(false);
    });
    return () => {
      live = false;
    };
  }, [hash]);
  const show = src && !broken;
  return (
    <span
      className={`pimg ${size} ${show ? "has" : loading ? "loading" : "empty"} ${className}`}
      style={show ? undefined : { ["--h" as string]: productHue(name) }}
      data-testid="product-image"
      data-state={show ? "image" : loading ? "loading" : "placeholder"}
    >
      {show ? (
        <img src={src} alt={name} loading="lazy" decoding="async" draggable={false} onError={() => setBroken(true)} />
      ) : loading ? null : (
        <span className="pimg-mono" aria-hidden>
          {productMonogram(name)}
        </span>
      )}
    </span>
  );
}
