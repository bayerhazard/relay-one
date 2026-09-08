import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import AccountGroup from "$lib/components/AccountGroup.svelte";

function makeTree() {
  return {
    name: "INBOX",
    label: "Posteingang",
    children: [
      { name: "Archive", label: "Archive", children: [] },
    ],
  };
}

const baseAccount = { id: 7, name: "Zweitkonto", username: "zweit@example.com", connected: true };

function renderGroup(props: Record<string, unknown> = {}) {
  const onMoveMessage = vi.fn();
  const result = render(AccountGroup, {
    account: baseAccount,
    folderTree: makeTree(),
    selectedFolder: "INBOX",
    collapsedFolders: new Set<string>(),
    dragSource: "99",
    dragTarget: null,
    onMoveMessage,
    ...props,
  });
  return { ...result, onMoveMessage };
}

function dropEvent(): Event {
  const event = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "preventDefault", { value: vi.fn() });
  // jsdom DragEvent support is incomplete — resolveUid falls back to the
  // dragSource prop when getData throws / returns null.
  Object.defineProperty(event, "dataTransfer", { value: null });
  return event;
}

describe("AccountGroup cross-account drop targets", () => {
  it("accepts a drop on the ROOT (inbox) row and reports this account's id", () => {
    const { container, onMoveMessage } = renderGroup();
    const rootRow = container.querySelector(".root-row")!;
    expect(rootRow).not.toBeNull();
    rootRow.dispatchEvent(dropEvent());
    expect(onMoveMessage).toHaveBeenCalledOnce();
    expect(onMoveMessage).toHaveBeenCalledWith(99, "INBOX", 7);
  });

  it("passes even when the viewed folder has the SAME name (cross-account)", () => {
    // Viewing "INBOX" of account A, dropping onto account B's "INBOX" root —
    // the component no longer suppresses same-name drops; the parent decides.
    const { container, onMoveMessage } = renderGroup({ selectedFolder: "INBOX" });
    container.querySelector(".root-row")!.dispatchEvent(dropEvent());
    expect(onMoveMessage).toHaveBeenCalledWith(99, "INBOX", 7);
  });

  it("same-name suppression also removed for subfolder rows", () => {
    const { container, onMoveMessage } = renderGroup({ selectedFolder: "Archive" });
    const rows = container.querySelectorAll(".tree-row:not(.root-row)");
    const archiveRow = Array.from(rows).find((r) => r.textContent?.includes("Archive"))!;
    archiveRow.dispatchEvent(dropEvent());
    expect(onMoveMessage).toHaveBeenCalledWith(99, "Archive", 7);
  });

  it("does not call onMoveMessage without a dragged message", () => {
    const { container, onMoveMessage } = renderGroup({ dragSource: null });
    container.querySelector(".root-row")!.dispatchEvent(dropEvent());
    expect(onMoveMessage).not.toHaveBeenCalled();
  });
});

describe("AccountGroup folder icons", () => {
  it("root (INBOX) row carries an inbox icon", () => {
    const { container } = renderGroup();
    const icon = container.querySelector(".root-row .tree-icon svg");
    expect(icon).not.toBeNull();
  });

  it("folder rows carry a semantic icon before the label", () => {
    const { container } = renderGroup();
    const row = Array.from(container.querySelectorAll(".tree-row:not(.root-row)"))
      .find((r) => r.textContent?.includes("Archive"))!;
    const icon = row.querySelector(".tree-icon svg");
    expect(icon).not.toBeNull();
    // icon precedes the label inside the row
    const label = row.querySelector(".tree-label")!;
    expect(icon!.compareDocumentPosition(label) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("AccountGroup unread INBOX badge (26.9.147)", () => {
  it("shows the count when there are unread messages", () => {
    const { container } = renderGroup({ unreadCount: 5 });
    const badge = container.querySelector(".unread-badge");
    expect(badge).not.toBeNull();
    expect(badge!.textContent?.trim()).toBe("5");
    expect(badge!.getAttribute("aria-label")).toBe("5 ungelesene Mails");
  });

  it("hides the badge when there are no unread messages", () => {
    const { container } = renderGroup({ unreadCount: 0 });
    expect(container.querySelector(".unread-badge")).toBeNull();
  });

  it("caps large counts at 99+ but keeps the full number in the label", () => {
    const { container } = renderGroup({ unreadCount: 150 });
    const badge = container.querySelector(".unread-badge");
    expect(badge!.textContent?.trim()).toBe("99+");
    expect(badge!.getAttribute("aria-label")).toBe("150 ungelesene Mails");
  });
});
