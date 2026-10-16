import { beforeEach, describe, expect, it } from "vitest";
import { migrateLegacyBrowserStorage, readMigratedLocalStorage } from "../legacyStorage";

describe("legacy browser storage compatibility", () => {
  beforeEach(() => localStorage.clear());

  it("copies a legacy value only when the TILLBH key is absent", () => {
    localStorage.setItem("amwapos.lang", "ar");
    expect(readMigratedLocalStorage("tillbh.lang", "amwapos.lang")).toBe("ar");
    expect(localStorage.getItem("tillbh.lang")).toBe("ar");
    expect(localStorage.getItem("amwapos.lang")).toBe("ar");
  });

  it("never overwrites a current TILLBH value", () => {
    localStorage.setItem("tillbh.lang", "en");
    localStorage.setItem("amwapos.lang", "ar");
    expect(readMigratedLocalStorage("tillbh.lang", "amwapos.lang")).toBe("en");
    expect(localStorage.getItem("tillbh.lang")).toBe("en");
  });

  it("migrates benign preferences but not auth or setup secrets", () => {
    localStorage.setItem("amwapos.lang", "ar");
    localStorage.setItem("amwapos.ai.lang", "en");
    localStorage.setItem("amwapos.setup.draft", JSON.stringify({ pair_code: "12345678" }));
    sessionStorage.setItem("amwapos.session", "legacy-session-token");

    migrateLegacyBrowserStorage();

    expect(localStorage.getItem("tillbh.lang")).toBe("ar");
    expect(localStorage.getItem("tillbh.ai.lang")).toBe("en");
    expect(localStorage.getItem("tillbh.setup.draft")).toBeNull();
    expect(sessionStorage.getItem("tillbh.session")).toBeNull();
  });
});
