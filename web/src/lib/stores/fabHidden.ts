import { writable } from "svelte/store";

// M6 (Review 2026-09-14): the floating assistant button overlaps mobile
// drawers/sheets in the module pages. Each page pushes truth onto this flag
// while an overlay is open — AssistantFab hides itself accordingly.
export const fabHidden = writable(false);
