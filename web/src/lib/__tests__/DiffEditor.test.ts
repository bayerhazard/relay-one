import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { fireEvent } from "@testing-library/svelte";
import DiffEditor from "$lib/components/DiffEditor.svelte";

describe("DiffEditor", () => {
  it("renders toolbar with title", () => {
    render(DiffEditor, {
      original: "hello",
      modified: "hello",
      onaccept: vi.fn(),
      onreject: vi.fn(),
    });
    expect(screen.getByText("KI-Vorschlag (Änderungen anzeigen)")).toBeTruthy();
  });

  it("renders accept and reject buttons", () => {
    render(DiffEditor, {
      original: "hello",
      modified: "hello",
      onaccept: vi.fn(),
      onreject: vi.fn(),
    });
    expect(screen.getByText("✓ Übernehmen")).toBeTruthy();
    expect(screen.getByText("✕ Ablehnen")).toBeTruthy();
  });

  it("calls onaccept when accept button is clicked", async () => {
    const onaccept = vi.fn();
    render(DiffEditor, {
      original: "hello",
      modified: "world",
      onaccept,
      onreject: vi.fn(),
    });

    await fireEvent.click(screen.getByText("✓ Übernehmen"));
    expect(onaccept).toHaveBeenCalledTimes(1);
  });

  it("calls onreject when reject button is clicked", async () => {
    const onreject = vi.fn();
    render(DiffEditor, {
      original: "hello",
      modified: "world",
      onaccept: vi.fn(),
      onreject,
    });

    await fireEvent.click(screen.getByText("✕ Ablehnen"));
    expect(onreject).toHaveBeenCalledTimes(1);
  });

  it("shows unchanged lines", () => {
    const { container } = render(DiffEditor, {
      original: "same line",
      modified: "same line",
      onaccept: vi.fn(),
      onreject: vi.fn(),
    });
    expect(container.querySelector(".diff-line")).toBeTruthy();
    expect(container.querySelector(".diff-line.added")).toBeFalsy();
    expect(container.querySelector(".diff-line.removed")).toBeFalsy();
  });

  it("shows added and removed lines for different text", () => {
    const { container } = render(DiffEditor, {
      original: "hello",
      modified: "world",
      onaccept: vi.fn(),
      onreject: vi.fn(),
    });
    expect(container.querySelector(".diff-line.removed")).toBeTruthy();
    expect(container.querySelector(".diff-line.added")).toBeTruthy();
  });

  it("has diff-editor class", () => {
    const { container } = render(DiffEditor, {
      original: "test",
      modified: "test",
      onaccept: vi.fn(),
      onreject: vi.fn(),
    });
    expect(container.querySelector(".diff-editor")).toBeTruthy();
  });
});
