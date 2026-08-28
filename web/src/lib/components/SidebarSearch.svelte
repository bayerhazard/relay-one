<script lang="ts">
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
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
    </span>
  {/if}
  <input type="text" class="ss-input" {placeholder} aria-label={ariaLabel} bind:value oninput={onInput} onfocus={onFocus} onblur={onBlur} onkeydown={onKeydown} />
  {@render children?.()}
  {#if value}
    <button type="button" class="ss-clear" onclick={() => (value = "")} aria-label={clearLabel}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
    </button>
  {/if}
</div>

<style>
  .ss-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--color-border);
    background: var(--color-list);
    transition: border-color 0.15s ease-in-out;
  }
  .ss-bar:focus-within {
    border-color: var(--color-accent);
  }
  .ss-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--color-text-secondary);
    opacity: 0.6;
  }
  .ss-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--color-text);
    font-size: var(--fs-base);
    font-family: inherit;
    outline: none;
  }
  .ss-input::placeholder {
    color: var(--color-text-secondary);
  }
  .ss-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border: none;
    background: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 2px;
    border-radius: var(--radius-s);
  }
  .ss-clear:hover {
    color: var(--color-text);
  }
</style>
