<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  let { active = "" }: { active?: string } = $props();
</script>

<div class="mi-row">
  <button type="button" class="mi-btn" class:active={active === "mail"} onclick={() => goto("/")} title={$t("mail.title")} aria-label={$t("mail.title")}>
    <Symbol name="post" size={16} />
  </button>
  <button type="button" class="mi-btn" class:active={active === "contacts"} onclick={() => goto("/contacts")} title={$t("contacts.title")} aria-label={$t("contacts.title")}>
    <Symbol name="team" size={16} />
  </button>
  <button type="button" class="mi-btn" class:active={active === "calendar"} onclick={() => goto("/calendar")} title={$t("calendar.title")} aria-label={$t("calendar.title")}>
    <Symbol name="kalender" size={16} />
  </button>
  <button type="button" class="mi-btn" class:active={active === "meetings"} onclick={() => goto("/meetings")} title={$t("meetings.title")} aria-label={$t("meetings.title")}>
    <Symbol name="besprechung" size={16} />
  </button>
  <button type="button" class="mi-btn" class:active={active === "tasks"} onclick={() => goto("/tasks")} title={$t("tasks.title")} aria-label={$t("tasks.title")}>
    <Symbol name="aufgabe" size={16} />
  </button>
</div>

<style>
  /* ── Module switcher [RL-BEREICHSWECHSEL] ─────────────────────────────────
     A row of nav items, deliberately not AM-KNOPF: the active module is a
     selection state that buttons do not have. */
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
    border-radius: var(--am-radius-mittel);
    color: var(--am-text-gedaempft);
    cursor: pointer;
    transition: background var(--am-dauer-schnell) var(--am-kurve), color var(--am-dauer-schnell) var(--am-kurve);
  }
  .mi-btn:hover {
    background: var(--am-flaeche-2);
    color: var(--am-text-primaer);
  }
  .mi-btn:focus-visible {
    outline: 2px solid var(--am-fokus-ring);
    outline-offset: 2px;
  }
  .mi-btn.active {
    background: var(--am-flaeche-2);
    color: var(--am-handlung-ruhend);
  }
</style>
