<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    getCalendars, listEvents, createEvent, updateEvent, deleteEvent, inviteEvent,
    getCalDavSettings, syncCalDav, importEvents, getEventIcs,
    listInvitations, acceptInvitation, declineInvitation, listAccounts,
    getConflicts, getConflictAlternatives, extractTime, rsvpDraft,
    createTodo, meetingPrep, smartSchedule, agendaDigest,
    type CalendarInfo, type EventInfo, type InvitationInfo, type TimeSlot,
    type MeetingPrepResult, type ScheduleSuggestion, type AgendaDigestResult,
  } from "$lib/services/tauri";
  import { t, translate, lang } from "$lib/i18n";
  import { calendarView } from "$lib/stores/calendarView";
  import { dataVersion } from "$lib/stores/invalidation";
  import { germanHolidays } from "$lib/holidays";
  import Huelle from "$lib/components/Huelle.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import RecipientInput from "$lib/components/RecipientInput.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";

  // Dates follow the UI language, not the browser's (CI ABGLEICH RL-G4):
  // "Oktober 2026" in German, "October 2026" in English.
  let locale = $derived($lang === "de" ? "de-DE" : "en-US");

  // Synthetic built-in "Feiertage" calendar (German public holidays).
  const HOLIDAY_CAL_ID = -1;
  const HOLIDAY_CAL: CalendarInfo = {
    id: HOLIDAY_CAL_ID, name: "Feiertage", color: "var(--am-kalender-1)",
    read_only: true, last_synced_at: null,
  };
  // ─── State ───────────────────────────────────
  let calendars = $state<CalendarInfo[]>([]);
  let events = $state<EventInfo[]>([]);

  // All calendars shown in the list: built-in holidays first, then CalDAV.
  let allCalendars = $derived<CalendarInfo[]>([HOLIDAY_CAL, ...calendars]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let syncing = $state(false);
  // The search sits in the header (CI HB-SUCHE, RL-G1); it filters the
  // shown events as the field in the column did.
  let calSearch = $state("");

  // Current date anchor for the visible range.
  let viewDate = $state(new Date());
  let today = $state(new Date());

  // Center view mode.
  let viewMode = $state<"month" | "week" | "day">("month");

  // Assistant-driven view sync (Concept §10.2): when the effect router sets the
  // calendarView store, follow it. One-way (store → local); manual navigation
  // after mount is preserved until the next assistant effect arrives.
  $effect(() => {
    const v = $calendarView;
    viewDate = v.viewDate;
    viewMode = v.viewMode;
  });
  // Event shown in the right detail pane.
  let selectedEvent = $state<EventInfo | null>(null);
  // Delete-confirmation dialog (Backspace/Delete or the "Löschen" button).
  let showDeleteConfirm = $state(false);
  let pendingDeleteEvent = $state<EventInfo | null>(null);
  // Multi-calendar: which calendars are visible (by id).
  let visibleCals = $state<Set<number>>(new Set());

  // ─── iMIP invitation queue (Phase 2) ─────────
  let invitations = $state<InvitationInfo[]>([]);
  let invBusy = $state<string | null>(null);

  async function loadInvitations() {
    try {
      const raw = await listInvitations();
      // T10 (Review 2026-09-13): doppelte event_uid im Backend-Result
      // → `each_key_duplicate` im keyed Each der Einladungsliste.
      const first = new Map<string, InvitationInfo>();
      for (const inv of raw) if (!first.has(inv.event_uid)) first.set(inv.event_uid, inv);
      invitations = [...first.values()];
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
      return d.toLocaleString(locale, {
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
        inv.summary ?? translate("calendar.invitation"),
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
      aiSlots = await getConflictAlternatives(form.summary || translate("calendar.title"), start, end, defaultCalId());
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

  // ─── Phase 4 — Meeting-Prep / Smart Scheduling / Digest ───
  let prepResult = $state<MeetingPrepResult | null>(null);
  let prepBusy = $state(false);
  let showPrep = $state(false);

  let smartSlots = $state<ScheduleSuggestion[]>([]);
  let smartBusy = $state(false);
  let showSmart = $state(false);

  let digest = $state<AgendaDigestResult | null>(null);
  let digestBusy = $state(false);

  async function loadMeetingPrep() {
    if (!form.start || prepBusy) return;
    prepBusy = true;
    showPrep = true;
    prepResult = null;
    try {
      const start = toUtc(form.start, form.all_day);
      prepResult = await meetingPrep(form.summary || translate("calendar.title"), start, []);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      prepBusy = false;
    }
  }

  async function loadSmartSchedule() {
    if (smartBusy) return;
    smartBusy = true;
    showSmart = true;
    smartSlots = [];
    try {
      const request = `${form.summary || translate("calendar.title")}${form.start ? `, ${translate("calendar.currently")} ${form.start}` : ""}`;
      smartSlots = await smartSchedule(request, "", "", translate("calendar.smartScheduleHint"));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      smartBusy = false;
    }
  }

  function applySmartSlot(slot: ScheduleSuggestion) {
    form.start = toLocalInput(new Date(slot.start));
    form.end = toLocalInput(new Date(slot.end));
    form.all_day = false;
    smartSlots = [];
    showSmart = false;
    checkConflicts();
  }

  async function loadDigest() {
    if (digestBusy) return;
    digestBusy = true;
    digest = null;
    try {
      digest = await agendaDigest(undefined, 7);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      digestBusy = false;
    }
  }

  // Re-check conflicts whenever the time fields change.
  $effect(() => {
    if (editorOpen && form.start && form.end) {
      const t = setTimeout(checkConflicts, 400);
      return () => clearTimeout(t);
    }
  });

  // Fixed palette for calendar colors, indexed by position (CI RL-R5): six
  // tones from the blue and gold ladders, each 3:1 as a graphic in both
  // modes. Holidays keep tone 1, so connected calendars start at tone 2.
  const CAL_COLORS = [2, 3, 4, 5, 6, 1].map((n) => `var(--am-kalender-${n})`);
  function calColor(cal: CalendarInfo | undefined): string {
    if (!cal) return CAL_COLORS[0];
    if (cal.id === HOLIDAY_CAL_ID) return cal.color ?? CAL_COLORS[0];
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
    let calEvents = events.filter(
      (ev) => visibleCals.size === 0 || visibleCals.has(ev.calendar_id)
    );
    const q = calSearch.trim().toLowerCase();
    if (q) {
      calEvents = calEvents.filter((ev) =>
        (ev.summary ?? "").toLowerCase().includes(q)
      );
    }
    const hols = visibleCals.has(HOLIDAY_CAL_ID) ? holidayEvents : [];
    return [...calEvents, ...hols];
  });

  // ─── Derived: month grid ─────────────────────
  const WEEKDAYS = $derived(
    [1, 2, 3, 4, 5, 6, 7].map((d) =>
      new Date(2024, 0, d).toLocaleDateString(locale, { weekday: "short" })
    )
  );

  let monthLabel = $derived(
    viewDate.toLocaleDateString(locale, { month: "long", year: "numeric" })
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
    return ev.occurrence_end ?? ev.end ?? ev.start;
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

  function calById(id: number): CalendarInfo | undefined {
    return allCalendars.find((c) => c.id === id);
  }
  function dowShort(d: Date): string {
    return d.toLocaleDateString(locale, { weekday: "short" });
  }

  // Week label: "12. – 18. Sep 2026".
  let weekLabel = $derived.by(() => {
    const a = weekDays[0];
    const b = weekDays[6];
    const opts: Intl.DateTimeFormatOptions = { day: "numeric", month: "short" };
    const ay = a.getFullYear() === b.getFullYear() ? "" : ` ${a.getFullYear()}`;
    return `${a.toLocaleDateString(locale, opts)}${ay} – ${b.toLocaleDateString(locale, { ...opts, year: "numeric" })}`;
  });

  let dayLabel = $derived(
    viewDate.toLocaleDateString(locale, { weekday: "long", day: "numeric", month: "long", year: "numeric" })
  );

  // The big label in the toolbar, per view mode.
  let periodLabel = $derived(
    viewMode === "week" ? weekLabel : viewMode === "day" ? dayLabel : monthLabel
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
    viewDate.toLocaleDateString(locale, { month: "short", year: "numeric" })
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
      // Recurring-Expansion liefert dieselbe id mehrfach (je Occurrence) —
      // das ist legitim; Keyed-Each nutzen deshalb evKey (T10).
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
      if (calId === null) throw new Error(translate("calendar.noCalForImport"));
      const { imported } = await importEvents(calId, ics);
      await loadEvents();
      if (imported === 0) error = translate("calendar.importNoEvents");
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

  // Rechtsklick (T3, Review 2026-09-13): Kontextmenü auf Termine.
  // ─── Phone (Kai, 7.10.2026: "GUI nicht gut") ─────────────────────────
  // One head line instead of four, the month in compact cells with dots,
  // the chosen day's events as a list below — as Apple's and Google's
  // calendars on a phone.
  let handy = $state(false);
  let tagGewaehlt = $state(new Date());
  let mehrMenue = $state<{ x: number; y: number } | null>(null);
  let mehrEintraege = $derived([
    { label: $t("common.refresh"), action: () => void handleSync() },
    { label: $t("calendar.importIcs"), action: () => triggerImport() },
  ]);
  let tagTermine = $derived(eventsByDay.get(dayKey(tagGewaehlt)) ?? []);
  let tagTitel = $derived(
    tagGewaehlt.toLocaleDateString($lang === "de" ? "de-DE" : "en-GB", { weekday: "long", day: "numeric", month: "long" })
  );

  onMount(() => {
    const mq = window.matchMedia("(max-width: 40rem)");
    handy = mq.matches;
    const wechsel = (e: MediaQueryListEvent) => (handy = e.matches);
    mq.addEventListener("change", wechsel);
    return () => mq.removeEventListener("change", wechsel);
  });

  /** A tap on a day: on the phone it chooses the day, elsewhere a new event. */
  function tagAntippen(day: Date) {
    if (handy) tagGewaehlt = day;
    else openNewEventOn(day);
  }

  let evCtx = $state<{ x: number; y: number; event: EventInfo } | null>(null);
  // Ein actives Menu rendert nur bei geöffnetem ctx — die Labels werden
  // beim Öffnen frisch übersetzt, momentan über Snapshot-Ausdrücke.
  let evCtxItems = $derived.by(() => {
    const c = evCtx;
    if (!c) return [];
    return [
      { label: translate("calendar.editEvent"), action: () => openEditEvent(c.event) },
      { label: translate("calendar.deleteEvent"), danger: true, action: () => removeEvent(c.event) },
    ];
  });

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
    participants: [] as string[],
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
      participants: [],
    };
    editorOpen = true;
  }

  function openEditEvent(ev: EventInfo) {
    editingId = ev.id;
    const endVal = ev.end ?? ev.start;
    form = {
      summary: ev.summary ?? "",
      start: ev.all_day ? ev.start.slice(0, 10) : toLocalInput(new Date(ev.start)),
      end: ev.all_day ? endVal.slice(0, 10) : toLocalInput(new Date(endVal)),
      all_day: ev.all_day,
      location: ev.location ?? "",
      description: ev.description ?? "",
      reminder_minutes: 15,
      participants: (ev.attendees ?? []).map((a) => a.email),
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

  let showInviteDialog = $state(false);
  let pendingInviteEventId: number | null = $state(null);
  let pendingInviteAttendees: { email: string; name?: string }[] = $state([]);

  async function saveEvent() {
    if (editingId === null && defaultCalId() === null) return;
    error = null;
    try {
      const start = fromLocalInput(form.start, form.all_day);
      const end = form.end ? fromLocalInput(form.end, form.all_day) : undefined;
      const attendees = form.participants
        .map((e) => e.trim())
        .filter(Boolean)
        .map((email) => ({ email, rsvp: true }));
      let eventId: number | null = null;
      if (editingId === null) {
        const created = await createEvent({
          calendar_id: defaultCalId()!,
          summary: form.summary,
          start,
          end,
          all_day: form.all_day,
          location: form.location || undefined,
          description: form.description || undefined,
          reminder_minutes: form.reminder_minutes,
          attendees,
        });
        eventId = created.id;
      } else {
        await updateEvent(editingId, {
          summary: form.summary,
          start,
          end,
          all_day: form.all_day,
          location: form.location || undefined,
          description: form.description || undefined,
          reminder_minutes: form.reminder_minutes,
          attendees,
        });
        eventId = editingId;
      }
      editorOpen = false;
      if (attendees.length > 0 && eventId != null) {
        pendingInviteEventId = eventId;
        pendingInviteAttendees = attendees;
        showInviteDialog = true;
      }
      await loadEvents();
      await loadUpcoming();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function confirmSendInvites() {
    showInviteDialog = false;
    const eventId = pendingInviteEventId;
    const attendees = pendingInviteAttendees;
    pendingInviteEventId = null;
    pendingInviteAttendees = [];
    if (eventId == null || attendees.length === 0) return;
    try {
      const accts = await listAccounts();
      const acctId = accts.length > 0 ? accts[0].id : 1;
      await inviteEvent(eventId, acctId, attendees);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function skipInvites() {
    showInviteDialog = false;
    pendingInviteEventId = null;
    pendingInviteAttendees = [];
  }

  // Open the delete-confirmation dialog for an event (button or Backspace/Delete).
  function removeEvent(ev: EventInfo) {
    if (showDeleteConfirm) return;
    pendingDeleteEvent = ev;
    showDeleteConfirm = true;
  }

  async function confirmDeleteEvent() {
    const ev = pendingDeleteEvent;
    showDeleteConfirm = false;
    pendingDeleteEvent = null;
    if (!ev) return;
    try {
      await deleteEvent(ev.id);
      if (selectedEvent?.id === ev.id) clearSelection();
      await loadEvents();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function cancelDeleteEvent() {
    showDeleteConfirm = false;
    pendingDeleteEvent = null;
  }

  // Stabler Each-Key über Occurrences (T10, Review 2026-09-13):
  // Recurring-Events liefern dieselbe id pro Occurrence; ohne occurrence_start
  // fällt der Key auf die Serien-Transferzeit zurück.
  function evKey(ev: EventInfo): string {
    return `${ev.id}@${ev.occurrence_start ?? ev.start}`;
  }

  function isInputFocused(): boolean {
    const tag = (document.activeElement?.tagName || "").toUpperCase();
    const editable = document.activeElement?.getAttribute("contenteditable") === "true";
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || editable;
  }

  // Focus into the editor's first field when it opens (CI HB-DIALOG), so
  // the keyboard lands in the dialog and Escape reaches it.
  let summaryInput = $state<HTMLInputElement | null>(null);
  $effect(() => {
    if (editorOpen && summaryInput) summaryInput.focus();
  });

  // On the phone picking a day in the column closes the shell's sheet (RL-G2).
  let spalteOffen = $state(false);

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && showDeleteConfirm) {
      cancelDeleteEvent();
      return;
    }
    if (isInputFocused()) return;
    if ((e.key === "Backspace" || e.key === "Delete" || e.key === "Del") && selectedEvent) {
      e.preventDefault();
      removeEvent(selectedEvent);
    }
  }

  function fmtEventTime(ev: EventInfo): string {
    if (ev.all_day) return translate("calendar.allDay");
    const d = new Date(effStart(ev));
    if (isNaN(d.getTime())) return "";
    return d.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
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
    const time = d.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
    if (sameDay) return `heute ${time}`;
    if (diffMin < 60 * 24) return `morgen ${time}`;
    return d.toLocaleDateString(locale, { day: "numeric", month: "short" }) + ` ${time}`;
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
        ? s.toLocaleDateString(locale, dateOpt)
        : `${s.toLocaleDateString(locale, dateOpt)} – ${e.toLocaleDateString(locale, dateOpt)}`;
    }
    const timeOpt: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" };
    const base = s.toLocaleDateString(locale, dateOpt);
    if (sameDay) {
      return `${base}, ${s.toLocaleTimeString(locale, timeOpt)} – ${e.toLocaleTimeString(locale, timeOpt)}`;
    }
    return `${base}, ${s.toLocaleTimeString(locale, timeOpt)} – ${e.toLocaleDateString(locale, dateOpt)}, ${e.toLocaleTimeString(locale, timeOpt)}`;
  }

  onMount(async () => {
    await loadCalendars();
    await loadEvents();
    await loadUpcoming();
    loadInvitations();
  });

  // Reload after an assistant plan execution (Concept §10.5). Skips the first
  // run so the onMount load is not duplicated.
  let assistantReloaded = false;
  $effect(() => {
    const v = $dataVersion;
    if (!assistantReloaded) { assistantReloaded = true; return; }
    void (async () => {
      await loadCalendars();
      await loadEvents();
      await loadUpcoming();
      loadInvitations();
    })();
  });
</script>

<svelte:head><title>{tabTitel($t("calendar.title"))}</title></svelte:head>

<Huelle bereich="calendar" bind:spalteOffen bind:suche={calSearch} suchePlatzhalter={$t("calendar.searchPlaceholder")}>
  {#snippet spalte()}
    <!-- Mini month for quick navigation -->
    <div class="cal-mini">
      <div class="cal-mini-head">
        <button type="button" class="btn btn-still btn-symbol btn-klein" onclick={() => shiftMini(-1)} aria-label={$t("calendar.prevMonth")} title={$t("calendar.prevMonth")}><Symbol name="chevron-links" size={16} /></button>
        <span class="cal-mini-label">{miniMonthLabel}</span>
        <button type="button" class="btn btn-still btn-symbol btn-klein" onclick={() => shiftMini(1)} aria-label={$t("calendar.nextMonth")} title={$t("calendar.nextMonth")}><Symbol name="chevron-rechts" size={16} /></button>
      </div>
      <div class="cal-mini-grid">
        {#each gridDays as d (localDayKey(d))}
          <button
            type="button"
            class="cal-mini-day"
            class:other={d.getMonth() !== viewDate.getMonth()}
            class:today={localDayKey(d) === localDayKey(today)}
            class:sel={localDayKey(d) === localDayKey(viewDate)}
            onclick={() => { viewDate = new Date(d); spalteOffen = false; }}
          >{d.getDate()}</button>
        {/each}
      </div>
    </div>

    <div class="cal-cal-list">
      <div class="cal-spalte-titel">{$t("calendar.title")}</div>
      {#each allCalendars as cal (cal.id)}
        <label class="cal-cal-item">
          <input
            type="checkbox"
            checked={calVisible(cal)}
            onchange={() => toggleCal(cal)}
          />
          <span class="cal-cal-dot" style="background: {calColor(cal)}"></span>
          <span class="cal-cal-name">{cal.name ?? $t("calendar.title")}</span>
        </label>
      {/each}
      {#if calendars.length === 0}
        <div class="cal-empty">
          <p>{$t("calendar.noCaldav")}</p>
          <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => goto("/settings")}>
            {$t("calendar.connectCaldav")}
          </button>
        </div>
      {/if}
    </div>

    {#if invitations.length > 0}
      <div class="cal-invitations">
        <div class="cal-invitations-head">
          <span>{$t("calendar.invitations")}</span>
          <span class="cal-invitations-badge">{invitations.length}</span>
        </div>
        {#each invitations as inv (inv.event_uid)}
          <div class="cal-inv-item">
            <div class="cal-inv-info">
              <span class="cal-inv-title">{inv.summary ?? $t("calendar.invitation")}</span>
              {#if inv.start_at}<span class="cal-inv-when">{fmtInvWhen(inv.start_at)}</span>{/if}
              <span class="cal-inv-organizer">{inv.organizer}</span>
            </div>
            <div class="cal-inv-actions">
              <button
                type="button"
                class="btn btn-still btn-symbol btn-klein"
                title={$t("calendar.aiDraft")}
                aria-label={$t("calendar.aiDraft")}
                disabled={invDraftBusy}
                onclick={() => draftRsvp(inv, "ACCEPTED")}
              ><Symbol name="ai" size={16} /></button>
              <button
                type="button"
                class="btn btn-sekundaer btn-klein"
                title={$t("calendar.accept")}
                disabled={invBusy !== null}
                onclick={() => respondToInvitation(inv, "ACCEPTED")}
              >{$t("calendar.accept")}</button>
              <button
                type="button"
                class="btn btn-still btn-klein"
                title={$t("calendar.decline")}
                disabled={invBusy !== null}
                onclick={() => respondToInvitation(inv, "DECLINED")}
              >{$t("calendar.decline")}</button>
            </div>
          </div>
          {#if invDraft?.uid === inv.event_uid}
            <div class="feld cal-inv-draftbox">
              <textarea rows="3" aria-label={$t("calendar.aiDraft")} value={invDraft.text} oninput={(e) => (invDraft = { uid: inv.event_uid, text: e.currentTarget.value })}></textarea>
            </div>
          {/if}
        {/each}
      </div>
    {/if}

    {#if upcoming.length > 0}
      <div class="cal-upcoming">
        <div class="cal-spalte-titel cal-upcoming-head">{$t("calendar.upcoming")}</div>
        {#each upcoming as ev (evKey(ev))}
          <button type="button" class="cal-upcoming-item" onclick={() => selectEvent(ev)}
                  oncontextmenu={(e) => { e.preventDefault(); evCtx = { x: e.clientX, y: e.clientY, event: ev }; }}>
            <span class="cal-upcoming-bell"><Symbol name="zeitplan" size={16} /></span>
            <div class="cal-upcoming-info">
              <span class="cal-upcoming-title">{ev.summary ?? $t("calendar.untitled")}</span>
              <span class="cal-upcoming-when">{fmtUpcomingWhen(ev)}</span>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  {/snippet}

<div class="cal-app" class:detail-open={!!selectedEvent}>
  <main class="cal-main">
    <!-- HB-SEITENKOPF: the period as the title (28 px), the navigation and
         the page's actions right; "Neuer Termin" is the one primary. -->
    <div class="seitenkopf cal-kopf">
      <div class="seitenkopf-zeile">
        <h1>{periodLabel}</h1>
        {#if handy}
          <div class="cal-kopf-nav">
            <button type="button" class="btn btn-still btn-symbol" onclick={() => shiftPeriod(-1)} aria-label={$t("calendar.prevPeriod")} title={$t("calendar.prevPeriod")}><Symbol name="chevron-links" size={20} /></button>
            <button type="button" class="btn btn-still" onclick={() => { goToday(); tagGewaehlt = new Date(); }}>{$t("calendar.today")}</button>
            <button type="button" class="btn btn-still btn-symbol" onclick={() => shiftPeriod(1)} aria-label={$t("calendar.nextPeriod")} title={$t("calendar.nextPeriod")}><Symbol name="chevron-rechts" size={20} /></button>
          </div>
        {/if}
      </div>
      <div class="btn-reihe">
        {#if !handy}
          <div class="cal-kopf-nav">
            <button type="button" class="btn btn-still btn-symbol" onclick={() => shiftPeriod(-1)} aria-label={$t("calendar.prevPeriod")} title={$t("calendar.prevPeriod")}><Symbol name="chevron-links" size={20} /></button>
            <button type="button" class="btn btn-still" onclick={goToday}>{$t("calendar.today")}</button>
            <button type="button" class="btn btn-still btn-symbol" onclick={() => shiftPeriod(1)} aria-label={$t("calendar.nextPeriod")} title={$t("calendar.nextPeriod")}><Symbol name="chevron-rechts" size={20} /></button>
          </div>
        {/if}
        <div class="cal-viewtoggle" role="group" aria-label={$t("calendar.view")}>
          <button type="button" class="cal-vt" class:active={viewMode === "month"} aria-pressed={viewMode === "month"} onclick={() => setViewMode("month")}>{$t("calendar.viewMonth")}</button>
          <button type="button" class="cal-vt" class:active={viewMode === "week"} aria-pressed={viewMode === "week"} onclick={() => setViewMode("week")}>{$t("calendar.viewWeek")}</button>
          <button type="button" class="cal-vt" class:active={viewMode === "day"} aria-pressed={viewMode === "day"} onclick={() => setViewMode("day")}>{$t("calendar.viewDay")}</button>
        </div>
        {#if handy}
          <!-- Rarely needed: refresh and import behind "Mehr"; the new event
               as a named sign (CI G4: known action, in the head, tooltip). -->
          <div class="cal-kopf-rechts">
            <button type="button" class="btn btn-still btn-symbol" aria-haspopup="menu" aria-label={$t("common.more")} title={$t("common.more")}
              onclick={(e) => { const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); mehrMenue = { x: r.left - 160, y: r.bottom + 4 }; }}>
              <Symbol name="mehr" size={20} />
            </button>
            <button type="button" class="btn btn-primaer btn-symbol" onclick={() => openNewEventOn(tagGewaehlt)} aria-label={$t("calendar.createEvent")} title={$t("calendar.createEvent")}>
              <Symbol name="plus" size={20} />
            </button>
          </div>
        {:else}
          <button
            type="button"
            class="btn btn-still btn-symbol"
            onclick={handleSync}
            disabled={syncing}
            title={syncing ? $t("common.syncing") : $t("common.refresh")}
            aria-label={syncing ? $t("common.syncing") : $t("common.refresh")}
          >
            <Symbol name="neu-laden" size={20} />
          </button>
          <!-- ".ics importieren" was in the column footer; now a second button
               in the page head (CI ABGLEICH RL-G1). -->
          <button type="button" class="btn btn-sekundaer" onclick={triggerImport} disabled={importing}>{$t("calendar.importIcs")}</button>
          <button type="button" class="btn btn-primaer" onclick={() => openNewEvent()}>
            <Symbol name="plus" size={16} />
            {$t("calendar.createEvent")}
          </button>
        {/if}
        <input
          type="file"
          accept=".ics,text/calendar"
          class="cal-file-input"
          bind:this={importInput}
          onchange={onImportFile}
        />
      </div>
    </div>
    <ContextMenu menu={mehrMenue} items={mehrEintraege} onclose={() => (mehrMenue = null)} />

    {#if error}
      <div class="hinweis cal-alert" data-art="fehler" role="alert"><Symbol name="achtung" size={16} /><span>{error}</span></div>
    {/if}

    {#if viewMode === "month"}
      <div class="cal-monat" class:handy>
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
            class:gewaehlt={handy && dayKey(day) === dayKey(tagGewaehlt)}
            onclick={() => tagAntippen(day)}
          >
            <span class="cal-cell-num">{day.getDate()}</span>
            {#if handy}
              <!-- Phone: a dot per event (up to three) in its calendar's colour;
                   the titles stand in the list below. -->
              <span class="cal-punkte" aria-hidden="true">
                {#each (eventsByDay.get(dayKey(day)) ?? []).slice(0, 3) as ev (evKey(ev))}
                  <span class="cal-punkt" style="background: {calColor(calById(ev.calendar_id) ?? calendars[0])}"></span>
                {/each}
              </span>
            {:else}
            <div class="cal-cell-events">
              {#each (eventsByDay.get(dayKey(day)) ?? []) as ev (evKey(ev))}
                <button
                  type="button"
                  class="cal-event"
                  class:cancelled={ev.status === "CANCELLED"}
                  style="border-left-color: {calColor(calById(ev.calendar_id) ?? calendars[0])}"
                  onclick={(e) => { e.stopPropagation(); selectEvent(ev); }}
                  oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); evCtx = { x: e.clientX, y: e.clientY, event: ev }; }}
                  title={ev.summary ?? ""}
                >
                  <span class="cal-event-time">{fmtEventTime(ev)}</span>
                  <span class="cal-event-title">{ev.summary ?? $t("calendar.untitled")}</span>
                </button>
              {/each}
            </div>
            {/if}
          </div>
        {/each}
      </div>
      {#if handy}
        <!-- The chosen day's events, in full. -->
        <section class="cal-tagesliste" aria-label={tagTitel}>
          <h2>{tagTitel}</h2>
          {#if tagTermine.length === 0}
            <p class="cal-tagesliste-leer">{$t("calendar.noEventsToday")}</p>
          {:else}
            {#each tagTermine as ev (evKey(ev))}
              <button type="button" class="cal-tag-termin" class:cancelled={ev.status === "CANCELLED"} onclick={() => selectEvent(ev)}>
                <span class="cal-tag-farbe" style="background: {calColor(calById(ev.calendar_id) ?? calendars[0])}"></span>
                <span class="cal-tag-zeit">{fmtEventTime(ev)}</span>
                <span class="cal-tag-titel">{ev.summary ?? $t("calendar.untitled")}</span>
              </button>
            {/each}
          {/if}
        </section>
      {/if}
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
              {#each (eventsByDay.get(localDayKey(d)) ?? []) as ev (evKey(ev))}
                <button
                  type="button"
                  class="cal-event cal-event-block"
                  class:cancelled={ev.status === "CANCELLED"}
                  style="border-left-color: {calColor(calById(ev.calendar_id) ?? calendars[0])}"
                  onclick={(e) => { e.stopPropagation(); selectEvent(ev); }}
                  oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); evCtx = { x: e.clientX, y: e.clientY, event: ev }; }}
                >
                  <span class="cal-event-time">{fmtEventTime(ev)}</span>
                  <span class="cal-event-title">{ev.summary ?? $t("calendar.untitled")}</span>
                </button>
              {/each}
            </div>
          {/each}
        </div>
      </div>
    {:else}
      <div class="cal-dayview">
        <div class="cal-digest-bar">
          <button type="button" class="btn btn-sekundaer" disabled={digestBusy} onclick={loadDigest}>
            <Symbol name="ai" size={16} /> {digestBusy ? "…" : $t("calendar.morningDigest")}
          </button>
        </div>
        {#if digest}
          <div class="karte cal-digest">
            <p class="cal-digest-text">{digest.digest}</p>
            {#if digest.priorities.length > 0}
              <div class="cal-digest-section"><strong>{$t("calendar.priorities")}</strong><ul>{#each digest.priorities as p (p)}<li>{p}</li>{/each}</ul></div>
            {/if}
            {#if digest.followups.length > 0}
              <div class="cal-digest-section"><strong>Follow-ups</strong><ul>{#each digest.followups as f (f)}<li>{f}</li>{/each}</ul></div>
            {/if}
          </div>
        {/if}
        {#if dayEvents.length === 0}
          <EmptyState title={$t("calendar.noEventsToday")} icon="kalender" />
        {:else}
          {#each dayEvents as ev (evKey(ev))}
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
                <span class="cal-day-title">{ev.summary ?? $t("calendar.untitled")}</span>
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
          <span class="cal-detail-cal">
            {#if cal}<span class="cal-cal-dot" style="background: {calColor(cal)}" aria-hidden="true"></span>{/if}
            {cal?.name ?? $t("calendar.title")}
          </span>
          <button type="button" class="btn btn-still btn-symbol" onclick={clearSelection} aria-label={$t("calendar.close")} title={$t("calendar.close")}><Symbol name="schliessen" size={20} /></button>
        </div>
        <h2 class="cal-detail-title">{ev.summary ?? $t("calendar.untitled")}</h2>

        <div class="cal-detail-rows">
          <div class="cal-detail-row">
            <span class="cal-detail-ico"><Symbol name="zeitplan" size={16} /></span>
            <span>{fmtEventRange(ev)}</span>
          </div>
          {#if ev.location}
            <div class="cal-detail-row">
              <span class="cal-detail-ico"><Symbol name="ort" size={16} /></span>
              <span>{ev.location}</span>
            </div>
          {/if}
          {#if ev.rrule}
            <div class="cal-detail-row">
              <span class="cal-detail-ico"><Symbol name="wiederholen" size={16} /></span>
              <span>{$t("calendar.recurring")}</span>
            </div>
          {/if}
          {#if ev.organizer}
            <div class="cal-detail-row">
              <span class="cal-detail-ico"><Symbol name="post" size={16} /></span>
              <span>{ev.organizer}</span>
            </div>
          {/if}
          {#if ev.attendees?.length}
            <div class="cal-detail-row">
              <span class="cal-detail-ico"><Symbol name="team" size={16} /></span>
              <div class="cal-attendees">
                {#each ev.attendees as a (a.email)}
                  {@const ps = a.part_stat?.toLowerCase() ?? 'needsaction'}
                  <div class="cal-attendee cal-attendee-{ps}">
                    <span class="cal-attendee-name">{a.name ?? a.email}</span>
                    <span class="cal-attendee-status">{$t(`calendar.rsvp.${ps === 'needsaction' ? 'needsAction' : ps}`)}</span>
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </div>

        {#if ev.description}
          <p class="cal-detail-desc">{ev.description}</p>
        {/if}

        <div class="cal-detail-actions">
          <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => openEditEvent(ev)}>{$t("calendar.edit")}</button>
          <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => handleExport(ev)}>ICS</button>
          <!-- Destructive: the object goes in the word (AM-KNOPF btn-gefahr). -->
          <button type="button" class="btn btn-gefahr btn-klein" onclick={() => removeEvent(ev)}>{$t("calendar.deleteEvent")}</button>
        </div>
      </div>
    {:else}
      <EmptyState title={$t("calendar.selectEvent")} icon="kalender" />
    {/if}
  </aside>
</div>
</Huelle>

<!-- ─── Event editor dialog (HB-DIALOG) ─── -->
{#if editorOpen}
  <div
    class="dialog-schicht"
    role="dialog"
    aria-modal="true"
    aria-labelledby="cal-editor-title"
    tabindex="-1"
    onclick={(e) => { if ((e.target as HTMLElement).classList.contains("dialog-schicht")) editorOpen = false; }}
    onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); editorOpen = false; } }}
  >
    <div class="karte dialog-karte" data-breite="normal">
      <div class="dialog-kopf">
        <h2 id="cal-editor-title">{editingId === null ? $t("calendar.createEvent") : $t("calendar.editEvent")}</h2>
        <button type="button" class="dialog-zu" onclick={() => editorOpen = false} aria-label={$t("calendar.close")} title={$t("calendar.close")}><Symbol name="schliessen" size={20} /></button>
      </div>

      <div class="dialog-koerper">
        <div class="feld">
          <label for="cal-ev-summary">{$t("calendar.titleLabel")}</label>
          <input id="cal-ev-summary" type="text" bind:this={summaryInput} bind:value={form.summary} placeholder={$t("calendar.phTitle")} />
        </div>

        <div class="cal-nl">
          <input
            type="text"
            bind:value={nlText}
            class="input"
            placeholder={$t("calendar.phNl")}
            aria-label={$t("calendar.phNl")}
            onkeydown={(e) => { if (e.key === "Enter") applyTimeExtraction(); }}
          />
          <button
            type="button"
            class="btn btn-sekundaer"
            disabled={nlBusy || !nlText.trim()}
            onclick={applyTimeExtraction}
          ><Symbol name="ai" size={16} /> {nlBusy ? "…" : $t("calendar.extractTime")}</button>
        </div>

        <div class="feld-paar">
          <div class="feld">
            <label for="cal-ev-start">{$t("calendar.start")}</label>
            <input id="cal-ev-start" type={form.all_day ? "date" : "datetime-local"} bind:value={form.start} />
          </div>
          <div class="feld">
            <label for="cal-ev-end">{$t("calendar.end")}</label>
            <input id="cal-ev-end" type={form.all_day ? "date" : "datetime-local"} bind:value={form.end} />
          </div>
        </div>

        {#if conflicts.length > 0}
          <!-- HB-ZUSTAND "achtung" carries the colour; the body is Relay's own. -->
          <div class="hinweis cal-conflict" data-art="achtung">
            <Symbol name="achtung" size={16} />
            <div class="cal-conflict-body">
              <div class="cal-conflict-head">
                <span>{$t("calendar.conflict", { n: conflicts.length, unit: conflicts.length === 1 ? $t("calendar.conflictUnit") : $t("calendar.conflictUnitPlural") })}</span>
                <button type="button" class="btn btn-sekundaer btn-klein" disabled={conflictBusy} onclick={loadAlternatives}>
                  <Symbol name="ai" size={16} /> {conflictBusy ? "…" : $t("calendar.aiAlternatives")}
                </button>
              </div>
              <ul class="cal-conflict-list">
                {#each conflicts as c (c.id)}
                  <li>{c.summary ?? $t("calendar.untitled")} · {fmtInvWhen(c.start)}</li>
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
                <p class="cal-conflict-none">{$t("calendar.noAlternatives")}</p>
              {/if}
            </div>
          </div>
        {/if}

        <div class="btn-reihe cal-ai-row">
          <button type="button" class="btn btn-sekundaer" disabled={prepBusy || !form.start} onclick={loadMeetingPrep}>
            <Symbol name="ai" size={16} /> {prepBusy ? "…" : $t("calendar.meetingPrep")}
          </button>
          <button type="button" class="btn btn-sekundaer" disabled={smartBusy} onclick={loadSmartSchedule}>
            <Symbol name="ai" size={16} /> {smartBusy ? "…" : $t("calendar.smartScheduling")}
          </button>
        </div>

        {#if showSmart && smartSlots.length > 0}
          <div class="cal-slots">
            {#each smartSlots as slot (slot.start)}
              <button type="button" class="cal-slot" onclick={() => applySmartSlot(slot)}>
                <span class="cal-slot-when">{fmtInvWhen(slot.start)} – {fmtInvWhen(slot.end)}</span>
                {#if slot.reason}<span class="cal-slot-reason">{slot.reason}</span>{/if}
              </button>
            {/each}
          </div>
        {/if}

        {#if showPrep && prepResult}
          <div class="karte cal-prep">
            <div class="cal-prep-title">{$t("calendar.meetingPrep")}</div>
            {#if prepResult.attendees.length > 0}
              <div class="cal-prep-section"><strong>{$t("calendar.attendees")}</strong><ul>{#each prepResult.attendees as a (a)}<li>{a}</li>{/each}</ul></div>
            {/if}
            {#if prepResult.agenda.length > 0}
              <div class="cal-prep-section"><strong>{$t("calendar.agenda")}</strong><ul>{#each prepResult.agenda as a (a)}<li>{a}</li>{/each}</ul></div>
            {/if}
            {#if prepResult.prep_notes}
              <div class="cal-prep-section"><strong>{$t("calendar.preparation")}</strong><p>{prepResult.prep_notes}</p></div>
            {/if}
          </div>
        {/if}

        <label class="cal-check">
          <input type="checkbox" bind:checked={form.all_day} />
          <span>{$t("calendar.allDay")}</span>
        </label>

        <div class="feld">
          <label for="cal-ev-location">{$t("calendar.location")}</label>
          <input id="cal-ev-location" type="text" bind:value={form.location} placeholder={$t("calendar.phLocation")} />
        </div>

        <!-- Recipient chips are a composite input and stay Relay's own: inside
             .feld, AM-FELD would restyle its inner text input. -->
        <div class="cal-attendees-field" role="group" aria-labelledby="cal-ev-attendees">
          <span id="cal-ev-attendees" class="cal-attendees-label">{$t("calendar.attendees")}</span>
          <RecipientInput bind:value={form.participants} accountId={undefined} />
        </div>

        <div class="feld">
          <label for="cal-ev-description">{$t("calendar.description")}</label>
          <textarea id="cal-ev-description" bind:value={form.description} rows="3"></textarea>
        </div>
      </div>

      <!-- Footer as in HB-DIALOG: "Speichern" first, "Abbrechen" after it,
           and the destructive step pushed away to the far end. -->
      <div class="dialog-fuss">
        <button type="button" class="btn btn-primaer" onclick={saveEvent}>{$t("common.save")}</button>
        <button type="button" class="btn btn-sekundaer" onclick={() => editorOpen = false}>{$t("common.cancel")}</button>
        {#if editingId !== null}
          <button type="button" class="btn btn-gefahr cal-ev-delete" onclick={() => { const ev = events.find(x => x.id === editingId); if (ev) removeEvent(ev); editorOpen = false; }}>
            {$t("calendar.deleteEvent")}
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}

  <ContextMenu menu={evCtx} items={evCtxItems} onclose={() => (evCtx = null)} />

  <AssistantFab module="calendar" context={$t("calendar.viewContext", { view: periodLabel })} />

  <svelte:window onkeydown={handleKeydown} />

  {#if showDeleteConfirm && pendingDeleteEvent}
    <ConfirmationDialog
      open={showDeleteConfirm}
      title={$t("calendar.deleteEvent")}
      message={$t("calendar.deleteConfirm", { name: pendingDeleteEvent.summary ?? $t("calendar.untitled") })}
      confirmLabel={$t("calendar.delete")}
      cancelLabel={$t("common.cancel")}
      danger={true}
      onconfirm={confirmDeleteEvent}
      oncancel={cancelDeleteEvent}
    />
  {/if}

  {#if showInviteDialog}
    <ConfirmationDialog
      open={showInviteDialog}
      title={$t("calendar.sendInvites")}
      message={$t("calendar.inviteMessage", { count: pendingInviteAttendees.length, names: pendingInviteAttendees.map(a => a.email).join(", ") })}
      confirmLabel={$t("calendar.sendInvites")}
      cancelLabel={$t("common.skip")}
      danger={false}
      onconfirm={confirmSendInvites}
      oncancel={skipInvites}
    />
  {/if}

<style>
  /* ── Calendar page inside the shell [RL-KALENDER] ─────────────────────── */
  /* The shell (Huelle) draws header, areas and column; the page is the
     main pane with the detail pane beside it. */
  .cal-app {
    display: flex;
    min-height: 0;
    color: var(--am-text-primaer);
  }

  /* ── Narrow layout (mobile ≤768px) [RL-KALENDER] ──────────────────────── */
  /* Detail pane: hidden by default, full-screen overlay when an event is selected. */
  @media (max-width: 768px) {
    .cal-detail { display: none; }
    .cal-app.detail-open .cal-detail {
      display: block;
      position: fixed;
      inset: 0;
      width: 100%;
      min-width: 0;
      z-index: 60;
      border-left: none;
    }
  }

  /* ── Column: mini month, calendars, invitations, reminders [RL-KALENDER] ── */
  /* Group titles like .huelle-nav-titel (small, uppercase, muted); the
     shell's class is hidden on phones, where the column is a sheet. */
  .cal-spalte-titel {
    padding: var(--am-raum-3) var(--am-raum-3) var(--am-raum-1);
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--am-text-gedaempft);
  }
  .cal-cal-list { padding: 0 8px 8px; }
  .cal-upcoming { border-top: 1px solid var(--am-rand); padding: 10px 8px; }
  .cal-upcoming-head { padding-top: 0; }
  .cal-upcoming-item {
    display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
    background: none; border: none; color: var(--am-text-primaer); padding: 7px 8px;
    border-radius: var(--am-radius-mittel); cursor: pointer;
  }
  .cal-upcoming-item:hover { background: var(--am-flaeche-2); }
  .cal-upcoming-bell { color: var(--am-gold-500); flex-shrink: 0; }
  .cal-upcoming-info { display: flex; flex-direction: column; gap: 1px; overflow: hidden; }
  .cal-upcoming-title { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-upcoming-when { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .cal-invitations { border-top: 1px solid var(--am-rand); padding: 10px 8px; }
  .cal-invitations-head {
    display: flex; align-items: center; justify-content: space-between;
    font-size: var(--fs-xs); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--am-text-gedaempft); padding: 0 6px 8px;
  }
  .cal-invitations-badge {
    background: var(--am-gold-500); color: var(--am-blau-900);
    border-radius: 999px; font-size: var(--fs-xs); font-weight: 700; padding: 1px 7px;
  }
  /* The answer buttons have the full target size now, so they wrap under
     the invitation text instead of squeezing it. */
  .cal-inv-item {
    display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px;
    border-radius: var(--am-radius-mittel);
  }
  .cal-inv-item:hover { background: var(--am-flaeche-2); }
  .cal-inv-info { display: flex; flex-direction: column; gap: 1px; overflow: hidden; flex: 1 1 100%; min-width: 0; }
  .cal-inv-title { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-inv-when { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .cal-inv-organizer {
    font-size: var(--fs-xs); color: var(--am-text-gedaempft);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .cal-inv-actions { display: flex; gap: 4px; flex-wrap: wrap; }
  .cal-inv-draftbox { padding: 0 8px 8px; margin-bottom: 0; }
  .cal-empty {
    padding: 20px 12px;
    text-align: center;
    color: var(--am-text-gedaempft);
    font-size: var(--fs-sm);
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
    min-height: var(--am-ziel-zeiger);
    padding: 0 12px;
    color: var(--am-text-primaer);
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    font-size: 0.875rem;
  }
  .cal-cal-item:hover { background: var(--am-flaeche-2); }
  .cal-cal-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .cal-cal-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Calendar list checkboxes: AM-HAKEN draws them; a hidden calendar fades. */
  .cal-cal-item input[type="checkbox"] { margin: 0; }
  .cal-cal-item:has(input[type="checkbox"]:not(:checked)) { opacity: 0.5; }

  .cal-file-input { display: none; }

  /* ── Mini month (left pane) [RL-KALENDER] ─────────────────────────────── */
  .cal-mini { padding: 12px 14px; border-bottom: 1px solid var(--am-rand); }
  .cal-mini-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
  .cal-mini-label { font-size: var(--fs-sm); font-weight: 600; }
  .cal-mini-grid { display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 1px; }
  .cal-mini-day {
    background: none; border: none; color: var(--am-text-primaer);
    font-size: var(--fs-xs); padding: 4px 0; cursor: pointer; border-radius: var(--am-radius-klein);
    font-variant-numeric: tabular-nums;
  }
  .cal-mini-day:hover { background: var(--am-flaeche-2); }
  .cal-mini-day.other { color: var(--am-text-gedaempft); }
  .cal-mini-day.today { font-weight: 700; color: var(--am-handlung-ruhend); }
  .cal-mini-day.sel { background: var(--am-handlung-ruhend); color: var(--am-handlung-text); font-weight: 600; }

  /* ── Main pane and page head [RL-KALENDER] ────────────────────────────── */
  .cal-main { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden; }
  /* Many actions next to a long period ("Dienstag, 6. Oktober 2026"):
     the head may wrap instead of running off the side (layout only). */
  .cal-kopf { flex-wrap: wrap; flex-shrink: 0; }
  .cal-kopf h1 { white-space: nowrap; }
  .cal-kopf > .btn-reihe { gap: var(--am-raum-2); }
  .cal-kopf-nav { display: flex; align-items: center; gap: var(--am-raum-1); }
  @media (max-width: 40rem) {
    .cal-viewtoggle .cal-vt { padding: 4px 8px; }
    /* Phone head: title with the navigation in one line, the views with
       "Mehr" and "+" in the second (Kai, 7.10.2026). */
    .cal-kopf > .seitenkopf-zeile { width: 100%; justify-content: space-between; }
    .cal-kopf > .btn-reihe { width: 100%; justify-content: space-between; }
    .cal-kopf-rechts { display: flex; gap: var(--am-raum-2); }
  }

  /* Error line: HB-ZUSTAND draws it, only its place is set here. */
  .cal-alert { margin: 12px 20px 0; }

  /* View switcher Monat/Woche/Tag — Relay's own until HB-SEGMENT. */
  .cal-viewtoggle {
    display: inline-flex; background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand); border-radius: var(--am-radius-mittel); padding: 2px;
  }
  .cal-vt {
    background: none; border: none; color: var(--am-text-gedaempft);
    font-size: var(--fs-sm); padding: 5px 12px; cursor: pointer; border-radius: var(--am-radius-klein);
  }
  .cal-vt:hover { color: var(--am-text-primaer); }
  .cal-vt.active { background: var(--am-seite); color: var(--am-text-primaer); font-weight: 600; }

  /* ── Month grid [RL-KALENDER] ─────────────────────────────────────────── */
  .cal-monat { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .cal-grid {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    /* The weekday row is its own size; only the weeks share the height
       (it took a week's 96 px before). */
    grid-template-rows: auto;
    grid-auto-rows: minmax(96px, 1fr);
    overflow-y: auto;
  }

  /* ── Phone month [RL-KALENDER] (Kai, 7.10.2026) ────────────────────────
     Compact cells with dots, the chosen day's list below; the whole month
     fits, and the list leaves room for the assistant's shield. */
  .cal-monat.handy { overflow-y: auto; }
  .cal-monat.handy .cal-grid { flex: none; grid-auto-rows: 52px; overflow: visible; }
  .cal-monat.handy .cal-grid-head-cell { padding: 6px 0; text-align: center; border-right: none; }
  .cal-monat.handy .cal-cell { align-items: center; padding: 4px 0; gap: 4px; border-right: none; }
  .cal-monat.handy .cal-cell:hover { background: none; }
  .cal-cell.gewaehlt .cal-cell-num { box-shadow: inset 0 0 0 2px var(--am-handlung-ruhend); border-radius: 50%; }
  .cal-cell.gewaehlt.is-today .cal-cell-num { box-shadow: none; }
  .cal-punkte { display: flex; gap: 3px; min-height: 6px; }
  .cal-punkt { width: 6px; height: 6px; border-radius: 50%; }
  .cal-tagesliste { padding: var(--am-raum-4) var(--am-raum-4) 96px; display: flex; flex-direction: column; gap: var(--am-raum-1); }
  .cal-tagesliste h2 { font-size: 1rem; font-weight: 600; margin: 0 0 var(--am-raum-2); }
  .cal-tagesliste-leer { margin: 0; color: var(--am-text-gedaempft); font-size: 0.875rem; }
  .cal-tag-termin {
    display: flex; align-items: center; gap: var(--am-raum-3);
    min-height: 44px; padding: 0 var(--am-raum-3);
    border: none; border-radius: var(--am-radius-mittel);
    background: var(--am-flaeche-1); color: var(--am-text-primaer);
    text-align: left; cursor: pointer; font-size: 0.9375rem;
  }
  .cal-tag-termin.cancelled { opacity: 0.5; text-decoration: line-through; }
  .cal-tag-farbe { width: 4px; align-self: stretch; margin: 10px 0; border-radius: 2px; flex-shrink: 0; }
  .cal-tag-zeit { color: var(--am-text-gedaempft); font-variant-numeric: tabular-nums; flex-shrink: 0; min-width: 3.5rem; }
  .cal-tag-titel { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-grid-head {
    display: contents;
  }
  .cal-grid-head-cell {
    padding: 8px 10px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--am-text-gedaempft);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--am-rand);
    border-right: 1px solid var(--am-rand);
    background: var(--am-flaeche-1);
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .cal-cell {
    border-right: 1px solid var(--am-rand);
    border-bottom: 1px solid var(--am-rand);
    padding: 4px 6px;
    cursor: pointer;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cal-cell:hover { background: var(--am-flaeche-2); }
  /* Other months: muted numbers, not a faded cell — opacity broke the
     contrast (axe, Etappe 7). */
  .cal-cell.other-month .cal-cell-num { color: var(--am-text-gedaempft); }
  .cal-cell.is-today .cal-cell-num {
    background: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
    border-radius: 50%;
  }
  .cal-cell-num {
    font-size: var(--fs-xs);
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
    min-width: 0;
    padding: 2px 6px;
    border: none;
    background: var(--am-flaeche-1);
    color: var(--am-text-primaer);
    border-radius: var(--am-radius-klein);
    cursor: pointer;
    font-size: var(--fs-xs);
    text-align: left;
    overflow: hidden;
  }
  .cal-event:hover { background: var(--am-flaeche-2); }
  .cal-event.cancelled { opacity: 0.5; text-decoration: line-through; }
  .cal-event-time { color: var(--am-text-gedaempft); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .cal-event-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* ── Week view [RL-KALENDER] ──────────────────────────────────────────── */
  .cal-week { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
  .cal-week-head { display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); border-bottom: 1px solid var(--am-rand); }
  .cal-week-head-cell {
    display: flex; flex-direction: column; align-items: center; gap: 2px;
    padding: 8px 4px; border-right: 1px solid var(--am-rand);
  }
  .cal-week-dow { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.04em; color: var(--am-text-gedaempft); font-weight: 600; }
  .cal-week-num { font-size: var(--fs-lg); font-weight: 600; width: 30px; height: 30px; display: flex; align-items: center; justify-content: center; border-radius: 50%; }
  .cal-week-head-cell.is-today .cal-week-num { background: var(--am-handlung-ruhend); color: var(--am-handlung-text); }
  .cal-week-body { flex: 1; display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); overflow-y: auto; }
  .cal-week-col { border-right: 1px solid var(--am-rand); padding: 6px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; min-height: 120px; }
  .cal-week-col:last-child { border-right: none; }
  .cal-week-col:hover { background: var(--am-flaeche-2); }
  .cal-week-col.is-today { background: color-mix(in srgb, var(--am-handlung-ruhend) 5%, transparent); }
  .cal-event-block { flex-direction: column; align-items: flex-start; gap: 2px; padding: 6px 8px; border-left: 3px solid var(--am-handlung-ruhend); background: var(--am-flaeche-1); max-width: 100%; }
  /* Phone: seven columns of about 50 px — the title wraps instead of
     running into the next day (Kai, 10.10.2026). */
  @media (max-width: 40rem) {
    .cal-event-block { padding: 4px; }
    .cal-event-block .cal-event-title {
      white-space: normal;
      overflow-wrap: anywhere;
      display: -webkit-box;
      -webkit-line-clamp: 3;
      line-clamp: 3;
      -webkit-box-orient: vertical;
      font-size: 0.6875rem;
    }
    .cal-event-block .cal-event-time { font-size: 0.6875rem; }
  }

  /* ── Day view and digest [RL-KALENDER] ────────────────────────────────── */
  .cal-dayview { flex: 1; overflow-y: auto; padding: 16px 20px; display: flex; flex-direction: column; gap: 8px; }
  .cal-day-item {
    display: flex; align-items: center; gap: 12px; text-align: left;
    padding: 12px 14px; border: 1px solid var(--am-rand); border-radius: var(--am-radius-mittel);
    background: var(--am-seite); cursor: pointer;
  }
  .cal-day-item:hover { background: var(--am-flaeche-2); }
  .cal-day-item.sel { border-color: var(--am-handlung-ruhend); box-shadow: 0 0 0 2px var(--am-fokus-ring); }
  .cal-day-item.cancelled { opacity: 0.5; }
  .cal-day-time { font-size: var(--fs-sm); font-weight: 600; color: var(--am-text-gedaempft); min-width: 64px; font-variant-numeric: tabular-nums; }
  .cal-day-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .cal-day-info { display: flex; flex-direction: column; gap: 2px; overflow: hidden; }
  .cal-day-title { font-size: var(--fs-base); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-day-loc { font-size: var(--fs-xs); color: var(--am-text-gedaempft); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-digest-bar { display: flex; gap: 8px; }
  /* The digest is an AM-KARTE; only its text is set here. */
  .cal-digest-text { margin: 0 0 10px; font-size: 0.9rem; color: var(--am-text-primaer); }
  .cal-digest-section { margin-bottom: 10px; font-size: 0.85rem; color: var(--am-text-primaer); }
  .cal-digest-section ul { margin: 4px 0 0; padding-left: 18px; }

  /* ── Detail pane (right) [RL-KALENDER] ────────────────────────────────── */
  .cal-detail {
    width: 300px; min-width: 300px; background: var(--am-flaeche-1);
    border-left: 1px solid var(--am-rand); overflow-y: auto;
  }
  .cal-detail-inner { padding: 18px; display: flex; flex-direction: column; gap: 14px; }
  .cal-detail-top { display: flex; align-items: center; justify-content: space-between; }
  .cal-detail-cal { display: inline-flex; align-items: center; gap: 6px; color: var(--am-text-primaer); font-size: var(--fs-xs); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .cal-detail-title { margin: 0; font-size: var(--fs-lg); font-weight: 600; line-height: 1.3; }
  .cal-detail-rows { display: flex; flex-direction: column; gap: 10px; }
  .cal-detail-row { display: flex; align-items: flex-start; gap: 10px; font-size: var(--fs-sm); color: var(--am-text-primaer); }
  .cal-detail-ico { color: var(--am-text-gedaempft); width: 16px; flex-shrink: 0; text-align: center; }
  .cal-detail-desc { margin: 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--am-text-gedaempft); white-space: pre-wrap; word-break: break-word; }
  .cal-detail-actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 6px; }
  .cal-attendees { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .cal-attendee { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: var(--fs-sm); }
  .cal-attendee-name { color: var(--am-text-primaer); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cal-attendee-status { font-size: var(--fs-xs); flex-shrink: 0; padding: 1px 8px; border-radius: 10px; }
  .cal-attendee-needsaction .cal-attendee-status { color: var(--am-text-gedaempft); background: var(--am-flaeche-1); }
  /* RSVP states (CI R7): red is for "needs action" only — a decline is
     information, so it stays neutral; the word always carries the state. */
  .cal-attendee-accepted .cal-attendee-status { color: var(--am-erfolg); background: var(--am-erfolg-flaeche); }
  .cal-attendee-declined .cal-attendee-status { color: var(--am-text-gedaempft); background: var(--am-flaeche-2); }
  .cal-attendee-tentative .cal-attendee-status { color: var(--am-achtung); background: var(--am-achtung-flaeche); }

  /* ── Event editor dialog [RL-KALENDER] ────────────────────────────────── */
  /* Frame, fields and buttons come from HB-DIALOG, AM-FELD and AM-KNOPF;
     what remains is the spacing of Relay's own parts inside the body. */
  .cal-nl { display: flex; gap: 8px; align-items: center; margin-bottom: var(--am-raum-4); }
  .cal-nl .input { flex: 1; min-width: 0; }
  .cal-conflict { margin-bottom: var(--am-raum-4); }
  .cal-conflict-body { flex: 1; min-width: 0; }
  .cal-conflict-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .cal-conflict-list { margin: 8px 0 0; padding-left: 18px; font-size: var(--fs-xs); }
  .cal-conflict-none { font-size: var(--fs-xs); margin: 8px 0 0; }
  .cal-ai-row { gap: 8px; margin-bottom: var(--am-raum-4); }
  .cal-slots { display: flex; flex-direction: column; gap: 6px; margin: 10px 0 var(--am-raum-4); }
  /* Suggested slots are a choice list, not buttons in the AM-KNOPF sense. */
  .cal-slot {
    display: flex; flex-direction: column; gap: 2px; text-align: left;
    border: 1px solid var(--am-rand); border-radius: var(--am-radius-mittel);
    background: var(--am-seite); padding: 8px 10px; cursor: pointer;
  }
  .cal-slot:hover { background: var(--am-flaeche-2); border-color: var(--am-gold-500); }
  .cal-slot-when { font-size: var(--fs-sm); color: var(--am-text-primaer); }
  .cal-slot-reason { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .cal-prep { margin-bottom: var(--am-raum-4); }
  .cal-prep-title {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-handlung-ruhend);
    margin-bottom: 8px;
  }
  .cal-prep-section { margin-bottom: 10px; font-size: 0.85rem; color: var(--am-text-primaer); }
  .cal-prep-section ul { margin: 4px 0 0; padding-left: 18px; }
  .cal-prep-section p { margin: 4px 0 0; }
  .cal-check { display: flex; align-items: center; gap: 8px; margin-bottom: var(--am-raum-4); font-size: var(--fs-sm); }
  /* Label of the recipient-chip field, set like an AM-FELD label. */
  .cal-attendees-field { margin-bottom: var(--am-raum-4); }
  .cal-attendees-label {
    display: block;
    font-size: 0.875rem;
    font-weight: 600;
    margin-bottom: var(--am-raum-2);
  }
  /* The destructive step stands apart at the far end of the footer. */
  .cal-ev-delete { margin-left: auto; }
</style>
