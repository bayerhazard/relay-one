<script lang="ts">
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
    dragSource?: string | null;
    dragTarget?: string | null;
    onSelectFolder?: (accountId: number, folder: string) => void;
    onToggleCollapse?: (accountId: number) => void;
    onToggleFolder?: (accountId: number, folderName: string) => void;
    onMoveMessage?: (uid: number, targetFolder: string, targetAccountId?: number) => void;
    onFolderMouseDown?: (e: MouseEvent, name: string) => void;
    onContextMenu?: (e: MouseEvent, name: string) => void;
  }

  let {
    account,
    folderTree,
    selectedFolder,
    collapsedFolders,
    dragSource = $bindable(null),
    dragTarget = $bindable(null),
    onSelectFolder = () => {},
    onToggleCollapse = () => {},
    onToggleFolder = () => {},
    onMoveMessage = () => {},
    onFolderMouseDown = () => {},
    onContextMenu,
  }: Props = $props();

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
    if (uid != null && selectedFolder !== targetFolder) {
      onMoveMessage(uid, targetFolder, account.id);
    }
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
    onclick={handleInboxClick}
    ondblclick={handleRootDblClick}
    oncontextmenu={(e) => onContextMenu?.(e, "INBOX")}
    role="button"
    tabindex="0"
    onkeydown={(e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); handleInboxClick(); }
    }}
  >
    <span class="tree-label">{account.name}</span>
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
    style={`padding-left: ${14 + (depth + 1) * 20}px`}
    data-folder={node.name}
    role="button"
    tabindex="0"
    onclick={() => { if (dragSource == null) onSelectFolder(account.id, node.name); dragSource = null; }}
    ondblclick={(e) => {
      if (node.children.length > 0) {
        e.preventDefault();
        onToggleFolder(account.id, node.name);
      }
    }}
    onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelectFolder(account.id, node.name); } }}
    onmousedown={(e) => onFolderMouseDown?.(e, node.name)}
    oncontextmenu={(e) => onContextMenu?.(e, node.name)}
    ondragenter={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = node.name; }}
    ondragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = node.name; }}
    ondragleave={(e) => {
      const related = e.relatedTarget as Node | null;
      if (related && (e.currentTarget as HTMLElement).contains(related)) return;
      if (dragTarget === node.name) dragTarget = null;
    }}
    ondrop={(e) => handleDrop(e, node.name)}
  >
    <span class="tree-label">{node.label}</span>
    {#if node.children.length > 0}
      <span class="chevron" aria-hidden="true">
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

  .chevron {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    color: var(--color-text-secondary);
    opacity: 0.45;
    transition: opacity 0.12s ease;
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
