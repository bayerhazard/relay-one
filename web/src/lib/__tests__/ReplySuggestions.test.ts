import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { fireEvent } from "@testing-library/svelte";
import ReplySuggestions from "$lib/components/ReplySuggestions.svelte";

describe("ReplySuggestions", () => {
  it("renders nothing when suggestions are empty", () => {
    const { container } = render(ReplySuggestions, {
      suggestions: [],
      onselect: vi.fn(),
    });
    expect(container.querySelector(".reply-suggestions")).toBeFalsy();
  });

  it("renders all suggestions", () => {
    render(ReplySuggestions, {
      suggestions: ["Danke!", "Wird erledigt.", "Ich melde mich."],
      onselect: vi.fn(),
    });
    expect(screen.getByText("Danke!")).toBeTruthy();
    expect(screen.getByText("Wird erledigt.")).toBeTruthy();
    expect(screen.getByText("Ich melde mich.")).toBeTruthy();
  });

  it("renders label", () => {
    render(ReplySuggestions, {
      suggestions: ["Test"],
      onselect: vi.fn(),
    });
    expect(screen.getByText("Antwortvorschläge:")).toBeTruthy();
  });

  it("calls onselect when suggestion is clicked", async () => {
    const onselect = vi.fn();
    render(ReplySuggestions, {
      suggestions: ["Option A", "Option B"],
      onselect,
    });

    await fireEvent.click(screen.getByText("Option A"));
    expect(onselect).toHaveBeenCalledWith("Option A");
    expect(onselect).toHaveBeenCalledTimes(1);
  });

  it("has suggestion-chip class", () => {
    const { container } = render(ReplySuggestions, {
      suggestions: ["Test"],
      onselect: vi.fn(),
    });
    const chips = container.querySelectorAll(".suggestion-chip");
    expect(chips.length).toBe(1);
  });
});
