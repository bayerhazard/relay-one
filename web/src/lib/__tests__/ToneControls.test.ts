import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import ToneControls from "$lib/components/ToneControls.svelte";

describe("ToneControls", () => {
  it("renders both slider labels", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Seriosität")).toBeTruthy();
    expect(screen.getByText("Textumfang")).toBeTruthy();
  });

  it("renders end labels for seriositaet", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Locker")).toBeTruthy();
    expect(screen.getByText("Formell")).toBeTruthy();
  });

  it("renders end labels for textumfang", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Knapp")).toBeTruthy();
    expect(screen.getByText("Ausführlich")).toBeTruthy();
  });

  it("shows 'Ausgewogen' for seriositaet 4", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Ausgewogen")).toBeTruthy();
  });

  it("shows 'Locker' for seriositaet 2", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 2, textumfang: 4 },
      onchange: vi.fn(),
    });
    const labels = container.querySelectorAll(".slider-label");
    expect(labels[0]?.textContent).toBe("Locker");
  });

  it("shows 'Formell' for seriositaet 7", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 7, textumfang: 4 },
      onchange: vi.fn(),
    });
    const labels = container.querySelectorAll(".slider-label");
    expect(labels[0]?.textContent).toBe("Formell");
  });

  it("shows 'Normal' for textumfang 4", () => {
    render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(screen.getByText("Normal")).toBeTruthy();
  });

  it("shows 'Knapp' for textumfang 2", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 2 },
      onchange: vi.fn(),
    });
    const labels = container.querySelectorAll(".slider-label");
    expect(labels[1]?.textContent).toBe("Knapp");
  });

  it("shows 'Ausführlich' for textumfang 7", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 7 },
      onchange: vi.fn(),
    });
    const labels = container.querySelectorAll(".slider-label");
    expect(labels[1]?.textContent).toBe("Ausführlich");
  });

  it("has tone-controls class", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(container.querySelector(".tone-controls")).toBeTruthy();
  });

  it("has sliders-grid class", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    expect(container.querySelector(".sliders-grid")).toBeTruthy();
  });

  it("has two track elements", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    const tracks = container.querySelectorAll(".track");
    expect(tracks.length).toBe(2);
  });

  it("has ARIA slider roles", () => {
    const { container } = render(ToneControls, {
      values: { seriositaet: 4, textumfang: 4 },
      onchange: vi.fn(),
    });
    const sliders = container.querySelectorAll('[role="slider"]');
    expect(sliders.length).toBe(2);
  });

  // --- ARIA attributes ---

  describe("ARIA attributes", () => {
    it("seriositaet slider has correct ARIA attributes and tabindex", () => {
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 4 },
        onchange: vi.fn(),
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      expect(slider.getAttribute("role")).toBe("slider");
      expect(slider.getAttribute("aria-valuemin")).toBe("1");
      expect(slider.getAttribute("aria-valuemax")).toBe("7");
      expect(slider.getAttribute("aria-valuenow")).toBe("3");
      expect(slider.getAttribute("tabindex")).toBe("0");
    });

    it("textumfang slider has correct ARIA attributes and tabindex", () => {
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 5 },
        onchange: vi.fn(),
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      expect(slider.getAttribute("role")).toBe("slider");
      expect(slider.getAttribute("aria-valuemin")).toBe("1");
      expect(slider.getAttribute("aria-valuemax")).toBe("7");
      expect(slider.getAttribute("aria-valuenow")).toBe("5");
      expect(slider.getAttribute("tabindex")).toBe("0");
    });
  });

  // --- Keyboard interaction: seriositaet ---

  describe("keyboard interaction — seriositaet", () => {
    it("ArrowUp increments value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 4 });
    });

    it("ArrowDown decrements value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 2, textumfang: 4 });
    });

    it("ArrowRight increments value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 5, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowRight" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 6, textumfang: 4 });
    });

    it("ArrowLeft decrements value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 5, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowLeft" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 4 });
    });

    it("clamps minimum value at 1 on ArrowDown", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 1, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      // value stays 1 — onchange is still called with clamped value
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 1, textumfang: 4 });
    });

    it("clamps maximum value at 7 on ArrowUp", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 7, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 7, textumfang: 4 });
    });

    it("clamps minimum value at 1 on ArrowLeft", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 1, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowLeft" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 1, textumfang: 4 });
    });

    it("clamps maximum value at 7 on ArrowRight", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 7, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowRight" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 7, textumfang: 4 });
    });
  });

  // --- Keyboard interaction: textumfang ---

  describe("keyboard interaction — textumfang", () => {
    it("ArrowUp increments value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 3 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 4 });
    });

    it("ArrowDown decrements value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 3 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 2 });
    });

    it("ArrowRight increments value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 2 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowRight" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 3 });
    });

    it("ArrowLeft decrements value (verified via onchange)", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 6 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowLeft" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 5 });
    });

    it("clamps minimum value at 1 on ArrowDown", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 1 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 1 });
    });

    it("clamps maximum value at 7 on ArrowUp", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 7 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 7 });
    });
  });

  // --- onchange callback ---

  describe("onchange callback on keyboard interaction", () => {
    it("calls onchange when seriositaet increases via ArrowUp", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 4,
        textumfang: 4,
      });
    });

    it("calls onchange when seriositaet decreases via ArrowDown", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 5, textumfang: 2 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 4,
        textumfang: 2,
      });
    });

    it("calls onchange when textumfang increases via ArrowUp", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 2 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 3,
        textumfang: 3,
      });
    });

    it("calls onchange when textumfang decreases via ArrowDown", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 6 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 3,
        textumfang: 5,
      });
    });

    it("calls onchange when seriositaet changes via ArrowRight", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 2, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowRight" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 3,
        textumfang: 4,
      });
    });

    it("calls onchange when seriositaet changes via ArrowLeft", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 6, textumfang: 4 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowLeft" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 5,
        textumfang: 4,
      });
    });

    it("does not modify the other slider value when one changes", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 6 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      expect(onChange).toHaveBeenCalledWith({
        seriositaet: 4,
        textumfang: 6,
      });
    });
  });

  // --- Label updates ---

  describe("label updates correspond to keyboard value changes", () => {
    it("seriositaet value moves from 3 to 2 on ArrowDown — label shifts from 'Ausgewogen' to 'Locker'", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 3, textumfang: 4 },
        onchange: onChange,
      });
      // Initial label for value 3
      expect(screen.getByText("Ausgewogen")).toBeTruthy();
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      // Value changed to 2 — labelFor('seriositaet', 2) = 'Locker'
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 2, textumfang: 4 });
    });

    it("seriositaet value moves from 4 to 5 on ArrowUp — label shifts from 'Ausgewogen' to 'Formell'", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 4 },
        onchange: onChange,
      });
      // Initial label for value 4
      expect(screen.getByText("Ausgewogen")).toBeTruthy();
      const slider = screen.getByRole("slider", { name: "Seriosität" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      // Value changed to 5 — labelFor('seriositaet', 5) = 'Formell'
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 5, textumfang: 4 });
    });

    it("textumfang value moves from 4 to 5 on ArrowUp — label shifts from 'Normal' to 'Ausführlich'", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 4 },
        onchange: onChange,
      });
      // Initial label for value 4
      expect(screen.getByText("Normal")).toBeTruthy();
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowUp" });
      // Value changed to 5 — labelFor('textumfang', 5) = 'Ausführlich'
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 5 });
    });

    it("textumfang value moves from 5 to 4 on ArrowDown — label shifts from 'Ausführlich' to 'Normal'", () => {
      const onChange = vi.fn();
      render(ToneControls, {
        values: { seriositaet: 4, textumfang: 5 },
        onchange: onChange,
      });
      const slider = screen.getByRole("slider", { name: "Textumfang" });
      fireEvent.keyDown(slider, { key: "ArrowDown" });
      // Value changed to 4 — labelFor('textumfang', 4) = 'Normal'
      expect(onChange).toHaveBeenCalledWith({ seriositaet: 4, textumfang: 4 });
    });
  });
});
