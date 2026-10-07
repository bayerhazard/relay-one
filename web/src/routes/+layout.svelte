<script lang="ts">
  import "../styles/global.css";
  import Symbol from "$lib/components/Symbol.svelte";
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
  import { appDienstStarten } from "$lib/pwa";

  interface Props {
    children: import("svelte").Snippet;
  }
  let { children }: Props = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);

  // Appearance is applied in one place for every module (CI RL-T2); the
  // inline script in app.html has already set the class before first paint.
  onMount(() => applyAppearance());
  onMount(() => { void appDienstStarten(); });

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
  <!-- HB-ZUSTAND, pinned to the top edge as a slim banner. -->
  <div class="hinweis offline-banner" data-art="achtung"><Symbol name="achtung" size={16} /><span>{$t("app.offline")}</span></div>
{/if}

{#if error}
  <div class="fatal-error">
    <div class="hinweis" data-art="fehler">
      <Symbol name="achtung" size={16} />
      <div>
        <h2>{$t("app.startError")}</h2>
        <p>{error}</p>
      </div>
    </div>
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
  /* ── Start error, full screen [RL-HUELLE] ────────────────────────────────── */
  /* The HB-ZUSTAND error line, centred on an empty page. */
  .fatal-error {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    padding: var(--am-raum-4);
    background: var(--am-seite);
  }
  .fatal-error .hinweis {
    max-width: 35rem;
  }
  .fatal-error h2 {
    margin: 0 0 var(--am-raum-1);
    font-size: 1rem;
    color: inherit;
  }

  /* ── Loading dots while the cache starts [RL-HUELLE] ─────────────────────── */
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

  /* ── Offline banner, on HB-ZUSTAND [RL-HUELLE] ───────────────────────────── */
  /* Colours, sign and border come from .hinweis[data-art="achtung"]; this
     only pins it to the top edge as a full-width strip. */
  .offline-banner {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 9999;
    align-items: center;
    justify-content: center;
    padding: var(--am-raum-1) var(--am-raum-4);
    border-width: 0 0 1px;
    border-radius: 0;
    font-size: 0.75rem;
  }
  .offline-banner > :global(svg) {
    margin-top: 0;
  }
</style>
