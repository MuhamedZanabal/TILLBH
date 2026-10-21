# Locale preservation engineering slice — 2026-10-21

## Scope

This branch contains a reversible, narrow change to the existing TILLBH React interface. It does not replace the current storage or transaction architecture.

- Language changes happen in-place; no `location.reload()` is issued.
- The root React app subscribes to the external locale state, so screens render with the selected language without remounting the root providers.
- Admin navigation labels are evaluated at render time rather than at module-import time.
- Language preference is stored per local terminal user, with a terminal-local fallback.
- Login, session restore, logout and session replacement update the correct preference scope.
- Header language selector has dedicated 44 px targets and accessible state.
- Unit-level regression tests exercise preservation of an in-progress input/cart and preference isolation.

## Financial scope and required validation

The change is not permitted to mutate cart totals, inventory, payment intent, applied discounts or authorizations. The canonical tax-exclusive fixture remains: 4.488 BHD subtotal minus 0.150 BHD discount plus 0.434 BHD VAT = **4.772 BHD payable**. This is an acceptance example, **not** evidence of an executed checkout on this branch.

## Known gaps

The existing hub uses SQLite and not the PostgreSQL Shop Server required by the newer directive. This is a separate high-risk data-authority migration and must not be introduced through an unverified edit to payment-sensitive code.

The existing translation layer uses a static Arabic dictionary and `t()/tb()`, not the specified i18next/react-i18next. This work adds live rerendering and scope isolation, **not** a claim of i18next migration or exhaustive Arabic-only QA.

The desktop/Windows installer and multi-terminal hardware acceptance checks still require clean-machine and live-store verification. If CI does not run or fails, this branch is **not** release-verified.

## Local verification commands

```sh
npm ci
npm run typecheck
npm run lint
npm run format:check
npm test
npx vite build
npx playwright test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Windows installer acceptance additionally requires `npx tauri build --bundles nsis` on Windows and `./scripts/installer-smoke.ps1`, plus clean Windows hardware validation.

## Rollback

The original `main` branch is not modified. This feature can be reverted by discarding this branch. Do not migrate production databases or apply generated designs automatically.

## Test compatibility

The existing `data-testid="lang-toggle"` remains a one-click language switch; the cashier additionally exposes direct EN and AR selection in the header, and the cashier More menu continues to expose the prior language-switch path. This preserves the existing Playwright assertions while making the selector directly visible. Tests must still be executed before acceptance.
