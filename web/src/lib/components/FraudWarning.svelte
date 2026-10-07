<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  interface Props {
    score: number;
    warnings: string[];
  }

  import { t } from "$lib/i18n";

  let { score, warnings }: Props = $props();
</script>

<!-- HB-ZUSTAND notice line: "achtung" for a suspicion, "fehler" once the
     score is high. `fraud-warning` is the layout hook in the message row. -->
{#if score > 0.6}
  <div class="hinweis fraud-warning" data-art={score >= 0.8 ? "fehler" : "achtung"}>
    <Symbol name="achtung" size={16} />
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
  /* ── Fraud warning in the message row [RL-BETRUGSHINWEIS] ───────────── */
  .fraud-warning {
    margin-top: var(--am-raum-1);
    padding: var(--am-raum-1) var(--am-raum-2);
    font-size: 0.75rem;
  }
  .fraud-title {
    font-weight: 600;
  }
  .fraud-list {
    margin: 2px 0 0 14px;
    padding: 0;
    color: var(--am-text-sekundaer);
  }
</style>
