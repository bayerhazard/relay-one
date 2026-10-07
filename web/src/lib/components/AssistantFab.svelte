<script lang="ts">
  // Shared assistant entry point (HB-ASSISTENT, after Rocket's assistent.tsx):
  // the shield bottom right opens the panel above it. Drop into every module
  // page; `module` tells the assistant which module it was opened from (used
  // for module-aware navigation and the examples). Placement — also above the
  // bottom bar on the phone — comes from the CI CSS (.assistent-knopf).
  import AssistantDrawer from "./AssistantDrawer.svelte";
  import Schild from "./Schild.svelte";
  import { t } from "$lib/i18n";
  import { assistantCommand } from "$lib/stores/assistantCommand";
  import { fabHidden } from "$lib/stores/fabHidden";
  import type { AgentPlan } from "$lib/services/tauri";

  interface Props {
    module: "mail" | "calendar" | "contacts" | "tasks" | "meetings" | "settings";
    context?: string;
  }

  let { module, context = "" }: Props = $props();
  let open = $state(false);
  // The agent is running: the shield stops blinking (Rocket: !senden.isPending).
  let busy = $state(false);
  let knopf = $state<HTMLButtonElement | null>(null);
  // Phase C: a mail footer chip hands a pre-built plan to the drawer — open it
  // and inject the card (Concept §9.4).
  let externalPlan = $state<AgentPlan | null>(null);
  $effect(() => {
    const cmd = $assistantCommand;
    if (cmd) {
      externalPlan = cmd.plan;
      open = true;
    }
  });

  const label = $derived(open ? $t("assistant.closePanel") : $t("assistant.open"));
</script>

<!-- M6 (Review 2026-09-14): hidden while a mobile drawer/sheet is open. -->
{#if !$fabHidden}
  <button
    type="button"
    class="assistent-knopf"
    class:offen={open}
    bind:this={knopf}
    onclick={() => (open = !open)}
    aria-label={label}
    title={label}
    aria-expanded={open}
    aria-controls="assistent-panel"
  >
    <Schild size={42} zwinkert={!open && !busy} />
  </button>
{/if}
<AssistantDrawer
  {open}
  {module}
  {context}
  {externalPlan}
  returnFocus={knopf}
  bind:busy
  onclose={() => (open = false)}
/>
