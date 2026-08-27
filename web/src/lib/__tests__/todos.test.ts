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
