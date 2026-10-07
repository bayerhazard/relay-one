<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onMount } from "svelte";
  import {
    listTodos, createTodo, toggleTodo, patchTodo, deleteTodo, succeedTodo,
    quickAddTodo, todoViews, getCalendars,
    type TodoInfo, type TodoPatchInput,
  } from "$lib/services/tauri";
  import Huelle from "$lib/components/Huelle.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import SummaryLine from "$lib/components/SummaryLine.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { fmtDateByLang, localeTag as fmtLocaleTag } from "$lib/utils/format";
  import { t, translate } from "$lib/i18n";
  import { dataVersion } from "$lib/stores/invalidation";
  import { parseQuickAdd, PRIO_LABEL, type ParsedQuickAdd } from "$lib/utils/quickAdd";

  // On the phone a choice in the column closes the shell's sheet (RL-G2).
  let spalteOffen = $state(false);
  function schliesseSpalte() {
    spalteOffen = false;
  }

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

  /** Focus the Quick-Add field (empty state's way forward + global Q shortcut). */
  function focusQuickAdd() {
    schliesseSpalte();
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
    schliesseSpalte();
  }

  /** Toggle the tag filter (combined with the current view). */
  function selectTag(tag: string) {
    tagFilter = tagFilter === tag ? null : tag;
    schliesseSpalte();
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

<svelte:head><title>{tabTitel(selectionLabel())}</title></svelte:head>

<Huelle bereich="tasks" bind:spalteOffen bind:suche={tkSearch} suchePlatzhalter={$t("tasks.searchPlaceholder")}>
  {#snippet spalte()}
    <!-- The inside of the area (RL-G2): views, projects, tags. -->
    <nav class="tk-views" aria-label={$t("tasks.viewsLabel")}>
      <button type="button" class="tk-view" class:active={selection === "inbox"} aria-current={selection === "inbox" ? "page" : undefined} onclick={() => select("inbox")}>
        <Symbol name="eingang" size={20} />
        <span class="tk-view-label">{$t("tasks.viewInbox")}</span>
        {#if counts.inbox}<span class="tk-badge">{counts.inbox}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "today"} aria-current={selection === "today" ? "page" : undefined} onclick={() => select("today")}>
        <Symbol name="frist" size={20} />
        <span class="tk-view-label">{$t("tasks.viewToday")}</span>
        {#if counts.today}<span class="tk-badge">{counts.today}</span>{/if}
        {#if counts.overdue}<span class="tk-badge tk-badge-danger">{counts.overdue}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "upcoming"} aria-current={selection === "upcoming" ? "page" : undefined} onclick={() => select("upcoming")}>
        <Symbol name="zeitplan" size={20} />
        <span class="tk-view-label">{$t("tasks.viewUpcoming")}</span>
        {#if counts.upcoming}<span class="tk-badge">{counts.upcoming}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "all"} aria-current={selection === "all" ? "page" : undefined} onclick={() => select("all")}>
        <Symbol name="liste" size={20} />
        <span class="tk-view-label">{$t("tasks.viewAll")}</span>
        {#if counts.all}<span class="tk-badge">{counts.all}</span>{/if}
      </button>
      <button type="button" class="tk-view" class:active={selection === "done"} aria-current={selection === "done" ? "page" : undefined} onclick={() => select("done")}>
        <Symbol name="erledigt" size={20} />
        <span class="tk-view-label">{$t("tasks.viewDone")}</span>
        {#if counts.done}<span class="tk-badge">{counts.done}</span>{/if}
      </button>
    </nav>

    <!-- Projects (= CalDAV calendars) -->
    {#if projects.length}
      <div class="tk-projects" role="group" aria-labelledby="tk-projects-head">
        <div class="tk-gruppe-titel" id="tk-projects-head">{$t("tasks.projects")}</div>
        {#each projects as p (p.id)}
          <button type="button" class="tk-view" class:active={selection === `p:${p.id}`} aria-current={selection === `p:${p.id}` ? "page" : undefined} onclick={() => select(`p:${p.id}`)}>
            <span class="tk-zeichen" aria-hidden="true"><span class="tk-dot" style={`background:${p.color || "var(--am-text-gedaempft)"}`}></span></span>
            <span class="tk-view-label">{p.name}</span>
          </button>
        {/each}
      </div>
    {/if}

    {#if allTags.length}
      <div class="tk-projects tk-tags" role="group" aria-labelledby="tk-tags-head">
        <div class="tk-gruppe-titel" id="tk-tags-head">{$t("tasks.tags")}</div>
        {#each allTags as [tag, count] (tag)}
          <button type="button" class="tk-view" class:active={tagFilter === tag} aria-pressed={tagFilter === tag} onclick={() => selectTag(tag)}>
            <Symbol name="schlagwort" size={20} />
            <span class="tk-view-label">{tag}</span>
            <span class="tk-badge">{count}</span>
          </button>
        {/each}
      </div>
    {/if}
  {/snippet}

  <main class="tk-main">
    <!-- HB-SEITENKOPF: the chosen view at 28 px with its count; the sort
         as a quiet control. The page's one primary is "Anlegen" in the
         quick capture right below (RL-G4, G2). -->
    <div class="seitenkopf">
      <div class="seitenkopf-zeile">
        <h1>{selectionLabel()}</h1>
        <span class="seitenkopf-zahl">{visibleTodos.length}</span>
      </div>
      <div class="btn-reihe tk-sort-inline">
        <label for="tk-sort">{$t("tasks.sortLabel")}</label>
        <select id="tk-sort" class="input" value={sortMode} onchange={(e) => setSort((e.currentTarget as HTMLSelectElement).value)}>
          <option value="work">{$t("tasks.sortWork")}</option>
          <option value="due">{$t("tasks.sortDue")}</option>
          <option value="priority">{$t("tasks.sortPriority")}</option>
          <option value="title">{$t("tasks.sortTitle")}</option>
          <option value="created">{$t("tasks.sortCreated")}</option>
          <option value="manual">{$t("tasks.sortManual")}</option>
        </select>
      </div>
    </div>

    <div class="tk-inhalt">
    <!-- Quick Add: instant capture with live parse chips -->
    <div class="tk-quickadd" class:focused={qaFocused}>
      <span class="tk-qa-plus" aria-hidden="true">
        <Symbol name="plus" size={16} />
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
      <button type="button" class="btn btn-primaer btn-klein tk-qa-submit" onclick={submitQuickAdd} disabled={busy || !qaText.trim()}>
        {$t("tasks.add")}
      </button>
    </div>
    {#if qaText.trim()}
      <div class="tk-qa-chips" aria-live="polite">
        <span class="tk-chip tk-chip-title">{qaParsed.title || $t("tasks.untitled")}</span>
        {#if qaParsed.due}
          <span class="tk-chip">
            <Symbol name="kalender" size={16} />
            {fmtDateByLang(qaParsed.due, fmtLocaleTag())}{qaParsed.dueHasTime ? ` ${String(qaParsed.due.getHours()).padStart(2, "0")}:${String(qaParsed.due.getMinutes()).padStart(2, "0")}` : ""}
          </span>
        {/if}
        {#if qaParsed.rrule}
          <span class="tk-chip">
            <Symbol name="wiederholen" size={16} />
            {recurLabel(qaParsed.rrule)}
          </span>
        {/if}
        {#if qaParsed.priority !== null}<span class={`tk-chip tk-prio-chip ${prioClassByUi(qaParsed.priority)}`}>{PRIO_LABEL[qaParsed.priority]}</span>{/if}
        {#if qaParsed.project}<span class="tk-chip">#{qaParsed.project}</span>{/if}
        {#each qaParsed.labels as l (l)}<span class="tk-chip">{l}</span>{/each}
      </div>
    {/if}

    {#if nextSteps.length}
      <div class="karte tk-next">
        <div class="tk-next-head">{$t("tasks.nextSteps")}</div>
        <ol class="tk-next-list">
          {#each nextSteps as task (task.uid)}
            <li class="tk-next-item">
              <button type="button" class="tk-check tk-check-sm" onclick={() => onToggle(task)} aria-label={$t("tasks.markDone")} title={$t("tasks.markDone")}></button>
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
      <div class="tk-state">
        <div class="hinweis" data-art="fehler" role="alert"><Symbol name="achtung" size={16} /><span>{error}</span></div>
        <button type="button" class="btn btn-sekundaer" onclick={loadAll}>{$t("tasks.reload")}</button>
      </div>
    {:else if visibleTodos.length === 0}
      {#if tkSearch}
        <EmptyState icon="suche" title={$t("tasks.notFound")} subtitle={$t("tasks.notFoundHint")} actionLabel={$t("tasks.clearSearch")} onaction={() => (tkSearch = "")} />
      {:else}
        <EmptyState icon="aufgabe" title={$t("tasks.viewEmpty")} subtitle={$t("tasks.viewEmptyHint")} actionLabel={$t("tasks.new")} onaction={focusQuickAdd} />
      {/if}
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
                  title={isDone(todo) ? $t("tasks.reopen") : $t("tasks.markDone")}
                >
                  {#if isDone(todo)}<Symbol name="erfolg" size={16} />{/if}
                </button>
                <button type="button" class="tk-item-body" onclick={() => openDetail(todo)}>
                  <span class="tk-item-summary">{todo.summary || $t("tasks.untitled")}</span>
                  {#if todo.description}<SummaryLine summary={todo.description} />{/if}
                  {#if todo.due_at || todo.labels.length || todo.rrule || isBlocked(todo)}
                    <span class="tk-item-meta">
                      {#if isBlocked(todo)}
                        <span class="tk-item-blocked" title={$t("tasks.blockedHint")}><Symbol name="schloss" size={16} /> {openBlockers(todo).length}</span>
                      {/if}
                      {#if todo.due_at}
                        <span class="tk-item-due" class:overdue={isOverdue(todo)}>{dueLabel(todo)}</span>
                      {/if}
                      {#if todo.rrule}
                        <span class="tk-item-rep" title={recurLabel(todo.rrule)}>
                          <Symbol name="wiederholen" size={16} />
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
                      <button type="button" class="tk-check tk-check-sm" class:checked={isDone(sub)} onclick={() => onToggle(sub)} aria-label={isDone(sub) ? $t("tasks.reopen") : $t("tasks.markDone")} title={isDone(sub) ? $t("tasks.reopen") : $t("tasks.markDone")}>
                        {#if isDone(sub)}<Symbol name="erfolg" size={16} />{/if}
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
    </div>
  </main>
</Huelle>

<!-- Detail dialog (HB-DIALOG). Escape is also handled globally (onGlobalKey). -->
{#if detail}
  <div
    class="dialog-schicht"
    role="dialog"
    aria-modal="true"
    aria-labelledby="tk-detail-title"
    tabindex="-1"
    onclick={(e) => { if ((e.target as HTMLElement).classList.contains("dialog-schicht")) closeDetail(); }}
    onkeydown={(e) => { if (e.key === "Escape") closeDetail(); }}
  >
    <div class="karte dialog-karte" data-breite="normal">
      <div class="dialog-kopf">
        <h2 id="tk-detail-title">{$t("tasks.details")}</h2>
        <button type="button" class="dialog-zu" onclick={closeDetail} aria-label={$t("tasks.closeDialog")} title={$t("tasks.closeDialog")}><Symbol name="schliessen" size={20} /></button>
      </div>

      <div class="dialog-koerper">
        <div class="feld">
          <label for="tk-edit-summary">{$t("tasks.taskLabel")}</label>
          <input id="tk-edit-summary" type="text" bind:value={edit.summary} />
        </div>

        <div class="feld">
          <label for="tk-edit-description">{$t("tasks.description")}</label>
          <textarea id="tk-edit-description" rows="3" bind:value={edit.description} placeholder={$t("tasks.phOptional")}></textarea>
        </div>

        <div class="feld-paar">
          <div class="feld">
            <label for="tk-edit-due">{$t("tasks.dueDate")}</label>
            <input id="tk-edit-due" type="date" bind:value={edit.due} />
          </div>
          <div class="feld">
            <label for="tk-edit-time">{$t("tasks.dueTime")}</label>
            <input id="tk-edit-time" type="time" bind:value={edit.dueTime} />
          </div>
        </div>

        <div class="feld">
          <label for="tk-edit-priority">{$t("tasks.priorityLabel")}</label>
          <select
            id="tk-edit-priority"
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
        </div>

        <div class="feld">
          <label for="tk-edit-repeat">{$t("tasks.repeatLabel")}</label>
          <select
            id="tk-edit-repeat"
            value={recurToken(edit.rrule)}
            onchange={(e) => (edit.rrule = rruleFromToken((e.currentTarget as HTMLSelectElement).value))}
          >
            <option value="">{$t("tasks.repeatNone")}</option>
            <option value="daily">{$t("tasks.repeatDaily")}</option>
            <option value="weekly">{$t("tasks.repeatWeekly")}</option>
            <option value="monthly">{$t("tasks.repeatMonthly")}</option>
            <option value="yearly">{$t("tasks.repeatYearly")}</option>
          </select>
        </div>

        <div class="feld">
          <label for="tk-edit-project">{$t("tasks.projectLabel")}</label>
          <select id="tk-edit-project" bind:value={edit.projectId}>
            <option value={null}>{$t("tasks.viewInbox")}</option>
            {#each projects as p (p.id)}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>
        </div>

        <div class="feld">
          <label for="tk-edit-labels">{$t("tasks.labelsLabel")}</label>
          <input id="tk-edit-labels" type="text" bind:value={edit.labelsText} placeholder={$t("tasks.phLabels")} />
        </div>

        <div class="feld">
          <label for="tk-edit-deps">{$t("tasks.dependsOnLabel")}</label>
          <select id="tk-edit-deps" multiple bind:value={edit.dependencies} size={Math.min(5, Math.max(2, blockerOptions.length))}>
            {#each blockerOptions as b (b.uid)}
              <option value={b.uid}>{b.summary || $t("tasks.untitled")}</option>
            {/each}
          </select>
        </div>
      </div>

      <div class="dialog-fuss">
        <button type="button" class="btn btn-primaer" onclick={saveDetail} disabled={busy}>{busy ? $t("tasks.saving") : $t("common.save")}</button>
        <button type="button" class="btn btn-sekundaer" onclick={() => openSubtaskFor(detail!)}>{$t("tasks.subtaskAdd")}</button>
        <button type="button" class="btn btn-gefahr tk-delete" onclick={() => askDelete(detail!)}>{$t("tasks.deleteBtn")}</button>
      </div>
    </div>
  </div>
{/if}

<!-- Sub-task creation dialog (HB-DIALOG) -->
{#if subtaskParent}
  <div
    class="dialog-schicht"
    role="dialog"
    aria-modal="true"
    aria-labelledby="tk-subtask-title"
    tabindex="-1"
    onclick={(e) => { if ((e.target as HTMLElement).classList.contains("dialog-schicht") && !busy) subtaskParent = null; }}
    onkeydown={(e) => { if (e.key === "Escape") subtaskParent = null; }}
  >
    <div class="karte dialog-karte" data-breite="schmal">
      <div class="dialog-kopf">
        <h2 id="tk-subtask-title">{$t("tasks.subtaskAdd")}</h2>
        <button type="button" class="dialog-zu" onclick={() => (subtaskParent = null)} disabled={busy} aria-label={$t("tasks.closeDialog")} title={$t("tasks.closeDialog")}><Symbol name="schliessen" size={20} /></button>
      </div>
      <div class="dialog-koerper">
        <p class="tk-modal-sub">{subtaskParent.summary || $t("tasks.untitled")}</p>
        <input
          type="text"
          class="input"
          bind:value={subtaskTitle}
          placeholder={$t("tasks.phSubtask")}
          aria-label={$t("tasks.subtaskAdd")}
          onkeydown={(e) => { if (e.key === "Enter") void saveSubtask(); }}
        />
      </div>
      <div class="dialog-fuss">
        <button type="button" class="btn btn-primaer" onclick={saveSubtask} disabled={busy || !subtaskTitle.trim()}>{$t("common.save")}</button>
        <button type="button" class="btn btn-sekundaer" onclick={() => (subtaskParent = null)} disabled={busy}>{$t("common.cancel")}</button>
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
  /* ── Views in the column [RL-AUFGABEN] ───────────────────────────────────
     The inside of the area under the five areas (RL-G2), rows like
     HB-UNTERNAV / HB-NAVIGATION: 40 px, 20 px signs, the chosen one with the
     gold edge, bold, sign in gold, no surface. The same in the phone sheet. */
  .tk-views,
  .tk-projects { display: flex; flex-direction: column; gap: 2px; padding: var(--am-raum-2) var(--am-raum-4); }
  .tk-projects { border-top: 1px solid var(--am-trennlinie); }
  .tk-view {
    display: flex;
    align-items: center;
    gap: var(--am-raum-3);
    width: 100%;
    min-height: var(--am-ziel-zeiger);
    padding: var(--am-raum-2) var(--am-raum-3);
    border: none;
    background: none;
    color: var(--am-text-sekundaer);
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    font-size: 0.875rem;
    font-family: inherit;
    text-align: left;
  }
  .tk-view > :global(svg) { flex: none; }
  .tk-view:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .tk-view:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }
  .tk-view.active {
    background: transparent;
    color: var(--am-text-primaer);
    font-weight: 600;
    box-shadow: inset 2px 0 0 var(--am-gold-auszeichnung);
    border-radius: 0 var(--am-radius-mittel) var(--am-radius-mittel) 0;
  }
  .tk-view.active:hover { background: var(--am-flaeche-2); }
  .tk-view.active > :global(svg) { color: var(--am-gold-beschriftung); }
  .tk-view-label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* The project's calendar colour, in the 20 px place of a sign. */
  .tk-zeichen { width: 20px; height: 20px; flex: none; display: inline-flex; align-items: center; justify-content: center; }
  .tk-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .tk-badge {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--am-text-gedaempft);
    background: var(--am-flaeche-2);
    border-radius: 999px;
    padding: 0 7px;
    min-width: 20px;
    text-align: center;
  }
  .tk-badge-danger { color: var(--am-fehler); }
  /* Group title as .huelle-nav-titel draws it — that class shows only in
     the desktop column, this one also in the phone sheet. */
  .tk-gruppe-titel {
    margin: 0;
    padding: var(--am-raum-3) var(--am-raum-3) var(--am-raum-1);
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--am-text-gedaempft);
  }

  /* ── Page and quick capture [RL-AUFGABEN] ─────────────────────────────── */
  .tk-main { display: flex; flex-direction: column; min-height: 0; min-width: 0; overflow-y: auto; }
  /* A column, so the empty and loading states can fill the rest. */
  .tk-inhalt { flex: 1; display: flex; flex-direction: column; padding: var(--am-raum-6) var(--am-raum-8); }
  .tk-inhalt > :global(.leerzustand) { flex: 1; height: auto; }

  /* A composite field (icon, input, key hint, button) and Relay's own;
     surface, border and focus ring follow AM-FELD's .input. */
  .tk-quickadd {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 4px 4px 12px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-seite);
    margin-bottom: var(--am-raum-4);
  }
  .tk-quickadd.focused { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }
  .tk-qa-plus { display: inline-flex; color: var(--am-handlung-ruhend); flex-shrink: 0; }
  .tk-qa-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--am-text-primaer);
    font-size: 0.9375rem;
    font-family: var(--am-schrift-sans);
    outline: none;
    padding: 6px 0;
  }
  .tk-qa-input::placeholder { color: var(--am-text-deaktiviert); }
  .tk-qa-kbd {
    font-size: 0.7rem;
    color: var(--am-text-gedaempft);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    padding: 1px 6px;
    flex-shrink: 0;
  }
  .tk-qa-submit { flex-shrink: 0; }

  .tk-qa-chips { display: flex; flex-wrap: wrap; gap: 6px; margin: calc(-1 * var(--am-raum-2)) 0 var(--am-raum-4); }
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

  /* ── Sort, next steps and states [RL-AUFGABEN] ─────────────────── */
  /* Sort control in the page head: the select is an AM-FELD .input, sized
     to its text. */
  .tk-sort-inline {
    display: inline-flex; align-items: center; gap: 8px;
    font-size: var(--fs-xs); color: var(--am-text-gedaempft); flex-shrink: 0;
  }
  .tk-sort-inline select { width: auto; }

  /* "Next steps" strip (Today view), an AM-KARTE. */
  .tk-next { margin-bottom: 16px; }
  .tk-next-head { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.04em; color: var(--am-text-gedaempft); margin-bottom: 8px; }
  .tk-next-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .tk-next-item { display: flex; align-items: center; gap: 10px; }
  .tk-next-title {
    flex: 1; min-width: 0; text-align: left; background: none; border: none; cursor: pointer;
    color: var(--am-text-primaer); font-size: var(--fs-sm); font-family: inherit;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .tk-next-title:hover { color: var(--am-handlung-ruhend); }

  .tk-group-head {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
    padding: 14px 4px 6px;
  }

  /* Loading and error: centred in the list area; HB-ZUSTAND draws the error. */
  .tk-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    flex: 1;
    color: var(--am-text-gedaempft);
    font-size: var(--fs-base);
  }

  /* ── Task rows [RL-AUFGABEN] ──────────────────────────────────────────── */
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

  /* The round completion check is part of the row, not an AM-HAKEN box:
     it is a button that completes (or spawns the next occurrence). */
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

  /* ── Task dialogs [RL-AUFGABEN] ───────────────────────────────────────── */
  /* Frame, fields and buttons come from HB-DIALOG, AM-FELD and AM-KNOPF;
     footer as in HB-DIALOG: "Speichern" first, the destructive step at the
     far end. */
  .tk-delete { margin-left: auto; }
  .tk-modal-sub { margin: 0 0 var(--am-raum-3); font-size: var(--fs-xs); color: var(--am-text-gedaempft); }

  /* ── Narrow [RL-AUFGABEN] ─────────────────────────────────────────────── */
  @media (max-width: 40rem) {
    .tk-inhalt { padding: var(--am-raum-4); }
    .tk-qa-kbd { display: none; }
  }
</style>
