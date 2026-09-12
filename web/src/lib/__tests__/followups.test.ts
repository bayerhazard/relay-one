import { describe, it, expect, vi, afterEach } from "vitest";
import {
  getFollowups,
  getMeetingFollowups,
  createPlanFromSuggestion,
  parseCachedFollowups,
  type FollowupSuggestion,
  type AgentPlanStep,
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

const card: AgentPlanStep = {
  tool: "tasks_create",
  tier: "write",
  title: "Aufgabe anlegen",
  rows: [["Was", "Rückruf"]],
  request: { method: "POST", path: "/tasks", body: { summary: "Rückruf" } },
  danach: "/tasks",
};

const planFixture = {
  id: "plan-1",
  session_id: null,
  origin: "mail_followup",
  status: "pending",
  steps: [card],
  created_at: "2026-09-07T00:00:00Z",
  expires_at: "2026-09-07T01:00:00Z",
  executed_at: null,
  result_json: null,
};

describe("followups v2 service (Phase C)", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("getFollowups POSTs /ai/followups and returns the actions object", async () => {
    mockFetchOnce(200, {
      actions: [
        { id: "fu-1", titel: "Rückruf", tool: "tasks_create", args: { summary: "Rückruf" }, plan: card },
      ],
    });
    const res = await getFollowups("Q3-Budget", "chef@example.com", "bitte bis Freitag");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/followups");
    expect(opts?.method).toBe("POST");
    const body = JSON.parse(opts?.body as string);
    expect(body).toEqual({
      subject: "Q3-Budget",
      from: "chef@example.com",
      body: "bitte bis Freitag",
      account_id: null,
      uid: null,
      folder: null,
    });
    expect(res.actions).toHaveLength(1);
    expect(res.actions[0].tool).toBe("tasks_create");
    expect(res.actions[0].plan.tier).toBe("write");
  });

  it("getFollowups returns an empty actions list when the AI finds nothing", async () => {
    mockFetchOnce(200, { actions: [] });
    const res = await getFollowups("Hallo", "a@b.c", "text");
    expect(res.actions).toEqual([]);
  });

  it("createPlanFromSuggestion POSTs /ai/plans and returns the plan", async () => {
    const suggestion: FollowupSuggestion = {
      id: "fu-1",
      titel: "Rückruf",
      tool: "tasks_create",
      args: { summary: "Rückruf" },
      plan: card,
    };
    const plan = {
      id: "plan-1",
      session_id: null,
      origin: "mail_followup",
      status: "pending",
      steps: [card],
      created_at: "2026-09-07T00:00:00Z",
      expires_at: "2026-09-07T01:00:00Z",
      executed_at: null,
      result_json: null,
    };
    mockFetchOnce(200, plan);
    const res = await createPlanFromSuggestion(suggestion, { sourceMessageId: 42 });
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/plans");
    expect(opts?.method).toBe("POST");
    const body = JSON.parse(opts?.body as string);
    expect(body).toEqual({
      tool: "tasks_create",
      args: { summary: "Rückruf" },
      source_message_id: 42,
      locale: null,
      origin: null,
    });
    expect(res.origin).toBe("mail_followup");
    expect(res.status).toBe("pending");
  });

  it("createPlanFromSuggestion forwards the meeting_followup origin", async () => {
    const suggestion: FollowupSuggestion = {
      id: "fu-1",
      titel: "Vorlage senden",
      tool: "tasks_create",
      args: { summary: "Vorlage senden" },
      plan: card,
    };
    mockFetchOnce(200, { ...planFixture, origin: "meeting_followup" });
    const res = await createPlanFromSuggestion(suggestion, {
      sourceMessageId: 7,
      origin: "meeting_followup",
    });
    const body = JSON.parse(vi.mocked(fetch).mock.calls[0][1]?.body as string);
    expect(body.source_message_id).toBe(7);
    expect(body.origin).toBe("meeting_followup");
    expect(res.origin).toBe("meeting_followup");
  });

  it("getMeetingFollowups POSTs /ai/meetings/followups with the meeting id", async () => {
    mockFetchOnce(200, { actions: [] });
    const res = await getMeetingFollowups(7);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/meetings/followups");
    expect(opts?.method).toBe("POST");
    expect(JSON.parse(opts?.body as string)).toEqual({ meeting_id: 7 });
    expect(res.actions).toEqual([]);
  });
});

describe("parseCachedFollowups (instant footer, 26.9.145)", () => {
  const action: FollowupSuggestion = {
    id: "fu-1",
    titel: "Rückruf",
    tool: "tasks_create",
    args: { summary: "Rückruf" },
    plan: card,
  };

  it("parses the v2 object shape {actions:[...]} stored by the server pre-gen", () => {
    const raw = JSON.stringify({ actions: [action] });
    const res = parseCachedFollowups(raw);
    expect(res).toHaveLength(1);
    expect(res?.[0].id).toBe("fu-1");
  });

  it("tolerates a bare array", () => {
    const res = parseCachedFollowups(JSON.stringify([action]));
    expect(res).toHaveLength(1);
    expect(res?.[0].tool).toBe("tasks_create");
  });

  it("returns null for an empty actions list (falls back to on-demand)", () => {
    expect(parseCachedFollowups(JSON.stringify({ actions: [] }))).toBeNull();
    expect(parseCachedFollowups("[]")).toBeNull();
  });

  it("returns null for absent input", () => {
    expect(parseCachedFollowups(null)).toBeNull();
    expect(parseCachedFollowups(undefined)).toBeNull();
    expect(parseCachedFollowups("")).toBeNull();
  });

  it("returns null for malformed JSON (falls back to on-demand)", () => {
    expect(parseCachedFollowups("{not json")).toBeNull();
    expect(parseCachedFollowups("42")).toBeNull();
    expect(parseCachedFollowups(JSON.stringify({ actions: "nope" }))).toBeNull();
  });
});
