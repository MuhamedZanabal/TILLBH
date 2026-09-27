import { useEffect, useMemo, useState } from "react";
import { MessageCircle, RefreshCw } from "lucide-react";
import { api } from "../../api";
import type { WaPhoneContact } from "../../api/types";
import { useFeature } from "../../components/FeatureGate";
import { useToast } from "../../components/toast";
import { Banner, Button, Modal, Skeleton } from "../../components/ui";
import { useSession } from "../../state/session";
import { relative } from "../../lib/time";
import { t } from "../../i18n";
import { useAction, useLoad } from "./common";

type Filter = "new" | "exists" | "all";

/**
 * Customers page: shown while WhatsApp is connected. Imports the contacts
 * saved on the shop's phone as customers (saved name → name, number → phone,
 * digits/hyphens/slashes in the name → address).
 */
export function WhatsAppContactsButton({ onImported }: { onImported: () => void }) {
  const on = useFeature("whatsapp.enabled");
  const { has } = useSession();
  const allowed = on && has("customers.manage") && has("whatsapp.manage");
  const [connected, setConnected] = useState(false);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!allowed) return;
    let alive = true;
    const poll = () =>
      api.whatsapp.status().then(
        (s) => alive && setConnected(!!s.whatsapp?.connected),
        () => undefined,
      );
    void poll();
    const id = setInterval(poll, 15000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [allowed]);
  if (!allowed || !connected) return null;
  return (
    <>
      <Button icon={<MessageCircle size={18} />} onClick={() => setOpen(true)} data-testid="wa-import-customers">
        {t("Import WhatsApp customers")}
      </Button>
      {open ? (
        <WhatsAppContactsSheet
          onClose={() => setOpen(false)}
          onImported={() => {
            setOpen(false);
            onImported();
          }}
        />
      ) : null}
    </>
  );
}

function WhatsAppContactsSheet({ onClose, onImported }: { onClose: () => void; onImported: () => void }) {
  const toast = useToast();
  const list = useLoad(() => api.whatsapp.phoneContacts(), []);
  const act = useAction();
  const [filter, setFilter] = useState<Filter>("new");
  const [q, setQ] = useState("");
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const [updateExisting, setUpdateExisting] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const rows = useMemo(() => list.data?.contacts ?? [], [list.data]);
  // Default selection: every contact that is not a customer yet.
  const selected = useMemo(
    () => picked ?? new Set(rows.filter((r) => r.status === "new").map((r) => r.jid)),
    [picked, rows],
  );
  const shown = rows
    .filter((r) => (filter === "all" ? true : r.status === filter))
    .filter((r) => {
      const s = q.trim().toLowerCase();
      return !s || (r.name ?? "").toLowerCase().includes(s) || (r.phone ?? "").includes(s);
    });
  const importable = (r: WaPhoneContact) => r.status === "new" || (r.status === "exists" && updateExisting);
  const chosen = rows.filter((r) => selected.has(r.jid) && importable(r));
  const toggle = (jid: string) => {
    const next = new Set(selected);
    if (next.has(jid)) next.delete(jid);
    else next.add(jid);
    setPicked(next);
  };

  const refresh = async () => {
    if (!(await act.run(() => api.whatsapp.phoneContactsRefresh()))) return;
    // WhatsApp sends the contacts in the background: reload for up to ~30 s.
    setWaiting(true);
    let last = -1;
    for (let i = 0; i < 15; i++) {
      await new Promise((r) => setTimeout(r, 2000));
      const d = await api.whatsapp.phoneContacts().catch(() => null);
      if (d) {
        list.setData(d);
        if (d.contacts.length && d.contacts.length === last) break;
        last = d.contacts.length;
      }
    }
    setWaiting(false);
  };

  const c = list.data?.counts;
  return (
    <Modal
      title={t("Import WhatsApp customers")}
      size="sheet"
      testId="wa-import-sheet"
      onClose={onClose}
      footer={
        <div className="col" style={{ width: "100%" }}>
          <button
            type="button"
            className={`toggle ${updateExisting ? "on" : ""}`}
            aria-pressed={updateExisting}
            onClick={() => setUpdateExisting(!updateExisting)}
          >
            {t("Also update existing customers with the phone's name (and fill an empty address)")}
          </button>
          <Button
            variant="primary"
            size="xl"
            block
            disabled={!chosen.length}
            loading={act.busy && !waiting}
            data-testid="wa-import-confirm"
            onClick={async () => {
              const r = await act.run(() =>
                api.whatsapp.phoneContactsImport(
                  chosen.map((x) => x.jid),
                  updateExisting,
                ),
              );
              if (r) {
                toast(
                  "success",
                  t("{0} customers added, {1} updated", r.created, r.updated),
                  r.skipped ? t("{0} skipped", r.skipped) : undefined,
                );
                onImported();
              }
            }}
          >
            {t("Import {0} contacts", chosen.length)}
          </Button>
        </div>
      }
    >
      <div className="col gap-16">
        <p className="small muted" style={{ margin: 0 }}>
          {t(
            "The name saved on the shop's phone becomes the customer name, the number becomes the phone and WhatsApp number, and any numbers with hyphens or slashes in the name (e.g. “825 - 3325 husband”) become the address. Customers already saved with the same number are not duplicated.",
          )}
        </p>
        <div className="row wrap">
          <Button
            icon={<RefreshCw size={18} />}
            loading={waiting}
            onClick={() => void refresh()}
            data-testid="wa-refresh"
          >
            {t("Refresh from phone")}
          </Button>
          <span className="tiny">
            {list.data?.last_sync_at
              ? t("Contacts received {0}", relative(list.data.last_sync_at))
              : t("No contacts received yet")}
          </span>
        </div>
        {act.error ? <Banner tone="danger">{act.error}</Banner> : null}
        {list.error ? <Banner tone="danger">{list.error}</Banner> : null}
        {!list.data ? (
          <Skeleton />
        ) : !rows.length ? (
          <Banner tone="info" title={t("No contacts yet")}>
            {t(
              "Press Refresh from phone. WhatsApp sends the phone's saved contacts in the background; they appear here within a minute.",
            )}
          </Banner>
        ) : (
          <>
            <div className="toggle-group" role="tablist">
              {(
                [
                  ["new", t("New ({0})", c?.new ?? 0)],
                  ["exists", t("Already customers ({0})", c?.exists ?? 0)],
                  ["all", t("All ({0})", rows.length)],
                ] as const
              ).map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  role="tab"
                  aria-selected={filter === k}
                  className={`toggle ${filter === k ? "on" : ""}`}
                  onClick={() => setFilter(k)}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="row">
              <input
                className="input grow"
                placeholder={t("Search name or number…")}
                aria-label={t("Search name or number…")}
                value={q}
                onChange={(e) => setQ(e.target.value)}
              />
              <Button onClick={() => setPicked(new Set([...selected, ...shown.filter(importable).map((r) => r.jid)]))}>
                {t("Select all")}
              </Button>
              <Button
                variant="ghost"
                onClick={() => {
                  const hide = new Set(shown.map((r) => r.jid));
                  setPicked(new Set([...selected].filter((j) => !hide.has(j))));
                }}
              >
                {t("None")}
              </Button>
            </div>
            <div className="wa-contacts" role="list">
              {shown.slice(0, 400).map((r) => {
                const can = importable(r);
                return (
                  <label key={r.jid} role="listitem" className={`wa-contact ${can ? "" : "off"}`}>
                    <input
                      type="checkbox"
                      checked={can && selected.has(r.jid)}
                      disabled={!can}
                      onChange={() => toggle(r.jid)}
                      aria-label={r.name ?? r.phone ?? r.jid}
                    />
                    <span className="wc-main">
                      <span className="wc-name ellipsis" dir="auto">
                        {r.name ?? t("(no name saved)")}
                      </span>
                      <span className="tiny">
                        <span dir="ltr">{r.phone ?? t("no number")}</span>
                        {r.address ? (
                          <>
                            {" · "}
                            {t("Address")}: <strong dir="auto">{r.address}</strong>
                          </>
                        ) : null}
                      </span>
                    </span>
                    {r.status === "exists" ? (
                      <span className="chip">{t("Customer: {0}", r.customer_name ?? "")}</span>
                    ) : r.status === "new" ? (
                      <span className="chip brand">{t("New")}</span>
                    ) : (
                      <span className="chip">{r.status === "no_phone" ? t("No number") : t("No name")}</span>
                    )}
                  </label>
                );
              })}
              {shown.length > 400 ? (
                <div className="tiny" style={{ padding: 12 }}>
                  {t("Showing 400 of {0}. Search to narrow the list; Select all includes every match.", shown.length)}
                </div>
              ) : null}
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
