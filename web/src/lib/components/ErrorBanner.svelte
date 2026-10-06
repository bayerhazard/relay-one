  <script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
    import { t } from "$lib/i18n";

    interface Props {
      message: string;
      onretry?: () => void;
      retryLabel?: string;
    }

    let {
      message,
      onretry,
      retryLabel = "",
    }: Props = $props();
  </script>

  <!-- HB-ZUSTAND: the error notice line, sign and sentence. `error-banner`
       is only the layout hook (and what other tests look for). -->
  <div class="hinweis error-banner" data-art="fehler" role="alert" aria-live="polite">
    <Symbol name="achtung" size={16} />
    <span class="error-text">{message}</span>
    {#if onretry}
      <button type="button" class="btn btn-sekundaer btn-klein retry-btn" onclick={onretry}>
        {retryLabel || $t("error.retry")}
      </button>
    {/if}
  </div>

  <style>
    /* ── Error notice [RL-FEHLERHINWEIS] ────────────────────────────────────── */
    .error-banner {
      align-items: center;
      margin: var(--am-raum-2) var(--am-raum-3);
    }

    .error-text {
      flex: 1;
      min-width: 0;
      word-break: break-word;
    }

    .retry-btn {
      flex-shrink: 0;
    }
  </style>
