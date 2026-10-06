// @vitest-environment happy-dom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import Page from "../../routes/settings/+page.svelte";

vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/settings") } }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));

// The settings inside the shell (Etappe 6, RL-G1, RL-G2): the sections are
// rows in the shell's column, the chosen section heads the page, the
// profile photo is gone.

describe("settings page in the shell", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => [] }),
    );
  });
  afterEach(() => { vi.unstubAllGlobals(); });

  it("lists the eight sections in the column, the chosen one current", () => {
    render(Page);
    const spalte = document.getElementById("relay-spalte")!;
    const zeilen = spalte.querySelectorAll("button.einst-eintrag");
    expect(zeilen).toHaveLength(8);
    const aktuell = spalte.querySelectorAll('[aria-current="page"]');
    expect(aktuell).toHaveLength(1);
    expect(aktuell[0].textContent).toContain("Allgemein");
  });

  it("heads the page with the chosen section's title and switches it", async () => {
    render(Page);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Allgemeine Einstellungen");

    const spalte = document.getElementById("relay-spalte")!;
    const kontakte = [...spalte.querySelectorAll("button")].find((b) => b.textContent?.includes("Kontakte"))!;
    await fireEvent.click(kontakte);
    await waitFor(() => expect(kontakte.getAttribute("aria-current")).toBe("page"));
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(document.title).toContain("Kontakte · Relay");
  });

  it("closes the column sheet after a choice (phone)", async () => {
    render(Page);
    const knopf = document.querySelector<HTMLButtonElement>(".relay-spalte-knopf")!;
    await fireEvent.click(knopf);
    expect(knopf.getAttribute("aria-expanded")).toBe("true");
    const spalte = document.getElementById("relay-spalte")!;
    const cache = [...spalte.querySelectorAll("button")].find((b) => b.textContent?.includes("Cache"))!;
    await fireEvent.click(cache);
    await waitFor(() => expect(knopf.getAttribute("aria-expanded")).toBe("false"));
    expect(cache.getAttribute("aria-current")).toBe("page");
  });

  it("shows no profile photo (RL-G1)", async () => {
    render(Page);
    const spalte = document.getElementById("relay-spalte")!;
    const kontakte = [...spalte.querySelectorAll("button")].find((b) => b.textContent?.includes("Kontakte"))!;
    await fireEvent.click(kontakte);
    expect(document.body.textContent).not.toContain("Profilbild");
    expect(document.querySelector(".photo-preview")).toBeNull();
  });

  it("has no search in the header and no own back-to-mail logo", () => {
    render(Page);
    expect(screen.queryByRole("searchbox")).toBeNull();
    expect(document.querySelector(".ml-logo-btn, .settings-sidebar")).toBeNull();
  });
});
