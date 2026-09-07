<script lang="ts">
  // A clickable entity reference in an assistant answer (Concept §10.3). Clicking
  // sets the cross-module selection store and navigates to the owning module —
  // the target page reads the selection on mount and highlights the item.
  import { goto } from "$app/navigation";
  import { selection } from "$lib/stores/selection";

  interface Props {
    art: "mail" | "contact" | "task" | "event";
    id: string;
    name: string;
    onOpen?: (art: string, id: string) => void;
  }

  let { art, id, name, onOpen }: Props = $props();

  const route = $derived(
    art === "mail" ? "/" : art === "contact" ? "/contacts" : art === "task" ? "/tasks" : "/calendar",
  );

  function open() {
    if (onOpen) {
      onOpen(art, id);
      return;
    }
    if (art === "mail") selection.setMail({ uid: Number(id) || 0, folder: "", account: 0 }, true);
    else if (art === "contact") selection.setContact(id, true);
    else if (art === "task") selection.setTask(id, true);
    else selection.setEvent(id, true);
    goto(route);
  }
</script>

<button type="button" class="entity-chip" class:mail={art === "mail"} class:contact={art === "contact"}
  class:task={art === "task"} class:event={art === "event"} onclick={open}>
  <span class="entity-chip-dot" aria-hidden="true"></span>
  <span class="entity-chip-name">{name}</span>
</button>

<style>
  .entity-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 3px 10px 3px 8px;
    border: 1px solid var(--color-border);
    border-radius: 999px;
    background: var(--color-list);
    color: var(--color-text);
    font-family: inherit;
    font-size: 0.8125rem;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .entity-chip:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
    background: var(--color-active-wash);
  }
  .entity-chip-dot {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--color-accent);
  }
  .entity-chip.contact .entity-chip-dot {
    background: var(--color-success, #007e46);
  }
  .entity-chip.task .entity-chip-dot {
    background: var(--color-warning, #9f5100);
  }
  .entity-chip-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
