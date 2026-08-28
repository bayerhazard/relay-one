<script lang="ts">
  // Phase 4.5 — Globaler AI-Assistent (Centerpiece).
  // Slide-in drawer mit Chat-UI und Action-Vorschau.
  import { t } from "$lib/i18n";
  import {
    askAssistant,
    createEvent,
    createTodo,
    getCalendars,
    type AssistantAction,
    type AssistantResult,
  } from "$lib/services/tauri";

  interface Props {
    open: boolean;
    context?: string;
    onclose: () => void;
  }

  let { open, context = "", onclose }: Props = $props();

  interface ChatMsg {
    role: "user" | "assistant";
    text: string;
    actions: AssistantAction[];
    error?: string;
  }

  let messages = $state<ChatMsg[]>([]);
  let input = $state("");
  let loading = $state(false);
  let inputEl = $state<HTMLTextAreaElement | null>(null);

  $effect(() => {
    if (open) {
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  function reset() {
    messages = [];
    input = "";
  }

  async function send() {
    const text = input.trim();
    if (!text || loading) return;
    input = "";
    messages = [...messages, { role: "user", text, actions: [] }];
    loading = true;
    try {
      const res: AssistantResult = await askAssistant(text, context);
      messages = [...messages, { role: "assistant", text: res.reply || "(keine Antwort)", actions: res.actions }];
    } catch (e) {
      messages = [...messages, { role: "assistant", text: "", actions: [], error: String(e) }];
    } finally {
      loading = false;
    }
  }

  async function runAction(action: AssistantAction): Promise<string> {
    const p = action.payload ?? {};
    try {
      switch (action.type) {
        case "event_create": {
          const cals = await getCalendars();
          const cal = cals[0];
          if (!cal) return "Kein Kalender gefunden.";
          const start = (p.start as string) ?? new Date().toISOString();
          await createEvent({
            calendar_id: cal.id,
            summary: (p.summary as string) ?? (p.title as string) ?? "Termin",
            start,
            end: (p.end as string) ?? undefined,
            description: (p.description as string) ?? undefined,
            attendees: Array.isArray(p.attendees)
              ? (p.attendees as string[]).map((email) => ({ email }))
              : undefined,
          });
          return "Termin angelegt.";
        }
        case "task_create":
          await createTodo({
            summary: (p.summary as string) ?? (p.title as string) ?? "Aufgabe",
            due: (p.due as string) ?? undefined,
          });
          return "Aufgabe angelegt.";
        case "find_mail":
          return `Suche: ${(p.query as string) ?? ""}`;
        default:
          return `Aktion „${action.type}" vorbereitet.`;
      }
    } catch (e) {
      return `Fehler: ${String(e)}`;
    }
  }

  async function handleAction(action: AssistantAction) {
    const result = await runAction(action);
    messages = [...messages, { role: "assistant", text: result, actions: [] }];
  }
</script>

{#if open}
  <div class="assistant-scrim" onclick={onclose}>
    <aside class="assistant-drawer" onclick={(e) => e.stopPropagation()}>
      <header class="assistant-header">
        <span class="assistant-title">Assistent</span>
        <button type="button" class="assistant-close" onclick={onclose} aria-label="Schließen">✕</button>
      </header>
      <div class="assistant-body">
        {#if messages.length === 0}
          <p class="assistant-hint">Frag mich nach Terminen, Aufgaben oder Mails.</p>
        {/if}
        {#each messages as m (m.text + m.actions.length)}
          {#if m.error}
            <div class="chat-msg assistant error">{m.error}</div>
          {:else}
            <div class="chat-msg {m.role}">
              <div class="chat-text">{m.text}</div>
              {#if m.actions.length > 0}
                <div class="chat-actions">
                  {#each m.actions as a (a.type + JSON.stringify(a.payload))}
                    <button type="button" class="chat-action" onclick={() => handleAction(a)}>
                      {a.type}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        {/each}
        {#if loading}
          <div class="chat-msg assistant"><span class="chat-typing">…</span></div>
        {/if}
      </div>
      <footer class="assistant-footer">
        <textarea
          bind:this={inputEl}
          bind:value={input}
          class="assistant-input"
          placeholder="z.B. Plan morgen 14 Uhr Kaffee mit Anna"
          rows={2}
          onkeydown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
        ></textarea>
        <button type="button" class="assistant-send" disabled={loading || !input.trim()} onclick={send}>Senden</button>
      </footer>
    </aside>
  </div>
{/if}

<style>
  .assistant-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    z-index: 1000;
  }
  .assistant-drawer {
    position: absolute;
    top: 0;
    right: 0;
    height: 100%;
    width: min(420px, 100%);
    background: var(--color-list);
    border-left: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
  }
  .assistant-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    border-bottom: 1px solid var(--color-border);
  }
  .assistant-title {
    font-weight: 600;
    color: var(--color-text);
  }
  .assistant-close {
    border: none;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: 1rem;
  }
  .assistant-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
  }
  .assistant-hint {
    color: var(--color-text-secondary);
    font-size: 0.85rem;
  }
  .assistant-footer {
    display: flex;
    gap: 8px;
    padding: 12px;
    border-top: 1px solid var(--color-border);
  }
  .assistant-input {
    flex: 1;
    resize: none;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    font: inherit;
    color: var(--color-text);
    background: var(--color-card);
  }
  .assistant-send {
    border: none;
    border-radius: var(--radius-s);
    background: var(--color-accent);
    color: #fff;
    padding: 8px 14px;
    cursor: pointer;
    font-weight: 500;
  }
  .assistant-send:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .chat-msg {
    max-width: 85%;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    margin-bottom: 10px;
    font-size: 0.9rem;
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .chat-msg.user {
    margin-left: auto;
    background: var(--color-accent);
    color: #fff;
  }
  .chat-msg.assistant {
    margin-right: auto;
    background: var(--color-card);
    border: 1px solid var(--color-border);
    color: var(--color-text);
  }
  .chat-msg.error {
    background: var(--color-active-wash);
    color: var(--color-danger);
    border: 1px solid var(--color-danger);
  }
  .chat-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
  .chat-action {
    font-size: 0.75rem;
    font-weight: 500;
    padding: 5px 10px;
    border: 1px solid var(--color-accent);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--color-accent);
    cursor: pointer;
  }
  .chat-action:hover {
    background: var(--color-accent);
    color: #fff;
  }
  .chat-typing {
    color: var(--color-text-secondary);
  }
</style>
