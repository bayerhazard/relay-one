<script lang="ts">
  // Unified empty / informational state used across the mailbox (empty folder,
  // no selection, no search results, offline). Keeps the visual language
  // consistent: centered icon + title + optional subtitle + optional action.
  interface Props {
    icon?: string;
    title: string;
    subtitle?: string;
    actionLabel?: string;
    onaction?: () => void;
    tone?: "neutral" | "error";
    offsetHeader?: boolean;
  }

  let {
    icon = "",
    title,
    subtitle = "",
    actionLabel = "",
    onaction,
    tone = "neutral",
    offsetHeader = false,
  }: Props = $props();

  let iconName = $derived.by(() => {
    if (!icon) return "";
    const clean = icon.trim();
    const cp = clean.codePointAt(0);

    // Envelope check (✉ is 0x2709, or keyword, or starts with envelope)
    if (cp === 0x2709 || clean.includes("envelope") || clean.includes("✉") || clean.includes("2709")) return "envelope";

    // Inbox check (📬 is 0x1F4ED, 📭 is 0x1F4EB, 📪 is 0x1F4EA, 📩 is 0x1F4E9, 📥 is 0x1F4E5, or keywords)
    if (
      cp === 0x1F4ED ||
      cp === 0x1F4EB ||
      cp === 0x1F4EA ||
      cp === 0x1F4E9 ||
      cp === 0x1F4E5 ||
      clean.includes("inbox") ||
      clean.includes("📬") ||
      clean.includes("📭") ||
      clean.includes("📪") ||
      clean.includes("📥") ||
      clean.includes("1F4ED") ||
      clean.includes("1F4EB") ||
      clean.includes("1F4EA")
    ) {
      return "inbox";
    }

    // Search check (🔍 is 0x1F50D, or keyword search)
    if (cp === 0x1F50D || clean.includes("search") || clean.includes("🔍") || clean.includes("1F50D")) return "search";

    // Warning check (⚠ is 0x26A0, or warning)
    if (cp === 0x26A0 || clean.includes("warning") || clean.includes("⚠") || clean.includes("26A0")) return "warning";

    return "";
  });
</script>

<div class="empty-state" class:error={tone === "error"} class:offset-header={offsetHeader}>
  {#if icon}
    {#if iconName}
      <div class="empty-state-icon-wrapper" aria-hidden="true">
        {#if iconName === "envelope"}
          <svg class="empty-state-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="4" width="20" height="16" rx="2" />
            <path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" />
          </svg>
        {:else if iconName === "inbox"}
          <svg class="empty-state-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2 10v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V10" />
            <path d="M2 10l10-6 10 6" />
            <path d="M2 10l10 7 10-7" />
          </svg>
        {:else if iconName === "search"}
          <svg class="empty-state-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
        {:else if iconName === "warning"}
          <svg class="empty-state-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
            <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        {/if}
      </div>
    {:else}
      <div class="empty-state-icon" aria-hidden="true">{icon}</div>
    {/if}
  {/if}
  <p class="empty-state-title">{title}</p>
  {#if subtitle}
    <p class="empty-state-subtitle">{subtitle}</p>
  {/if}
  {#if actionLabel && onaction}
    <button type="button" class="empty-state-action" onclick={onaction}>
      {actionLabel}
    </button>
  {/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 48px 32px;
    text-align: center;
    color: var(--color-text-secondary);
    gap: 8px;
  }
  .empty-state.offset-header {
    padding-top: calc(72px + 48px);
  }
  .empty-state-icon-wrapper {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: var(--color-active-wash);
    color: var(--color-accent);
    margin-bottom: 12px;
    transition: all 0.2s ease-in-out;
  }
  .empty-state-svg {
    width: 28px;
    height: 28px;
    stroke-width: 1.5;
  }
  .empty-state.error .empty-state-icon-wrapper {
    background: color-mix(in srgb, var(--color-danger) 10%, transparent);
    color: var(--color-danger);
  }
  .empty-state-icon {
    font-size: 2.5rem;
    margin-bottom: 12px;
    opacity: 0.35;
    line-height: 1;
  }
  .empty-state-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--color-text);
    margin: 0;
  }
  .empty-state-subtitle {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    margin: 0;
    max-width: 280px;
    line-height: 1.5;
  }
  .empty-state-action {
    margin-top: 16px;
    padding: 8px 18px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-list);
    color: var(--color-text);
    font-size: 0.8125rem;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .empty-state-action:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
    background: var(--color-active-wash);
  }
  .empty-state.error .empty-state-action:hover {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }
</style>
