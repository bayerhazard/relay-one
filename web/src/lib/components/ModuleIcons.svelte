<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  let { active = "" }: { active?: string } = $props();
</script>

<div class="mi-row">
  <button type="button" class="mi-btn" class:active={active === "mail"} onclick={() => goto("/")} title={$t("mail.title")} aria-label={$t("mail.title")}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "contacts"} onclick={() => goto("/contacts")} title={$t("contacts.title")} aria-label={$t("contacts.title")}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "calendar"} onclick={() => goto("/calendar")} title={$t("calendar.title")} aria-label={$t("calendar.title")}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "meetings"} onclick={() => goto("/meetings")} title={$t("meetings.title")} aria-label={$t("meetings.title")}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "tasks"} onclick={() => goto("/tasks")} title={$t("tasks.title")} aria-label={$t("tasks.title")}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>
  </button>
</div>

<style>
  .mi-row {
    display: flex;
    align-items: center;
    gap: 6.4px;
  }
  .mi-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    background: none;
    border: none;
    border-radius: var(--radius-m);
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: background 0.15s ease-in-out, color 0.15s ease-in-out;
  }
  .mi-btn:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .mi-btn.active {
    background: var(--color-active-wash);
    color: var(--color-accent);
  }
</style>
