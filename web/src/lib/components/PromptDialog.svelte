<script lang="ts">
  // In-app text-input dialog. Replaces window.prompt(), which silently
  // returns null and renders nothing inside the Tauri macOS WKWebView.
  import { t } from "$lib/i18n";
  import Symbol from "./Symbol.svelte";

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
    title = "",
    message = "",
    value = "",
    placeholder = "",
    confirmLabel = "",
    cancelLabel = "",
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
    } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
      // Enter in the field confirms; on a focused button it activates that
      // button ("Abbrechen" must cancel, not confirm).
      e.preventDefault();
      confirm();
    }
  }

  function handleBackdropClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.classList.contains("dialog-schicht")) {
      oncancel();
    }
  }
</script>

<!-- HB-DIALOG, narrow: one field in `.feld`, footer with the confirming
     button first and "Abbrechen" after it. -->
{#if open}
  <div
    class="dialog-schicht"
    role="dialog"
    aria-labelledby="prompt-dialog-title"
    aria-modal="true"
    tabindex="-1"
    onkeydown={handleKeydown}
    onclick={handleBackdropClick}
  >
    <div class="karte dialog-karte" data-breite="schmal">
      <div class="dialog-kopf">
        <h2 id="prompt-dialog-title">{title || $t("prompt.title")}</h2>
        <button type="button" class="dialog-zu" aria-label="Schließen" title="Schließen" onclick={oncancel}>
          <Symbol name="schliessen" size={20} />
        </button>
      </div>
      <div class="dialog-koerper">
        <div class="feld">
          {#if message}
            <label for="prompt-dialog-input">{message}</label>
          {/if}
          <input
            id="prompt-dialog-input"
            type="text"
            bind:this={inputEl}
            bind:value={inputValue}
            {placeholder}
            aria-label={message ? undefined : title || $t("prompt.title")}
            onclick={(e) => e.stopPropagation()}
          />
        </div>
      </div>
      <div class="dialog-fuss">
        <button type="button" class="btn btn-primaer" onclick={confirm} disabled={!inputValue.trim()}>
          {confirmLabel || $t("prompt.ok")}
        </button>
        <button type="button" class="btn btn-sekundaer" onclick={oncancel}>
          {cancelLabel || $t("prompt.cancel")}
        </button>
      </div>
    </div>
  </div>
{/if}
