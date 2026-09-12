<script lang="ts">
  import { onMount } from "svelte";
  import {
    listMeetings, getMeeting, triggerMeetingScan, openEventStream,
    getMeetingFollowups, createPlanFromSuggestion,
    type MeetingInfo, type MeetingDetail, type FollowupSuggestion,
  } from "$lib/services/tauri";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import ModuleIcons from "$lib/components/ModuleIcons.svelte";
  import SidebarSearch from "$lib/components/SidebarSearch.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import { assistantCommand } from "$lib/stores/assistantCommand";
  import { isFollowupDoneKey, meetingFollowupKey } from "$lib/utils/followupMemory";
  import { useSidebarResize } from "$lib/composables/useSidebarResize";
  import { t, translate } from "$lib/i18n";
  import { renderMarkdown } from "$lib/utils/markdown";

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

  let meetings = $state<MeetingInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let scanning = $state(false);
  let scanMsg = $state<string | null>(null);
  let search = $state("");
  let selectedId = $state<number | null>(null);
  let detail = $state<MeetingDetail | null>(null);
  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  // Letztes geladenes Suchfeld — der Effect feuert nur bei echter Änderung.
  let loadedQuery = $state("");

  // Debounced server-seitige Suche: bind:value hält `search` aktuell.
  $effect(() => {
    if (search === loadedQuery) return;
    const timer = setTimeout(() => void loadMeetings(), 300);
    return () => clearTimeout(timer);
  });

  async function loadMeetings() {
    loading = true;
    error = null;
    try {
      meetings = await listMeetings({ query: search.trim() || undefined, limit: 200 });
      loadedQuery = search;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function selectMeeting(m: MeetingInfo) {
    selectedId = m.id;
    detail = null;
    detailError = null;
    detailLoading = true;
    if (isNarrow) sidebarOpen = false;
    try {
      detail = await getMeeting(m.id);
    } catch (e: unknown) {
      detailError = e instanceof Error ? e.message : String(e);
    } finally {
      detailLoading = false;
    }
  }

  async function handleScan() {
    if (scanning) return;
    scanning = true;
    scanMsg = null;
    try {
      const report = await triggerMeetingScan();
      const n = report.inserted + report.updated + report.deleted;
      scanMsg = translate("meetings.scanDone", { n });
      await loadMeetings();
    } catch (e: unknown) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      scanning = false;
    }
  }

  function fmtDate(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleDateString(undefined, { year: "numeric", month: "2-digit", day: "2-digit" });
  }

  function fmtDateTime(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleString(undefined, {
      year: "numeric", month: "2-digit", day: "2-digit",
      hour: "2-digit", minute: "2-digit",
    });
  }

  let detailHtml = $derived(detail ? renderMarkdown(detail.body_md) : "");

  // AI-Followups (Tasks/Termine) aus der Meeting-Zusammenfassung — on-demand
  // beim Öffnen generiert, pro Meeting-ID gecacht; bereits ausgeführte
  // Vorschläge bleiben ausgeblendet (persistente Memory, Key "meeting-<id>").
  let followups = $state<FollowupSuggestion[]>([]);
  let followupsLoading = $state(false);
  let followupsError = $state<string | null>(null);
  let followupsForId = $state<number | null>(null);
  let followupsInFlight: number | null = null; // nicht reaktiv, nur Duplikat-Guard
  const followupsCache = new Map<number, FollowupSuggestion[]>();
  let followupPlanBusy = $state(false);

  function filterDoneFollowups(id: number, actions: FollowupSuggestion[]): FollowupSuggestion[] {
    const key = meetingFollowupKey(id);
    return actions.filter((a) => !isFollowupDoneKey(key, a));
  }

  $effect(() => {
    const id = detail?.id ?? null;
    if (id == null) {
      followups = [];
      followupsForId = null;
      return;
    }
    if (followupsForId === id) return;
    followupsForId = id;
    const cached = followupsCache.get(id);
    if (cached) {
      followups = filterDoneFollowups(id, cached);
      followupsLoading = false;
      followupsError = null;
      return;
    }
    if (followupsInFlight === id) return;
    followupsInFlight = id;
    followupsLoading = true;
    followupsError = null;
    followups = [];
    getMeetingFollowups(id)
      .then((res) => {
        if (followupsForId !== id) return;
        followupsCache.set(id, res.actions);
        followups = filterDoneFollowups(id, res.actions);
      })
      .catch((e: unknown) => {
        if (followupsForId !== id) return;
        followupsError = e instanceof Error ? e.message : String(e);
        followups = [];
      })
      .finally(() => {
        if (followupsInFlight === id) followupsInFlight = null;
        if (followupsForId === id) followupsLoading = false;
      });
  });

  async function handleFollowupChip(s: FollowupSuggestion) {
    if (followupPlanBusy || !detail) return;
    followupPlanBusy = true;
    try {
      const plan = await createPlanFromSuggestion(s, {
        sourceMessageId: detail.id,
        origin: "meeting_followup",
      });
      assistantCommand.showPlan(plan);
    } catch (e: unknown) {
      followupsError = e instanceof Error ? e.message : String(e);
    } finally {
      followupPlanBusy = false;
    }
  }

  onMount(() => {
    void loadMeetings();
    // Live-Update: der Server feuert "meetings-changed" nach jedem Scan,
    // der Änderungen gefunden hat.
    const es = openEventStream((event) => {
      if (event === "meetings-changed") void loadMeetings();
    });
    return () => es?.close();
  });
</script>

<div class="mt-app" class:narrow={isNarrow} class:sidebar-open={isNarrow && sidebarOpen}>
  {#if isNarrow && sidebarOpen}
    <div class="mt-scrim" role="presentation" onclick={() => (sidebarOpen = false)}></div>
  {/if}

  <!-- SIDEBAR -->
  <aside class="mt-sidebar" style={isNarrow ? "" : `width: ${$sidebarWidth}px; min-width: ${$sidebarWidth}px;`}>
    <div class="mt-sidebar-header">
      {#if isNarrow}
        <button type="button" class="mt-nav-btn mt-sidebar-close" onclick={() => (sidebarOpen = false)} aria-label={$t("meetings.close")}>←</button>
      {/if}
      <ModuleLogo to="/" label={$t("meetings.title")} noHover />
    </div>

    <div class="mt-tools">
      <button type="button" class="mt-btn mt-btn-ghost" onclick={handleScan} disabled={scanning}>
        {scanning ? $t("meetings.scanning") : $t("meetings.scan")}
      </button>
    </div>

    {#if scanMsg}
      <div class="mt-scan-msg">{scanMsg}</div>
    {/if}

    <div class="mt-count">{$t("meetings.count", { n: meetings.length })}</div>

    <div class="mt-list">
      {#if loading}
        <div class="mt-state">{$t("meetings.loading")}</div>
      {:else if error}
        <div class="mt-state mt-state-error">
          <p>{error}</p>
          <button type="button" class="mt-btn mt-btn-ghost" onclick={loadMeetings}>{$t("meetings.reload")}</button>
        </div>
      {:else if meetings.length === 0}
        <div class="mt-state">{$t("meetings.empty")}</div>
      {:else}
        {#each meetings as m (m.id)}
          <button
            type="button"
            class="mt-item"
            class:active={selectedId === m.id}
            onclick={() => selectMeeting(m)}
          >
            <span class="mt-item-date">{fmtDate(m.meeting_date)}</span>
            <span class="mt-item-title">{m.title}</span>
            <span class="mt-item-meta">
              {$t("meetings.duration", { n: m.duration_min })}
              {#if m.tags.length > 0}<span class="mt-item-tags">{m.tags.join(", ")}</span>{/if}
            </span>
          </button>
        {/each}
      {/if}
    </div>

    <div class="mt-sidebar-footer">
      <SidebarSearch
        bind:value={search}
        placeholder={$t("meetings.searchPlaceholder")}
        ariaLabel={$t("meetings.searchLabel")}
        clearLabel={$t("meetings.clearSearch")}
      />
      <div class="mt-module-row">
        <ModuleIcons active="meetings" />
      </div>
    </div>
  </aside>
  {#if !isNarrow}
    <div class="resize-handle" role="separator" aria-orientation="vertical" onmousedown={startResize}></div>
  {/if}

  <!-- MAIN -->
  <main class="mt-main">
    {#if isNarrow}
      <div class="mt-mobile-header">
        <button type="button" class="mt-nav-btn mt-menu-toggle" onclick={() => (sidebarOpen = true)} aria-label={$t("meetings.menu")}>☰</button>
        <h1>{$t("meetings.title")}</h1>
      </div>
    {/if}

    {#if detailLoading}
      <div class="mt-state">{$t("meetings.loading")}</div>
    {:else if detailError}
      <div class="mt-state mt-state-error">
        <p>{detailError}</p>
      </div>
    {:else if !detail}
      <div class="mt-state">{$t("meetings.selectHint")}</div>
    {:else}
      <article class="mt-detail">
        <header class="mt-detail-header">
          <h1>{detail.title}</h1>
          <div class="mt-detail-meta">
            <span>{fmtDateTime(detail.meeting_date)}</span>
            <span>·</span>
            <span>{$t("meetings.duration", { n: detail.duration_min })}</span>
            {#if detail.template}<span>·</span><span>{$t("meetings.template")}: {detail.template}</span>{/if}
            {#if detail.language}<span>·</span><span>{$t("meetings.language")}: {detail.language}</span>{/if}
          </div>
        </header>

        {#if detail.participants.length > 0}
          <section class="mt-section">
            <h2>{$t("meetings.participants")}</h2>
            <ul class="mt-participants">
              {#each detail.participants as p (p)}
                <li>{p}</li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if detail.tags.length > 0}
          <section class="mt-section mt-tags-row">
            {#each detail.tags as tag (tag)}
              <span class="mt-tag">{tag}</span>
            {/each}
          </section>
        {/if}

        <section class="mt-section mt-body">
          {#if detail.body_md.length > 0}
            {@html detailHtml}
          {:else}
            <p class="mt-body-empty">—</p>
          {/if}
        </section>

        <section class="mt-section mt-followups">
          <h2>{$t("meetings.followupsTitle")}</h2>
          {#if followupsLoading}
            <div class="mt-followups-row"><span class="mt-followups-muted">{$t("meetings.followupsLoading")}</span></div>
          {:else if followupsError}
            <div class="mt-followups-row"><span class="mt-followups-muted">{followupsError}</span></div>
          {:else if followups.length === 0}
            <div class="mt-followups-row"><span class="mt-followups-muted">{$t("meetings.followupsEmpty")}</span></div>
          {:else}
            {#each followups as a (a.id)}
              <div class="mt-followups-row">
                <div class="mt-followups-label">
                  <span>{a.titel}</span>
                </div>
                <button
                  type="button"
                  class="mt-btn mt-followups-btn"
                  disabled={followupPlanBusy}
                  onclick={() => handleFollowupChip(a)}
                >
                  {$t("meetings.followupsAccept")}
                </button>
              </div>
            {/each}
          {/if}
        </section>

        {#if detail.source_url}
          <footer class="mt-detail-footer">
            <a class="mt-link" href={detail.source_url} target="_blank" rel="noopener noreferrer">
              {$t("meetings.openInInsilo")}
            </a>
          </footer>
        {/if}
      </article>
    {/if}
  </main>
</div>

<AssistantFab module="meetings" />

<style>
  .mt-app {
    display: flex;
    height: 100vh;
    background: var(--color-list);
    color: var(--color-text);
  }
  .mt-sidebar {
    flex-shrink: 0;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .mt-sidebar-header {
    height: var(--am-header-h);
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    margin-bottom: 12px;
  }
  .mt-nav-btn {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-s);
    font-size: 1rem;
  }
  .mt-nav-btn:hover { color: var(--color-text); background: var(--color-active-wash); }

  .mt-tools { padding: 0 12px 8px; display: flex; gap: 8px; }
  .mt-btn {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 7px 12px;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
    background: var(--color-card);
    color: var(--color-text);
  }
  .mt-btn:disabled { opacity: 0.5; cursor: default; }
  .mt-btn-ghost { background: none; }
  .mt-btn-ghost:hover:not(:disabled) { background: var(--color-active-wash); }

  .mt-scan-msg {
    padding: 0 12px 6px;
    font-size: 0.78rem;
    color: var(--color-text-secondary);
  }
  .mt-count {
    padding: 4px 12px 8px;
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    border-bottom: 1px solid var(--color-border);
  }

  .mt-list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 4px 8px;
  }
  .mt-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    padding: 10px;
    cursor: pointer;
    color: var(--color-text);
  }
  .mt-item:hover { background: var(--color-active-wash); }
  .mt-item.active { background: var(--color-active-wash); }
  .mt-item-date {
    font-size: 0.72rem;
    color: var(--color-text-secondary);
    font-variant-numeric: tabular-nums;
  }
  .mt-item-title {
    font-size: 0.88rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mt-item-meta {
    font-size: 0.72rem;
    color: var(--color-text-secondary);
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .mt-item-tags {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mt-state {
    padding: 24px 16px;
    font-size: 0.85rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .mt-state-error { color: var(--color-danger, #c0392b); }
  .mt-state-error .mt-btn { margin-top: 10px; }

  .mt-sidebar-footer {
    border-top: 1px solid var(--color-border);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
  }
  .mt-module-row { display: flex; }

  .mt-main {
    flex: 1;
    overflow-y: auto;
    min-width: 0;
    background: var(--color-list);
  }
  .mt-detail {
    max-width: 860px;
    margin: 0 auto;
    padding: 32px 40px 64px;
  }
  .mt-detail-header h1 {
    font-size: 1.4rem;
    font-weight: 600;
    margin: 0 0 6px;
  }
  .mt-detail-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 0.8rem;
    color: var(--color-text-secondary);
    margin-bottom: 24px;
  }
  .mt-section { margin-bottom: 24px; }
  .mt-section h2 {
    font-size: 0.8rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-secondary);
    margin: 0 0 8px;
  }
  .mt-followups-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--color-border);
  }
  .mt-followups-row:last-child { border-bottom: none; }
  .mt-followups-label { font-size: 0.9rem; }
  .mt-followups-muted { color: var(--color-text-secondary); font-size: 0.85rem; }
  .mt-followups-btn { flex-shrink: 0; }
  .mt-participants {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .mt-participants li {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 3px 10px;
    font-size: 0.8rem;
  }
  .mt-tags-row { display: flex; flex-wrap: wrap; gap: 6px; }
  .mt-tag {
    background: var(--color-active-wash);
    border-radius: var(--radius-s);
    padding: 3px 10px;
    font-size: 0.75rem;
    color: var(--color-accent);
  }

  .mt-body { font-size: 0.92rem; line-height: 1.6; }
  .mt-body-empty { color: var(--color-text-secondary); }
  .mt-body :global(h1), .mt-body :global(h2), .mt-body :global(h3) {
    margin: 1.2em 0 0.5em;
    line-height: 1.3;
  }
  .mt-body :global(h1) { font-size: 1.15rem; }
  .mt-body :global(h2) { font-size: 1.05rem; }
  .mt-body :global(h3) { font-size: 0.95rem; }
  .mt-body :global(p) { margin: 0.6em 0; }
  .mt-body :global(ul), .mt-body :global(ol) { margin: 0.6em 0; padding-left: 1.4em; }
  .mt-body :global(li) { margin: 0.25em 0; }
  .mt-body :global(code) {
    background: var(--color-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 1px 5px;
    font-size: 0.85em;
  }
  .mt-body :global(a) { color: var(--color-accent); }

  .mt-detail-footer {
    margin-top: 32px;
    padding-top: 16px;
    border-top: 1px solid var(--color-border);
  }
  .mt-link {
    color: var(--color-accent);
    font-size: 0.85rem;
    text-decoration: none;
  }
  .mt-link:hover { text-decoration: underline; }

  .mt-mobile-header { display: none; }
  .mt-scrim { display: none; }
  .mt-app.narrow .resize-handle { display: none; }

  @media (max-width: 768px) {
    .mt-mobile-header {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 0 12px;
      height: var(--am-header-h);
      border-bottom: 1px solid var(--color-border);
    }
    .mt-mobile-header h1 { font-size: 1rem; margin: 0; }
    .mt-sidebar {
      position: fixed;
      inset: 0 auto 0 0;
      width: min(85vw, 340px) !important;
      min-width: 0 !important;
      z-index: 40;
      transform: translateX(-100%);
      transition: transform 0.2s ease-in-out;
    }
    .mt-app.sidebar-open .mt-sidebar { transform: translateX(0); }
    .mt-app.sidebar-open .mt-scrim {
      display: block;
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.4);
      z-index: 30;
    }
    .mt-detail { padding: 20px 16px 48px; }
  }
</style>
