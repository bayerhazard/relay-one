<script lang="ts">
  interface Props {
    score: number;
    warnings: string[];
  }

  import { t } from "$lib/i18n";

  let { score, warnings }: Props = $props();
</script>

{#if score > 0.6}
  <div class="fraud-warning">
    <span class="fraud-icon">&#x26A0;</span>
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
    background: color-mix(in srgb, var(--color-danger) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-danger) 25%, transparent);
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
    color: var(--color-danger);
  }
  .fraud-list {
    margin: 2px 0 0 14px;
    padding: 0;
    color: var(--color-text-secondary);
  }
</style>
