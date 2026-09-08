<script lang="ts">
  import { iconSVG, folderIconFor } from "$lib/icons";
  import { t } from "$lib/i18n";

  interface AccountInfo {
    id: number;
    name: string;
    username: string;
    connected: boolean;
  }

  interface FolderNode {
    name: string;
    label: string;
    children: FolderNode[];
    local_only?: boolean;
  }

  interface Props {
    account: AccountInfo;
    folderTree: FolderNode;
    selectedFolder: string | null;
    collapsedFolders: Set<string>;
    unreadCount?: number;
    dragSource?: string | null;
    dragTarget?: string | null;
    onSelectFolder?: (accountId: number, folder: string) => void;
    onToggleCollapse?: (accountId: number) => void;
    onToggleFolder?: (accountId: number, folderName: string) => void;
    onMoveMessage?: (uid: number, targetFolder: string, targetAccountId?: number) => void;
    onFolderMouseDown?: (e: MouseEvent, name: string) => void;
    onContextMenu?: (e: { clientX: number; clientY: number; preventDefault: () => void }, name: string) => void;
  }

  let {
    account,
    folderTree,
    selectedFolder,
    collapsedFolders,
    unreadCount = 0,
    dragSource = $bindable(null),
    dragTarget = $bindable(null),
    onSelectFolder = () => {},
    onToggleCollapse = () => {},
    onToggleFolder = () => {},
    onMoveMessage = () => {},
    onFolderMouseDown = () => {},
    onContextMenu,
  }: Props = $props();

  // Subtle pop whenever the unread count INCREASES (new mail arrived at this
  // account) — draws the eye without a permanent animation. Honors reduced
  // motion via the CSS media query on the transform.
  let prevCount = $state(0);
  let bump = $state(false);
  $effect(() => {
    const c = unreadCount;
    if (c > prevCount) {
      bump = true;
      prevCount = c;
      const id = setTimeout(() => (bump = false), 320);
      return () => clearTimeout(id);
    }
    prevCount = c;
  });

  function handleInboxClick() {
    onSelectFolder(account.id, "INBOX");
  }

  function handleRootDblClick(e: MouseEvent) {
    e.preventDefault();
    onToggleCollapse(account.id);
  }

  function resolveUid(e: DragEvent): number | null {
    let raw: string | null = null;
    try {
      raw = e.dataTransfer?.getData("text/plain") || null;
    } catch {
      raw = null;
    }
    if (!raw) raw = dragSource;
    if (raw == null) return null;
    const uid = parseInt(raw, 10);
    return isNaN(uid) ? null : uid;
  }

  function handleDrop(e: DragEvent, targetFolder: string) {
    e.preventDefault();
    dragTarget = null;
    const uid = resolveUid(e);
    // Same-folder suppression happens in the parent's handler — it knows both
    // accounts, so a drop onto another account's SAME-NAMED folder must pass.
    if (uid != null) {
      onMoveMessage(uid, targetFolder, account.id);
    }
    dragSource = null;
  }

  // ─── Long-press → context menu (touch devices, iOS has no contextmenu) ──
  let isTouch = false;
  try {
    isTouch = typeof window !== "undefined" && window.matchMedia("(pointer: coarse)").matches;
  } catch { isTouch = false; }

  let lpTimer: ReturnType<typeof setTimeout> | null = null;
  let lpStartX = 0;
  let lpStartY = 0;
  let lpFired = false;

  function cancelLongPress() {
    if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
  }

  function handleTouchStart(e: TouchEvent, name: string) {
    if (!isTouch || !onContextMenu) return;
    lpFired = false;
    const t = e.changedTouches[0];
    lpStartX = t.clientX;
    lpStartY = t.clientY;
    cancelLongPress();
    lpTimer = setTimeout(() => {
      lpTimer = null;
      lpFired = true;
      try { navigator.vibrate?.(15); } catch { /* unsupported */ }
      onContextMenu({ clientX: lpStartX, clientY: lpStartY, preventDefault: () => {} }, name);
    }, 500);
  }

  function handleTouchMove(e: TouchEvent) {
    if (!lpTimer) return;
    const t = e.changedTouches[0];
    if (Math.abs(t.clientX - lpStartX) > 10 || Math.abs(t.clientY - lpStartY) > 10) {
      cancelLongPress();
    }
  }

  function handleTouchEnd() {
    cancelLongPress();
    // Reset the fired flag after the synthetic click has passed.
    if (lpFired) setTimeout(() => { lpFired = false; }, 300);
  }

  function handleRowClick(name: string, isRoot: boolean) {
    // Suppress the synthetic click that follows a long-press.
    if (lpFired) { lpFired = false; return; }
    if (isRoot) handleInboxClick();
    else if (dragSource == null) onSelectFolder(account.id, name);
    dragSource = null;
  }

  // Chevron SVG — inline, compact
  function chevronSVG(open: boolean): string {
    if (open) {
      return `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5l3 3 3-3"/></svg>`;
    }
    return `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3l3 3-3 3"/></svg>`;
  }
</script>

<div class="account-group">
  <!-- Root = Account Name / Inbox. Expand/collapse via DOUBLE-CLICK on the
       account name only — no chevron button (per user request). -->
  <div
    class="tree-row root-row"
    class:active={selectedFolder === "INBOX"}
    class:drag-over={dragTarget === "INBOX"}
    onclick={() => handleRowClick("INBOX", true)}
    ondblclick={handleRootDblClick}
    ondragenter={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = "INBOX"; }}
    ondragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = "INBOX"; }}
    ondragleave={(e) => {
      const related = e.relatedTarget as Node | null;
      if (related && (e.currentTarget as HTMLElement).contains(related)) return;
      if (dragTarget === "INBOX") dragTarget = null;
    }}
    ondrop={(e) => handleDrop(e, "INBOX")}
    oncontextmenu={(e) => onContextMenu?.(e, "INBOX")}
    ontouchstart={(e) => handleTouchStart(e, "INBOX")}
    ontouchmove={handleTouchMove}
    ontouchend={handleTouchEnd}
    ontouchcancel={handleTouchEnd}
    role="button"
    tabindex="0"
    onkeydown={(e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); handleInboxClick(); }
    }}
  >
    <span class="tree-icon">{@html iconSVG("inbox")}</span>
    <span class="tree-label">{account.name}</span>
    {#if unreadCount > 0}
      <span
        class="unread-badge"
        class:unread-bump={bump}
        title={$t("mail.unreadCount", { count: unreadCount })}
        aria-label={$t("mail.unreadCount", { count: unreadCount })}
      >{unreadCount > 99 ? "99+" : unreadCount}</span>
    {/if}
  </div>

  <!-- Children tree -->
  {#if !collapsedFolders.has("INBOX") && folderTree.children.length > 0}
    {@render renderFolderTree(folderTree.children)}
  {/if}

  <!-- Subtle divider between accounts -->
  <div class="account-divider"></div>
</div>

{#snippet renderFolderTree(nodes: FolderNode[], depth: number = 0)}
  {#each nodes as node}
    {@render renderFolderNode(node, depth)}
  {/each}
{/snippet}

{#snippet renderFolderNode(node: FolderNode, depth: number = 0)}
  <div
    class="tree-row"
    class:active={selectedFolder === node.name}
    class:drag-over={dragTarget === node.name}
    style={`padding-left: ${14 + (depth + 1) * 15}px`}
    data-folder={node.name}
    role="button"
    tabindex="0"
    onclick={() => handleRowClick(node.name, false)}
    ondblclick={(e) => {
      if (node.children.length > 0) {
        e.preventDefault();
        onToggleFolder(account.id, node.name);
      }
    }}
    onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelectFolder(account.id, node.name); } }}
    onmousedown={(e) => onFolderMouseDown?.(e, node.name)}
    oncontextmenu={(e) => onContextMenu?.(e, node.name)}
    ontouchstart={(e) => handleTouchStart(e, node.name)}
    ontouchmove={handleTouchMove}
    ontouchend={handleTouchEnd}
    ontouchcancel={handleTouchEnd}
    ondragenter={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = node.name; }}
    ondragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = node.name; }}
    ondragleave={(e) => {
      const related = e.relatedTarget as Node | null;
      if (related && (e.currentTarget as HTMLElement).contains(related)) return;
      if (dragTarget === node.name) dragTarget = null;
    }}
    ondrop={(e) => handleDrop(e, node.name)}
  >
    <span class="tree-icon">{@html folderIconFor(node.name)}</span>
    <span class="tree-label">{node.label}</span>
    {#if node.children.length > 0}
      <span
        class="chevron"
        role="button"
        tabindex="0"
        aria-label={collapsedFolders.has(node.name) ? "Unterordner einblenden" : "Unterordner ausblenden"}
        onclick={(e) => { e.stopPropagation(); onToggleFolder(account.id, node.name); }}
        onkeydown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            e.stopPropagation();
            onToggleFolder(account.id, node.name);
          }
        }}
      >
        {@html chevronSVG(!collapsedFolders.has(node.name))}
      </span>
    {/if}
  </div>

  {#if node.children.length > 0 && !collapsedFolders.has(node.name)}
    {@render renderFolderTree(node.children, depth + 1)}
  {/if}
{/snippet}

<style>
  .account-group {
    padding: 4px 0;
  }

  .tree-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex: none;
    color: var(--color-text-secondary);
  }
  .tree-row.active .tree-icon {
    color: var(--color-accent);
  }

  /* ── Tree Row ────────────────────────────── */
  .tree-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    margin: 0 8px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 400;
    color: var(--color-text-secondary);
    transition: background 0.12s ease, color 0.12s ease;
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
  }

  .tree-row:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }

  .tree-row.active {
    background: var(--color-active-wash);
    color: var(--color-accent);
    font-weight: 600;
  }

  .tree-row.drag-over {
    background: var(--color-active-wash);
  }

  /* Root row is slightly larger/bolder */
  .root-row {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-text);
  }

  .root-row:hover {
    color: var(--color-text);
  }

  .root-row.active {
    color: var(--color-accent);
  }

  .tree-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 1.3;
  }

  /* Unread-INBOX badge (root row). Signals "new mail at this account" — the
     gold/blue fill distinguishes it from the neutral header count. */
  .unread-badge {
    flex: none;
    font-size: 0.6875rem;
    font-weight: 700;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    color: var(--color-unread-badge-text);
    background: var(--color-unread);
    padding: 3px 7px;
    border-radius: 100px;
    min-width: 20px;
    text-align: center;
  }

  .unread-bump {
    animation: badge-pop 0.32s ease;
  }

  @keyframes badge-pop {
    0% { transform: scale(0.7); }
    60% { transform: scale(1.15); }
    100% { transform: scale(1); }
  }

  @media (prefers-reduced-motion: reduce) {
    .unread-bump { animation: none; }
  }

  .chevron {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-text-secondary);
    opacity: 0.45;
    transition: opacity 0.12s ease;
    /* Bigger touch target on phones (chevron toggles expand/collapse —
       dblclick is unreachable on touch). */
    min-width: 24px;
    min-height: 24px;
    margin: -4px -6px -4px 0;
    border-radius: 6px;
    cursor: pointer;
  }

  .tree-row:hover .chevron {
    opacity: 0.8;
  }

  /* ── Divider ─────────────────────────────── */
  .account-divider {
    height: 1px;
    background: var(--color-border);
    margin: 6px 16px 2px;
    opacity: 0.5;
  }
</style>
