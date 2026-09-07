import { writable } from "svelte/store";

// Cross-module selection state (Concept §10.2). Each module page reads the
// relevant field on mount (and via $effect) to open/highlight a specific item
// that the assistant navigated to. `highlight` drives the 2 s gold outline
// (§10.3) — set true when a selection arrives from an assistant effect.

export interface MailSelection {
  uid: number;
  folder: string;
  account: number;
}

export interface SelectionState {
  mail: MailSelection | null;
  contact: string | null;
  task: string | null;
  event: string | null;
  /** True when the current selection came from an assistant effect (highlight). */
  highlight: boolean;
}

function defaultState(): SelectionState {
  return { mail: null, contact: null, task: null, event: null, highlight: false };
}

function createSelectionStore() {
  const { subscribe, set, update } = writable<SelectionState>(defaultState());
  return {
    subscribe,
    setMail: (sel: MailSelection | null, highlight = false) =>
      update((s) => ({ ...s, mail: sel, highlight: sel ? highlight : s.highlight })),
    setContact: (id: string | null, highlight = false) =>
      update((s) => ({ ...s, contact: id, highlight: id ? highlight : s.highlight })),
    setTask: (id: string | null, highlight = false) =>
      update((s) => ({ ...s, task: id, highlight: id ? highlight : s.highlight })),
    setEvent: (id: string | null, highlight = false) =>
      update((s) => ({ ...s, event: id, highlight: id ? highlight : s.highlight })),
    clearHighlight: () => update((s) => ({ ...s, highlight: false })),
    reset: () => set(defaultState()),
  };
}

export const selection = createSelectionStore();
