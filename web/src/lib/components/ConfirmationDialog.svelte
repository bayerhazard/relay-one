  <script lang="ts">
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
      title = "Bestätigung",
      message,
      confirmLabel = "Bestätigen",
      cancelLabel = "Abbrechen",
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
      } else if (e.key === "Enter") {
        e.preventDefault();
        onconfirm();
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
    <!-- svelte-ignore a11y_autofocus -->
    <div
      class="dialog-overlay"
      role="alertdialog"
      aria-labelledby="dialog-title"
      aria-describedby="dialog-message"
      aria-modal="true"
      onkeydown={handleKeydown}
      onclick={handleBackdropClick}
    >
      <div class="dialog-panel" class:danger>
        <div class="dialog-content">
          <h2 id="dialog-title" class="dialog-title">{title}</h2>
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
              autofocus
            >
              {cancelLabel}
            </button>
            <button
              type="button"
              class="btn-confirm"
              class:danger
              onclick={onconfirm}
            >
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
      color: var(--color-text);
    }

    .dialog-message {
      font-size: 0.875rem;
      line-height: 1.5;
      color: var(--color-text-secondary);
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

    .btn-cancel:focus-visible {
      outline: 2px solid var(--color-accent);
      outline-offset: 2px;
    }

    .btn-alt {
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

    .btn-alt:hover {
      background: var(--color-sidebar);
      border-color: var(--color-text-secondary);
    }

    .btn-alt:focus-visible {
      outline: 2px solid var(--color-accent);
      outline-offset: 2px;
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

    .btn-confirm:hover {
      background: var(--color-accent-hover);
    }

    .btn-confirm:focus-visible {
      outline: 2px solid var(--color-accent);
      outline-offset: 2px;
    }

    .btn-confirm.danger {
      background: var(--color-danger);
    }

    .btn-confirm.danger:hover {
      opacity: 0.85;
    }
  </style>
