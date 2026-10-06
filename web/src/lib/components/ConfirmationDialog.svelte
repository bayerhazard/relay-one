  <script lang="ts">
    import { t } from "$lib/i18n";

    interface Props {
      open: boolean;
      title?: string;
      message: string;
      confirmLabel?: string;
      cancelLabel?: string;
      altLabel?: string;
      onconfirm: () => void;
      oncancel: () => void;
      onalt?: () => void;
      danger?: boolean;
    }

    let {
      open,
      title = "",
      message,
      confirmLabel = "",
      cancelLabel = "",
      altLabel = "",
      onconfirm,
      oncancel,
      onalt,
      danger = false,
    }: Props = $props();

    function handleKeydown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        oncancel();
      } else if (e.key === "Enter" && !danger) {
        // A destructive question must not confirm on Enter: focus starts on
        // "Abbrechen" (CI HB-DIALOG), so Enter activates whatever is focused.
        e.preventDefault();
        onconfirm();
      }
    }

    // `autofocus` does not fire for a dialog that appears later, so focus
    // moves to "Abbrechen" explicitly — keyboard and screen reader land in
    // the dialog, and Enter never confirms by accident (CI HB-DIALOG).
    let cancelButton = $state<HTMLButtonElement | null>(null);
    $effect(() => {
      if (open && cancelButton) cancelButton.focus();
    });

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
      role="alertdialog"
      tabindex="-1"
      aria-labelledby="dialog-title"
      aria-describedby="dialog-message"
      aria-modal="true"
      onkeydown={handleKeydown}
      onclick={handleBackdropClick}
    >
      <div class="dialog-panel" class:danger>
        <div class="dialog-content">
          <h2 id="dialog-title" class="dialog-title">{title || $t("confirmation.title")}</h2>
          <p id="dialog-message" class="dialog-message">{message}</p>
          <div class="dialog-actions">
            {#if altLabel && onalt}
              <button type="button" class="btn-alt" onclick={onalt}>
                {altLabel}
              </button>
            {/if}
            <button
              type="button"
              class="btn-cancel"
              onclick={oncancel}
              bind:this={cancelButton}
            >
              {cancelLabel || $t("common.cancel")}
            </button>
            <button
              type="button"
              class="btn-confirm"
              class:danger
              onclick={onconfirm}
            >
              {confirmLabel || $t("confirmation.confirm")}
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
      background: var(--am-deckschicht);
      animation: fadeIn 0.15s ease-out;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    /* A dialog floats: surface 3, emphasised border, the one shadow (CI R6). */
    .dialog-panel {
      background: var(--am-flaeche-3);
      border: 1px solid var(--am-rand-betont-farbe);
      border-radius: var(--am-radius-gross);
      box-shadow: var(--am-schatten-1);
      max-width: 400px;
      width: 90vw;
      animation: panelIn 0.15s ease-out;
    }

    @keyframes panelIn {
      from {
        opacity: 0;
        transform: scale(0.96) translateY(-8px);
      }
      to {
        opacity: 1;
        transform: scale(1) translateY(0);
      }
    }

    .dialog-content {
      padding: 24px;
    }

    .dialog-title {
      font-size: 1.125rem;
      font-weight: 700;
      margin-bottom: 12px;
      color: var(--am-text-primaer);
    }

    .dialog-message {
      font-size: 0.875rem;
      line-height: 1.5;
      color: var(--am-text-gedaempft);
      margin-bottom: 24px;
    }

    .dialog-actions {
      display: flex;
      justify-content: flex-end;
      gap: 10px;
      flex-wrap: wrap;
    }

    .btn-cancel {
      padding: 8px 18px;
      border: 1px solid var(--am-rand);
      border-radius: var(--am-radius-mittel);
      background: var(--am-seite);
      color: var(--am-text-primaer);
      font-size: 0.8125rem;
      font-weight: 600;
      cursor: pointer;
      transition: all var(--am-dauer-schnell) var(--am-kurve);
    }

    .btn-cancel:hover {
      background: var(--am-flaeche-1);
      border-color: var(--am-text-gedaempft);
    }

    .btn-cancel:focus-visible {
      outline: 2px solid var(--am-handlung-ruhend);
      outline-offset: 2px;
    }

    .btn-alt {
      padding: 8px 18px;
      border: 1px solid var(--am-rand);
      border-radius: var(--am-radius-mittel);
      background: var(--am-seite);
      color: var(--am-text-primaer);
      font-size: 0.8125rem;
      font-weight: 600;
      cursor: pointer;
      transition: all var(--am-dauer-schnell) var(--am-kurve);
    }

    .btn-alt:hover {
      background: var(--am-flaeche-1);
      border-color: var(--am-text-gedaempft);
    }

    .btn-alt:focus-visible {
      outline: 2px solid var(--am-handlung-ruhend);
      outline-offset: 2px;
    }

    .btn-confirm {
      padding: 8px 18px;
      border: none;
      border-radius: var(--am-radius-mittel);
      background: var(--am-handlung-ruhend);
      color: var(--am-handlung-text);
      font-size: 0.8125rem;
      font-weight: 600;
      cursor: pointer;
      transition: all var(--am-dauer-schnell) var(--am-kurve);
    }

    .btn-confirm:hover {
      background: var(--am-handlung-hover);
    }

    .btn-confirm:focus-visible {
      outline: 2px solid var(--am-handlung-ruhend);
      outline-offset: 2px;
    }

    .btn-confirm.danger {
      background: var(--am-fehler);
    }

    .btn-confirm.danger:hover {
      background: var(--am-fehler-hover);
    }
  </style>
