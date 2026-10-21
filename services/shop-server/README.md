# PostgreSQL Shop Server authority foundation

**Status:** SCHEMA SOURCE IMPLEMENTED — POSTGRESQL RUNTIME NOT VERIFIED — NO SERVER BINARIES BUILT.

This is an isolated, additive architecture slice. The current TILLBH retail runtime still uses a SQLite shop hub. These migrations are not applied by any existing Tauri client and are not a replacement for the existing hub.

## Scope committed

- `migrations/0001_authority_core.sql`: tenant/branch/terminal/product identities, immutable sale totals and line snapshots, confirmed cash/electronic tenders, append-only stock movements, balanced GL, events, tenant-scoped RLS
- `migrations/0002_sale_transaction_guard.sql`: sale children inserted only in the same transaction as the sale; terminal/branch composite-identity check
- `tests/0001_authority_smoke.sql`: test fixture with BHD 4.488 − 0.150 + 0.434 = 4.772; verifies idempotency uniqueness, append-only sale history, and tenant row isolation
- `.github/workflows/postgres-authority.yml`: disposable PostgreSQL 16 service and unprivileged runtime-role smoke test

## Reproduce in a disposable PostgreSQL 15+ database

Do not run these commands against production.

```bash
psql -v ON_ERROR_STOP=1 -f services/shop-server/migrations/0001_authority_core.sql
psql -v ON_ERROR_STOP=1 -f services/shop-server/migrations/0002_sale_transaction_guard.sql
# Provision an unprivileged test role with USAGE on schema, table DML, and sequence usage.
psql -v ON_ERROR_STOP=1 -f services/shop-server/tests/0001_authority_smoke.sql
```

The smoke test explicitly rolls back its fixture rows. It does not roll back the schema migrations. Check the CI workflow for the complete disposable-role setup.

## Required checkout protocol (not yet implemented)

1. Authenticate device and staff **at the server**. Verify tenant, branch, role, terminal state and risk limits. Never accept arbitrary tenant IDs from the frontend as authority.
2. Determine the stable `operation_id` and compare the canonical SHA-256 payload to any previous operation with the same tenant and terminal. Same ID/different hash is a conflict.
3. In one PostgreSQL transaction, lock the idempotency and affected inventory resources; validate authoritative product prices, discounts, effective tax-policy versions and stock. Recompute all totals in integer fils.
4. Insert the sale, all lines, already-verified tenders, stock movements, balanced journal and outbox records. The deferred sale and GL constraints must pass before COMMIT.
5. Return the immutable committed sale snapshot only after the database acknowledges COMMIT. For a lost HTTP response, lookup/replay the previous result without a second write.
6. On reconnect, separately validate and ingest authorized, journaled offline cash sales. Resolve stale price/policy and stock conflicts explicitly; no silent overwrites.
7. Broadcast outbox events to terminals after commit. An event's delivery must not determine transaction durability.

## Security boundaries

RLS is defense in depth, **not authentication**. An application role that can set arbitrary `tillbh.tenant_id` values could select another tenant's data. Therefore never expose database credentials to terminals. The shop service must authenticate the caller, establish the authorized tenant context on every transaction, and use a least-privilege DB role with restricted SQL entrypoints. Migrations are applied using a distinct administrator role.

Do not enable provider-paid tenders based on OCR screenshots; the settlement ledger accepts only an authorized verification method with recorded provenance. The current schema does not implement such authorized adapters.

## Known gaps and release blockers

- No compiled or running Rust PostgreSQL Shop Server in this slice.
- No existing POS command, server handler, TLS device enrollment or synchronization route wired to PostgreSQL.
- No versioned product-price/tax-policy tables or server-side recalculation implementation yet.
- No authorization enforcement for `verifier_user_id` or database privilege setup for commercial use.
- No concurrency, negative-stock, power-loss, crash, backup/restore or Windows service smoke pass.
- No supplier receiving, refunds, partial receiving, customer-credit ledger, multi-branch transfer, periods, OCR, provider settlement, or full chart-of-accounts flows in this database slice.
- The current schema requires one sale-linked stock movement for every sale line; non-stock products require a future explicit tracking policy.
- Row-level-security policies require a tightly controlled authorized service and a non-superuser runtime role.

**Never advertise this schema as a fully functional PostgreSQL checkout server.**

## Rollback

No TILLBH production data is touched. Discard this branch to revert all source changes. For the **disposable** CI database only, destroy the database/container to reset it. Do not run a destructive `DROP SCHEMA` against production.
