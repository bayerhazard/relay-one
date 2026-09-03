import { idbGetAll, idbPut, idbDelete } from "./db";

export interface QueuedDraft {
  id?: number;
  accountId: number;
  to: string[];
  cc?: string[];
  bcc?: string[];
  subject: string;
  bodyText: string;
  bodyHtml?: string;
  inReplyTo?: string;
  references?: string;
  recipientEmail?: string;
  attachments?: { filename: string; content: string; content_type: string; size: number }[];
  createdAt: number;
}

export async function queueDraft(draft: Omit<QueuedDraft, "id" | "createdAt">): Promise<void> {
  try {
    await idbPut("outbox", { ...draft, createdAt: Date.now() });
  } catch {
    // IndexedDB unavailable
  }
}

export async function getQueuedDrafts(): Promise<QueuedDraft[]> {
  try {
    return await idbGetAll<QueuedDraft>("outbox");
  } catch {
    return [];
  }
}

export async function removeQueuedDraft(id: number): Promise<void> {
  try {
    await idbDelete("outbox", id);
  } catch {
    // ignore
  }
}

export async function clearOutbox(): Promise<void> {
  const drafts = await getQueuedDrafts();
  for (const d of drafts) {
    if (d.id != null) await removeQueuedDraft(d.id);
  }
}
