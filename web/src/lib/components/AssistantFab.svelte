<script lang="ts">
  // Shared assistant entry point: floating action button + slide-in drawer.
  // Drop into every module page; `module` tells the assistant which module it
  // was opened from (used for module-aware navigation).
  import AssistantDrawer from "./AssistantDrawer.svelte";

  interface Props {
    module: "mail" | "calendar" | "contacts" | "tasks" | "settings";
    context?: string;
  }

  let { module, context = "" }: Props = $props();
  let open = $state(false);
</script>

<button
  type="button"
  class="assistant-fab"
  onclick={() => (open = true)}
  title="Assistent"
  aria-label="Assistent öffnen"
>
  ✦
</button>
<AssistantDrawer
  open={open}
  {module}
  {context}
  onclose={() => (open = false)}
/>

<style>
  .assistant-fab {
    position: fixed;
    bottom: 20px;
    right: 20px;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    border: none;
    background: var(--color-accent);
    color: #fff;
    font-size: 1.4rem;
    cursor: pointer;
    z-index: 900;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
  }
  .assistant-fab:hover {
    filter: brightness(1.1);
  }
</style>
