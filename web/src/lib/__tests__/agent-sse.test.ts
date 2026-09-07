import { describe, it, expect, vi, afterEach } from "vitest";
import { get } from "svelte/store";
import { runAgent } from "$lib/services/tauri";
import type { AgentEvent, AgentPlan, AgentResult } from "$lib/services/tauri";
import { dataVersion, bumpDataVersion } from "$lib/stores/invalidation";

function sseStream(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const c of chunks) controller.enqueue(encoder.encode(c));
      controller.close();
    },
  });
}

function okSse(body: ReadableStream<Uint8Array>): void {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, status: 200, body }));
}

const plan: AgentPlan = {
  id: "p1",
  session_id: "s1",
  origin: "calendar",
  status: "pending",
  steps: [
    { tool: "calendar.create", tier: "write", title: "Termin", rows: [], request: { method: "POST", path: "/events", body: {} }, danach: "" },
  ],
  created_at: "2026-09-07T10:00:00Z",
  expires_at: "2026-09-07T11:00:00Z",
  executed_at: null,
  result_json: null,
};

const doneResult: AgentResult = {
  answer: "Klar, Termin steht.",
  plans: [plan],
  navigation: null,
  effects: [],
  steps: [{ tool: "calendar.list", label: "Kalender geprüft" }],
  session_id: "s1",
};

describe("runAgent — SSE-Parser", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("dispatcht status/plan/effect/done in Reihenfolge und löst mit dem Ergebnis auf", async () => {
    const frames = [
      'event: status\ndata: {"step":1,"label":"Kalender prüfe"}\n\n',
      `event: plan\ndata: ${JSON.stringify(plan)}\n\n`,
      'event: effect\ndata: {"effect":"calendar.set_view","view":"day","date":"2026-09-08"}\n\n',
      `event: done\ndata: ${JSON.stringify(doneResult)}\n\n`,
    ];
    okSse(sseStream(frames));
    const events: AgentEvent[] = [];
    const res = await runAgent("Plan einen Termin", {}, (ev) => events.push(ev));
    expect(res).toStrictEqual(doneResult);
    expect(events.map((e) => e.type)).toEqual(["status", "plan", "effect", "done"]);
    expect(events[0]).toEqual({ type: "status", step: "1", label: "Kalender prüfe" });
    expect((events[1] as { plan: AgentPlan }).plan.id).toBe("p1");
    expect((events[2] as { effect: Record<string, unknown> }).effect.view).toBe("day");
  });

  it("ignoriert Keep-Alive-Kommentarzeilen", async () => {
    const frames = [
      ": ping\n\n",
      `event: done\ndata: ${JSON.stringify(doneResult)}\n\n`,
    ];
    okSse(sseStream(frames));
    const events: AgentEvent[] = [];
    const res = await runAgent("hi", {}, (ev) => events.push(ev));
    expect(res).toStrictEqual(doneResult);
    expect(events.map((e) => e.type)).toEqual(["done"]);
  });

  it("wirft bei Stream ohne done-Event", async () => {
    const frames = ['event: status\ndata: {"step":1,"label":"…"}\n\n'];
    okSse(sseStream(frames));
    await expect(runAgent("hi", {}, () => {})).rejects.toThrow(/ohne Ergebnis/);
  });

  it("wirft eine Fehlermeldung mit Hinweis bei 409 llm_not_configured", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: false,
      status: 409,
      json: async () => ({ error: "llm_not_configured", hinweis: "Adresse und Modell unter Einstellungen" }),
    }));
    await expect(runAgent("hi", {}, () => {})).rejects.toThrow(/llm_not_configured.*Einstellungen/s);
  });

  it("wirft den Fehler eines error-Events im Stream", async () => {
    const frames = ['event: error\ndata: {"error":"Modell ist nicht erreichbar"}\n\n'];
    okSse(sseStream(frames));
    await expect(runAgent("hi", {}, () => {})).rejects.toThrow(/Modell ist nicht erreichbar/);
  });

  it("gibt einen Abort weiter", async () => {
    const controller = new AbortController();
    controller.abort();
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new DOMException("Aborted", "AbortError")));
    await expect(runAgent("hi", {}, () => {}, controller.signal)).rejects.toBeInstanceOf(DOMException);
  });
});

describe("invalidation store", () => {
  it("start bei 0 und erhöht sich bei bumpDataVersion", () => {
    const before = get(dataVersion);
    bumpDataVersion();
    expect(get(dataVersion)).toBe(before + 1);
    bumpDataVersion();
    expect(get(dataVersion)).toBe(before + 2);
  });
});
