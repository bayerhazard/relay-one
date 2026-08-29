import type { FollowupAction } from "$lib/services/tauri";

// Persistent memory of which AI follow-up actions the user already executed,
// keyed by message UID. The backend `id` (fu-N) is position-based and not
// stable across re-generation, so we derive a content-based fingerprint from
// the most stable field of each action. Stored in localStorage so it survives
// app restarts (per user profile) — the same store the app uses for settings.
const STORAGE_KEY = "relay_followups_done";

type DoneMap = Record<string, string[]>;

function norm(s: string | null | undefined): string {
  return (s ?? "").trim().toLowerCase().replace(/\s+/g, " ");
}

/** Stable, content-based identifier for a follow-up action. */
export function followupFingerprint(a: FollowupAction): string {
  switch (a.kind) {
    case "task":
      return `task|${norm(a.task?.summary)}`;
    case "event":
      return `event|${norm(a.event?.summary)}`;
    case "email":
      return `email|${norm(a.email?.subject)}`;
    default:
      return `${a.kind}|${a.id}`;
  }
}

function loadDone(): DoneMap {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as DoneMap;
    }
  } catch {
    /* ignore — localStorage unavailable or corrupt */
  }
  return {};
}

function saveDone(map: DoneMap): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch (e) {
    console.warn("Failed to persist followup memory", e);
  }
}

/** Set of fingerprints already executed for a given message UID. */
export function getDoneFingerprints(uid: number): Set<string> {
  return new Set(loadDone()[String(uid)] ?? []);
}

export function isFollowupDone(uid: number, a: FollowupAction): boolean {
  return getDoneFingerprints(uid).has(followupFingerprint(a));
}

export function markFollowupDone(uid: number, a: FollowupAction): void {
  const fp = followupFingerprint(a);
  const map = loadDone();
  const key = String(uid);
  const list = map[key] ?? [];
  if (list.includes(fp)) return;
  list.push(fp);
  map[key] = list;
  saveDone(map);
}

/** Clear all remembered actions. Returns the number of entries removed. */
export function clearFollowupMemory(): number {
  const map = loadDone();
  let count = 0;
  for (const key of Object.keys(map)) count += map[key]?.length ?? 0;
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
  return count;
}
