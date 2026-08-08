<script lang="ts">
  type Folder = {
    name: string;
    tag: string;
    label?: string;
    icon?: string;
    indent?: boolean;
  };

  interface Props {
    folders: Folder[];
    selectedFolder: string;
    dragSource?: string | null;
    dragTarget?: string | null;
    onSelect?: (name: string) => void;
    onFolderMouseDown?: (e: MouseEvent, name: string) => void;
    onMoveMessage?: (uid: number, targetFolder: string) => void;
    onContextMenu?: (e: MouseEvent, name: string) => void;
  }

  let {
    folders,
    selectedFolder,
    dragSource = $bindable(null),
    dragTarget = $bindable(null),
    onSelect = () => {},
    onFolderMouseDown = () => {},
    onMoveMessage = () => {},
    onContextMenu,
  }: Props = $props();

  function handleContextMenu(e: MouseEvent, folderName: string) {
    e.preventDefault();
    onContextMenu?.(e, folderName);
  }

  // Resolve the dragged message UID. Prefer the native dataTransfer payload
  // (required on macOS WKWebView, where the drop target can't rely on shared
  // component state), fall back to the local dragSource for folder reordering
  // and non-DnD code paths.
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
</script>

{#each folders as folder (folder.name)}
  <div
    class="folder-item"
    class:active={selectedFolder === folder.name}
    class:drag-over={dragTarget === folder.name}
    class:indent={folder.indent}
    data-folder={folder.name}
    role="button"
    tabindex="0"
    onclick={() => { if (dragSource == null) onSelect(folder.name); dragSource = null; }}
    onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(folder.name); } }}
    onmousedown={(e) => onFolderMouseDown(e, folder.name)}
    oncontextmenu={(e) => handleContextMenu(e, folder.name)}
    ondragenter={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = folder.name; }}
    ondragover={(e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "move"; dragTarget = folder.name; }}
    ondragleave={(e) => {
      // Only clear when actually leaving the folder element (not when moving
      // over a child), to avoid highlight flicker.
      const related = e.relatedTarget as Node | null;
      if (related && (e.currentTarget as HTMLElement).contains(related)) return;
      if (dragTarget === folder.name) dragTarget = null;
    }}
    ondrop={(e) => {
      e.preventDefault();
      dragTarget = null;
      const uid = resolveUid(e);
      if (uid != null && selectedFolder !== folder.name) {
        onMoveMessage(uid, folder.name);
      }
      dragSource = null;
    }}
   >
    {#if folder.icon}
      <span class="folder-icon-wrapper">{@html folder.icon}</span>
    {/if}
    <span class="folder-name-label">{folder.label ?? folder.name}</span>
  </div>
{/each}
