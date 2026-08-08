import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import PriorityBadge from "$lib/components/PriorityBadge.svelte";

describe("PriorityBadge", () => {
  it("renders nothing when intensity is below threshold", () => {
    const { container } = render(PriorityBadge, { intensity: 0.5 });
    expect(container.querySelector(".priority-dot")).toBeFalsy();
  });

  it("renders nothing at exactly threshold", () => {
    const { container } = render(PriorityBadge, { intensity: 0.7 });
    expect(container.querySelector(".priority-dot")).toBeFalsy();
  });

  it("renders dot when intensity exceeds threshold", () => {
    render(PriorityBadge, { intensity: 0.8 });
    expect(screen.getByTitle("Hohe Dringlichkeit")).toBeTruthy();
  });

  it("renders at max intensity", () => {
    render(PriorityBadge, { intensity: 1.0 });
    expect(screen.getByTitle("Hohe Dringlichkeit")).toBeTruthy();
  });

  it("suppresses dot when fraud score >= 0.3", () => {
    const { container } = render(PriorityBadge, { intensity: 0.9, fraudScore: 0.3 });
    expect(container.querySelector(".priority-dot")).toBeFalsy();
  });

  it("shows dot when fraud score is below threshold", () => {
    render(PriorityBadge, { intensity: 0.9, fraudScore: 0.2 });
    expect(screen.getByTitle("Hohe Dringlichkeit")).toBeTruthy();
  });

  it("suppresses dot when fraud score is high regardless of intensity", () => {
    const { container } = render(PriorityBadge, { intensity: 1.0, fraudScore: 0.5 });
    expect(container.querySelector(".priority-dot")).toBeFalsy();
  });
});
