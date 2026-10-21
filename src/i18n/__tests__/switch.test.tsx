import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState, useSyncExternalStore } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getLang, setLang, setLangScope, subscribeLang, switchLang } from "..";

function CheckoutInProgress() {
  const locale = useSyncExternalStore(subscribeLang, getLang, getLang);
  const [scan, setScan] = useState("6281007031126");
  const [cart] = useState({ id: "cart-001", amountMinor: 4772 });
  return (
    <div>
      <span data-testid="locale">{locale}</span>
      <input aria-label="Scan" value={scan} onChange={(event) => setScan(event.target.value)} />
      <span data-testid="cart">
        {cart.id}:{cart.amountMinor}
      </span>
      <button onClick={() => switchLang(locale === "en" ? "ar" : "en")}>Switch language</button>
    </div>
  );
}

beforeEach(() => {
  localStorage.clear();
  setLangScope(null);
  setLang("en");
});

afterEach(() => {
  cleanup();
  setLangScope(null);
  setLang("en");
  localStorage.clear();
});

describe("instant per-user language selection", () => {
  it("switches language in place without destroying an unfinished scan or cart", () => {
    render(<CheckoutInProgress />);
    const scan = screen.getByRole("textbox", { name: "Scan" });
    fireEvent.change(scan, { target: { value: "6281007031126-partial" } });
    const oldNode = scan;
    fireEvent.click(screen.getByRole("button", { name: "Switch language" }));
    expect(screen.getByTestId("locale")).toHaveTextContent("ar");
    expect(document.documentElement.dir).toBe("rtl");
    expect(screen.getByRole("textbox", { name: "Scan" })).toBe(oldNode);
    expect(scan).toHaveValue("6281007031126-partial");
    expect(screen.getByTestId("cart")).toHaveTextContent("cart-001:4772");
    fireEvent.click(screen.getByRole("button", { name: "Switch language" }));
    expect(document.documentElement.dir).toBe("ltr");
    expect(scan).toHaveValue("6281007031126-partial");
  });

  it("keeps preferences separate for users, with a terminal-local fallback", () => {
    setLangScope("user-one");
    setLang("ar");
    expect(getLang()).toBe("ar");
    expect(localStorage.getItem("tillbh.lang.user.user-one")).toBe("ar");

    setLangScope("user-two");
    expect(getLang()).toBe("en");
    setLang("en");
    setLangScope("user-one");
    expect(getLang()).toBe("ar");
    setLangScope("user-two");
    expect(getLang()).toBe("en");

    setLangScope(null);
    setLang("ar"); // New users inherit this terminal default, not another user's override.
    setLangScope("user-three");
    expect(getLang()).toBe("ar");
  });
});
