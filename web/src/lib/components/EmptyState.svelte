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

<!-- AM-LEER: sign, title (a paragraph, not a heading), sentence. -->
<div class="leerzustand" class:error={tone === "error"} class:offset-header={offsetHeader}>
  {#if icon}
    <Symbol name={icon} size={40} />
  {/if}
  <p class="leerzustand-titel">{title}</p>
  {#if subtitle}
    <p>{subtitle}</p>
  {/if}
  {#if actionLabel && onaction}
    <button type="button" class="btn btn-sekundaer" onclick={onaction}>
      {actionLabel}
    </button>
  {/if}
</div>

<style>
  /* ── Empty state in a pane [RL-HUELLE] ───────────────────────────────── */
  /* Fills the pane and sits in its middle; the look is AM-LEER's. */
  .leerzustand {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
  }
  .leerzustand.offset-header {
    padding-top: calc(72px + var(--am-raum-16));
  }
  .leerzustand.error > :global(svg) {
    color: var(--am-fehler);
  }
  /* The title rule from the CI's HB-TABELLE, which Relay does not carry yet:
     without it the title would read like the grey sentence below it. */
  .leerzustand-titel {
    color: var(--am-text-primaer);
    font-size: 1rem;
    font-weight: 500;
    margin: 0 auto var(--am-raum-2);
  }
</style>
