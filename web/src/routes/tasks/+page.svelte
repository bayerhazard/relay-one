<script lang="ts">
  import { onMount } from "svelte";
  import {
    listTodos, createTodo, toggleTodo, patchTodo, deleteTodo, succeedTodo,
    quickAddTodo, todoViews, getCalendars,
    type TodoInfo, type TodoPatchInput,
  } from "$lib/services/tauri";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import SidebarFooter from "$lib/components/SidebarFooter.svelte";
  import SidebarSearch from "$lib/components/SidebarSearch.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import SummaryLine from "$lib/components/SummaryLine.svelte";
  import { useSidebarResize } from "$lib/composables/useSidebarResize";
  import { fmtDateByLang, localeTag as fmtLocaleTag } from "$lib/utils/format";
  import { t, translate } from "$lib/i18n";
  import { fabHidden } from "$lib/stores/fabHidden";
  import { dataVersion } from "$lib/stores/invalidation";
  import { parseQuickAdd, PRIO_LABEL, type ParsedQuickAdd } from "$lib/utils/quickAdd";

  const { width: sidebarWidth, startResize, destroy: destroyResize } = useSidebarResize();
  $effect(() => () => destroyResize());

  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isNarrow = $derived(viewportWidth <= 768);
  let sidebarOpen = $state(false);
  $effect(() => { fabHidden.set(isNarrow && sidebarOpen); });
  $effect(() => {
    const onResize = () => (viewportWidth = window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  // ── Views ────────────────────────────────────────────────────────────
  // "inbox" | "today" | "upcoming" | "all" | "done" | "p:<calendarId>"
  type Selection = string;

  interface Project {
    id: number;
    name: string;
    color: string | null;
  }

  let todos = $state<TodoInfo[]>([]);
  let projects = $state<Project[]>([]);
  let counts = $state({ inbox: 0, today: 0, upcoming: 0, overdue: 0, done: 0, all: 0 });
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selection = $state<Selection>("all");
  let busy = $state(false);
  let tkSearch = $state("");
  /** Active tag filter (combined with the current view); null = none. */
  let tagFilter = $state<string | null>(null);

  /** Sort mode (server-side): work | due | priority | title | created | manual. */
  let sortMode = $state<string>("work");

  function persistSort() {
    try {
      localStorage.setItem("relay_todo_sort", sortMode);
    } catch {
      /* ignore (private mode) */
    }
  }

  async function setSort(mode: string) {
    sortMode = mode;
    persistSort();
    await loadAll();
  }

  /** All labels across the tasks with their counts (for the sidebar). */
  let allTags = $derived.by(() => {
    const m = new Map<string, number>();
    for (const t of todos) {
      for (const l of t.labels) m.set(l, (m.get(l) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  // ── Quick Add ────────────────────────────────────────────────────────
  let qaText = $state("");
  let qaFocused = $state(false);
  let qaInput = $state<HTMLInputElement | null>(null);
  // Live preview of what the parser understood (chips below the field).
  let qaParsed = $derived<ParsedQuickAdd>(parseQuickAdd(qaText));

  /** Focus the Quick-Add field (sidebar "New" button + global Q shortcut). */
  function focusQuickAdd() {
    if (isNarrow) sidebarOpen = false;
    qaInput?.focus();
  }

  async function submitQuickAdd() {
    const text = qaText.trim();
    if (!text || busy) return;
    busy = true;
    error = null;
    try {
      const pid = selection.startsWith("p:") ? Number(selection.slice(2)) : undefined;
      await quickAddTodo(text, pid);
      qaText = "";
      await loadAll();
      // Keep focus for the next capture.
      qaInput?.focus();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  // ── Data loading ─────────────────────────────────────────────────────
  async function loadAll() {
    loading = true;
    error = null;
    try {
      await refreshQuiet();
    } finally {
      loading = false;
    }
  }

  /** Reload the list without toggling the loading state (poll / focus). */
  async function refreshQuiet() {
    try {
      const [list, views] = await Promise.all([
        listTodos(undefined, sortMode),
        todoViews(),
      ]);
      todos = list;
      counts = views;
      error = null;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function loadProjects() {
    try {
      const cals = await getCalendars();
      projects = cals.map((c) => ({
        id: c.id,
        name: c.name || translate("tasks.calendarFallback", { id: c.id }),
        color: c.color,
      }));
    } catch {
      projects = [];
    }
  }

  onMount(() => {
    try {
      const m = localStorage.getItem("relay_todo_sort");
      if (m) sortMode = m;
    } catch {
      /* ignore */
    }
    void loadAll();
    void loadProjects();

    // Auto-refresh: the server syncs CalDAV in the background; poll the local
    // cache periodically and whenever the tab regains focus/visibility.
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void refreshQuiet();
    }, 60_000);
    const onVis = () => { if (document.visibilityState === "visible") void refreshQuiet(); };
    document.addEventListener("visibilitychange", onVis);
    window.addEventListener("focus", onVis);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVis);
      window.removeEventListener("focus", onVis);
    };
  });

  // Reload after an assistant plan execution; skip the first run.
  let assistantReloaded = false;
  $effect(() => {
    const v = $dataVersion;
    if (!assistantReloaded) { assistantReloaded = true; return; }
    void loadAll();
  });

  // Global shortcuts: "Q" focuses Quick Add; Escape closes the task dialogs
  // (matching Contacts/Calendar/Mail). ConfirmationDialog and ContextMenu own
  // their own Escape handling, so leave them alone.
  function onGlobalKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (busy || deleteTarget || ctxMenu) return;
      if (subtaskParent) { subtaskParent = null; return; }
      if (detail) { closeDetail(); return; }
      return;
    }
    const el = e.target as HTMLElement | null;
    const typing = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
    if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "q" || e.key === "Q") {
      e.preventDefault();
      qaInput?.focus();
    }
  }

  // ── Filtering ────────────────────────────────────────────────────────
  function isDone(t: TodoInfo): boolean {
    return t.status === "COMPLETED";
  }

  function dueDay(t: TodoInfo): string | null {
    if (!t.due_at) return null;
    const d = new Date(t.due_at);
    if (isNaN(d.getTime())) return null;
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function todayKey(): string {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function inSelection(t: TodoInfo, sel: Selection): boolean {
    const dk = dueDay(t);
    const tk = todayKey();
    switch (true) {
      case sel === "inbox": return t.project_id === null;
      case sel === "today": return dk !== null && dk <= tk;
      case sel === "upcoming": return dk !== null && dk > tk;
      case sel === "all": return true;
      case sel === "done": return isDone(t);
      case sel.startsWith("p:"): return t.project_id === Number(sel.slice(2));
      default: return true;
    }
  }

  let visibleTodos = $derived.by(() => {
    const q = tkSearch.trim().toLowerCase();
    let list = todos.filter((t) => {
      if (selection === "done") return isDone(t);
      // Every non-"done" view hides completed tasks unless the toggle is on.
      if (isDone(t)) return false;
      if (tagFilter && !t.labels.includes(tagFilter)) return false;
      return inSelection(t, selection);
    });
    if (q) {
      list = list.filter((t) =>
        (t.summary ?? "").toLowerCase().includes(q) ||
        (t.description ?? "").toLowerCase().includes(q) ||
        t.labels.some((l) => l.toLowerCase().includes(q)),
      );
    }
    return list;
  });

  // Subtask map: parent_uid -> children (only those in the current list).
  let subtasksOf = $derived.by(() => {
    const map = new Map<string, TodoInfo[]>();
    for (const t of visibleTodos) {
      if (t.parent_uid) {
        const arr = map.get(t.parent_uid) ?? [];
        arr.push(t);
        map.set(t.parent_uid, arr);
      }
    }
    return map;
  });

  // Top-level tasks: no parent, or parent not visible → shown in the main list.
  let topLevel = $derived.by(() => {
    const visibleUids = new Set(visibleTodos.map((t) => t.uid));
    return visibleTodos.filter((t) => !t.parent_uid || !visibleUids.has(t.parent_uid));
  });

  // "Next steps" strip for the Today view: the first unblocked tasks in work order.
  let nextSteps = $derived.by(() => {
    if (selection !== "today") return [] as TodoInfo[];
    return topLevel.filter((t) => !isDone(t) && !isBlocked(t)).slice(0, 5);
  });

  // Day grouping for the Upcoming view.
  let groups = $derived.by(() => {
    if (selection !== "upcoming") return [{ label: null as string | null, items: topLevel }];
    const byDay = new Map<string, TodoInfo[]>();
    const noDate: TodoInfo[] = [];
    for (const t of topLevel) {
      const dk = dueDay(t);
      if (!dk) { noDate.push(t); continue; }
      const arr = byDay.get(dk) ?? [];
      arr.push(t);
      byDay.set(dk, arr);
    }
    const ordered = [...byDay.entries()].sort((a, b) => a[0].localeCompare(b[0]));
    const out = ordered.map(([day, items]) => ({ label: dayLabel(day), items }));
    if (noDate.length) out.push({ label: translate("tasks.noDate"), items: noDate });
    return out;
  });

  function dayLabel(key: string): string {
    const tk = todayKey();
    if (key === tk) return translate("tasks.today");
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    const tk2 = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;
    if (key === tk2) return translate("tasks.tomorrow");
    const [y, m, d] = key.split("-").map(Number);
    return fmtDateByLang(new Date(y, m - 1, d), fmtLocaleTag());
  }

  // ── Mutations ────────────────────────────────────────────────────────
  async function onToggle(t: TodoInfo) {
    try {
      if (isDone(t)) {
        await toggleTodo(t.uid, false);
      } else if (t.rrule) {
        // Recurring: complete this one and spawn the next occurrence.
        await succeedTodo(t.uid);
      } else {
        await toggleTodo(t.uid, true);
      }
      await loadAll();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  let deleteTarget = $state<TodoInfo | null>(null);

  let ctxMenu = $state<{ x: number; y: number; todo: TodoInfo } | null>(null);
  let ctxItems = $derived.by(() => {
    const c = ctxMenu;
    if (!c) return [];
    return [
      { label: isDone(c.todo) ? translate("tasks.reopen") : translate("tasks.markDone"), action: () => onToggle(c.todo) },
      { label: translate("tasks.subtaskAdd"), action: () => openSubtaskFor(c.todo) },
      { label: translate("tasks.deleteBtn"), danger: true, action: () => askDelete(c.todo) },
    ];
  });

  function askDelete(t: TodoInfo): void { deleteTarget = t; }

  async function removeTodo(t: TodoInfo) {
    try {
      await deleteTodo(t.uid);
      await loadAll();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      deleteTarget = null;
    }
  }

  // ── Detail panel ─────────────────────────────────────────────────────
  let detailUid = $state<string | null>(null);
  let detail = $state<TodoInfo | null>(null);
  $effect(() => {
    if (!detailUid) { detail = null; return; }
    detail = todos.find((t) => t.uid === detailUid) ?? null;
  });

  let edit = $state({
    summary: "", description: "", due: "", dueTime: "", priority: null as number | null,
    rrule: "", labelsText: "", projectId: null as number | null,
    dependencies: [] as string[],
  });

  $effect(() => {
    const d = detail;
    if (!d) return;
    edit = {
      summary: d.summary ?? "",
      description: d.description ?? "",
      due: d.due_at ? dueDay(d) ?? "" : "",
      dueTime: d.due_at && d.due_has_time ? timeOf(d.due_at) : "",
      priority: d.priority,
      rrule: d.rrule ?? "",
      labelsText: d.labels.join(", "),
      projectId: d.project_id,
      dependencies: [...(d.dependencies ?? [])],
    };
  });

  function timeOf(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return "";
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }

  /** Open tasks that can be chosen as a blocker for the task being edited. */
  let blockerOptions = $derived.by(() => {
    if (!detail) return [] as TodoInfo[];
    return todos
      .filter((t) => t.uid !== detail!.uid && t.status !== "COMPLETED")
      .sort((a, b) => (a.summary ?? "").localeCompare(b.summary ?? ""));
  });

  /** Open blocker UIDs for a task (unknown/done blockers do not count). */
  function openBlockers(t: TodoInfo): string[] {
    if (!t.dependencies?.length) return [];
    const byUid = new Map(todos.map((x) => [x.uid, x]));
    return t.dependencies.filter((uid) => {
      const b = byUid.get(uid);
      return b ? b.status !== "COMPLETED" : false;
    });
  }

  function isBlocked(t: TodoInfo): boolean {
    return openBlockers(t).length > 0;
  }

  function openDetail(t: TodoInfo) {
    detailUid = t.uid;
  }

  function closeDetail() {
    detailUid = null;
  }

  async function saveDetail() {
    const d = detail;
    if (!d) return;
    busy = true;
    try {
      // Combine date + optional time into an RFC 3339 timestamp.
      let due: string | null = null;
      if (edit.due) {
        const t = edit.dueTime || "00:00";
        const [y, m, day] = edit.due.split("-").map(Number);
        const [hh, mm] = t.split(":").map(Number);
        due = new Date(y, m - 1, day, hh, mm).toISOString();
      }
      const patch: TodoPatchInput = {
        summary: edit.summary,
        description: edit.description || null,
        due,
        priority: edit.priority,
        labels: edit.labelsText.split(",").map((s) => s.trim()).filter(Boolean),
        rrule: edit.rrule.trim() || null,
        project_id: edit.projectId,
        dependencies: edit.dependencies,
      };
      await patchTodo(d.uid, patch);
      await loadAll();
      closeDetail();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  let subtaskParent = $state<TodoInfo | null>(null);
  let subtaskTitle = $state("");

  function openSubtaskFor(parent: TodoInfo) {
    subtaskParent = parent;
    subtaskTitle = "";
  }

  async function saveSubtask() {
    const parent = subtaskParent;
    const title = subtaskTitle.trim();
    if (!parent || !title) return;
    busy = true;
    try {
      await createTodo({
        summary: title,
        parent_uid: parent.uid,
        project_id: parent.project_id ?? undefined,
      });
      subtaskParent = null;
      await loadAll();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function prioClass(p: number | null): string {
    return p ? `prio-${p}` : "";
  }

  function dueLabel(t: TodoInfo): string {
    if (!t.due_at) return "";
    const d = new Date(t.due_at);
    if (isNaN(d.getTime())) return "";
    const base = fmtDateByLang(d, fmtLocaleTag());
    if (t.due_has_time) {
      return `${base} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    }
    return base;
  }

  function isOverdue(t: TodoInfo): boolean {
    if (!t.due_at || isDone(t)) return false;
    const dk = dueDay(t);
    return dk !== null && dk < todayKey();
  }

  function select(sel: Selection) {
    selection = sel;
    if (isNarrow) sidebarOpen = false;
  }

  /** Toggle the tag filter (combined with the current view). */
  function selectTag(tag: string) {
    tagFilter = tagFilter === tag ? null : tag;
    if (isNarrow) sidebarOpen = false;
  }

  function selectionLabel(): string {
    switch (true) {
      case selection === "inbox": return translate("tasks.viewInbox");
      case selection === "today": return translate("tasks.viewToday");
      case selection === "upcoming": return translate("tasks.viewUpcoming");
      case selection === "all": return translate("tasks.viewAll");
      case selection === "done": return translate("tasks.viewDone");
      case selection.startsWith("p:"): {
        const p = projects.find((x) => x.id === Number(selection.slice(2)));
        return p?.name ?? translate("tasks.projects");
      }
      default: return translate("tasks.title");
    }
  }

  /** Human-readable recurrence label from an RRULE. */
  function recurLabel(rrule: string): string {
    const token = recurToken(rrule);
    switch (token) {
      case "daily": return translate("tasks.repeatDaily");
      case "weekly": return translate("tasks.repeatWeekly");
      case "monthly": return translate("tasks.repeatMonthly");
      case "yearly": return translate("tasks.repeatYearly");
      default: return rrule;
    }
  }

  /** Collapse an RRULE to one of the four simple tokens for the <select>. */
  function recurToken(rrule: string): string {
    const r = (rrule || "").toUpperCase();
    if (r.includes("FREQ=DAILY") && !r.includes("INTERVAL=2")) return "daily";
    if (r.includes("FREQ=WEEKLY") && !r.includes("INTERVAL=2")) return "weekly";
    if (r.includes("FREQ=MONTHLY")) return "monthly";
    if (r.includes("FREQ=YEARLY")) return "yearly";
    return "";
  }

  function rruleFromToken(token: string): string {
    switch (token) {
      case "daily": return "FREQ=DAILY";
      case "weekly": return "FREQ=WEEKLY";
      case "monthly": return "FREQ=MONTHLY";
      case "yearly": return "FREQ=YEARLY";
      default: return "";
    }
  }

  /** CSS class for a canonical priority (1..5). */
  function prioClassByUi(ui: number | null): string {
    return ui ? `prio-${ui}` : "";
  }
</script>

<svelte:window onkeydown={onGlobalKey} />

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

    <!-- Focus views -->
    <nav class="tk-views" aria-label={$t("tasks.viewsLabel")}>
      <button type="button" class="tk-view" class:active={selection === "inbox"} onclick={() => select("inbox")}>
        <span class="tk-view-label">{$t("tasks.viewInbox")}</span>
        {#if counts.inbox}<span class="tk-badge">{counts.inbox}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "today"} onclick={() => select("today")}>
        <span class="tk-view-label">{$t("tasks.viewToday")}</span>
        {#if counts.today}<span class="tk-badge">{counts.today}</span>{/if}
        {#if counts.overdue}<span class="tk-badge tk-badge-danger">{counts.overdue}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "upcoming"} onclick={() => select("upcoming")}>
        <span class="tk-view-label">{$t("tasks.viewUpcoming")}</span>
        {#if counts.upcoming}<span class="tk-badge">{counts.upcoming}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "all"} onclick={() => select("all")}>
        <span class="tk-view-label">{$t("tasks.viewAll")}</span>
        {#if counts.all}<span class="tk-badge">{counts.all}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "done"} onclick={() => select("done")}>
        <span class="tk-view-label">{$t("tasks.viewDone")}</span>
        {#if counts.done}<span class="tk-badge">{counts.done}</span>{/if}
      </button>
    </nav>

    <!-- Projects (= CalDAV calendars) -->
    {#if projects.length}
      <div class="tk-projects">
        <div class="tk-projects-head">{$t("tasks.projects")}</div>
        {#each projects as p (p.id)}
          <button type="button" class="tk-view" class:active={selection === `p:${p.id}`} onclick={() => select(`p:${p.id}`)}>
            <span class="tk-dot" style={`background:${p.color || "var(--am-text-gedaempft)"}`}></span>
            <span class="tk-view-label">{p.name}</span>
          </button>
        {/each}
      </div>
    {/if}

    {#if allTags.length}
      <div class="tk-projects tk-tags">
        <div class="tk-projects-head">{$t("tasks.tags")}</div>
        {#each allTags as [tag, count] (tag)}
          <button type="button" class="tk-view" class:active={tagFilter === tag} onclick={() => selectTag(tag)}>
            <span class="tk-tag">{tag}</span>
            <span class="tk-badge">{count}</span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="tk-count">{$t("tasks.count", { n: visibleTodos.length })}</div>

    <SidebarFooter active="tasks">
      <SidebarSearch
        bind:value={tkSearch}
        placeholder={$t("tasks.searchPlaceholder")}
        ariaLabel={$t("tasks.searchLabel")}
        clearLabel={$t("tasks.clearSearch")}
      />
    </SidebarFooter>
  </aside>

  {#if !isNarrow}
    <div class="resize-handle" role="separator" aria-orientation="vertical" onmousedown={startResize}></div>
  {/if}

  <main class="tk-main">
    {#if isNarrow}
      <div class="tk-mobile-header">
        <button type="button" class="tk-nav-btn tk-menu-toggle" onclick={() => (sidebarOpen = true)} aria-label={$t("tasks.menu")}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M3 6h18M3 12h18M3 18h18"/></svg></button>
        <h1>{selectionLabel()}</h1>
        <button type="button" class="tk-nav-btn tk-mobile-new" onclick={focusQuickAdd} aria-label={$t("tasks.new")}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        </button>
      </div>
    {/if}

    <!-- Quick Add: instant capture with live parse chips -->
    <div class="tk-quickadd" class:focused={qaFocused}>
      <span class="tk-qa-plus" aria-hidden="true">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
      </span>
      <input
        bind:this={qaInput}
        type="text"
        class="tk-qa-input"
        bind:value={qaText}
        placeholder={$t("tasks.qaPlaceholder")}
        aria-label={$t("tasks.qaLabel")}
        onfocus={() => (qaFocused = true)}
        onblur={() => (qaFocused = false)}
        onkeydown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); void submitQuickAdd(); }
          if (e.key === "Escape") { qaText = ""; (e.currentTarget as HTMLInputElement).blur(); }
        }}
      />
      <kbd class="tk-qa-kbd">Q</kbd>
      <button type="button" class="tk-btn tk-btn-ghost tk-qa-submit" onclick={submitQuickAdd} disabled={busy || !qaText.trim()}>
        {$t("tasks.add")}
      </button>
    </div>
    {#if qaText.trim()}
      <div class="tk-qa-chips" aria-live="polite">
        <span class="tk-chip tk-chip-title">{qaParsed.title || $t("tasks.untitled")}</span>
        {#if qaParsed.due}
          <span class="tk-chip">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/></svg>
            {fmtDateByLang(qaParsed.due, fmtLocaleTag())}{qaParsed.dueHasTime ? ` ${String(qaParsed.due.getHours()).padStart(2, "0")}:${String(qaParsed.due.getMinutes()).padStart(2, "0")}` : ""}
          </span>
        {/if}
        {#if qaParsed.rrule}
          <span class="tk-chip">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/></svg>
            {recurLabel(qaParsed.rrule)}
          </span>
        {/if}
        {#if qaParsed.priority !== null}<span class={`tk-chip tk-prio-chip ${prioClassByUi(qaParsed.priority)}`}>{PRIO_LABEL[qaParsed.priority]}</span>{/if}
        {#if qaParsed.project}<span class="tk-chip">#{qaParsed.project}</span>{/if}
        {#each qaParsed.labels as l (l)}<span class="tk-chip">{l}</span>{/each}
      </div>
    {/if}

    <div class="tk-list-head">
      <h2>{selectionLabel()}</h2>
      <label class="tk-sort-inline">
        <span>{$t("tasks.sortLabel")}</span>
        <select value={sortMode} onchange={(e) => setSort((e.currentTarget as HTMLSelectElement).value)}>
          <option value="work">{$t("tasks.sortWork")}</option>
          <option value="due">{$t("tasks.sortDue")}</option>
          <option value="priority">{$t("tasks.sortPriority")}</option>
          <option value="title">{$t("tasks.sortTitle")}</option>
          <option value="created">{$t("tasks.sortCreated")}</option>
          <option value="manual">{$t("tasks.sortManual")}</option>
        </select>
      </label>
    </div>

    {#if nextSteps.length}
      <div class="tk-next">
        <div class="tk-next-head">{$t("tasks.nextSteps")}</div>
        <ol class="tk-next-list">
          {#each nextSteps as task (task.uid)}
            <li class="tk-next-item">
              <button type="button" class="tk-check tk-check-sm" onclick={() => onToggle(task)} aria-label={$t("tasks.markDone")}></button>
              <button type="button" class="tk-next-title" onclick={() => openDetail(task)}>{task.summary || $t("tasks.untitled")}</button>
              {#if task.due_at}<span class="tk-item-due" class:overdue={isOverdue(task)}>{dueLabel(task)}</span>{/if}
            </li>
          {/each}
        </ol>
      </div>
    {/if}

    {#if loading}
      <div class="tk-state">{$t("tasks.loading")}</div>
    {:else if error}
      <div class="tk-state tk-state-error">
        <p>{error}</p>
        <button type="button" class="tk-btn tk-btn-ghost" onclick={loadAll}>{$t("tasks.reload")}</button>
      </div>
    {:else if visibleTodos.length === 0}
      <div class="tk-state">
        <p>{tkSearch ? $t("tasks.notFound") : $t("tasks.viewEmpty")}</p>
      </div>
    {:else}
      {#each groups as g (g.label ?? "_")}
        {#if g.label}<div class="tk-group-head">{g.label}</div>{/if}
        <ul class="tk-list">
          {#each g.items as todo (todo.uid)}
            <li class="tk-item-wrap">
              <div
                class="tk-item"
                class:done={isDone(todo)}
                class:overdue={isOverdue(todo)}
                class:blocked={isBlocked(todo)}
                class:selected={detailUid === todo.uid}
                oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, todo }; }}
              >
                <button
                  type="button"
                  class="tk-check"
                  class:checked={isDone(todo)}
                  onclick={() => onToggle(todo)}
                  aria-label={isDone(todo) ? $t("tasks.reopen") : $t("tasks.markDone")}
                >
                  {#if isDone(todo)}✓{/if}
                </button>
                <button type="button" class="tk-item-body" onclick={() => openDetail(todo)}>
                  <span class="tk-item-summary">{todo.summary || $t("tasks.untitled")}</span>
                  {#if todo.description}<SummaryLine summary={todo.description} />{/if}
                  {#if todo.due_at || todo.labels.length || todo.rrule || isBlocked(todo)}
                    <span class="tk-item-meta">
                      {#if isBlocked(todo)}
                        <span class="tk-item-blocked" title={$t("tasks.blockedHint")}>⛓ {openBlockers(todo).length}</span>
                      {/if}
                      {#if todo.due_at}
                        <span class="tk-item-due" class:overdue={isOverdue(todo)}>{dueLabel(todo)}</span>
                      {/if}
                      {#if todo.rrule}
                        <span class="tk-item-rep" title={recurLabel(todo.rrule)}>
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/></svg>
                        </span>
                      {/if}
                      {#each todo.labels as l (l)}<span class="tk-tag">{l}</span>{/each}
                    </span>
                  {/if}
                </button>
                {#if todo.priority}
                  <span class={`tk-prio ${prioClass(todo.priority)}`} title={$t("tasks.priority", { p: todo.priority })}>{PRIO_LABEL[todo.priority]}</span>
                {/if}
              </div>

              <!-- Sub-tasks -->
              {#if subtasksOf.get(todo.uid)?.length}
                <ul class="tk-subtasks">
                  {#each subtasksOf.get(todo.uid) ?? [] as sub (sub.uid)}
                    <li class="tk-subtask">
                      <button type="button" class="tk-check tk-check-sm" class:checked={isDone(sub)} onclick={() => onToggle(sub)} aria-label={isDone(sub) ? $t("tasks.reopen") : $t("tasks.markDone")}>
                        {#if isDone(sub)}✓{/if}
                      </button>
                      <button type="button" class="tk-subtask-title" class:done={isDone(sub)} onclick={() => openDetail(sub)}>{sub.summary || $t("tasks.untitled")}</button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  </main>
</div>

<!-- Detail panel (centred modal) -->
{#if detail}
  <div class="tk-detail-scrim" role="button" tabindex="0" aria-label={$t("tasks.closeDialog")} onclick={(e) => { if (e.target === e.currentTarget) closeDetail(); }} onkeydown={(e) => { if (e.key === "Escape") closeDetail(); }}>
  <aside class="tk-detail" role="dialog" aria-modal="true" tabindex="-1" aria-label={$t("tasks.details")}>
    <header class="tk-detail-head">
      <h2>{$t("tasks.details")}</h2>
      <button type="button" class="tk-icon-btn" onclick={closeDetail} aria-label={$t("tasks.closeDialog")}>✕</button>
    </header>

    <label class="tk-field">
      <span>{$t("tasks.taskLabel")}</span>
      <input type="text" bind:value={edit.summary} />
    </label>

    <label class="tk-field">
      <span>{$t("tasks.description")}</span>
      <textarea rows="3" bind:value={edit.description} placeholder={$t("tasks.phOptional")}></textarea>
    </label>

    <div class="tk-field-row">
      <label class="tk-field">
        <span>{$t("tasks.dueDate")}</span>
        <input type="date" bind:value={edit.due} />
      </label>
      <label class="tk-field">
        <span>{$t("tasks.dueTime")}</span>
        <input type="time" bind:value={edit.dueTime} />
      </label>
    </div>

    <label class="tk-field">
      <span>{$t("tasks.priorityLabel")}</span>
      <select
        value={edit.priority ?? ""}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLSelectElement).value;
          edit.priority = v === "" ? null : Number(v);
        }}
      >
        <option value="">{$t("tasks.priorityNone")}</option>
        {#each [1, 2, 3, 4, 5] as p}
          <option value={p}>{PRIO_LABEL[p]}</option>
        {/each}
      </select>
    </label>

    <label class="tk-field">
      <span>{$t("tasks.repeatLabel")}</span>
      <select
        value={recurToken(edit.rrule)}
        onchange={(e) => (edit.rrule = rruleFromToken((e.currentTarget as HTMLSelectElement).value))}
      >
        <option value="">{$t("tasks.repeatNone")}</option>
        <option value="daily">{$t("tasks.repeatDaily")}</option>
        <option value="weekly">{$t("tasks.repeatWeekly")}</option>
        <option value="monthly">{$t("tasks.repeatMonthly")}</option>
        <option value="yearly">{$t("tasks.repeatYearly")}</option>
      </select>
    </label>

    <label class="tk-field">
      <span>{$t("tasks.projectLabel")}</span>
      <select bind:value={edit.projectId}>
        <option value={null}>{$t("tasks.viewInbox")}</option>
        {#each projects as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
    </label>

    <div class="tk-field-row">
      <label class="tk-field">
        <span>{$t("tasks.labelsLabel")}</span>
        <input type="text" bind:value={edit.labelsText} placeholder={$t("tasks.phLabels")} />
      </label>
    </div>

    <label class="tk-field">
      <span>{$t("tasks.dependsOnLabel")}</span>
      <select multiple bind:value={edit.dependencies} size={Math.min(5, Math.max(2, blockerOptions.length))}>
        {#each blockerOptions as b (b.uid)}
          <option value={b.uid}>{b.summary || $t("tasks.untitled")}</option>
        {/each}
      </select>
    </label>

    <div class="tk-detail-actions">
      <button type="button" class="tk-btn tk-btn-ghost" onclick={() => openSubtaskFor(detail!)}>{$t("tasks.subtaskAdd")}</button>
      <button type="button" class="tk-btn tk-btn-danger" onclick={() => askDelete(detail!)}>{$t("tasks.deleteBtn")}</button>
      <span class="tk-spacer"></span>
      <button type="button" class="tk-btn tk-btn-primary" onclick={saveDetail} disabled={busy}>{busy ? $t("tasks.saving") : $t("common.save")}</button>
    </div>
  </aside>
  </div>
{/if}

<!-- Sub-task creation dialog -->
{#if subtaskParent}
  <div class="tk-modal-backdrop" role="button" tabindex="0" aria-label={$t("tasks.subtaskAdd")} onclick={(e) => { if (e.target === e.currentTarget && !busy) subtaskParent = null; }} onkeydown={(e) => { if (e.key === "Escape") subtaskParent = null; }}>
    <div class="tk-modal" role="dialog" aria-modal="true" tabindex="-1">
      <h2>{$t("tasks.subtaskAdd")}</h2>
      <p class="tk-modal-sub">{subtaskParent.summary || $t("tasks.untitled")}</p>
      <input
        type="text"
        bind:value={subtaskTitle}
        placeholder={$t("tasks.phSubtask")}
        onkeydown={(e) => { if (e.key === "Enter") void saveSubtask(); }}
      />
      <div class="tk-modal-actions">
        <button type="button" class="tk-btn tk-btn-ghost" onclick={() => (subtaskParent = null)} disabled={busy}>{$t("common.cancel")}</button>
        <button type="button" class="tk-btn tk-btn-primary" onclick={saveSubtask} disabled={busy || !subtaskTitle.trim()}>{$t("common.save")}</button>
      </div>
    </div>
  </div>
{/if}

<ContextMenu menu={ctxMenu} items={ctxItems} onclose={() => (ctxMenu = null)} />

<AssistantFab module="tasks" />

<ConfirmationDialog
  open={deleteTarget !== null}
  title={$t("tasks.deleteBtn")}
  message={deleteTarget ? translate("tasks.deleteConfirm", { name: deleteTarget.summary || translate("tasks.untitled") }) : ""}
  confirmLabel={$t("tasks.deleteBtn")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={() => { if (deleteTarget) void removeTodo(deleteTarget); }}
  oncancel={() => (deleteTarget = null)}
/>

<style>
  .tk-app {
    display: flex;
    height: 100vh;
    background: var(--am-seite);
    color: var(--am-text-primaer);
  }
  .tk-sidebar {
    flex-shrink: 0;
    background: var(--am-flaeche-1);
    border-right: 1px solid var(--am-rand);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }
  .tk-sidebar-header {
    height: var(--am-leistenhoehe);
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--am-rand);
    flex-shrink: 0;
    margin-bottom: 16px;
  }

  /* Focus views */
  .tk-views { display: flex; flex-direction: column; gap: 2px; padding: 10px 8px 4px; }
  .tk-view {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    background: none;
    color: var(--am-text-gedaempft);
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    font-size: var(--fs-base);
    font-family: inherit;
    text-align: left;
  }
  .tk-view:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .tk-view.active { background: var(--am-flaeche-2); color: var(--am-text-primaer); font-weight: 600; }
  .tk-view-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tk-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .tk-badge {
    font-size: var(--fs-xs);
    color: var(--am-text-gedaempft);
    background: var(--am-flaeche-1);
    border-radius: 999px;
    padding: 0 7px;
    min-width: 20px;
    text-align: center;
  }
  .tk-badge-danger { color: var(--am-fehler); }

  .tk-projects { padding: 8px; border-top: 1px solid var(--am-rand); margin-top: 6px; }
  .tk-tags .tk-view { justify-content: space-between; }
  .tk-projects-head {
    padding: 4px 10px 8px;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
  }
  .tk-count {
    padding: 10px 16px;
    font-size: var(--fs-xs);
    color: var(--am-text-gedaempft);
    border-top: 1px solid var(--am-rand);
    margin-top: 8px;
  }

  /* Inline sort control in the list header. */
  .tk-sort-inline {
    display: inline-flex; align-items: center; gap: 8px;
    font-size: var(--fs-xs); color: var(--am-text-gedaempft); flex-shrink: 0;
  }
  .tk-sort-inline select {
    font-family: inherit; font-size: var(--fs-xs);
    padding: 4px 6px; border: 1px solid var(--am-rand); border-radius: var(--am-radius-klein);
    background: var(--am-flaeche-1); color: var(--am-text-primaer);
  }

  /* "Next steps" strip (Today view). */
  .tk-next {
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    padding: 10px 12px;
    margin-bottom: 16px;
    background: var(--am-flaeche-1);
  }
  .tk-next-head { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.04em; color: var(--am-text-gedaempft); margin-bottom: 8px; }
  .tk-next-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .tk-next-item { display: flex; align-items: center; gap: 10px; }
  .tk-next-title {
    flex: 1; min-width: 0; text-align: left; background: none; border: none; cursor: pointer;
    color: var(--am-text-primaer); font-size: var(--fs-sm); font-family: inherit;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .tk-next-title:hover { color: var(--am-handlung-ruhend); }

  .tk-main { flex: 1; overflow-y: auto; padding: 20px 24px; min-width: 0; }

  /* Quick Add */
  .tk-quickadd {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-flaeche-1);
    margin-bottom: 8px;
  }
  .tk-quickadd.focused { border-color: var(--am-handlung-ruhend); }
  .tk-qa-plus { display: inline-flex; color: var(--am-handlung-ruhend); flex-shrink: 0; }
  .tk-qa-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--am-text-primaer);
    font-size: var(--fs-base);
    font-family: inherit;
    outline: none;
    padding: 6px 0;
  }
  .tk-qa-kbd {
    font-size: 0.7rem;
    color: var(--am-text-gedaempft);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    padding: 1px 6px;
    flex-shrink: 0;
  }
  .tk-qa-submit { flex-shrink: 0; }

  .tk-qa-chips { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 14px; }
  .tk-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--am-text-gedaempft);
    background: var(--am-flaeche-2);
    border-radius: var(--am-radius-klein);
    padding: 3px 8px;
  }
  .tk-chip-title { color: var(--am-text-primaer); font-weight: 500; }
  .tk-prio-chip { color: var(--am-handlung-text); }
  .tk-prio-chip.prio-1 { background: var(--am-fehler); }
  .tk-prio-chip.prio-2 { background: var(--am-achtung); }
  .tk-prio-chip.prio-3 { background: var(--am-handlung-ruhend); }

  .tk-list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    margin: 4px 0 12px;
  }
  .tk-list-head h2 { margin: 0; font-size: var(--fs-md); font-weight: 600; }

  .tk-group-head {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
    padding: 14px 4px 6px;
  }

  .tk-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 60%;
    color: var(--am-text-gedaempft);
    font-size: var(--fs-base);
  }
  .tk-state-error { color: var(--am-fehler); }

  .tk-list { list-style: none; margin: 0; padding: 0 0 84px; display: flex; flex-direction: column; }
  .tk-item-wrap { display: flex; flex-direction: column; }
  /* Flat row, aligned with the mail inbox list (border-left + bottom divider). */
  .tk-item {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 10px 16px;
    border-left: 3px solid transparent;
    border-bottom: 1px solid var(--am-rand);
  }
  .tk-item:hover { background: var(--am-flaeche-1); }
  .tk-item.selected { background: var(--am-flaeche-2); border-left-color: var(--am-handlung-ruhend); }
  /* Overdue: a red bar on the left edge only, like the urgent marking in the
     mail list. Declared after .selected so the bar stays red when selected. */
  .tk-item.overdue { border-left-color: var(--am-fehler); }
  .tk-item.done { opacity: 0.6; }
  .tk-item.done .tk-item-summary { text-decoration: line-through; }
  .tk-item.done .tk-item-summary { text-decoration: line-through; }

  .tk-check {
    width: 22px;
    height: 22px;
    min-width: 22px;
    border-radius: 50%;
    border: 2px solid var(--am-rand);
    background: none;
    color: var(--am-handlung-text);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-sm);
    padding: 0;
  }
  .tk-check:hover { border-color: var(--am-handlung-ruhend); }
  .tk-check.checked { background: var(--am-handlung-ruhend); border-color: var(--am-handlung-ruhend); }
  .tk-check-sm { width: 18px; height: 18px; min-width: 18px; font-size: 0.7rem; }

  .tk-item-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    background: none;
    border: none;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font-family: inherit;
    padding: 0;
  }
  .tk-item-summary { font-weight: 600; font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tk-item-meta { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .tk-item-due { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .tk-item-due.overdue { color: var(--am-fehler); font-weight: 600; }
  .tk-item-rep { display: inline-flex; align-items: center; color: var(--am-text-gedaempft); }
  .tk-item-label { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  /* Tag pill (no "@"), like the mail draft badge / quick-add chips. */
  .tk-tag {
    display: inline-flex;
    align-items: center;
    font-size: var(--fs-xs);
    color: var(--am-handlung-ruhend);
    background: color-mix(in srgb, var(--am-handlung-ruhend) 12%, transparent);
    padding: 1px 7px;
    border-radius: var(--am-radius-klein);
    white-space: nowrap;
  }

  .tk-prio {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--am-text-gedaempft);
    background: var(--am-flaeche-2);
    padding: 2px 7px;
    border-radius: var(--am-radius-klein);
    flex-shrink: 0;
  }
  /* Priority colours follow the design-guide state palette (theme-aware). */
  .tk-prio.prio-1 { background: var(--am-fehler); color: var(--am-handlung-text); }
  .tk-prio.prio-2 { background: var(--am-achtung); color: var(--am-handlung-text); }
  .tk-prio.prio-3 { background: var(--am-handlung-ruhend); color: var(--am-handlung-text); }
  /* 4/5 are intentionally muted (border + secondary text, no filled colour). */
  .tk-prio.prio-4 { background: var(--am-flaeche-2); color: var(--am-text-gedaempft); }
  .tk-prio.prio-5 {
    background: transparent;
    color: var(--am-text-gedaempft);
    border: 1px solid var(--am-rand);
  }

  /* Blocked tasks: de-emphasised, not hidden. */
  .tk-item.blocked .tk-item-summary { color: var(--am-text-gedaempft); }
  .tk-item-blocked { color: var(--am-text-gedaempft); font-size: var(--fs-xs); }

  .tk-subtasks { list-style: none; margin: 4px 0 4px 34px; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .tk-subtask { display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: var(--am-radius-klein); }
  .tk-subtask:hover { background: var(--am-flaeche-2); }
  .tk-subtask-title {
    background: none; border: none; text-align: left; cursor: pointer;
    color: var(--am-text-gedaempft); font-size: var(--fs-sm); font-family: inherit;
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .tk-subtask-title.done { text-decoration: line-through; opacity: 0.6; }

  .tk-icon-btn {
    background: none;
    border: none;
    color: var(--am-text-gedaempft);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--am-radius-mittel);
    flex-shrink: 0;
  }
  .tk-icon-btn:hover { color: var(--am-text-primaer); background: var(--am-flaeche-2); }

  .tk-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 14px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-flaeche-1);
    color: var(--am-text-primaer);
    font-size: var(--fs-sm);
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
  }
  .tk-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .tk-btn-primary { background: var(--am-handlung-ruhend); border-color: var(--am-handlung-ruhend); color: var(--am-handlung-text); }
  .tk-btn-ghost { border-color: transparent; background: transparent; color: var(--am-text-gedaempft); }
  .tk-btn-ghost:hover { background: var(--am-flaeche-2); }
  /* Secondary danger: the border and the word carry the red (CI R1). */
  .tk-btn-danger { background: var(--am-seite); border-color: var(--am-fehler); color: var(--am-fehler); }
  .tk-btn-danger:hover:not(:disabled) { background: var(--am-fehler-flaeche); }

  /* Detail panel — centred modal, matching Contacts/Calendar pattern */
  .tk-detail-scrim {
    position: fixed;
    inset: 0;
    background: var(--am-deckschicht);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .tk-detail {
    width: 420px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-gross);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tk-detail-head { display: flex; align-items: center; justify-content: space-between; }
  .tk-detail-head h2 { margin: 0; font-size: var(--fs-md); }
  .tk-spacer { flex: 1; }
  .tk-detail-actions { display: flex; align-items: center; gap: 8px; margin-top: 4px; flex-wrap: wrap; }

  .tk-field { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .tk-field-row { display: flex; gap: 10px; }
  .tk-field-row .tk-field { flex: 1; }
  .tk-field input, .tk-field textarea, .tk-field select, .tk-modal input {
    padding: 8px 10px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    background: var(--am-seite);
    color: var(--am-text-primaer);
    font-size: var(--fs-sm);
    font-family: inherit;
  }
  .tk-field input:focus, .tk-field textarea:focus, .tk-field select:focus, .tk-modal input:focus {
    outline: none;
    border-color: var(--am-handlung-ruhend);
  }

  .tk-modal-backdrop {
    position: fixed;
    inset: 0;
    background: var(--am-deckschicht);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 90;
  }
  .tk-modal {
    width: 420px;
    max-width: calc(100vw - 32px);
    background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-gross);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tk-modal h2 { margin: 0; font-size: var(--fs-md); }
  .tk-modal-sub { margin: 0; font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .tk-modal-actions { display: flex; justify-content: flex-end; gap: 8px; }

  /* ── Narrow (mobile ≤768px) ── */
  .tk-app.narrow .tk-sidebar {
    position: fixed;
    top: 0; left: 0; bottom: 0;
    width: 85%;
    max-width: 320px;
    z-index: 60;
    transform: translateX(-100%);
    transition: transform var(--am-dauer-mittel) var(--am-kurve);
    box-shadow: var(--am-schatten-1);
  }
  .tk-app.narrow.sidebar-open .tk-sidebar { transform: translateX(0); }
  .tk-app.narrow .tk-scrim { position: fixed; inset: 0; background: var(--am-deckschicht); z-index: 55; }
  .tk-app.narrow .tk-sidebar-close,
  .tk-app.narrow .tk-menu-toggle { display: inline-flex; }
  .tk-app:not(.narrow) .tk-sidebar-close,
  .tk-app:not(.narrow) .tk-menu-toggle { display: none; }
  .tk-app.narrow .resize-handle { display: none; }
  .tk-app.narrow .tk-main { padding: 12px; }
  .tk-mobile-header { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
  .tk-mobile-header h1 { margin: 0; font-size: var(--fs-md); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; min-width: 0; }
  .tk-mobile-new { margin-left: auto; }
  .tk-nav-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px;
    background: none;
    border: none;
    color: var(--am-text-primaer);
    cursor: pointer;
    border-radius: var(--am-radius-mittel);
    font-size: 1.25rem;
  }
  .tk-nav-btn:hover { background: var(--am-flaeche-2); }
  @media (prefers-reduced-motion: reduce) {
    .tk-app.narrow .tk-sidebar { transition: none; }
  }
  .tk-app.narrow .tk-qa-kbd { display: none; }
</style>
