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
  //  - swipe right → mark read/unread (iOS default leading action, commits on release)
  //  - swipe left  → reveals Flag + Trash behind the row; full swipe = delete
  // Long-press (~500ms) opens the context menu (iOS context-menu interaction).
  let swipeState = $state<Record<number, { dx: number; startX: number; startY: number; dir: "x" | "y" | null; active: boolean; committed: boolean }>>({});
  let revealedUid = $state<number | null>(null); // row pinned open showing Flag/Trash
  let isTouch = $state(false);

  $effect(() => {
    if (typeof window === "undefined") return;
    try {
      isTouch = window.matchMedia("(pointer: coarse)").matches;
    } catch {
      isTouch = false;
    }
  });

  const SWIPE_READ = 80;         // right swipe past this → toggle read on release
  const SWIPE_READ_FULL = 200;   // right full swipe (haptic commit feedback)
  const SWIPE_REVEAL = -110;     // left half swipe → pin Flag/Trash buttons open
  const SWIPE_DELETE = -240;     // left full swipe → delete
  const REVEAL_WIDTH = -152;     // two 76px action buttons

  function vibrate(ms: number) {
    try { navigator.vibrate?.(ms); } catch { /* unsupported — ignore */ }
  }

  // ─── Long-press → context menu ──────────────────────────────
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelLongPress() {
    if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
  }

  function startLongPress(uid: number, x: number, y: number) {
    cancelLongPress();
    longPressTimer = setTimeout(() => {
      longPressTimer = null;
      const s = swipeState[uid];
      // Only fire when the finger has not started a swipe/scroll.
      if (s && s.dir === null) {
        delete swipeState[uid];
        // Suppress the synthetic click that follows touchend.
        lastTouchUid = uid;
        setTimeout(() => { if (lastTouchUid === uid) lastTouchUid = null; }, 500);
        vibrate(15);
        openContextMenu(x, y, uid);
      }
    }, 500);
  }

  function touchStart(e: TouchEvent, uid: number) {
    if (!isTouch) return;
    // Tapping any row closes a previously revealed row.
    if (revealedUid !== null && revealedUid !== uid) revealedUid = null;
    const t = e.changedTouches[0];
    swipeState[uid] = { dx: 0, startX: t.clientX, startY: t.clientY, dir: null, active: true, committed: false };
    startLongPress(uid, t.clientX, t.clientY);
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
        cancelLongPress();
      } else {
        return;
      }
    }
    if (s.dir !== "x") return; // vertical scroll — let the page handle it
    e.preventDefault();
    // Clamp: right swipe max full-read, left swipe max full-delete.
    s.dx = Math.max(-280, Math.min(260, dx));
    // Haptic tick when crossing a commit threshold (once per crossing).
    const commit = s.dx <= SWIPE_DELETE || s.dx >= SWIPE_READ_FULL;
    if (commit && !s.committed) { s.committed = true; vibrate(10); }
    else if (!commit && s.committed) { s.committed = false; }
  }

  // Ignore click if this row was just swiped (touch gesture ended on it).
  let lastTouchUid: number | null = null;
  function touchEnd(_e: TouchEvent, msg: Message, uid: number) {
    cancelLongPress();
    const s = swipeState[uid];
    if (!s) return;
    delete swipeState[uid];
    if (s.dir !== "x") return;
    lastTouchUid = uid;
    setTimeout(() => { if (lastTouchUid === uid) lastTouchUid = null; }, 400);
    const dx = s.dx;
    if (dx <= SWIPE_DELETE) {
      // Full left swipe → delete (iOS trailing-edge action).
      ondelete?.(uid);
    } else if (dx <= SWIPE_REVEAL) {
      // Half left swipe → pin the Flag/Trash buttons open (iOS behaviour).
      revealedUid = uid;
    } else if (dx >= SWIPE_READ) {
      // Right swipe → toggle read (iOS leading action).
      ontoggleRead?.(uid);
    }
    // Small swipes snap back (state is cleared).
  }

  function touchCancel(uid: number) {
    cancelLongPress();
    delete swipeState[uid];
  }

  function swipeStyle(uid: number) {
    const s = swipeState[uid];
    if (s) return `transform: translateX(${s.dx}px); transition: none;`;
    if (revealedUid === uid) return `transform: translateX(${REVEAL_WIDTH}px);`;
    return "";
  }

  function hasSwipe(uid: number) {
    return !!swipeState[uid];
  }

  function bgDir(uid: number): "left" | "right" | null {
    const s = swipeState[uid];
    if (s && s.dir === "x") {
      if (s.dx > 0) return "left";   // right swipe → read button on the left edge
      if (s.dx < 0) return "right";  // left swipe → flag/trash on the right edge
      return null;
    }
    if (revealedUid === uid) return "right";
    return null;
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
      // Row was swiped / long-pressed — suppress the synthetic click.
      lastTouchUid = null;
      return;
    }
    // Tapping a revealed row just closes it (iOS behaviour).
    if (revealedUid === uid) {
      revealedUid = null;
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

  function openContextMenu(x: number, y: number, uid: number) {
    const menuWidth = 180;
    const menuHeight = 150;
    const vw = typeof window !== "undefined" ? window.innerWidth : menuWidth;
    const vh = typeof window !== "undefined" ? window.innerHeight : menuHeight;
    contextMenu = {
      x: Math.max(4, Math.min(x, vw - menuWidth)),
      y: Math.max(4, Math.min(y, vh - menuHeight)),
      uid,
    };
  }

  function handleContextMenu(e: MouseEvent, uid: number) {
    e.preventDefault();
    openContextMenu(e.clientX, e.clientY, uid);
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
        <div class="swipe-container" class:swiping={hasSwipe(msg.uid)} class:revealed={revealedUid === msg.uid}>
          <!-- Behind-actions (iOS look): right swipe → read (left edge);
               left swipe → flag + trash (right edge). -->
          <div class="swipe-bg" class:visible={bgDir(msg.uid) !== null}>
            {#if bgDir(msg.uid) === "left"}
              <div class="swipe-bg-left">
                <button type="button" class="swipe-action read" onclick={() => ontoggleRead?.(msg.uid)} tabindex="-1" aria-hidden="true">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.8" stroke="currentColor" class="swipe-icon">
                    {#if msg.is_read}
                      <path stroke-linecap="round" stroke-linejoin="round" d="M21.75 9v.906a2.25 2.25 0 01-1.183 1.981l-6.478 3.488M2.25 9v.906a2.25 2.25 0 001.183 1.981l6.478 3.488m8.839 2.51l-4.66-2.51m0 0l-1.023-.55a2.25 2.25 0 00-2.134 0l-1.022.55m0 0l-4.661 2.51m16.5 1.615a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V8.844a2.25 2.25 0 011.183-1.98l7.5-4.04a2.25 2.25 0 012.134 0l7.5 4.04a2.25 2.25 0 011.183 1.98V19.5z" />
                    {:else}
                      <path stroke-linecap="round" stroke-linejoin="round" d="M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0l-7.5-4.615a2.25 2.25 0 01-1.07-1.916V6.75" />
                    {/if}
                  </svg>
                  <span class="swipe-label">{msg.is_read ? "Ungelesen" : "Gelesen"}</span>
                </button>
              </div>
            {:else if bgDir(msg.uid) === "right"}
              <div class="swipe-bg-right">
                <button type="button" class="swipe-action flag" onclick={() => { revealedUid = null; ontoggleFlag?.(msg.uid); }} tabindex="-1" aria-hidden="true">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.8" stroke="currentColor" class="swipe-icon">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M3 3v1.5M3 21v-6m0 0l2.77-.693a9 9 0 016.208.682l.108.054a9 9 0 006.086.71l3.114-.732a48.524 48.524 0 01-.005-10.499l-3.11.732a9 9 0 01-6.085-.711l-.108-.054a9 9 0 00-6.208-.682L3 4.5M3 15V4.5" />
                  </svg>
                  <span class="swipe-label">{msg.is_flagged ? "Entmarkieren" : "Markieren"}</span>
                </button>
                <button type="button" class="swipe-action delete" onclick={() => { revealedUid = null; ondelete?.(msg.uid); }} tabindex="-1" aria-hidden="true">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.8" stroke="currentColor" class="swipe-icon">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                  </svg>
                  <span class="swipe-label">Löschen</span>
                </button>
              </div>
            {/if}
          </div>
          <div
            class="message-item"
            class:selected={selectedSet.has(msg.uid)}
            class:unread={!msg.is_read && !pendingReadUids.has(msg.uid)}
            style="height: {itemHeight}px; {swipeStyle(msg.uid)}"
            draggable={!isTouch}
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
    <div class="ctx-menu-scrim" class:sheet-scrim={isTouch} role="presentation" onclick={closeContextMenu} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" class:sheet={isTouch} style={isTouch ? "" : `left: ${contextMenu.x}px; top: ${contextMenu.y}px;`} role="menu">
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => onreply?.(uid))}>Antworten</button>
      <div class="ctx-menu-separator" role="separator"></div>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => ontoggleRead?.(uid))}>{contextMsg?.is_read ? "Als ungelesen markieren" : "Als gelesen markieren"}</button>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => runContextAction((uid) => ontoggleFlag?.(uid))}>{contextMsg?.is_flagged ? "Markierung löschen" : "Markieren"}</button>
      <div class="ctx-menu-separator" role="separator"></div>
      <button type="button" class="ctx-menu-item danger" role="menuitem" onclick={() => runContextAction((uid) => ondelete?.(uid))}>Löschen</button>
    </div>
  {/if}
</div>

<style>
  .message-list {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    /* NOTE: do NOT use contain: size/inline-size/layout/paint/strict here —
       ALL of them (except `style`) create a containing block for
       position:fixed descendants, which offsets the context menu (rendered
       inside this component) relative to the list instead of the viewport.
       `contain: style` isolates styles without that side effect. */
    contain: style;
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
  .ctx-menu-scrim.sheet-scrim {
    background: rgba(0, 0, 0, 0.35);
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
  .ctx-menu-item.danger {
    color: var(--color-danger);
  }
  .ctx-menu-separator {
    height: 1px;
    margin: 4px 8px;
    background: var(--color-border);
  }

  /* iOS-style bottom sheet (touch devices): slides up from the bottom edge,
     full width, large touch targets, safe-area aware. */
  @keyframes sheetUp {
    from { transform: translateY(100%); }
    to { transform: translateY(0); }
  }
  .ctx-menu.sheet {
    left: 0;
    right: 0;
    bottom: 0;
    top: auto;
    width: 100%;
    min-width: 0;
    max-width: none;
    max-height: 65vh;
    overflow-y: auto;
    border: none;
    border-radius: 16px 16px 0 0;
    box-shadow: 0 -8px 32px rgba(0, 0, 0, 0.2);
    padding: 8px 12px calc(12px + env(safe-area-inset-bottom, 0px));
    animation: sheetUp 0.28s cubic-bezier(0.32, 0.72, 0, 1);
  }
  .ctx-menu.sheet .ctx-menu-item {
    display: flex;
    align-items: center;
    min-height: 48px;
    padding: 12px 16px;
    font-size: 1rem;
    border-radius: 10px;
  }
  .ctx-menu.sheet .ctx-menu-item:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .ctx-menu.sheet .ctx-menu-item.danger:hover {
    color: var(--color-danger);
  }
  .ctx-menu.sheet .ctx-menu-separator {
    margin: 4px 16px;
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
    /* Hidden unless a swipe is actually in progress or the row is pinned
       open. Showing buttons permanently makes them shine through
       translucent selected rows in dark mode. */
    opacity: 0;
    transition: opacity 0.15s ease;
  }
  .swipe-bg.visible {
    opacity: 1;
  }
  .swipe-bg-left,
  .swipe-bg-right {
    display: flex;
    align-items: stretch;
  }
  .swipe-bg-left { margin-right: auto; }
  .swipe-bg-right { margin-left: auto; }
  .swipe-action {
    border: none;
    color: #fff;
    width: 76px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    cursor: pointer;
    pointer-events: auto;
  }
  .swipe-container:not(.swiping):not(.revealed) .swipe-action {
    pointer-events: none;
  }
  .swipe-icon {
    width: 20px;
    height: 20px;
  }
  .swipe-label {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }
  /* iOS system colours (Mail.app conventions). */
  .swipe-action.flag { background: #ff9500; }
  .swipe-action.read { background: #007aff; }
  .swipe-action.delete { background: #ff3b30; }

  @media (pointer: coarse) {
    .message-item {
      touch-action: pan-y;
      /* Long-press must not trigger the iOS text-selection loupe / callout. */
      -webkit-touch-callout: none;
      -webkit-user-select: none;
      user-select: none;
    }
  }
</style>
