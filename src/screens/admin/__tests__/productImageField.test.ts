import { describe, expect, it } from "vitest";
import { IMAGE_MAX_BYTES, autoStatusLabel, discoveryNote, readImageFile } from "../productImageField";

describe("product picture upload checks", () => {
  it("refuses types other than PNG, JPEG, WebP and GIF", async () => {
    const f = new File(["<svg/>"], "x.svg", { type: "image/svg+xml" });
    await expect(readImageFile(f)).rejects.toThrow("PNG, JPEG, WebP or GIF");
  });

  it("refuses files over 8 MB before reading them", async () => {
    const f = new File([new Uint8Array(IMAGE_MAX_BYTES + 1)], "big.png", { type: "image/png" });
    await expect(readImageFile(f)).rejects.toThrow("8 MB");
  });

  it("reads an accepted file as a data URL", async () => {
    const f = new File([new Uint8Array([137, 80, 78, 71])], "a.png", { type: "image/png" });
    await expect(readImageFile(f)).resolves.toMatch(/^data:image\/png;base64,/);
  });

  it("labels every automatic-search state", () => {
    for (const s of ["not_attempted", "pending", "processing", "found", "not_found", "failed", "skipped"] as const) {
      expect(autoStatusLabel(s)).toBeTruthy();
    }
    expect(autoStatusLabel("found")).toBe("Found automatically");
  });

  it("explains every reason automatic pictures are not running, and nothing when they are", () => {
    expect(discoveryNote("active")).toBeNull();
    expect(discoveryNote("switched_off")).toContain("switched off");
    expect(discoveryNote("disabled_by_administrator")).toContain("TILLBH_IMAGE_SEARCH=off");
    expect(discoveryNote("no_sources")).toContain("no source");
  });
});
