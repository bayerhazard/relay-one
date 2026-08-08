  <script lang="ts">
    interface Props {
      message: string;
      onretry?: () => void;
      retryLabel?: string;
    }

    let {
      message,
      onretry,
      retryLabel = "Erneut versuchen",
    }: Props = $props();
  </script>

  <div class="error-banner" role="alert" aria-live="polite">
    <div class="error-banner-body">
      <span class="error-icon">&#x26A0;</span>
      <p class="error-text">{message}</p>
    </div>
    {#if onretry}
      <button type="button" class="retry-btn" onclick={onretry}>
        {retryLabel}
      </button>
    {/if}
  </div>

  <style>
    .error-banner {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      padding: 10px 14px;
      margin: 8px 12px;
      background: color-mix(in srgb, var(--color-danger) 10%, transparent);
      border: 1px solid color-mix(in srgb, var(--color-danger) 25%, transparent);
      border-radius: 8px;
      animation: bannerIn 0.2s ease-out;
    }

    @keyframes bannerIn {
      from {
        opacity: 0;
        transform: translateY(-6px);
      }
      to {
        opacity: 1;
        transform: translateY(0);
      }
    }

    .error-banner-body {
      display: flex;
      align-items: flex-start;
      gap: 8px;
      flex: 1;
      min-width: 0;
    }

    .error-icon {
      font-size: 0.875rem;
      flex-shrink: 0;
      line-height: 1.4;
    }

    .error-text {
      font-size: 0.75rem;
      color: var(--color-danger);
      line-height: 1.4;
      word-break: break-word;
      margin: 0;
    }

    .retry-btn {
      flex-shrink: 0;
      padding: 5px 14px;
      border: 1px solid var(--color-danger);
      border-radius: 6px;
      background: var(--color-list);
      color: var(--color-danger);
      font-size: 0.6875rem;
      font-weight: 600;
      cursor: pointer;
      white-space: nowrap;
      transition: all 0.15s ease-in-out;
    }

    .retry-btn:hover {
      background: color-mix(in srgb, var(--color-danger) 10%, transparent);
    }

    .retry-btn:focus-visible {
      outline: 2px solid var(--color-danger);
      outline-offset: 2px;
    }
  </style>
