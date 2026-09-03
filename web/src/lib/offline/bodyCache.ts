import { idbGet, idbPut, idbGetAllByIndex, idbDelete } from "./db";

const MAX_BODIES = 200;

export interface CachedBody {
  key: string;
  accountId: number;
  folder: string;
  uid: number;
  body_text: string;
  body_html?: string;
  subject?: string;
  from?: string;
  to?: string;
  cc?: string;
  date?: string;
  flags?: string;
  accessedAt: number;
}

function makeKey(accountId: number, folder: string, uid: number): string {
  return `${accountId}:${folder}:${uid}`;
}

export async function cacheBody(accountId: number, folder: string, uid: number, data: {
  body_text: string;
  body_html?: string;
  subject?: string;
  from?: string;
  to?: string;
  cc?: string;
  date?: string;
  flags?: string;
}): Promise<void> {
  try {
    const entry: CachedBody = {
      key: makeKey(accountId, folder, uid),
      accountId,
      folder,
      uid,
      body_text: data.body_text,
      body_html: data.body_html,
      subject: data.subject,
      from: data.from,
      to: data.to,
      cc: data.cc,
      date: data.date,
      flags: data.flags,
      accessedAt: Date.now(),
    };
    await idbPut("bodies", entry);
    await evictIfNeeded(accountId);
  } catch {
    // IndexedDB unavailable (private mode) — silently skip
  }
}

export async function getCachedBody(accountId: number, folder: string, uid: number): Promise<CachedBody | null> {
  try {
    const key = makeKey(accountId, folder, uid);
    const entry = await idbGet<CachedBody>("bodies", key);
    if (entry) {
      entry.accessedAt = Date.now();
      await idbPut("bodies", entry);
    }
    return entry ?? null;
  } catch {
    return null;
  }
}

async function evictIfNeeded(accountId: number): Promise<void> {
  const bodies = await idbGetAllByIndex<CachedBody>("bodies", "account", accountId);
  if (bodies.length <= MAX_BODIES) return;
  bodies.sort((a, b) => a.accessedAt - b.accessedAt);
  const toRemove = bodies.slice(0, bodies.length - MAX_BODIES);
  for (const b of toRemove) {
    await idbDelete("bodies", b.key);
  }
}
