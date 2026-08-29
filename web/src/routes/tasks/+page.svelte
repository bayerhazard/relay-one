<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listTodos, createTodo, toggleTodo, deleteTodo, syncTodos, nlCreate,
    type TodoInfo, type TodoInput,
  } from "$lib/services/tauri";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import ModuleIcons from "$lib/components/ModuleIcons.svelte";
  import SidebarSearch from "$lib/components/SidebarSearch.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import { useSidebarResize } from "$lib/composables/useSidebarResize";
  import { t, translate } from "$lib/i18n";

  const { width: sidebarWidth, startResize, destroy: destroyResize } = useSidebarResize();
  $effect(() => () => destroyResize());

  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isNarrow = $derived(viewportWidth <= 768);
  let sidebarOpen = $state(false);
  $effect(() => {
    const onResize = () => (viewportWidth = window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  type Filter = "all" | "open" | "done";

  let todos = $state<TodoInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let filter = $state<Filter>("open");
  let busy = $state(false);
  let syncing = $state(false);
  let syncMsg = $state<string | null>(null);
  let tkSearch = $state("");

  let visibleTodos = $derived.by(() => {
    const q = tkSearch.trim().toLowerCase();
    if (!q) return todos;
    return todos.filter((t) => (t.summary ?? "").toLowerCase().includes(q));
  });

  let editorOpen = $state(false);
  let form = $state({ summary: "", description: "", due: "", priority: 5 as number });

  // NL-Erstellung (Phase 4.1)
  let nlInput = $state("");
  let nlLoading = $state(false);
  let nlResult = $state<string | null>(null);

  async function handleNlCreate() {
    const text = nlInput.trim();
    if (!text || nlLoading) return;
    nlLoading = true;
    nlResult = null;
    try {
      const res = await nlCreate(text, translate("tasks.title"));
      if (res.type === "event") {
        nlResult = translate("tasks.nlEventDetected");
      } else {
        await createTodo({ summary: res.title, due: res.due ?? undefined });
        nlResult = translate("tasks.nlCreated", { title: res.title });
        await loadTodos();
      }
      nlInput = "";
    } catch (e) {
      nlResult = String(e);
    } finally {
      nlLoading = false;
    }
  }

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
    const name = t.summary || translate("tasks.untitled");
    if (!confirm(translate("tasks.deleteConfirm", { name }))) return;
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
      syncMsg = translate("tasks.synced", { n: r.synced });
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

<div class="tk-app" class:narrow={isNarrow} class:sidebar-open={isNarrow && sidebarOpen}>
  {#if isNarrow && sidebarOpen}
    <div class="tk-scrim" role="presentation" onclick={() => (sidebarOpen = false)}></div>
  {/if}
  <aside class="tk-sidebar" style={isNarrow ? "" : `width: ${$sidebarWidth}px; min-width: ${$sidebarWidth}px;`}>
    <div class="tk-sidebar-header">
      {#if isNarrow}
        <button type="button" class="tk-nav-btn tk-sidebar-close" onclick={() => (sidebarOpen = false)} aria-label={$t("tasks.close")}>←</button>
      {/if}
      <ModuleLogo to="/" label={$t("tasks.title")} noHover />
    </div>

    <div class="tk-tools">
      <button type="button" class="tk-btn tk-btn-primary" onclick={openCreate}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        {$t("tasks.new")}
      </button>
      <button type="button" class="tk-btn tk-btn-ghost" onclick={onSync} disabled={syncing}>
        {syncing ? $t("tasks.syncing") : $t("tasks.syncFromCaldav")}
      </button>
    </div>

    <div class="tk-nl">
      <input
        type="text"
        class="tk-nl-input"
        bind:value={nlInput}
        placeholder={$t("tasks.nlPlaceholder")}
        onkeydown={(e) => { if (e.key === "Enter") handleNlCreate(); }}
      />
      <button type="button" class="tk-btn tk-btn-primary" disabled={nlLoading || !nlInput.trim()} onclick={handleNlCreate}>
        {nlLoading ? "…" : $t("tasks.create")}
      </button>
      {#if nlResult}
        <span class="tk-nl-result">{nlResult}</span>
      {/if}
    </div>

    <div class="tk-filters">
      <button type="button" class="tk-filter" class:active={filter === "open"} onclick={() => setFilter("open")}>{$t("tasks.filterOpen")}</button>
      <button type="button" class="tk-filter" class:active={filter === "done"} onclick={() => setFilter("done")}>{$t("tasks.filterDone")}</button>
      <button type="button" class="tk-filter" class:active={filter === "all"} onclick={() => setFilter("all")}>{$t("tasks.filterAll")}</button>
    </div>

    {#if syncMsg}
      <div class="tk-sync-msg">{syncMsg}</div>
    {/if}
    <div class="tk-count">{visibleTodos.length} {filter === "done" ? $t("tasks.countDone") : $t("tasks.countTasks")}</div>

    <div class="tk-sidebar-footer">
      <SidebarSearch
        bind:value={tkSearch}
        placeholder={$t("tasks.searchPlaceholder")}
        ariaLabel={$t("tasks.searchLabel")}
        clearLabel={$t("tasks.clearSearch")}
      />
      <div class="tk-module-row">
        <ModuleIcons active="tasks" />
      </div>
    </div>
  </aside>
  {#if !isNarrow}
    <div class="resize-handle" role="separator" aria-orientation="vertical" onmousedown={startResize}></div>
  {/if}

  <main class="tk-main">
    {#if isNarrow}
      <div class="tk-mobile-header">
        <button type="button" class="tk-nav-btn tk-menu-toggle" onclick={() => (sidebarOpen = true)} aria-label={$t("tasks.menu")}>☰</button>
        <h1>{$t("tasks.title")}</h1>
      </div>
    {/if}
    {#if loading}
      <div class="tk-state">{$t("tasks.loading")}</div>
    {:else if error}
      <div class="tk-state tk-state-error">
        <p>{error}</p>
        <button type="button" class="tk-btn tk-btn-ghost" onclick={loadTodos}>{$t("tasks.reload")}</button>
      </div>
    {:else if visibleTodos.length === 0}
      <div class="tk-state">
        <p>{tkSearch ? $t("tasks.notFound") : filter === "open" ? $t("tasks.noOpen") : filter === "done" ? $t("tasks.noDone") : $t("tasks.empty")}</p>
        {#if filter !== "done"}
          <button type="button" class="tk-btn tk-btn-ghost" onclick={openCreate}>{$t("tasks.createTask")}</button>
        {/if}
      </div>
    {:else}
      <ul class="tk-list">
        {#each visibleTodos as todo (todo.uid)}
          <li class="tk-item" class:done={todo.status === "COMPLETED"} class:overdue={isOverdue(todo)}>
            <button
              type="button"
              class="tk-check"
              class:checked={todo.status === "COMPLETED"}
              onclick={() => onToggle(todo)}
              aria-label={todo.status === "COMPLETED" ? $t("tasks.reopen") : $t("tasks.markDone")}
            >
              {#if todo.status === "COMPLETED"}✓{/if}
            </button>
            <div class="tk-item-body">
              <span class="tk-item-summary">{todo.summary || $t("tasks.untitled")}</span>
              {#if todo.due_at}
                <span class="tk-item-due" class:overdue={isOverdue(todo)}>
                  {isOverdue(todo) ? $t("tasks.overdue") : $t("tasks.due")}{dueLabel(todo)}
                </span>
              {/if}
            </div>
            {#if todo.priority}
              <span class="tk-prio" title={$t("tasks.priority", { p: todo.priority })}>P{todo.priority}</span>
            {/if}
            <button type="button" class="tk-icon-btn tk-icon-btn-danger" onclick={() => removeTodo(todo)} title={$t("tasks.deleteBtn")}>
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
      aria-label={$t("tasks.closeDialog")}
      onclick={(e) => { if (e.target === e.currentTarget && !busy) editorOpen = false; }}
      onkeydown={(e) => { if (e.key === "Escape" || e.key === "Enter") !busy && (editorOpen = false); }}
    >
      <div class="tk-modal" role="dialog" aria-modal="true" tabindex="-1" aria-label={$t("tasks.new")}>
        <h2>{$t("tasks.new")}</h2>
        <label>{$t("tasks.taskLabel")}
          <input type="text" bind:value={form.summary} placeholder={$t("tasks.phSummary")} />
        </label>
        <label>{$t("tasks.description")}
          <textarea bind:value={form.description} placeholder={$t("tasks.phOptional")} rows="2"></textarea>
        </label>
        <label>{$t("tasks.dueDate")}
          <input type="date" bind:value={form.due} />
        </label>
        <label>{$t("tasks.priorityLabel")}
          <input type="number" min="1" max="9" bind:value={form.priority} />
        </label>
        <div class="tk-modal-actions">
          <button type="button" class="tk-btn tk-btn-ghost" onclick={() => editorOpen = false} disabled={busy}>{$t("common.cancel")}</button>
          <button type="button" class="tk-btn tk-btn-primary" onclick={saveTodo} disabled={busy || !form.summary.trim()}>
            {busy ? $t("tasks.saving") : $t("common.save")}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

  <AssistantFab module="tasks" />

<style>
  .tk-app {
    display: flex;
    height: 100vh;
    background: var(--color-list);
    color: var(--color-text);
  }
  .tk-sidebar {
    flex-shrink: 0;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
  }
  .tk-sidebar-header {
    height: 72px;
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    margin-bottom: 16px;
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
  .tk-brand { font-weight: 600; font-size: var(--fs-base); }

  .tk-tools { padding: 12px 12px 4px; display: flex; flex-direction: column; gap: 8px; }
  .tk-nl {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 12px 8px;
  }
  .tk-nl-input {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    font: inherit;
    color: var(--color-text);
    background: var(--color-card);
  }
  .tk-nl-result {
    font-size: 0.78rem;
    color: var(--color-text-secondary);
  }
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
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .tk-filter:hover { background: var(--color-active-wash); }
  .tk-filter.active { background: var(--color-active-wash); color: var(--color-text); border-color: var(--color-border); }
  .tk-sync-msg {
    padding: 8px 16px;
    font-size: var(--fs-xs);
    color: var(--color-text-secondary);
  }
  .tk-count {
    padding: 10px 16px;
    font-size: var(--fs-xs);
    color: var(--color-text-secondary);
    border-top: 1px solid var(--color-border);
  }
  .tk-sidebar-footer {
    margin-top: auto;
    padding: 12px;
    border-top: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .tk-module-row { display: flex; justify-content: center; }

  .tk-main { flex: 1; overflow-y: auto; padding: 20px 24px; }
  .tk-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
    color: var(--color-text-secondary);
    font-size: var(--fs-base);
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
    font-size: var(--fs-sm);
  }
  .tk-check:hover { border-color: var(--color-accent); }
  .tk-check.checked { background: var(--color-accent); border-color: var(--color-accent); }

  .tk-item-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tk-item-summary { font-weight: 500; font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tk-item-due { font-size: var(--fs-xs); color: var(--color-text-secondary); }
  .tk-item-due.overdue { color: var(--color-danger); font-weight: 600; }
  .tk-prio {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-sm);
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
  .tk-modal h2 { margin: 0 0 4px; font-size: var(--fs-md); }
  .tk-modal label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--color-text-secondary); }
  .tk-modal input, .tk-modal textarea {
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-list);
    color: var(--color-text);
    font-size: var(--fs-sm);
    font-family: inherit;
  }
  .tk-modal input:focus, .tk-modal textarea:focus { outline: none; border-color: var(--color-accent); }
  .tk-modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }

  /* ── Narrow (mobile ≤768px): sidebar collapses to a slide-in overlay ── */
  .tk-app.narrow .tk-sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 85%;
    max-width: 320px;
    z-index: 60;
    transform: translateX(-100%);
    transition: transform 0.25s cubic-bezier(0.32, 0.72, 0, 1);
    box-shadow: 2px 0 12px rgba(0, 0, 0, 0.18);
  }
  .tk-app.narrow.sidebar-open .tk-sidebar { transform: translateX(0); }
  .tk-app.narrow .tk-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 55;
  }
  .tk-app.narrow .tk-sidebar-close,
  .tk-app.narrow .tk-menu-toggle { display: inline-flex; }
  .tk-app:not(.narrow) .tk-sidebar-close,
  .tk-app:not(.narrow) .tk-menu-toggle { display: none; }
  .tk-app.narrow .resize-handle { display: none; }
  .tk-app.narrow .tk-main { padding: 12px; }
  .tk-mobile-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }
  .tk-mobile-header h1 {
    margin: 0;
    font-size: var(--fs-md);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tk-nav-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px;
    background: none;
    border: none;
    color: var(--color-text);
    cursor: pointer;
    border-radius: var(--radius-s);
    font-size: 1.25rem;
  }
  .tk-nav-btn:hover { background: var(--color-active-wash); }
</style>
