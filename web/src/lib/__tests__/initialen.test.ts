import { describe, expect, it } from "vitest";
import { initialen, olaresNutzerAusHost } from "$lib/initialen";

describe("profile circle", () => {
  it("takes two letters from a name", () => {
    expect(initialen("Kai Böhm")).toBe("KB");
    expect(initialen("kaivostudio")).toBe("KA");
    expect(initialen("")).toBe("…");
  });

  it("finds the Olares user in the app's address", () => {
    expect(olaresNutzerAusHost("31747cb8.kaivostudio.olares.de")).toBe("kaivostudio");
    expect(olaresNutzerAusHost("mail.aimighty.olares.com")).toBe("aimighty");
    expect(olaresNutzerAusHost("127.0.0.1")).toBe("");
    expect(olaresNutzerAusHost("relay.example.org")).toBe("");
  });
});
