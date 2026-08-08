<script lang="ts">
  import { computeDiff } from "$lib/utils/diff";

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
    <span class="diff-title">KI-Vorschlag (&Auml;nderungen anzeigen)</span>
    <div class="diff-actions">
      <button type="button" class="btn-reject" onclick={onreject}>&#x2715; Ablehnen</button>
      <button type="button" class="btn-accept" onclick={onaccept}>&#x2713; &Uuml;bernehmen</button>
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
          {#if line.type === "added"}+{:else if line.type === "removed"}&minus;{:else} {/if}
        </span>
        <span class="line-text">{line.content}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .diff-editor {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    overflow: hidden;
  }
  .diff-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
  }
  .diff-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-text-secondary);
  }
  .diff-actions {
    display: flex;
    gap: 8px;
  }
  .btn-accept,
  .btn-reject {
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid transparent;
  }
  .btn-accept {
    background: var(--color-success);
    color: white;
  }
  .btn-accept:hover {
    opacity: 0.9;
  }
  .btn-reject {
    background: var(--color-list);
    color: var(--color-danger);
    border-color: var(--color-danger);
  }
  .btn-reject:hover {
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
  }
  .diff-content {
    padding: 12px;
    font-family: "SF Mono", "Menlo", "Consolas", monospace;
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
    background: color-mix(in srgb, var(--color-success) 12%, transparent);
  }
  .diff-line.removed {
    background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  }
  .line-prefix {
    width: 16px;
    text-align: center;
    flex-shrink: 0;
    color: var(--color-text-secondary);
    user-select: none;
  }
  .added .line-prefix {
    color: var(--color-success);
  }
  .removed .line-prefix {
    color: var(--color-danger);
  }
  .line-text {
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
