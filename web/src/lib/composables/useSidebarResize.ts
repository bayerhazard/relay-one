import { writable, get, type Writable } from "svelte/store";

/**
 * Resizable sidebar width, mirroring the Mail module's drag behavior.
 *
 * Call at the top level of a component's <script> and wire `destroy()`
 * into an `$effect` cleanup (or onDestroy) so the document listeners are
 * guaranteed to be torn down on unmount (prevents leaked mousemove
 * listeners when the mouse is released outside the window).
 *
 * Usage:
 *   const { width, startResize, destroy } = useSidebarResize();
 *   $effect(() => () => destroy());
 *   <aside style={`width: ${$width}px`}>...</aside>
 *   <div class="resize-handle" onmousedown={startResize}></div>
 */
export interface SidebarResize {
  /** Current width in px (reactive store). */
  width: Writable<number>;
  /** Start a drag-resize from a mousedown on the handle. */
  startResize: (e: MouseEvent) => void;
  /** Abort any in-flight drag listeners (call on unmount). */
  destroy: () => void;
}

export function useSidebarResize(
  defaultWidth = 220,
  min = 140,
  max = 400
): SidebarResize {
  const width = writable(defaultWidth);
  const controllers = new Set<AbortController>();

  function startResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = get(width);
    const ac = new AbortController();
    controllers.add(ac);
    const { signal } = ac;

    function finish() {
      controllers.delete(ac);
      ac.abort();
    }
    function onMove(ev: MouseEvent) {
      const dx = ev.clientX - startX;
      width.set(Math.max(min, Math.min(max, startW + dx)));
    }

    document.addEventListener("mousemove", onMove, { signal });
    document.addEventListener("mouseup", finish, { signal });
    // Safety net: release if the pointer leaves the window or it loses focus.
    window.addEventListener("blur", finish, { signal });
  }

  function destroy(): void {
    controllers.forEach((c) => c.abort());
    controllers.clear();
  }

  return { width, startResize, destroy };
}
