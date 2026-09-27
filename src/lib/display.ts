import { isDesktop } from "../api/transport";

/** Display sizes offered in Settings → Appearance (percent). */
export const DISPLAY_SCALES = [90, 100, 110, 125, 150] as const;

export function scaleFactor(v: string | number | null | undefined): number {
  const n = Number(v) || 100;
  return Math.min(150, Math.max(90, n)) / 100;
}

/**
 * Zoom the whole app (text, buttons, spacing). On the desktop this is the
 * WebView's own zoom, so layout reflows exactly as at a smaller screen; in a
 * browser (dev server, tests) CSS zoom on the root stands in.
 */
export async function applyDisplayScale(v: string | number | null | undefined): Promise<void> {
  const f = scaleFactor(v);
  const root = document.documentElement;
  root.dataset.scale = String(Math.round(f * 100));
  if (isDesktop()) {
    try {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(f);
      root.style.removeProperty("zoom");
      return;
    } catch {
      // Older runtime without the zoom permission: fall back to CSS zoom.
    }
  }
  if (f === 1) root.style.removeProperty("zoom");
  else root.style.setProperty("zoom", String(f));
}
