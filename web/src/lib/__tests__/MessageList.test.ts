import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import MessageList from "$lib/components/MessageList.svelte";

function makeMessage(overrides: Partial<import("$lib/stores/mailbox").Message> = {}) {
  return {
    uid: 1,
    subject: "Test Betreff",
    from: "Alice <alice@example.com>",
    date: "2026-01-15T10:00:00Z",
    is_read: true,
    ...overrides,
  };
}

describe("MessageList", () => {
  const defaultProps = {
    messages: [makeMessage()],
    selectedUids: [] as number[],
    onselect: vi.fn(),
    onselectToggle: vi.fn(),
    onselectRange: vi.fn(),
    ondelete: vi.fn(),
    loading: false,
    accountId: 1,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders a message item", () => {
    render(MessageList, defaultProps);
    expect(screen.getByText("Alice")).toBeTruthy();
  });

  it("renders subject line", () => {
    render(MessageList, defaultProps);
    expect(screen.getByText("Test Betreff")).toBeTruthy();
  });

  it("renders multiple messages", () => {
    const messages = [
      makeMessage({ uid: 1, subject: "Erste" }),
      makeMessage({ uid: 2, subject: "Zweite" }),
      makeMessage({ uid: 3, subject: "Dritte" }),
    ];
    render(MessageList, { ...defaultProps, messages });
    expect(screen.getByText("Erste")).toBeTruthy();
    expect(screen.getByText("Zweite")).toBeTruthy();
    expect(screen.getByText("Dritte")).toBeTruthy();
  });

  it("shows skeleton rows when loading", () => {
    const { container } = render(MessageList, { ...defaultProps, loading: true });
    const skeleton = container.querySelector(".skeleton-list");
    expect(skeleton).toBeTruthy();
    expect(skeleton?.getAttribute("aria-hidden")).toBe("true");
    const rows = container.querySelectorAll(".skeleton-row");
    expect(rows.length).toBe(7);
  });

  it("hides skeleton rows when not loading", () => {
    const { container } = render(MessageList, defaultProps);
    expect(container.querySelector(".skeleton-list")).toBeFalsy();
  });

  describe("context menu", () => {
    function getFirstItem() {
      return screen.getAllByRole("option")[0];
    }

    it("opens a plain HTML context menu on right-click", async () => {
      render(MessageList, defaultProps);
      await fireEvent.contextMenu(getFirstItem());

      expect(screen.getByRole("menu")).toBeTruthy();
      expect(screen.getByRole("menuitem", { name: "Antworten" })).toBeTruthy();
      expect(screen.getByRole("menuitem", { name: "Weiterleiten" })).toBeTruthy();
      expect(screen.getByRole("menuitem", { name: "Löschen" })).toBeTruthy();
    });

    it("closes the context menu on outside click", async () => {
      render(MessageList, defaultProps);
      await fireEvent.contextMenu(getFirstItem());
      expect(screen.getByRole("menu")).toBeTruthy();

      await fireEvent.click(screen.getByRole("presentation"));
      expect(screen.queryByRole("menu")).toBeNull();
    });

    it("triggers ondelete callback with correct uid when delete is clicked", async () => {
      const ondelete = vi.fn();
      render(MessageList, { ...defaultProps, ondelete, messages: [makeMessage({ uid: 99 })] });

      await fireEvent.contextMenu(getFirstItem());
      await fireEvent.click(screen.getByRole("menuitem", { name: "Löschen" }));

      expect(ondelete).toHaveBeenCalledOnce();
      expect(ondelete).toHaveBeenCalledWith(99);
    });

    it("does not crash when ondelete is not provided", async () => {
      render(MessageList, { ...defaultProps, ondelete: undefined });

      await fireEvent.contextMenu(getFirstItem());
      expect(() => fireEvent.click(screen.getByRole("menuitem", { name: "Löschen" }))).not.toThrow();
    });

    it("triggers onreply callback with correct uid when reply is clicked", async () => {
      const onreply = vi.fn();
      render(MessageList, { ...defaultProps, onreply, messages: [makeMessage({ uid: 42 })] });

      await fireEvent.contextMenu(getFirstItem());
      await fireEvent.click(screen.getByRole("menuitem", { name: "Antworten" }));

      expect(onreply).toHaveBeenCalledOnce();
      expect(onreply).toHaveBeenCalledWith(42);
    });

    it("triggers onforward callback with correct uid when forward is clicked", async () => {
      const onforward = vi.fn();
      render(MessageList, { ...defaultProps, onforward, messages: [makeMessage({ uid: 43 })] });

      await fireEvent.contextMenu(getFirstItem());
      await fireEvent.click(screen.getByRole("menuitem", { name: "Weiterleiten" }));

      expect(onforward).toHaveBeenCalledOnce();
      expect(onforward).toHaveBeenCalledWith(43);
    });

    it("does not crash when onforward is not provided", async () => {
      render(MessageList, { ...defaultProps, onforward: undefined });

      await fireEvent.contextMenu(getFirstItem());
      expect(() => fireEvent.click(screen.getByRole("menuitem", { name: "Weiterleiten" }))).not.toThrow();
    });

    it("triggers ontoggleRead callback when the read toggle is clicked", async () => {
      const ontoggleRead = vi.fn();
      render(MessageList, { ...defaultProps, ontoggleRead, messages: [makeMessage({ uid: 7 })] });

      await fireEvent.contextMenu(getFirstItem());
      await fireEvent.click(screen.getByRole("menuitem", { name: "Als ungelesen markieren" }));

      expect(ontoggleRead).toHaveBeenCalledOnce();
      expect(ontoggleRead).toHaveBeenCalledWith(7);
    });

    it("triggers ontoggleFlag callback when the flag toggle is clicked", async () => {
      const ontoggleFlag = vi.fn();
      render(MessageList, { ...defaultProps, ontoggleFlag, messages: [makeMessage({ uid: 11 })] });

      await fireEvent.contextMenu(getFirstItem());
      await fireEvent.click(screen.getByRole("menuitem", { name: "Markieren" }));

      expect(ontoggleFlag).toHaveBeenCalledOnce();
      expect(ontoggleFlag).toHaveBeenCalledWith(11);
    });
  });

  describe("visual states", () => {
    it("applies selected class to the selected message", () => {
      const messages = [
        makeMessage({ uid: 1, subject: "Eins" }),
        makeMessage({ uid: 2, subject: "Zwei" }),
      ];
      const { container } = render(MessageList, {
        ...defaultProps,
        messages,
        selectedUids: [2],
      });

      const items = container.querySelectorAll(".message-item");
      expect(items[0].classList.contains("selected")).toBe(false);
      expect(items[1].classList.contains("selected")).toBe(true);
    });

    it("applies unread class to unread messages", () => {
      const messages = [
        makeMessage({ uid: 1, is_read: true }),
        makeMessage({ uid: 2, is_read: false }),
      ];
      const { container } = render(MessageList, {
        ...defaultProps,
        messages,
      });

      const items = container.querySelectorAll(".message-item");
      expect(items[0].classList.contains("unread")).toBe(false);
      expect(items[1].classList.contains("unread")).toBe(true);
    });

    it("applies aria-selected attribute correctly", () => {
      const messages = [
        makeMessage({ uid: 1 }),
        makeMessage({ uid: 2 }),
      ];
      render(MessageList, {
        ...defaultProps,
        messages,
        selectedUids: [2],
      });

      const items = screen.getAllByRole("option");
      expect(items[0].getAttribute("aria-selected")).toBe("false");
      expect(items[1].getAttribute("aria-selected")).toBe("true");
    });
  });

  describe("selection", () => {
    it("calls onselect when a message is clicked without modifiers", async () => {
      const onselect = vi.fn();
      render(MessageList, { ...defaultProps, onselect, messages: [makeMessage({ uid: 7 })] });

      const item = screen.getAllByRole("option")[0];
      await fireEvent.click(item);

      expect(onselect).toHaveBeenCalledOnce();
      expect(onselect).toHaveBeenCalledWith(7);
    });
  });

  describe("draft badge", () => {
    it("shows draft badge when isDraftFolder=true", () => {
      render(MessageList, { ...defaultProps, isDraftFolder: true });
      expect(screen.getByText("Entwurf")).toBeTruthy();
    });

    it("hides draft badge when isDraftFolder=false", () => {
      render(MessageList, { ...defaultProps, isDraftFolder: false });
      expect(screen.queryByText("Entwurf")).toBeNull();
    });

    it("hides draft badge when isDraftFolder is undefined", () => {
      render(MessageList, defaultProps);
      expect(screen.queryByText("Entwurf")).toBeNull();
    });
  });
});
