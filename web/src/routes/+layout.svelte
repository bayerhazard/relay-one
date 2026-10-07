<script lang="ts">
  import "../styles/global.css";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { cacheInit } from "$lib/services/tauri";
  import { isOnline } from "$lib/offline/online";
  import { effects, applyEffect } from "$lib/stores/effects";
  import { calendarView } from "$lib/stores/calendarView";
  import { selection } from "$lib/stores/selection";
  import { t } from "$lib/i18n";
  import { assistantAction } from "$lib/stores/assistantAction";
  import { applyAppearance } from "$lib/stores/appearance";

  interface Props {
    children: import("svelte").Snippet;
  }
  let { children }: Props = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);

  // Appearance is applied in one place for every module (CI RL-T2); the
  // inline script in app.html has already set the class before first paint.
  onMount(() => applyAppearance());

  onMount(async () => {
    try {
      await cacheInit();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });

  // Effect router (Concept §5.5): consumes the assistant effect queue
  // sequentially. Effects are declarative and whitelist-checked server-side —
  // navigation only ever targets known module routes. `compose.open` reuses the
  // v1 single-shot store so the existing compose flow keeps working.
  $effect(() => {
    const q = $effects;
    if (q.length === 0) return;
    const e = effects.shift();
    if (!e) return;
    applyEffect(e, {
      goto,
      setView: (view, date) => calendarView.setView(view, date),
      setMail: (sel, highlight) => selection.setMail(sel, highlight),
      setContact: (id, highlight) => selection.setContact(id, highlight),
      setTask: (id, highlight) => selection.setTask(id, highlight),
      setEvent: (id, highlight) => selection.setEvent(id, highlight),
      setMeeting: (id, highlight) => selection.setMeeting(id, highlight),
      openCompose: (a) =>
        assistantAction.set({ type: "open_compose", to: a.to, subject: a.subject, body: a.body }),
      selectedMail: $selection.mail,
    });
  });
</script>

{#if !loading && !error && !$isOnline}
  <div class="offline-banner">{$t("app.offline")}</div>
{/if}

{#if error}
  <div class="fatal-error">
    <h2>{$t("app.startError")}</h2>
    <p>{error}</p>
  </div>
{:else if loading}
  <div class="loading-screen">
    <div class="loading-dots">
      <span class="dot"></span>
      <span class="dot"></span>
      <span class="dot"></span>
    </div>
  </div>
{:else}
  {@render children()}
{/if}

<style>
  .fatal-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    color: var(--am-fehler);
    font-family: var(--am-schrift-sans);
  }
  .fatal-error h2 { margin-bottom: 8px; }

  .loading-screen {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background: var(--am-seite);
  }
  .loading-dots {
    display: flex;
    gap: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--am-handlung-ruhend);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .dot:nth-child(2) { animation-delay: 0.2s; }
  .dot:nth-child(3) { animation-delay: 0.4s; }
  @keyframes pulse {
    0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }

  .offline-banner {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 9999;
    padding: 6px 16px;
    background: var(--am-achtung-flaeche);
    color: var(--am-achtung);
    font-size: 12px;
    font-family: var(--am-schrift-sans);
    text-align: center;
    border-bottom: 1px solid var(--am-achtung-rand);
  }
</style>
