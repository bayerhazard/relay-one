<script lang="ts">
  interface Props {
    suggestions: string[];
    onselect: (suggestion: string) => void;
  }

  let { suggestions = [], onselect }: Props = $props();
</script>

{#if suggestions.length > 0}
  <div class="reply-suggestions">
    <span class="suggestions-label">Antwortvorschl&auml;ge:</span>
    {#each suggestions as suggestion, i}
      <button type="button" class="suggestion-chip" onclick={() => onselect(suggestion)}>
        {suggestion}
      </button>
    {/each}
  </div>
{/if}

<style>
  .reply-suggestions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 0;
  }
  .suggestions-label {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    width: 100%;
    margin-bottom: 2px;
  }
  .suggestion-chip {
    padding: 4px 12px;
    border: 1px solid var(--color-border);
    border-radius: 16px;
    background: var(--color-list);
    font-size: 0.75rem;
    cursor: pointer;
    transition: all 0.15s ease;
    max-width: 280px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .suggestion-chip:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
    background: var(--color-preview);
  }
</style>
