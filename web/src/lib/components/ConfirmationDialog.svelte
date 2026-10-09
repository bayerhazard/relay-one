  <script lang="ts">
    import type { Snippet } from "svelte";
    import { t } from "$lib/i18n";

    interface Props {
      open: boolean;
      title?: string;
      message: string;
      confirmLabel?: string;
      cancelLabel?: string;
      onconfirm: () => void;
      oncancel: () => void;
      danger?: boolean;
      /** Enter confirms. Off for anything that cannot be taken back, also when
       * it is not red (e.g. "Abo beenden"): Enter then activates the focus. */
      enterConfirms?: boolean;
      /** More under the question (e.g. a choice to tick). */
      children?: Snippet;
    }

    let {
      open,
      title = "",
      message,
      confirmLabel = "",
      cancelLabel = "",
      onconfirm,
      oncancel,
      danger = false,
      enterConfirms = !danger,
      children,
    }: Props = $props();

    function handleKeydown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        oncancel();
      } else if (e.key === "Enter" && enterConfirms) {
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
      if (target.classList.contains("dialog-schicht")) {
        oncancel();
      }
    }
  </script>

  <!-- HB-DIALOG, the Rückfrage: confirming button first, "Abbrechen" quiet
       after it and focused, as in Rocket's dialog.tsx. -->
  {#if open}
    <div
      class="dialog-schicht"
      role="alertdialog"
      tabindex="-1"
      aria-labelledby="dialog-title"
      aria-describedby="dialog-message"
      aria-modal="true"
      onkeydown={handleKeydown}
      onclick={handleBackdropClick}
    >
      <div class="karte rueckfrage">
        <h2 id="dialog-title">{title || $t("confirmation.title")}</h2>
        <p id="dialog-message" class="rueckfrage-text">{message}</p>
        {@render children?.()}
        <div class="btn-reihe">
          <button type="button" class="btn {danger ? 'btn-gefahr' : 'btn-primaer'}" onclick={onconfirm}>
            {confirmLabel || $t("confirmation.confirm")}
          </button>
          <button type="button" class="btn btn-still" onclick={oncancel} bind:this={cancelButton}>
            {cancelLabel || $t("common.cancel")}
          </button>
        </div>
      </div>
    </div>
  {/if}
