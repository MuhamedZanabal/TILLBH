import { describe, expect, it } from "vitest";
import { capabilityText, itemStatusText } from "../waCatalog";

describe("WhatsApp catalogue wording", () => {
  it("never presents an unsupported account as ready", () => {
    expect(capabilityText("supported")).toContain("with a catalogue");
    expect(capabilityText("personal")).toContain("need WhatsApp Business");
    expect(capabilityText("business_no_catalog")).toContain("could not be read");
    expect(capabilityText("disconnected")).toContain("not connected");
  });

  it("names every product state", () => {
    expect(itemStatusText("synced")).toBe("On WhatsApp");
    expect(itemStatusText("failed")).toBe("Could not publish");
    expect(itemStatusText("queued")).toBe(itemStatusText("syncing"));
    expect(itemStatusText(null)).toBe("Not on WhatsApp");
  });
});
