<script lang="ts">
  // AI-Assistent v2 (Concept §10.1): agentic drawer. Streams the agent loop via
  // SSE, renders confirmation cards (PlanCard), a live status line and a "what I
  // did" trace. Write actions never execute directly — they become plans the
  // user confirms (B4 fixed). Voice input is unchanged from v1.
  import { goto } from "$app/navigation";
  import { get } from "svelte/store";
  import { t, translate, lang } from "$lib/i18n";
  import { effects } from "$lib/stores/effects";
  import { bumpDataVersion } from "$lib/stores/invalidation";
  import { blobToWavBase64 } from "$lib/utils/wav";
  import {
    runAgent,
    confirmPlan as confirmPlanApi,
    cancelPlan as cancelPlanApi,
    undoPlan as undoPlanApi,
    getVoiceSettings,
    voiceTranscribe,
    voiceSpeak,
    type AgentPlan,
    type AgentStep,
    type AgentEvent,
    type PlanStatus,
  } from "$lib/services/tauri";
  import PlanCard from "$lib/components/PlanCard.svelte";

  interface Props {
    open: boolean;
    module: "mail" | "calendar" | "contacts" | "tasks" | "settings";
    context?: string;
    onclose: () => void;
    /** Phase C: a plan handed in from a mail footer chip (origin=mail_followup). */
    externalPlan?: AgentPlan | null;
  }

  let { open, module, context: _context = "", onclose, externalPlan = null }: Props = $props();

  interface ChatMsg {
    role: "user" | "assistant";
    text: string;
    plans: AgentPlan[];
    steps: AgentStep[];
    error?: string;
    nachfrage?: string;
  }

  let messages = $state<ChatMsg[]>([]);
  let input = $state("");
  let loading = $state(false);
  let statusLabel = $state<string | null>(null);
  let streamPlans = $state<AgentPlan[]>([]);
  let sessionId = $state<string | null>(null);
  let planBusy = $state<string | null>(null);
  let abortController = $state<AbortController | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);
  let popEl = $state<HTMLElement | null>(null);
  // Phase C: track the last injected external plan id so a re-render of the
  // same plan does not inject it twice.
  let lastExternalPlanId = $state<string | null>(null);

  // ─── Voice input (dictation into the assistant) ───────────────────
  let voiceEnabled = $state(false);
  let isRecording = $state(false);
  let transcribing = $state(false);
  let voiceError = $state<string | null>(null);
  let mediaRecorder: MediaRecorder | null = null;
  let audioChunks: Blob[] = [];

  // ─── Phase D: TTS (speak assistant replies) ──────────────────────
  let ttsEnabled = $state(false);
  let ttsAuto = $state(false);
  let speakingMsg = $state<number | null>(null);
  let ttsError = $state<string | null>(null);
  let spokenCount = $state(0);
  let currentAudio: HTMLAudioElement | null = null;

  function stopSpeaking() {
    if (currentAudio) {
      currentAudio.pause();
      currentAudio = null;
    }
    speakingMsg = null;
  }

  async function speakMessage(text: string, index: number) {
    ttsError = null;
    if (speakingMsg === index) {
      stopSpeaking();
      return;
    }
    stopSpeaking();
    try {
      const audio = await voiceSpeak(text, get(lang));
      currentAudio = audio;
      speakingMsg = index;
      audio.addEventListener("ended", () => {
        if (currentAudio === audio) {
          currentAudio = null;
          speakingMsg = null;
        }
      });
    } catch (e) {
      speakingMsg = null;
      ttsError = e instanceof Error ? e.message : String(e);
    }
  }

  // Auto-TTS: speak each new assistant reply once (only when enabled + on).
  $effect(() => {
    if (!ttsAuto || !ttsEnabled) return;
    const assistantMsgs = messages
      .map((m, i) => ({ m, i }))
      .filter(({ m }) => m.role === "assistant" && !m.error && m.text.trim());
    if (assistantMsgs.length > spokenCount) {
      const last = assistantMsgs[assistantMsgs.length - 1];
      spokenCount = assistantMsgs.length;
      void speakMessage(last.m.text, last.i);
    }
  });

  // Stop any in-flight TTS when the drawer closes.
  $effect(() => {
    if (!open) stopSpeaking();
  });

  $effect(() => {
    if (!open) return;
    requestAnimationFrame(() => inputEl?.focus());
    spokenCount = 0;
    stopSpeaking();
    getVoiceSettings().then((s) => {
      voiceEnabled = s?.enabled ?? false;
      ttsEnabled = s?.ttsEnabled ?? false;
      ttsAuto = s?.ttsAuto ?? false;
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

  // Phase C (Concept §9.4): a mail footer chip built a plan (origin=mail_followup)
  // and handed it in. Inject it as an assistant message so the T1 card renders
  // for confirmation. Nothing executes until the user confirms the card.
  $effect(() => {
    const plan = externalPlan;
    if (!plan) return;
    if (plan.id === lastExternalPlanId) return;
    lastExternalPlanId = plan.id;
    messages = [
      ...messages,
      {
        role: "assistant",
        text: $t("assistant.followupCard"),
        plans: [plan],
        steps: [],
      },
    ];
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
    messages = [...messages, { role: "user", text, plans: [], steps: [] }];
    loading = true;
    statusLabel = null;
    streamPlans = [];
    const controller = new AbortController();
    abortController = controller;
    try {
      const result = await runAgent(
        text,
        { sessionId: sessionId ?? undefined, module, lang: get(lang) },
        (ev: AgentEvent) => {
          if (ev.type === "status") statusLabel = ev.label;
          else if (ev.type === "plan") streamPlans = [...streamPlans, ev.plan];
          else if (ev.type === "effect") applyEffect(ev.effect);
          else if (ev.type === "done") sessionId = ev.result.session_id;
        },
        controller.signal,
      );
      sessionId = result.session_id;
      const answer = result.answer || result.nachfrage || translate("assistant.noReply");
      messages = [
        ...messages,
        {
          role: "assistant",
          text: answer,
          plans: result.plans,
          steps: result.steps,
          nachfrage: result.nachfrage ?? undefined,
        },
      ];
    } catch (e: unknown) {
      if (!controller.signal.aborted) {
        const raw = e instanceof Error ? e.message : String(e);
        // 409 llm_not_configured → friendly setup hint instead of the raw code.
        const msg = raw.includes("llm_not_configured") ? translate("assistant.setupHint") : raw;
        messages = [
          ...messages,
          { role: "assistant", text: "", plans: [], steps: [], error: msg },
        ];
      }
    } finally {
      loading = false;
      statusLabel = null;
      streamPlans = [];
      abortController = null;
    }
  }

  function stop() {
    abortController?.abort();
  }

  // Apply a declarative effect to the queue (Concept §5.5). The effect router in
  // +layout.svelte consumes it. Unknown shapes are ignored.
  function applyEffect(eff: Record<string, unknown>) {
    const kind = eff.effect as string | undefined;
    if (!kind) return;
    if (kind === "navigate") {
      const m = eff.module as "mail" | "calendar" | "contacts" | "tasks" | "settings" | undefined;
      if (m) effects.push({ kind: "navigate", module: m });
    } else if (kind === "calendar.set_view") {
      effects.push({
        kind: "calendar.set_view",
        view: (eff.view as "day" | "week" | "month") ?? "day",
        date: (eff.date as string | undefined) ?? undefined,
      });
    } else if (kind === "mail.open") {
      effects.push({ kind: "mail.open", uid: Number(eff.uid) || 0, folder: (eff.folder as string) ?? undefined, account_id: (eff.account_id as number) ?? undefined });
    } else if (kind === "contacts.open") {
      effects.push({ kind: "contacts.open", uid: String(eff.uid ?? "") });
    } else if (kind === "tasks.open") {
      effects.push({ kind: "tasks.open", uid: String(eff.uid ?? "") });
    } else if (kind === "calendar.open_event") {
      effects.push({ kind: "calendar.open_event", id: String(eff.id ?? "") });
    } else if (kind === "compose.open") {
      effects.push({ kind: "compose.open", to: String(eff.to ?? ""), subject: String(eff.subject ?? ""), body: String(eff.body ?? "") });
    } else if (kind === "highlight") {
      effects.push({ kind: "highlight", art: (eff.art as "mail" | "contact" | "task" | "event") ?? "task", id: String(eff.id ?? "") });
    }
  }

  // ─── Plan lifecycle (server-side execution, Concept §5.3) ──────────
  function setPlanStatus(planId: string, status: PlanStatus, resultJson?: string | null) {
    for (const m of messages) {
      for (const p of m.plans) {
        if (p.id === planId) {
          p.status = status;
          if (resultJson) p.result_json = resultJson;
        }
      }
    }
  }

  async function onConfirmPlan(plan: AgentPlan, confirmExternal: boolean) {
    planBusy = plan.id;
    try {
      const res = await confirmPlanApi(plan.id, confirmExternal);
      setPlanStatus(plan.id, res.status as PlanStatus, JSON.stringify(res.results));
      bumpDataVersion();
    } catch {
      setPlanStatus(plan.id, "failed");
    } finally {
      planBusy = null;
    }
  }

  async function onDiscardPlan(plan: AgentPlan) {
    planBusy = plan.id;
    try {
      await cancelPlanApi(plan.id);
      setPlanStatus(plan.id, "cancelled");
    } catch {
      /* keep pending on error */
    } finally {
      planBusy = null;
    }
  }

  async function onUndoPlan(plan: AgentPlan) {
    planBusy = plan.id;
    try {
      await undoPlanApi(plan.id);
      setPlanStatus(plan.id, "cancelled");
      bumpDataVersion();
    } catch {
      /* keep executed on error */
    } finally {
      planBusy = null;
    }
  }

  function onOpenPath(path: string) {
    if (path) goto(path);
  }

</script>

{#if open}
  <aside class="assistant-pop" bind:this={popEl} role="dialog" aria-label={$t("assistant.title")}>
      <header class="assistant-header">
        <span class="assistant-title">{$t("assistant.title")}</span>
        <button type="button" class="assistant-close" onclick={onclose} aria-label={$t("assistant.close")}>✕</button>
      </header>
      <div class="assistant-body">
        {#if messages.length === 0}
          <p class="assistant-hint">{$t("assistant.hint")}</p>
        {/if}
        {#each messages as m, i (m.text + m.plans.length + m.steps.length)}
          {#if m.error}
            <div class="chat-msg assistant error">{m.error}</div>
          {:else}
            <div class="chat-msg {m.role}">
              <div class="chat-text">{m.text}</div>
              {#if m.role === "assistant" && ttsEnabled && m.text.trim()}
                <button
                  type="button"
                  class="chat-speak"
                  class:speaking={speakingMsg === i}
                  onclick={() => speakMessage(m.text, i)}
                  title={speakingMsg === i ? $t("assistant.speakStop") : $t("assistant.speak")}
                  aria-label={speakingMsg === i ? $t("assistant.speakStop") : $t("assistant.speak")}
                >{speakingMsg === i ? "⏹" : "🔊"}</button>
              {/if}
            </div>
            {#each m.plans as plan (plan.id)}
              <div class="chat-plan">
                <PlanCard
                  {plan}
                  busy={planBusy === plan.id}
                  onConfirm={onConfirmPlan}
                  onDiscard={onDiscardPlan}
                  onUndo={onUndoPlan}
                  onOpen={onOpenPath}
                />
              </div>
            {/each}
            {#if m.steps.length > 0}
              <details class="chat-steps">
                <summary>{$t("assistant.whatIDid")}</summary>
                <ul>
                  {#each m.steps as s (s.tool + s.label)}
                    <li>{s.label}</li>
                  {/each}
                </ul>
              </details>
            {/if}
          {/if}
        {/each}
        {#if loading}
          <div class="chat-msg assistant status" role="status">
            <span class="chat-typing">…</span>
            <span class="assistant-thinking">{statusLabel ?? $t("assistant.thinking")}</span>
          </div>
          {#each streamPlans as plan (plan.id)}
            <div class="chat-plan">
              <PlanCard
                {plan}
                busy={planBusy === plan.id}
                onConfirm={onConfirmPlan}
                onDiscard={onDiscardPlan}
                onUndo={onUndoPlan}
                onOpen={onOpenPath}
              />
            </div>
          {/each}
        {/if}
        {#if ttsError}
          <div class="chat-msg assistant error" role="alert">{ttsError}</div>
        {/if}
      </div>
      <footer class="assistant-footer">
        <div class="assistant-input-wrap">
          <input
            bind:this={inputEl}
            bind:value={input}
            class="assistant-input"
            type="text"
            placeholder={$t("assistant.placeholder")}
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
        {#if loading}
          <button type="button" class="assistant-send assistant-stop" onclick={stop} aria-label={$t("assistant.stop")}>
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" width="16" height="16" aria-hidden="true">
              <rect x="6" y="6" width="12" height="12" rx="2" />
            </svg>
            {$t("assistant.stop")}
          </button>
        {:else}
          <button type="button" class="assistant-send" disabled={transcribing || !input.trim()} onclick={send}>
            {transcribing ? "…" : $t("assistant.send")}
          </button>
        {/if}
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
  .assistant-thinking {
    display: block;
    margin-top: 4px;
    font-size: 0.78rem;
    color: var(--color-text-secondary);
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
  .chat-msg.outcome {
    margin-right: auto;
    max-width: 100%;
    padding: 6px 10px;
    font-size: 0.8rem;
    color: var(--color-text-secondary);
    background: transparent;
    border: none;
    border-left: 2px solid var(--color-border);
  }
  .chat-speak {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-top: 6px;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    border: 1px solid var(--color-border);
    background: transparent;
    color: var(--color-text-secondary);
    font-size: 0.85rem;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }
  .chat-speak:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .chat-speak.speaking {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: #fff;
  }
  .chat-typing {
    color: var(--color-text-secondary);
  }
  .chat-plan {
    margin: 0 0 10px;
    animation: planIn 200ms cubic-bezier(0.2, 0, 0, 1);
  }
  @keyframes planIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .chat-plan { animation: none; }
  }
  .chat-steps {
    margin: 0 0 10px;
    font-size: 0.78rem;
    color: var(--color-text-secondary);
  }
  .chat-steps summary {
    cursor: pointer;
    font-weight: 600;
    user-select: none;
  }
  .chat-steps ul {
    margin: 6px 0 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .assistant-stop {
    background: var(--color-danger);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .assistant-stop:hover {
    filter: brightness(1.08);
  }
</style>
