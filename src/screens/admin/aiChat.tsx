import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import {
  Bot,
  Brain,
  CalendarClock,
  Inbox,
  Keyboard,
  Paperclip,
  Pencil,
  Pin,
  Plus,
  Send,
  Wrench,
  X,
} from "lucide-react";
import { api } from "../../api";
import type {
  AiBriefing,
  AiContext,
  AiConversation,
  AiPin,
  AiPlaybookResult,
  AiSlashResult,
  AiStatus,
  AiStreamEvent,
} from "../../api/types";
import { useSession } from "../../state/session";
import { useToast } from "../../components/toast";
import { FeatureGate } from "../../components/FeatureGate";
import { Banner, Button, Checkbox, Chip, Field, Modal, PageHeader, Skeleton, TextInput } from "../../components/ui";
import { Confirm, useAction, useLoad } from "./common";
import { formatMoney, formatQty } from "../../lib/money";
import { formatDateTime, relative } from "../../lib/time";
import { getLang, t, tb } from "../../i18n";
import { ActionInbox, Linkified, PLAYBOOKS, PlaybookResult, ProposalCard, TOOL_LABEL, providerLabel } from "./ai";
import { fileToBase64 } from "./automation";

// ---------------------------------------------------------------- live view (C8)

interface LiveStep {
  id: string;
  name: string;
  input: unknown;
  result?: string;
  is_error?: boolean;
  truncated?: boolean;
}
interface Live {
  text: string;
  thinking: string;
  steps: LiveStep[];
  round: number;
  provider: string;
  model: string;
  fallback: { from: string; to: string; reason: string } | null;
  nudged: boolean;
  unverified: boolean;
  tokensIn: number;
  tokensOut: number;
}
const emptyLive = (): Live => ({
  text: "",
  thinking: "",
  steps: [],
  round: 0,
  provider: "",
  model: "",
  fallback: null,
  nudged: false,
  unverified: false,
  tokensIn: 0,
  tokensOut: 0,
});

function applyEvents(l: Live, events: AiStreamEvent[]): Live {
  const next: Live = { ...l, steps: [...l.steps] };
  for (const e of events) {
    switch (e.type) {
      case "start":
        next.provider = e.provider;
        next.model = e.model;
        break;
      case "round":
        next.round = e.n;
        // A new round starts a new answer; keep what was said so far visible.
        if (next.text && !next.text.endsWith("\n\n")) next.text += "\n\n";
        break;
      case "text":
        next.text += e.delta;
        break;
      case "thinking":
        next.thinking += e.delta;
        break;
      case "tool_call":
        next.steps.push({ id: e.id, name: e.name, input: e.input });
        break;
      case "tool_result": {
        const i = next.steps.findIndex((s) => s.id === e.id);
        if (i >= 0)
          next.steps[i] = { ...next.steps[i], result: e.content, is_error: e.is_error, truncated: e.truncated };
        break;
      }
      case "usage":
        next.tokensIn += e.input_tokens;
        next.tokensOut += e.output_tokens;
        break;
      case "fallback":
        next.fallback = { from: e.from, to: e.to, reason: e.reason };
        break;
      case "nudge":
        next.nudged = true;
        break;
      case "unverified":
        next.unverified = true;
        break;
      default:
        break;
    }
  }
  return next;
}

const toolLabel = (name: string) => TOOL_LABEL[name]?.() ?? name;

function pretty(v: unknown): string {
  if (typeof v === "string") {
    try {
      return JSON.stringify(JSON.parse(v), null, 1);
    } catch {
      return v;
    }
  }
  return JSON.stringify(v, null, 1);
}

/** One tool call: name, input and result, all visible. */
function ToolStep({
  name,
  input,
  result,
  isError,
  running,
}: {
  name: string;
  input: unknown;
  result?: string;
  isError?: boolean;
  running?: boolean;
}) {
  return (
    <details className="ai-step" data-testid="ai-tool-step">
      <summary className="row" style={{ gap: 6 }}>
        <Wrench size={14} />
        <span className="small">{toolLabel(name)}</span>
        <code dir="ltr" className="tiny">
          {name}
        </code>
        {running ? (
          <Chip>{t("Running…")}</Chip>
        ) : isError ? (
          <Chip tone="danger">{t("Error")}</Chip>
        ) : (
          <Chip tone="success">{t("Done")}</Chip>
        )}
      </summary>
      <div className="col gap-4" style={{ marginTop: 6 }}>
        <div className="tiny muted">{t("Input")}</div>
        <pre className="tiny" dir="ltr" style={{ whiteSpace: "pre-wrap", maxHeight: 160, overflow: "auto" }}>
          {pretty(input)}
        </pre>
        {result !== undefined ? (
          <>
            <div className="tiny muted">{t("Result the assistant received")}</div>
            <pre className="tiny" dir="ltr" style={{ whiteSpace: "pre-wrap", maxHeight: 260, overflow: "auto" }}>
              {pretty(result)}
            </pre>
          </>
        ) : null}
      </div>
    </details>
  );
}

function Thinking({ text, open }: { text: string; open?: boolean }) {
  if (!text.trim()) return null;
  return (
    <details open={open} data-testid="ai-thinking">
      <summary className="row small muted" style={{ gap: 6 }}>
        <Brain size={14} /> {t("Thinking")}
      </summary>
      <div className="small muted" style={{ whiteSpace: "pre-wrap", marginTop: 4 }}>
        {text}
      </div>
    </details>
  );
}

function LiveView({ live }: { live: Live }) {
  return (
    <div className="bubble in col gap-8" style={{ alignSelf: "flex-start", maxWidth: "95%" }} data-testid="ai-live">
      <div className="row tiny" style={{ gap: 6 }}>
        <span className="spinner" aria-hidden />
        <span>
          {live.provider
            ? `${providerLabel(live.provider as AiStatus["active_provider"])} · ${live.model}`
            : t("Starting…")}
        </span>
        {live.round ? <span>· {t("Step {0}", live.round)}</span> : null}
        {live.tokensIn || live.tokensOut ? (
          <span>· {t("{0} in / {1} out tokens", live.tokensIn, live.tokensOut)}</span>
        ) : null}
      </div>
      {live.fallback ? (
        <Banner tone="warning" title={t("Switched to the free fallback")}>
          <span dir="ltr">
            {live.fallback.from} → {live.fallback.to}
          </span>{" "}
          · {tb(live.fallback.reason)}
        </Banner>
      ) : null}
      <Thinking text={live.thinking} open />
      {live.steps.map((s) => (
        <ToolStep
          key={s.id}
          name={s.name}
          input={s.input}
          result={s.result}
          isError={s.is_error}
          running={s.result === undefined}
        />
      ))}
      {live.nudged ? (
        <div className="tiny">{t("AMWAPOS asked the assistant to back its figures with a tool.")}</div>
      ) : null}
      {live.text ? <div style={{ whiteSpace: "pre-wrap" }}>{live.text}</div> : null}
    </div>
  );
}

// ---------------------------------------------------------------- stored messages

function Attachment({ id }: { id: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let on = true;
    api.ai.attachment(id).then(
      (a) => on && setSrc(`data:${a.media_type};base64,${a.data}`),
      () => undefined,
    );
    return () => {
      on = false;
    };
  }, [id]);
  return src ? (
    <img src={src} alt={t("Attached photo")} style={{ maxWidth: 180, maxHeight: 140, borderRadius: 6 }} />
  ) : (
    <Chip>{t("Photo")}</Chip>
  );
}

type Message = AiConversation["messages"][number];

function MessageView({ m }: { m: Message }) {
  if (m.kind === "nudge") {
    return <div className="tiny muted">{t("AMWAPOS asked the assistant to back its figures with a tool.")}</div>;
  }
  const mine = m.role === "user";
  return (
    <div
      className={`bubble ${mine ? "out" : "in"} col gap-8`}
      style={{ alignSelf: mine ? "flex-end" : "flex-start", maxWidth: "90%" }}
    >
      {m.thinking ? <Thinking text={m.thinking} /> : null}
      {(m.calls ?? []).map((c) => (
        <ToolStep key={c.id} name={c.name} input={c.input} result={c.result} isError={c.is_error} />
      ))}
      {m.attachments?.length ? (
        <div className="row wrap">
          {m.attachments.map((a) => (
            <Attachment key={a.attachment_id} id={a.attachment_id} />
          ))}
        </div>
      ) : null}
      {m.has_context ? <Chip>{t("Till cart attached")}</Chip> : null}
      {m.text ? <Linkified text={m.text} /> : null}
      {m.role === "assistant" && m.text && m.evidence?.length ? (
        <div className="row wrap" style={{ gap: 4 }} data-testid="ai-evidence">
          <span className="tiny">{t("Evidence")}:</span>
          {m.evidence.slice(0, 8).map((ev, j) => (
            <Chip key={j}>
              <span dir="ltr">
                {ev.tool}
                {ev.ids.length ? ` · ${ev.ids.join(", ")}` : ""}
              </span>
            </Chip>
          ))}
        </div>
      ) : null}
      {m.unverified ? (
        <div data-testid="ai-unverified">
          <Chip tone="warning">{t("Unverified: no tool result backs these figures")}</Chip>
        </div>
      ) : null}
      {m.stop_reason === "refusal" ? (
        <div className="tiny">{t("The provider declined to answer this request.")}</div>
      ) : null}
      {m.stop_reason === "max_tokens" ? <div className="tiny">{t("The answer was cut short.")}</div> : null}
      <div className="tiny">{formatDateTime(m.at)}</div>
    </div>
  );
}

// ---------------------------------------------------------------- slash commands (F1)

interface SlashDef {
  name: string;
  args?: string;
  read?: boolean;
  desc: () => string;
}

export const SLASH: SlashDef[] = [
  { name: "help", desc: () => t("List every command") },
  { name: "new", desc: () => t("Start a new conversation") },
  { name: "rename", args: "<name>", desc: () => t("Name this conversation") },
  {
    name: "pin",
    args: "<kind> <name or id>",
    desc: () => t("Pin a product, customer, supplier, order, shift, PO, sale or delivery"),
  },
  { name: "unpin", args: "<kind> <id>", desc: () => t("Remove a pinned record") },
  { name: "price", args: "<product> <amount>", desc: () => t("Propose a new price (you confirm it)") },
  {
    name: "explain",
    args: "<command> [text]",
    desc: () => t("Let the assistant look into a command's data and explain it"),
  },
  { name: "ask", args: "<question>", desc: () => t("Ask the assistant (same as typing)") },
  { name: "attach", desc: () => t("Attach a photo, invoice or payment screenshot") },
  { name: "inbox", desc: () => t("Open the action inbox") },
  { name: "briefings", desc: () => t("Scheduled briefings and notes") },
  { name: "goto", args: "<page>", desc: () => t("Open an admin page") },
  { name: "model", desc: () => t("Show the provider, model and today's tokens") },
  { name: "shortcuts", desc: () => t("Keyboard shortcuts") },
  { name: "clear", desc: () => t("Clear command results") },
  { name: "kpi", read: true, desc: () => t("Today's dashboard figures") },
  { name: "stock", args: "<product>", read: true, desc: () => t("Find products and their stock") },
  { name: "low", read: true, desc: () => t("Products low on stock") },
  { name: "sale", args: "<receipt>", read: true, desc: () => t("Find a sale by receipt number") },
  { name: "refund", args: "<receipt>", read: true, desc: () => t("What can be refunded on a receipt") },
  { name: "customer", args: "<name or phone>", read: true, desc: () => t("Find customers") },
  { name: "supplier", args: "[name]", read: true, desc: () => t("Suppliers") },
  { name: "po", args: "[status]", read: true, desc: () => t("Purchase orders") },
  { name: "shift", read: true, desc: () => t("The open shift") },
  { name: "shifts", read: true, desc: () => t("Today's shifts") },
  { name: "cash", read: true, desc: () => t("Today's cash in / out / drops") },
  { name: "deliveries", read: true, desc: () => t("Open deliveries") },
  { name: "orders", read: true, desc: () => t("Digital orders") },
  { name: "transfers", read: true, desc: () => t("Stock transfers") },
  { name: "stocktakes", read: true, desc: () => t("Stocktakes") },
  { name: "ghost", read: true, desc: () => t("Unknown barcodes scanned at the till") },
  { name: "payments", read: true, desc: () => t("Payment screenshots to review") },
  { name: "invoices", read: true, desc: () => t("Invoice scans") },
  { name: "triage", read: true, desc: () => t("Sorted WhatsApp messages") },
  { name: "audit", args: "[event]", read: true, desc: () => t("Audit log") },
  { name: "backup", read: true, desc: () => t("Backup health") },
  { name: "diagnostics", read: true, desc: () => t("Health checks") },
  { name: "users", read: true, desc: () => t("Staff accounts") },
  { name: "devices", read: true, desc: () => t("Tills and hub") },
  { name: "sync", read: true, desc: () => t("Sync status") },
  { name: "categories", read: true, desc: () => t("Product categories") },
  { name: "notes", read: true, desc: () => t("Briefing notes") },
  { name: "eod", read: true, desc: () => t("End-of-day playbook") },
  { name: "cash_short", read: true, desc: () => t("Cash-short playbook") },
  { name: "reorder", read: true, desc: () => t("Reorder playbook") },
  { name: "refund_spike", read: true, desc: () => t("Refund-spike playbook") },
];

const PAGES = [
  "dashboard",
  "sales",
  "refunds",
  "shifts",
  "products",
  "inventory",
  "suppliers",
  "purchase-orders",
  "customers",
  "deliveries",
  "orders",
  "reports",
  "end-of-day",
  "whatsapp",
  "payment-reviews",
  "invoice-scan",
  "users",
  "settings",
  "backups",
  "audit",
  "diagnostics",
  "sync",
];

const PIN_KINDS: AiPin["kind"][] = ["product", "customer", "supplier", "order", "shift", "po", "sale", "delivery"];
const PIN_LABEL: Record<AiPin["kind"], () => string> = {
  product: () => t("Product"),
  customer: () => t("Customer"),
  supplier: () => t("Supplier"),
  order: () => t("Order"),
  shift: () => t("Shift"),
  po: () => t("Purchase order"),
  sale: () => t("Sale"),
  delivery: () => t("Delivery"),
};

/** Money fields named without the _minor suffix (dashboard figures are in fils too). */
const MONEY_KEY =
  /(^|\.)(sales|refunds|gross_profit|average_basket|revenue|expected_cash|counted_cash|variance)(_prev)?$/;

function cellValue(k: string, v: unknown): ReactNode {
  if (v === null || v === undefined) return "—";
  if (typeof v === "number" && (k.endsWith("_minor") || MONEY_KEY.test(k))) return formatMoney(v);
  if (typeof v === "number" && k.endsWith("_milli")) return formatQty(v);
  if (typeof v === "boolean") return v ? t("Yes") : t("No");
  if (typeof v === "object") return <code className="tiny">{JSON.stringify(v).slice(0, 60)}</code>;
  return String(v);
}

function rowsOf(v: unknown): Record<string, unknown>[] | null {
  if (Array.isArray(v)) return v.filter((x) => x && typeof x === "object") as Record<string, unknown>[];
  if (v && typeof v === "object") {
    for (const k of ["rows", "items", "data", "steps"]) {
      const x = (v as Record<string, unknown>)[k];
      if (Array.isArray(x)) return x.filter((y) => y && typeof y === "object") as Record<string, unknown>[];
    }
  }
  return null;
}

/** Any read result as a table (lists) or key/value list (records). */
export function ResultView({ value }: { value: unknown }) {
  const rows = rowsOf(value);
  if (rows && rows.length) {
    const cols = Object.keys(rows[0])
      .filter((k) => rows.some((r) => r[k] === null || typeof r[k] !== "object"))
      .slice(0, 8);
    return (
      <div style={{ overflowX: "auto" }}>
        <table className="table">
          <thead>
            <tr>
              {cols.map((c) => (
                <th key={c}>
                  <code className="tiny" dir="ltr">
                    {c}
                  </code>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 50).map((r, i) => (
              <tr key={i}>
                {cols.map((c) => (
                  <td key={c}>{cellValue(c, r[c])}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
  if (rows) return <div className="small muted">{t("Nothing found.")}</div>;
  if (value && typeof value === "object") {
    // One level of nesting is shown as "group.field" (e.g. kpis.sales_minor).
    const entries: [string, unknown][] = [];
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (v === null || typeof v !== "object") entries.push([k, v]);
      else if (!Array.isArray(v))
        for (const [k2, v2] of Object.entries(v as Record<string, unknown>)) {
          if (v2 === null || typeof v2 !== "object") entries.push([`${k}.${k2}`, v2]);
        }
    }
    return (
      <dl className="kv">
        {entries.slice(0, 40).map(([k, v]) => (
          <div key={k} style={{ display: "contents" }}>
            <dt>
              <code className="tiny" dir="ltr">
                {k}
              </code>
            </dt>
            <dd>{cellValue(k, v)}</dd>
          </div>
        ))}
      </dl>
    );
  }
  return <div>{String(value)}</div>;
}

function SlashResultCard({ r, onClose, onExplain }: { r: AiSlashResult; onClose: () => void; onExplain: () => void }) {
  const def = SLASH.find((s) => s.name === r.command);
  return (
    <div className="card card-pad col gap-8" data-testid="ai-slash-result">
      <div className="row">
        <strong className="grow">
          /{r.command} · {def?.desc()}
        </strong>
        <Button variant="ghost" onClick={onExplain}>
          {t("Explain with the assistant")}
        </Button>
        <button className="icon-btn" aria-label={t("Close")} onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="tiny muted">
        {t("Read directly with your permissions; the assistant did not see this.")} <code dir="ltr">{r.ran}</code>
        {r.truncated ? ` · ${t("First 50 rows")}` : ""}
      </div>
      <ResultView value={r.result} />
      <details>
        <summary className="tiny">{t("Raw data")}</summary>
        <pre className="tiny" dir="ltr" style={{ whiteSpace: "pre-wrap", maxHeight: 300, overflow: "auto" }}>
          {JSON.stringify(r.result, null, 1)}
        </pre>
      </details>
    </div>
  );
}

// ---------------------------------------------------------------- briefings (A8)

const DAY_LABEL = (): string[] => [t("Mon"), t("Tue"), t("Wed"), t("Thu"), t("Fri"), t("Sat"), t("Sun")];

function BriefingForm({ b, onSaved, onCancel }: { b: AiBriefing | null; onSaved: () => void; onCancel: () => void }) {
  const act = useAction();
  const [name, setName] = useState(b?.name ?? t("End of day"));
  const [playbook, setPlaybook] = useState<AiBriefing["playbook"]>(b?.playbook ?? "eod");
  const [at, setAt] = useState(b?.at_time ?? "22:00");
  const [days, setDays] = useState(b?.days ?? "1234567");
  const [withAi, setWithAi] = useState(b?.with_ai ?? false);
  const [enabled, setEnabled] = useState(b?.enabled ?? true);
  return (
    <Confirm
      title={b ? t("Edit briefing") : t("New briefing")}
      confirmLabel={t("Save")}
      busy={act.busy}
      error={act.error}
      onCancel={onCancel}
      onConfirm={async () => {
        const r = await act.run(() =>
          api.ai.briefingSave(b?.briefing_id ?? null, { name, playbook, at_time: at, days, with_ai: withAi, enabled }),
        );
        if (r) onSaved();
      }}
    >
      <div className="col gap-8">
        <TextInput label={t("Name")} value={name} onChange={(e) => setName(e.target.value)} />
        <Field label={t("Playbook")}>
          <select
            className="select"
            value={playbook}
            onChange={(e) => setPlaybook(e.target.value as AiBriefing["playbook"])}
          >
            {PLAYBOOKS.map((p) => (
              <option key={p.name} value={p.name}>
                {p.label()}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("Time")}>
          <input className="input" type="time" value={at} onChange={(e) => setAt(e.target.value)} />
        </Field>
        <Field label={t("Days")}>
          <div className="row wrap">
            {DAY_LABEL().map((d, i) => {
              const k = String(i + 1);
              return (
                <Checkbox
                  key={k}
                  label={d}
                  checked={days.includes(k)}
                  onChange={(x) =>
                    setDays(x ? [...new Set((days + k).split(""))].sort().join("") : days.replace(k, ""))
                  }
                />
              );
            })}
          </div>
        </Field>
        <Checkbox label={t("Add an AI summary (needs a real provider)")} checked={withAi} onChange={setWithAi} />
        <Checkbox label={t("Enabled")} checked={enabled} onChange={setEnabled} />
        <div className="tiny">
          {t(
            "Runs while AMWAPOS is open on this computer, with the permissions of the person who saves it. It only reads.",
          )}
        </div>
      </div>
    </Confirm>
  );
}

function BriefingsPanel() {
  const toast = useToast();
  const list = useLoad(() => api.ai.briefings(), []);
  const notes = useLoad(() => api.ai.notes(30), []);
  const act = useAction();
  const [edit, setEdit] = useState<AiBriefing | "new" | null>(null);
  const label = (p: string) => PLAYBOOKS.find((x) => x.name === p)?.label() ?? p;
  return (
    <div className="col gap-16" data-testid="ai-briefings">
      <div className="card card-pad col gap-8">
        <div className="row">
          <strong className="grow">{t("Scheduled briefings")}</strong>
          <Button icon={<Plus size={16} />} onClick={() => setEdit("new")}>
            {t("New briefing")}
          </Button>
        </div>
        {act.error ? <Banner tone="danger">{act.error}</Banner> : null}
        {!list.data ? (
          <Skeleton />
        ) : list.data.length ? (
          <table className="table">
            <tbody>
              {list.data.map((b) => (
                <tr key={b.briefing_id}>
                  <td>
                    <strong>{b.name}</strong>
                    <div className="tiny">
                      {label(b.playbook)} · {b.at_time} ·{" "}
                      {b.days
                        .split("")
                        .map((d) => DAY_LABEL()[Number(d) - 1])
                        .join(" ")}
                      {b.with_ai ? ` · ${t("AI summary")}` : ""}
                    </div>
                  </td>
                  <td>{b.enabled ? <Chip tone="success">{t("On")}</Chip> : <Chip>{t("Off")}</Chip>}</td>
                  <td className="tiny">{b.last_run_on ? t("Last run {0}", b.last_run_on) : t("Not run yet")}</td>
                  <td className="row">
                    <Button
                      variant="ghost"
                      loading={act.busy}
                      onClick={async () => {
                        if (await act.run(() => api.ai.briefingRun(b.briefing_id))) {
                          toast("success", t("Briefing ran; see the note below."));
                          void notes.reload();
                          void list.reload();
                        }
                      }}
                    >
                      {t("Run now")}
                    </Button>
                    <Button variant="ghost" onClick={() => setEdit(b)}>
                      {t("Edit")}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={async () => {
                        if (await act.run(() => api.ai.briefingDelete(b.briefing_id))) void list.reload();
                      }}
                    >
                      {t("Delete")}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="small muted">
            {t("No briefings yet. For example: the end-of-day playbook every night at 22:00.")}
          </div>
        )}
      </div>
      <div className="card card-pad col gap-8">
        <strong>{t("Notes")}</strong>
        {!notes.data ? (
          <Skeleton />
        ) : notes.data.length ? (
          notes.data.map((n) => (
            <details
              key={n.note_id}
              onToggle={(e) => {
                if ((e.target as HTMLDetailsElement).open && !n.read_at)
                  void api.ai.noteRead(n.note_id).then(() => notes.reload());
              }}
            >
              <summary className="row">
                <strong className="grow">{n.title}</strong>
                {!n.read_at ? <Chip tone="info">{t("New")}</Chip> : null}
                <Chip tone={n.status === "ok" ? "success" : "warning"}>
                  {n.status === "ok" ? t("Complete") : t("Partial")}
                </Chip>
                <span className="tiny">{formatDateTime(n.created_at)}</span>
              </summary>
              <div className="col gap-8" style={{ marginTop: 8 }}>
                {n.summary ? <div style={{ whiteSpace: "pre-wrap" }}>{n.summary}</div> : null}
                {n.error ? <Banner tone="warning">{tb(n.error)}</Banner> : null}
                <PlaybookResult r={n.data as AiPlaybookResult} onAsk={() => undefined} />
              </div>
            </details>
          ))
        ) : (
          <div className="small muted">{t("No notes yet.")}</div>
        )}
      </div>
      {edit ? (
        <BriefingForm
          b={edit === "new" ? null : edit}
          onCancel={() => setEdit(null)}
          onSaved={() => {
            setEdit(null);
            void list.reload();
          }}
        />
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------- shortcuts (F5)

function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  const rows: [string, string][] = [
    ["Ctrl+K", t("Focus the question box")],
    ["/", t("Start a slash command")],
    ["Enter", t("Send")],
    ["Shift+Enter", t("New line")],
    ["↑", t("Recall the last question (empty box)")],
    ["Tab", t("Complete the highlighted command")],
    ["Esc", t("Close the command list or clear the box")],
    ["Alt+N", t("New conversation")],
    ["Alt+I", t("Action inbox")],
    ["Alt+B", t("Briefings and notes")],
    ["Ctrl+Enter", t("On a focused proposal card: review and confirm")],
    ["Ctrl+Backspace", t("On a focused proposal card: reject")],
    ["?", t("This list")],
  ];
  return (
    <Modal title={t("Keyboard shortcuts")} onClose={onClose} closeOnBackdrop>
      <table className="table">
        <tbody>
          {rows.map(([k, d]) => (
            <tr key={k}>
              <td>
                <kbd dir="ltr">{k}</kbd>
              </td>
              <td>{d}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Modal>
  );
}

// ---------------------------------------------------------------- the chat

type View = "chat" | "inbox" | "briefings";

const newStreamId = () => `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 12)}`;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface PendingPhoto {
  id: string;
  name: string;
}

/**
 * The assistant, usable on the AI page and in the till (compact). Everything
 * the model does is shown as it happens: thinking, each tool call with its
 * input and result, provider fallbacks and checks.
 */
export function AiChat({
  status,
  compact = false,
  context,
  initialText = "",
}: {
  status: AiStatus;
  compact?: boolean;
  /** Called at send time; the till passes its cart (F6). */
  context?: () => AiContext | null;
  initialText?: string;
}) {
  const { has } = useSession();
  const toast = useToast();
  const nav = useNavigate();
  const list = useLoad(() => api.ai.conversations(), []);
  const [cid, setCid] = useState<string | null>(null);
  const [conv, setConv] = useState<AiConversation | null>(null);
  const [text, setText] = useState(initialText);
  const [view, setView] = useState<View>("chat");
  const [live, setLive] = useState<Live | null>(null);
  const [playbook, setPlaybook] = useState<AiPlaybookResult | null>(null);
  const [slashResults, setSlashResults] = useState<AiSlashResult[]>([]);
  const [photos, setPhotos] = useState<PendingPhoto[]>([]);
  const [attachKind, setAttachKind] = useState<"photo" | "invoice" | "payment">("photo");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [shortcuts, setShortcuts] = useState(false);
  const [paletteIdx, setPaletteIdx] = useState(0);
  const [sendContext, setSendContext] = useState(true);
  const lastQuestion = useRef("");
  const act = useAction();
  const pb = useAction();
  const box = useRef<HTMLTextAreaElement>(null);
  const file = useRef<HTMLInputElement>(null);
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!cid) return setConv(null);
    void api.ai.conversation(cid).then(setConv, () => setConv(null));
  }, [cid]);
  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [conv, live, slashResults]);

  const reloadConv = useCallback(async () => {
    if (conv) setConv(await api.ai.conversation(conv.conversation_id));
    void list.reload();
  }, [conv, list]);

  // F1 palette: commands matching what was typed after "/".
  const palette = useMemo(() => {
    if (!text.startsWith("/") || text.includes(" ")) return [];
    const q = text.slice(1).toLowerCase();
    return SLASH.filter((s) => s.name.startsWith(q)).slice(0, 12);
  }, [text]);
  useEffect(() => setPaletteIdx(0), [palette.length]);

  const ask = useCallback(
    async (question: string) => {
      const q = question.trim();
      if (!q) return;
      lastQuestion.current = q;
      const streamId = newStreamId();
      setLive(emptyLive());
      let stop = false;
      const poll = (async () => {
        let after = 0;
        while (!stop) {
          try {
            const r = await api.ai.stream(streamId, after);
            if (r.events.length) {
              setLive((l) => (l ? applyEvents(l, r.events) : l));
              after = r.next;
            }
            if (r.done) break;
          } catch {
            // The question itself reports errors.
          }
          await sleep(200);
        }
      })();
      const ctx = context && sendContext ? context() : null;
      const r = await act.run(() =>
        api.ai.ask(q, conv?.conversation_id ?? null, getLang(), {
          stream_id: streamId,
          images: photos.map((p) => p.id),
          context: ctx,
        }),
      );
      stop = true;
      await poll;
      setLive(null);
      if (r) {
        setText("");
        setPhotos([]);
        setConv(r);
        setCid(r.conversation_id);
        void list.reload();
      }
    },
    [act, context, conv, list, photos, sendContext],
  );

  const resolvePin = async (kind: AiPin["kind"], q: string): Promise<string | null> => {
    if (/^[0-9A-Z]{26}$/.test(q)) return q;
    if (kind === "product") {
      const r = await api.products.search({ q, limit: 1 });
      return r.rows[0]?.product_id ?? null;
    }
    if (kind === "customer") {
      const r = await api.customers.search(q, false, 1);
      return r[0]?.customer_id ?? null;
    }
    return null;
  };

  const runSlash = async (line: string) => {
    const [head, ...rest] = line.slice(1).trim().split(/\s+/);
    const name = (head ?? "").toLowerCase();
    const arg = rest.join(" ");
    const def = SLASH.find((s) => s.name === name);
    setText("");
    if (!def) {
      toast("error", t("Unknown command /{0}. Type /help for the list.", name));
      return;
    }
    if (def.read) {
      const r = await act.run(() => api.ai.slash(name, arg));
      if (r) setSlashResults((x) => [...x.slice(-4), r]);
      return;
    }
    switch (name) {
      case "help":
        setText("/");
        box.current?.focus();
        return;
      case "new":
        setCid(null);
        setConv(null);
        setView("chat");
        return;
      case "rename":
        if (!conv) return toast("info", t("Ask a question first; then name the conversation."));
        if (!arg) return setRenaming(conv.title);
        if (await act.run(() => api.ai.rename(conv.conversation_id, arg))) void reloadConv();
        return;
      case "pin":
      case "unpin": {
        if (!conv) return toast("info", t("Ask a question first; then pin records to it."));
        const [kind, ...q] = arg.split(/\s+/);
        if (!PIN_KINDS.includes(kind as AiPin["kind"]) || !q.length) {
          return toast("info", t("Use /{0} <kind> <name or id>. Kinds: {1}", name, PIN_KINDS.join(", ")));
        }
        const id = name === "pin" ? await resolvePin(kind as AiPin["kind"], q.join(" ")) : q.join(" ");
        if (!id) return toast("error", t("Nothing found to pin."));
        const r = await act.run(() =>
          name === "pin"
            ? api.ai.pin(conv.conversation_id, kind as AiPin["kind"], id)
            : api.ai.unpin(conv.conversation_id, kind as AiPin["kind"], id),
        );
        if (r) void reloadConv();
        return;
      }
      case "price": {
        const m = arg.match(/^(.+)\s+([\d.,٠-٩]+)$/);
        if (!m) return toast("info", t("Use /price <product> <amount>, e.g. /price Tea 100g 0.450"));
        return void ask(t("Set the price of {0} to {1}", m[1], m[2]));
      }
      case "explain": {
        if (!arg) return toast("info", t("Use /explain <command>, e.g. /explain low"));
        return void ask(
          t("Use your tools to look at: {0}. Explain what needs attention and what I should do next.", arg),
        );
      }
      case "ask":
        return void ask(arg);
      case "attach":
        file.current?.click();
        return;
      case "inbox":
        setView("inbox");
        return;
      case "briefings":
        setView("briefings");
        return;
      case "goto": {
        const page = PAGES.find((p) => p === arg.trim().toLowerCase());
        if (!page) return toast("info", t("Pages: {0}", PAGES.join(", ")));
        nav(`/admin/${page}`);
        return;
      }
      case "model":
        toast(
          "info",
          `${providerLabel(status.active_provider)} · ${status.model_id}` +
            (status.daily_token_cap
              ? ` · ${t("Tokens today: {0} of {1}", status.tokens_today ?? 0, status.daily_token_cap)}`
              : ` · ${t("Tokens today: {0}", status.tokens_today ?? 0)}`) +
            (status.fallback_ready ? ` · ${t("Fallback ready")}` : ""),
        );
        return;
      case "shortcuts":
        setShortcuts(true);
        return;
      case "clear":
        setSlashResults([]);
        setPlaybook(null);
        return;
    }
  };

  const submit = () => {
    const v = text.trim();
    if (!v) return;
    if (v.startsWith("/")) void runSlash(v);
    else void ask(v);
  };

  // F5: page-level shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = ["INPUT", "TEXTAREA", "SELECT"].includes((e.target as HTMLElement)?.tagName ?? "");
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        box.current?.focus();
      } else if (e.altKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setCid(null);
        setConv(null);
        setView("chat");
        box.current?.focus();
      } else if (e.altKey && e.key.toLowerCase() === "i") {
        e.preventDefault();
        setView((v) => (v === "inbox" ? "chat" : "inbox"));
      } else if (e.altKey && e.key.toLowerCase() === "b") {
        e.preventDefault();
        setView((v) => (v === "briefings" ? "chat" : "briefings"));
      } else if (!typing && e.key === "/") {
        e.preventDefault();
        setText("/");
        box.current?.focus();
      } else if (!typing && e.key === "?") {
        setShortcuts(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const attach = async (f: File) => {
    const data = await fileToBase64(f);
    if (attachKind === "photo") {
      const r = await act.run(() => api.ai.attachImage(f.type, data));
      if (r) setPhotos((p) => [...p, { id: r.attachment_id, name: f.name }]);
    } else if (attachKind === "invoice") {
      const r = await act.run(() => api.invoiceScan.import({ file_name: f.name, data }));
      if (r) {
        toast("success", t("Invoice {0} is being read; the assistant can open it when OCR finishes.", r.scan_number));
        setText((x) => `${x}${x ? " " : ""}${t("Check invoice scan {0} and propose receiving it.", r.scan_number)}`);
      }
    } else {
      const r = await act.run(() => api.payreviews.upload({ file_name: f.name, data }));
      if (r) {
        toast("success", t("Payment screenshot {0} added for review.", r.review_number));
        setText((x) => `${x}${x ? " " : ""}${t("Compare payment review {0} with what is owed.", r.review_number)}`);
      }
    }
  };

  const conversations = (
    <div className="card" style={{ maxHeight: compact ? 220 : 640, overflow: "auto" }}>
      <div style={{ padding: 12 }} className="col gap-8">
        <Button icon={<Plus size={16} />} onClick={() => (setCid(null), setConv(null), setView("chat"))}>
          {t("New conversation")}
        </Button>
        <div className="col" style={{ gap: 6 }}>
          <Button
            variant={view === "inbox" ? "primary" : "ghost"}
            icon={<Inbox size={16} />}
            data-testid="ai-inbox-button"
            onClick={() => setView(view === "inbox" ? "chat" : "inbox")}
          >
            {t("Action inbox")}
          </Button>
          {has("admin.access") ? (
            <Button
              variant={view === "briefings" ? "primary" : "ghost"}
              icon={<CalendarClock size={16} />}
              data-testid="ai-briefings-button"
              onClick={() => setView(view === "briefings" ? "chat" : "briefings")}
            >
              {t("Briefings")}
            </Button>
          ) : null}
        </div>
      </div>
      {(list.data ?? []).map((c) => (
        <button
          key={c.conversation_id}
          className={`list-row ${cid === c.conversation_id ? "active" : ""}`}
          style={{
            display: "block",
            width: "100%",
            textAlign: "start",
            padding: 12,
            borderTop: "1px solid var(--border)",
          }}
          onClick={() => (setCid(c.conversation_id), setView("chat"))}
        >
          <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.title}</div>
          <div className="tiny">
            {relative(c.updated_at)}
            {c.open_proposals ? ` · ${t("{0} to review", c.open_proposals)}` : ""}
          </div>
        </button>
      ))}
    </div>
  );

  const chat = (
    <div className="col gap-16">
      {!compact ? (
        <div className="small muted">
          {status.mutations && status.can_mutate
            ? t(
                "The assistant may propose any admin change you are allowed to make. Nothing changes until a person confirms.",
              )
            : t("Read-only: the assistant cannot change anything.")}{" "}
          {t("Provider")}: {providerLabel(status.active_provider)} · {status.model_id}
          {status.daily_token_cap
            ? ` · ${t("Tokens today: {0} of {1}", status.tokens_today ?? 0, status.daily_token_cap)}`
            : ""}
          {status.fallback_ready ? ` · ${t("Fallback ready")}` : ""}
        </div>
      ) : null}
      {conv ? (
        <div className="row wrap" style={{ gap: 6 }}>
          {renaming !== null ? (
            <form
              className="row"
              onSubmit={async (e) => {
                e.preventDefault();
                if (await act.run(() => api.ai.rename(conv.conversation_id, renaming))) {
                  setRenaming(null);
                  void reloadConv();
                }
              }}
            >
              <input
                className="input"
                autoFocus
                aria-label={t("Conversation name")}
                value={renaming}
                onChange={(e) => setRenaming(e.target.value)}
              />
              <Button type="submit" variant="primary">
                {t("Save")}
              </Button>
              <Button variant="ghost" onClick={() => setRenaming(null)}>
                {t("Cancel")}
              </Button>
            </form>
          ) : (
            <>
              <strong>{conv.title}</strong>
              <button className="icon-btn" aria-label={t("Rename")} onClick={() => setRenaming(conv.title)}>
                <Pencil size={14} />
              </button>
            </>
          )}
          {(conv.pins ?? []).map((p) => (
            <Chip key={`${p.kind}-${p.id}`}>
              <Pin size={12} /> {PIN_LABEL[p.kind]()}: {p.label}{" "}
              <button
                className="icon-btn"
                aria-label={t("Unpin")}
                onClick={async () => {
                  if (await act.run(() => api.ai.unpin(conv.conversation_id, p.kind, p.id))) void reloadConv();
                }}
              >
                <X size={12} />
              </button>
            </Chip>
          ))}
          <span className="tiny muted">{t("Pin with /pin product <name>")}</span>
        </div>
      ) : null}
      {!compact ? (
        <div className="row wrap" data-testid="ai-playbooks">
          <span className="small muted">{t("Playbooks")}:</span>
          {PLAYBOOKS.map((b) => (
            <Button
              key={b.name}
              variant="ghost"
              loading={pb.busy}
              onClick={async () => {
                const r = await pb.run(() => api.ai.playbook(b.name));
                if (r) setPlaybook(r);
              }}
            >
              {b.label()}
            </Button>
          ))}
        </div>
      ) : null}
      {pb.error ? <Banner tone="danger">{pb.error}</Banner> : null}
      {playbook ? <PlaybookResult r={playbook} onAsk={(q) => (setText(q), setPlaybook(null))} /> : null}
      {conv?.untrusted_seen ? (
        <Banner tone="warning">
          {t("This conversation read customer messages, photos or scanned text. Check any proposal carefully.")}
        </Banner>
      ) : null}
      <div className="card card-pad col gap-16" style={{ minHeight: compact ? 200 : 320 }}>
        {!conv && !live ? (
          <div className="empty">
            <Bot size={28} />
            <div>{t("Try: “Which products are running low?”, or type / for commands.")}</div>
          </div>
        ) : null}
        {conv?.messages.map((m, i) => (
          <MessageView key={i} m={m} />
        ))}
        {live ? <LiveView live={live} /> : null}
        {conv?.proposals.map((p) => (
          <ProposalCard key={p.proposal_id} p={p} onChanged={() => void reloadConv()} />
        ))}
        {slashResults.map((r, i) => (
          <SlashResultCard
            key={i}
            r={r}
            onClose={() => setSlashResults((x) => x.filter((_, j) => j !== i))}
            onExplain={() =>
              void ask(
                t(
                  "Use your tools to look at: {0}. Explain what needs attention and what I should do next.",
                  `/${r.command}`,
                ),
              )
            }
          />
        ))}
        <div ref={end} />
      </div>
      {act.error ? <Banner tone="danger">{act.error}</Banner> : null}
      {photos.length ? (
        <div className="row wrap">
          {photos.map((p) => (
            <Chip key={p.id}>
              {p.name}{" "}
              <button
                className="icon-btn"
                aria-label={t("Remove")}
                onClick={() => setPhotos((x) => x.filter((y) => y.id !== p.id))}
              >
                <X size={12} />
              </button>
            </Chip>
          ))}
          <span className="tiny">{t("Photos are sent to the AI provider and treated as outside text.")}</span>
        </div>
      ) : null}
      {context ? (
        <Checkbox label={t("Include the current cart")} checked={sendContext} onChange={setSendContext} />
      ) : null}
      <div style={{ position: "relative" }}>
        {palette.length ? (
          <div
            className="card"
            role="listbox"
            data-testid="ai-slash-palette"
            style={{
              position: "absolute",
              bottom: "100%",
              insetInlineStart: 0,
              insetInlineEnd: 0,
              maxHeight: 280,
              overflow: "auto",
              zIndex: 5,
            }}
          >
            {palette.map((s, i) => (
              <button
                key={s.name}
                role="option"
                aria-selected={i === paletteIdx}
                className={`list-row ${i === paletteIdx ? "active" : ""}`}
                style={{ display: "flex", gap: 8, width: "100%", textAlign: "start", padding: "6px 12px" }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  setText(`/${s.name}${s.args ? " " : ""}`);
                  box.current?.focus();
                }}
              >
                <code dir="ltr">
                  /{s.name}
                  {s.args ? ` ${s.args}` : ""}
                </code>
                <span className="small muted">{s.desc()}</span>
              </button>
            ))}
          </div>
        ) : null}
        <textarea
          ref={box}
          style={{ width: "100%" }}
          className="input"
          rows={compact ? 1 : 2}
          maxLength={4000}
          value={text}
          aria-label={t("Question")}
          placeholder={t("Ask a question, or type / for commands")}
          data-testid="ai-composer"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (palette.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              setPaletteIdx((i) => (i + (e.key === "ArrowDown" ? 1 : palette.length - 1)) % palette.length);
            } else if (palette.length && e.key === "Tab") {
              e.preventDefault();
              const s = palette[paletteIdx];
              setText(`/${s.name}${s.args ? " " : ""}`);
            } else if (e.key === "Escape") {
              setText("");
            } else if (e.key === "ArrowUp" && !text && lastQuestion.current) {
              e.preventDefault();
              setText(lastQuestion.current);
            } else if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (palette.length) {
                // Complete the highlighted command; run it when it needs nothing more.
                const s = palette[paletteIdx];
                if (s.args) return setText(`/${s.name} `);
                return void runSlash(`/${s.name}`);
              }
              submit();
            }
          }}
        />
        <div className="row" style={{ marginTop: 8 }}>
          <input
            ref={file}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              if (f) void attach(f);
            }}
          />
          <select
            className="select"
            aria-label={t("Attach as")}
            value={attachKind}
            style={{ width: "auto" }}
            onChange={(e) => setAttachKind(e.target.value as typeof attachKind)}
          >
            <option value="photo">{t("Photo for the assistant")}</option>
            {has("ocr.scan") ? <option value="invoice">{t("Supplier invoice (OCR)")}</option> : null}
            {has("payments.review") ? <option value="payment">{t("Payment screenshot (OCR)")}</option> : null}
          </select>
          <button
            className="icon-btn"
            aria-label={t("Attach")}
            title={t("Attach")}
            onClick={() => file.current?.click()}
          >
            <Paperclip size={18} />
          </button>
          <span className="grow" />
          <Button
            id="ai-send"
            variant="primary"
            icon={<Send size={16} />}
            loading={act.busy}
            disabled={!text.trim()}
            onClick={submit}
          >
            {t("Ask")}
          </Button>
          <button
            className="icon-btn"
            aria-label={t("Keyboard shortcuts")}
            title={t("Keyboard shortcuts")}
            onClick={() => setShortcuts(true)}
          >
            <Keyboard size={18} />
          </button>
        </div>
      </div>
      {shortcuts ? <ShortcutsHelp onClose={() => setShortcuts(false)} /> : null}
    </div>
  );

  if (compact) {
    return (
      <div className="col gap-8">
        {view === "inbox" ? <ActionInbox onOpen={(id) => (setCid(id), setView("chat"))} /> : null}
        {view === "briefings" ? <BriefingsPanel /> : null}
        {view === "chat" ? chat : null}
        <details>
          <summary className="small">{t("Conversations, inbox and briefings")}</summary>
          {conversations}
        </details>
      </div>
    );
  }
  return (
    <div className="grid-2" style={{ gridTemplateColumns: "280px 1fr", gap: 16, alignItems: "start" }}>
      {conversations}
      {view === "inbox" ? <ActionInbox onOpen={(id) => (setCid(id), setView("chat"))} /> : null}
      {view === "briefings" ? <BriefingsPanel /> : null}
      {view === "chat" ? chat : null}
    </div>
  );
}

/** Setup state shared by the page and the till widget. */
export function AiReady({ children }: { children: (st: AiStatus) => ReactNode }) {
  const { has } = useSession();
  const status = useLoad(() => api.ai.status(), []);
  const st = status.data;
  if (status.error) return <Banner tone="danger">{status.error}</Banner>;
  if (!st) return <Skeleton />;
  if (!st.ready) {
    return (
      <Banner tone="info" title={t("The assistant is not set up yet")}>
        <div className="col gap-8">
          {!st.key_configured ? <div>• {t("No AI provider key is stored.")}</div> : null}
          {st.settings.consent === false ? (
            <div>• {t("An owner has not agreed to send store data to the provider.")}</div>
          ) : null}
          {has("settings.manage") ? (
            <Link to="/admin/settings?section=ai">{t("Open Settings → AI")}</Link>
          ) : (
            <div>{t("Ask the owner to finish the setup in Settings → AI.")}</div>
          )}
        </div>
      </Banner>
    );
  }
  return <>{children(st)}</>;
}

export function AiAssistantPage() {
  const [search] = useSearchParams();
  const status = useLoad(() => api.ai.status(), []);
  const st = status.data;
  return (
    <div>
      <PageHeader
        title={t("AI Assistant")}
        subtitle={t("Ask about sales, stock, margins and purchasing. The assistant reads data with your permissions.")}
      />
      {st ? (
        <div className="row" style={{ marginBottom: 12 }} data-testid="ai-provider-chip">
          <Chip tone={st.active_provider === "fake" ? "default" : "info"}>
            {st.active_provider === "fake" ? t("Offline test model") : providerLabel(st.active_provider)} ·{" "}
            {st.model_id}
          </Chip>
          <span className="tiny muted">{t("Press ? for shortcuts, / for commands.")}</span>
        </div>
      ) : null}
      <FeatureGate feature="ai.enabled">
        <AiReady>{(s) => <AiChat status={s} initialText={search.get("q") ?? ""} />}</AiReady>
      </FeatureGate>
    </div>
  );
}
