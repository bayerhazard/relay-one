import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import PromptDialog from "$lib/components/PromptDialog.svelte";

describe("PromptDialog", () => {
  const defaultProps = {
    open: true,
    title: "Eingabe",
    message: "Bitte Wert eingeben",
    onconfirm: vi.fn(),
    oncancel: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders when open is true", () => {
    render(PromptDialog, defaultProps);
    expect(screen.getByText("Eingabe")).toBeTruthy();
  });

  it("does not render when open is false", () => {
    render(PromptDialog, { ...defaultProps, open: false });
    expect(screen.queryByText("Eingabe")).toBeNull();
  });

  it("calls oncancel when Escape is pressed", async () => {
    render(PromptDialog, defaultProps);
    const overlay = screen.getByRole("dialog");
    await fireEvent.keyDown(overlay, { key: "Escape" });
    expect(defaultProps.oncancel).toHaveBeenCalledOnce();
  });

  it("calls onconfirm with trimmed value when Enter is pressed", async () => {
    render(PromptDialog, { ...defaultProps, value: "  test  " });
    const overlay = screen.getByRole("dialog");
    await fireEvent.keyDown(overlay, { key: "Enter" });
    expect(defaultProps.onconfirm).toHaveBeenCalledWith("test");
  });

  it("does not call onconfirm when input is empty", async () => {
    render(PromptDialog, { ...defaultProps, value: "" });
    const overlay = screen.getByRole("dialog");
    await fireEvent.keyDown(overlay, { key: "Enter" });
    expect(defaultProps.onconfirm).not.toHaveBeenCalled();
  });
});
