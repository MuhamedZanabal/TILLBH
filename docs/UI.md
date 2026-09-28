# UI contract (1024×768, touch-first)

The primary cashier display is **1024×768 CSS pixels at 100% scaling**, driven by
touch; mouse, keyboard and a barcode scanner (keyboard wedge) still work. Admin
runs on the same panel.

Tokens live in `src/styles/tokens.css` (light and dark). Components use them;
no new colours in components.

## Tokens

| Token | Value / use |
| --- | --- |
| Look ("Amwaj" v2) | Calm paper surfaces (`--bg` with a soft teal glow), a night-ink chrome (`--brand-dark`), soft layered shadows (`--shadow-sm/md/lg`). |
| `--brand` (`--pos-accent`) | Deep sea teal `#08747e` (dark: `#3cc4cb`). The one accent. |
| `--pos-pay` / `--pos-pay-grad` | PAY only: emerald `#0c8448` (dark: `#2fbf68`). |
| `--sand` | Badges and counts on the dark chrome only. |
| `--danger`, `--warning`, `--success` | One each. |
| `--radius` / `--radius-card` / `--radius-modal` | 12 / 16 / 20 px. |
| `--pos-touch-min` / `--pos-touch-lg` | 48 / 60 px. |
| `--pos-top-h` / `--pos-dock-h` | 56 / 88 px. |
| `--money-figures` | `tabular-nums lining-nums` for every amount (`.money`, `.num`). |
| `--dur` / `--dur-press` | 170 ms / 60 ms; `prefers-reduced-motion` makes both 0. |

Compact density is the default (`data-density="compact"`): tighter rows, same
48 px hit targets.

## Rules

- Hit targets ≥ 48×48 px (small buttons extend their hit area with `::after`),
  8 px between neighbours; POS primary actions 56–64 px.
- Every control has default, pressed (≤ 80 ms), disabled and focus-visible
  states; hover is pointer-only (`@media (hover: hover)`), never the only way.
- Body 16 px, secondary 14 px, line height ≥ 1.25; POS totals 36 px; prices
  never wrap (`.money { white-space: nowrap }`).
- Logical properties only; `dir="rtl"` mirrors nav and the cart/search columns.
- One sheet at a time (payment, refund, shift close, More, AI sheets). Esc and a
  48 px ✕ close them.
- The scan field keeps the keyboard except while a text field or the AI
  composer has focus.

## Cashier layout

The till lists no catalogue: products are added by scanning or by typing in the
scan field (results drop down under it).

```
| 56  brand · shift · backup/sync/printer pills · Held · clock · Send · AI · More |
| scan / search field (56) ...................... | checkout column (340):      |
| cart lines (64 min): product tile · name ·      |  customer card (F3)         |
|   qty stepper · line total · remove             |  Hold · Sale discount · Refund |
|                                                 |  last sale (when empty)     |
|                                                 |  subtotal · discount · VAT · items |
|                                                 |  TOTAL (34)                 |
|                                                 |  PAY full width × 72        |
```

At 125 % / 150 % display size (or a window under 900 px) the checkout column
folds into the 88 px dock (sums · TOTAL · PAY 224×64) and Customer, Hold,
Sale discount and Refund move back beside the scan field. The Send rail and
the till assistant open over the cart, never over the checkout column.

## Display size

Settings → Appearance → Display size (90, 100, 110, 125, 150 %) zooms the whole
app on that computer (the WebView's own zoom; CSS zoom in a browser). At 125 %
a 1024×768 panel behaves as 819×614 CSS px and at 150 % as 683×512: the header
drops the business name and clock, the dock keeps TOTAL and PAY only below
760 px, and the AI page folds its side columns into sheets. The layout spec
checks the till and the payment sheet at both sizes.

PAY ends at least 16 px above the bottom edge (column and dock), so a 16 px
taskbar overlap never hides it. The till assistant opens over the search column, between the top bar and
the dock, and never covers PAY.

## Proof

`e2e/layout1024.spec.ts` runs these states at 1024×768 and saves screenshots
when `E2E_SHOTS=<dir>` is set: empty cart; 12 lines + low stock + held ticket;
payment (cash + change, and PAY/Confirm again at 1024×700); shift close; refund
step 2; AI empty / tools + thinking / high-risk proposal with diff; till
assistant over a 6-line cart; Arabic POS + assistant; dark compact; backup
overdue + hub "Update needed" with the admin banner and nav flyout.
