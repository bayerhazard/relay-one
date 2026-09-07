<script lang="ts">
  // Shared assistant entry point: floating action button + slide-in drawer.
  // Drop into every module page; `module` tells the assistant which module it
  // was opened from (used for module-aware navigation).
  import AssistantDrawer from "./AssistantDrawer.svelte";
  import { t } from "$lib/i18n";

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
  title={$t("assistant.title")}
  aria-label={$t("assistant.open")}
>
  <!-- Shield silhouette from the app icon (icon.svg), flat gold fill. -->
  <svg viewBox="40.46 30.4 79.08 99.2" width="52" height="52" aria-hidden="true" focusable="false">
    <path
      d="M80 129.6C68.551 126.707 59.0993 120.114 51.6451 109.822C44.1909 99.5299 40.4638 88.1012 40.4638 75.5359V45.2799L80 30.3999L119.536 45.2799V75.5359C119.536 88.1012 115.809 99.5299 108.355 109.822C100.901 120.114 91.449 126.707 80 129.6Z"
      fill="var(--gold)"
    />
  </svg>
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
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
    z-index: 900;
    opacity: 0.55;
    transition: opacity 150ms ease;
  }
  .assistant-fab:hover {
    opacity: 1;
  }
  .assistant-fab:focus-visible {
    outline: var(--fokus-ring);
    outline-offset: 2px;
    border-radius: 8px;
    opacity: 1;
  }
</style>
