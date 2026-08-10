import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import ToneControls from "$lib/components/ToneControls.svelte";

describe("ToneControls", () => {
  it("renders both control rows", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Tonfall")).toBeTruthy();
    expect(screen.getByText("Textumfang")).toBeTruthy();
  });

  it("renders all four option buttons", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Locker")).toBeTruthy();
    expect(screen.getByText("Formell")).toBeTruthy();
    expect(screen.getByText("Knapp")).toBeTruthy();
    expect(screen.getByText("Ausführlich")).toBeTruthy();
  });

  it("does not render intermediate labels (Ausgewogen/Normal)", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.queryByText("Ausgewogen")).toBeNull();
    expect(screen.queryByText("Normal")).toBeNull();
  });

  it("highlights 'Formell' when seriositaet >= 4", () => {
    render(ToneControls, {
      values: { seriositaet: 6, textumfang: 2 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Formell").classList.contains("active")).toBe(true);
    expect(screen.getByText("Locker").classList.contains("active")).toBe(false);
  });

  it("highlights 'Locker' when seriositaet < 4", () => {
    render(ToneControls, {
      values: { seriositaet: 2, textumfang: 6 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Locker").classList.contains("active")).toBe(true);
    expect(screen.getByText("Formell").classList.contains("active")).toBe(false);
  });

  it("highlights 'Ausführlich' when textumfang >= 4", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 6 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Ausführlich").classList.contains("active")).toBe(true);
    expect(screen.getByText("Knapp").classList.contains("active")).toBe(false);
  });

  it("clicking 'Locker' sets seriositaet to 2", () => {
    const onChange = vi.fn();
    render(ToneControls, {
      values: { seriositaet: 6, textumfang: 4 },
      onchange: onChange,
    });
    fireEvent.click(screen.getByText("Locker"));
    expect(onChange).toHaveBeenCalledWith({ seriositaet: 2, textumfang: 4 });
  });

  it("clicking 'Formell' sets seriositaet to 6", () => {
    const onChange = vi.fn();
    render(ToneControls, {
      values: { seriositaet: 2, textumfang: 4 },
      onchange: onChange,
    });
    fireEvent.click(screen.getByText("Formell"));
    expect(onChange).toHaveBeenCalledWith({ seriositaet: 6, textumfang: 4 });
  });

  it("clicking 'Knapp' sets textumfang to 2", () => {
    const onChange = vi.fn();
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 6 },
      onchange: onChange,
    });
    fireEvent.click(screen.getByText("Knapp"));
    expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 2 });
  });

  it("clicking 'Ausführlich' sets textumfang to 6", () => {
    const onChange = vi.fn();
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 2 },
      onchange: onChange,
    });
    fireEvent.click(screen.getByText("Ausführlich"));
    expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 6 });
  });

  it("has tone-controls class", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(container.querySelector(".tone-controls")).toBeTruthy();
  });

  it("has two segmented groups", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    const groups = container.querySelectorAll(".segmented");
    expect(groups.length).toBe(2);
  });

  it("has no slider tracks anymore", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(container.querySelectorAll(".track").length).toBe(0);
    expect(container.querySelectorAll('[role="slider"]').length).toBe(0);
  });
});
