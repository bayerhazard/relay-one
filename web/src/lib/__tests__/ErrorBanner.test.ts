import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import ErrorBanner from "$lib/components/ErrorBanner.svelte";

describe("ErrorBanner", () => {
  it("renders error message", () => {
    render(ErrorBanner, { message: "Testfehler" });
    expect(screen.getByText("Testfehler")).toBeTruthy();
  });

  it("has role alert", () => {
    render(ErrorBanner, { message: "Testfehler" });
    expect(screen.getByRole("alert")).toBeTruthy();
  });

  it("renders retry button when onretry provided", async () => {
    const onretry = vi.fn();
    render(ErrorBanner, { message: "Fehler", onretry, retryLabel: "Wiederholen" });
    const btn = screen.getByText("Wiederholen");
    await fireEvent.click(btn);
    expect(onretry).toHaveBeenCalledOnce();
  });

  it("does not render retry button without onretry", () => {
    render(ErrorBanner, { message: "Fehler" });
    expect(screen.queryByText("Erneut versuchen")).toBeNull();
  });
});
