import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import SummaryLine from "$lib/components/SummaryLine.svelte";

describe("SummaryLine", () => {
  it("renders summary text", () => {
    render(SummaryLine, { summary: "Dies ist eine Zusammenfassung" });
    expect(screen.getByText("Dies ist eine Zusammenfassung")).toBeTruthy();
  });

  it("has summary-line class", () => {
    const { container } = render(SummaryLine, { summary: "Test" });
    const element = container.querySelector(".summary-line");
    expect(element).toBeTruthy();
  });

  it("handles empty summary", () => {
    render(SummaryLine, { summary: "" });
    const element = screen.getByRole("generic");
    expect(element).toBeTruthy();
  });
});
