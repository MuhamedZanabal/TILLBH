import { describe, expect, it } from "vitest";
import { chooseDeliveryAddress, deliveryCollectionLabel, repeatableSaleLines } from "../serviceFlow";

describe("service flow helpers", () => {
  it("uses the default saved address when delivery has no typed address", () => {
    const saved = [
      { address_id: "work", label: "Work", area: "Seef", address: "Office 8", notes: null, is_default: false },
      {
        address_id: "home",
        label: "Home",
        area: "Amwaj",
        address: "Bldg 10, Road 20",
        notes: "Blue gate",
        is_default: true,
      },
    ];
    expect(chooseDeliveryAddress({ address: "", area: "" }, saved)).toEqual({
      address: "Bldg 10, Road 20",
      area: "Amwaj",
      saved_address_id: "home",
      label: "Home",
      notes: "Blue gate",
    });
  });

  it("does not overwrite delivery address text already entered by a person", () => {
    const saved = [
      { address_id: "home", label: "Home", area: "Amwaj", address: "Saved", notes: null, is_default: true },
    ];
    expect(chooseDeliveryAddress({ address: "Typed address", area: "Hidd" }, saved)).toEqual({
      address: "Typed address",
      area: "Hidd",
      saved_address_id: null,
      label: null,
      notes: null,
    });
  });

  it("repeats only the quantity that was not refunded", () => {
    expect(
      repeatableSaleLines([
        { product_id: "p1", name: "Milk", qty_milli: 2000, refunded_qty_milli: 500 },
        { product_id: "p2", name: "Bread", qty_milli: 1000, refunded_qty_milli: 1000 },
        { product_id: null, name: "Custom item", qty_milli: 1000, refunded_qty_milli: 0 },
      ]),
    ).toEqual([
      { product_id: "p1", description: "Milk", qty_milli: 1500 },
      { product_id: null, description: "Custom item", qty_milli: 1000 },
    ]);
  });

  it("makes the rider's cash collection amount explicit", () => {
    expect(deliveryCollectionLabel("cod", 7250)).toBe("Collect 7.250");
    expect(deliveryCollectionLabel("paid", 7250)).toBe("Paid");
    expect(deliveryCollectionLabel("pending", 7250)).toBe("Payment pending");
  });
});
