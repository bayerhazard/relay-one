import { writable } from "svelte/store";

// Data-invalidation counter (Concept §10.5). Bumped after an assistant plan is
// executed or undone so the module pages reload the affected data. Relay is a
// single-user local box, so a global bump (everything except accounts) is the
// intended fallback; pages that own the changed entity reload on the bump.
export const dataVersion = writable(0);

export function bumpDataVersion(): void {
  dataVersion.update((n) => n + 1);
}
