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

<div class="ss-bar">
  {#if showIcon}
    <span class="ss-icon" aria-hidden="true">
      <Symbol name="suche" size={16} />
    </span>
  {/if}
  <input type="text" class="ss-input" {placeholder} aria-label={ariaLabel} bind:value oninput={onInput} onfocus={onFocus} onblur={onBlur} onkeydown={onKeydown} />
  {@render children?.()}
  {#if value}
    <button type="button" class="ss-clear" onclick={() => (value = "")} aria-label={clearLabel}>
      <Symbol name="schliessen" size={16} />
    </button>
  {/if}
</div>

<style>
  .ss-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    flex: 0 0 auto;
    min-width: 0;
    height: 34px;
    padding: 0 12px;
    border-radius: var(--am-radius-mittel);
    border: 1px solid var(--am-rand);
    background: var(--am-seite);
    transition: border-color var(--am-dauer-schnell) var(--am-kurve);
  }
  .ss-bar:focus-within {
    border-color: var(--am-handlung-ruhend);
  }
  .ss-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--am-text-gedaempft);
    opacity: 0.6;
  }
  .ss-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--am-text-primaer);
    font-size: var(--fs-base);
    font-family: inherit;
    outline: none;
  }
  .ss-input::placeholder {
    color: var(--am-text-gedaempft);
  }
  .ss-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border: none;
    background: none;
    color: var(--am-text-gedaempft);
    cursor: pointer;
    padding: 2px;
    border-radius: var(--am-radius-klein);
  }
  .ss-clear:hover {
    color: var(--am-text-primaer);
  }
</style>
