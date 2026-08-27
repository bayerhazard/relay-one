<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    getCalendars, listEvents, createEvent, updateEvent, deleteEvent,
    getCalDavSettings, syncCalDav, importEvents, getEventIcs,
    listInvitations, acceptInvitation, declineInvitation, listAccounts,
    getConflicts, getConflictAlternatives, extractTime, rsvpDraft,
    type CalendarInfo, type EventInfo, type InvitationInfo, type TimeSlot,
  } from "$lib/services/tauri";
  import { t } from "$lib/i18n";
  import { germanHolidays } from "$lib/holidays";

  // Synthetic built-in "Feiertage" calendar (German public holidays).
  const HOLIDAY_CAL_ID = -1;
  const HOLIDAY_CAL: CalendarInfo = {
    id: HOLIDAY_CAL_ID, name: "Feiertage", color: "#caa960",
    read_only: true, last_synced_at: null,
  };
  // All calendars shown in the list: built-in holidays first, then CalDAV.
  let allCalendars = $derived<CalendarInfo[]>([HOLIDAY_CAL, ...calendars]);

  // ─── State ───────────────────────────────────
  let calendars = $state<CalendarInfo[]>([]);
  let events = $state<EventInfo[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let syncing = $state(false);

  // Current date anchor for the visible range.
  let viewDate = $state(new Date());
  let today = $state(new Date());

  // Center view mode.
  let viewMode = $state<"month" | "week" | "day">("month");
  // Event shown in the right detail pane.
  let selectedEvent = $state<EventInfo | null>(null);
  // Multi-calendar: which calendars are visible (by id).
  let visibleCals = $state<Set<number>>(new Set());

  // ─── iMIP invitation queue (Phase 2) ─────────
  let invitations = $state<InvitationInfo[]>([]);
  let invBusy = $state<string | null>(null);

  async function loadInvitations() {
    try {
      invitations = await listInvitations();
    } catch {
      invitations = [];
    }
  }

  async function respondToInvitation(inv: InvitationInfo, decision: "ACCEPTED" | "DECLINED") {
    if (invBusy) return;
    invBusy = inv.event_uid;
    try {
      const accounts = await listAccounts();
      const acct =
        accounts.find((a) => a.sender_email === inv.attendee_email || a.username === inv.attendee_email) ??
        accounts[0];
      if (!acct) throw new Error("Kein E-Mail-Konto gefunden");
      if (decision === "ACCEPTED") await acceptInvitation(inv.event_uid, acct.id);
      else await declineInvitation(inv.event_uid, acct.id);
      invitations = invitations.filter((i) => i.event_uid !== inv.event_uid);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      invBusy = null;
    }
  }

  function fmtInvWhen(iso: string): string {
    try {
      const d = new Date(iso);
      return d.toLocaleString(undefined, {
        weekday: "short", day: "2-digit", month: "2-digit",
        hour: "2-digit", minute: "2-digit",
      });
    } catch {
      return iso;
    }
  }

  // AI-drafted RSVP reply for the currently expanded invitation.
  let invDraft = $state<{ uid: string; text: string } | null>(null);
  let invDraftBusy = $state(false);

  async function draftRsvp(inv: InvitationInfo, decision: "ACCEPTED" | "DECLINED") {
    if (invDraftBusy) return;
    invDraftBusy = true;
    try {
      const text = await rsvpDraft(
        inv.summary ?? "(Einladung)",
        inv.start_at ?? "",
        inv.organizer,
        decision,
      );
      invDraft = { uid: inv.event_uid, text };
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      invDraftBusy = false;
    }
  }

  // ─── Conflicts + Calendar AI (Phase 2.4 / 2.5) ───────
  let conflicts = $state<EventInfo[]>([]);
  let aiSlots = $state<TimeSlot[]>([]);
  let nlText = $state("");
  let nlBusy = $state(false);
  let conflictBusy = $state(false);
  let showAlternatives = $state(false);

  function toUtc(v: string, allDay: boolean): string {
    if (allDay) return `${v}T00:00:00Z`;
    const d = new Date(v);
    return d.toISOString();
  }

  async function checkConflicts() {
    if (!form.start || !form.end) {
      conflicts = [];
      return;
    }
    try {
      const start = toUtc(form.start, form.all_day);
      const end = toUtc(form.end, form.all_day);
      conflicts = await getConflicts(start, end, defaultCalId(), editingId);
    } catch {
      conflicts = [];
    }
  }

  async function loadAlternatives() {
    if (!form.start || !form.end) return;
    conflictBusy = true;
    showAlternatives = true;
    try {
      const start = toUtc(form.start, form.all_day);
      const end = toUtc(form.end, form.all_day);
      aiSlots = await getConflictAlternatives(form.summary || "Termin", start, end, defaultCalId());
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      aiSlots = [];
    } finally {
      conflictBusy = false;
    }
  }

  function applySlot(slot: TimeSlot) {
    form.start = toLocalInput(new Date(slot.start));
    form.end = toLocalInput(new Date(slot.end));
    form.all_day = false;
    aiSlots = [];
    showAlternatives = false;
    checkConflicts();
  }

  async function applyTimeExtraction() {
    if (!nlText.trim() || nlBusy) return;
    nlBusy = true;
    try {
      const t = await extractTime(nlText);
      if (t.start) form.start = toLocalInput(new Date(t.start));
      if (t.end) form.end = toLocalInput(new Date(t.end));
      if (t.summary && !form.summary) form.summary = t.summary;
      form.all_day = t.all_day;
      nlText = "";
      checkConflicts();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      nlBusy = false;
    }
  }

  // Re-check conflicts whenever the time fields change.
  $effect(() => {
    if (editorOpen && form.start && form.end) {
      const t = setTimeout(checkConflicts, 400);
      return () => clearTimeout(t);
    }
  });

  // Fixed palette for calendar colors (index by position).
  const CAL_COLORS = ["#caa960", "#4f83b3", "#7ba05b", "#b3564f", "#8a6fb3", "#4fb3a5"];
  function calColor(cal: CalendarInfo | undefined): string {
    if (!cal) return CAL_COLORS[0];
    if (cal.id === HOLIDAY_CAL_ID) return cal.color;
    const idx = calendars.findIndex((c) => c.id === cal.id);
    return CAL_COLORS[(idx < 0 ? 0 : idx) % CAL_COLORS.length];
  }
  function calVisible(cal: CalendarInfo): boolean {
    return visibleCals.has(cal.id);
  }
  function toggleCal(cal: CalendarInfo) {
    const next = new Set(visibleCals);
    if (next.has(cal.id)) next.delete(cal.id);
    else next.add(cal.id);
    visibleCals = next;
  }
  // Events filtered to visible calendars (CalDAV + built-in holidays).
  let shownEvents = $derived.by(() => {
    const calEvents = events.filter(
      (ev) => visibleCals.size === 0 || visibleCals.has(ev.calendar_id)
    );
    const hols = visibleCals.has(HOLIDAY_CAL_ID) ? holidayEvents : [];
    return [...calEvents, ...hols];
  });

  // ─── Derived: month grid ─────────────────────
  const WEEKDAYS = $derived(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);

  let monthLabel = $derived(
    viewDate.toLocaleDateString(undefined, { month: "long", year: "numeric" })
  );

  // Build a 6x7 grid of dates covering the visible month.
  let gridDays = $derived.by(() => {
    const y = viewDate.getFullYear();
    const m = viewDate.getMonth();
    // Monday-first: offset of the 1st (getDay: 0=Sun..6=Sat) → Mon=0.
    const firstDow = (new Date(y, m, 1).getDay() + 6) % 7;
    const start = new Date(y, m, 1 - firstDow);
    const days: Date[] = [];
    for (let i = 0; i < 42; i++) {
      days.push(new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
    }
    return days;
  });

  // Effective start of an event: the specific occurrence for recurring events,
  // otherwise the event's own start.
  function effStart(ev: EventInfo): string {
    return ev.occurrence_start ?? ev.start;
  }
  function effEnd(ev: EventInfo): string {
    return ev.occurrence_end ?? ev.end;
  }

  // Map of LOCAL "YYYY-MM-DD" → events for that day. Grouping by the user's
  // local date (not the UTC date) so an event at 01:00 local shows on the
  // right day even when its UTC instant is the previous day.
  let eventsByDay = $derived.by(() => {
    const map = new Map<string, EventInfo[]>();
    for (const ev of shownEvents) {
      const d = new Date(effStart(ev));
      if (isNaN(d.getTime())) continue;
      const key = localDayKey(d);
      const arr = map.get(key) ?? [];
      arr.push(ev);
      map.set(key, arr);
    }
    // Sort each day's events by start time.
    for (const arr of map.values()) {
      arr.sort((a, b) => effStart(a).localeCompare(effStart(b)));
    }
    return map;
  });

  // The 7 days of the week containing viewDate (Monday-first).
  let weekDays = $derived.by(() => {
    const d = new Date(viewDate);
    const dow = (d.getDay() + 6) % 7; // Mon=0
    d.setDate(d.getDate() - dow);
    const days: Date[] = [];
    for (let i = 0; i < 7; i++) {
      days.push(new Date(d.getFullYear(), d.getMonth(), d.getDate() + i));
    }
    return days;
  });

  // A single day's events (for the day view), sorted.
  let dayEvents = $derived.by(() => {
    const key = localDayKey(viewDate);
    return eventsByDay.get(key) ?? [];
  });

  // The big label in the toolbar, per view mode.
  let periodLabel = $derived(
    viewMode === "week" ? weekLabel : viewMode === "day" ? dayLabel : monthLabel
  );

  function calById(id: number): CalendarInfo | undefined {
    return allCalendars.find((c) => c.id === id);
  }
  function dowShort(d: Date): string {
    return d.toLocaleDateString(undefined, { weekday: "short" });
  }

  // Week label: "12. – 18. Sep 2026".
  let weekLabel = $derived.by(() => {
    const a = weekDays[0];
    const b = weekDays[6];
    const opts: Intl.DateTimeFormatOptions = { day: "numeric", month: "short" };
    const ay = a.getFullYear() === b.getFullYear() ? "" : ` ${a.getFullYear()}`;
    return `${a.toLocaleDateString(undefined, opts)}${ay} – ${b.toLocaleDateString(undefined, { ...opts, year: "numeric" })}`;
  });

  let dayLabel = $derived(
    viewDate.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" })
  );

  function setViewMode(mode: "month" | "week" | "day") {
    viewMode = mode;
  }
  function selectEvent(ev: EventInfo) {
    selectedEvent = ev;
  }
  function clearSelection() {
    selectedEvent = null;
  }

  // Navigate the visible period relative to the current view mode.
  function shiftPeriod(delta: number) {
    const d = new Date(viewDate);
    if (viewMode === "month") d.setMonth(d.getMonth() + delta);
    else if (viewMode === "week") d.setDate(d.getDate() + delta * 7);
    else d.setDate(d.getDate() + delta);
    viewDate = d;
  }
  function goToday() {
    viewDate = new Date();
  }

  // Mini-month navigation: shift the month of viewDate by delta months.
  function shiftMini(delta: number) {
    const d = new Date(viewDate);
    d.setDate(1);
    d.setMonth(d.getMonth() + delta);
    viewDate = d;
  }
  let miniMonthLabel = $derived(
    viewDate.toLocaleDateString(undefined, { month: "short", year: "numeric" })
  );

  // Start a new event pre-filled with a given day.
  function openNewEventOn(day: Date) {
    const d = new Date(day);
    d.setHours(9, 0, 0, 0);
    openNewEvent(d);
  }

  function localDayKey(d: Date): string {
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function dayKey(d: Date): string {
    return localDayKey(d);
  }

  // ─── Data loading ────────────────────────────
  async function loadCalendars() {
    try {
      calendars = await getCalendars();
      // Default: all calendars visible, including the built-in holidays.
      visibleCals = new Set([HOLIDAY_CAL_ID, ...calendars.map((c) => c.id)]);
    } catch (e) {
      console.warn("calendars load failed", e);
    }
  }

  // Synthetic all-day holiday events for the years covered by the window.
  let holidayEvents = $derived.by(() => {
    const [from, to] = viewWindow();
    const years = new Set([from.getFullYear(), to.getFullYear()]);
    const out: EventInfo[] = [];
    let n = 0;
    for (const y of years) {
      for (const h of germanHolidays(y)) {
        n += 1;
        out.push({
          id: -(1000 + n),
          calendar_id: HOLIDAY_CAL_ID,
          uid: `holiday-${h.date}`,
          summary: h.name,
          start: `${h.date}T00:00:00Z`,
          end: `${h.date}T23:59:59Z`,
          all_day: true,
          location: null,
          description: null,
          status: "CONFIRMED",
          organizer: null,
          rrule: null,
        });
      }
    }
    return out;
  });

  // The [from, to) window to fetch, depending on the active view mode.
  function viewWindow(): [Date, Date] {
    if (viewMode === "week") {
      const a = weekDays[0];
      const b = new Date(weekDays[6]);
      b.setDate(b.getDate() + 1);
      return [a, b];
    }
    if (viewMode === "day") {
      const a = new Date(viewDate);
      a.setHours(0, 0, 0, 0);
      const b = new Date(a);
      b.setDate(b.getDate() + 1);
      return [a, b];
    }
    const y = viewDate.getFullYear();
    const m = viewDate.getMonth();
    return [new Date(y, m, 1), new Date(y, m + 1, 1)];
  }

  async function loadEvents() {
    loading = true;
    error = null;
    try {
      const [from, to] = viewWindow();
      // Fetch across all calendars; visibility is filtered client-side.
      events = await listEvents(null, isoDate(from), isoDate(to));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      events = [];
    } finally {
      loading = false;
    }
  }

  function isoDate(d: Date): string {
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  async function handleSync() {
    syncing = true;
    error = null;
    try {
      await syncCalDav();
      await loadCalendars();
      await loadEvents();
      await loadUpcoming();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      syncing = false;
    }
  }

  // ─── ICS import / export ─────────────────────
  let importing = $state(false);
  let importInput = $state<HTMLInputElement | null>(null);

  function triggerImport() {
    importInput?.click();
  }

  async function onImportFile(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ""; // allow re-selecting the same file
    if (!file) return;
    importing = true;
    error = null;
    try {
      const ics = await file.text();
      const calId = defaultCalId();
      if (calId === null) throw new Error("Kein Kalender für den Import verfügbar.");
      const { imported } = await importEvents(calId, ics);
      await loadEvents();
      if (imported === 0) error = "Keine Termine in der Datei gefunden.";
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      importing = false;
    }
  }

  async function handleExport(ev: EventInfo) {
    try {
      const { ics, filename } = await getEventIcs(ev.id);
      const blob = new Blob([ics], { type: "text/calendar" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename || "termin.ics";
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  // ─── Upcoming reminders (next 24h with alarms) ──
  let upcoming = $state<EventInfo[]>([]);
  async function loadUpcoming() {
    try {
      const now = new Date();
      const from = new Date(now);
      const to = new Date(now.getTime() + 24 * 3600 * 1000);
      const evs = await listEvents(null, isoDate(from), isoDate(to));
      upcoming = evs
        .filter((ev) => (ev.alarms ?? 0) > 0 && visibleCals.has(ev.calendar_id))
        .sort((a, b) => effStart(a).localeCompare(effStart(b)))
        .slice(0, 5);
    } catch {
      upcoming = [];
    }
  }

  // ─── Event editor ────────────────────────────
  let editorOpen = $state(false);
  let editingId = $state<number | null>(null);
  let form = $state({
    summary: "",
    start: "",
    end: "",
    all_day: false,
    location: "",
    description: "",
    reminder_minutes: 15,
  });

  function openNewEvent(day?: Date) {
    editingId = null;
    const base = day ?? viewDate;
    const s = new Date(base); s.setHours(9, 0, 0, 0);
    const e = new Date(base); e.setHours(10, 0, 0, 0);
    form = {
      summary: "",
      start: toLocalInput(s),
      end: toLocalInput(e),
      all_day: false,
      location: "",
      description: "",
      reminder_minutes: 15,
    };
    editorOpen = true;
  }

  function openEditEvent(ev: EventInfo) {
    editingId = ev.id;
    form = {
      summary: ev.summary ?? "",
      start: ev.all_day ? ev.start.slice(0, 10) : toLocalInput(new Date(ev.start)),
      end: ev.all_day ? ev.end.slice(0, 10) : toLocalInput(new Date(ev.end)),
      all_day: ev.all_day,
      location: ev.location ?? "",
      description: ev.description ?? "",
      reminder_minutes: 15,
    };
    editorOpen = true;
  }

  function toLocalInput(d: Date): string {
    const p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  function fromLocalInput(v: string, allDay: boolean): string {
    // Return RFC3339 UTC string the server expects.
    if (allDay) return `${v}T00:00:00Z`;
    const d = new Date(v);
    return d.toISOString();
  }

  // The calendar a new event is created in: first visible real calendar.
  function defaultCalId(): number | null {
    const c = calendars.find((c) => visibleCals.has(c.id));
    return c ? c.id : calendars[0]?.id ?? null;
  }

  async function saveEvent() {
    if (editingId === null && defaultCalId() === null) return;
    error = null;
    try {
      const start = fromLocalInput(form.start, form.all_day);
      const end = form.end ? fromLocalInput(form.end, form.all_day) : undefined;
      if (editingId === null) {
        await createEvent({
          calendar_id: defaultCalId()!,
          summary: form.summary,
          start,
          end,
          all_day: form.all_day,
          location: form.location || undefined,
          description: form.description || undefined,
          reminder_minutes: form.reminder_minutes,
        });
      } else {
        await updateEvent(editingId, {
          summary: form.summary,
          start,
          end,
          all_day: form.all_day,
          location: form.location || undefined,
          description: form.description || undefined,
          reminder_minutes: form.reminder_minutes,
        });
      }
      editorOpen = false;
      await loadEvents();
      await loadUpcoming();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function removeEvent(ev: EventInfo) {
    if (!confirm(`${ev.summary ?? "Termin"} löschen?`)) return;
    try {
      await deleteEvent(ev.id);
      await loadEvents();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function fmtEventTime(ev: EventInfo): string {
    if (ev.all_day) return "Ganztägig";
    const d = new Date(effStart(ev));
    if (isNaN(d.getTime())) return "";
    return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }

  // Compact "when" label for the upcoming-reminders list.
  function fmtUpcomingWhen(ev: EventInfo): string {
    const d = new Date(effStart(ev));
    if (isNaN(d.getTime())) return "";
    const now = new Date();
    const diffMin = Math.round((d.getTime() - now.getTime()) / 60000);
    if (diffMin < 1) return "jetzt";
    if (diffMin < 60) return `in ${diffMin} min`;
    const sameDay = localDayKey(d) === localDayKey(now);
    const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    if (sameDay) return `heute ${time}`;
    if (diffMin < 60 * 24) return `morgen ${time}`;
    return d.toLocaleDateString(undefined, { day: "numeric", month: "short" }) + ` ${time}`;
  }

  // Human-readable date range for the detail pane.
  function fmtEventRange(ev: EventInfo): string {
    const s = new Date(effStart(ev));
    const e = new Date(effEnd(ev));
    if (isNaN(s.getTime())) return "";
    const sameDay = localDayKey(s) === localDayKey(e);
    const dateOpt: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "long" };
    if (ev.all_day) {
      return sameDay
        ? s.toLocaleDateString(undefined, dateOpt)
        : `${s.toLocaleDateString(undefined, dateOpt)} – ${e.toLocaleDateString(undefined, dateOpt)}`;
    }
    const timeOpt: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" };
    const base = s.toLocaleDateString(undefined, dateOpt);
    if (sameDay) {
      return `${base}, ${s.toLocaleTimeString(undefined, timeOpt)} – ${e.toLocaleTimeString(undefined, timeOpt)}`;
    }
    return `${base}, ${s.toLocaleTimeString(undefined, timeOpt)} – ${e.toLocaleDateString(undefined, dateOpt)}, ${e.toLocaleTimeString(undefined, timeOpt)}`;
  }

  onMount(async () => {
    await loadCalendars();
    await loadEvents();
    await loadUpcoming();
    loadInvitations();
  });
</script>

<div class="cal-app">
  <aside class="cal-sidebar">
    <div class="cal-sidebar-header">
      <button type="button" class="cal-back" onclick={() => goto("/")} title="Zurück zur Post">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5M12 19l-7-7 7-7"/></svg>
      </button>
      <span class="cal-brand">Kalender</span>
    </div>

    <!-- Mini month for quick navigation -->
    <div class="cal-mini">
      <div class="cal-mini-head">
        <button type="button" class="cal-mini-nav" onclick={() => shiftMini(-1)} aria-label="Vorheriger Monat">‹</button>
        <span class="cal-mini-label">{miniMonthLabel}</span>
        <button type="button" class="cal-mini-nav" onclick={() => shiftMini(1)} aria-label="Nächster Monat">›</button>
      </div>
      <div class="cal-mini-grid">
        {#each gridDays as d (localDayKey(d))}
          <button
            type="button"
            class="cal-mini-day"
            class:other={d.getMonth() !== viewDate.getMonth()}
            class:today={localDayKey(d) === localDayKey(today)}
            class:sel={localDayKey(d) === localDayKey(viewDate)}
            onclick={() => { viewDate = new Date(d); }}
          >{d.getDate()}</button>
        {/each}
      </div>
    </div>

    <div class="cal-cal-list">
      {#each allCalendars as cal (cal.id)}
        <label class="cal-cal-item">
          <input
            type="checkbox"
            checked={calVisible(cal)}
            onchange={() => toggleCal(cal)}
          />
          <span class="cal-cal-dot" style="background: {calColor(cal)}"></span>
          <span class="cal-cal-name">{cal.name}</span>
        </label>
      {/each}
      {#if calendars.length === 0}
        <div class="cal-empty">
          <p>Keine CalDAV-Kalender verbunden.</p>
          <button type="button" class="cal-btn cal-btn-ghost" onclick={() => goto("/settings")}>
            CalDAV verbinden
          </button>
        </div>
      {/if}
    </div>

    {#if invitations.length > 0}
      <div class="cal-invitations">
        <div class="cal-invitations-head">
          <span>Einladungen</span>
          <span class="cal-invitations-badge">{invitations.length}</span>
        </div>
        {#each invitations as inv (inv.event_uid)}
          <div class="cal-inv-item">
            <div class="cal-inv-info">
              <span class="cal-inv-title">{inv.summary ?? "(Einladung)"}</span>
              {#if inv.start_at}<span class="cal-inv-when">{fmtInvWhen(inv.start_at)}</span>{/if}
              <span class="cal-inv-organizer">{inv.organizer}</span>
            </div>
            <div class="cal-inv-actions">
              <button
                type="button"
                class="cal-inv-draft"
                title="KI-Antwort-Entwurf"
                disabled={invDraftBusy}
                onclick={() => draftRsvp(inv, "ACCEPTED")}
              >✎</button>
              <button
                type="button"
                class="cal-inv-accept"
                title="Annehmen"
                disabled={invBusy !== null}
                onclick={() => respondToInvitation(inv, "ACCEPTED")}
              >✓</button>
              <button
                type="button"
                class="cal-inv-decline"
                title="Ablehnen"
                disabled={invBusy !== null}
                onclick={() => respondToInvitation(inv, "DECLINED")}
              >✕</button>
            </div>
          </div>
          {#if invDraft?.uid === inv.event_uid}
            <div class="cal-inv-draftbox">
              <textarea class="cal-input" rows="3" value={invDraft.text} oninput={(e) => (invDraft = { uid: inv.event_uid, text: e.currentTarget.value })}></textarea>
            </div>
          {/if}
        {/each}
      </div>
    {/if}

    {#if upcoming.length > 0}
      <div class="cal-upcoming">
        <div class="cal-upcoming-head">Nächste Erinnerungen</div>
        {#each upcoming as ev (ev.id)}
          <button type="button" class="cal-upcoming-item" onclick={() => selectEvent(ev)}>
            <span class="cal-upcoming-bell" aria-hidden>◷</span>
            <div class="cal-upcoming-info">
              <span class="cal-upcoming-title">{ev.summary ?? "(ohne Titel)"}</span>
              <span class="cal-upcoming-when">{fmtUpcomingWhen(ev)}</span>
            </div>
          </button>
        {/each}
      </div>
    {/if}

    <div class="cal-sidebar-footer">
      <div class="cal-footer-actions">
        <button type="button" class="cal-btn cal-btn-ghost" onclick={handleSync} disabled={syncing}>
          {syncing ? "Synchronisiere…" : "Synchronisieren"}
        </button>
        <button type="button" class="cal-btn cal-btn-ghost" onclick={triggerImport} disabled={importing}>
          {importing ? "Importiere…" : "ICS importieren"}
        </button>
      </div>
      <input
        type="file"
        accept=".ics,text/calendar"
        class="cal-file-input"
        bind:this={importInput}
        onchange={onImportFile}
      />
    </div>
  </aside>

  <main class="cal-main">
    <header class="cal-toolbar">
      <div class="cal-toolbar-left">
        <button type="button" class="cal-btn cal-btn-ghost cal-nav" onclick={() => shiftPeriod(-1)} aria-label="Zurück">‹</button>
        <button type="button" class="cal-btn cal-btn-ghost" onclick={goToday}>Heute</button>
        <button type="button" class="cal-btn cal-btn-ghost cal-nav" onclick={() => shiftPeriod(1)} aria-label="Vor">›</button>
        <h1 class="cal-month">{periodLabel}</h1>
      </div>
      <div class="cal-toolbar-right">
        <div class="cal-viewtoggle" role="tablist" aria-label="Ansicht">
          <button type="button" class="cal-vt" class:active={viewMode === "month"} onclick={() => setViewMode("month")}>Monat</button>
          <button type="button" class="cal-vt" class:active={viewMode === "week"} onclick={() => setViewMode("week")}>Woche</button>
          <button type="button" class="cal-vt" class:active={viewMode === "day"} onclick={() => setViewMode("day")}>Tag</button>
        </div>
        <button type="button" class="cal-btn cal-btn-primary" onclick={() => openNewEvent()}>+ Termin</button>
      </div>
    </header>

    {#if error}
      <div class="cal-alert">{error}</div>
    {/if}

    {#if viewMode === "month"}
      <div class="cal-grid">
        <div class="cal-grid-head">
          {#each WEEKDAYS as wd}
            <div class="cal-grid-head-cell">{wd}</div>
          {/each}
        </div>

        {#each gridDays as day (dayKey(day))}
          <div
            class="cal-cell"
            class:other-month={day.getMonth() !== viewDate.getMonth()}
            class:is-today={dayKey(day) === dayKey(today)}
            onclick={() => openNewEventOn(day)}
          >
            <span class="cal-cell-num">{day.getDate()}</span>
            <div class="cal-cell-events">
              {#each (eventsByDay.get(dayKey(day)) ?? []) as ev (ev.id)}
                <button
                  type="button"
                  class="cal-event"
                  class:cancelled={ev.status === "CANCELLED"}
                  style="border-left-color: {calColor(calById(ev.calendar_id) ?? calendars[0])}"
                  onclick={(e) => { e.stopPropagation(); selectEvent(ev); }}
                  title={ev.summary ?? ""}
                >
                  <span class="cal-event-time">{fmtEventTime(ev)}</span>
                  <span class="cal-event-title">{ev.summary ?? "(ohne Titel)"}</span>
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {:else if viewMode === "week"}
      <div class="cal-week">
        <div class="cal-week-head">
          {#each weekDays as d (localDayKey(d))}
            <div class="cal-week-head-cell" class:is-today={localDayKey(d) === localDayKey(today)}>
              <span class="cal-week-dow">{dowShort(d)}</span>
              <span class="cal-week-num">{d.getDate()}</span>
            </div>
          {/each}
        </div>
        <div class="cal-week-body">
          {#each weekDays as d (localDayKey(d))}
            <div class="cal-week-col" class:is-today={localDayKey(d) === localDayKey(today)} onclick={() => openNewEventOn(d)}>
              {#each (eventsByDay.get(localDayKey(d)) ?? []) as ev (ev.id)}
                <button
                  type="button"
                  class="cal-event cal-event-block"
                  class:cancelled={ev.status === "CANCELLED"}
                  style="border-left-color: {calColor(calById(ev.calendar_id) ?? calendars[0])}"
                  onclick={(e) => { e.stopPropagation(); selectEvent(ev); }}
                >
                  <span class="cal-event-time">{fmtEventTime(ev)}</span>
                  <span class="cal-event-title">{ev.summary ?? "(ohne Titel)"}</span>
                </button>
              {/each}
            </div>
          {/each}
        </div>
      </div>
    {:else}
      <div class="cal-dayview">
        {#if dayEvents.length === 0}
          <div class="cal-day-empty">Keine Termine an diesem Tag.</div>
        {:else}
          {#each dayEvents as ev (ev.id)}
            <button
              type="button"
              class="cal-day-item"
              class:cancelled={ev.status === "CANCELLED"}
              class:sel={selectedEvent?.id === ev.id}
              onclick={() => selectEvent(ev)}
            >
              <span class="cal-day-time">{fmtEventTime(ev)}</span>
              <span class="cal-day-dot" style="background: {calColor(calById(ev.calendar_id) ?? calendars[0])}"></span>
              <div class="cal-day-info">
                <span class="cal-day-title">{ev.summary ?? "(ohne Titel)"}</span>
                {#if ev.location}<span class="cal-day-loc">{ev.location}</span>{/if}
              </div>
            </button>
          {/each}
        {/if}
      </div>
    {/if}
  </main>

  <!-- ─── Right: detail pane ─── -->
  <aside class="cal-detail">
    {#if selectedEvent}
      {@const ev = selectedEvent}
      {@const cal = calById(ev.calendar_id)}
      <div class="cal-detail-inner">
        <div class="cal-detail-top">
          <span class="cal-detail-cal" style="color: {cal ? calColor(cal) : 'var(--color-text)'}">
            {cal?.name ?? "Kalender"}
          </span>
          <button type="button" class="cal-detail-close" onclick={clearSelection} aria-label="Schließen">×</button>
        </div>
        <h2 class="cal-detail-title">{ev.summary ?? "(ohne Titel)"}</h2>

        <div class="cal-detail-rows">
          <div class="cal-detail-row">
            <span class="cal-detail-ico" aria-hidden>◷</span>
            <span>{fmtEventRange(ev)}</span>
          </div>
          {#if ev.location}
            <div class="cal-detail-row">
              <span class="cal-detail-ico" aria-hidden>⚲</span>
              <span>{ev.location}</span>
            </div>
          {/if}
          {#if ev.rrule}
            <div class="cal-detail-row">
              <span class="cal-detail-ico" aria-hidden>↻</span>
              <span>Wiederholter Termin</span>
            </div>
          {/if}
          {#if ev.organizer}
            <div class="cal-detail-row">
              <span class="cal-detail-ico" aria-hidden>✉</span>
              <span>{ev.organizer}</span>
            </div>
          {/if}
        </div>

        {#if ev.description}
          <p class="cal-detail-desc">{ev.description}</p>
        {/if}

        <div class="cal-detail-actions">
          <button type="button" class="cal-btn cal-btn-ghost" onclick={() => openEditEvent(ev)}>Bearbeiten</button>
          <button type="button" class="cal-btn cal-btn-ghost" onclick={() => handleExport(ev)}>ICS</button>
          <button type="button" class="cal-btn cal-btn-danger" onclick={() => { removeEvent(ev); clearSelection(); }}>Löschen</button>
        </div>
      </div>
    {:else}
      <div class="cal-detail-empty">
        <p>Wähle einen Termin aus, um Details zu sehen.</p>
      </div>
    {/if}
  </aside>
</div>

<!-- ─── Event editor dialog ─── -->
{#if editorOpen}
  <div class="cal-modal-scrim" onclick={() => editorOpen = false}>
    <div class="cal-modal" onclick={(e) => e.stopPropagation()}>
      <h2>{editingId === null ? "Neuer Termin" : "Termin bearbeiten"}</h2>

      <label class="cal-field">
        <span>Titel</span>
        <input type="text" bind:value={form.summary} class="cal-input" placeholder="Termin-Titel" />
      </label>

      <div class="cal-nl">
        <input
          type="text"
          bind:value={nlText}
          class="cal-input"
          placeholder="z. B. Mittwoch 14 Uhr, 1 Stunde — KI erkennt die Zeit"
          onkeydown={(e) => { if (e.key === "Enter") applyTimeExtraction(); }}
        />
        <button
          type="button"
          class="cal-btn cal-btn-ghost"
          disabled={nlBusy || !nlText.trim()}
          onclick={applyTimeExtraction}
        >{nlBusy ? "…" : "⚡ Zeit"}</button>
      </div>

      <div class="cal-field-row">
        <label class="cal-field">
          <span>Beginn</span>
          <input type={form.all_day ? "date" : "datetime-local"} bind:value={form.start} class="cal-input" />
        </label>
        <label class="cal-field">
          <span>Ende</span>
          <input type={form.all_day ? "date" : "datetime-local"} bind:value={form.end} class="cal-input" />
        </label>
      </div>

      {#if conflicts.length > 0}
        <div class="cal-conflict">
          <div class="cal-conflict-head">
            <span>⚠ Termin überschneidet sich mit {conflicts.length} {conflicts.length === 1 ? "Termin" : "Terminen"}</span>
            <button type="button" class="cal-btn cal-btn-ghost cal-conflict-ai" disabled={conflictBusy} onclick={loadAlternatives}>
              {conflictBusy ? "…" : "⚡ KI-Alternativen"}
            </button>
          </div>
          <ul class="cal-conflict-list">
            {#each conflicts as c (c.id)}
              <li>{c.summary ?? "(ohne Titel)"} · {fmtInvWhen(c.start)}</li>
            {/each}
          </ul>
          {#if showAlternatives && aiSlots.length > 0}
            <div class="cal-slots">
              {#each aiSlots as slot (slot.start)}
                <button type="button" class="cal-slot" onclick={() => applySlot(slot)}>
                  <span class="cal-slot-when">{fmtInvWhen(slot.start)} – {fmtInvWhen(slot.end)}</span>
                  {#if slot.reason}<span class="cal-slot-reason">{slot.reason}</span>{/if}
                </button>
              {/each}
            </div>
          {/if}
          {#if showAlternatives && aiSlots.length === 0 && !conflictBusy}
            <p class="cal-conflict-none">Keine freien Alternativen gefunden.</p>
          {/if}
        </div>
      {/if}

      <label class="cal-check">
        <input type="checkbox" bind:checked={form.all_day} />
        <span>Ganztägig</span>
      </label>

      <label class="cal-field">
        <span>Ort</span>
        <input type="text" bind:value={form.location} class="cal-input" placeholder="Ort" />
      </label>

      <label class="cal-field">
        <span>Beschreibung</span>
        <textarea bind:value={form.description} class="cal-input cal-textarea" rows="3"></textarea>
      </label>

      {#if editingId !== null}
        <button type="button" class="cal-btn cal-btn-danger" onclick={() => { const ev = events.find(x => x.id === editingId); if (ev) removeEvent(ev); editorOpen = false; }}>
          Termin löschen
        </button>
      {/if}

      <div class="cal-modal-actions">
        <button type="button" class="cal-btn cal-btn-ghost" onclick={() => editorOpen = false}>Abbrechen</button>
        <button type="button" class="cal-btn cal-btn-primary" onclick={saveEvent}>Speichern</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .cal-app {
    display: flex;
    height: 100vh;
    background: var(--color-list);
    color: var(--color-text);
  }

  /* ── Sidebar ── */
  .cal-sidebar {
    width: 240px;
    min-width: 240px;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
  }
  .cal-sidebar-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--color-border);
  }
  .cal-back {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-s);
  }
  .cal-back:hover { color: var(--color-text); background: var(--color-active-wash); }
  .cal-brand { font-weight: 600; font-size: 15px; }

  .cal-cal-list { flex: 1; overflow-y: auto; padding: 8px; }
  .cal-upcoming { border-top: 1px solid var(--color-border); padding: 10px 8px; max-height: 200px; overflow-y: auto; }
  .cal-upcoming-head { font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: var(--color-text-secondary); padding: 0 6px 8px; }
  .cal-upcoming-item {
    display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
    background: none; border: none; color: var(--color-text); padding: 7px 8px;
    border-radius: var(--radius-m); cursor: pointer;
  }
  .cal-upcoming-item:hover { background: var(--color-active-wash); }
  .cal-upcoming-bell { color: var(--gold, #caa960); flex-shrink: 0; }
  .cal-upcoming-info { display: flex; flex-direction: column; gap: 1px; overflow: hidden; }
  .cal-upcoming-title { font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-upcoming-when { font-size: 11px; color: var(--color-text-secondary); }
  .cal-invitations { border-top: 1px solid var(--color-border); padding: 10px 8px; max-height: 240px; overflow-y: auto; }
  .cal-invitations-head {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--color-text-secondary); padding: 0 6px 8px;
  }
  .cal-invitations-badge {
    background: var(--gold, #caa960); color: var(--b-900, #051729);
    border-radius: 999px; font-size: 10px; font-weight: 700; padding: 1px 7px;
  }
  .cal-inv-item {
    display: flex; align-items: center; gap: 8px; padding: 8px;
    border-radius: var(--radius-m);
  }
  .cal-inv-item:hover { background: var(--color-active-wash); }
  .cal-inv-info { display: flex; flex-direction: column; gap: 1px; overflow: hidden; flex: 1; }
  .cal-inv-title { font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-inv-when { font-size: 11px; color: var(--color-text-secondary); }
  .cal-inv-organizer {
    font-size: 11px; color: var(--color-text-secondary);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .cal-inv-actions { display: flex; gap: 4px; flex-shrink: 0; }
  .cal-inv-accept, .cal-inv-decline {
    width: 26px; height: 26px; border-radius: var(--radius-s);
    border: 1px solid var(--color-border); background: none; cursor: pointer;
    font-size: 13px; line-height: 1; display: flex; align-items: center; justify-content: center;
  }
  .cal-inv-accept { color: #7ba05b; }
  .cal-inv-decline { color: #b3564f; }
  .cal-inv-accept:hover, .cal-inv-decline:hover { background: var(--color-active-wash); }
  .cal-inv-accept:disabled, .cal-inv-decline:disabled { opacity: 0.4; cursor: default; }
  .cal-inv-draft {
    width: 26px; height: 26px; border-radius: var(--radius-s);
    border: 1px solid var(--color-border); background: none; cursor: pointer;
    font-size: 13px; line-height: 1; display: flex; align-items: center; justify-content: center;
    color: var(--color-text-secondary);
  }
  .cal-inv-draft:hover { background: var(--color-active-wash); }
  .cal-inv-draft:disabled { opacity: 0.4; cursor: default; }
  .cal-inv-draftbox { padding: 0 8px 8px; }
  .cal-inv-draftbox .cal-input { font-size: 12px; resize: vertical; }
  .cal-empty {
    padding: 20px 12px;
    text-align: center;
    color: var(--color-text-secondary);
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: center;
  }
  .cal-cal-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 9px 12px;
    border: none;
    background: none;
    color: var(--color-text);
    border-radius: var(--radius-m);
    cursor: pointer;
    font-size: 14px;
    text-align: left;
  }
  .cal-cal-item:hover { background: var(--color-active-wash); }
  .cal-cal-item.active { background: var(--color-active-wash); font-weight: 600; }
  .cal-cal-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .cal-cal-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .cal-sidebar-footer { padding: 12px; border-top: 1px solid var(--color-border); }
  .cal-footer-actions { display: flex; flex-direction: column; gap: 6px; }
  .cal-footer-actions .cal-btn { width: 100%; }
  .cal-file-input { display: none; }

  /* ── Main ── */
  .cal-main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
  .cal-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    border-bottom: 1px solid var(--color-border);
  }
  .cal-month { font-size: 20px; font-weight: 600; margin: 0; }
  .cal-toolbar-right { display: flex; align-items: center; gap: 8px; }

  .cal-alert {
    margin: 12px 20px 0;
    padding: 10px 14px;
    background: color-mix(in srgb, var(--color-danger) 12%, transparent);
    color: var(--color-danger);
    border-radius: var(--radius-m);
    font-size: 13px;
  }

  /* ── Grid ── */
  .cal-grid {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    grid-auto-rows: minmax(96px, 1fr);
    overflow-y: auto;
  }
  .cal-grid-head {
    display: contents;
  }
  .cal-grid-head-cell {
    padding: 8px 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--color-border);
    border-right: 1px solid var(--color-border);
    background: var(--color-sidebar);
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .cal-cell {
    border-right: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
    padding: 4px 6px;
    cursor: pointer;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cal-cell:hover { background: var(--color-active-wash); }
  .cal-cell.other-month { opacity: 0.45; }
  .cal-cell.is-today .cal-cell-num {
    background: var(--color-accent);
    color: #fff;
    border-radius: 50%;
  }
  .cal-cell-num {
    font-size: 12px;
    font-weight: 600;
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .cal-cell-events { display: flex; flex-direction: column; gap: 2px; overflow: hidden; }
  .cal-event {
    display: flex;
    gap: 6px;
    align-items: baseline;
    padding: 2px 6px;
    border: none;
    background: var(--color-unread-wash);
    color: var(--color-text);
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: 12px;
    text-align: left;
    overflow: hidden;
  }
  .cal-event:hover { background: var(--color-active-wash); }
  .cal-event.cancelled { opacity: 0.5; text-decoration: line-through; }
  .cal-event-time { color: var(--color-text-secondary); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .cal-event-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* ── Buttons ── */
  .cal-btn {
    padding: 7px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--color-border);
    background: var(--color-list);
    color: var(--color-text);
    font-size: 13px;
    cursor: pointer;
  }
  .cal-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .cal-btn-ghost { border-color: transparent; background: transparent; color: var(--color-text-secondary); }
  .cal-btn-ghost:hover { background: var(--color-active-wash); color: var(--color-text); }
  .cal-btn-primary { background: var(--color-accent); border-color: var(--color-accent); color: #fff; font-weight: 600; }
  .cal-btn-primary:hover { background: var(--color-accent-hover); }
  .cal-btn-danger { border-color: var(--color-danger); color: var(--color-danger); background: transparent; }
  .cal-btn-danger:hover { background: color-mix(in srgb, var(--color-danger) 10%, transparent); }

  /* ── Modal ── */
  .cal-modal-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .cal-modal {
    background: var(--color-list);
    border-radius: var(--radius-l);
    padding: 24px;
    width: min(480px, 92vw);
    max-height: 90vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
  }
  .cal-modal h2 { margin: 0 0 4px; font-size: 18px; }
  .cal-field { display: flex; flex-direction: column; gap: 5px; font-size: 13px; color: var(--color-text-secondary); }
  .cal-field-row { display: flex; gap: 12px; }
  .cal-field-row .cal-field { flex: 1; }
  .cal-nl { display: flex; gap: 8px; align-items: center; margin-bottom: 4px; }
  .cal-nl .cal-input { flex: 1; }
  .cal-conflict {
    border: 1px solid var(--gold, #caa960);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    background: var(--color-active-wash);
  }
  .cal-conflict-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 13px; }
  .cal-conflict-ai { font-size: 12px; padding: 4px 10px; }
  .cal-conflict-list { margin: 8px 0 0; padding-left: 18px; font-size: 12px; color: var(--color-text-secondary); }
  .cal-conflict-none { font-size: 12px; color: var(--color-text-secondary); margin: 8px 0 0; }
  .cal-slots { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
  .cal-slot {
    display: flex; flex-direction: column; gap: 2px; text-align: left;
    border: 1px solid var(--color-border); border-radius: var(--radius-m);
    background: none; padding: 8px 10px; cursor: pointer;
  }
  .cal-slot:hover { background: var(--color-active-wash); border-color: var(--gold, #caa960); }
  .cal-slot-when { font-size: 13px; color: var(--color-text); }
  .cal-slot-reason { font-size: 11px; color: var(--color-text-secondary); }
  .cal-input {
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-m);
    background: var(--color-list);
    color: var(--color-text);
    font-size: 14px;
    font-family: inherit;
  }
  .cal-input:focus { outline: none; box-shadow: var(--fokus-ring); }
  .cal-textarea { resize: vertical; }
  .cal-check { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--color-text); }
  .cal-modal-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 4px; }

  /* ── Mini month (left pane) ── */
  .cal-mini { padding: 12px 14px; border-bottom: 1px solid var(--color-border); }
  .cal-mini-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
  .cal-mini-label { font-size: 13px; font-weight: 600; }
  .cal-mini-nav {
    background: none; border: none; color: var(--color-text-secondary);
    cursor: pointer; font-size: 16px; padding: 2px 8px; border-radius: var(--radius-s);
  }
  .cal-mini-nav:hover { background: var(--color-active-wash); color: var(--color-text); }
  .cal-mini-grid { display: grid; grid-template-columns: repeat(7, 1fr); gap: 1px; }
  .cal-mini-day {
    background: none; border: none; color: var(--color-text);
    font-size: 11px; padding: 4px 0; cursor: pointer; border-radius: var(--radius-s);
    font-variant-numeric: tabular-nums;
  }
  .cal-mini-day:hover { background: var(--color-active-wash); }
  .cal-mini-day.other { color: var(--color-text-secondary); opacity: 0.5; }
  .cal-mini-day.today { font-weight: 700; color: var(--color-accent); }
  .cal-mini-day.sel { background: var(--color-accent); color: #fff; font-weight: 600; }

  /* Calendar list checkboxes */
  .cal-cal-item input[type="checkbox"] { accent-color: var(--color-accent); margin: 0; }
  .cal-cal-item:has(input[type="checkbox"]:not(:checked)) { opacity: 0.5; }

  /* ── Toolbar additions ── */
  .cal-toolbar-left { display: flex; align-items: center; gap: 8px; }
  .cal-nav { padding: 7px 11px; font-size: 15px; }
  .cal-viewtoggle {
    display: inline-flex; background: var(--color-sidebar);
    border: 1px solid var(--color-border); border-radius: var(--radius-m); padding: 2px;
  }
  .cal-vt {
    background: none; border: none; color: var(--color-text-secondary);
    font-size: 13px; padding: 5px 12px; cursor: pointer; border-radius: var(--radius-s);
  }
  .cal-vt:hover { color: var(--color-text); }
  .cal-vt.active { background: var(--color-list); color: var(--color-text); font-weight: 600; box-shadow: 0 1px 2px rgba(0,0,0,0.08); }

  /* ── Week view ── */
  .cal-week { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
  .cal-week-head { display: grid; grid-template-columns: repeat(7, 1fr); border-bottom: 1px solid var(--color-border); }
  .cal-week-head-cell {
    display: flex; flex-direction: column; align-items: center; gap: 2px;
    padding: 8px 4px; border-right: 1px solid var(--color-border);
  }
  .cal-week-dow { font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--color-text-secondary); font-weight: 600; }
  .cal-week-num { font-size: 18px; font-weight: 600; width: 30px; height: 30px; display: flex; align-items: center; justify-content: center; border-radius: 50%; }
  .cal-week-head-cell.is-today .cal-week-num { background: var(--color-accent); color: #fff; }
  .cal-week-body { flex: 1; display: grid; grid-template-columns: repeat(7, 1fr); overflow-y: auto; }
  .cal-week-col { border-right: 1px solid var(--color-border); padding: 6px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; min-height: 120px; }
  .cal-week-col:last-child { border-right: none; }
  .cal-week-col:hover { background: var(--color-active-wash); }
  .cal-week-col.is-today { background: color-mix(in srgb, var(--color-accent) 5%, transparent); }
  .cal-event-block { flex-direction: column; align-items: flex-start; gap: 2px; padding: 6px 8px; border-left: 3px solid var(--color-accent); background: var(--color-unread-wash); }

  /* ── Day view ── */
  .cal-dayview { flex: 1; overflow-y: auto; padding: 16px 20px; display: flex; flex-direction: column; gap: 8px; }
  .cal-day-empty { color: var(--color-text-secondary); font-size: 14px; padding: 40px 0; text-align: center; }
  .cal-day-item {
    display: flex; align-items: center; gap: 12px; text-align: left;
    padding: 12px 14px; border: 1px solid var(--color-border); border-radius: var(--radius-m);
    background: var(--color-list); cursor: pointer;
  }
  .cal-day-item:hover { background: var(--color-active-wash); }
  .cal-day-item.sel { border-color: var(--color-accent); box-shadow: var(--fokus-ring); }
  .cal-day-item.cancelled { opacity: 0.5; }
  .cal-day-time { font-size: 13px; font-weight: 600; color: var(--color-text-secondary); min-width: 64px; font-variant-numeric: tabular-nums; }
  .cal-day-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .cal-day-info { display: flex; flex-direction: column; gap: 2px; overflow: hidden; }
  .cal-day-title { font-size: 14px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-day-loc { font-size: 12px; color: var(--color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* ── Detail pane (right) ── */
  .cal-detail {
    width: 300px; min-width: 300px; background: var(--color-sidebar);
    border-left: 1px solid var(--color-border); overflow-y: auto;
  }
  .cal-detail-inner { padding: 18px; display: flex; flex-direction: column; gap: 14px; }
  .cal-detail-top { display: flex; align-items: center; justify-content: space-between; }
  .cal-detail-cal { font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .cal-detail-close { background: none; border: none; color: var(--color-text-secondary); font-size: 20px; cursor: pointer; padding: 0 6px; border-radius: var(--radius-s); line-height: 1; }
  .cal-detail-close:hover { background: var(--color-active-wash); color: var(--color-text); }
  .cal-detail-title { margin: 0; font-size: 18px; font-weight: 600; line-height: 1.3; }
  .cal-detail-rows { display: flex; flex-direction: column; gap: 10px; }
  .cal-detail-row { display: flex; align-items: flex-start; gap: 10px; font-size: 13px; color: var(--color-text); }
  .cal-detail-ico { color: var(--color-text-secondary); width: 16px; flex-shrink: 0; text-align: center; }
  .cal-detail-desc { margin: 0; font-size: 13px; line-height: 1.5; color: var(--color-text-secondary); white-space: pre-wrap; word-break: break-word; }
  .cal-detail-actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 6px; }
  .cal-detail-empty { padding: 40px 24px; text-align: center; color: var(--color-text-secondary); font-size: 13px; }
</style>
