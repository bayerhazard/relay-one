import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import PlanCard from "$lib/components/PlanCard.svelte";
import type { AgentPlan } from "$lib/services/tauri";

function makePlan(overrides: Partial<AgentPlan> = {}): AgentPlan {
  return {
    id: "plan-1",
    session_id: "sess-1",
    origin: "calendar",
    status: "pending",
    steps: [
      {
        tool: "calendar.create",
        tier: "write",
        title: "Termin anlegen",
        rows: [["Wann", "Mo 14:00"]],
        request: { method: "POST", path: "/events", body: {} },
        danach: "/calendar?date={id}",
      },
    ],
    created_at: "2026-09-07T10:00:00Z",
    expires_at: "2026-09-07T11:00:00Z",
    executed_at: null,
    result_json: null,
    ...overrides,
  };
}

describe("PlanCard — pending (write tier)", () => {
  it("zeigt Schritt-Anzahl, Status und Aktions-Buttons", () => {
    render(PlanCard, { plan: makePlan() });
    expect(screen.getByText("1 Schritt")).toBeTruthy();
    expect(screen.getByText("Ausstehend")).toBeTruthy();
    expect(screen.getByText("Ausführen")).toBeTruthy();
    expect(screen.getByText("Verwerfen")).toBeTruthy();
  });

  it("ruft onConfirm(plan, false) beim Ausführen auf", async () => {
    const onConfirm = vi.fn();
    const plan = makePlan();
    render(PlanCard, { plan, onConfirm });
    await fireEvent.click(screen.getByText("Ausführen"));
    expect(onConfirm).toHaveBeenCalledWith(plan, false);
  });

  it("ruft onDiscard(plan) beim Verwerfen auf", async () => {
    const onDiscard = vi.fn();
    const plan = makePlan();
    render(PlanCard, { plan, onDiscard });
    await fireEvent.click(screen.getByText("Verwerfen"));
    expect(onDiscard).toHaveBeenCalledWith(plan);
  });

  it("zeigt Plural bei mehreren Schritten", () => {
    const plan = makePlan({
      steps: [
        { tool: "a", tier: "write", title: "A", rows: [], request: { method: "POST", path: "/a", body: {} }, danach: "" },
        { tool: "b", tier: "write", title: "B", rows: [], request: { method: "POST", path: "/b", body: {} }, danach: "" },
      ],
    });
    render(PlanCard, { plan });
    expect(screen.getByText("2 Schritte")).toBeTruthy();
  });
});

describe("PlanCard — external tier (zweistufig)", () => {
  function externalPlan(): AgentPlan {
    return makePlan({
      steps: [
        { tool: "calendar.invite", tier: "external", title: "Einladung senden", rows: [["An", "anna@x.com"]], request: { method: "POST", path: "/invite", body: {} }, danach: "" },
      ],
    });
  }

  it("zeigt die Extern-Warnung und nur den Arme-Button", () => {
    render(PlanCard, { plan: externalPlan() });
    expect(screen.getByText(/kann nicht zurückgenommen/)).toBeTruthy();
    expect(screen.getByText("Ausführen")).toBeTruthy();
    expect(screen.queryByText("Endgültig senden")).toBeNull();
  });

  it("schaltet nach dem Armen auf „Endgültig senden“ um und bestätigt mit true", async () => {
    const onConfirm = vi.fn();
    render(PlanCard, { plan: externalPlan(), onConfirm });
    await fireEvent.click(screen.getByText("Ausführen"));
    const confirmBtn = screen.getByText("Endgültig senden");
    await fireEvent.click(confirmBtn);
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm.mock.calls[0][1]).toBe(true);
  });
});

describe("PlanCard — executed", () => {
  it("zeigt Status Erledigt, Öffnen (mit id-Substitution) und Rückgängig", async () => {
    const onOpen = vi.fn();
    const onUndo = vi.fn();
    const plan = makePlan({
      status: "executed",
      executed_at: "2026-09-07T10:05:00Z",
      result_json: JSON.stringify([{ id: "evt-42" }]),
    });
    render(PlanCard, { plan, onOpen, onUndo });
    expect(screen.getByText("Erledigt")).toBeTruthy();
    const openBtn = screen.getByText("Öffnen");
    await fireEvent.click(openBtn);
    expect(onOpen).toHaveBeenCalledWith("/calendar?date=evt-42");
    await fireEvent.click(screen.getByText("Rückgängig"));
    expect(onUndo).toHaveBeenCalledWith(plan);
  });

  it("zeigt keinen Öffnen-Button ohne onOpen-Callback", () => {
    const plan = makePlan({
      status: "executed",
      result_json: JSON.stringify([{ id: "evt-42" }]),
    });
    render(PlanCard, { plan });
    expect(screen.queryByText("Öffnen")).toBeNull();
    expect(screen.getByText("Rückgängig")).toBeTruthy();
  });
});

describe("PlanCard — terminal states ohne Aktionen", () => {
  it("cancelled: Status Verworfen, keine Aktions-Buttons", () => {
    render(PlanCard, { plan: makePlan({ status: "cancelled" }) });
    expect(screen.getByText("Verworfen")).toBeTruthy();
    expect(screen.queryByText("Ausführen")).toBeNull();
    expect(screen.queryByText("Rückgängig")).toBeNull();
  });

  it("expired: Status Abgelaufen, keine Aktions-Buttons", () => {
    render(PlanCard, { plan: makePlan({ status: "expired" }) });
    expect(screen.getByText("Abgelaufen")).toBeTruthy();
    expect(screen.queryByText("Ausführen")).toBeNull();
  });

  it("failed: Status Fehlgeschlagen, keine Aktions-Buttons", () => {
    render(PlanCard, { plan: makePlan({ status: "failed" }) });
    expect(screen.getByText("Fehlgeschlagen")).toBeTruthy();
    expect(screen.queryByText("Ausführen")).toBeNull();
  });
});
