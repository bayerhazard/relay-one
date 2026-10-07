<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onMount } from "svelte";
  import {
    listMeetings, getMeeting, triggerMeetingScan, deleteMeeting, openEventStream,
    getMeetingFollowups, createPlanFromSuggestion,
    type MeetingInfo, type MeetingDetail, type FollowupSuggestion,
  } from "$lib/services/tauri";
  import Huelle from "$lib/components/Huelle.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import { goto } from "$app/navigation";
  import { base } from "$app/paths";
  import { assistantCommand } from "$lib/stores/assistantCommand";
  import { assistantAction } from "$lib/stores/assistantAction";
  import { selection } from "$lib/stores/selection";
  import { isFollowupDoneKey, meetingFollowupKey } from "$lib/utils/followupMemory";
  import { t, translate } from "$lib/i18n";
  import { renderMarkdown } from "$lib/utils/markdown";
  import { fmtDateByLang, localeTag } from "$lib/utils/format";

  let meetings = $state<MeetingInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let scanning = $state(false);
  let search = $state("");
  let selectedId = $state<number | null>(null);
  // Phones show one pane at a time, as the mail does: the list, or the
  // detail of the chosen meeting with a way back. On wider screens both
  // panes stand side by side and this only steers the tab title.
  let detailOffen = $state(false);

  // Rechtsklick (T3, Review 2026-09-13): Kontextmenü auf der Meetingliste.
  let ctxMenu = $state<{ x: number; y: number; meeting: MeetingInfo } | null>(null);
  let ctxItems = $derived.by(() => {
    const c = ctxMenu;
    if (!c) return [];
    return [
      {
        label: translate("meetings.emailMinutes"),
        action: () => {
          selectedId = c.meeting.id;
          void emailMinutesFor(c.meeting);
        },
      },
      { label: translate("meetings.delete"), danger: true, action: () => askDeleteFor(c.meeting) },
    ];
  });
  let detail = $state<MeetingDetail | null>(null);
  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  // Letztes geladenes Suchfeld — der Effect feuert nur bei echter Änderung.
  let loadedQuery = $state("");

  // Debounced server-seitige Suche: the field sits in the header now (CI
  // HB-SUCHE, RL-G1); bind:suche keeps `search` current.
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
    detailOffen = true;
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
    try {
      await triggerMeetingScan();
      await loadMeetings();
    } catch {
      // Scan-Fehler bleiben unsichtbar — die Liste lädt sich ohnehin neu.
    } finally {
      scanning = false;
    }
  }

  // Meeting aus Relay entfernen (Soft-Delete + Tombstone: der nächste Scan
  // stellt es nicht wieder her, solange die Insilo-Datei existiert).
  async function handleDelete() {
    if (!detail) return;
    // In-app Bestätigung (S4, Review 2026-09-13) — kein natives confirm().
    deleteTarget = { id: detail.id, title: detail.title };
  }

  // Der Export trägt eine YAML-Frontmatter (Metadaten) vor dem eigentlichen
  // Text — die gehört nicht in den Mail-Body.
  function stripFrontmatter(md: string): string {
    return md.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/, "");
  }

  // "Minutes per Mail senden" öffnet das in-app-Compose im Mail-Modul mit
  // vorbefülltem Betreff und der Zusammenfassung als Text (gleicher Hand-off
  // wie der Assistent: Aktion setzen, dann ins Mail-Modul navigieren).
  // Kontextmenü-Varianten (S4+T3): gleiche Logik wie Detail-Buttons, aber
  // bezogen auf eine MeetingInfo aus der Liste statt auf das offene Detail.
  let deleteTarget = $state<{ id: number; title: string } | null>(null);
  function askDeleteFor(m: MeetingInfo): void {
    deleteTarget = { id: m.id, title: m.title };
  }
  async function handleDeleteTarget() {
    const t = deleteTarget;
    if (!t) return;
    deleteTarget = null;
    try {
      await deleteMeeting(t.id);
      if (detail && detail.id === t.id) { detail = null; selectedId = null; detailOffen = false; }
      await loadMeetings();
    } catch {
      // Fehler bleibt unsichtbar; Liste ist ohnehin frisch geladen.
    }
  }
  async function emailMinutesFor(m: MeetingInfo) {
    try {
      const d = await getMeeting(m.id);
      assistantAction.set({
        type: "open_compose",
        to: "",
        subject: translate("meetings.minutesSubject", { title: d.title, date: fmtDate(d.meeting_date) }),
        body: stripFrontmatter(d.body_md),
      });
      await goto(base + "/");
    } catch {
      // Detail-Fetch fehlgeschlagen → kein Versand.
    }
  }

  async function emailMinutes() {
    if (!detail) return;
    assistantAction.set({
      type: "open_compose",
      to: "",
      subject: translate("meetings.minutesSubject", { title: detail.title, date: fmtDate(detail.meeting_date) }),
      body: stripFrontmatter(detail.body_md),
    });
    await goto(base + "/");
  }

  function fmtDate(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    // T15 (Review 2026-09-13): einheitliches Datumsformat über die App-Locale.
    return fmtDateByLang(d, localeTag());
  }

  function fmtDateTime(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleString(undefined, {
      year: "numeric", month: "2-digit", day: "2-digit",
      hour: "2-digit", minute: "2-digit",
    });
  }

  let detailHtml = $derived(detail ? renderMarkdown(stripFrontmatter(detail.body_md)) : "");

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

  // Assistent-Effekt "meetings.open": wenn eine Meeting-Selektion von außen
  // ankommt (Concept §10.2), Detail laden und 2 s goldene Markierung.
  $effect(() => {
    const target = $selection.meeting;
    if (target == null) return;
    if (selectedId === target && detail) {
      selection.clearHighlight();
      return;
    }
    const m = meetings.find((x) => x.id === target);
    if (!m) return;
    void selectMeeting(m);
    selection.clearHighlight();
  });
</script>

<svelte:head><title>{tabTitel(detailOffen && detail ? detail.title : $t("meetings.title"))}</title></svelte:head>

<Huelle bereich="meetings" bind:suche={search} suchePlatzhalter={$t("meetings.searchPlaceholder")}>
<!-- No inner navigation (`spalte`): the list of meetings is content, so it
     stands as the left pane, the detail right — two panes like the mail. -->
<div class="mt-app" class:detail-offen={detailOffen}>
  <!-- HB-SEITENKOPF: title 28 px with the count; no primary — meetings come
       from Insilo, "Aktualisieren" only fetches them. -->
  <div class="seitenkopf">
    <div class="seitenkopf-zeile">
      <h1>{$t("meetings.title")}</h1>
      <span class="seitenkopf-zahl">{meetings.length}</span>
    </div>
    <div class="btn-reihe">
      <button type="button" class="btn btn-still" onclick={handleScan} disabled={scanning}>
        {scanning ? $t("meetings.scanning") : $t("meetings.scan")}
      </button>
    </div>
  </div>

  <div class="mt-panes">
  <!-- LIST -->
  <section class="mt-list-pane" aria-label={$t("meetings.title")}>
    <div class="mt-list">
      {#if loading}
        <div class="mt-state">{$t("meetings.loading")}</div>
      {:else if error}
        <div class="mt-state">
          <div class="hinweis" data-art="fehler" role="alert">
            <Symbol name="achtung" size={16} />
            <span>{error}</span>
          </div>
          <button type="button" class="btn btn-sekundaer mt-state-retry" onclick={loadMeetings}>{$t("meetings.reload")}</button>
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
            oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, meeting: m }; }}
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
  </section>

  <!-- DETAIL -->
  <main class="mt-main">
    <div class="mt-back-bar">
      <button type="button" class="btn btn-still" onclick={() => (detailOffen = false)}>
        <Symbol name="zurueck" size={20} />{$t("common.back")}
      </button>
    </div>

    {#if detailLoading}
      <div class="mt-state">{$t("meetings.loading")}</div>
    {:else if detailError}
      <div class="mt-state">
        <div class="hinweis" data-art="fehler" role="alert">
          <Symbol name="achtung" size={16} />
          <span>{detailError}</span>
        </div>
      </div>
    {:else if !detail}
      <EmptyState title={$t("meetings.selectHint")} icon="besprechung" />
    {:else}
      <article class="mt-detail">
        <header class="mt-detail-header">
          <div class="mt-detail-headline">
            <h2 class="mt-detail-titel">{detail.title}</h2>
            <div class="mt-detail-meta">
              <span>{fmtDateTime(detail.meeting_date)}</span>
              <span>·</span>
              <span>{$t("meetings.duration", { n: detail.duration_min })}</span>
              {#if detail.template}<span>·</span><span>{$t("meetings.template")}: {detail.template}</span>{/if}
              {#if detail.language}<span>·</span><span>{$t("meetings.language")}: {detail.language}</span>{/if}
            </div>
          </div>
          <div class="mt-detail-actions">
            <button type="button" class="btn btn-sekundaer" onclick={emailMinutes}>
              {$t("meetings.emailMinutes")}
            </button>
            <button type="button" class="btn btn-gefahr" onclick={handleDelete}>
              {$t("meetings.delete")}
            </button>
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
            <div class="hinweis" data-art="fehler" role="alert"><Symbol name="achtung" size={16} /><span>{followupsError}</span></div>
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
                  class="btn btn-sekundaer btn-klein mt-followups-btn"
                  disabled={followupPlanBusy}
                  onclick={() => handleFollowupChip(a)}
                >
                  {$t("meetings.followupsAccept")}
                </button>
              </div>
            {/each}
          {/if}
        </section>

        {#if detail.source_url.startsWith("http")}
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
</div>
</Huelle>

<ConfirmationDialog
  open={deleteTarget !== null}
  title={$t("meetings.delete")}
  message={deleteTarget ? translate("meetings.deleteConfirm", { title: deleteTarget.title }) : ""}
  confirmLabel={$t("meetings.delete")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={() => void handleDeleteTarget()}
  oncancel={() => (deleteTarget = null)}
/>

<ContextMenu
  menu={ctxMenu}
  items={ctxItems}
  onclose={() => (ctxMenu = null)}
/>

<AssistantFab module="meetings" />

<style>
  /* ── Meetings page inside the shell: list and detail pane [RL-MEETINGS] ──
     Two panes like the mail, each scrolling itself: the list left at a
     fixed width, the detail right. No inner navigation in the column. */
  .mt-app {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--am-seite);
    color: var(--am-text-primaer);
  }
  .mt-panes {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .mt-list-pane {
    flex: 0 0 340px;
    width: 340px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--am-trennlinie);
  }

  /* ── Meeting list rows [RL-MEETINGS] ─────────────────────────────────── */
  /* List rows stay Relay's own (not `.btn`). */
  .mt-list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: var(--am-raum-2);
  }
  .mt-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: var(--am-radius-klein);
    padding: 10px;
    cursor: pointer;
    color: var(--am-text-primaer);
  }
  .mt-item:hover { background: var(--am-flaeche-2); }
  .mt-item.active { background: var(--am-flaeche-2); }
  .mt-item-date {
    font-size: 0.72rem;
    color: var(--am-text-gedaempft);
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
    color: var(--am-text-gedaempft);
    display: flex;
    gap: 8px;
    align-items: baseline;
    white-space: nowrap;
  }
  .mt-item-tags {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Loading and error state [RL-MEETINGS] ───────────────────────────── */
  .mt-state {
    padding: 24px 16px;
    font-size: 0.85rem;
    color: var(--am-text-gedaempft);
    text-align: center;
  }
  .mt-state-retry { margin-top: 10px; }

  .mt-main {
    flex: 1;
    overflow-y: auto;
    min-width: 0;
    min-height: 0;
    background: var(--am-seite);
  }
  /* "Zurück" to the list — only on the phone, where the detail replaces it. */
  .mt-back-bar {
    display: none;
    padding: var(--am-raum-2) var(--am-raum-3);
    border-bottom: 1px solid var(--am-trennlinie);
  }
  /* ── Meeting detail [RL-MEETINGS] ────────────────────────────────────── */
  .mt-detail {
    max-width: 860px;
    margin: 0 auto;
    padding: 32px 40px 64px;
  }
  .mt-detail-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 24px;
  }
  .mt-detail-headline { min-width: 0; }
  .mt-detail-actions { display: flex; gap: 8px; flex-shrink: 0; }
  .mt-detail-titel {
    font-size: 1.4rem;
    font-weight: 600;
    margin: 0 0 6px;
  }
  .mt-detail-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 0.8rem;
    color: var(--am-text-gedaempft);
  }
  .mt-section { margin-bottom: 24px; }
  .mt-section h2 {
    font-size: 0.8rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
    margin: 0 0 8px;
  }
  /* ── Follow-up suggestions [RL-MEETINGS] ─────────────────────────────── */
  .mt-followups-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--am-rand);
  }
  .mt-followups-row:last-child { border-bottom: none; }
  .mt-followups-label { font-size: 0.9rem; }
  .mt-followups-muted { color: var(--am-text-gedaempft); font-size: 0.85rem; }
  .mt-followups-btn { flex-shrink: 0; }
  /* ── Participant and tag chips [RL-MEETINGS] ─────────────────────────── */
  .mt-participants {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .mt-participants li {
    background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    padding: 3px 10px;
    font-size: 0.8rem;
  }
  .mt-tags-row { display: flex; flex-wrap: wrap; gap: 6px; }
  .mt-tag {
    background: var(--am-flaeche-2);
    border-radius: var(--am-radius-klein);
    padding: 3px 10px;
    font-size: 0.75rem;
    color: var(--am-handlung-ruhend);
  }

  /* ── Minutes body (rendered Markdown) [RL-MEETINGS] ──────────────────── */
  .mt-body { font-size: 0.92rem; line-height: 1.6; }
  .mt-body-empty { color: var(--am-text-gedaempft); }
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
    background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    padding: 1px 5px;
    font-size: 0.85em;
  }
  .mt-body :global(a) { color: var(--am-handlung-ruhend); }

  .mt-detail-footer {
    margin-top: 32px;
    padding-top: 16px;
    border-top: 1px solid var(--am-rand);
  }
  .mt-link {
    color: var(--am-handlung-ruhend);
    font-size: 0.85rem;
    text-decoration: none;
  }
  .mt-link:hover { text-decoration: underline; }

  /* ── Phone (≤768px): one pane at a time, as the mail [RL-MEETINGS] ───── */
  @media (max-width: 768px) {
    .mt-list-pane { flex: 1 1 auto; width: auto; border-right: none; }
    .mt-main { display: none; }
    .mt-app.detail-offen .mt-list-pane { display: none; }
    .mt-app.detail-offen .mt-main { display: block; }
    .mt-back-bar { display: block; }
    .mt-detail { padding: 20px 16px 48px; }
    .mt-detail-header { flex-wrap: wrap; }
  }
</style>
