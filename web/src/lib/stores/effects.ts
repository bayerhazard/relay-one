import { writable } from "svelte/store";

// Declarative assistant effects (Concept §5.5). The agent loop emits a list of
// effects per response; the effect router (in +layout.svelte) consumes this
// queue sequentially. This is the Phase-A queue that replaces the v1
// single-shot `assistantAction` once the v2 agent becomes default (§6.12).
//
// Effects are declarative and whitelist-checked server-side — the model can
// never set arbitrary paths or URLs.

export type Effect =
  | { kind: "navigate"; module: "mail" | "calendar" | "contacts" | "tasks" | "settings" }
  | { kind: "calendar.set_view"; view: "day" | "week" | "month"; date?: string }
  | { kind: "mail.open"; uid: number; folder?: string; account_id?: number }
  | { kind: "contacts.open"; uid: string }
  | { kind: "tasks.open"; uid: string }
  | { kind: "calendar.open_event"; id: string }
  | { kind: "compose.open"; to: string; subject: string; body: string }
  | { kind: "highlight"; art: "mail" | "contact" | "task" | "event"; id: string };

function createEffectQueue() {
  let queue: Effect[] = [];
  const { subscribe, set } = writable<Effect[]>([]);

  return {
    subscribe,
    /** Append one effect (or many) to the back of the queue. */
    push: (e: Effect) => {
      queue = [...queue, e];
      set(queue);
    },
    pushMany: (es: Effect[]) => {
      queue = [...queue, ...es];
      set(queue);
    },
    /** Consume and return the front effect, or `null` when empty. */
    shift: (): Effect | null => {
      if (queue.length === 0) {
        set([]);
        return null;
      }
      const [head, ...rest] = queue;
      queue = rest;
      set(queue);
      return head;
    },
    clear: () => {
      queue = [];
      set([]);
    },
  };
}

export const effects = createEffectQueue();

// The side-effect surface the router may touch. Kept as an interface so the
// dispatch is pure and unit-testable without a live app (Concept §12.5).
export interface EffectContext {
  goto: (url: string) => void;
  setView: (view: "day" | "week" | "month", date?: Date) => void;
  setMail: (sel: { uid: number; folder: string; account: number } | null, highlight?: boolean) => void;
  setContact: (id: string | null, highlight?: boolean) => void;
  setTask: (id: string | null, highlight?: boolean) => void;
  setEvent: (id: string | null, highlight?: boolean) => void;
  openCompose: (a: { to: string; subject: string; body: string }) => void;
  selectedMail?: { uid: number; folder: string; account: number } | null;
}

/**
 * Dispatch a single declarative effect (Concept §5.5). Effects are
 * whitelist-checked server-side; this is the last line of defence. An unknown
 * or malformed kind (e.g. after a version skew) is ignored safely — it must
 * never throw or navigate to an unlisted route.
 */
export function applyEffect(e: Effect, ctx: EffectContext): void {
  switch (e.kind) {
    case "navigate":
      ctx.goto(e.module === "mail" ? "/" : `/${e.module}`);
      break;
    case "calendar.set_view": {
      const date = e.date ? new Date(e.date) : undefined;
      ctx.setView(e.view, date);
      ctx.goto("/calendar");
      break;
    }
    case "mail.open":
      ctx.setMail({ uid: e.uid, folder: e.folder ?? "", account: e.account_id ?? 0 }, true);
      ctx.goto("/");
      break;
    case "contacts.open":
      ctx.setContact(e.uid, true);
      ctx.goto("/contacts");
      break;
    case "tasks.open":
      ctx.setTask(e.uid, true);
      ctx.goto("/tasks");
      break;
    case "calendar.open_event":
      ctx.setEvent(e.id, true);
      ctx.goto("/calendar");
      break;
    case "compose.open":
      ctx.openCompose({ to: e.to, subject: e.subject, body: e.body });
      ctx.goto("/");
      break;
    case "highlight":
      // Transient gold pulse (§10.3): single-id arts set directly; mail re-
      // highlights only an already-selected mail.
      if (e.art === "contact") ctx.setContact(e.id, true);
      else if (e.art === "task") ctx.setTask(e.id, true);
      else if (e.art === "event") ctx.setEvent(e.id, true);
      else if (e.art === "mail" && ctx.selectedMail) ctx.setMail(ctx.selectedMail, true);
      break;
    default:
      // Unknown effect kind: ignore (defensive; the payload is typed above).
      break;
  }
}
