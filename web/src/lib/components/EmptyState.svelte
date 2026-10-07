<script lang="ts">
  // Unified empty / informational state used across the mailbox (empty folder,
  // no selection, no search results, offline). Keeps the visual language
  // consistent: centered icon + title + optional subtitle + optional action.
  // The icon is a name from the CI set (HB-SYMBOL), 40 px as in every empty
  // state (CI R2) — no more emoji guessed into icons (RL-Z1).
  import Symbol from "./Symbol.svelte";
  import type { SymbolName } from "$lib/symbole";

  interface Props {
    icon?: SymbolName;
    title: string;
    subtitle?: string;
    actionLabel?: string;
    onaction?: () => void;
    tone?: "neutral" | "error";
    offsetHeader?: boolean;
  }

  let {
    icon,
    title,
    subtitle = "",
    actionLabel = "",
    onaction,
    tone = "neutral",
    offsetHeader = false,
  }: Props = $props();
</script>

<div class="empty-state" class:error={tone === "error"} class:offset-header={offsetHeader}>
  {#if icon}
    <div class="empty-state-icon-wrapper">
      <Symbol name={icon} size={40} />
    </div>
  {/if}
  <p class="empty-state-title">{title}</p>
  {#if subtitle}
    <p class="empty-state-subtitle">{subtitle}</p>
  {/if}
  {#if actionLabel && onaction}
    <button type="button" class="empty-state-action" onclick={onaction}>
      {actionLabel}
    </button>
  {/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 48px 32px;
    text-align: center;
    color: var(--am-text-gedaempft);
    gap: 8px;
  }
  .empty-state.offset-header {
    padding-top: calc(72px + 48px);
  }
  /* The icon stands free, as in AM-LEER: 40 px, Blau 300, no circle. */
  .empty-state-icon-wrapper {
    display: flex;
    color: var(--am-blau-300);
    margin-bottom: var(--am-raum-4);
  }
  .empty-state.error .empty-state-icon-wrapper {
    color: var(--am-fehler);
  }
  .empty-state-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--am-text-primaer);
    margin: 0;
  }
  .empty-state-subtitle {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
    margin: 0;
    max-width: 280px;
    line-height: 1.5;
  }
  .empty-state-action {
    margin-top: 16px;
    padding: 8px 18px;
    border: 1px solid var(--am-rand);
    border-radius: 8px;
    background: var(--am-seite);
    color: var(--am-text-primaer);
    font-size: 0.8125rem;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .empty-state-action:hover {
    border-color: var(--am-handlung-ruhend);
    color: var(--am-handlung-ruhend);
    background: var(--am-flaeche-2);
  }
  .empty-state.error .empty-state-action:hover {
    border-color: var(--am-fehler);
    color: var(--am-fehler);
  }
</style>
