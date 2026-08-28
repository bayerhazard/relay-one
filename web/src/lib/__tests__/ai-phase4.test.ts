import { describe, it, expect, vi, afterEach } from "vitest";
import {
  nlCreate, smartSchedule, meetingPrep, agendaDigest, askAssistant,
} from "$lib/services/tauri";

function mockFetchOnce(status: number, body: unknown): void {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
    }),
  );
}

describe("Phase 4 AI services", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("nlCreate POSTs /ai/nl-create", async () => {
    mockFetchOnce(200, { type: "event", title: "Kaffee", start: "2026-09-02T14:00:00Z", end: null, attendees: [], description: null, due: null });
    const res = await nlCreate("Morgen 14 Uhr Kaffee", "Kalender");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/nl-create");
    expect(opts?.method).toBe("POST");
    expect(res.type).toBe("event");
    expect(res.title).toBe("Kaffee");
  });

  it("smartSchedule returns the suggestions array", async () => {
    mockFetchOnce(200, { suggestions: [{ start: "a", end: "b", confidence: 0.9, reason: "frei" }] });
    const res = await smartSchedule("60 Min", "anna@x.com");
    const [url] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/schedule");
    expect(res).toHaveLength(1);
    expect(res[0].confidence).toBe(0.9);
  });

  it("meetingPrep POSTs /ai/meeting-prep", async () => {
    mockFetchOnce(200, { attendees: ["Anna"], agenda: ["Budget"], prep_notes: "Vorbereiten" });
    const res = await meetingPrep("Q3-Review", "2026-09-01T10:00:00Z", []);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/meeting-prep");
    expect(JSON.parse(opts?.body as string)).toEqual({ summary: "Q3-Review", start: "2026-09-01T10:00:00Z", attendees: [] });
    expect(res.agenda).toEqual(["Budget"]);
  });

  it("agendaDigest POSTs /ai/agenda-digest", async () => {
    mockFetchOnce(200, { digest: "Heute 3 Termine", priorities: ["Q3"], followups: ["Antwort"] });
    const res = await agendaDigest(undefined, 7);
    const [url] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/agenda-digest");
    expect(res.priorities).toEqual(["Q3"]);
  });

  it("askAssistant returns reply + actions", async () => {
    mockFetchOnce(200, { reply: "Klar", actions: [{ type: "event_create", payload: { summary: "X" } }] });
    const res = await askAssistant("Plan einen Termin");
    const [url] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/assistant");
    expect(res.reply).toBe("Klar");
    expect(res.actions[0].type).toBe("event_create");
  });
});
