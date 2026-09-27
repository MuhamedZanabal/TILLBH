import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { WaPairingPanel } from "../WaQr";
import type { WaStatus } from "../../api/types";

const status = (over: Partial<WaStatus>): WaStatus => ({
  enabled: true,
  process: "running",
  session: "pairing",
  connected: false,
  ready: false,
  account: null,
  qr: null,
  pair_code: null,
  last_error: null,
  restarts: 0,
  next_retry_at: null,
  banned_until: null,
  adapter: "rust",
  session_file: "",
  inbox_rev: 0,
  last_send_at: null,
  last_send_error: null,
  ...over,
});

describe("WhatsApp connect proposal: Confirm card", () => {
  it("shows the pairing QR as an image, like the WhatsApp page", () => {
    render(
      <WaPairingPanel
        initial={status({ qr: { svg: "<svg xmlns='http://www.w3.org/2000/svg'/>", expires_at: "" } })}
        poll={false}
      />,
    );
    const img = screen.getByTestId("wa-qr") as HTMLImageElement;
    expect(img.tagName).toBe("IMG");
    expect(img.src.startsWith("data:image/svg+xml")).toBe(true);
    // The QR is never printed as raw text.
    expect(document.body.textContent).not.toContain("<svg");
  });

  it("says the phone is linked once pairing finishes", () => {
    render(<WaPairingPanel initial={status({ session: "paired", ready: true })} poll={false} />);
    expect(screen.getByTestId("wa-linked")).toBeTruthy();
    expect(screen.queryByTestId("wa-qr")).toBeNull();
  });
});
