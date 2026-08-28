<script lang="ts">
  import { goto } from "$app/navigation";
  let { active = "" }: { active?: string } = $props();
</script>

<div class="mi-row">
  <button type="button" class="mi-btn" class:active={active === "calendar"} onclick={() => goto("/calendar")} title="Kalender" aria-label="Kalender">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "contacts"} onclick={() => goto("/contacts")} title="Kontakte" aria-label="Kontakte">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/></svg>
  </button>
  <button type="button" class="mi-btn" class:active={active === "tasks"} onclick={() => goto("/tasks")} title="Aufgaben" aria-label="Aufgaben">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>
  </button>
</div>

<style>
  .mi-row {
    display: flex;
    align-items: center;
    gap: 8px;
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
