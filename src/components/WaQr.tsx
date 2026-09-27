import { useEffect, useState } from "react";
import { api } from "../api";
import type { WaStatus } from "../api/types";
import { t } from "../i18n";

const svgUrl = (svg: string) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

/** The WhatsApp pairing QR as an image (the same one the WhatsApp page shows). */
export function WaQr({ svg }: { svg: string }) {
  return (
    <div className="col gap-8" style={{ alignItems: "center" }}>
      <img src={svgUrl(svg)} alt={t("WhatsApp pairing QR code")} width={280} height={280} data-testid="wa-qr" />
      <div className="small" style={{ textAlign: "center" }}>
        {t("On the store phone open WhatsApp → Settings → Linked devices → Link a device, and scan this code.")}
      </div>
    </div>
  );
}

/**
 * Pairing after a confirmed "connect WhatsApp" proposal: shows the QR image
 * and refreshes it until the phone is linked. The code is shown here only; it
 * is never stored with the proposal or sent to the assistant.
 */
export function WaPairingPanel({ initial, poll = true }: { initial: WaStatus; poll?: boolean }) {
  const [wa, setWa] = useState<WaStatus>(initial);
  useEffect(() => {
    if (!poll) return;
    const id = window.setInterval(() => {
      void api.whatsapp.status().then(
        (r) => setWa(r.whatsapp),
        () => undefined,
      );
    }, 2000);
    return () => window.clearInterval(id);
  }, [poll]);
  if (wa.session === "paired" || wa.ready) {
    return (
      <div className="small" data-testid="wa-linked">
        {t("The phone is linked.")}
      </div>
    );
  }
  if (wa.qr?.svg) return <WaQr svg={wa.qr.svg} />;
  return <div className="small muted">{t("Waiting for the QR code…")}</div>;
}
