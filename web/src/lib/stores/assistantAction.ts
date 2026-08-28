import { writable } from "svelte/store";

// A pending action handed off from the AI assistant to a module page after
// navigation. The target page reads it on mount, performs the action, then
// clears it.
export type AssistantPendingAction =
  | { type: "open_compose"; to: string; subject: string; body: string }
  | {
      type: "open_event_editor";
      summary: string;
      start?: string;
      end?: string;
      description?: string;
      attendees?: string[];
    }
  | { type: "search"; query: string }
  | null;

function createAssistantActionStore() {
  const { subscribe, set } = writable<AssistantPendingAction>(null);
  return {
    subscribe,
    set: (action: AssistantPendingAction) => set(action),
    clear: () => set(null),
  };
}

export const assistantAction = createAssistantActionStore();
