<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import type { Snippet } from "svelte";

  let {
    value = $bindable(""),
    placeholder = "",
    ariaLabel = "",
    clearLabel = "",
    showIcon = true,
    onInput,
    onFocus,
    onBlur,
    onKeydown,
    children,
  }: {
    value?: string;
    placeholder?: string;
    ariaLabel?: string;
    clearLabel?: string;
    showIcon?: boolean;
    onInput?: (e: Event) => void;
    onFocus?: (e: Event) => void;
    onBlur?: (e: Event) => void;
    onKeydown?: (e: KeyboardEvent) => void;
    children?: Snippet;
  } = $props();
</script>

<!-- Search field after HB-SUCHE's pattern: the input is the real AM-FELD
     `.input`; icon and trailing buttons sit on top of it. -->
<div class="ss-bar">
  {#if showIcon}
    <span class="ss-icon" aria-hidden="true">
      <Symbol name="suche" size={16} />
    </span>
  {/if}
  <input type="text" class="input ss-input" class:mit-zeichen={showIcon} {placeholder} aria-label={ariaLabel} bind:value oninput={onInput} onfocus={onFocus} onblur={onBlur} onkeydown={onKeydown} />
  <div class="ss-aktionen">
    {@render children?.()}
    {#if value}
      <button type="button" class="btn btn-still btn-symbol btn-klein" onclick={() => (value = "")} aria-label={clearLabel} title={clearLabel}>
        <Symbol name="schliessen" size={16} />
      </button>
    {/if}
  </div>
</div>

<style>
  /* ── Sidebar search [RL-SPALTE] ───────────────────────────────────────────
     Field and buttons from AM-FELD / AM-KNOPF; only the overlay placement
     lives here (HB-SUCHE's `.kopfsuche` arrangement, until Etappe 6). */
  .ss-bar {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    flex: 0 0 auto;
    min-width: 0;
  }
  .ss-icon {
    position: absolute;
    left: var(--am-raum-3);
    display: inline-flex;
    color: var(--am-text-gedaempft);
    pointer-events: none;
  }
  .ss-input {
    min-width: 0;
  }
  /* Room for the icon on the left and for each trailing button on the right. */
  .ss-input.mit-zeichen {
    padding-left: calc(var(--am-raum-3) + 16px + var(--am-raum-2));
  }
  .ss-bar:has(.ss-aktionen > :global(*)) .ss-input {
    padding-right: var(--am-ziel-zeiger);
  }
  .ss-bar:has(.ss-aktionen > :global(* + *)) .ss-input {
    padding-right: calc(var(--am-ziel-zeiger) * 2);
  }
  .ss-aktionen {
    position: absolute;
    top: 0;
    right: 0;
    display: flex;
    align-items: center;
  }
</style>
