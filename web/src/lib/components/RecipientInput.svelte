<script lang="ts">
  import { searchContacts } from '$lib/services/tauri';
  import type { ContactInfo } from '$lib/services/tauri';

  let { value = $bindable([]), accountId, onchange }: { value: string[]; accountId: number | undefined; onchange?: (value: string[]) => void } = $props();

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
        <button type="button" class="chip-remove" onclick={() => removeRecipient(i)}>&times;</button>
      </span>
    {/each}
    <input
      type="text"
      autocomplete="new-password"
      spellcheck="false"
      bind:this={inputRef}
      value={query}
      oninput={handleInput}
      onkeydown={handleKeyDown}
      onfocus={() => { if (suggestions.length > 0) showDropdown = true; }}
      placeholder={value.length === 0 ? "Name oder E-Mail-Adresse" : ""}
      aria-label="Empfänger eingeben"
    />
    {#if loading}
      <span class="spinner">⏳</span>
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
  .recipient-input {
    position: relative;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-list);
    transition: all 0.15s ease-in-out;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .recipient-input:focus-within {
    border-color: var(--color-accent);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 5px 10px;
    min-height: 38px;
    box-sizing: border-box;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--color-active-wash);
    border: 1px solid color-mix(in srgb, var(--color-accent) 15%, transparent);
    border-radius: 6px;
    padding: 3px 8px 3px 1px;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--color-accent);
    user-select: none;
    transition: all 0.12s ease-in-out;
  }

  .chip:hover {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
  }

  .chip-remove {
    cursor: pointer;
    background: none;
    border: none;
    font-size: 0.9375rem;
    line-height: 1;
    color: var(--color-accent);
    opacity: 0.5;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all 0.12s ease;
  }

  .chip-remove:hover {
    opacity: 1;
    transform: scale(1.1);
  }

  .chips input {
    flex: 1;
    min-width: 120px;
    border: none;
    outline: none;
    font-size: 0.875rem;
    padding: 2px 0;
    background: transparent;
    color: var(--color-text);
  }
  .chips input:-webkit-autofill {
    -webkit-box-shadow: 0 0 0px 1000px var(--color-list) inset !important;
    -webkit-text-fill-color: var(--color-text) !important;
  }

  .spinner {
    font-size: 0.8em;
    opacity: 0.6;
    margin-right: 4px;
  }

  .suggestions {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    max-height: 200px;
    overflow-y: auto;
    z-index: 1000;
    box-shadow: none;
    padding: 4px 0;
  }

  .suggestion {
    padding: 8px 12px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: transparent;
    transition: background 0.1s ease;
  }

  .suggestion:hover,
  .suggestion:focus {
    background: var(--color-active-wash);
    outline: none;
  }

  .suggestion-name {
    font-weight: 600;
    font-size: 0.8125rem;
    color: var(--color-text);
  }

  .suggestion-email {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
  }
</style>