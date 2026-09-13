<script lang="ts">
  // Wiederverwendbares Rechtsklick-Kontextmenü (T3, Review 2026-09-13).
  // Optik = Goldstandard des Mail-Moduls (`.ctx-menu`), als kontrollierte
  // Komponente für alle Module. Long-Press bleibt Modulsache (Mail hat das
  // ausgefeilte Touch-Pattern); diese Komponente bedient den Desktop-Fall
  // (contextmenu → preventDefault → {x,y} übergeben).
  interface MenuItem {
    label: string;
    danger?: boolean;
    action: () => void;
  }
  interface Props {
    /** `{x,y}` des contextmenu-Events, oder `null` = geschlossen. */
    menu: { x: number; y: number } | null;
    items: MenuItem[];
    onclose: () => void;
  }
  let { menu, items, onclose }: Props = $props();

  // Viewport-Clamp: Menü nicht über den rechten/unteren Rand ragen lassen.
  let x = $derived.by(() =>
    Math.min(
      Math.max(menu?.x ?? 0, 8),
      (typeof window !== "undefined" ? window.innerWidth : 1440) - 240,
    ),
  );
  let y = $derived.by(() =>
    Math.min(
      Math.max(menu?.y ?? 0, 8),
      (typeof window !== "undefined" ? window.innerHeight : 900) - 80,
    ),
  );

  // Escape schließt das Menü global (wie das Mail-Kontextmenü).
  $effect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onclose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

{#if menu}
  <div
    class="ctx-scrim"
    role="presentation"
    onclick={onclose}
    oncontextmenu={(e) => { e.preventDefault(); onclose(); }}
  >
    <div
      class="ctx-pop"
      role="menu"
      style="left: {x}px; top: {y}px;"
      onclick={(e) => e.stopPropagation()}
    >
      {#each items as it (it.label)}
        <button type="button" class="ctx-item" class:danger={it.danger} role="menuitem"
                onclick={() => { it.action(); onclose(); }}>
          {it.label}
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .ctx-scrim {
    position: fixed;
    inset: 0;
    z-index: 1000;
  }
  .ctx-pop {
    position: fixed;
    z-index: 1001;
    min-width: 200px;
    max-width: 320px;
    max-height: min(70vh, 560px);
    overflow-y: auto;
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ctx-item {
    text-align: left;
    padding: var(--am-raum-2) var(--am-raum-4);
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 0.875rem;
    cursor: pointer;
    color: var(--color-text);
    white-space: nowrap;
    font-family: inherit;
  }
  .ctx-item:hover { background: var(--color-active-wash); }
  .ctx-item.danger { color: var(--color-danger, #c0392b); }
</style>
