<script lang="ts">
  import DiffEditor from "./DiffEditor.svelte";
  import ToneControls from "./ToneControls.svelte";
  import { t } from "$lib/i18n";
  import RecipientInput from "./RecipientInput.svelte";
  import {
    aiGenerateMail, aiSuggestRecipient, aiSuggestSubject, aiFormatText, getToneProfile,
    saveDraft, openFilePicker,
    getVoiceSettings, voiceTranscribe,
    resolveRecipientFromText,
  } from "$lib/services/tauri";
  import { get } from "svelte/store";
  import { showDiffEnabled } from "$lib/stores/settings";
  import { textToHtml, wrapHtmlQuote, sanitizeHtml } from "$lib/utils/format";
  import { blobToWavBase64 } from "$lib/utils/wav";
  import type { MailChainEntry } from "$lib/types/mail";
  import ErrorBanner from "$lib/components/ErrorBanner.svelte";

  type ComposeMode = "new" | "reply" | "forward";
  export type ToneValues = { seriositaet: number; textumfang: number };

  interface Props {
    mode: ComposeMode;
    mailChain: MailChainEntry[];
    sendError?: string | null;
    replySubject?: string;
    replyTo?: string;
    accountId?: number;
    recipientEmail?: string;
    senderName?: string;
    recipientName?: string;
    onclose: () => void;
    onsend: (data: { to: string; subject: string; body: string; bodyHtml: string; cc?: string; bcc?: string; attachments?: { id?: number; filename: string; content: string; contentType: string }[]; aiDraft?: string | null }) => Promise<void>;
    /** Fired after a draft was persisted, so the parent can keep the edited draft uid. */
    ondraftSaved?: (uid: number) => void;
    // Pre-filled draft data
    draftTo?: string;
    draftCc?: string;
    draftSubject?: string;
    draftBody?: string;
    draftUid?: number | null;
    /** Pre-filled attachments (drafts, forward). Each item carries base64 content. */
    initialAttachments?: { id?: number; filename: string; content: string; contentType: string; size: number }[];
    /** Assistant hand-off: pre-fill a fresh compose (recipient + subject + drafted body). */
    prefill?: { to: string; subject: string; body: string } | null;
  }

  let {
    mode, mailChain = [], sendError = null, replySubject = "", replyTo = "",
    accountId, recipientEmail, senderName = "", recipientName = "", onclose, onsend,
    ondraftSaved, draftTo = "", draftCc = "", draftSubject = "", draftBody = "", draftUid = null,
    initialAttachments = [], prefill = null,
  }: Props = $props();

  let to = $state<string[]>([]);
  let cc = $state<string[]>([]);
  let bcc = $state<string[]>([]);
  interface ComposeAttachment {
    id?: number;
    filename: string;
    content: string;
    contentType: string;
    size: number;
  }
  let attachments = $state<ComposeAttachment[]>([]);

  // Pre-fill attachments from the prop (draft reopen, forward). Runs once per
  // open; subsequent changes are driven by the user via add/removeAttachment.
  $effect(() => {
    const seed = initialAttachments ?? [];
    if (seed.length > 0 && attachments.length === 0) {
      attachments = seed.map((a) => ({
        id: a.id,
        filename: a.filename,
        content: a.content,
        contentType: a.contentType,
        size: a.size,
      }));
    }
  });
  // Cc and Bcc are independently toggled. Stays open if the field has content.
  let showCc = $state(false);
  let showBcc = $state(false);
  let ccVisible = $derived(showCc || cc.length > 0);
  let bccVisible = $derived(showBcc || bcc.length > 0);
  let subject = $state("");
  let userInput = $state("");
  let editorEl: HTMLElement | undefined = $state();
  let aiDraft = $state<string | null>(null);
  let showDiff = $state(false);
  let tone = $state<ToneValues>({ seriositaet: 4, textumfang: 4 });
  let toneLoaded = $state(false);
  let isGenerating = $state(false);
  let generationError = $state<string | null>(null);
  let generationStep = $state(0); // 0=none, 1=text, 2=filling fields
  let generationStatus = $state("");

  // Voice-to-Mail state
  let isRecording = $state(false);
  let mediaRecorder: MediaRecorder | null = null;
  let audioChunks: Blob[] = [];
  let voiceError = $state<string | null>(null);
  let voiceEnabled = $state(false);

  // Load voice settings on mount
  $effect(() => {
    getVoiceSettings().then(vs => {
      voiceEnabled = vs?.enabled ?? false;
    }).catch(() => {});
  });

  // Narrow (mobile) detection — reduces the editor height (-30%) on phones
  // via the `rows` attribute without affecting the desktop layout.
  let isNarrow = $state(false);
  $effect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(max-width: 600px)");
    const update = () => { isNarrow = mq.matches; };
    update();
    mq.addEventListener("change", update);
    return () => mq.removeEventListener("change", update);
  });

  // Draft persistence (local — distinct from the prop `draftUid`)
  let localDraftUid = $state<number | null>(null);
  let isSavingDraft = $state(false);
  let showCloseDialog = $state(false);

  async function doSaveDraft(): Promise<boolean> {
    if (isSavingDraft) return false;
    isSavingDraft = true;
    try {
      const result = await saveDraft(
        accountId ?? 0,
        to,
        subject,
        userInput,
        undefined,
        cc.length > 0 ? cc : undefined,
        bcc.length > 0 ? bcc : undefined,
        draftUid ?? localDraftUid,
        attachments.length > 0 ? attachments : [],
      );
      localDraftUid = result.uid;
      lastPropDraftUid = result.uid;
      ondraftSaved?.(result.uid);
      return true;
    } catch (e: unknown) {
      console.warn("Entwurf speichern fehlgeschlagen", e);
      return false;
    } finally {
      isSavingDraft = false;
    }
  }

  function handleClose() {
    if (userInput.trim()) {
      showCloseDialog = true;
    } else {
      onclose();
    }
  }

  async function handleCloseSave() {
    await doSaveDraft();
    showCloseDialog = false;
    onclose();
  }

  function handleCloseDiscard() {
    showCloseDialog = false;
    onclose();
  }

  // ─── Voice-to-Mail ───────────────────────────────────────────────

  async function toggleVoiceInput() {
    if (isRecording) {
      stopRecording();
    } else {
      await startRecording();
    }
  }

  async function startRecording() {
    voiceError = null;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      mediaRecorder = new MediaRecorder(stream);
      audioChunks = [];

      mediaRecorder.ondataavailable = (e) => {
        if (e.data.size > 0) audioChunks.push(e.data);
      };

      mediaRecorder.onstop = async () => {
        // Stop all tracks
        stream.getTracks().forEach(track => track.stop());

        if (audioChunks.length === 0) {
          voiceError = "Kein Audio aufgezeichnet.";
          return;
        }

        const audioBlob = new Blob(audioChunks, { type: 'audio/webm' });
        // MediaRecorder produces browser-specific containers (WebM/Opus,
        // MP4) — decode + re-encode as real 16 kHz PCM WAV, which the STT
        // endpoint (Whisper / vLLM) can actually parse.
        const base64 = await blobToWavBase64(audioBlob);

        // Transcribe
        isGenerating = true;
        generationStep = 2;
        generationStatus = "Transkribiere...";

        try {
          const transcript = await voiceTranscribe(base64);
          if (transcript.trim()) {
            setEditorText(transcript);
            // Auto-generate after successful transcription
            await generate();
          } else {
            voiceError = "Kein Text erkannt.";
            isGenerating = false;
            generationStep = 0;
            generationStatus = "";
          }
        } catch (e: unknown) {
          voiceError = e instanceof Error ? e.message : String(e);
          isGenerating = false;
          generationStep = 0;
          generationStatus = "";
        }
      };

      mediaRecorder.start();
      isRecording = true;

      // Auto-stop after 2 seconds of silence (RMS-based detection).
      let lastVoiceAt = 0;
      const audioContext = new AudioContext();
      const source = audioContext.createMediaStreamSource(stream);
      const analyser = audioContext.createAnalyser();
      analyser.fftSize = 1024;
      analyser.smoothingTimeConstant = 0.4;
      source.connect(analyser);

      const rmsLevel = (): number => {
        const buf = new Float32Array(analyser.fftSize);
        analyser.getFloatTimeDomainData(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
        return Math.sqrt(sum / buf.length);
      };

      const checkSilence = () => {
        if (!isRecording) return;
        const rms = rmsLevel();
        if (rms >= 0.01) {
          // Voice detected — reset the silence window.
          lastVoiceAt = Date.now();
        } else if (Date.now() - lastVoiceAt >= 2000) {
          // 2 seconds without speech → stop and transcribe immediately.
          void audioContext.close().catch(() => {});
          stopRecording();
          return;
        }
        requestAnimationFrame(checkSilence);
      };
      lastVoiceAt = Date.now();
      checkSilence();

    } catch (e: unknown) {
      voiceError = `Mikrofon-Zugriff fehlgeschlagen: ${e instanceof Error ? e.message : String(e)}`;
      isRecording = false;
    }
  }

  function stopRecording() {
    if (mediaRecorder && mediaRecorder.state !== 'inactive') {
      mediaRecorder.stop();
    }
    isRecording = false;
  }

  let lastMode = $state<ComposeMode | null>(null);
  let lastReplyTo = $state<string | null>(null);
  let lastPropDraftUid = $state<number | null>(null);
  $effect(() => {
    if (mode !== lastMode || replyTo !== lastReplyTo || draftUid !== lastPropDraftUid) {
      // Pre-fill only when a *different* draft is being opened. `doSaveDraft`
      // keeps `lastPropDraftUid` in sync after a save, so adopting the returned
      // uid does not wipe the freshly typed content.
      if (draftUid != null && draftUid !== lastPropDraftUid) {
        // Pre-fill from draft data
        to = draftTo ? draftTo.split(",").map(s => s.trim()).filter(Boolean) : [];
        cc = draftCc ? draftCc.split(",").map(s => s.trim()).filter(Boolean) : [];
        bcc = [];
        subject = draftSubject;
        setEditorText(draftBody);
        localDraftUid = draftUid;
        lastPropDraftUid = draftUid;
      } else {
        to = replyTo ? [replyTo] : [];
        cc = [];
        bcc = [];
        subject = mode === "reply" ? `Re: ${replySubject}` : mode === "forward" ? `Fwd: ${replySubject}` : "";
      }
      toneLoaded = false;
      lastMode = mode;
      lastReplyTo = replyTo;
    }
  });

  // Assistant hand-off: pre-fill a fresh compose with recipient + subject + drafted body.
  $effect(() => {
    if (prefill) {
      to = prefill.to ? [prefill.to] : [];
      cc = [];
      bcc = [];
      subject = prefill.subject;
      setEditorText(prefill.body);
      aiDraft = null;
      toneLoaded = false;
    }
  });

  // Load tone profile when recipient changes
  $effect(() => {
    const recipient = to[0]?.trim();
    if (recipient && accountId && !toneLoaded) {
      getToneProfile(accountId, recipient).then(profile => {
        if (profile && profile.sample_count > 0) {
          // Map formality (0-1) to seriositaet (1-7)
          tone.seriositaet = Math.round(profile.formality_score * 6 + 1);
          // textumfang stays at default — no direct mapping from profile
        }
        toneLoaded = true;
      }).catch(() => {
        // Profile not found or error — use defaults
        toneLoaded = true;
      });
    }
  });

  async function generate() {
    isGenerating = true;
    generationError = null;
    generationStep = 1;
    generationStatus = "Ermittle Adresse...";
    try {
      let originalMessage: string | undefined;
      if (mode === "reply" && mailChain.length > 0) {
        originalMessage = mailChain.map(m => m.text).join("\n\n---\n\n");
      }

      // Step 1: Extract recipient from contacts (fast, no LLM)
      if (!to[0]?.trim()) {
        const contact = await resolveRecipientFromText(userInput).catch(() => null);
        if (contact?.email) {
          to = [contact.email];
        } else {
          const email = await aiSuggestRecipient("", subject, userInput, originalMessage).catch(() => "");
          if (email.includes("@")) to = [email.trim()];
        }
      }

      // Step 2: Generate main text
      generationStep = 2;
      generationStatus = "Generiere Text...";
      const result = await aiGenerateMail(
        accountId ?? 0,
        to[0] || "",
        subject || "",
        userInput,
        senderName,
        tone.seriositaet,
        tone.textumfang,
        originalMessage,
      );

      // Step 3: Suggest subject if empty
      generationStep = 3;
      generationStatus = "Ermittle fehlende Felder...";
      if (!subject.trim()) {
        const s = await aiSuggestSubject(to[0] || "", "", userInput, originalMessage).catch(() => "");
        if (s.trim()) subject = s.trim();
      }

      // Set result
      aiDraft = result;
      generationStep = 0; // Done
      generationStatus = "";

      if (get(showDiffEnabled)) {
        showDiff = true;
      } else {
        setEditorText(result);
      }
    } catch (e: unknown) {
      generationError = e instanceof Error ? e.message : String(e);
    } finally {
      isGenerating = false;
    }
  }

  function handleGenerateClick() {
    if (isRecording) {
      toggleVoiceInput();
    } else {
      generate();
    }
  }

  function handleMicToggle(e: MouseEvent) {
    e.stopPropagation();
    if (voiceEnabled) toggleVoiceInput();
  }

  async function handleFormat() {
    if (!userInput.trim()) return;
    isGenerating = true;
    try {
      setEditorText(await aiFormatText(userInput));
    } catch (e: unknown) {
      generationError = e instanceof Error ? e.message : String(e);
    } finally {
      isGenerating = false;
    }
  }

  function acceptDraft() {
    setEditorText(aiDraft ?? userInput);
    aiDraft = null;
    showDiff = false;
  }

  function rejectDraft() {
    aiDraft = null;
    showDiff = false;
  }

  function syncInput() {
    if (editorEl) userInput = editorEl.innerText ?? editorEl.textContent ?? "";
  }

  function setEditorText(text: string) {
    if (editorEl) {
      editorEl.textContent = text;
      userInput = text;
    }
  }

  function execCmd(cmd: string, value?: string) {
    editorEl?.focus();
    document.execCommand(cmd, false, value);
    syncInput();
  }

  function execLink() {
    const url = prompt($t("compose.linkPrompt"));
    if (url) execCmd("createLink", url);
  }

  function execCode() {
    const sel = window.getSelection();
    if (!sel || !sel.toString()) return;
    const text = sel.toString().replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    execCmd("insertHTML", `<code>${text}</code>&nbsp;`);
  }

  function execHeading() {
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;
    const node = sel.anchorNode?.parentElement;
    if (node?.tagName === "H3") {
      execCmd("formatBlock", "P");
    } else {
      execCmd("formatBlock", "H3");
    }
  }

  async function handleSend() {
    let body = userInput;
    let bodyHtml = editorEl?.innerHTML || textToHtml(userInput);
    if (mailChain.length > 0) {
      body += "\n\n" + mailChain.map(m => "> " + m.text.replace(/\n/g, "\n> ")).join("\n\n");
      const htmlQuotes = mailChain.map(m =>
        m.html ? wrapHtmlQuote(sanitizeHtml(m.html)) : wrapHtmlQuote(textToHtml(m.text))
      ).join("\n");
      bodyHtml += "\n" + htmlQuotes;
    }
    try {
      await onsend({
        to: to.filter((t) => t.trim()).join(", "),
        subject,
        body,
        bodyHtml,
        cc: cc.length > 0 ? cc.join(", ") : undefined,
        bcc: bcc.length > 0 ? bcc.join(", ") : undefined,
        attachments: attachments.length > 0 ? attachments.map(a => ({
          id: a.id,
          filename: a.filename,
          content: a.content,
          contentType: a.contentType,
        })) : undefined,
        aiDraft,
      });
    } catch (e) {
      console.warn("ComposeWindow handleSend error (handled by parent)", e);
    }
  }

  async function addAttachment() {
    const files = await openFilePicker();
    for (const file of files) {
      const ext = file.filename.split('.').pop()?.toLowerCase() || '';
      const contentType = guessContentType(file.filename, ext);
      attachments = [...attachments, {
        filename: file.filename,
        content: file.content,
        contentType,
        size: file.size,
      }];
    }
  }

  function removeAttachment(idx: number) {
    attachments = attachments.filter((_, i) => i !== idx);
  }

  function guessContentType(filename: string, ext: string): string {
    const types: Record<string, string> = {
      pdf: 'application/pdf',
      jpg: 'image/jpeg', jpeg: 'image/jpeg',
      png: 'image/png', gif: 'image/gif', svg: 'image/svg+xml', webp: 'image/webp',
      txt: 'text/plain', csv: 'text/csv', html: 'text/html', xml: 'application/xml',
      json: 'application/json', js: 'application/javascript', ts: 'text/plain',
      zip: 'application/zip', gz: 'application/gzip', '7z': 'application/x-7z-compressed',
      doc: 'application/msword', docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
      xls: 'application/vnd.ms-excel', xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      ppt: 'application/vnd.ms-powerpoint', pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
      mp3: 'audio/mpeg', mp4: 'video/mp4', wav: 'audio/wav',
    };
    return types[ext] || 'application/octet-stream';
  }

  function formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="compose-window">
  <div class="compose-header">
    <h2>{mode === "new" ? $t("compose.newMessage") : mode === "forward" ? $t("compose.forwardTitle") : $t("compose.replyTitle")}</h2>
    <button type="button" class="close-btn" onclick={handleClose} title={$t("compose.close")} aria-label={$t("compose.close")}>
      <span class="close-icon-desktop">&#x2715;</span>
      <span class="close-icon-mobile">&#8592; {$t("compose.back")}</span>
    </button>
  </div>

  <div class="compose-body">
    <div class="field">
      <label for="to">{$t("compose.toLabel")}</label>
     <div class="to-row">
        <RecipientInput bind:value={to} {accountId} />
        <span class="ccbcc-group">
        {#if !ccVisible}
          <button type="button" class="ccbcc-toggle" onclick={() => showCc = true} title={$t("compose.ccAdd")}>
            Cc
          </button>
        {/if}
        {#if !bccVisible}
          <button type="button" class="ccbcc-toggle" onclick={() => showBcc = true} title={$t("compose.bccAdd")}>
            Bcc
          </button>
        {/if}
        </span>
      </div>
    </div>
    {#if ccVisible}
      <div class="field">
        <label>{$t("compose.ccLabel")}</label>
        <div class="ccbcc-input-wrapper">
          <RecipientInput bind:value={cc} {accountId} />
          <button type="button" class="ccbcc-clear-btn" onclick={() => { cc = []; showCc = false; }} title={$t("compose.ccRemove")}>&times;</button>
        </div>
      </div>
    {/if}
    {#if bccVisible}
      <div class="field">
        <label>{$t("compose.bccLabel")}</label>
        <div class="ccbcc-input-wrapper">
          <RecipientInput bind:value={bcc} {accountId} />
          <button type="button" class="ccbcc-clear-btn" onclick={() => { bcc = []; showBcc = false; }} title={$t("compose.bccRemove")}>&times;</button>
        </div>
      </div>
    {/if}
    <div class="field">
      <label for="subject">{$t("compose.subjectLabel")}</label>
      <div class="to-row">
        <input id="subject" type="text" bind:value={subject} placeholder={$t("compose.subject")} />
        <span class="ccbcc-group">
          <button type="button" class="ccbcc-toggle" onclick={addAttachment} title={$t("compose.attachFile")}>
            {$t("compose.attachment")}
          </button>
        </span>
      </div>
    </div>

    {#if (mode === "reply" || mode === "forward") && mailChain.length > 0}
      <div class="chain-preview">
        <div class="chain-header">{$t("compose.originalMessage")}</div>
        <div class="chain-scroll-area">
          {#each mailChain as msg}
            <div class="chain-msg">
              {#if msg.html}
                <div class="chain-body-html">{@html sanitizeHtml(msg.html).slice(0, 1000)}{msg.html.length > 1000 ? "..." : ""}</div>
              {:else}
                <pre class="chain-body">{msg.text.slice(0, 1000)}{msg.text.length > 1000 ? "..." : ""}</pre>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <div class="tone-section">
      <ToneControls bind:values={tone} />
    </div>

    {#if showDiff && aiDraft}
      <DiffEditor original={userInput} modified={aiDraft} onaccept={acceptDraft} onreject={rejectDraft} />
    {:else}
      <div class="editor-preview">
        <div class="editor-resize">
          <div class="editor-header">
          <span>Deine Nachricht:</span>
          <div class="fmt-toolbar">
            <button type="button" class="fmt-btn" onclick={() => execCmd("bold")} title="Fett">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 5h6a3.5 3.5 0 0 1 0 7H7z"/><path d="M7 12h7a3.5 3.5 0 0 1 0 7H7z"/></svg>
            </button>
            <button type="button" class="fmt-btn" onclick={() => execCmd("italic")} title="Kursiv">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><line x1="19" y1="4" x2="10" y2="4"/><line x1="14" y1="20" x2="5" y2="20"/><line x1="15" y1="4" x2="9" y2="20"/></svg>
            </button>
            <button type="button" class="fmt-btn" onclick={execHeading} title="Überschrift">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4v16"/><path d="M18 4v16"/><path d="M6 12h12"/></svg>
            </button>
            <button type="button" class="fmt-btn" onclick={() => execCmd("insertUnorderedList")} title="Liste">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><line x1="9" y1="6" x2="20" y2="6"/><line x1="9" y1="12" x2="20" y2="12"/><line x1="9" y1="18" x2="20" y2="18"/><circle cx="4.5" cy="6" r="1" fill="currentColor"/><circle cx="4.5" cy="12" r="1" fill="currentColor"/><circle cx="4.5" cy="18" r="1" fill="currentColor"/></svg>
            </button>
            <button type="button" class="fmt-btn" onclick={execLink} title="Link">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>
            </button>
            <button type="button" class="fmt-btn" onclick={execCode} title="Code">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
            </button>
          </div>
        </div>
          <div class="editor-wrapper" class:has-attachments={attachments.length > 0}>
        <div
          class="editor"
          contenteditable="true"
          bind:this={editorEl}
          oninput={syncInput}
          data-placeholder={$t("compose.messagePlaceholder")}
          aria-label={$t("compose.messageAria")}
        ></div>
        {#if isGenerating && generationStep > 0}
          <div class="generation-status">
            <span class="dot"></span>
            <span class="dot"></span>
            <span class="dot"></span>
          </div>
        {/if}
       {#if attachments.length > 0}
          <div class="editor-attachments">
            {#each attachments as att, i (i)}
              <span class="attachment-pill">
                <span class="attachment-label" title={att.filename}>{att.filename} ({formatFileSize(att.size)})</span>
                <button type="button" class="attachment-remove" onclick={() => removeAttachment(i)} title="Entfernen">&times;</button>
              </span>
            {/each}
      </div>
    {/if}
        </div>
      </div>
    </div>
  {/if}

    {#if voiceError}
      <ErrorBanner message={voiceError} onretry={() => voiceError = null} retryLabel={$t("compose.close")} />
    {/if}
    {#if generationError}
      <ErrorBanner message={generationError} onretry={generate} retryLabel={$t("compose.retryGenerate")} />
    {/if}
    {#if sendError}
      <ErrorBanner message={sendError} onretry={handleSend} retryLabel={$t("compose.retrySend")} />
    {/if}

  </div>

  <div class="editor-toolbar">
    <button type="button" class="btn-ai primary" class:recording={isRecording} onclick={handleGenerateClick} disabled={isGenerating}>
      <span class="toggle-mic" class:voice-enabled={voiceEnabled} onclick={handleMicToggle} title={isRecording ? $t("compose.recordingStop") : $t("compose.dictationStart")}>
        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
        </svg>
      </span>
      {#if false && voiceEnabled}
        <span class="mic-divider" aria-hidden="true"></span>
      {/if}
      <span class="btn-label">
        {#if isGenerating}
          {generationStep === 1 ? $t("compose.generatingText") : $t("compose.detectingFields")}
        {:else if isRecording}
          {$t("compose.recording")}
        {:else if mode === "reply"}
          <span class="label-long">{$t("compose.generateReply")}</span><span class="label-short">{$t("compose.generate")}</span>
        {:else}
          {$t("compose.generate")}
        {/if}
      </span>
    </button>
    <button type="button" class="btn-ai" onclick={handleFormat} disabled={isGenerating || !userInput.trim()}>
      {$t("compose.format")}
    </button>
    <div class="spacer"></div>
    <button type="button" class="btn-send" onclick={handleSend} disabled={!to[0]?.trim() || !subject.trim() || !userInput.trim()}>
      {$t("compose.send")}
    </button>
  </div>
</div>

{#if showCloseDialog}
  <div class="close-dialog-overlay" role="presentation" onclick={handleCloseDiscard}>
    <div class="close-dialog" role="presentation" onclick={(e) => e.stopPropagation()}>
      <p class="close-dialog-title">{$t("compose.saveDraft")}</p>
      <div class="close-dialog-actions">
        <button type="button" class="btn-discard" onclick={handleCloseDiscard}>{$t("compose.discard")}</button>
        <button type="button" class="btn-save" onclick={handleCloseSave}>{$t("compose.save")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  @keyframes scaleIn {
    from { opacity: 0; transform: scale(0.95); }
    to { opacity: 1; transform: scale(1); }
  }
  .compose-window {
    border: 1px solid var(--color-border);
    border-radius: 12px;
    background: var(--color-list);
    overflow: hidden;
    margin: 16px;
    box-shadow: none;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .compose-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 20px;
    background: var(--color-list);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .compose-header h2 { font-size: 1rem; font-weight: 600; color: var(--color-text); }
  .close-btn {
    background: none; border: none; cursor: pointer;
    font-size: 1rem; color: var(--color-text-secondary);
    padding: 4px 8px; border-radius: 6px;
    transition: all 0.15s ease;
  }
  .close-btn:hover { background: var(--color-sidebar); color: var(--color-text); }
  .close-icon-mobile { display: none; }
  .compose-body {
    padding: 20px;
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }
  .compose-body::-webkit-scrollbar {
    display: none;
  }
  .field {
    display: flex; align-items: center; gap: 10px; margin-bottom: 12px;
  }
  .field label {
    width: 60px; font-size: 0.875rem; color: var(--color-text-secondary);
    text-align: right; flex-shrink: 0; font-weight: 500;
  }
  .field input {
    flex: 1; border: 1px solid var(--color-border); border-radius: 8px;
    padding: 9px 14px; font-size: 0.875rem;
    background: var(--color-list);
    color: var(--color-text);
    transition: all 0.15s ease-in-out;
  }
  .field input:focus {
    outline: none;
    border-color: var(--color-accent);
  }
  .to-row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .to-row :global(> *:first-child) {
    flex: 1;
    min-width: 0;
    max-width: 575px;
  }
  .ccbcc-group {
    display: flex;
    gap: 0;
    margin-left: auto;
    flex-shrink: 0;
  }
  .ccbcc-toggle {
    flex-shrink: 0;
    border: none;
    background: none;
    color: var(--color-text-secondary);
    font-size: 0.75rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    padding: 6px 1px;
    transition: color 0.12s ease;
  }
  .ccbcc-toggle:hover {
    color: var(--color-accent);
  }
  .ccbcc-input-wrapper {
    position: relative;
    flex: 1;
    display: flex;
    align-items: center;
  }
  .ccbcc-clear-btn {
    position: absolute;
    right: 10px;
    border: none;
    background: none;
    font-size: 1.125rem;
    line-height: 1;
    cursor: pointer;
    color: var(--color-text-secondary);
    opacity: 0.5;
    padding: 2px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all 0.12s ease;
  }
  .ccbcc-clear-btn:hover {
    opacity: 1;
    color: var(--color-danger);
  }
  .tone-section {
    margin: 16px 0; padding: 12px 0;
    border-top: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
  }
  .chain-preview {
    margin-top: 10px;
    margin-bottom: 16px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    overflow: hidden;
    max-height: 120px;
    display: flex;
    flex-direction: column;
  }
  .chain-scroll-area {
    overflow-y: auto;
    flex: 1;
  }
  .chain-scroll-area::-webkit-scrollbar {
    width: 4px;
  }
  .chain-scroll-area::-webkit-scrollbar-track {
    background: transparent;
  }
  .chain-scroll-area::-webkit-scrollbar-thumb {
    background: var(--color-border);
    border-radius: 2px;
  }
  .chain-header {
    font-size: 0.875rem;
    font-weight: 700;
    color: var(--color-text-secondary);
    padding: 8px 12px;
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .chain-body {
    font-family: inherit;
    font-size: 0.75rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    padding: 10px 14px;
    color: var(--color-text-secondary);
  }
  .chain-body-html {
    font-size: 0.75rem;
    line-height: 1.5;
    word-break: break-word;
    margin: 0;
    padding: 10px 14px;
    color: var(--color-text-secondary);
  }
  .editor-preview {
    margin-bottom: 16px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    overflow: visible;
    display: flex;
    flex-direction: column;
    transition: all 0.15s ease-in-out;
  }
  .editor-preview:focus-within {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 15%, transparent);
  }
  .editor-resize {
    overflow: visible;
    display: flex;
    flex-direction: column;
    min-height: 200px;
  }
  /* Cap the editor so the sticky send toolbar never gets pushed out of the
     viewport by an overgrown textarea — the textarea scrolls internally
     instead (fixed toolbar + scrollable input, incl. reply/forward chains). */
  .editor {
    width: 100%;
    min-height: 120px;
    max-height: min(55vh, 480px);
    overflow-y: auto;
    overscroll-behavior: contain;
    outline: none;
    padding: 10px 12px;
    font-size: 0.9rem;
    line-height: 1.5;
    color: var(--color-text);
  }
  .editor-header {
    font-size: 0.875rem;
    font-weight: 700;
    color: var(--color-text-secondary);
    padding: 5px 12px;
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .fmt-toolbar {
    display: flex;
    gap: 1px;
  }
  .fmt-btn {
    background: transparent;
    border: none;
    border-radius: 4px;
    color: var(--color-text-secondary);
    padding: 4px 5px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .fmt-btn:hover {
    background: var(--color-hover);
    color: var(--color-text);
  }
  .editor:empty::before {
    content: attr(data-placeholder);
    color: var(--color-text-tertiary);
    pointer-events: none;
  }
  .editor h3 {
    font-size: 1.1rem;
    font-weight: 700;
    margin: 0.5em 0 0.25em;
  }
  .editor ul {
    margin: 0.25em 0;
    padding-left: 1.2em;
  }
  .editor code {
    background: var(--color-border);
    border-radius: 3px;
    padding: 1px 4px;
    font-size: 0.85em;
    font-family: var(--font-mono, monospace);
  }
  .editor a {
    color: var(--color-accent);
    text-decoration: underline;
  }
  .editor-wrapper {
    position: relative;
    width: 100%;
  }
  .editor {
    width: 100%; border: none;
    padding: 14px; font-size: 0.875rem; font-family: inherit;
    line-height: 1.6; resize: vertical; min-height: 200px;
    background: var(--color-list);
    color: var(--color-text);
  }
  .editor:focus {
    outline: none;
  }
  .editor-wrapper.has-attachments .editor {
    padding-bottom: 44px;
  }
  .editor-attachments {
    position: absolute;
    bottom: 18px;
    left: 14px;
    right: 14px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .editor-toolbar {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    /* Fixed flex child of .compose-window (below the scrollable body), so
       "Senden" is ALWAYS reachable without scrolling — same as the header
       stays pinned at the top. */
    flex-shrink: 0;
    padding: 12px 20px calc(12px + env(safe-area-inset-bottom, 0px));
    background: var(--color-list);
    border-top: 1px solid var(--color-border);
  }
  .spacer { flex: 1; }
  .btn-ai {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 14px;
    height: 34px;
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: var(--color-list);
    color: var(--color-text);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    font-family: inherit;
    transition: all 0.15s ease;
  }
  .btn-ai:hover:not(:disabled) { border-color: var(--color-accent); }
  .btn-ai.primary { background: var(--color-accent); color: white; border-color: var(--color-accent); }
  .btn-ai.primary:hover:not(:disabled) { background: var(--color-accent); border-color: var(--color-accent); }
  .btn-ai:disabled { opacity: 0.45; cursor: default; }
  .btn-ai.recording { background: var(--color-danger); border-color: var(--color-danger); color: white; animation: toolbarPulse 1.5s ease-in-out infinite; }

  .toggle-mic {
    display: none;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.12s ease;
    flex-shrink: 0;
  }
  .toggle-mic.voice-enabled { display: flex; }
  .toggle-mic:hover { background: transparent; }
  .btn-ai.primary .toggle-mic:hover { background: transparent; }
  .btn-ai.recording .toggle-mic:hover { background: transparent; }
  .toggle-mic svg { width: 14px; height: 14px; }
  /*
   * DISABLED — Mic-Divider (deaktiviert, nicht gelöscht). Zum Wiederaktivieren
   * den Markup-Block `{#if false && voiceEnabled}` auf `{#if voiceEnabled}`
   * zurückstellen und diese Regeln einkommentieren.
   *
  .mic-divider {
    width: 5px;
    height: calc(100% + 14px);
    margin-top: -7px;
    margin-bottom: -7px;
    background: var(--color-list);
    opacity: 0.9;
    border-radius: 2px;
    flex-shrink: 0;
    pointer-events: none;
  }
   */
  .btn-label { pointer-events: none; }
  .label-short { display: none; }

  .btn-send {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 5px 18px;
    height: 34px;
    background: var(--color-success);
    color: white;
    border: none;
    border-radius: 10px;
    font-size: 0.8125rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: inherit;
  }
  .btn-send:hover:not(:disabled) { background: color-mix(in srgb, var(--color-success) 85%, #000000); }
  .btn-send:disabled { opacity: 0.5; cursor: default; }

  @keyframes toolbarPulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.85; }
  }

  .error-banner {
    background: color-mix(in srgb, var(--color-danger) 8%, transparent); color: var(--color-danger);
    padding: 10px 14px; border-radius: 8px; font-size: 0.75rem; margin-top: 12px;
  }
  .generation-status {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    background: color-mix(in srgb, var(--color-list) 85%, transparent);
    pointer-events: none;
    z-index: 5;
  }
  .generation-status .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent);
    animation: dotPulse 1.4s ease-in-out infinite;
  }
  .generation-status .dot:nth-child(1) { animation-delay: 0s; }
  .generation-status .dot:nth-child(2) { animation-delay: 0.2s; }
  .generation-status .dot:nth-child(3) { animation-delay: 0.4s; }
  @keyframes dotPulse {
    0%, 80%, 100% { opacity: 0.2; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }

  .attachment-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--color-active-wash);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    font-size: 0.75rem;
    color: var(--color-text);
    transition: all 0.12s ease;
  }
  .attachment-pill:hover {
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
  }
  .attachment-label {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 200px;
  }
  .attachment-remove {
    border: none;
    background: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    opacity: 0;
    padding: 0;
    transition: all 0.12s ease;
    flex-shrink: 0;
  }
  .attachment-pill:hover .attachment-remove {
    opacity: 1;
  }
  .attachment-remove:hover {
    color: var(--color-danger);
  }

  .close-dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fadeIn 0.15s ease-out;
  }
  .close-dialog {
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 24px 28px;
    min-width: 280px;
    box-shadow: none;
    animation: scaleIn 0.15s ease-out;
  }
  .close-dialog-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--color-text);
    margin: 0 0 20px 0;
    text-align: center;
  }
  .close-dialog-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }
  .btn-discard {
    padding: 8px 18px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-list);
    color: var(--color-text);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    font-family: inherit;
    transition: all 0.15s ease;
  }
  .btn-discard:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
  .btn-save {
    padding: 8px 18px;
    border: none;
    border-radius: 8px;
    background: var(--color-accent);
    color: white;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    font-family: inherit;
    transition: all 0.15s ease;
  }
  .btn-save:hover {
    background: var(--color-accent-hover);
  }

  /* Mobile: compose fills the screen (iPhone). */
  @media (max-width: 600px) {
    .compose-window {
      margin: 0;
      border-radius: 0;
      border: none;
      box-shadow: none;
      height: 100%;
      min-height: 0;
    }
    .compose-header {
      padding-top: max(16px, env(safe-area-inset-top, 0px));
      padding-left: 16px;
      padding-right: 16px;
    }
    .close-icon-desktop { display: none; }
    .close-icon-mobile { display: inline; font-size: 0.9375rem; font-weight: 500; }
    .close-btn { padding: 10px 12px; }
    .compose-body {
      padding: 16px;
      /* The sticky toolbar carries its own safe-area padding. */
      padding-bottom: 16px;
    }
    /* Stack fields vertically on phones — labels above inputs, no cramped
       right-aligned 60px labels next to inputs. */
    .field {
      flex-direction: column;
      align-items: stretch;
      gap: 6px;
      margin-bottom: 16px;
    }
    .field label {
      width: auto;
      text-align: left;
      font-size: 0.75rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }
    .field input,
    .to-row input {
      padding: 12px 14px;
      font-size: 1rem;
    }
    .to-row {
      gap: 8px;
    }
    .ccbcc-group {
      flex-shrink: 0;
    }
    .tone-section {
      margin-top: 4px;
    }
    /* Editor: 16px font prevents iOS focus auto-zoom; shorter default
       height leaves room for the sticky toolbar. */
    .editor {
      font-size: 1rem;
      min-height: 120px;
    }
    .editor-resize {
      min-height: 120px;
    }
    /* Toolbar: all three actions in ONE row, 45px touch targets.
       Padding above/below the buttons stays 10px so they don't hug the
       borders — the editor is scrollable above, the toolbar stays pinned. */
    .editor-toolbar {
      flex-wrap: nowrap;
      gap: 8px;
      padding: 10px 16px calc(10px + env(safe-area-inset-bottom, 0px));
    }
    .editor-toolbar .spacer {
      display: none;
    }
    .btn-ai {
      flex: 1 1 0;
      min-width: 0;
      height: 45px;
      padding: 0 8px;
      justify-content: center;
      align-items: center;
    }
    .btn-send {
      flex: 0 0 auto;
      height: 45px;
      padding: 0 16px;
      justify-content: center;
      align-items: center;
    }
    .label-long { display: none; }
    .label-short { display: inline; }
  }
</style>



