# UI contract (1024×768, touch-first)

The primary cashier display is **1024×768 CSS pixels at 100% scaling**, driven by
touch; mouse, keyboard and a barcode scanner (keyboard wedge) still work. Admin
runs on the same panel.

Tokens live in `src/styles/tokens.css` (light and dark). Components use them;
no new colours in components.

## Tokens

| Token | Value / use |
| --- | --- |
| `--brand` (`--pos-accent`) | Deep teal `#0b6e79` (dark: `#3cc2cc`). The one accent. |
| `--pos-pay` | PAY only: green `#0f7a3e` (dark: `#2fbf68`). |
| `--danger`, `--warning`, `--success` | One each. |
| `--radius` / `--radius-card` | 10 / 12 px. |
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
| 56  brand · shift · backup/sync/printer pills · Held · clock · AI · More |
| 56  scan / search field ............ | Customer | Hold | Discount | Refund |
|     cart lines (56 min), full width                                       |
| 88  dock: subtotal · discount · VAT · items · TOTAL (36) · PAY 224×64    |
```

## Display size

Settings → Appearance → Display size (90, 100, 110, 125, 150 %) zooms the whole
app on that computer (the WebView's own zoom; CSS zoom in a browser). At 125 %
a 1024×768 panel behaves as 819×614 CSS px and at 150 % as 683×512: the header
drops the business name and clock, the dock keeps TOTAL and PAY only below
760 px, and the AI page folds its side columns into sheets. The layout spec
checks the till and the payment sheet at both sizes.

PAY sits in the top 72 px of the dock, so a 16 px taskbar overlap never hides
it. The till assistant opens over the search column, between the top bar and
the dock, and never covers PAY.

## Proof

`e2e/layout1024.spec.ts` runs these states at 1024×768 and saves screenshots
when `E2E_SHOTS=<dir>` is set: empty cart; 12 lines + low stock + held ticket;
payment (cash + change, and PAY/Confirm again at 1024×700); shift close; refund
step 2; AI empty / tools + thinking / high-risk proposal with diff; till
assistant over a 6-line cart; Arabic POS + assistant; dark compact; backup
overdue + hub "Update needed" with the admin banner and nav flyout.
