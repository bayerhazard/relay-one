import { describe, it, expect, beforeEach } from "vitest";
import {
  followupFingerprint,
  isFollowupDone,
  markFollowupDone,
  clearFollowupMemory,
} from "$lib/utils/followupMemory";
import type { FollowupAction, FollowupSuggestion, AgentPlanStep } from "$lib/services/tauri";

const card: AgentPlanStep = {
  tool: "tasks_create",
  tier: "write",
  title: "Aufgabe anlegen",
  rows: [["Was", "Rückruf"]],
  request: { method: "POST", path: "/tasks", body: { summary: "Rückruf" } },
  danach: "/tasks",
};

function suggestion(tool: string, title: string): FollowupSuggestion {
  return { id: "fu-1", titel: title, tool, args: { summary: title }, plan: { ...card, tool, title } };
}

describe("followupMemory (Erinnerung an ausgeführte Vorschläge)", () => {
  beforeEach(() => {
    clearFollowupMemory();
    localStorage.clear();
  });

  it("marks a v2 suggestion as done and reports it back", () => {
    const s = suggestion("tasks_create", "Rückruf");
    expect(isFollowupDone(42, s)).toBe(false);
    markFollowupDone(42, s);
    expect(isFollowupDone(42, s)).toBe(true);
  });

  it("scopes done-state per message UID", () => {
    const s = suggestion("tasks_create", "Rückruf");
    markFollowupDone(1, s);
    expect(isFollowupDone(1, s)).toBe(true);
    expect(isFollowupDone(2, s)).toBe(false);
  });

  it("derives the same fingerprint for equal suggestions (stable across regen)", () => {
    const a = suggestion("tasks_create", "Rückruf");
    const b = suggestion("tasks_create", "Rückruf");
    expect(followupFingerprint(a)).toBe(followupFingerprint(b));
  });

  it("treats different actions as different fingerprints", () => {
    const a = suggestion("tasks_create", "Rückruf");
    const b = suggestion("calendar_confirm", "Termin bestätigen");
    expect(followupFingerprint(a)).not.toBe(followupFingerprint(b));
  });

  it("fingerprint is content-based and ignores whitespace/case", () => {
    const a = suggestion("tasks_create", "  Rückruf ");
    const b = suggestion("tasks_create", "rückruf");
    expect(followupFingerprint(a)).toBe(followupFingerprint(b));
  });

  it("legacy FollowupAction fingerprints keep working", () => {
    const action: FollowupAction = {
      id: "fu-2",
      kind: "task",
      label: "Rückruf",
      task: { summary: "Rückruf", due: null },
    };
    const other: FollowupAction = {
      id: "fu-3",
      kind: "task",
      label: "Rückruf",
      task: { summary: "Etwas anderes", due: null },
    };
    markFollowupDone(42, action);
    expect(isFollowupDone(42, action)).toBe(true);
    expect(isFollowupDone(42, other)).toBe(false);
  });
});