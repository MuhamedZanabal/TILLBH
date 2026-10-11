// Adds upgrade-specific state through the installed predecessor's own backend.
// Run immediately after installer-smoke.mjs first while its authenticated owner
// session and shift are still active.
import { chromium } from "@playwright/test";

async function appPage() {
  let browser;
  for (let i = 0; i < 30 && !browser; i += 1) {
    try {
      browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  if (!browser) throw new Error("predecessor WebView2 debugging endpoint is unavailable");
  const page = browser.contexts()[0]?.pages()[0];
  if (!page) throw new Error("predecessor WebView2 page was not found");
  return page;
}

async function rpc(page, cmd, args = {}) {
  return page.evaluate(
    async ([rpcCmd, rpcArgs]) => {
      const token = sessionStorage.getItem("amwapos.session");
      if (!token) throw new Error("predecessor authenticated session is missing");
      return window.__TAURI_INTERNALS__.invoke("rpc", { cmd: rpcCmd, token, args: rpcArgs });
    },
    [cmd, args],
  );
}

const page = await appPage();
const found = await rpc(page, "products.search", { q: "Smoke Juice", limit: 20, offset: 0 });
const rows = found.rows ?? found;
if (rows.length !== 1) throw new Error(`expected one smoke product, got ${rows.length}`);
const productId = rows[0].product_id;

await rpc(page, "barcodes.add", {
  product_id: productId,
  barcode: "6299999000018",
  make_primary: false,
});
await rpc(page, "products.price_update", {
  product_id: productId,
  amount_minor: 1375,
  reason: "upgrade-compatibility-marker",
});
await rpc(page, "inventory.adjust", {
  product_id: productId,
  mode: "increase",
  qty_milli: 2000,
  reason: "upgrade compatibility seed",
  operation_id: "upgrade-seed-stock-v1",
});
await rpc(page, "customers.save", {
  customer_id: null,
  customer: {
    name: "Upgrade Customer",
    phone: "+97339000001",
    whatsapp: "+97339000001",
    area: "Amwaj",
  },
});
await rpc(page, "suppliers.save", {
  supplier_id: null,
  supplier: {
    name: "Upgrade Supplier",
    contact_name: "Upgrade Contact",
    phone: "+97339000002",
  },
});
const backup = await rpc(page, "settings.get", { key: "local.backup" });
await rpc(page, "settings.save", {
  key: "local.backup",
  value: { ...backup, automatic: true, interval_hours: 12, keep: 17 },
});

const product = await rpc(page, "products.get", { product_id: productId });
if ((product.barcodes ?? []).length < 2) throw new Error("second barcode was not persisted by predecessor backend");
console.log("upgrade compatibility seed: passed");
