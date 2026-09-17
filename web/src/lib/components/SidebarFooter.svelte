<script lang="ts">
  import type { Snippet } from "svelte";
  import ModuleIcons from "$lib/components/ModuleIcons.svelte";

  let {
    active = "",
    children,
    extra,
  }: {
    active?: string;
    children?: Snippet;
    extra?: Snippet;
  } = $props();
</script>

<div class="sidebar-footer">
  {#if children}
    <div class="footer-row">{@render children()}</div>
  {/if}
  <div class="footer-row module-row">
    <ModuleIcons {active} />
  </div>
  {@render extra?.()}
</div>

<style>
  .sidebar-footer {
    margin-top: auto;
    padding: 12px;
    border-top: 1px solid var(--color-border);
  }
  .footer-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin-bottom: 8px;
    padding: 0 15px;
  }
  .module-row {
    padding: 0 10px;
  }
  :global(.app-container.narrow) .sidebar-footer {
    padding-bottom: max(8px, env(safe-area-inset-bottom, 0px));
  }
</style>
