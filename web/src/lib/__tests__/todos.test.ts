import { describe, it, expect, vi, afterEach } from "vitest";
import {
  listTodos, createTodo, toggleTodo, deleteTodo, syncTodos,
  type TodoInput,
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

const input: TodoInput = { summary: "Einkaufen", due: "2026-09-01", priority: 3 };

describe("todos service", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("listTodos(open) calls GET /todos?completed=false", async () => {
    mockFetchOnce(200, []);
    await listTodos(false);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos?completed=false");
    expect(opts?.method).toBe("GET");
  });

  it("listTodos(all) omits the query", async () => {
    mockFetchOnce(200, []);
    await listTodos();
    const [url] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos");
    expect(String(url)).not.toContain("completed=");
  });

  it("createTodo POSTs the input", async () => {
    mockFetchOnce(200, { id: 1, uid: "u1", summary: "Einkaufen" });
    await createTodo(input);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos");
    expect(opts?.method).toBe("POST");
    expect(JSON.parse(opts?.body as string)).toEqual(input);
  });

  it("toggleTodo PATCHes /todos/:uid", async () => {
    mockFetchOnce(200, { id: 1, uid: "u1", status: "COMPLETED" });
    await toggleTodo("u1", true);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/u1");
    expect(opts?.method).toBe("PATCH");
    expect(JSON.parse(opts?.body as string)).toEqual({ completed: true });
  });

  it("deleteTodo DELETEs /todos/:uid", async () => {
    mockFetchOnce(200, { deleted: true });
    await deleteTodo("u1");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/u1");
    expect(opts?.method).toBe("DELETE");
  });

  it("syncTodos POSTs /todos/sync", async () => {
    mockFetchOnce(200, { synced: 5 });
    const res = await syncTodos();
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/sync");
    expect(opts?.method).toBe("POST");
    expect(res.synced).toBe(5);
  });
});

// ─── Task-module extensions (26.9.163) ───────────────────────────────

import {
  quickAddTodo, patchTodo, succeedTodo, reorderTodos, todoViews,
  type TodoPatchInput,
} from "$lib/services/tauri";

describe("todos service — extended API", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("quickAddTodo POSTs text + project_id", async () => {
    mockFetchOnce(200, { id: 1, uid: "u1", summary: "Budget" });
    await quickAddTodo("Freitag Budget p1", 7);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/quick-add");
    expect(opts?.method).toBe("POST");
    expect(JSON.parse(opts?.body as string)).toEqual({ text: "Freitag Budget p1", project_id: 7 });
  });

  it("quickAddTodo sends null project when none given", async () => {
    mockFetchOnce(200, {});
    await quickAddTodo("Heute anrufen");
    const [, opts] = vi.mocked(fetch).mock.calls[0];
    expect(JSON.parse(opts?.body as string).project_id).toBeNull();
  });

  it("patchTodo PATCHes a partial field set", async () => {
    mockFetchOnce(200, { uid: "u1" });
    const patch: TodoPatchInput = { priority: 1, labels: ["Firma"], project_id: null };
    await patchTodo("u1", patch);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/u1");
    expect(opts?.method).toBe("PATCH");
    expect(JSON.parse(opts?.body as string)).toEqual(patch);
  });

  it("succeedTodo POSTs to /todos/:uid/succeed", async () => {
    mockFetchOnce(200, { uid: "next" });
    await succeedTodo("u1");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/u1/succeed");
    expect(opts?.method).toBe("POST");
  });

  it("reorderTodos POSTs the uid list", async () => {
    mockFetchOnce(200, { reordered: 2 });
    const r = await reorderTodos(["a", "b"]);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/reorder");
    expect(JSON.parse(opts?.body as string)).toEqual({ uids: ["a", "b"] });
    expect(r.reordered).toBe(2);
  });

  it("todoViews GETs /todos/views", async () => {
    mockFetchOnce(200, { inbox: 1, today: 2, upcoming: 3, overdue: 0, done: 4, all: 10 });
    const v = await todoViews();
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/todos/views");
    expect(opts?.method).toBe("GET");
    expect(v.today).toBe(2);
  });
});
