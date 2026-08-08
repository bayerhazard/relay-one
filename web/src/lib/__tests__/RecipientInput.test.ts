import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import RecipientInput from "$lib/components/RecipientInput.svelte";

vi.mock("$lib/services/tauri", () => ({
  searchContacts: vi.fn().mockResolvedValue([]),
}));

describe("RecipientInput", () => {
  const defaultProps = {
    value: [] as string[],
    accountId: 1,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders input field", () => {
    render(RecipientInput, defaultProps);
    expect(screen.getByPlaceholderText("Name oder E-Mail-Adresse")).toBeTruthy();
  });

  it("renders chips for each value", () => {
    render(RecipientInput, { ...defaultProps, value: ["test@example.com"] });
    expect(screen.getByText("test@example.com")).toBeTruthy();
  });

  it("has aria-label for accessibility", () => {
    render(RecipientInput, defaultProps);
    const input = screen.getByPlaceholderText("Name oder E-Mail-Adresse");
    expect(input.getAttribute("aria-label")).toBe("Empfänger eingeben");
  });
});
