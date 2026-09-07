import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { effects, applyEffect, type Effect, type EffectContext } from "$lib/stores/effects";
import { calendarView } from "$lib/stores/calendarView";
import { selection } from "$lib/stores/selection";

describe("effects queue (Concept §5.5)", () => {
  beforeEach(() => effects.clear());

  it("push/shift consume effects sequentially (FIFO)", () => {
    effects.push({ kind: "navigate", module: "calendar" });
    effects.push({ kind: "navigate", module: "tasks" });
    expect(get(effects)).toHaveLength(2);
    expect(effects.shift()).toEqual({ kind: "navigate", module: "calendar" });
    expect(effects.shift()).toEqual({ kind: "navigate", module: "tasks" });
    expect(effects.shift()).toBeNull();
  });

  it("pushMany appends a batch to the back of the queue", () => {
    effects.push({ kind: "navigate", module: "mail" });
    effects.pushMany([
      { kind: "calendar.set_view", view: "day", date: "2026-09-08" },
      { kind: "highlight", art: "event", id: "ev-1" },
    ]);
    expect(get(effects)).toHaveLength(3);
    expect(effects.shift()?.kind).toBe("navigate");
    expect(effects.shift()?.kind).toBe("calendar.set_view");
    expect(effects.shift()?.kind).toBe("highlight");
  });

  it("clear empties the queue", () => {
    effects.push({ kind: "navigate", module: "settings" });
    effects.clear();
    expect(get(effects)).toHaveLength(0);
    expect(effects.shift()).toBeNull();
  });
});

describe("calendarView store (Concept §10.2)", () => {
  it("setView changes mode and keeps the date when none is given", () => {
    const before = get(calendarView).viewDate;
    calendarView.setView("week");
    const s = get(calendarView);
    expect(s.viewMode).toBe("week");
    expect(s.viewDate.getTime()).toBe(before.getTime());
  });

  it("setView with a date updates both fields", () => {
    const d = new Date("2026-09-08T00:00:00");
    calendarView.setView("day", d);
    const s = get(calendarView);
    expect(s.viewMode).toBe("day");
    expect(s.viewDate.getTime()).toBe(d.getTime());
  });

  it("setDate changes only the date", () => {
    const d = new Date("2026-10-01T00:00:00");
    calendarView.setDate(d);
    expect(get(calendarView).viewDate.getTime()).toBe(d.getTime());
  });
});

describe("selection store (Concept §10.2)", () => {
  beforeEach(() => selection.reset());

  it("setMail stores the selection and sets highlight", () => {
    selection.setMail({ uid: 7, folder: "INBOX", account: 1 }, true);
    const s = get(selection);
    expect(s.mail).toEqual({ uid: 7, folder: "INBOX", account: 1 });
    expect(s.highlight).toBe(true);
  });

  it("setContact / setTask / setEvent each target their own field", () => {
    selection.setContact("c-1", true);
    selection.setTask("t-1", true);
    selection.setEvent("e-1", true);
    const s = get(selection);
    expect(s.contact).toBe("c-1");
    expect(s.task).toBe("t-1");
    expect(s.event).toBe("e-1");
  });

  it("clearHighlight drops the highlight flag only", () => {
    selection.setTask("t-1", true);
    selection.clearHighlight();
    const s = get(selection);
    expect(s.highlight).toBe(false);
    expect(s.task).toBe("t-1");
  });

  it("reset clears all selections", () => {
    selection.setMail({ uid: 1, folder: "INBOX", account: 0 }, true);
    selection.reset();
    const s = get(selection);
    expect(s.mail).toBeNull();
    expect(s.contact).toBeNull();
    expect(s.task).toBeNull();
    expect(s.event).toBeNull();
    expect(s.highlight).toBe(false);
  });
});

describe("effect router (applyEffect, Concept §5.5 / §12.5)", () => {
  function mockCtx() {
    const calls: string[] = [];
    const gotoArgs: string[] = [];
    const ctx: EffectContext = {
      goto: (url) => {
        calls.push("goto");
        gotoArgs.push(url);
      },
      setView: (view) => calls.push(`setView:${view}`),
      setMail: () => calls.push("setMail"),
      setContact: (id) => calls.push(`setContact:${id}`),
      setTask: (id) => calls.push(`setTask:${id}`),
      setEvent: (id) => calls.push(`setEvent:${id}`),
      openCompose: () => calls.push("openCompose"),
    };
    return { ctx, calls, gotoArgs };
  }

  it("navigate routes to the module (mail maps to /)", () => {
    const { ctx, gotoArgs } = mockCtx();
    applyEffect({ kind: "navigate", module: "calendar" }, ctx);
    applyEffect({ kind: "navigate", module: "mail" }, ctx);
    expect(gotoArgs).toEqual(["/calendar", "/"]);
  });

  it("calendar.set_view sets the view and navigates to /calendar", () => {
    const { ctx, calls, gotoArgs } = mockCtx();
    applyEffect({ kind: "calendar.set_view", view: "day", date: "2026-09-08" }, ctx);
    expect(calls).toContain("setView:day");
    expect(gotoArgs).toEqual(["/calendar"]);
  });

  it("contacts.open selects the contact and navigates", () => {
    const { ctx, calls, gotoArgs } = mockCtx();
    applyEffect({ kind: "contacts.open", uid: "c-9" }, ctx);
    expect(calls).toContain("setContact:c-9");
    expect(gotoArgs).toEqual(["/contacts"]);
  });

  it("ignores an unknown effect kind (version skew) without throwing", () => {
    const { ctx, calls, gotoArgs } = mockCtx();
    const bogus = { kind: "selfdestruct", payload: "rm -rf" } as unknown as Effect;
    expect(() => applyEffect(bogus, ctx)).not.toThrow();
    expect(calls).toHaveLength(0);
    expect(gotoArgs).toHaveLength(0);
  });

  it("ignores a malformed effect (missing kind) safely", () => {
    const { ctx, calls } = mockCtx();
    const malformed = { kind: undefined } as unknown as Effect;
    expect(() => applyEffect(malformed, ctx)).not.toThrow();
    expect(calls).toHaveLength(0);
  });

  it("highlight mail is a no-op when no mail is selected", () => {
    const { ctx, calls } = mockCtx(); // selectedMail is undefined here
    applyEffect({ kind: "highlight", art: "mail", id: "123" }, ctx);
    expect(calls).toHaveLength(0);
  });

  it("highlight contact selects the contact", () => {
    const { ctx, calls } = mockCtx();
    applyEffect({ kind: "highlight", art: "contact", id: "c-42" }, ctx);
    expect(calls).toContain("setContact:c-42");
  });
});
