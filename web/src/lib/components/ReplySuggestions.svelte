<script lang="ts">
  interface Props {
    suggestions: string[];
    onselect: (suggestion: string) => void;
  }

  import { t } from "$lib/i18n";

  let { suggestions = [], onselect }: Props = $props();
</script>

{#if suggestions.length > 0}
  <div class="reply-suggestions">
    <span class="suggestions-label">{$t("reply.suggestions")}</span>
    {#each suggestions as suggestion, i}
      <button type="button" class="suggestion-chip" onclick={() => onselect(suggestion)}>
        {suggestion}
      </button>
    {/each}
  </div>
{/if}

<style>
  /* ── Reply suggestions [RL-VORSCHLAEGE] ────────────────────────────────────
     One-tap reply chips under a message — chips, Relay's own (not AM-KNOPF). */
  .reply-suggestions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 0;
  }
  .suggestions-label {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
    width: 100%;
    margin-bottom: 2px;
  }
  .suggestion-chip {
    padding: 4px 12px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-seite);
    color: var(--am-text-primaer);
    font-family: inherit;
    font-size: 0.75rem;
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
    max-width: 280px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .suggestion-chip:hover {
    border-color: var(--am-handlung-ruhend);
    color: var(--am-handlung-ruhend);
    background: var(--am-seite);
  }
  .suggestion-chip:focus-visible {
    outline: 2px solid var(--am-fokus-ring);
    outline-offset: 2px;
  }
</style>
