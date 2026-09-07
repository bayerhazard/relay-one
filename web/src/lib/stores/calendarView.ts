import { writable } from "svelte/store";

// Calendar view state (Concept §10.2). The calendar page derives its local
// `viewDate`/`viewMode` from this store, and the effect router writes to it
// (e.g. a `calendar.set_view` effect). Two-way: the page writes back when the
// user navigates, so an assistant effect and manual navigation stay coherent.

export type CalendarViewMode = "month" | "week" | "day";

export interface CalendarViewState {
  viewDate: Date;
  viewMode: CalendarViewMode;
}

function defaultState(): CalendarViewState {
  return { viewDate: new Date(), viewMode: "month" };
}

function createCalendarViewStore() {
  const { subscribe, set, update } = writable<CalendarViewState>(defaultState());
  return {
    subscribe,
    set: (s: CalendarViewState) => set(s),
    setView: (mode: CalendarViewMode, date?: Date) =>
      update((s) => ({ viewMode: mode, viewDate: date ?? s.viewDate })),
    setDate: (date: Date) => update((s) => ({ ...s, viewDate: date })),
    reset: () => set(defaultState()),
  };
}

export const calendarView = createCalendarViewStore();
