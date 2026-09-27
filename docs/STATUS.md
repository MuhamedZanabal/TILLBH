# Completion status

**Overall: feature-complete for the Windows soak, not release-complete.** A row is **Complete**
only when it is implemented, tested automatically in this repository, and needs no Windows
hardware or third-party account. Everything that needs the store's hardware stays **Partially
complete** until the owner signs off the hardware pass in [OPERATIONS.md](OPERATIONS.md).

Classes:
- **Complete:** implemented and tested here; no hardware or vendor dependency.
- **Partially complete:** implemented and tested off-target; hardware verification is pending.
- **Blocked:** needs something outside the repository before work can continue.
- **Deferred:** deliberately not built yet. The admin page says **Not enabled**. Each needs an
  owner, a key and a rollback plan.

## Soak installer (CI)

| | |
| --- | --- |
| File | `AMWAPOS_0.1.0_x64-setup.exe` (artifact `amwapos-windows-unsigned`) |
| SHA-256 | `d6c0e260be719cd05a3755601bc7c70ea95153e535695173934ab371ee94f452` |
| Built by | GitHub Actions CI run #12 (`36024872450`) on `windows-2022`, commit `ad7e650` |
| WebView2 | Bootstrapper **embedded** (`webviewInstallMode: embedBootstrapper`); the CI job fails if the configuration changes to an install-time download |
| Signing | **Unsigned.** For internal soak only (SmartScreen will warn). |
| Download | https://github.com/ZanaNowshad/AMWAPOS/actions/runs/36024872450 (artifact `amwapos-windows-unsigned`, id 10819612747, kept for 90 days) |

Evidence was re-run on 2026-09-24:
- **Rust** (`cargo test --workspace`, Linux and Windows CI), 77 tests:
  - 38 core unit tests;
  - 7 back-office, 15 flow, 6 printing and 6 sync tests;
  - 4 encrypted-channel tests and 1 HTTP sync test.
- **Lint:** `cargo fmt` and `clippy -D warnings` are clean; `tsc` and `eslint` are clean.
- **Frontend unit tests** (vitest): 20.
- **End-to-end** (Playwright): 3 flows.
  - Owner: setup → sale → split pay → refund → shift close, with the backup banner.
  - Cashier: Admin blocked, manager PIN, wrong PIN, audit row, and a scanner burst.
  - Arabic: right-to-left sale, admin, dark/compact theme.
- **Proxy test:** no product name, barcode, receipt number, pairing code, device key or PIN hash
  crosses the LAN in clear.
- **Performance** (100k products, release build, Linux sandbox): P95 scan 0.73 ms, search
  23.6 ms, cart 0.78 ms, sale commit 9.9 ms.

## Known limits

| # | Limit (frozen wording) | State |
| --- | --- | --- |
| 1 | Arabic receipt glyphs print as `?` (image-fallback not built). | **Fixed in code** (commit `eda19ef`). Arabic lines are shaped and rasterized (`GS v 0`), and tests prove no `?` reaches the printer. Still open until Arabic is seen on the store's 80 mm printer (hardware pass). |
| 2 | No RTL shell. | **Fixed** (commit `39652d4`). Cashier and admin work fully in Arabic right-to-left. |
| 3 | DB file unencrypted (BitLocker-dependent). | **Open.** See SECURITY.md: stolen-database paragraph and residual risks. |
| 4 | Unsigned installer; not run on Windows. | **Open.** The installer is now built and tested by Windows CI (above), but it is unsigned and has not been installed on a store Windows 10/11 machine. |

## Matrix

| Area | Status | Evidence | Pending |
| --- | --- | --- | --- |
| Integer money, VAT incl./excl., discount allocation, rounding (backend and UI previews) | Complete | `money`, `pricing` tests; vitest `mulDivRound` (half away from zero, as backend) | — |
| Server-authoritative cart & pricing, effective-dated prices | Complete | Flow and back-office tests. Price lookups use the application clock (a Windows CI clock-skew bug was fixed). | — |
| Exactly-once sale / refund / cash / bulk price / import | Complete | Idempotency replay and mismatch tests; lost-response sync test | — |
| Append-only financial history, hash-chained audit | Complete | Trigger tests; audit verify; E2E "Audit chain verified" | — |
| PIN auth (Argon2id), lockout, roles, backend permission checks | Complete | Core tests; E2E cashier blocked from Admin | — |
| Manager override (single-use, permission-bound, audited) | Complete | Core tests; E2E: wrong PIN changes nothing; audit row names the approver | — |
| Shifts, blind close, variance approval, cash in/out | Complete | Flow tests; E2E expected-cash check | — |
| Refunds bounded by refundable qty, exact proration | Complete | Flow tests; E2E refund | — |
| Inventory ledger, weighted-average cost, adjustments, stocktake | Complete | Back-office tests | — |
| Suppliers, purchase orders, receiving | Complete | Back-office tests | — |
| Customers, addresses, deliveries, delivery board (events, payment state) | Complete | Core tests; board with translated columns | — |
| Reports (14) + CSV export (formula-injection safe) | Complete | Report tests; CSV escape round-trip test | — |
| CSV product import | Complete | Preview/apply tests, duplicate isolation, scientific-notation guard, 100k import | — |
| Backup / verified restore / safety backup | Partially complete | Round-trip and tamper tests; E2E backup | USB / network-share folder checked in the soak (OPERATIONS checklist) |
| Backup-while-closed | Complete (operational rule) | OPERATIONS "Backup rule" (the hub keeps AMWAPOS running). Red banner on every Admin page and a till header pill, with one-click Backup Now. `backup.health` reports ok / overdue / failed. | A Windows scheduled task was deliberately not built (reasons in OPERATIONS) |
| Diagnostics | Complete | Readable details; last backup shown in local time with age; sync errors classified | — |
| Sync protocol v2 (encrypted, SPAKE2 pairing, one live code, burn after 5, pairing-id reuse rejected, versioned) | Complete | Encrypted-channel tests: proxy test, protocol 1 refused, burn, version mismatch shown as "Update needed", not offline | — |
| Lost hub credential | Complete | Reported, never silently replaced; owner reset + re-pair (sync test) | — |
| Multi-terminal operation on store Wi-Fi | Partially complete | Sync convergence tests; signed + encrypted HTTP tests on loopback | Two tills + hub on store Wi-Fi; unplug the hub mid-sale |
| Printing: receipt, COPY reprint, cash-drawer pulse, failure keeps sale | Partially complete | `tests/printing.rs`: receipt content and width; COPY; drawer pulses on cash only (not card, not reprint, not when off); failed printer keeps the sale and retry works; ESC/POS framing | 80 mm printer + drawer (network and Windows spooler) |
| Arabic receipts | Partially complete | Raster unit tests (shaping, lam-alef, bidi, placement); sale/refund receipts with Arabic names and bilingual labels; no `?` in text mode; test page has an Arabic line | Arabic on the physical printer |
| Barcode scanner (HID wedge) | Partially complete | Vitest heuristics; E2E burst of 5 scans at scanner speed, none dropped, field cleared | Physical USB scanner |
| Mid-sale power loss | Partially complete | WAL + `synchronous=FULL`; sale commit is one transaction; exactly-once replay tests | Pull the plug mid-sale on a till |
| Cashier Mode / Admin Mode UI, English and Arabic (RTL), light/dark, density | Partially complete | Playwright English + Arabic flows, dark/compact screenshot. The unit test fails on any untranslated `t()` key or status label. Backend messages translated through `tb()`. | Visual check in WebView2 during the soak |
| Performance targets (100k products) | Partially complete | Numbers above (Linux sandbox) | Re-measure on the till hardware during the soak |
| Windows desktop shell (single instance, Credential Manager, ProgramData ACLs, 30-day rotating logs) | Partially complete | Built and unit-tested on Windows CI; launched under Xvfb on Linux | First launch on a Windows 10/11 till |
| NSIS installer (per-machine, firewall rules, ACLs, data kept on uninstall, embedded WebView2) | Partially complete | Built by Windows CI (above); embedded WebView2 enforced by CI | Install / upgrade / uninstall on Windows 10 and 11 |
| CI (lint, types, unit, E2E, Windows build + installer) | Complete | GitHub Actions green on this branch | — |
| Release workflow (tag → draft release, SBOMs, SHA-256 sums, optional signing) | Partially complete | Workflow lint-clean; SBOM generation run locally | First tag run |
| Code signing | Deferred | Unsigned is accepted for internal soak | Authenticode certificate |
| Auto-update (flag `updates`) | Partially complete | Ed25519-signed manifest, size + SHA-256 checks, re-verify before install, safety backup; refuses unsigned builds (`tests/updates.rs`) | Signing key (`AMWAPOS_UPDATE_PUBKEY`), hosting, install run on Windows |
| WhatsApp (flags `whatsapp.enabled`, `.send_receipts`, `.delivery_notices`) | Partially complete | In-process `whatsapp-rust` =0.7.0 adapter behind a trait; supervisor restart after panic, separate status flags, reconnect after restart, idempotent sends, persist-before-ack inbound, media → review (`tests/whatsapp.rs` with `FakeAdapter`); post-commit receipts/notices, payload-hash idempotency, EN/AR templates (`tests/automation.rs`) | Pairing and soak test with a real phone; the real adapter has never connected to WhatsApp |
| OCR (flags `ocr.enabled`, `.payment_screenshots`, `.supplier_invoices`) | Partially complete | Separate worker running bundled Tesseract with SHA-256-checked eng+ara models; `ocr_model_missing` keeps it off; statuses ocr_match/likely_match/mismatch/needs_review; draft PO only (`tests/ocr.rs` with real Tesseract, `tests/automation.rs`) | Accuracy on real supplier invoices and BenefitPay screenshots |
| AI assistant (flags `ai.enabled`, `ai.mutations`, `ai.dual_control`) | Partially complete | Read and proposal tools as the signed-in user; confirm runs the normal command; undo by compensating record where one exists; fake-provider and loopback-stub tests (`tests/ai*.rs`) | Vendor API key and owner consent; never called against a real provider or OpenRouter here |
| Card terminal / BenefitPay integration | Deferred | Tenders recorded manually with a reference | Provider SDK, merchant account, owner, rollback |
| Migration (CSV/XLSX/ZIP/folder) | Complete | Detect → map → preview (no writes) → apply through normal commands (`tests/migration.rs`) | — |
| Customer credit (flag `customer_credit`) | Complete | Append-only ledger, limit + manager override, refunds, cash payments in drawer (`tests/credit.rs`) | — |
| Windows Hello step-up (flag `windows_hello`) | Partially complete | Runtime gate + audit (`tests/step_up.rs`); WinRT call type-checked for Windows | Run on a Windows machine with Hello |
| PDF receipts (flag `pdf_receipts`) | Complete | Raster PDF after commit; failure never affects the sale (`tests/automation.rs`) | — |
| Database encryption at rest | Deferred (known limit 3) | BitLocker guidance and threat paragraph in SECURITY.md | Owner decision |


## Spec pass (items 3–42), 2026-09-25

Flags (all default off): `hub`, `whatsapp.enabled`, `whatsapp.send_receipts`, `whatsapp.delivery_notices`,
`ocr.enabled`, `ocr.payment_screenshots`, `ocr.supplier_invoices`, `ocr.ai_parse`, `ai.enabled`, `ai.mutations`,
`customers.credit`, `windows_hello`, `pdf_receipts`, `updates`. Old keys (`whatsapp`, `ocr`, `payment_reviews`, `ai`,
`ai_mutations`, `customer_credit`) are read as aliases. Store settings: negative stock allowed with a warning (default),
costing method `weighted_average` (receiving updates cost, audited) or `manual`, receipts 80 mm (58 mm option), EN or EN+AR labels.

Not in this pass by decision: live WhatsApp link and real photos (owner soak), code signing, card SDK, Cloud API,
hub TLS rewrite, Task Scheduler backups, the full acceptance matrix. The updater is AMWAPOS' own Ed25519-verified
flow rather than tauri-plugin-updater; it refuses unsigned builds and opens the installer window (no silent apply).

## Product brief: 12 pillars, 2026-09-25

Every new module is behind a flag that is off by default. With all flags off,
the new tables exist (migrations 0009, 0010) but nothing reads or writes them.
`customers.credit` was not touched and stays off.

| Flag | Module |
|---|---|
| `inventory.locations` | Stock locations; transfers draft → ship → receive (idempotent op ids, in-transit qty, no stock creation) |
| `loyalty.enabled` | Integer points ledger; earn on committed sales; redeem as a discount priced by `price_cart`; proportional reversal on refund |
| `orders.digital` | Phone/WhatsApp/web/other orders; human confirm; idempotent load into a till sale; optional delivery on commit |
| `org.multi_branch` | Branch CRUD, user branch assignment, session branch switch, branch prices, branch pairing, branch-scoped mutations and reports |
| `pwa.companion` | Hub-served read-only owner phone page; hashed, revocable bearer token (≤ 24 h), LAN peers only |

Always on (no flag, no behaviour change for existing flows): ticket number on hold
and recall by number, low-stock hint on cart lines, safe-drop running total and
expected-cash formula on the shift screens, supplier last cost on the product,
supplier performance report, saved report date ranges, end-of-day pack + CSV zip.

Limits: one hub per LAN holds all branches (no cross-hub mesh). The phone page's
offline shell needs HTTPS for its service worker; on plain LAN HTTP the page
still shows the last snapshot it received. Soak, signing keys, the live WhatsApp
phone and Windows Hello hardware remain the owner's checks; nothing here is
claimed as soak-tested or production-ready.

Tests: `crates/amwapos-core/tests/pillars.rs` (transfers in transit + idempotency,
loyalty money/VAT/refund reversal, digital-order idempotent convert, multi-branch
isolation with the flag on and identical behaviour with it off, end-of-day pack)
and `crates/amwapos-hub/tests/companion.rs` (flag, token, revoke, LAN routes).

## AI: bring your own API key, 2026-09-26

- Providers: `fake | openai | anthropic | google | openrouter | custom`. The fake
  model is active whenever provider=fake or no key is stored; it never uses the
  network. No OAuth / device-code / subscription sign-in exists or is planned.
- Keys and the optional extra-header value live only in Windows Credential
  Manager (service `AMWAPOS`, accounts `ai/<provider>` and `ai/<provider>/header`).
  Settings (provider, model_id, base_url, header name, max_output_tokens,
  timeout_ms, model list cache) are in SQLite; no secret is.
- Owner-only: `ai.configure`, `ai.test`, `ai.models`. Changes apply on the next
  request (settings are re-read every round).
- Every prompt starts with `crates/amwapos-core/src/ai_prompts/constitution.txt`
  (verbatim) plus matching lines of `ai_prompts/playbooks.txt`; untrusted text is
  wrapped in `<<<DATA … END DATA>>>`; proposals need the user's own change request,
  and zeroing stock after DATA was read is refused. One retry on 429/502/503.
- Errors: `AI_NOT_ENABLED`, `AI_NO_KEY`, `AI_PROVIDER_ERROR`, `AI_TIMEOUT`,
  `AI_MODEL_NOT_FOUND`. Diagnostics export replaces any stored AI secret value.
- Tests: `crates/amwapos-hub/tests/ai_byok.rs` (loopback stubs only).

## AI: full admin tool map, 2026-09-26

Status: **Partially complete.** Tested here with the offline test model only;
never run against a live provider, the live WhatsApp phone or Windows Hello.

- `crates/amwapos-core/src/ai_tools.rs`: 71 read tools and 93 `propose_*` tools,
  each an existing command. A tool is offered only with `admin.access`, one of its
  permissions, the owner role when owner-only, and its feature flag. The
  accountant role never gets proposal tools. With `ai.mutations` off, every
  `propose_*` tool is hidden and refused. Lists are capped at 50 rows.
- Every other command is listed in `NO_TOOL` with its reason (forbidden, till or
  setup only, file/binary, or covered by another tool). A test fails when a new
  command has neither a tool nor a reason.
- Writes only record a proposal (`command:<cmd>`, migration 0011). Confirm runs
  the same command through `Runtime::dispatch` with the confirmer's session, so
  permissions, owner-only rules, manager approval and Windows Hello step-up all
  apply. A failed human check (wrong PIN, cancelled Hello) leaves the proposal
  open. Command proposals are irreversible from the AI page (no fake undo).
- PINs and approval tokens come only from the Confirm card and are never stored
  or sent to the model. One-time secrets (phone-view link) are returned once to
  the card and stripped from the stored result. QR, pairing codes, PIN hashes,
  keys and session data are stripped from every read.
- Limits: 30 proposals per hour per user, 200 items per bulk proposal, proposals
  expire after 60 minutes, optional daily token cap (Settings → AI). After DATA is
  read in a conversation, every proposal is high risk.
- A reply with figures and no tool call gets one "[AMWAPOS check]" nudge; if it
  still has no tool call it is shown as **Unverified**. Answers carry evidence
  chips; record paths become links; a barcode in the question names its product.
- AI page: action inbox with today's digest, playbook buttons (eod, cash_short,
  reorder, refund_spike; reads only, no model), before/after diff on each card.
  Invoice scan: **Improve parse** (`invoicescan.ai_parse`).
- Consent is per provider: moving between two real providers asks the owner to
  agree again.
- Tests: `crates/amwapos-hub/tests/ai_admin.rs` (15), `ai_byok.rs` unchanged.

## AI workspace, 2026-09-26

Status: **Partially complete.** Tested with the offline test model and loopback
provider stubs (Anthropic and OpenAI-style SSE); never run against a live
provider, OpenRouter, the live WhatsApp phone or a Windows till.

- **C8 streaming and transparency.** With a `stream_id`, every provider streams
  (Anthropic SSE with `thinking.display: "summarized"`, thinking signatures
  replayed unchanged; OpenAI-compatible SSE with reasoning and tool-call deltas;
  Gemini SSE with thought summaries). The page polls `ai.stream` and shows, as
  they happen: thinking, each tool call with its input, each tool result exactly
  as the model saw it, nudges, fallbacks, token use. Stored conversations keep the
  same trace (thinking, calls with results).
- **C4 free fallback.** Owner opt-in, with its own consent and an OpenRouter key in
  Credential Manager. Used only when the chosen provider is unavailable (timeout,
  unreachable, 429, 5xx, unknown model), never for a wrong key or a refusal.
  Default model `openrouter/free` (editable). Shown on the page and audited
  (`ai.fallback`).
- **A5 photos.** Attach a photo for the model (PNG/JPEG/WEBP/GIF, 5 MB, content
  sniffed, owner-only), or send a supplier invoice / payment screenshot into the
  existing OCR pipelines from the same button. A photo marks the thread untrusted.
- **A6 memory.** Rename conversations; pin up to 8 records (product, customer,
  supplier, order, shift, PO, sale, delivery), read with the user's permissions,
  labels passed as DATA.
- **A8 briefings.** A playbook at a time of day and days of the week, run while
  the app is open with the permissions of whoever saved it (short internal
  session, ended after the run), once per day; optional AI summary with a real
  provider. Notes are listed on the AI page (`ai.notes`). Audited.
- **E1 triage.** Incoming WhatsApp messages sorted into order / payment /
  complaint / question / spam / other by rules, optionally re-sorted by the AI;
  a person can correct each one. Suggested next step per category.
- **E2 draft reply.** AI draft (or a template when no provider) put in the reply
  box; nothing is queued or sent until a person presses Send. Audited.
- **E4 payment comparison.** Every payment review carries expected vs detected,
  the difference, and reference / confidence / duplicate checks. It never settles.
- **F1 slash commands.** 46 commands with a palette: direct reads (no model),
  playbooks, pin/rename/new, `/price`, `/explain`, `/goto`, `/attach`, `/model`.
- **F5 keyboard.** Ctrl+K, `/`, `?`, Alt+N/I/B, Enter/Shift+Enter, ↑ recall, Tab
  complete, Esc; Ctrl+Enter / Ctrl+Backspace on a focused proposal card.
- **F6 till assistant.** The full assistant in a drawer from the till header, with
  the open cart as DATA context (optional). Till shortcuts pause while it is open.
  Cashiers need `ai.use` (not granted by default).
- Tests: `crates/amwapos-hub/tests/ai_workspace.rs` (12), unit tests in
  `ai_workspace.rs` and `ai_stream.rs`, e2e "AI assistant" in `e2e/checkout.spec.ts`.

## AI hardening, 2026-09-27

Evidence: `crates/amwapos-hub/tests/ai_hardening.rs` (12 tests), `ai_admin.rs`, `ai_byok.rs`, `ai_workspace.rs`, `src/components/__tests__/WaQr.test.tsx`. Everything here ran offline (test model or a loopback stub). No real provider, WhatsApp or Windows Hello was used.

| Item | Status | Evidence | Pending |
| --- | --- | --- | --- |
| Proposal tools follow permissions, not the role name | Complete | A custom role named like the accountant but holding `prices.manage` can propose prices. A read-only role under any name gets no `propose_*` tool. A buyer gets PO/reorder only. | — |
| Strict write intent | Complete | "Should I enable loyalty?" records nothing. `/price …`, `/reorder …` and "set price of SKU X to 1.500" record a proposal. | — |
| WhatsApp connect Confirm card shows the QR image | Complete | The same `WaQr` component as the WhatsApp page (vitest). The QR and pairing code are never stored or sent to the model. | Live pairing with a real phone |
| Undo (compensating command) | Complete | price / bulk price / cost, archive-restore, loyalty ±, credit ± (flag), delivery status, cancel an unsent WhatsApp message, device rename. Restore, role, flag, backup restore and WhatsApp logout stay irreversible and link to their page. | — |
| Permission upgrade seeds | Complete | New permissions are granted once to built-in roles whose defaults include them. A permission the owner removes is not added back. Cashier never gets `ai.use`, `ai.mutate`, `admin.access` or `orders.manage`. | — |
| Rider `orders.manage` | Decision | The Delivery role includes `orders.manage` so riders can move digital orders to "out for delivery". The owner can remove it in Users → Roles (OPERATIONS soak list). | Owner review |
| Features checklist | Complete | Settings → Features shows "Default: off" on every module and a count of modules on. The setup wizard says optional modules start off. No flag defaults on. | — |
| Two-person control (flag `ai.dual_control`, default off) | Complete | Off: one confirm. On: high-risk proposals need a second, different person. The same person, even with DATA saying "approve", is refused. | — |
| B3 alerts | Complete | Refund spike, discount spike, negative stock, silent tills / dead letters, backup overdue. Checked every 5 minutes while the app is open, with thresholds in AI settings. One inbox alert per check per day. No writes. | — |
| B4 reorder / B5 margin price | Complete | Suggestions are reads. `propose_reorder` / `propose_margin_price` only record proposals, and confirm runs `po.save` / the price command. | — |
| B8 branch compare | Complete | `ai.branch_compare` returns `enabled:false` when `org.multi_branch` is off. | Real multi-branch data |
| C6 customer redaction | Complete | Name, phone and address are replaced by ids in tool results before provider HTTP. The loopback stub never sees them. | — |
| A9 answer language / C1 fast model | Complete | Settings (`ui`/`en`/`ar`, default `ui`). The fast model is used for triage and drafts, and is empty by default. | Real-provider quality |
| Hub bind | Complete | Listens on the chosen card, else the first LAN address, else 127.0.0.1. Never 0.0.0.0. | Store Wi-Fi soak |
| Idempotency on new writes | Complete | `loyalty.adjust` takes `operation_id` (same key + different payload is refused). Order convert refuses a key used on another order. Transfers check per step. Proposal confirm is an atomic status change. | — |
| Export guards | Complete | EOD zip CSVs neutralise formulas (test). The companion token appears only on the Confirm card, never in proposals, conversation, list, audit or diagnostics (test). | — |
| Print widths | Complete | The test page is 384 dots at 58 mm and 576 dots at 80 mm, with the Arabic line, and also renders to PDF (test). | Physical printer |
