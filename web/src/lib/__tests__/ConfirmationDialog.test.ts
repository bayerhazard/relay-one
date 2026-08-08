import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";

describe("ConfirmationDialog", () => {
  const defaultProps = {
    open: true,
    message: "Möchtest du fortfahren?",
    onconfirm: vi.fn(),
    oncancel: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders when open is true", () => {
    render(ConfirmationDialog, defaultProps);
    expect(screen.getByText("Möchtest du fortfahren?")).toBeTruthy();
  });

  it("does not render when open is false", () => {
    render(ConfirmationDialog, { ...defaultProps, open: false });
    expect(screen.queryByText("Möchtest du fortfahren?")).toBeNull();
  });

  it("calls onconfirm when confirm button is clicked", async () => {
    render(ConfirmationDialog, defaultProps);
    await fireEvent.click(screen.getByText("Bestätigen"));
    expect(defaultProps.onconfirm).toHaveBeenCalledOnce();
  });

  it("calls oncancel when cancel button is clicked", async () => {
    render(ConfirmationDialog, defaultProps);
    await fireEvent.click(screen.getByText("Abbrechen"));
    expect(defaultProps.oncancel).toHaveBeenCalledOnce();
  });

  it("calls oncancel when Escape is pressed", async () => {
    render(ConfirmationDialog, defaultProps);
    const overlay = screen.getByRole("alertdialog");
    await fireEvent.keyDown(overlay, { key: "Escape" });
    expect(defaultProps.oncancel).toHaveBeenCalledOnce();
  });

  it("calls onconfirm when Enter is pressed", async () => {
    render(ConfirmationDialog, defaultProps);
    const overlay = screen.getByRole("alertdialog");
    await fireEvent.keyDown(overlay, { key: "Enter" });
    expect(defaultProps.onconfirm).toHaveBeenCalledOnce();
  });

  it("shows custom title", () => {
    render(ConfirmationDialog, {
      ...defaultProps,
      title: "Löschen?",
      confirmLabel: "Löschen",
    });
    expect(screen.getByText("Löschen?")).toBeTruthy();
    expect(screen.getByText("Löschen")).toBeTruthy();
  });
});
