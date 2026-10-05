export interface SavedDeliveryAddress {
  address_id: string;
  label: string;
  area: string | null;
  address: string;
  notes: string | null;
  is_default: boolean;
}

export interface DeliveryAddressChoice {
  address: string;
  area: string;
  saved_address_id: string | null;
  label: string | null;
  notes: string | null;
}

export function chooseDeliveryAddress(
  current: { address: string; area: string },
  saved: SavedDeliveryAddress[],
): DeliveryAddressChoice {
  if (current.address.trim() || current.area.trim()) {
    return {
      address: current.address,
      area: current.area,
      saved_address_id: null,
      label: null,
      notes: null,
    };
  }
  const selected = saved.find((a) => a.is_default) ?? saved[0];
  if (!selected) {
    return { address: "", area: "", saved_address_id: null, label: null, notes: null };
  }
  return {
    address: selected.address,
    area: selected.area ?? "",
    saved_address_id: selected.address_id,
    label: selected.label,
    notes: selected.notes,
  };
}

export function repeatableSaleLines(
  items: { product_id: string | null; name: string; qty_milli: number; refunded_qty_milli: number }[],
): { product_id: string | null; description: string; qty_milli: number }[] {
  return items
    .map((item) => ({
      product_id: item.product_id,
      description: item.name,
      qty_milli: Math.max(0, item.qty_milli - item.refunded_qty_milli),
    }))
    .filter((item) => item.qty_milli > 0);
}

function formatBhd(minor: number): string {
  return (minor / 1000).toFixed(3);
}

export function deliveryCollectionLabel(paymentStatus: string, amountMinor: number): string {
  if (paymentStatus === "paid") return "Paid";
  if (paymentStatus === "cod") return `Collect ${formatBhd(amountMinor)}`;
  return "Payment pending";
}
