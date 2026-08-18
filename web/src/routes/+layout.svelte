<script lang="ts">
  import "../styles/global.css";
  import { onMount } from "svelte";
  import { cacheInit } from "$lib/services/tauri";

  interface Props {
    children: import("svelte").Snippet;
  }
  let { children }: Props = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      await cacheInit();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });
</script>

{#if error}
  <div class="fatal-error">
    <h2>Fehler beim Start</h2>
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
    color: var(--color-danger);
    font-family: "Geist", sans-serif;
  }
  .fatal-error h2 { margin-bottom: 8px; }

  .loading-screen {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background: var(--color-list);
  }
  .loading-dots {
    display: flex;
    gap: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .dot:nth-child(2) { animation-delay: 0.2s; }
  .dot:nth-child(3) { animation-delay: 0.4s; }
  @keyframes pulse {
    0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }
</style>
