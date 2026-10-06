<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { computeDiff } from "$lib/utils/diff";
  import { t } from "$lib/i18n";

  interface Props {
    original: string;
    modified: string;
    onaccept: () => void;
    onreject: () => void;
  }

  let { original, modified, onaccept, onreject }: Props = $props();

  let diffs = $derived(computeDiff(original, modified));
</script>

<div class="diff-editor">
  <div class="diff-toolbar">
    <span class="diff-title">{$t("diff.title")}</span>
    <div class="diff-actions">
      <!-- The diff sits inside the compose view, whose one primary is
           "Senden": accepting is secondary, rejecting quiet (AM-KNOPF). -->
      <button type="button" class="btn btn-still btn-klein" onclick={onreject}>{$t("diff.reject")}</button>
      <button type="button" class="btn btn-sekundaer btn-klein" onclick={onaccept}>{$t("diff.accept")}</button>
    </div>
  </div>
  <div class="diff-content">
    {#each diffs as line}
      <div
        class="diff-line"
        class:added={line.type === "added"}
        class:removed={line.type === "removed"}
      >
        <span class="line-prefix">
          {#if line.type === "added"}+{:else if line.type === "removed"}&minus;{/if}
        </span>
        <span class="line-text">{line.content}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  /* ── Diff view [RL-VERGLEICH] ──────────────────────────────────────────────
     Shows the AI draft against the user's text, line by line. */
  .diff-editor {
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    overflow: hidden;
  }
  .diff-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 4px 4px 4px 12px;
    background: var(--am-flaeche-1);
    border-bottom: 1px solid var(--am-rand);
  }
  .diff-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
  }
  /* Rejecting a suggestion is not destructive — never red (CI R1/G2). */
  .diff-actions {
    display: flex;
    gap: 8px;
  }

  /* ── Diff lines [RL-VERGLEICH] ─────────────────────────────────────────── */
  .diff-content {
    padding: 12px;
    font-family: var(--am-schrift-mono);
    font-size: 0.75rem;
    max-height: 400px;
    overflow-y: auto;
    line-height: 1.5;
  }
  .diff-line {
    display: flex;
    gap: 8px;
    padding: 1px 4px;
    min-height: 20px;
  }
  .diff-line.added {
    background: color-mix(in srgb, var(--am-erfolg) 12%, transparent);
  }
  .diff-line.removed {
    background: color-mix(in srgb, var(--am-fehler) 12%, transparent);
  }
  .line-prefix {
    width: 16px;
    text-align: center;
    flex-shrink: 0;
    color: var(--am-text-gedaempft);
    user-select: none;
  }
  .added .line-prefix {
    color: var(--am-erfolg);
  }
  .removed .line-prefix {
    color: var(--am-fehler);
  }
  .line-text {
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
