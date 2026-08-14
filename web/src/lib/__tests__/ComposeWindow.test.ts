import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";
import { fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import ComposeWindow from "$lib/components/ComposeWindow.svelte";

// Bug 1 - An: field should be editable when replying.
// The $effect must not reset to[0] on user keystroke.

vi.mock("$lib/services/tauri", async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return {
    ...actual,
    searchContacts: vi.fn().mockResolvedValue([]),
    aiDraftFromBullets: vi.fn().mockResolvedValue("Draft"),
    aiGenerateReply: vi.fn().mockResolvedValue("Reply"),
    aiFormatText: vi.fn().mockResolvedValue("Formatted"),
    saveDraft: vi.fn().mockResolvedValue({ uid: 0 }),
    discardDraft: vi.fn().mockResolvedValue(undefined),
    getToneProfile: vi.fn().mockResolvedValue(null),
    getVoiceSettings: vi.fn().mockResolvedValue({ enabled: false }),
  };
});

vi.mock("$lib/stores/settings", () => ({
  showDiffEnabled: { subscribe: (fn: (v: boolean) => void) => { fn(false); return () => {}; } },
}));

describe("ComposeWindow - mode new", () => {
  const defaultProps = {
    mode: "new" as const,
    mailChain: [] as { text: string; html: string | null }[],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("renders header for new message", () => {
    render(ComposeWindow, defaultProps);
    expect(screen.getByText("Neue Nachricht")).toBeTruthy();
  });

  it("renders empty fields for new message", () => {
    render(ComposeWindow, defaultProps);
    const inputs = screen.getAllByRole("textbox") as HTMLInputElement[];
    expect(inputs.length).toBeGreaterThanOrEqual(2);
  });

  it("calls onclose when close button is clicked", async () => {
    const onclose = vi.fn();
    render(ComposeWindow, { ...defaultProps, onclose });
    const closeBtn = screen.getByText("\u2715");
    await fireEvent.click(closeBtn);
    expect(onclose).toHaveBeenCalledOnce();
  });

  it("close button has type='button'", () => {
    const { container } = render(ComposeWindow, defaultProps);
    const btn = container.querySelector(".close-btn");
    expect(btn?.getAttribute("type")).toBe("button");
  });

  it("renders send button", () => {
    render(ComposeWindow, defaultProps);
    const sendBtn = screen.getByText("Senden");
    expect(sendBtn).toBeTruthy();
  });

  it("send button has type='button'", () => {
    const { container } = render(ComposeWindow, defaultProps);
    const btn = container.querySelector(".btn-send");
    expect(btn?.getAttribute("type")).toBe("button");
  });

  it("renders generate button", () => {
    render(ComposeWindow, defaultProps);
    expect(screen.getByText("Generieren")).toBeTruthy();
  });

  it("generate button has type='button'", () => {
    const { container } = render(ComposeWindow, defaultProps);
    const btn = container.querySelector(".btn-ai.primary");
    expect(btn?.getAttribute("type")).toBe("button");
  });
});

describe("ComposeWindow - mode reply", () => {
  const replyProps = {
    mode: "reply" as const,
    mailChain: [{ text: "Original message", html: null }],
    replyTo: "test@example.com",
    replySubject: "Meeting",
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("renders header for reply", () => {
    render(ComposeWindow, replyProps);
    expect(screen.getByText("Antworten")).toBeTruthy();
  });

  it("renders original message chain", () => {
    render(ComposeWindow, replyProps);
    expect(screen.getByText("Original message", { exact: false })).toBeTruthy();
  });
});

describe("ComposeWindow - error states", () => {
  it("shows send error banner when sendError is set", () => {
    const props = {
      mode: "new" as const,
      mailChain: [] as { text: string; html: string | null }[],
      sendError: "SMTP connection failed",
      onclose: vi.fn(),
      onsend: vi.fn().mockResolvedValue(undefined),
    };
    render(ComposeWindow, props);
    expect(screen.getByText("SMTP connection failed")).toBeTruthy();
  });

  it("does not show error banner when sendError is null", () => {
    const props = {
      mode: "new" as const,
      mailChain: [] as { text: string; html: string | null }[],
      sendError: null,
      onclose: vi.fn(),
      onsend: vi.fn().mockResolvedValue(undefined),
    };
    const { container } = render(ComposeWindow, props);
    expect(container.querySelector(".error-banner")).toBeFalsy();
  });
});

describe("ComposeWindow - An: Feld editierbar (Bug 1)", () => {
  const replyProps = {
    mode: "reply" as const,
    mailChain: [{ text: "Original", html: null }],
    replyTo: "alice@example.com",
    replySubject: "Meeting",
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("pre-fills the An: field with replyTo address", () => {
    render(ComposeWindow, replyProps);
    expect(screen.getByText("alice@example.com")).toBeTruthy();
  });

  it("pre-fills the Betreff field with Re: prefix", () => {
    render(ComposeWindow, replyProps);
    const input = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    expect(input.value).toBe("Re: Meeting");
  });

  it("allows user to edit the An: field without resetting to replyTo", async () => {
    render(ComposeWindow, replyProps);
    const chipRemove = screen.getByText("alice@example.com").parentElement?.querySelector(".chip-remove") as HTMLButtonElement;
    if (chipRemove) await fireEvent.click(chipRemove);
    const input = screen.getByPlaceholderText("Name oder E-Mail-Adresse") as HTMLInputElement;
    input.value = "bob@example.com";
    await fireEvent.input(input);
    await tick();
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByText("bob@example.com")).toBeTruthy();
  });

  it("allows user to replace the An: field entirely", async () => {
    render(ComposeWindow, replyProps);
    const chipRemove = screen.getByText("alice@example.com").parentElement?.querySelector(".chip-remove") as HTMLButtonElement;
    if (chipRemove) await fireEvent.click(chipRemove);
    expect(screen.queryByText("alice@example.com")).toBeNull();
    const input = screen.getByPlaceholderText("Name oder E-Mail-Adresse") as HTMLInputElement;
    input.value = "carol@example.org";
    await fireEvent.input(input);
    await tick();
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByText("carol@example.org")).toBeTruthy();
  });
});

describe("ComposeWindow - field validation (send button disabled state)", () => {
  const defaultProps = {
    mode: "new" as const,
    mailChain: [] as { text: string; html: string | null }[],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("send button is disabled when An: field is empty (all fields empty)", () => {
    render(ComposeWindow, defaultProps);
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
  });

  it("send button is disabled when An: field is empty (subject and body filled)", async () => {
    render(ComposeWindow, defaultProps);
    const subjectInput = screen.getByPlaceholderText("Betreff");
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/);
    await fireEvent.input(subjectInput, { target: { value: "Test Betreff" } });
    await fireEvent.input(bodyInput, { target: { value: "Test body content" } });
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
  });

  async function fillTo(value: string) {
    const toInput = screen.getByPlaceholderText("Name oder E-Mail-Adresse") as HTMLInputElement;
    toInput.value = value;
    await fireEvent.input(toInput);
    await tick();
    await fireEvent.keyDown(toInput, { key: "Enter" });
    await tick();
  }

  it("send button is disabled when Betreff field is empty (to and body filled)", async () => {
    render(ComposeWindow, defaultProps);
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/);
    await fillTo("user@example.com");
    await fireEvent.input(bodyInput, { target: { value: "Test body content" } });
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
  });

  it("send button is disabled when body field is empty (to and subject filled)", async () => {
    render(ComposeWindow, defaultProps);
    const subjectInput = screen.getByPlaceholderText("Betreff");
    await fillTo("user@example.com");
    await fireEvent.input(subjectInput, { target: { value: "Test Betreff" } });
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
  });

  it("send button becomes enabled when all required fields are filled", async () => {
    render(ComposeWindow, defaultProps);
    const subjectInput = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/) as HTMLTextAreaElement;
    await fillTo("user@example.com");
    await waitFor(() => {
      expect(screen.getByText("user@example.com")).toBeTruthy();
    });
    subjectInput.value = "Test Betreff";
    await fireEvent.input(subjectInput);
    bodyInput.value = "Test body content";
    await fireEvent.input(bodyInput);
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(false);
  });

  it("send button disabled attribute prevents click when An: field is empty", async () => {
    const onsend = vi.fn().mockResolvedValue(undefined);
    render(ComposeWindow, { ...defaultProps, onsend });
    const subjectInput = screen.getByPlaceholderText("Betreff");
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/);
    await fireEvent.input(subjectInput, { target: { value: "Test Betreff" } });
    await fireEvent.input(bodyInput, { target: { value: "Test body content" } });
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
    expect(sendBtn.hasAttribute("disabled")).toBe(true);
  });

  it("onsend callback is called with correct data when all fields filled", async () => {
    const onsend = vi.fn().mockResolvedValue(undefined);
    render(ComposeWindow, { ...defaultProps, onsend });
    const subjectInput = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/) as HTMLTextAreaElement;
    await fillTo("user@example.com");
    await waitFor(() => {
      expect(screen.getByText("user@example.com")).toBeTruthy();
    });
    subjectInput.value = "Test Betreff";
    await fireEvent.input(subjectInput);
    bodyInput.value = "Test body content";
    await fireEvent.input(bodyInput);
    const sendBtn = screen.getByText("Senden") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(false);
    await fireEvent.click(sendBtn);
    expect(onsend).toHaveBeenCalledOnce();
    expect(onsend).toHaveBeenCalledWith({
      to: "user@example.com",
      subject: "Test Betreff",
      body: "Test body content",
      bodyHtml: expect.any(String),
      cc: undefined,
      bcc: undefined,
      aiDraft: null,
    });
  });
});

describe("ComposeWindow - draft functionality", () => {
  const draftProps = {
    mode: "new" as const,
    mailChain: [] as { text: string; html: string | null }[],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("pre-fills fields from draft props", async () => {
    render(ComposeWindow, {
      ...draftProps,
      draftTo: "alice@example.com",
      draftSubject: "Entwurf Betreff",
      draftBody: "Entwurf Inhalt",
      draftUid: 42,
    });
    await waitFor(() => {
      expect(screen.getByText("alice@example.com")).toBeTruthy();
    });
    const subjectInput = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    expect(subjectInput.value).toBe("Entwurf Betreff");
    const bodyInput = screen.getByPlaceholderText(/Gib Deine/) as HTMLTextAreaElement;
    expect(bodyInput.value).toBe("Entwurf Inhalt");
  });
});

describe("ComposeWindow - initial attachments", () => {
  const draftProps = {
    mode: "new" as const,
    mailChain: [] as { text: string; html: string | null }[],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("renders attachment pills pre-filled from initialAttachments", async () => {
    render(ComposeWindow, {
      ...draftProps,
      initialAttachments: [
        { filename: "draft.pdf", content: "bXlkYXRh", contentType: "application/pdf", size: 6 },
      ],
    });
    await waitFor(() => {
      expect(screen.getByText(/draft\.pdf/)).toBeTruthy();
    });
  });

  it("passes current attachments when saving a draft", async () => {
    const { saveDraft } = await import("$lib/services/tauri");
    const saveDraftMock = saveDraft as ReturnType<typeof vi.fn>;
    saveDraftMock.mockClear();
    saveDraftMock.mockResolvedValue({ uid: 7 });

    render(ComposeWindow, {
      ...draftProps,
      accountId: 1,
      draftTo: "bob@example.com",
      draftBody: "Inhalt",
      draftUid: 5,
      initialAttachments: [
        { filename: "anhang.pdf", content: "aGFsbG8=", contentType: "application/pdf", size: 5 },
      ],
    });
    await waitFor(() => {
      expect(screen.getByText(/anhang\.pdf/)).toBeTruthy();
    });
    // draft pre-fill populates userInput, so the close dialog offers "Speichern".
    await waitFor(() => {
      const bodyInput = screen.getByPlaceholderText(/Gib Deine/) as HTMLTextAreaElement;
      expect(bodyInput.value).toBe("Inhalt");
    });

    const closeBtn = screen.getByText("\u2715");
    await fireEvent.click(closeBtn);
    const saveBtn = screen.getByText("Speichern");
    await fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(saveDraftMock).toHaveBeenCalled();
    });
    const lastCall = saveDraftMock.mock.calls.at(-1) as unknown[];
    const attachmentsArg = lastCall[8] as { filename: string; content: string }[];
    expect(Array.isArray(attachmentsArg)).toBe(true);
    expect(attachmentsArg.length).toBe(1);
    expect(attachmentsArg[0].filename).toBe("anhang.pdf");
    expect(attachmentsArg[0].content).toBe("aGFsbG8=");
  });
});

describe("ComposeWindow - forward mode with attachments", () => {
  const forwardProps = {
    mode: "forward" as const,
    mailChain: [{ text: "Original mail body", html: null }],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("renders original attachment pills in forward mode (lazy, no content)", async () => {
    render(ComposeWindow, {
      ...forwardProps,
      replySubject: "Original Subject",
      initialAttachments: [
        { id: 11, filename: "scan.pdf", content: "", contentType: "application/pdf", size: 100 },
      ],
    });
    await waitFor(() => {
      expect(screen.getByText(/scan\.pdf/)).toBeTruthy();
    });
  });

  it("passes the attachment id through on send (content resolved lazily by parent)", async () => {
    const onsend = vi.fn().mockResolvedValue(undefined);
    render(ComposeWindow, {
      ...forwardProps,
      onsend,
      replySubject: "Original Subject",
      initialAttachments: [
        { id: 11, filename: "scan.pdf", content: "", contentType: "application/pdf", size: 100 },
      ],
    });
    await waitFor(() => {
      expect(screen.getByText(/scan\.pdf/)).toBeTruthy();
    });

    // Fill required fields so handleSend can run.
    const toInput = screen.getByPlaceholderText("Name oder E-Mail-Adresse") as HTMLInputElement;
    await fireEvent.input(toInput, { target: { value: "bob@example.com" } });
    const subjectInput = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    await fireEvent.input(subjectInput, { target: { value: "Fwd: Original Subject" } });

    const sendBtn = screen.getByText("Senden");
    await fireEvent.click(sendBtn);

    await waitFor(() => {
      expect(onsend).toHaveBeenCalled();
    });
    const data = onsend.mock.calls.at(-1)![0] as { attachments?: { id?: number; filename: string; content: string }[] };
    expect(data.attachments?.length).toBe(1);
    expect(data.attachments![0].id).toBe(11);
    // content is empty: the parent resolves it lazily at send time.
    expect(data.attachments![0].content).toBe("");
    expect(data.attachments![0].filename).toBe("scan.pdf");
  });
});

describe("ComposeWindow - mic divider", () => {
  const dividerProps = {
    mode: "new" as const,
    mailChain: [] as { text: string; html: string | null }[],
    onclose: vi.fn(),
    onsend: vi.fn().mockResolvedValue(undefined),
  };

  it("renders no mic-divider when voice is disabled", async () => {
    const { container } = render(ComposeWindow, dividerProps);
    await waitFor(() => {
      expect(container.querySelector(".mic-divider")).toBeNull();
    });
  });

  it("does not render the mic-divider (feature disabled) even when voice is enabled", async () => {
    const { getVoiceSettings } = await import("$lib/services/tauri");
    (getVoiceSettings as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ enabled: true });

    const { container } = render(ComposeWindow, dividerProps);
    await waitFor(() => {
      expect(container.querySelector(".mic-divider")).toBeNull();
    });
  });
});
