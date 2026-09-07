import { writable } from "svelte/store";
import type { AgentPlan } from "$lib/services/tauri";

// One-shot command to open the Assistant Drawer with a pre-built plan card
// (Phase C, Concept §9.4): a mail footer chip builds a plan
// (origin=mail_followup) and hands it to the Drawer, which opens and shows the
// T1 card for confirmation. The Drawer consumes (clears) the command.
export interface AssistantCommand {
  plan: AgentPlan;
  /** Monotonic nonce so a repeated command always re-triggers the effect. */
  nonce: number;
}

function createAssistantCommandStore() {
  const { subscribe, set } = writable<AssistantCommand | null>(null);
  let n = 0;
  return {
    subscribe,
    /** Hand a plan to the Drawer (opens it and injects the card). */
    showPlan: (plan: AgentPlan) => set({ plan, nonce: ++n }),
    /** Clear the pending command (Drawer consumed it). */
    clear: () => set(null),
  };
}

export const assistantCommand = createAssistantCommandStore();
