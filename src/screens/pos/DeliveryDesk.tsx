import { useEffect, useState } from "react";
import { LogOut, RefreshCw } from "lucide-react";
import { api } from "../../api";
import type { DeliveryRow } from "../../api/types";
import { useSession } from "../../state/session";
import { explain } from "../../lib/errors";
import { formatMoney } from "../../lib/money";
import { formatShort } from "../../lib/time";
import { Banner, Button, Chip, Empty } from "../../components/ui";
import { Logo } from "../../components/Logo";
import { t } from "../../i18n";
import { codeLabel } from "../../i18n/codes";
import { useFeature } from "../../components/FeatureGate";
import { OrdersList } from "../orders";

/** Minimal workspace for delivery staff: assigned deliveries and status only. */
export function DeliveryDesk() {
  const { session, logout, has } = useSession();
  const ordersOn = useFeature("orders.digital") && has("orders.manage");
  const [tab, setTab] = useState<"deliveries" | "orders">("deliveries");
  const [rows, setRows] = useState<DeliveryRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = () =>
    api.deliveries
      .list()
      .then(setRows)
      .catch((e) => setError(explain(e).message));
  useEffect(() => {
    void load();
    // New assignments arrive without the rider having to reload.
    const id = setInterval(() => void load(), 30000);
    return () => clearInterval(id);
  }, []);
  const next: Record<string, string> = { pending: "preparing", preparing: "dispatched", dispatched: "delivered" };
  const nextLabel: Record<string, string> = {
    preparing: t("Start preparing"),
    dispatched: t("Send with rider"),
    delivered: t("Mark delivered"),
  };
  return (
    <div className="pos-root">
      <header className="pos-header">
        <div className="brand">
          <Logo size={28} /> {t("TILLBH · Deliveries")}
        </div>
        <div className="grow" />
        <div className="hitem">{session?.display_name}</div>
        <Button size="sm" icon={<RefreshCw size={15} />} onClick={() => void load()}>
          {t("Refresh")}
        </Button>
        <Button size="sm" icon={<LogOut size={15} />} onClick={() => void logout()}>
          {t("Logout")}
        </Button>
      </header>
      <div className="content">
        {ordersOn ? (
          <div className="row" style={{ marginBottom: 12 }}>
            <button
              className={`filter-chip ${tab === "deliveries" ? "active" : ""}`}
              onClick={() => setTab("deliveries")}
            >
              {t("My deliveries")}
            </button>
            <button className={`filter-chip ${tab === "orders" ? "active" : ""}`} onClick={() => setTab("orders")}>
              {t("New orders")}
            </button>
          </div>
        ) : null}
        {tab === "orders" && ordersOn ? <OrdersList /> : null}
        {tab === "deliveries" && error ? <Banner tone="danger">{error}</Banner> : null}
        {tab !== "deliveries" ? null : rows.length === 0 ? (
          <Empty title={t("No deliveries right now")}>
            {t("Assigned deliveries will appear here automatically.")}
          </Empty>
        ) : null}
        <div className="col gap-12" hidden={tab !== "deliveries"}>
          {rows.map((d) => {
            const digits = (d.phone ?? "").replace(/\D/g, "");
            const status =
              d.status === "pending"
                ? t("New")
                : d.status === "preparing"
                  ? t("Preparing")
                  : d.status === "dispatched"
                    ? t("Out for delivery")
                    : codeLabel(d.status);
            const payment =
              d.payment_status === "cod"
                ? t("Collect {0}", formatMoney(d.amount_minor))
                : d.payment_status === "paid"
                  ? t("Paid")
                  : t("Payment pending");
            return (
              <div key={d.delivery_id} className="card card-pad delivery-card">
                <div className="delivery-card-top">
                  <div className="grow">
                    <div className="strong" dir="auto">
                      {d.customer_name ?? t("Customer")}
                    </div>
                    <div className="tiny muted">
                      {d.delivery_number} · {formatShort(d.created_at)}
                    </div>
                  </div>
                  <span className="money strong">{formatMoney(d.amount_minor)}</span>
                </div>
                <div className="delivery-address" dir="auto">
                  {[d.area, d.address].filter(Boolean).join(" · ") || t("No delivery address")}
                </div>
                <div className="delivery-card-actions">
                  <div className="row gap-8 wrap">
                    <Chip tone="info">{status}</Chip>
                    <Chip tone={d.payment_status === "paid" ? "success" : "warning"}>{payment}</Chip>
                  </div>
                  <span className="grow" />
                  {d.phone ? (
                    <a className="btn" href={`tel:${d.phone}`}>
                      {t("Call")}
                    </a>
                  ) : null}
                  {digits ? (
                    <a className="btn" href={`https://wa.me/${digits}`} target="_blank" rel="noreferrer">
                      {t("WhatsApp")}
                    </a>
                  ) : null}
                  {next[d.status] ? (
                    <Button
                      variant="primary"
                      size="lg"
                      onClick={async () => {
                        try {
                          await api.deliveries.update({ delivery_id: d.delivery_id, status: next[d.status] });
                          void load();
                        } catch (e) {
                          setError(explain(e).message);
                        }
                      }}
                    >
                      {nextLabel[next[d.status]] ?? t("Update delivery")}
                    </Button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
