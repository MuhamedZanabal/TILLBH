# Tillbahrain Foundation & Release Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the existing TILLBH codebase into a safe Tillbahrain foundation without losing business data, then establish evidence-producing release gates.

**Architecture:** Preserve the working retail domain and progressively replace incompatible infrastructure behind stable domain interfaces. Branding/state migrations are split so external identity can change before persistent state paths or credentials move. High-risk architectural migrations (rusqlite→sqlx, in-process WhatsApp→loopback sidecar, current LAN transport→8923+mDNS contract) each get their own compatibility tests before cutover.

**Tech Stack:** Tauri v2, Rust 2021 >=1.82, React 19, TypeScript, Vite, SQLite; target sqlx 0.8; Node >=20 WhatsApp sidecar.

**Spec:** `docs/superpowers/specs/2026-10-02-tillbahrain-master-design.md`

## Global Constraints
- Product: Tillbahrain; package: tillbahrain; identifier: com.tillbahrain.pos; version: 1.0.0.
- Currency BHD with integer minor units; timezone Asia/Bahrain; Bahrain phones +973.
- Retail only.
- Existing business data, receipt identity, credentials and audit history must not be stranded by namespace changes.
- Core POS must remain offline-capable and independent of optional integrations.
- Authorization fails closed.

## Review Focus
- Existing TILLBH installation upgraded to Tillbahrain must retain database and credentials.
- Duplicate finalize/retry must never create a second sale/payment/stock movement.
- Optional integration failure must degrade without blocking selling.
- Offline terminal reconciliation must not treat derived stock/loyalty caches as authority.
- Windows clean-install must prove the installed binary, database, restart persistence and no-printer degradation.

---

### Task 1: External product identity contract
- [ ] Set package/product/version/identifier/Rust floor and installed executable identity.
- [ ] Add `scripts/check_product_identity.py` and CI gate.
- [ ] Verify on GitHub Actions and Windows build.

### Task 2: Persistent namespace migration
- [ ] Test and implement non-destructive DB/data-dir and credential migration to Tillbahrain names.
- [ ] Verify restart, upgrade, backup and restore.

### Task 3: Persistence stack migration
- [ ] Pin current DB facade parity tests.
- [ ] Introduce sqlx 0.8 behind the facade and port incrementally.
- [ ] Remove rusqlite only after full parity/upgrade tests pass.

### Task 4: WhatsApp isolation
- [ ] Contract-test and implement Node >=20 loopback sidecar on 127.0.0.1:3131.
- [ ] Migrate current messaging behind the sidecar boundary without blocking POS on failure.

### Task 5: Customer, delivery and rider acceptance
- [ ] Preserve PR #3 correctness hardening.
- [ ] Reconcile PR #4 UX without weakening identity/payment rules.
- [ ] Add acceptance coverage for saved addresses, repeat order, timeline, flows A–E and rider custody.

### Task 6: LAN hub and sync contract
- [ ] Move to port 8923 + _tillbahrain-hub._tcp.local atomically with firewall/runtime tests.
- [ ] Prove revoked-device status codes, parity, DLQ, and two-terminal offline convergence.

### Task 7: Optional systems and CI gates
- [ ] Implement remaining storefront/Cloudflare, backup crypto, advisory license, updater, AI policy, intelligence and diagnostics requirements.
- [ ] Add invoke-contract, a11y, dead-CSS, nextest, gitleaks/static-policy and release-gate jobs.

### Task 8: Windows release evidence
- [ ] Build exact green head.
- [ ] Run clean Windows 10/11 acceptance.
- [ ] Record installer filename, bytes, SHA-256, exact commit, OS and result.
