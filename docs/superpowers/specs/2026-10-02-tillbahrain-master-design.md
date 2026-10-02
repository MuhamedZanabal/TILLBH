# Tillbahrain Master Design Authority

This repository implements **Tillbahrain**, a production-grade Bahrain-native, offline-first retail POS and store operating system for Windows.

## Binding product identity
- Product name: Tillbahrain
- Package name: tillbahrain
- Desktop identifier: com.tillbahrain.pos
- Initial production target: 1.0.0
- Windows 10/11 x64
- BHD, 3 decimal places, integer minor-unit money
- Asia/Bahrain
- +973 Bahrain phone normalization
- English primary; Arabic secondary with full RTL
- Retail only

## Binding engineering invariants
- Local SQLite is operational source of truth; external services are optional.
- Money is integer minor units. Quantities are exact decimal values. No f32/f64 source of truth.
- Completed financial history is immutable; corrections use compensating events.
- Checkout, refunds, PO receiving, loyalty and delivery financial mutations are atomic and idempotent.
- Authorization fails closed and actor/branch/device context is derived from authenticated state.
- Core retail remains usable without AI, WhatsApp, storefront, telemetry, license server or updater.
- Sales, stock, loyalty, cash custody, sync and audit histories must remain explainable after offline work and restart.
- No restaurant tables, waiter mode, KDS, kitchen tickets or restaurant workflows.

## Locked stack direction
- Tauri v2, Rust 2021 (Rust >= 1.82), React 19, TypeScript, Vite.
- SQLite with sqlx 0.8 is the target persistence stack.
- WhatsApp target architecture is an isolated Node >=20 loopback sidecar on 127.0.0.1:3131 with a per-start random token; only /health is unauthenticated.
- LAN hub target port is 8923 and mDNS service is _tillbahrain-hub._tcp.local.
- Credential-store service target is tillbahrain.
- Local database target name is tillbahrain.db.

## Migration rule
Existing valid business data and credentials are authoritative. Any rename of ProgramData paths, database files, credential namespaces, sync identity, receipt sequences or secrets must provide tested compatibility/migration before the old namespace stops being read.

## Release truth
A screen is not a complete feature. Required behavior needs persistence/domain logic, authorization, failure handling, tests and integration verification. Status is VERIFIED COMPLETE only after the required automated gates and clean Windows install acceptance are evidenced on the exact build commit.

The user's “TILLBAHRAIN — MASTER DEVELOPMENT DIRECTIVE” dated 2026-10-02 is the detailed source authority for feature scope and acceptance criteria. This file records the repository-level architectural constraints needed by implementation plans.
