<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listTodos, createTodo, toggleTodo, deleteTodo, syncTodos,
    type TodoInfo, type TodoInput,
  } from "$lib/services/tauri";

  type Filter = "all" | "open" | "done";

  let todos = $state<TodoInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let filter = $state<Filter>("open");
  let busy = $state(false);
  let syncing = $state(false);
  let syncMsg = $state<string | null>(null);

  let editorOpen = $state(false);
  let form = $state({ summary: "", description: "", due: "", priority: 5 as number });

  const completedMap: Record<Filter, boolean | undefined> = {
    all: undefined, open: false, done: true,
  };

  async function loadTodos() {
    loading = true;
    error = null;
    try {
      todos = await listTodos(completedMap[filter]);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function setFilter(f: Filter) {
    filter = f;
    loadTodos();
  }

  onMount(() => { loadTodos(); });

  function openCreate() {
    form = { summary: "", description: "", due: "", priority: 5 };
    editorOpen = true;
  }

  async function saveTodo() {
    if (!form.summary.trim()) return;
    busy = true;
    error = null;
    try {
      await createTodo({
        summary: form.summary,
        description: form.description || undefined,
        due: form.due || undefined,
        priority: form.priority,
      });
      editorOpen = false;
      await loadTodos();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function onToggle(t: TodoInfo) {
    const done = t.status === "COMPLETED";
    try {
      await toggleTodo(t.uid, !done);
      await loadTodos();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function removeTodo(t: TodoInfo) {
    const name = t.summary || "diese Aufgabe";
    if (!confirm(`Aufgabe „${name}" wirklich löschen?`)) return;
    try {
      await deleteTodo(t.uid);
      await loadTodos();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function onSync() {
    syncing = true;
    syncMsg = null;
    try {
      const r = await syncTodos();
      syncMsg = `${r.synced} Aufgaben synchronisiert.`;
      await loadTodos();
    } catch (e: unknown) {
      syncMsg = e instanceof Error ? e.message : String(e);
    } finally {
      syncing = false;
    }
  }

  function dueLabel(t: TodoInfo): string {
    if (!t.due_at) return "";
    const d = new Date(t.due_at);
    if (isNaN(d.getTime())) return t.due_at;
    return d.toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit", year: "numeric" });
  }

  function isOverdue(t: TodoInfo): boolean {
    if (!t.due_at || t.status === "COMPLETED") return false;
    const d = new Date(t.due_at);
    return !isNaN(d.getTime()) && d.getTime() < Date.now();
  }
</script>

<div class="tk-app">
  <aside class="tk-sidebar">
    <div class="tk-sidebar-header">
      <button type="button" class="tk-back" onclick={() => goto("/")} title="Zurück zur Post">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5M12 19l-7-7 7-7"/></svg>
      </button>
      <span class="tk-brand">Aufgaben</span>
    </div>

    <div class="tk-tools">
      <button type="button" class="tk-btn tk-btn-primary" onclick={openCreate}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        Neue Aufgabe
      </button>
      <button type="button" class="tk-btn tk-btn-ghost" onclick={onSync} disabled={syncing}>
        {syncing ? "Synchronisiere…" : "Von CalDAV laden"}
      </button>
    </div>

    <div class="tk-filters">
      <button type="button" class="tk-filter" class:active={filter === "open"} onclick={() => setFilter("open")}>Offen</button>
      <button type="button" class="tk-filter" class:active={filter === "done"} onclick={() => setFilter("done")}>Erledigt</button>
      <button type="button" class="tk-filter" class:active={filter === "all"} onclick={() => setFilter("all")}>Alle</button>
    </div>

    {#if syncMsg}
      <div class="tk-sync-msg">{syncMsg}</div>
    {/if}
    <div class="tk-count">{todos.length} {filter === "done" ? "erledigt" : "Aufgaben"}</div>
  </aside>

  <main class="tk-main">
    {#if loading}
      <div class="tk-state">Lade Aufgaben…</div>
    {:else if error}
      <div class="tk-state tk-state-error">
        <p>{error}</p>
        <button type="button" class="tk-btn tk-btn-ghost" onclick={loadTodos}>Erneut laden</button>
      </div>
    {:else if todos.length === 0}
      <div class="tk-state">
        <p>{filter === "open" ? "Keine offenen Aufgaben." : filter === "done" ? "Noch nichts erledigt." : "Noch keine Aufgaben."}</p>
        {#if filter !== "done"}
          <button type="button" class="tk-btn tk-btn-ghost" onclick={openCreate}>Aufgabe anlegen</button>
        {/if}
      </div>
    {:else}
      <ul class="tk-list">
        {#each todos as t (t.uid)}
          <li class="tk-item" class:done={t.status === "COMPLETED"} class:overdue={isOverdue(t)}>
            <button
              type="button"
              class="tk-check"
              class:checked={t.status === "COMPLETED"}
              onclick={() => onToggle(t)}
              aria-label={t.status === "COMPLETED" ? "Wieder öffnen" : "Erledigt markieren"}
            >
              {#if t.status === "COMPLETED"}✓{/if}
            </button>
            <div class="tk-item-body">
              <span class="tk-item-summary">{t.summary || "Ohne Titel"}</span>
              {#if t.due_at}
                <span class="tk-item-due" class:overdue={isOverdue(t)}>
                  {isOverdue(t) ? "Fällig " : "Bis "}{dueLabel(t)}
                </span>
              {/if}
            </div>
            {#if t.priority}
              <span class="tk-prio" title="Priorität {t.priority}">P{t.priority}</span>
            {/if}
            <button type="button" class="tk-icon-btn tk-icon-btn-danger" onclick={() => removeTodo(t)} title="Löschen">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </main>

  {#if editorOpen}
    <div
      class="tk-modal-backdrop"
      role="button"
      tabindex="0"
      aria-label="Dialog schließen"
      onclick={(e) => { if (e.target === e.currentTarget && !busy) editorOpen = false; }}
      onkeydown={(e) => { if (e.key === "Escape" || e.key === "Enter") !busy && (editorOpen = false); }}
    >
      <div class="tk-modal" role="dialog" aria-modal="true" tabindex="-1" aria-label="Neue Aufgabe">
        <h2>Neue Aufgabe</h2>
        <label>Aufgabe
          <input type="text" bind:value={form.summary} placeholder="z.B. Rechnung bezahlen" />
        </label>
        <label>Beschreibung
          <textarea bind:value={form.description} placeholder="Optional…" rows="2"></textarea>
        </label>
        <label>Fällig am
          <input type="date" bind:value={form.due} />
        </label>
        <label>Priorität (1 = hoch, 9 = niedrig)
          <input type="number" min="1" max="9" bind:value={form.priority} />
        </label>
        <div class="tk-modal-actions">
          <button type="button" class="tk-btn tk-btn-ghost" onclick={() => editorOpen = false} disabled={busy}>Abbrechen</button>
          <button type="button" class="tk-btn tk-btn-primary" onclick={saveTodo} disabled={busy || !form.summary.trim()}>
            {busy ? "Speichern…" : "Speichern"}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .tk-app {
    display: flex;
    height: 100vh;
    background: var(--color-list);
    color: var(--color-text);
  }
  .tk-sidebar {
    width: 240px;
    min-width: 240px;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
  }
  .tk-sidebar-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--color-border);
  }
  .tk-back {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-s);
  }
  .tk-back:hover { color: var(--color-text); background: var(--color-active-wash); }
  .tk-brand { font-weight: 600; font-size: 15px; }

  .tk-tools { padding: 12px 12px 4px; display: flex; flex-direction: column; gap: 8px; }
  .tk-filters {
    display: flex;
    gap: 4px;
    padding: 10px 12px 4px;
    border-top: 1px solid var(--color-border);
  }
  .tk-filter {
    flex: 1;
    padding: 6px 4px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: none;
    color: var(--color-text-secondary);
    font-size: 12px;
    cursor: pointer;
  }
  .tk-filter:hover { background: var(--color-active-wash); }
  .tk-filter.active { background: var(--color-active-wash); color: var(--color-text); border-color: var(--color-border); }
  .tk-sync-msg {
    padding: 8px 16px;
    font-size: 12px;
    color: var(--color-text-secondary);
  }
  .tk-count {
    padding: 10px 16px;
    font-size: 12px;
    color: var(--color-text-secondary);
    border-top: 1px solid var(--color-border);
    margin-top: auto;
  }

  .tk-main { flex: 1; overflow-y: auto; padding: 20px 24px; }
  .tk-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
    color: var(--color-text-secondary);
    font-size: 14px;
  }
  .tk-state-error { color: var(--color-danger); }

  .tk-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .tk-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: var(--color-card, var(--color-list));
    border: 1px solid var(--color-border);
    border-radius: var(--radius-m);
  }
  .tk-item.overdue { border-color: var(--color-danger); }
  .tk-item.done { opacity: 0.6; }
  .tk-item.done .tk-item-summary { text-decoration: line-through; }

  .tk-check {
    width: 22px;
    height: 22px;
    min-width: 22px;
    border-radius: 50%;
    border: 2px solid var(--color-border);
    background: none;
    color: #fff;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
  }
  .tk-check:hover { border-color: var(--color-accent); }
  .tk-check.checked { background: var(--color-accent); border-color: var(--color-accent); }

  .tk-item-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tk-item-summary { font-weight: 500; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tk-item-due { font-size: 12px; color: var(--color-text-secondary); }
  .tk-item-due.overdue { color: var(--color-danger); font-weight: 600; }
  .tk-prio {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-secondary);
    background: var(--color-active-wash);
    padding: 2px 6px;
    border-radius: var(--radius-s);
  }

  .tk-icon-btn {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--radius-s);
  }
  .tk-icon-btn:hover { color: var(--color-text); background: var(--color-active-wash); }
  .tk-icon-btn-danger:hover { color: var(--color-danger); }

  .tk-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 14px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-card, var(--color-list));
    color: var(--color-text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .tk-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .tk-btn-primary { background: var(--color-accent); border-color: var(--color-accent); color: #fff; }
  .tk-btn-ghost { border-color: transparent; background: transparent; color: var(--color-text-secondary); }
  .tk-btn-ghost:hover { background: var(--color-active-wash); }

  .tk-modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .tk-modal {
    width: 420px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    background: var(--color-card, var(--color-list));
    border: 1px solid var(--color-border);
    border-radius: var(--radius-l);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tk-modal h2 { margin: 0 0 4px; font-size: 17px; }
  .tk-modal label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--color-text-secondary); }
  .tk-modal input, .tk-modal textarea {
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-list);
    color: var(--color-text);
    font-size: 13px;
    font-family: inherit;
  }
  .tk-modal input:focus, .tk-modal textarea:focus { outline: none; border-color: var(--color-accent); }
  .tk-modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
</style>
