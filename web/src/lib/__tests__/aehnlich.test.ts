import { describe, expect, it } from "vitest";
import { adresseVon, aehnlichAnfrage, betreffKern, domainVon, istAehnlichSuche } from "$lib/aehnlich";

describe("similar mails", () => {
  it("reads the address and the domain", () => {
    expect(adresseVon("PayPal <Service@PayPal.de>")).toBe("service@paypal.de");
    expect(adresseVon("service@paypal.de")).toBe("service@paypal.de");
    expect(adresseVon("Nur ein Name")).toBeNull();
    expect(domainVon("PayPal <service@paypal.de>")).toBe("paypal.de");
  });

  it("keeps the stable start of a subject", () => {
    expect(betreffKern("Ihr Einkauf bei Travelscape, Ltd.")).toBe("Ihr Einkauf bei Travelscape");
    expect(betreffKern("Rechnung 2026-0412")).toBe("Rechnung");
    expect(betreffKern("AW: Re: Termin nächste Woche")).toBe("Termin nächste Woche");
    expect(betreffKern("Reminder to keep your domain contacts up to date")).toBe("Reminder to keep your");
    expect(betreffKern("Hi")).toBeNull();
    expect(betreffKern("12345")).toBeNull();
  });

  it("builds the search for each way", () => {
    const mail = { from: "PayPal <service@paypal.de>", subject: "Ihr Einkauf bei Travelscape, Ltd." };
    expect(aehnlichAnfrage("absender", mail)).toBe("von:service@paypal.de");
    expect(aehnlichAnfrage("domain", mail)).toBe("domain:paypal.de");
    expect(aehnlichAnfrage("betreff", mail)).toBe('betreff:"Ihr Einkauf bei Travelscape"');
    expect(aehnlichAnfrage("betreff", { subject: "OK" })).toBeNull();
    expect(istAehnlichSuche('betreff:"Ihr Einkauf"')).toBe(true);
    expect(istAehnlichSuche("Rechnung von Jonas")).toBe(false);
  });
});
