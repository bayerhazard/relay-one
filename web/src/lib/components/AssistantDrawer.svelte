<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  // AI-Assistent v2 (Concept §10.1) in the shape of HB-ASSISTENT (Rocket:
  // assistent.tsx): the panel above the shield — head, history, empty state
  // with examples, input. Streams the agent loop via SSE, renders
  // confirmation cards (PlanCard), a live status line and a "what I did"
  // trace. Write actions never execute directly — they become plans the user
  // confirms (B4 fixed). Relay's own on top of Rocket: dictation at the input,
  // reading answers aloud, the trace, stop while running.
  import Schild from "$lib/components/Schild.svelte";
  import { goto } from "$app/navigation";
  import { get } from "svelte/store";
  import { t, translate, lang } from "$lib/i18n";
  import { effects } from "$lib/stores/effects";
  import { bumpDataVersion } from "$lib/stores/invalidation";
  import { blobToWavBase64 } from "$lib/utils/wav";
  import { markFollowupDoneKey, meetingFollowupKey } from "$lib/utils/followupMemory";
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
  import { renderMarkdown } from "$lib/utils/markdown";

  interface Props {
    open: boolean;
    module: "mail" | "calendar" | "contacts" | "tasks" | "meetings" | "settings";
    context?: string;
    onclose: () => void;
    /** Phase C: a plan handed in from a mail footer chip (origin=mail_followup). */
    externalPlan?: AgentPlan | null;
    /** Where focus returns on Escape / the close button: the shield. */
    returnFocus?: HTMLElement | null;
    /** Out: the agent is running (the shield stops blinking). */
    busy?: boolean;
  }

  let {
    open,
    module,
    context: _context = "",
    onclose,
    externalPlan = null,
    returnFocus = null,
    busy = $bindable(false),
  }: Props = $props();

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
  let inputEl = $state<HTMLTextAreaElement | null>(null);
  let popEl = $state<HTMLElement | null>(null);
  let endEl = $state<HTMLElement | null>(null);

  $effect(() => {
    busy = loading;
  });

  // Examples per module for the empty state (HB-ASSISTENT: Rocket's
  // BEISPIELE). Literal keys so the i18n guard sees them.
  const examples = $derived.by(() => {
    void $lang;
    switch (module) {
      case "mail":
        return [translate("assistant.example.mail.1"), translate("assistant.example.mail.2"), translate("assistant.example.mail.3"), translate("assistant.example.mail.4")];
      case "calendar":
        return [translate("assistant.example.calendar.1"), translate("assistant.example.calendar.2"), translate("assistant.example.calendar.3"), translate("assistant.example.calendar.4")];
      case "contacts":
        return [translate("assistant.example.contacts.1"), translate("assistant.example.contacts.2"), translate("assistant.example.contacts.3"), translate("assistant.example.contacts.4")];
      case "tasks":
        return [translate("assistant.example.tasks.1"), translate("assistant.example.tasks.2"), translate("assistant.example.tasks.3"), translate("assistant.example.tasks.4")];
      case "meetings":
        return [translate("assistant.example.meetings.1"), translate("assistant.example.meetings.2"), translate("assistant.example.meetings.3"), translate("assistant.example.meetings.4")];
      default:
        return [translate("assistant.example.settings.1"), translate("assistant.example.settings.2"), translate("assistant.example.settings.3"), translate("assistant.example.settings.4")];
    }
  });

  function useExample(text: string) {
    input = text;
    inputEl?.focus();
  }

  // Close from inside (Escape, the close button): focus goes back to the
  // shield it opened from, as in Rocket.
  function closeAndReturn() {
    onclose();
    returnFocus?.focus();
  }

  // Keep the newest entry in view (Rocket: ende.scrollIntoView).
  $effect(() => {
    void messages.length;
    void loading;
    void streamPlans.length;
    endEl?.scrollIntoView?.({ block: "end" });
  });
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
    // composedPath, not target.contains: a click that swaps its own button
    // (send → stop, the external card's arming, the mic icon) leaves a
    // detached target behind by the time the event reaches the document.
    const onDocClick = (e: MouseEvent) => {
      if (popEl && !e.composedPath().includes(popEl)) onclose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        closeAndReturn();
      }
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
      const m = eff.module as "mail" | "calendar" | "contacts" | "tasks" | "meetings" | "settings" | undefined;
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
      // Remember executed followup steps so the same suggestion is not offered
      // again for the same source (mail uid or meeting id, namespaced by the
      // plan origin — persistent per-user memory).
      if (res.status === "executed" && plan.source_message_id != null) {
        const key =
          plan.origin === "meeting_followup"
            ? meetingFollowupKey(plan.source_message_id)
            : String(plan.source_message_id);
        for (const step of plan.steps) {
          markFollowupDoneKey(key, {
            id: step.tool,
            titel: step.title,
            tool: step.tool,
            args: step.request.body,
            plan: step,
          });
        }
      }
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
  <section class="assistent-panel" id="assistent-panel" bind:this={popEl} aria-label={$t("assistant.title")}>
    <header class="assistent-kopf">
      <Schild size={16} />
      <span>{$t("assistant.title")}</span>
      <button type="button" class="assistent-zu" onclick={closeAndReturn} aria-label={$t("assistant.close")} title={$t("assistant.close")}>
        <Symbol name="schliessen" size={16} />
      </button>
    </header>

    <div class="assistent-verlauf">
      {#if messages.length === 0 && !loading}
        <div class="assistent-leer">
          <p>{$t("assistant.empty")}</p>
          <ul>
            {#each examples as example (example)}
              <li><button type="button" class="alsLink" onclick={() => useExample(example)}>{example}</button></li>
            {/each}
          </ul>
        </div>
      {/if}
      {#each messages as m, i (i)}
        {#if m.role === "user"}
          <p class="assistent-nachricht nutzer">{m.text}</p>
        {:else if m.error}
          <p class="assistent-nachricht fehler" role="alert">{m.error}</p>
        {:else}
          <div class="assistent-antwort">
            {#if m.text.trim()}
              <!-- T5 (Review 2026-09-13): Antworten in Markdown rendern
                   (DOMPurify-gesichert wie alle {@html}-Ausgaben). -->
              <div class="assistent-nachricht antwort-text">{@html renderMarkdown(m.text)}</div>
              {#if ttsEnabled}
                <button
                  type="button"
                  class="btn btn-still btn-symbol btn-klein"
                  aria-pressed={speakingMsg === i}
                  onclick={() => speakMessage(m.text, i)}
                  title={speakingMsg === i ? $t("assistant.speakStop") : $t("assistant.speak")}
                  aria-label={speakingMsg === i ? $t("assistant.speakStop") : $t("assistant.speak")}
                >{#if speakingMsg === i}<Symbol name="stopp" size={16} />{:else}<Symbol name="vorlesen" size={16} />{/if}</button>
              {/if}
            {/if}
            {#each m.plans as plan (plan.id)}
              <PlanCard
                {plan}
                busy={planBusy === plan.id}
                onConfirm={onConfirmPlan}
                onDiscard={onDiscardPlan}
                onUndo={onUndoPlan}
                onOpen={onOpenPath}
              />
            {/each}
            {#if m.steps.length > 0}
              <details class="assistent-spur">
                <summary>{$t("assistant.whatIDid")}</summary>
                {#each m.steps as s, j (j)}
                  <p class="assistent-hinweis">{s.label}</p>
                {/each}
              </details>
            {/if}
          </div>
        {/if}
      {/each}
      {#if loading}
        <p class="assistent-nachricht arbeitet" role="status">{statusLabel ?? $t("assistant.thinking")}</p>
        {#if streamPlans.length > 0}
          <div class="assistent-antwort">
            {#each streamPlans as plan (plan.id)}
              <PlanCard
                {plan}
                busy={planBusy === plan.id}
                onConfirm={onConfirmPlan}
                onDiscard={onDiscardPlan}
                onUndo={onUndoPlan}
                onOpen={onOpenPath}
              />
            {/each}
          </div>
        {/if}
      {/if}
      {#if transcribing}
        <p class="assistent-nachricht arbeitet" role="status">{$t("assistant.transcribing")}</p>
      {/if}
      {#if voiceError}
        <p class="assistent-nachricht fehler" role="alert">{voiceError}</p>
      {/if}
      {#if ttsError}
        <p class="assistent-nachricht fehler" role="alert">{ttsError}</p>
      {/if}
      <div bind:this={endEl}></div>
    </div>

    <form
      class="assistent-eingabe"
      onsubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <!-- Dictation sits inside the field (HB-DIKTAT): speak, and the
           transcript goes out as the request. -->
      <div class="diktat-feld">
        <textarea
          bind:this={inputEl}
          bind:value={input}
          rows="2"
          placeholder={$t("assistant.taskPlaceholder")}
          aria-label={$t("assistant.task")}
          onkeydown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        ></textarea>
        <button
          type="button"
          class="btn btn-still btn-symbol diktat-knopf"
          class:aufnahme={isRecording}
          aria-pressed={isRecording}
          disabled={transcribing}
          onclick={toggleVoiceInput}
          title={isRecording ? $t("assistant.micStop") : $t("assistant.micStart")}
          aria-label={isRecording ? $t("assistant.micStop") : $t("assistant.micStart")}
        >
          <Symbol name={isRecording ? "stopp" : "mikrofon"} size={16} />
        </button>
      </div>
      {#if loading}
        <!-- While the agent runs, the same place stops it. -->
        <button type="button" class="assistent-senden" onclick={stop} aria-label={$t("assistant.halt")} title={$t("assistant.halt")}>
          <Symbol name="stopp" size={16} />
        </button>
      {:else}
        <button type="submit" class="assistent-senden" disabled={transcribing || !input.trim()} aria-label={$t("assistant.send")} title={$t("assistant.send")}>
          <Symbol name="pfeil-hoch" size={16} />
        </button>
      {/if}
    </form>
  </section>
{/if}

<style>
  /* ── Answer text [RL-ASSISTENT] ───────────────────────────────────────────
     Panel, head, history, messages, card and input are HB-ASSISTENT in
     global.css. Relay's own: answers come as Markdown (T5) — normalise the
     block spacing inside the bubble; pre-wrap would double the newlines
     between the rendered blocks. */
  .antwort-text {
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .antwort-text :global(p) { margin: 0.35em 0; }
  .antwort-text :global(p:first-child) { margin-top: 0; }
  .antwort-text :global(p:last-child) { margin-bottom: 0; }
  .antwort-text :global(ul), .antwort-text :global(ol) { margin: 0.35em 0; padding-left: 1.3em; }
  .antwort-text :global(li) { margin: 0.2em 0; }
  .antwort-text :global(code) {
    background: var(--am-flaeche-2);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-klein);
    padding: 1px 5px;
    font-family: var(--am-schrift-mono);
    font-size: 0.85em;
  }

  /* ── Examples [RL-ASSISTENT] ──────────────────────────────────────────────
     The link button itself (Rocket's .alsLink, outside HB-ASSISTENT). */
  .alsLink {
    border: 0;
    background: transparent;
    padding: 0;
    color: var(--am-handlung-ruhend);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .alsLink:hover { text-decoration: underline; }
  :global(.dunkel) .alsLink { color: var(--am-gold-beschriftung); }

  /* ── Trace [RL-ASSISTENT] ─────────────────────────────────────────────────
     "Was ich getan habe": folded, its lines are .assistent-hinweis. */
  .assistent-spur {
    display: grid;
    gap: var(--am-raum-1);
  }
  .assistent-spur summary {
    cursor: pointer;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
    user-select: none;
  }

  /* ── Dictation [RL-ASSISTENT] ─────────────────────────────────────────────
     The mic sits inside the field, bottom right; the field keeps room for
     it. Recording is a live state, not a button style: red with a pulse. */
  .diktat-feld {
    position: relative;
    flex: 1;
    display: flex;
    min-width: 0;
  }
  .diktat-feld textarea {
    padding-right: calc(var(--am-ziel-zeiger) + var(--am-raum-1));
  }
  .diktat-knopf {
    position: absolute;
    right: var(--am-raum-1);
    bottom: var(--am-raum-1);
    height: var(--am-ziel-zeiger);
  }
  .diktat-knopf.aufnahme,
  .diktat-knopf.aufnahme:hover {
    background: var(--am-fehler);
    color: var(--am-handlung-text);
    animation: diktat-puls 1.2s ease-in-out infinite;
  }
  @keyframes diktat-puls {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--am-fehler) 50%, transparent); }
    50% { box-shadow: 0 0 0 6px color-mix(in srgb, var(--am-fehler) 0%, transparent); }
  }
  @media (prefers-reduced-motion: reduce) {
    .diktat-knopf.aufnahme { animation: none; }
  }
</style>
