<script lang="ts">
  // In-app text-input dialog. Replaces window.prompt(), which silently
  // returns null and renders nothing inside the Tauri macOS WKWebView.
  interface Props {
    open: boolean;
    title?: string;
    message?: string;
    value?: string;
    placeholder?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    onconfirm: (value: string) => void;
    oncancel: () => void;
  }

  let {
    open,
    title = "Eingabe",
    message = "",
    value = "",
    placeholder = "",
    confirmLabel = "OK",
    cancelLabel = "Abbrechen",
    onconfirm,
    oncancel,
  }: Props = $props();

  let inputValue = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  // Reset the field each time the dialog opens.
  $effect(() => {
    if (open) {
      inputValue = value;
      requestAnimationFrame(() => {
        inputEl?.focus();
        inputEl?.select();
      });
    }
  });

  function confirm() {
    const trimmed = inputValue.trim();
    if (trimmed) onconfirm(trimmed);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    } else if (e.key === "Enter") {
      e.preventDefault();
      confirm();
    }
  }

  function handleBackdropClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.classList.contains("dialog-overlay")) {
      oncancel();
    }
  }
</script>

{#if open}
  <div
    class="dialog-overlay"
    role="dialog"
    aria-labelledby="prompt-dialog-title"
    aria-modal="true"
    tabindex="-1"
    onkeydown={handleKeydown}
    onclick={handleBackdropClick}
  >
    <div class="dialog-panel">
      <div class="dialog-content">
        <h2 id="prompt-dialog-title" class="dialog-title">{title}</h2>
        {#if message}
          <p class="dialog-message">{message}</p>
        {/if}
        <input
          type="text"
          class="dialog-input"
          bind:this={inputEl}
          bind:value={inputValue}
          {placeholder}
          onclick={(e) => e.stopPropagation()}
        />
        <div class="dialog-actions">
          <button type="button" class="btn-cancel" onclick={oncancel}>
            {cancelLabel}
          </button>
          <button type="button" class="btn-confirm" onclick={confirm} disabled={!inputValue.trim()}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
    animation: fadeIn 0.15s ease-out;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .dialog-panel {
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    box-shadow: none;
    max-width: 420px;
    width: 90vw;
    animation: panelIn 0.15s ease-out;
  }

  @keyframes panelIn {
    from { opacity: 0; transform: scale(0.96) translateY(-8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .dialog-content {
    padding: 24px;
  }

  .dialog-title {
    font-size: 1.125rem;
    font-weight: 700;
    margin-bottom: 12px;
    color: var(--color-text);
  }

  .dialog-message {
    font-size: 0.875rem;
    line-height: 1.5;
    color: var(--color-text-secondary);
    margin-bottom: 16px;
  }

  .dialog-input {
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    font-size: 0.9375rem;
    color: var(--color-text);
    background: var(--color-list);
    box-sizing: border-box;
    margin-bottom: 24px;
  }

  .dialog-input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 15%, transparent);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }

  .btn-cancel {
    padding: 8px 18px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-list);
    color: var(--color-text);
    font-size: 0.8125rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }

  .btn-cancel:hover {
    background: var(--color-sidebar);
    border-color: var(--color-text-secondary);
  }

  .btn-confirm {
    padding: 8px 18px;
    border: none;
    border-radius: 6px;
    background: var(--color-accent);
    color: #ffffff;
    font-size: 0.8125rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }

  .btn-confirm:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-confirm:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
