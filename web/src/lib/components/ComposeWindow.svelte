<script lang="ts">
  import DiffEditor from "./DiffEditor.svelte";
  import ToneControls from "./ToneControls.svelte";
  import RecipientInput from "./RecipientInput.svelte";
  import {
    aiGenerateMail, aiSuggestRecipient, aiSuggestSubject, aiFormatText, getToneProfile,
    saveDraft, openFilePicker,
    getVoiceSettings, voiceTranscribe,
    resolveRecipientFromText,
  } from "$lib/services/tauri";
  import { get } from "svelte/store";
  import { showDiffEnabled } from "$lib/stores/settings";
  import { textToHtml, wrapHtmlQuote } from "$lib/utils/format";
  import { blobToWavBase64 } from "$lib/utils/wav";
  import type { MailChainEntry } from "$lib/types/mail";
  import ErrorBanner from "$lib/components/ErrorBanner.svelte";

  type ComposeMode = "new" | "reply";
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
    onsend: (data: { to: string; subject: string; body: string; bodyHtml: string; cc?: string; bcc?: string; attachments?: { filename: string; content: string; contentType: string }[]; aiDraft?: string | null }) => Promise<void>;
    // Pre-filled draft data
    draftTo?: string;
    draftSubject?: string;
    draftBody?: string;
    draftUid?: number | null;
  }

  let {
    mode, mailChain = [], sendError = null, replySubject = "", replyTo = "",
    accountId, recipientEmail, senderName = "", recipientName = "", onclose, onsend,
    draftTo = "", draftSubject = "", draftBody = "", draftUid = null,
  }: Props = $props();

  let to = $state<string[]>([]);
  let cc = $state("");
  let bcc = $state("");
  interface ComposeAttachment {
    filename: string;
    content: string;
    contentType: string;
    size: number;
  }
  let attachments = $state<ComposeAttachment[]>([]);
  // Cc and Bcc are independently toggled. Stays open if the field has content.
  let showCc = $state(false);
  let showBcc = $state(false);
  let ccVisible = $derived(showCc || cc.trim().length > 0);
  let bccVisible = $derived(showBcc || bcc.trim().length > 0);
  let subject = $state("");
  let userInput = $state("");
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
        cc.trim() ? cc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
        bcc.trim() ? bcc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
      );
      localDraftUid = result.uid;
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
            userInput = transcript;
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

      // Auto-stop after 3 seconds of silence
      let silenceTimeout: ReturnType<typeof setTimeout>;
      const audioContext = new AudioContext();
      const source = audioContext.createMediaStreamSource(stream);
      const analyser = audioContext.createAnalyser();
      analyser.fftSize = 256;
      source.connect(analyser);

      const checkSilence = () => {
        if (!isRecording) return;
        const dataArray = new Uint8Array(analyser.frequencyBinCount);
        analyser.getByteFrequencyData(dataArray);
        const average = dataArray.reduce((a, b) => a + b, 0) / dataArray.length;

        if (average < 10) { // Silence threshold
          clearTimeout(silenceTimeout);
          silenceTimeout = setTimeout(() => {
            if (isRecording) stopRecording();
          }, 3000);
        } else {
          clearTimeout(silenceTimeout);
        }

        requestAnimationFrame(checkSilence);
      };
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
      if (draftUid != null && draftUid !== lastPropDraftUid) {
        // Pre-fill from draft data
        to = draftTo ? draftTo.split(",").map(s => s.trim()).filter(Boolean) : [];
        subject = draftSubject;
        userInput = draftBody;
        localDraftUid = draftUid;
        lastPropDraftUid = draftUid;
      } else {
        to = replyTo ? [replyTo] : [];
        subject = mode === "reply" ? `Re: ${replySubject}` : "";
      }
      toneLoaded = false;
      lastMode = mode;
      lastReplyTo = replyTo;
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
        userInput = result;
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
      userInput = await aiFormatText(userInput);
    } catch (e: unknown) {
      generationError = e instanceof Error ? e.message : String(e);
    } finally {
      isGenerating = false;
    }
  }

  function acceptDraft() {
    userInput = aiDraft ?? userInput;
    aiDraft = null;
    showDiff = false;
  }

  function rejectDraft() {
    aiDraft = null;
    showDiff = false;
  }

  async function handleSend() {
    let body = userInput;
    let bodyHtml = textToHtml(userInput);
    if (mailChain.length > 0) {
      body += "\n\n" + mailChain.map(m => "> " + m.text.replace(/\n/g, "\n> ")).join("\n\n");
      const htmlQuotes = mailChain.map(m =>
        m.html ? wrapHtmlQuote(m.html) : wrapHtmlQuote(textToHtml(m.text))
      ).join("\n");
      bodyHtml += "\n" + htmlQuotes;
    }
    try {
      await onsend({
        to: to.filter((t) => t.trim()).join(", "),
        subject,
        body,
        bodyHtml,
        cc: cc.trim() || undefined,
        bcc: bcc.trim() || undefined,
        attachments: attachments.length > 0 ? attachments.map(a => ({
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
    <h2>{mode === "new" ? "Neue Nachricht" : "Antworten"}</h2>
    <button type="button" class="close-btn" onclick={handleClose}>&#x2715;</button>
  </div>

  <div class="compose-body">
    <div class="field">
      <label for="to">An:</label>
     <div class="to-row">
        <RecipientInput bind:value={to} {accountId} />
        <span class="ccbcc-group">
        {#if !ccVisible}
          <button type="button" class="ccbcc-toggle" onclick={() => showCc = true} title="Cc hinzufügen">
            Cc
          </button>
        {/if}
        {#if !bccVisible}
          <button type="button" class="ccbcc-toggle" onclick={() => showBcc = true} title="Bcc hinzufügen">
            Bcc
          </button>
        {/if}
        </span>
      </div>
    </div>
    {#if ccVisible}
      <div class="field">
        <label for="cc">Cc:</label>
        <div class="ccbcc-input-wrapper">
          <input id="cc" type="text" autocomplete="new-password" spellcheck="false" bind:value={cc} placeholder="Empfänger in Kopie (Cc)..." />
          <button type="button" class="ccbcc-clear-btn" onclick={() => { cc = ""; showCc = false; }} title="Cc entfernen">&times;</button>
        </div>
      </div>
    {/if}
    {#if bccVisible}
      <div class="field">
        <label for="bcc">Bcc:</label>
        <div class="ccbcc-input-wrapper">
          <input id="bcc" type="text" autocomplete="new-password" spellcheck="false" bind:value={bcc} placeholder="Empfänger in Blindkopie (Bcc)..." />
          <button type="button" class="ccbcc-clear-btn" onclick={() => { bcc = ""; showBcc = false; }} title="Bcc entfernen">&times;</button>
        </div>
      </div>
    {/if}
    <div class="field">
      <label for="subject">Betreff:</label>
      <div class="to-row">
        <input id="subject" type="text" bind:value={subject} placeholder="Betreff" />
        <span class="ccbcc-group">
          <button type="button" class="ccbcc-toggle" onclick={addAttachment} title="Datei anhängen">
            Anhang
          </button>
        </span>
      </div>
    </div>

    {#if mode === "reply" && mailChain.length > 0}
      <div class="chain-preview">
        <div class="chain-header">Urspr&uuml;ngliche Nachricht:</div>
        <div class="chain-scroll-area">
          {#each mailChain as msg}
            <div class="chain-msg">
              {#if msg.html}
                <div class="chain-body-html">{@html msg.html.slice(0, 1000)}{msg.html.length > 1000 ? "..." : ""}</div>
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
          <div class="editor-header">Deine Nachricht:</div>
          <div class="editor-wrapper" class:has-attachments={attachments.length > 0}>
        <textarea
          id="editor"
          class="editor"
          bind:value={userInput}
          placeholder="Gib Deine eigene Nachricht oder Stichpunkte ein..."
          rows={12}
          aria-label=Nachrichtentext
        ></textarea>
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
      <ErrorBanner message={voiceError} onretry={() => voiceError = null} retryLabel="Schließen" />
    {/if}
    {#if generationError}
      <ErrorBanner message={generationError} onretry={generate} retryLabel="Erneut generieren" />
    {/if}
    {#if sendError}
      <ErrorBanner message={sendError} onretry={handleSend} retryLabel="Erneut senden" />
    {/if}

    <div class="editor-toolbar">
      <button type="button" class="btn-ai primary" class:recording={isRecording} onclick={handleGenerateClick} disabled={isGenerating}>
        <span class="toggle-mic" class:voice-enabled={voiceEnabled} onclick={handleMicToggle} title={isRecording ? "Aufnahme stoppen" : "Diktat starten"}>
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
          </svg>
        </span>
        <span class="btn-label">
          {isGenerating
            ? (generationStep === 1 ? "Generiere Text..." : "Ermittle Felder...")
            : (isRecording ? "Aufnahme" : (mode === "reply" ? "Antwort generieren" : "Generieren"))}
        </span>
      </button>
      <button type="button" class="btn-ai" onclick={handleFormat} disabled={isGenerating || !userInput.trim()}>
        Formatieren
      </button>
      <div class="spacer"></div>
      <button type="button" class="btn-send" onclick={handleSend} disabled={!to[0]?.trim() || !subject.trim() || !userInput.trim()}>
        Senden
      </button>
    </div>

  </div>
</div>

{#if showCloseDialog}
  <div class="close-dialog-overlay" onclick={handleCloseDiscard}>
    <div class="close-dialog" onclick={(e) => e.stopPropagation()}>
      <p class="close-dialog-title">Entwurf speichern?</p>
      <div class="close-dialog-actions">
        <button type="button" class="btn-discard" onclick={handleCloseDiscard}>Verwerfen</button>
        <button type="button" class="btn-save" onclick={handleCloseSave}>Speichern</button>
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
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05), 0 12px 36px rgba(0, 0, 0, 0.06);
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
  .ccbcc-input-wrapper input {
    padding-right: 32px;
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
  .editor-header {
    font-size: 0.875rem;
    font-weight: 700;
    color: var(--color-text-secondary);
    padding: 5px 12px;
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
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
    margin-top: 16px;
    align-items: center;
    flex-wrap: wrap;
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
  .btn-label { pointer-events: none; }

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
    background: #ffeef0; color: var(--color-danger);
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
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.15);
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
    }
    .compose-body {
      padding-bottom: max(16px, env(safe-area-inset-bottom, 0px));
    }
  }
</style>



