<script lang="ts">
  import type { Message } from "$lib/stores/mailbox";
  import SummaryLine from "./SummaryLine.svelte";
  import PriorityBadge from "./PriorityBadge.svelte";
  import FraudWarning from "./FraudWarning.svelte";
  import ReplySuggestions from "./ReplySuggestions.svelte";
  import EmptyState from "./EmptyState.svelte";
  import { formatDate, extractName } from "$lib/utils/format";

  interface Props {
    messages: Message[];
    selectedUids: number[];
    onselect: (uid: number) => void;
    onselectToggle: (uid: number) => void;
    onselectRange: (fromIdx: number, toIdx: number) => void;
    onreply?: (uid: number) => void;
    ondelete?: (uid: number) => void;
    ontoggleRead?: (uid: number) => void;
    ontoggleFlag?: (uid: number) => void;
    ondragstart?: (e: DragEvent, uid: number) => void;
    loading: boolean;
    accountId: number;
    isDraftFolder?: boolean;
    isSentFolder?: boolean;
    searchActive?: boolean;
  }

  let { messages, selectedUids, onselect, onselectToggle, onselectRange, onreply, ondelete, ontoggleRead, ontoggleFlag, ondragstart, loading, accountId, isDraftFolder = false, isSentFolder = false, searchActive = false }: Props = $props();

  let pendingReadUids = $state(new Set<number>());
  let pendingReadTimers = new Set<ReturnType<typeof setTimeout>>();

  // Clear any outstanding "mark read" timers on unmount.
  $effect(() => {
    return () => {
      for (const t of pendingReadTimers) clearTimeout(t);
      pendingReadTimers.clear();
    };
  });

  // iOS-Mail-style swipe gestures (touch devices).
  //  - swipe left  → mark read/unread (half) | full swipe = read toggle
  //  - swipe right → flag (half) | full swipe = delete
  // A sticky action button sits behind each row (iOS look).
  let swipeState = $state<Record<number, { dx: number; startX: number; startY: number; dir: "x" | "y" | null; active: boolean }>>({});
  let isTouch = $state(false);

  $effect(() => {
    if (typeof window === "undefined") return;
    try {
      isTouch = window.matchMedia("(pointer: coarse)").matches;
    } catch {
      isTouch = false;
    }
  });

  const SWIPE_DELETE = -220;
  const SWIPE_READ = -110;
  const SWIPE_FLAG = 110;
  const SWIPE_DELETE_RIGHT = 220;

  function touchStart(e: TouchEvent, uid: number) {
    if (!isTouch) return;
    const t = e.changedTouches[0];
    swipeState[uid] = { dx: 0, startX: t.clientX, startY: t.clientY, dir: null, active: true };
  }

  function touchMove(e: TouchEvent, uid: number) {
    const s = swipeState[uid];
    if (!s?.active) return;
    const t = e.changedTouches[0];
    const dx = t.clientX - s.startX;
    const dy = t.clientY - s.startY;
    if (s.dir === null) {
      // Lock axis after ~8px of movement.
      if (Math.abs(dx) > 8 || Math.abs(dy) > 8) {
        s.dir = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
      } else {
        return;
      }
    }
    if (s.dir !== "x") return; // vertical scroll — let the page handle it
    e.preventDefault();
    // Clamp between full-right and full-left.
    s.dx = Math.max(-260, Math.min(260, dx));
  }

  // Ignore click if this row was just swiped (touch gesture ended on it).
  let lastTouchUid: number | null = null;
  function touchEnd(_e: TouchEvent, msg: Message, uid: number) {
    const s = swipeState[uid];
    if (!s) return;
    delete swipeState[uid];
    if (s.dir !== "x") return;
    lastTouchUid = uid;
    setTimeout(() => { if (lastTouchUid === uid) lastTouchUid = null; }, 400);
    const dx = s.dx;
    if (dx <= SWIPE_DELETE || dx >= SWIPE_DELETE_RIGHT) {
      // Full swipe → delete.
      ondelete?.(uid);
    } else if (dx <= SWIPE_READ) {
      // Half left swipe → toggle read.
      ontoggleRead?.(uid);
    } else if (dx >= SWIPE_FLAG) {
      // Half right swipe → toggle flag.
      ontoggleFlag?.(uid);
    }
    // Small swipes snap back (state is cleared).
  }

  function touchCancel(uid: number) {
    delete swipeState[uid];
  }

  function swipeStyle(uid: number) {
    const s = swipeState[uid];
    if (!s) return "";
    return `transform: translateX(${s.dx}px); transition: none;`;
  }

  function hasSwipe(uid: number) {
    return !!swipeState[uid];
  }


  // Use ResizeObserver to get actual container height on mount/resize
  $effect(() => {
    if (!scrollElement) return;
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) {
        containerHeight = entry.contentRect.height;
      }
    });
    observer.observe(scrollElement);
    return () => observer.disconnect();
  });

  let scrollElement: HTMLDivElement;
  let itemHeight = 88;
  let overscan = 5;
  let totalHeight = $derived(messages.length * itemHeight);
  let scrollTop = $state(0);
  let containerHeight = $state(0);

  // Use ResizeObserver to get actual container height on mount/resize
  $effect(() => {
    if (!scrollElement) return;
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) {
        containerHeight = entry.contentRect.height;
      }
    });
    observer.observe(scrollElement);
    return () => observer.disconnect();
  });

  let lastClickedIndex = $state<number | null>(null);

  function handleScroll() {
    if (scrollElement) {
      scrollTop = scrollElement.scrollTop;
      containerHeight = scrollElement.clientHeight;
    }
  }

  let startIndex = $derived(Math.max(0, Math.floor(scrollTop / itemHeight) - overscan));
  let visibleCount = $derived(Math.min(messages.length - startIndex, Math.ceil(containerHeight / itemHeight) + 2 * overscan));
  let visibleItems = $derived(messages.slice(startIndex, startIndex + visibleCount));
  let offsetY = $derived(startIndex * itemHeight);
  let selectedSet = $derived(new Set(selectedUids));

  function handleClick(e: MouseEvent, uid: number, index: number) {
    if (lastTouchUid === uid) {
      // Row was swiped — suppress the synthetic click.
      lastTouchUid = null;
      return;
    }
    if (!pendingReadUids.has(uid)) {
      pendingReadUids = new Set([...pendingReadUids, uid]);
      const timer = setTimeout(() => {
        pendingReadUids.delete(uid);
        pendingReadUids = new Set(pendingReadUids);
        pendingReadTimers.delete(timer);
      }, 3000);
      pendingReadTimers.add(timer);
    }
    if (e.shiftKey && lastClickedIndex !== null) {
      onselectRange(Math.min(lastClickedIndex, index), Math.max(lastClickedIndex, index));
    } else if (e.ctrlKey || e.metaKey) {
      onselectToggle(uid);
    } else {
      onselect(uid);
      lastClickedIndex = index;
    }
  }

  // ─── Plain HTML context menu (replaces the Tauri native menu) ──────────
  let contextMenu = $state<{ x: number; y: number; uid: number } | null>(null);
  let contextMsg = $derived(
    contextMenu ? (messages.find((m) => m.uid === contextMenu.uid) ?? null) : null
  );

  function handleContextMenu(e: MouseEvent, uid: number) {
    e.preventDefault();
    const menuWidth = 180;
    const menuHeight = 150;
    const vw = typeof window !== "undefined" ? window.innerWidth : menuWidth;
    const vh = typeof window !== "undefined" ? window.innerHeight : menuHeight;
    contextMenu = {
      x: Math.max(4, Math.min(e.clientX, vw - menuWidth)),
      y: Math.max(4, Math.min(e.clientY, vh - menuHeight)),
      uid,
    };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  function runContextAction(action?: (uid: number) => void) {
    const uid = contextMenu?.uid;
    contextMenu = null;
    if (uid != null) action?.(uid);
  }

  $effect(() => {
    if (!contextMenu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeContextMenu();
    };
    const onBlur = () => closeContextMenu();
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  });
</script>

<div class="message-list" bind:this={scrollElement} onscroll={handleScroll}>
  <div style="height: {totalHeight}px; position: relative;">
    <div style="position: absolute; top: 0; left: 0; width: 100%; transform: translateY({offsetY}px);">
      {#each visibleItems as msg, i (msg.uid)}
        <div class="swipe-container" class:swiping={hasSwipe(msg.uid)}>
          <!-- Behind-actions (iOS look) -->
          <div class="swipe-bg">
            <div class="swipe-bg-left">
              <button type="button" class="swipe-action flag" onclick={() => ontoggleFlag?.(msg.uid)} tabindex="-1" aria-hidden="true">
                🚩
              </button>
              <button type="button" class="swipe-action read" onclick={() => ontoggleRead?.(msg.uid)} tabindex="-1" aria-hidden="true">
                {msg.is_read ? "○" : "●"}
              </button>
              <button type="button" class="swipe-action delete" onclick={() => ondelete?.(msg.uid)} tabindex="-1" aria-hidden="true">
                🗑
              </button>
            </div>
            <div class="swipe-bg-right">
              <button type="button" class="swipe-action flag" onclick={() => ontoggleFlag?.(msg.uid)} tabindex="-1" aria-hidden="true">
                🚩
              </button>
              <button type="button" class="swipe-action read" onclick={() => ontoggleRead?.(msg.uid)} tabindex="-1" aria-hidden="true">
                {msg.is_read ? "○" : "●"}
              </button>
              <button type="button" class="swipe-action delete" onclick={() => ondelete?.(msg.uid)} tabindex="-1" aria-hidden="true">
                🗑
              </button>
            </div>
          </div>
          <div
            class="message-item"
            class:selected={selectedSet.has(msg.uid)}
            class:unread={!msg.is_read && !pendingReadUids.has(msg.uid)}
            style="height: {itemHeight}px; {swipeStyle(msg.uid)}"
            draggable="true"
            onclick={(e) => handleClick(e, msg.uid, startIndex + i)}
            oncontextmenu={(e) => handleContextMenu(e, msg.uid)}
            ondragstart={(e) => ondragstart?.(e, msg.uid)}
            ontouchstart={(e) => touchStart(e, msg.uid)}
            ontouchmove={(e) => touchMove(e, msg.uid)}
            ontouchend={(e) => touchEnd(e, msg, msg.uid)}
            ontouchcancel={() => touchCancel(msg.uid)}
            role="option"
            aria-selected={selectedSet.has(msg.uid)}
          >
            <div class="msg-header">
              <span class="sender">{extractName(isSentFolder ? msg.to : msg.from) || "Unbekannt"}{msg.is_flagged && " 🚩"}</span>
              <span class="msg-header-right">
                {#if msg.has_attachments}
                  <span class="attach-indicator" title="Enthält einen Anhang" aria-label="Anhang">&#x1F4CE;</span>
                {/if}
                <span class="date">{formatDate(msg.date)}</span>
              </span>
            </div>
            <div class="msg-subject">
              {#if isDraftFolder}
                <span class="draft-badge">Entwurf</span>
              {/if}
              <PriorityBadge intensity={msg.ai_priority ?? 0} fraudScore={msg.ai_fraud_score ?? 0} />
              {msg.subject || "(Kein Betreff)"}
            </div>
            {#if msg.ai_summary}
              <SummaryLine summary={msg.ai_summary} />
            {/if}
            {#if (msg.ai_fraud_score ?? 0) > 0.6}
              <FraudWarning score={msg.ai_fraud_score ?? 0} warnings={[]} />
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </div>
  {#if loading}
    <div class="skeleton-list" aria-hidden="true">
      {#each Array(7) as _, i}
        <div class="skeleton-row" style="animation-delay: {i * 0.06}s">
          <div class="skeleton-line skeleton-sender" style="animation-delay: {i * 0.06}s"></div>
          <div class="skeleton-line skeleton-subject" style="animation-delay: {i * 0.06}s"></div>
          <div class="skeleton-line skeleton-summary" style="animation-delay: {i * 0.06}s"></div>
        </div>
      {/each}
    </div>
  {:else if messages.length === 0}
    {#if searchActive}
      <EmptyState icon="&#x1F50D;" title="Keine Treffer" subtitle="Keine Nachricht passt zu deiner Suche." />
    {:else}
      <EmptyState icon="&#x2709;" title="Keine Nachrichten" subtitle="Dieser Ordner ist leer." />
    {/if}
  {/if}
  {#if contextMenu}
    <div class="ctx-menu-scrim" role="presentation" onclick={closeContextMenu} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" style="left: {contextMenu.x}px; top: {contextMenu.y}px;" role="menu">
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => onreply?.(uid))}>Antworten</button>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => ondelete?.(uid))}>Löschen</button>
      <div class="ctx-menu-separator" role="separator"></div>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => ontoggleRead?.(uid))}>{contextMsg?.is_read ? "Als ungelesen markieren" : "Als gelesen markieren"}</button>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => ontoggleFlag?.(uid))}>{contextMsg?.is_flagged ? "Markierung löschen" : "Markieren"}</button>
    </div>
  {/if}
</div>

<style>
  .message-list {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    /* NOTE: do NOT use contain: strict/layout/paint here — it creates a
       containing block for position:fixed descendants, which would offset
       the context menu (rendered inside this component) relative to the
       list instead of the viewport. inline-size containment keeps the
       layout benefits without that side effect. */
    contain: inline-size;
    will-change: scroll-position;
    padding-top: 10px;
  }
  .message-list::-webkit-scrollbar {
    width: 6px;
  }
  .message-list::-webkit-scrollbar-track {
    background: transparent;
  }
  .message-list::-webkit-scrollbar-thumb {
    background: var(--color-border);
    border-radius: 3px;
  }
  .message-list::-webkit-scrollbar-thumb:hover {
    background: var(--color-text-secondary);
  }
  .message-item {
    padding: 10px 16px;
    border-left: 3px solid transparent;
    border-bottom: 1px solid var(--color-border);
    cursor: pointer;
    contain: layout style paint;
    transition: all 0.15s ease-in-out;
  }
  .message-item:hover {
    background: var(--color-sidebar);
  }
  .message-item.selected {
    background: var(--color-active-wash);
    border-left-color: var(--color-accent);
  }
  .msg-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.8125rem;
    font-weight: 700;
  }
  .sender {
    font-weight: 600;
    font-size: 0.875rem;
    color: var(--color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .msg-header-right {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }
  .attach-indicator {
    font-size: 0.75rem;
    opacity: 0.6;
    line-height: 1;
  }
  .date {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
  }
  .msg-subject {
    font-size: 0.8125rem;
    margin-top: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--color-text);
  }
  .unread .sender {
    font-weight: 500;
    color: var(--color-text);
  }
  .unread .msg-subject {
    font-weight: 500;
  }
  .unread {
    background: var(--color-unread-wash);
    border-left-color: var(--color-unread);
  }
  .unread:hover {
    background: var(--color-active-wash);
  }
  .loading-indicator {
    text-align: center;
    padding: 16px;
    color: var(--color-text-secondary);
  }

  /* ─── Skeleton Loading ─── */
  .skeleton-list {
    padding-top: 0;
  }
  .skeleton-row {
    height: 88px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: 10px;
    justify-content: center;
  }
  .skeleton-line {
    height: 12px;
    border-radius: 6px;
    background: linear-gradient(
      90deg,
      var(--color-border) 0%,
      var(--color-active-wash) 40%,
      var(--color-active-wash) 60%,
      var(--color-border) 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.8s ease-in-out infinite;
  }
  .skeleton-sender {
    width: 140px;
    height: 14px;
  }
  .skeleton-subject {
    width: 260px;
  }
  .skeleton-summary {
    width: 180px;
    height: 10px;
    opacity: 0.6;
  }
  .draft-badge {
    display: inline-block;
    font-size: 0.625rem;
    font-weight: 700;
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    padding: 1px 6px;
    border-radius: 4px;
    margin-right: 6px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* ─── Plain HTML context menu ─── */
  .ctx-menu-scrim {
    position: fixed;
    inset: 0;
    z-index: 1000;
  }
  .ctx-menu {
    position: fixed;
    z-index: 1001;
    min-width: 170px;
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
    padding: 6px;
    display: flex;
    flex-direction: column;
  }
  .ctx-menu-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 8px 12px;
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 0.8125rem;
    color: var(--color-text);
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
  }
  .ctx-menu-item:hover {
    background: var(--color-active-wash);
    color: var(--color-accent);
  }
  .ctx-menu-separator {
    height: 1px;
    margin: 4px 8px;
    background: var(--color-border);
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  /* ─── iOS-Mail-style swipe gestures ─────────────────────── */
  .swipe-container {
    position: relative;
    overflow: hidden;
    height: 88px;
  }
  .swipe-container.swiping .message-item {
    touch-action: pan-y;
    -webkit-user-select: none;
    user-select: none;
  }
  .message-item {
    position: relative;
    z-index: 2;
    background: var(--color-list);
    transition: transform 0.25s cubic-bezier(0.2, 0.8, 0.3, 1);
    touch-action: pan-y;
  }
  .swipe-container.swiping .message-item {
    transition: none;
  }
  .swipe-bg {
    position: absolute;
    inset: 0;
    z-index: 1;
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    pointer-events: none;
    /* Hidden unless a swipe is actually in progress. Showing the action
       buttons permanently makes them shine through translucent selected
       rows in dark mode (--color-active-wash is an rgba there). */
    opacity: 0;
    transition: opacity 0.15s ease;
  }
  .swipe-container.swiping .swipe-bg {
    opacity: 1;
  }
  .swipe-bg-left,
  .swipe-bg-right {
    display: flex;
    align-items: stretch;
  }
  .swipe-action {
    border: none;
    color: #fff;
    font-size: 1.25rem;
    width: 72px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    pointer-events: auto;
  }
  .swipe-container:not(.swiping) .swipe-action {
    pointer-events: none;
  }
  .swipe-action.flag { background: #f5a623; }
  .swipe-action.read { background: #4a90d9; }
  .swipe-action.delete { background: #d9534f; }

  @media (pointer: coarse) {
    .message-item {
      touch-action: pan-y;
    }
  }
</style>
