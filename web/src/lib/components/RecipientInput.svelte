<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { searchContacts } from '$lib/services/tauri';
  import type { ContactInfo } from '$lib/services/tauri';
  import { t } from '$lib/i18n';

  // `id` goes onto the inner input so a surrounding `<label for>` reaches it.
  let { value = $bindable([]), accountId, onchange, id }: { value: string[]; accountId: number | undefined; onchange?: (value: string[]) => void; id?: string } = $props();

  let inputRef: HTMLInputElement;
  let query = $state('');
  let suggestions = $state<ContactInfo[]>([]);
  let showDropdown = $state(false);
  let loading = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let blurTimer: ReturnType<typeof setTimeout> | null = null;


  // Cancel pending timers on unmount to avoid state updates on a dead component.
  $effect(() => {
    return () => {
      if (debounceTimer) clearTimeout(debounceTimer);
      if (blurTimer) clearTimeout(blurTimer);
    };
  });

  function handleInput() {
    query = inputRef.value;
    showDropdown = false;

    if (debounceTimer) clearTimeout(debounceTimer);
    if (query.trim().length < 2) {
      suggestions = [];
      return;
    }

    loading = true;
    debounceTimer = setTimeout(async () => {
      const q = query.trim();
      try {
        const results = await searchContacts(q);
        // Discard stale results if the query changed while awaiting.
        if (q !== query.trim()) return;
        suggestions = results;
        showDropdown = suggestions.length > 0;
      } catch {
        if (q === query.trim()) suggestions = [];
      } finally {
        if (q === query.trim()) loading = false;
      }
    }, 300);
  }

  function selectContact(contact: ContactInfo) {
    const email = contact.email || '';
    if (blurTimer) clearTimeout(blurTimer);
    blurTimer = null;
    if (!email || value.some(v => v.toLowerCase() === email.toLowerCase())) return;

    value = [...value, email];
    inputRef.value = '';
    query = '';
    showDropdown = false;
    suggestions = [];
    onchange?.(value);
  }

  function removeRecipient(idx: number) {
    value = value.filter((_, i) => i !== idx);
    onchange?.(value);
  }

  function commitQuery() {
    const trimmed = query.trim();
    if (!trimmed) return;
    if (!value.some(v => v.toLowerCase() === trimmed.toLowerCase())) {
      value = [...value, trimmed];
    }
    inputRef.value = '';
    query = '';
    showDropdown = false;
    suggestions = [];
    onchange?.(value);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (suggestions.length > 0) {
        selectContact(suggestions[0]);
      } else {
        commitQuery();
      }
    }
    if (e.key === 'Tab' || e.key === ',') {
      if (query.trim()) {
        e.preventDefault();
        commitQuery();
      }
    }
    if (e.key === 'Escape') {
      // An open suggestion list takes the Escape; otherwise it reaches the
      // dialog around the field and closes that.
      if (showDropdown) e.stopPropagation();
      showDropdown = false;
    }
    if (e.key === 'Backspace' && query === '' && value.length > 0) {
      removeRecipient(value.length - 1);
    }
  }

  function handleBlur() {
    if (blurTimer) clearTimeout(blurTimer);
    const qAtBlur = query.trim();
    const dropdownOpen = showDropdown;
    blurTimer = setTimeout(() => {
      showDropdown = false;
      // Don't commit on blur when dropdown was open — user is picking a suggestion.
      if (dropdownOpen) return;
      if (qAtBlur && query.trim() === qAtBlur) commitQuery();
    }, 150);
  }

  function formatDisplay(contact: ContactInfo): string {
    if (contact.display_name) return contact.display_name;
    if (contact.email) return contact.email;
    return 'Unbekannt';
  }
</script>

<div class="recipient-input" class:active={showDropdown} onfocusout={handleBlur}>
  <div class="chips">
    {#each value as email, i (email)}
      <span class="chip">
        {email}
        <button type="button" class="chip-remove" onclick={() => removeRecipient(i)} title="{$t("mail.fmtRemove")}: {email}" aria-label="{$t("mail.fmtRemove")}: {email}">
          <Symbol name="schliessen" size={16} />
        </button>
      </span>
    {/each}
    <input
      {id}
      type="text"
      autocomplete="new-password"
      spellcheck="false"
      bind:this={inputRef}
      value={query}
      oninput={handleInput}
      onkeydown={handleKeyDown}
      onfocus={() => { if (suggestions.length > 0) showDropdown = true; }}
      placeholder={value.length === 0 ? $t("recipient.placeholder") : ""}
      aria-label={$t("recipient.aria")}
    />
    {#if loading}
      <span class="spinner"><Symbol name="laden" size={16} /></span>
    {/if}
  </div>

  {#if showDropdown && suggestions.length > 0}
    <div class="suggestions">
      {#each suggestions as contact}
        <div
          class="suggestion"
          role="option"
          aria-selected={false}
          tabindex="-1"
          onclick={() => selectContact(contact)}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              selectContact(contact);
            }
          }}
        >
          <span class="suggestion-name">{formatDisplay(contact)}</span>
          {#if contact.email}
            <span class="suggestion-email">{contact.email}</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* ── Recipient field with chips [RL-EMPFAENGER] ──────────────────────────────
     A composite input (chips + free text + suggestions), Relay's own. The
     box matches an AM-FELD input: same height, rand, radius and focus ring. */
  .recipient-input {
    position: relative;
    border: var(--am-rand-ruhend) solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-seite);
    transition: all var(--am-dauer-schnell) var(--am-kurve);
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .recipient-input:focus-within {
    outline: 2px solid var(--am-fokus-ring);
    outline-offset: 2px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 4px 10px;
    min-height: calc(var(--am-ziel-zeiger) - 2 * var(--am-rand-ruhend));
    box-sizing: border-box;
  }

  /* ── Chips [RL-EMPFAENGER] ─────────────────────────────────────────────────── */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--am-flaeche-2);
    border: 1px solid color-mix(in srgb, var(--am-handlung-ruhend) 15%, transparent);
    border-radius: 6px;
    padding: 3px 8px 3px 1px;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--am-handlung-ruhend);
    user-select: none;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }

  .chip:hover {
    background: color-mix(in srgb, var(--am-handlung-ruhend) 12%, transparent);
  }

  /* Part of the chip, not an AM-KNOPF: a 40 px button would burst the chip. */
  .chip-remove {
    cursor: pointer;
    background: none;
    border: none;
    line-height: 1;
    color: var(--am-handlung-ruhend);
    opacity: 0.5;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }

  .chip-remove:hover,
  .chip-remove:focus-visible {
    opacity: 1;
  }

  .chip-remove:focus-visible {
    outline: 2px solid var(--am-fokus-ring);
    outline-offset: 1px;
  }

  /* The free-text part is borderless inside the box. It sits inside `.feld`
     in the compose window, so AM-FELD's input rules are reset here. */
  .chips input {
    flex: 1;
    min-width: 120px;
    min-height: 0;
    width: auto;
    border: none;
    border-radius: 0;
    outline: none;
    font-size: 0.875rem;
    padding: 2px 0;
    background: transparent;
    color: var(--am-text-primaer);
  }
  .chips input:-webkit-autofill {
    -webkit-box-shadow: 0 0 0px 1000px var(--am-seite) inset !important;
    -webkit-text-fill-color: var(--am-text-primaer) !important;
  }
  .chips input::placeholder { color: var(--am-text-deaktiviert); }

  .spinner {
    display: inline-flex;
    opacity: 0.6;
    margin-right: 4px;
  }

  /* ── Contact suggestions [RL-EMPFAENGER] ───────────────────────────────────── */
  .suggestions {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    background: var(--am-seite);
    border: 1px solid var(--am-rand-betont-farbe);
    border-radius: var(--am-radius-mittel);
    max-height: 200px;
    overflow-y: auto;
    z-index: 1000;
    box-shadow: var(--am-schatten-1);
    padding: 4px 0;
  }

  .suggestion {
    padding: 8px 12px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: transparent;
    transition: background var(--am-dauer-schnell) var(--am-kurve);
  }

  .suggestion:hover,
  .suggestion:focus {
    background: var(--am-flaeche-2);
    outline: none;
  }

  .suggestion-name {
    font-weight: 600;
    font-size: 0.8125rem;
    color: var(--am-text-primaer);
  }

  .suggestion-email {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
  }
</style>
