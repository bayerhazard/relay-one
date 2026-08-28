<script lang="ts">
  // Phase 4.5 — Globaler AI-Assistent (Centerpiece).
  // Slide-in drawer mit Chat-UI und Action-Vorschau.
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { assistantAction } from "$lib/stores/assistantAction";
  import { blobToWavBase64 } from "$lib/utils/wav";
  import {
    askAssistant,
    createEvent,
    createTodo,
    getCalendars,
    getVoiceSettings,
    voiceTranscribe,
    type AssistantAction,
    type AssistantResult,
  } from "$lib/services/tauri";

  interface Props {
    open: boolean;
    module: "mail" | "calendar" | "contacts" | "tasks" | "settings";
    context?: string;
    onclose: () => void;
  }

  let { open, module, context = "", onclose }: Props = $props();

  // Module context string handed to the LLM so it knows which module the
  // user started the assistant from.
  const MODULE_LABELS: Record<string, string> = {
    mail: "Posteingang (E-Mail)",
    calendar: "Kalender",
    contacts: "Kontakte",
    tasks: "Aufgaben",
    settings: "Einstellungen",
  };
  const fullContext = context
    ? `Aktives Modul: ${MODULE_LABELS[module] ?? module}. ${context}`
    : `Aktives Modul: ${MODULE_LABELS[module] ?? module}`;

  interface ChatMsg {
    role: "user" | "assistant";
    text: string;
    actions: AssistantAction[];
    error?: string;
  }

  let messages = $state<ChatMsg[]>([]);
  let input = $state("");
  let loading = $state(false);
  let inputEl = $state<HTMLInputElement | null>(null);
  let popEl = $state<HTMLElement | null>(null);

  // ─── Voice input (dictation into the assistant) ───────────────────
  let voiceEnabled = $state(false);
  let isRecording = $state(false);
  let transcribing = $state(false);
  let voiceError = $state<string | null>(null);
  let mediaRecorder: MediaRecorder | null = null;
  let audioChunks: Blob[] = [];

  $effect(() => {
    if (!open) return;
    requestAnimationFrame(() => inputEl?.focus());
    getVoiceSettings().then((s) => {
      voiceEnabled = s?.enabled ?? false;
    });
    const onDocClick = (e: MouseEvent) => {
      const t = e.target;
      if (popEl && t instanceof Node && !popEl.contains(t)) onclose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onclose();
    };
    // Defer listener attachment past the opening click so the trigger
    // click doesn't immediately close the popover.
    const timer = setTimeout(() => {
      document.addEventListener("click", onDocClick);
      document.addEventListener("keydown", onKey);
    }, 0);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("click", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  });

  async function toggleVoiceInput() {
    if (isRecording) {
      stopRecording();
    } else {
      await startRecording();
    }
  }

  async function startRecording() {
    voiceError = null;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      mediaRecorder = new MediaRecorder(stream);
      audioChunks = [];

      mediaRecorder.ondataavailable = (e) => {
        if (e.data.size > 0) audioChunks.push(e.data);
      };

      mediaRecorder.onstop = async () => {
        stream.getTracks().forEach((track) => track.stop());
        if (audioChunks.length === 0) {
          voiceError = $t("assistant.noAudio");
          return;
        }
        const audioBlob = new Blob(audioChunks, { type: "audio/webm" });
        const base64 = await blobToWavBase64(audioBlob);
        transcribing = true;
        try {
          const transcript = await voiceTranscribe(base64);
          if (transcript.trim()) {
            input = transcript;
            await send();
          } else {
            voiceError = $t("assistant.noText");
          }
        } catch (e: unknown) {
          voiceError = e instanceof Error ? e.message : String(e);
        } finally {
          transcribing = false;
        }
      };

      mediaRecorder.start();
      isRecording = true;

      // Auto-stop after 2 seconds of silence (RMS-based detection).
      let lastVoiceAt = Date.now();
      const audioContext = new AudioContext();
      const source = audioContext.createMediaStreamSource(stream);
      const analyser = audioContext.createAnalyser();
      analyser.fftSize = 1024;
      analyser.smoothingTimeConstant = 0.4;
      source.connect(analyser);
      const rmsLevel = (): number => {
        const buf = new Float32Array(analyser.fftSize);
        analyser.getFloatTimeDomainData(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
        return Math.sqrt(sum / buf.length);
      };
      const checkSilence = () => {
        if (!isRecording) return;
        const rms = rmsLevel();
        if (rms >= 0.01) {
          lastVoiceAt = Date.now();
        } else if (Date.now() - lastVoiceAt >= 2000) {
          void audioContext.close().catch(() => {});
          stopRecording();
          return;
        }
        requestAnimationFrame(checkSilence);
      };
      checkSilence();
    } catch (e: unknown) {
      voiceError = $t("assistant.micFailed");
      isRecording = false;
    }
  }

  function stopRecording() {
    if (mediaRecorder && mediaRecorder.state !== "inactive") {
      mediaRecorder.stop();
    }
    isRecording = false;
  }

  function reset() {
    messages = [];
    input = "";
  }

  async function send() {
    const text = input.trim();
    if (!text || loading) return;
    input = "";
    // Verlauf der bisherigen Runden (ohne die aktuelle) mitgeben, damit der
    // Assistent Zusammenhaenge ueber mehrere Nachrichten hinweg behaelt.
    const history = messages
      .filter((m) => !m.error && m.text.trim() !== "")
      .slice(-10)
      .map((m) => ({ role: m.role, text: m.text }));
    messages = [...messages, { role: "user", text, actions: [] }];
    loading = true;
    try {
      const res: AssistantResult = await askAssistant(text, fullContext, history);
      // Zeige die Antwort (ohne Action-Buttons).
      messages = [...messages, { role: "assistant", text: res.reply || "(keine Antwort)", actions: [] }];
      // Aktionen direkt ausführen, Output unterdrücken.
      for (const action of res.actions) {
        await runAction(action);
      }
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
          if (module !== "calendar") await goto("/calendar");
          onclose();
          return "Termin angelegt.";
        }
        case "task_create":
          await createTodo({
            summary: (p.summary as string) ?? (p.title as string) ?? "Aufgabe",
            due: (p.due as string) ?? undefined,
          });
          if (module !== "tasks") await goto("/tasks");
          onclose();
          return "Aufgabe angelegt.";
        case "find_mail": {
          assistantAction.set({ type: "search", query: (p.query as string) ?? "" });
          if (module !== "mail") await goto("/");
          onclose();
          return "Suche gestartet.";
        }
        case "compose_mail": {
          assistantAction.set({
            type: "open_compose",
            to: (p.to as string) ?? "",
            subject: (p.subject as string) ?? "",
            body: (p.body as string) ?? "",
          });
          if (module !== "mail") await goto("/");
          onclose();
          return "Mail-Entwurf geöffnet.";
        }
        default:
          return `Aktion „${action.type}" vorbereitet.`;
      }
    } catch (e) {
      return `Fehler: ${String(e)}`;
    }
  }

</script>

{#if open}
  <aside class="assistant-pop" bind:this={popEl} role="dialog" aria-label="Assistent">
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
            </div>
          {/if}
        {/each}
        {#if loading}
          <div class="chat-msg assistant"><span class="chat-typing">…</span></div>
        {/if}
      </div>
      <footer class="assistant-footer">
        <div class="assistant-input-wrap">
          <input
            bind:this={inputEl}
            bind:value={input}
            class="assistant-input"
            type="text"
            placeholder="Wie kann ich helfen?"
            onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); send(); } }}
          />
          <button
            type="button"
            class="assistant-mic"
            class:recording={isRecording}
            class:voice-enabled={voiceEnabled}
            disabled={transcribing}
            onclick={toggleVoiceInput}
            title={isRecording ? $t("assistant.micStop") : $t("assistant.micStart")}
            aria-label={isRecording ? $t("assistant.micStop") : $t("assistant.micStart")}
          >
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor" width="18" height="18">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
            </svg>
          </button>
        </div>
        <button type="button" class="assistant-send" disabled={loading || transcribing || !input.trim()} onclick={send}>
          {transcribing ? "…" : "Senden"}
        </button>
      </footer>
      {#if voiceError}
        <div class="assistant-voice-error">{voiceError}</div>
      {/if}
    </aside>
{/if}

<style>
  .assistant-pop {
    position: fixed;
    bottom: 80px;
    right: 20px;
    width: 380px;
    height: min(520px, calc(100vh - 100px));
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 14px;
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.18);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    transform-origin: bottom right;
    animation: popIn 160ms ease-out;
    z-index: 1000;
  }
  @keyframes popIn {
    from {
      opacity: 0;
      transform: scale(0.94) translateY(8px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }
  @media (max-width: 768px) {
    .assistant-pop {
      left: 0;
      right: 0;
      bottom: 0;
      width: 100%;
      height: min(70vh, 560px);
      border-radius: 16px 16px 0 0;
      transform-origin: bottom center;
    }
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
  .assistant-input-wrap {
    position: relative;
    flex: 1;
    display: flex;
  }
  .assistant-input {
    flex: 1;
    width: 100%;
    height: 40px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    padding: 0 40px 0 12px;
    font-family: inherit;
    font-size: 0.875rem;
    line-height: 1;
    color: var(--color-text);
    background: var(--color-card);
  }
  .assistant-mic {
    position: absolute;
    right: 5px;
    top: 50%;
    transform: translateY(-50%);
    width: 30px;
    height: 30px;
    border: none;
    border-radius: 50%;
    background: var(--color-card);
    color: var(--color-text-secondary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }
  .assistant-mic:hover {
    color: var(--color-text);
    background: var(--color-active-wash);
  }
  .assistant-mic.recording {
    background: var(--color-danger);
    color: #fff;
    animation: micPulse 1.2s ease-in-out infinite;
  }
  .assistant-mic:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .assistant-voice-error {
    position: absolute;
    bottom: 62px;
    left: 12px;
    right: 12px;
    padding: 8px 10px;
    background: var(--color-active-wash);
    color: var(--color-danger);
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-s);
    font-size: 0.8rem;
    z-index: 1;
  }
  @keyframes micPulse {
    0%, 100% { box-shadow: 0 0 0 0 rgba(220, 80, 80, 0.5); }
    50% { box-shadow: 0 0 0 6px rgba(220, 80, 80, 0); }
  }
  .assistant-send {
    height: 40px;
    border: none;
    border-radius: var(--radius-s);
    background: var(--color-accent);
    color: #fff;
    padding: 0 16px;
    cursor: pointer;
    font-weight: 500;
    display: flex;
    align-items: center;
    justify-content: center;
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
  .chat-typing {
    color: var(--color-text-secondary);
  }
</style>
