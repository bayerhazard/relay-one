<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  interface Props {
    score: number;
    warnings: string[];
  }

  import { t } from "$lib/i18n";

  let { score, warnings }: Props = $props();
</script>

{#if score > 0.6}
  <div class="fraud-warning">
    <span class="fraud-icon"><Symbol name="achtung" size={16} /></span>
    <div class="fraud-body">
      <span class="fraud-title">{$t("fraud.suspected")}</span>
      {#if warnings.length > 0}
        <ul class="fraud-list">
          {#each warnings as warning}
            <li>{warning}</li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
{/if}

<style>
  .fraud-warning {
    display: flex;
    gap: 8px;
    padding: 6px 8px;
    background: color-mix(in srgb, var(--am-fehler) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--am-fehler) 25%, transparent);
    border-radius: 6px;
    margin-top: 4px;
  }
  .fraud-icon {
    font-size: 0.875rem;
    flex-shrink: 0;
  }
  .fraud-body {
    font-size: 0.6875rem;
  }
  .fraud-title {
    font-weight: 600;
    color: var(--am-fehler);
  }
  .fraud-list {
    margin: 2px 0 0 14px;
    padding: 0;
    color: var(--am-text-gedaempft);
  }
</style>
