import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import FraudWarning from "$lib/components/FraudWarning.svelte";

describe("FraudWarning", () => {
  it("renders nothing when score is below threshold", () => {
    const { container } = render(FraudWarning, {
      score: 0.5,
      warnings: ["Test warning"],
    });
    expect(container.querySelector(".fraud-warning")).toBeFalsy();
  });

  it("renders warning when score exceeds threshold", () => {
    render(FraudWarning, {
      score: 0.7,
      warnings: ["Verdächtiger Absender"],
    });
    expect(screen.getByText("Phishing-Verdacht")).toBeTruthy();
  });

  it("displays warnings list", () => {
    render(FraudWarning, {
      score: 0.8,
      warnings: ["Urgency detected", "Suspicious link"],
    });
    expect(screen.getByText("Urgency detected")).toBeTruthy();
    expect(screen.getByText("Suspicious link")).toBeTruthy();
  });

  it("renders without warnings", () => {
    render(FraudWarning, {
      score: 0.9,
      warnings: [],
    });
    expect(screen.getByText("Phishing-Verdacht")).toBeTruthy();
    const list = screen.queryByRole("list");
    expect(list).toBeFalsy();
  });

  it("has fraud-warning class", () => {
    const { container } = render(FraudWarning, {
      score: 0.8,
      warnings: ["Test"],
    });
    expect(container.querySelector(".fraud-warning")).toBeTruthy();
  });
});
