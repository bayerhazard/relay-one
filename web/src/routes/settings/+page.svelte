<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import KontoDav from "$lib/components/KontoDav.svelte";
  import { onMount } from "svelte";
  import {
    getSettings, saveSettings,
    connectAccount, listAccounts, deleteAccount, updateAccountSettings,
    getMoveToTrash, setMoveToTrash, getAufraeumen, setAufraeumen,
    getCardDavSettings, setCardDavSettings, syncCardDav,
    syncCalDav,
    listCalDavAccounts, createCalDavAccount, updateCalDavAccount, deleteCalDavAccount,
    type CalDavAccount,
    getVoiceSettings, saveVoiceSettings,
    resetCircuitBreaker,
    getAttachmentCacheStats, cleanupAttachmentCache, clearAttachmentCache, clearAiSummaries,
    setupPush, teardownPush, pushEnabled,
    getDeleteQueue, retryDeleteQueueRow, removeDeleteQueueRow, downloadExport, createBackup, listBackups, restoreBackupSnapshot,
    getOlaresMailStatus,
  } from "$lib/services/tauri";
  import type { AccountInfo } from "$lib/stores/accounts";
  import { accounts } from "$lib/stores/accounts";
  import { settings, showDiffEnabled, ROUTER_BASE, ROUTER_CHAT_MODEL } from "$lib/stores/settings";
  import { clearFollowupMemory } from "$lib/utils/followupMemory";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import Huelle from "$lib/components/Huelle.svelte";
  import EinstellungenSpalte, { type Abschnitt } from "./EinstellungenSpalte.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import { t, lang, setLang, translate, localizeError } from "$lib/i18n";
  import { appearance, type Appearance } from "$lib/stores/appearance";

  // ─── Active Tab State ────────────────────────
  let activeTab = $state("general"); // 'general' | 'accounts' | 'carddav' | 'caldav' | 'ai' | 'voice' | 'cache' | 'archive'
  // The account just added: its calendar section stands open.
  let neuesKonto = $state<number | null>(null);

  // The sections in the shell's column (HB-UNTERNAV, RL-G2); the settings
  // belong to no area, they are reached through the profile (HB-KONTO).
  // On the phone the shell's sheet replaces the old menu/back drill-down.
  // `titel` heads the page (HB-SEITENKOPF), `desc` is the line under it.
  // The texts are translated here, literally, so the i18n guard sees them.
  const ABSCHNITTE: (Abschnitt & { titel: string; desc: string })[] = $derived([
    { id: "general", text: $t("settings.general"), zeichen: "einstellungen", titel: $t("settings.generalTitle"), desc: $t("settings.generalDesc") },
    { id: "accounts", text: $t("settings.accounts"), zeichen: "post", titel: $t("settings.accounts"), desc: $t("settings.accountsDesc") },
    { id: "carddav", text: $t("settings.contacts"), zeichen: "team", titel: $t("settings.contactsTitle"), desc: $t("settings.contactsDesc") },
    { id: "caldav", text: $t("settings.calendar"), zeichen: "kalender", titel: $t("settings.calendarTitle"), desc: $t("settings.calendarDesc") },
    { id: "ai", text: $t("settings.ai"), zeichen: "ai", titel: $t("settings.aiTitle"), desc: $t("settings.aiDesc") },
    { id: "voice", text: $t("settings.voice"), zeichen: "mikrofon", titel: $t("settings.voiceTitle"), desc: $t("settings.voiceDesc") },
    { id: "cache", text: $t("settings.cache"), zeichen: "datenbank", titel: $t("settings.cacheTitle"), desc: $t("settings.cacheDesc") },
    { id: "archive", text: $t("settings.archive"), zeichen: "datensicherung", titel: $t("settings.archiveTitle"), desc: $t("settings.archiveDesc") },
  ]);
  let abschnitt = $derived(ABSCHNITTE.find((a) => a.id === activeTab) ?? ABSCHNITTE[0]);

  // The shell's sheet on the phone; a choice closes it.
  let sheetOpen = $state(false);

  function selectTab(tab: string) {
    activeTab = tab;
    sheetOpen = false;
    if (tab === "archive") { loadDeleteQueue(); loadBackups(); }
  }

  // ─── Appearance ──────────────────────────────
  // Stored and applied by lib/stores/appearance.ts (CI RL-T2).
  function handleThemeChange(next: Appearance) {
    appearance.set(next);
  }

  // ─── LLM ─────────────────────────────────────
  let aiUrl = $state(ROUTER_BASE);
  let aiKey = $state("");
  let aiModel = $state(ROUTER_CHAT_MODEL);
  // Olares Router is the default source; manual entry is opt-in.
  let aiSource = $state<"router" | "manual">("router");
  let aiRouterAvailable = $state(false);
  let aiRouterUrl = $state(ROUTER_BASE);
  let aiChatModel = $state(ROUTER_CHAT_MODEL);
  let aiSaved = $state(false);
  let aiError = $state<string | null>(null);
  let cbResetDone = $state(false);
  let moveToTrash = $state(true);
  // Erweitert: the clean-up view (Aufräumen) in the mail column.
  let aufraeumen = $state(false);
  let autoDownloadImages = $state(true);
  let fetchLimit = $state(50);
  let notificationsEnabled = $state(false);
  let notificationsError = $state<string | null>(null);
  let notificationsBusy = $state(false);

  // ─── CardDAV ─────────────────────────────────
  let carddavUrl = $state("https://");
  let carddavUser = $state("");
  let carddavPass = $state("");
  let carddavInterval = $state(30);
  let carddavSaved = $state(false);
  let carddavError = $state<string | null>(null);
  let carddavSyncing = $state(false);
  let carddavSyncResult = $state<number | null>(null);

  // ─── CalDAV State (multi-account) ───────────
  let caldavAccounts = $state<CalDavAccount[]>([]);
  /** null = no form open; "new" = add; otherwise the account id being edited. */
  let caldavEditingId = $state<string | null>(null);
  let caldavName = $state("");
  let caldavUrl = $state("https://");
  let caldavUser = $state("");
  let caldavPass = $state("");
  let caldavInterval = $state(30);
  let caldavSaved = $state(false);
  let caldavError = $state<string | null>(null);
  let caldavSyncing = $state(false);
  let caldavSyncResult = $state<number | null>(null);
  let showDeleteCalDavConfirm = $state(false);
  let pendingDeleteCalDavId = $state<string | null>(null);

  // ─── Voice ───────────────────────────────────
  // Olares Router is the default source for STT + TTS; manual is opt-in.
  let voiceSource = $state<"router" | "manual">("router");
  let voiceEnabled = $state(false);
  let voiceSttUrl = $state("");
  let voiceSttKey = $state("");
  let voiceSttModel = $state("Systran/faster-whisper-small");
  // Phase D: TTS (proxy to a configured OpenAI-compatible endpoint).
  let voiceTtsEnabled = $state(false);
  let voiceTtsUrl = $state("");
  let voiceTtsKey = $state("");
  let voiceTtsModel = $state("tts-1");
  let voiceTtsAuto = $state(false);
  let voiceSaved = $state(false);
  let voiceError = $state<string | null>(null);

  // ─── Cache Management ─────────────────────────
  let deleteQueue = $state<Array<{ id: number; account_id: number; uid: number; folder: string; action: string; state: string; attempts: number; last_error: string | null }>>([]);

  let backupBusy = $state(false);
  let backupResult = $state<{ path: string; size: number } | null>(null);
  let backups = $state<Array<{ name: string; size: number }>>([]);
  let restoreResult = $state<string | null>(null);

  async function handleBackup() {
    if (backupBusy) return;
    backupBusy = true;
    backupResult = null;
    try {
      const b = await createBackup();
      backupResult = { path: b.path, size: b.size };
      await loadBackups();
    } catch (e) {
      console.error("backup failed", e);
    } finally {
      backupBusy = false;
    }
  }

  async function loadBackups() {
    try {
      const d = await listBackups();
      backups = d.backups ?? [];
    } catch (e) {
      console.warn("backup list load failed", e);
    }
  }

  function askRestoreBackup(name: string) {
    pendingRestoreName = name;
    showRestoreConfirm = true;
  }

  async function confirmRestoreBackup() {
    const name = pendingRestoreName;
    showRestoreConfirm = false;
    pendingRestoreName = null;
    if (name == null) return;
    void restoreBackup(name);
  }

  async function restoreBackup(name: string) {
    try {
      const r = await restoreBackupSnapshot(name);
      restoreResult = translate("settings.restored", { restored: r.restored, bytes: formatBytes(r.bytes), note: r.note ?? "" });
    } catch (e) {
      restoreResult = translate("settings.restoreFailed") + (e instanceof Error ? e.message : String(e));
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${bytes} B`;
  }

  async function loadDeleteQueue() {
    try {
      deleteQueue = await getDeleteQueue();
    } catch (e) {
      console.warn("delete queue load failed", e);
    }
  }

  async function retryDeleteQueue(id: number) {
    try {
      await retryDeleteQueueRow(id);
      await loadDeleteQueue();
    } catch (e) {
      console.error("retry failed", e);
    }
  }

  async function removeDeleteQueue(id: number) {
    try {
      await removeDeleteQueueRow(id);
      await loadDeleteQueue();
    } catch (e) {
      console.error("remove failed", e);
    }
  }
  let cacheStats = $state<{ total_attachments: number; cached_count: number; cached_size_mb: number } | null>(null);
  let cacheMaxMb = $state(100);
  let cacheCleaning = $state(false);
  let cacheCleanupResult = $state<number | null>(null);

  try { const v = localStorage.getItem("relay_fetch_limit"); if (v) fetchLimit = parseInt(v, 10) || 50; } catch {}
  try { autoDownloadImages = localStorage.getItem("relay_auto_download_images") !== "false"; } catch {}

  function handleFetchLimitChange() {
    const clamped = Math.max(10, Math.min(100000, fetchLimit));
    fetchLimit = clamped;
    try { localStorage.setItem("relay_fetch_limit", String(clamped)); } catch {}
  }

  onMount(async () => {
    try {
      const s = await settings.init();
      aiUrl = s.url;
      aiKey = s.api_key;
      aiModel = s.model;
      aiSource = s.source === "manual" ? "manual" : "router";
      aiRouterAvailable = s.router_available ?? false;
      aiRouterUrl = s.router_url || ROUTER_BASE;
      aiChatModel = s.chat_model || ROUTER_CHAT_MODEL;
    } catch (e) { console.warn("Settings load failed, using defaults", e); }
    try {
      moveToTrash = await getMoveToTrash();
      aufraeumen = await getAufraeumen().catch(() => false);
    } catch (e) { console.warn("move_to_trash load failed, using default", e); }
    await loadAccountList();
    // Back from Google's sign-in without a pop-up: open that account.
    const zurueck = new URLSearchParams(location.search);
    if (zurueck.has("google")) {
      activeTab = "accounts";
      neuesKonto = Number(zurueck.get("konto")) || null;
      history.replaceState(history.state, "", location.pathname);
    }

    // Load CardDAV settings
    try {
      const cs = await getCardDavSettings();
      if (cs) {
        carddavUrl = cs.url;
        carddavUser = cs.username;
        carddavPass = cs.password;
        carddavInterval = cs.sync_interval_minutes;
      }
    } catch (e) { console.warn("CardDAV settings load failed", e); }

    // Load CalDAV accounts
    await loadCaldavAccounts();

    // Load Voice settings
    try {
      const vs = await getVoiceSettings();
      if (vs) {
        voiceSource = vs.source === "manual" ? "manual" : "router";
        voiceEnabled = vs.enabled;
        voiceSttUrl = vs.sttUrl;
        voiceSttKey = vs.sttKey;
        voiceSttModel = vs.sttModel;
        voiceTtsEnabled = vs.ttsEnabled;
        voiceTtsUrl = vs.ttsUrl;
        voiceTtsKey = vs.ttsKey;
        voiceTtsModel = vs.ttsModel || "tts-1";
        voiceTtsAuto = vs.ttsAuto;
      }
    } catch (e) { console.warn("Voice settings load failed", e); }

    // Load push notification state
    try {
      notificationsEnabled = await pushEnabled();
    } catch (e) { console.warn("Push state load failed", e); }

    // Load Cache stats
    loadCacheStats();
  });

  async function loadCacheStats() {
    try {
      cacheStats = await getAttachmentCacheStats();
    } catch (e) { console.warn("Cache stats load failed", e); }
  }

  async function handleCleanupCache() {
    cacheCleaning = true;
    cacheCleanupResult = null;
    try {
      const result = await cleanupAttachmentCache(cacheMaxMb);
      cacheCleanupResult = result;
      loadCacheStats();
      setTimeout(() => (cacheCleanupResult = null), 5000);
    } catch (e: unknown) {
      console.error("Cache cleanup failed", e);
    } finally {
      cacheCleaning = false;
    }
  }

  // Clearing caches is final, so it asks first; in the view the buttons are
  // secondary and only the confirmation carries the red (CI RL-R1, RL-R2).
  let pendingClear = $state<"cache" | "summaries" | "actions" | null>(null);
  function confirmClear() {
    const kind = pendingClear;
    pendingClear = null;
    if (kind === "cache") handleClearCache();
    else if (kind === "summaries") handleClearAiSummaries();
    else if (kind === "actions") handleClearAiActions();
  }

  async function handleClearCache() {
    cacheCleaning = true;
    cacheCleanupResult = null;
    try {
      const result = await clearAttachmentCache();
      cacheCleanupResult = result;
      loadCacheStats();
      setTimeout(() => (cacheCleanupResult = null), 5000);
    } catch (e: unknown) {
      console.error("Cache clear failed", e);
    } finally {
      cacheCleaning = false;
    }
  }

  let aiSummariesClearing = $state(false);
  let aiSummariesResult = $state<number | null>(null);

  async function handleClearAiSummaries() {
    aiSummariesClearing = true;
    aiSummariesResult = null;
    try {
      const cleared = await clearAiSummaries();
      aiSummariesResult = cleared;
      setTimeout(() => (aiSummariesResult = null), 6000);
    } catch (e: unknown) {
      console.error("KI-Zusammenfassungen löschen fehlgeschlagen", e);
    } finally {
      aiSummariesClearing = false;
    }
  }

  let aiActionsClearing = $state(false);
  let aiActionsResult = $state<number | null>(null);

  async function handleClearAiActions() {
    aiActionsClearing = true;
    aiActionsResult = null;
    try {
      const cleared = clearFollowupMemory();
      aiActionsResult = cleared;
      setTimeout(() => (aiActionsResult = null), 6000);
    } catch (e: unknown) {
      console.error("KI-Aktionen löschen fehlgeschlagen", e);
    } finally {
      aiActionsClearing = false;
    }
  }

  async function handleSaveAI() {
    aiError = null;
    try {
      if (aiSource === "router") {
        await settings.save(ROUTER_BASE, "", ROUTER_CHAT_MODEL, "router");
      } else {
        await settings.save(aiUrl, aiKey, aiModel, "manual");
      }
      aiSaved = true;
      setTimeout(() => (aiSaved = false), 2000);
    } catch (e: unknown) {
      aiError = e instanceof Error ? e.message : String(e);
    }
  }

  function setAiSource(source: "router" | "manual") {
    aiSource = source;
    aiError = null;
  }

  async function handleResetCircuitBreaker() {
    try {
      await resetCircuitBreaker();
      cbResetDone = true;
      setTimeout(() => (cbResetDone = false), 2000);
    } catch (e: unknown) {
      aiError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleAufraeumenToggle() {
    aufraeumen = !aufraeumen;
    try {
      await setAufraeumen(aufraeumen);
    } catch (e) { console.warn("aufraeumen save failed", e); aufraeumen = !aufraeumen; }
  }

  async function handleMoveToTrashToggle() {
    moveToTrash = !moveToTrash;
    try {
      await setMoveToTrash(moveToTrash);
    } catch (e) { console.warn("move_to_trash save failed", e); }
  }

  function handleAutoDownloadImagesToggle() {
    autoDownloadImages = !autoDownloadImages;
    try { localStorage.setItem("relay_auto_download_images", String(autoDownloadImages)); } catch {}
  }

  async function handleNotificationsToggle() {
    if (notificationsBusy) return;
    notificationsBusy = true;
    notificationsError = null;
    try {
      if (notificationsEnabled) {
        await teardownPush();
        notificationsEnabled = false;
      } else {
        let accountId = 1;
        try {
          const acctList = await listAccounts();
          accountId = acctList?.[0]?.id ?? 1;
        } catch { /* default to 1 */ }
        const result = await setupPush(accountId, () => {
          notificationsError = translate("settings.notifDenied");
        });
        if (result === "registered" || result === "granted") {
          notificationsEnabled = true;
        } else if (result === "denied") {
          notificationsEnabled = false;
        } else {
          notificationsError = translate("settings.notifUnsupported");
          notificationsEnabled = false;
        }
      }
    } catch (e) {
      notificationsError = String(e);
    } finally {
      notificationsBusy = false;
    }
  }

async function handleSaveCardDav() {
    carddavError = null;
    carddavSaved = false;
    try {
      await setCardDavSettings({
        url: carddavUrl,
        username: carddavUser,
        password: carddavPass,
        sync_interval_minutes: carddavInterval,
      });
      carddavSaved = true;
      setTimeout(() => (carddavSaved = false), 2000);
    } catch (e: unknown) {
      carddavError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleSaveVoice() {
    voiceError = null;
    voiceSaved = false;
    // Validate manual endpoints only — Router values are managed centrally.
    if (voiceSource === "manual" && voiceEnabled) {
      if (!voiceSttUrl.trim()) {
        voiceError = translate("settings.voiceUrlRequired");
        return;
      }
      try {
        const u = new URL(voiceSttUrl.trim());
        if (u.protocol !== "https:" && u.protocol !== "http:") {
          voiceError = translate("settings.voiceUrlScheme");
          return;
        }
      } catch {
        voiceError = translate("settings.voiceUrlInvalid");
        return;
      }
      if (!voiceSttModel.trim()) {
        voiceError = translate("settings.voiceModelRequired");
        return;
      }
    }
    // Phase D: validate TTS (independent of STT being enabled).
    if (voiceSource === "manual" && voiceTtsEnabled) {
      if (!voiceTtsUrl.trim()) {
        voiceError = translate("settings.ttsUrlRequired");
        return;
      }
      try {
        const u = new URL(voiceTtsUrl.trim());
        if (u.protocol !== "https:" && u.protocol !== "http:") {
          voiceError = translate("settings.voiceUrlScheme");
          return;
        }
      } catch {
        voiceError = translate("settings.voiceUrlInvalid");
        return;
      }
      if (!voiceTtsModel.trim()) {
        voiceError = translate("settings.ttsModelRequired");
        return;
      }
    }
    try {
      await saveVoiceSettings(
        voiceEnabled, voiceSttUrl.trim(), voiceSttKey.trim(), voiceSttModel.trim(),
        voiceTtsEnabled, voiceTtsUrl.trim(), voiceTtsKey.trim(), voiceTtsModel.trim(),
        voiceTtsAuto, voiceSource,
      );
      voiceSaved = true;
      setTimeout(() => (voiceSaved = false), 2000);
    } catch (e: unknown) {
      voiceError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleSyncCardDav() {
    carddavSyncing = true;
    carddavSyncResult = null;
    carddavError = null;
    try {
      const count = await syncCardDav();
      carddavSyncResult = count;
    } catch (e: unknown) {
      carddavError = e instanceof Error ? e.message : String(e);
    } finally {
      carddavSyncing = false;
    }
  }

  async function loadCaldavAccounts() {
    try {
      caldavAccounts = await listCalDavAccounts();
    } catch (e) {
      console.warn("CalDAV accounts load failed", e);
      caldavAccounts = [];
    }
  }

  function caldavStartAdd() {
    caldavEditingId = "new";
    caldavName = "";
    caldavUrl = "https://";
    caldavUser = "";
    caldavPass = "";
    caldavInterval = 30;
    caldavError = null;
  }

  function caldavStartEdit(a: CalDavAccount) {
    caldavEditingId = a.id;
    caldavName = a.name;
    caldavUrl = a.url;
    caldavUser = a.username;
    caldavPass = "";
    caldavInterval = a.sync_interval_minutes;
    caldavError = null;
  }

  function caldavCancelEdit() {
    caldavEditingId = null;
    caldavError = null;
  }

  async function handleSaveCalDav() {
    caldavError = null;
    caldavSaved = false;
    try {
      if (caldavEditingId && caldavEditingId !== "new") {
        await updateCalDavAccount(caldavEditingId, {
          name: caldavName,
          url: caldavUrl,
          username: caldavUser,
          password: caldavPass || undefined,
          sync_interval_minutes: caldavInterval,
        });
      } else {
        await createCalDavAccount({
          name: caldavName,
          url: caldavUrl,
          username: caldavUser,
          password: caldavPass,
          sync_interval_minutes: caldavInterval,
        });
      }
      caldavSaved = true;
      setTimeout(() => (caldavSaved = false), 2000);
      caldavEditingId = null;
      await loadCaldavAccounts();
    } catch (e: unknown) {
      caldavError = e instanceof Error ? e.message : String(e);
    }
  }

  function cancelDeleteCalDav() {
    pendingDeleteCalDavId = null;
    showDeleteCalDavConfirm = false;
  }

  async function doDeleteCalDav() {
    const id = pendingDeleteCalDavId;
    showDeleteCalDavConfirm = false;
    pendingDeleteCalDavId = null;
    if (!id) return;
    try {
      await deleteCalDavAccount(id);
      if (caldavEditingId === id) caldavEditingId = null;
      await loadCaldavAccounts();
    } catch (e: unknown) {
      caldavError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleToggleCalDav(a: CalDavAccount) {
    try {
      await updateCalDavAccount(a.id, {
        name: a.name, url: a.url, username: a.username,
        enabled: !a.enabled, sync_interval_minutes: a.sync_interval_minutes,
      });
      await loadCaldavAccounts();
    } catch (e: unknown) {
      caldavError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleSyncCalDav() {
    caldavSyncing = true;
    caldavSyncResult = null;
    caldavError = null;
    try {
      const count = await syncCalDav();
      caldavSyncResult = count;
    } catch (e: unknown) {
      caldavError = e instanceof Error ? e.message : String(e);
    } finally {
      caldavSyncing = false;
    }
  }

  // ─── E-Mail-Konto ───────────────────────────
  let acctName = $state("");
  let imapHost = $state("");
  let imapPort = $state(993);
  let imapSsl = $state(true);
  let imapInsecure = $state(false);
  let smtpHost = $state("");
  let smtpPort = $state(587);
  let smtpTls = $state(true);
  let acctUser = $state("");
  let acctPass = $state("");
  let smtpUser = $state("");
  let smtpPass = $state("");
  let senderName = $state("");
  let senderMail = $state("");
  /** When true, empty passwords are taken from the Olares env server-side. */
  let useOlaresPassword = $state(false);
  let acctConnecting = $state(false);
  let acctError = $state<string | null>(null);
  let acctSuccess = $state<string | null>(null);
  let accountList = $state<AccountInfo[]>([]);

  // Used for editing state UI helper
  let isEditing = $state(false);
  let editingAccountId = $state<number | null>(null);

  async function loadAccountList() {
    try {
      const list = await listAccounts();
      accountList = list;
      accounts.setAccounts(list);
      if (list.length > 0) accounts.selectAccount(list[0].id);
    } catch (e) { console.warn("Account list load failed", e); }
  }

  async function handleConnectAccount() {
    if (!acctName || !imapHost || !smtpHost || !acctUser || !senderMail) {
      acctError = translate("error.fillRequired");
      return;
    }
    acctConnecting = true;
    acctError = null;
    acctSuccess = null;
    try {
      if (isEditing && editingAccountId != null) {
        // Edit mode: update the EXISTING account (name, imap_insecure etc.)
        // instead of creating a duplicate.
        await updateAccountSettings(editingAccountId, acctName, undefined, undefined, imapInsecure);
        acctSuccess = translate("settings.accountUpdated", {
          name: acctName,
          cert: imapInsecure ? translate("settings.certInsecure") : translate("settings.certVerified"),
        });
      } else {
        const neu = await connectAccount(
          acctName, imapHost, imapPort, imapSsl,
          smtpHost, smtpPort, smtpTls,
          acctUser, acctPass, smtpUser, smtpPass, senderName, senderMail,
          imapInsecure, useOlaresPassword,
        );
        // Its calendar, tasks and contacts open right away (26.10.18).
        neuesKonto = neu?.id ?? null;
        acctSuccess = translate("settings.accountConnected", { name: acctName });
      }
      
      // Clear fields
      acctName = ""; imapHost = ""; smtpHost = ""; acctUser = "";
      acctPass = ""; smtpUser = ""; smtpPass = ""; senderName = ""; senderMail = "";
      imapPort = 993; imapSsl = true; imapInsecure = false;
      smtpPort = 587; smtpTls = true;
      useOlaresPassword = false;
      isEditing = false;
      editingAccountId = null;
      
      await loadAccountList();
      setTimeout(() => (acctSuccess = null), 4000);
    } catch (e: unknown) {
      acctError = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      acctConnecting = false;
    }
  }

  function connectAndEditAccount(a: AccountInfo) {
    acctName = a.name;
    imapHost = a.imap_host;
    imapPort = a.imap_port;
    imapInsecure = !!a.imap_insecure;
    smtpHost = a.smtp_host;
    smtpPort = a.smtp_port;
    acctUser = a.username;
    smtpUser = a.smtp_username;
    senderName = a.sender_name;
    senderMail = a.sender_email;
    isEditing = true;
    editingAccountId = a.id;
    acctError = null;
    acctSuccess = null;
    
    // Scroll form into view if needed
    const formEl = document.getElementById("account-form");
    if (formEl) {
      formEl.scrollIntoView({ behavior: "smooth" });
    }
  }

  /** Fill the account form from the values Olares injected (secrets stay server-side). */
  async function importFromOlares() {
    acctError = null;
    acctSuccess = null;
    try {
      const s = await getOlaresMailStatus();
      if (!s.configured) {
        acctError = translate("settings.olaresNone");
        return;
      }
      acctName = s.account_name || s.identity.email || acctName;
      if (s.imap.server) imapHost = s.imap.server;
      imapPort = s.imap.port ?? 993;
      imapSsl = s.imap_ssl ?? true;
      acctUser = s.imap.username || s.identity.email || acctUser;
      if (s.smtp.server) smtpHost = s.smtp.server;
      smtpPort = s.smtp.port ?? 587;
      smtpTls = (s.smtp_security ?? "starttls") !== "ssl";
      smtpUser = s.smtp.username || s.imap.username || s.identity.email || "";
      senderName =
        [s.identity.first_name, s.identity.last_name].filter(Boolean).join(" ") ||
        s.identity.username || senderName;
      senderMail = s.smtp_from_address || s.identity.email || senderMail;
      useOlaresPassword = s.complete;
      acctSuccess = translate("settings.olaresImported");
      setTimeout(() => (acctSuccess = null), 4000);
    } catch (e: unknown) {
      acctError = localizeError(e instanceof Error ? e.message : String(e));
    }
  }

  function handleCancelEdit() {
    acctName = ""; imapHost = ""; smtpHost = ""; acctUser = "";
    acctPass = ""; smtpUser = ""; smtpPass = ""; senderName = ""; senderMail = "";
    imapPort = 993; imapSsl = true; imapInsecure = false;
    smtpPort = 587; smtpTls = true;
    useOlaresPassword = false;
    isEditing = false;
    editingAccountId = null;
    acctError = null;
    acctSuccess = null;
  }

  // In-app confirmation (window.confirm is unreliable in the Tauri WKWebView).
  let showDeleteAccountConfirm = $state(false);
  let pendingDeleteAccountId = $state<number | null>(null);
  // Restore-Backup ebenfalls in-app (S4, Review 2026-09-13).
  let showRestoreConfirm = $state(false);
  let pendingRestoreName = $state<string | null>(null);

  function handleDeleteAccount(id: number) {
    pendingDeleteAccountId = id;
    showDeleteAccountConfirm = true;
  }

  async function handleSyncModeChange(accountId: number, mode: string) {
    try {
      await updateAccountSettings(accountId, undefined, mode);
      await loadAccountList();
    } catch (e) {
      console.error("updateAccountSettings failed", e);
      notificationsError = String(e);
    }
  }

  async function confirmDeleteAccount() {
    const id = pendingDeleteAccountId;
    showDeleteAccountConfirm = false;
    pendingDeleteAccountId = null;
    if (id == null) return;
    try {
      await deleteAccount(id);
      await loadAccountList();
      if (isEditing) handleCancelEdit();
    } catch (e: unknown) {
      acctError = localizeError(e instanceof Error ? e.message : String(e));
    }
  }

  function cancelDeleteAccount() {
    showDeleteAccountConfirm = false;
    pendingDeleteAccountId = null;
  }

  // Get initials for Account Avatar
  function getInitials(name: string): string {
    if (!name) return "@";
    return name.trim().split(/\s+/).map(n => n[0]).join("").toUpperCase().slice(0, 2);
  }
</script>

<svelte:head><title>{tabTitel(abschnitt.text)}</title></svelte:head>

<Huelle bereich={null} bind:spalteOffen={sheetOpen}>
  {#snippet spalte()}
    <EinstellungenSpalte
      abschnitte={ABSCHNITTE.map((a) => (a.id === "accounts" ? { ...a, zahl: accountList.length } : a))}
      aktiv={activeTab}
      onwahl={selectTab}
    />
  {/snippet}

  <main class="settings-content-wrapper">
    <!-- HB-SEITENKOPF: the chosen section's title at 28 px, its line under it. -->
    <div class="seitenkopf">
      <div class="einst-kopf">
        <div class="seitenkopf-zeile">
          <h1>{abschnitt.titel}</h1>
          {#if activeTab === "accounts" && accountList.length > 0}
            <span class="seitenkopf-zahl">{accountList.length}</span>
          {/if}
        </div>
        <p class="tab-desc">{abschnitt.desc}</p>
      </div>
    </div>
    <div class="settings-content">

      <!-- ================= TAB: ALLGEMEIN ================= -->
      {#if activeTab === 'general'}

        <!-- Card: Sprache -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.language")}</h3>
            <p class="card-desc">{$t("settings.languageDesc")}</p>
          </div>
          <div class="card-body">
            <div class="lang-toggle" role="group" aria-label={$t("settings.language")}>
              <button type="button" class:active={$lang === "de"} onclick={() => setLang("de")}>Deutsch</button>
              <button type="button" class:active={$lang === "en"} onclick={() => setLang("en")}>English</button>
            </div>
          </div>
        </section>

        <!-- Card: Theme-Auswahl -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.appearance")}</h3>
            <p class="card-desc">{$t("settings.appearanceDesc")}</p>
          </div>
          
          <div class="theme-selection-grid">
            <button
              type="button"
              class="theme-card-option"
              class:active={$appearance === 'light'}
              onclick={() => handleThemeChange('light')}
            >
              <div class="theme-preview light">
                <div class="theme-window-mock">
                  <div class="mock-sidebar"></div>
                  <div class="mock-content">
                    <div class="mock-line short"></div>
                    <div class="mock-line"></div>
                  </div>
                </div>
              </div>
              <div class="theme-option-info">
                <span class="theme-dot light-dot"></span>
                <span class="theme-label">{$t("settings.lightMode")}</span>
              </div>
            </button>

            <button
              type="button"
              class="theme-card-option dark-option"
              class:active={$appearance === 'dark'}
              onclick={() => handleThemeChange('dark')}
            >
              <div class="theme-preview dark">
                <div class="theme-window-mock">
                  <div class="mock-sidebar"></div>
                  <div class="mock-content">
                    <div class="mock-line short"></div>
                    <div class="mock-line"></div>
                  </div>
                </div>
              </div>
              <div class="theme-option-info">
                <span class="theme-dot dark-dot"></span>
                <span class="theme-label">{$t("settings.darkMode")}</span>
              </div>
            </button>

            <button
              type="button"
              class="theme-card-option"
              class:active={$appearance === 'system'}
              onclick={() => handleThemeChange('system')}
            >
              <div class="theme-preview system">
                <div class="theme-window-mock">
                  <div class="mock-sidebar"></div>
                  <div class="mock-content">
                    <div class="mock-line short"></div>
                    <div class="mock-line"></div>
                  </div>
                </div>
              </div>
              <div class="theme-option-info">
                <span class="theme-dot system-dot"></span>
                <span class="theme-label">{$t("settings.systemMode")}</span>
              </div>
            </button>
          </div>
        </section>

        <!-- Card: Postfach Synchronisation -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.mailboxBehavior")}</h3>
            <p class="card-desc">{$t("settings.mailboxBehaviorDesc")}</p>
          </div>

          <div class="card-body">
            <!-- Sync limit -->
            <div class="form-row align-items-center">
              <div class="feld flex-2">
                <label for="fetch-limit-input">{$t("settings.maxMessages")}</label>
                <div class="input-with-badge">
                  <input
                    id="fetch-limit-input"
                    type="number"
                    min="10"
                    max="500"
                    bind:value={fetchLimit}
                    onchange={handleFetchLimitChange}
                    oninput={handleFetchLimitChange}
                   
                  />
                  <span class="input-badge">{$t("settings.mails")}</span>
                </div>
              </div>
              <div class="flex-3 py-2">
                <p class="hint-text mt-4">{$t("settings.fetchLimitHint")}</p>
              </div>
            </div>

            <div class="divider"></div>

            <!-- On/off switch (AM-HAKEN): Move to Trash -->
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-move-to-trash"
                class="schalter"
                class:an={moveToTrash}
                role="switch"
                aria-checked={moveToTrash}
                aria-labelledby="schalter-move-to-trash-titel"
                aria-describedby="schalter-move-to-trash-hinweis"
                onclick={handleMoveToTrashToggle}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-move-to-trash">
                <span id="schalter-move-to-trash-titel">{$t("settings.moveToTrash")}</span>
                <span class="schalter-hinweis" id="schalter-move-to-trash-hinweis">{$t("settings.moveToTrashDesc")}</span>
              </label>
            </div>

            <div class="divider"></div>

            <!-- Erweitert (Kai, 7.10.2026): off by default, Relay looks as before. -->
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-aufraeumen"
                class="schalter"
                class:an={aufraeumen}
                role="switch"
                aria-checked={aufraeumen}
                aria-labelledby="schalter-aufraeumen-titel"
                aria-describedby="schalter-aufraeumen-hinweis"
                onclick={handleAufraeumenToggle}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-aufraeumen">
                <span id="schalter-aufraeumen-titel">{$t("settings.aufraeumen")}</span>
                <span class="schalter-hinweis" id="schalter-aufraeumen-hinweis">{$t("settings.aufraeumenDesc")}</span>
              </label>
            </div>

            <div class="divider"></div>

            <!-- On/off switch (AM-HAKEN): Auto Download Images -->
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-auto-download-images"
                class="schalter"
                class:an={autoDownloadImages}
                role="switch"
                aria-checked={autoDownloadImages}
                aria-labelledby="schalter-auto-download-images-titel"
                aria-describedby="schalter-auto-download-images-hinweis"
                onclick={handleAutoDownloadImagesToggle}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-auto-download-images">
                <span id="schalter-auto-download-images-titel">{$t("settings.autoDownloadImages")}</span>
                <span class="schalter-hinweis" id="schalter-auto-download-images-hinweis">{$t("settings.autoDownloadImagesDesc")}</span>
              </label>
            </div>

            <div class="divider"></div>

            <!-- On/off switch (AM-HAKEN): Push Notifications -->
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-push"
                class="schalter"
                class:an={notificationsEnabled}
                role="switch"
                aria-checked={notificationsEnabled}
                aria-labelledby="schalter-push-titel"
                aria-describedby="schalter-push-hinweis"
                onclick={handleNotificationsToggle} disabled={notificationsBusy}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-push">
                <span id="schalter-push-titel">{$t("settings.push")}</span>
                <span class="schalter-hinweis" id="schalter-push-hinweis">{$t("settings.pushDesc")}</span>
              </label>
            </div>
            {#if notificationsError}
              <div class="hinweis" data-art="fehler" role="alert">
                <Symbol name="achtung" size={16} />
                <span>{notificationsError}</span>
              </div>
            {/if}
          </div>
        </section>
      {/if}

      <!-- ================= TAB: E-MAIL-KONTEN ================= -->
      {#if activeTab === 'accounts'}

        <!-- Liste verbundener Konten -->
        {#if accountList.length > 0}
          <section class="karte seitenkarte settings-card">
            <div class="card-header">
              <h3>{$t("settings.connectedAccounts", { count: accountList.length })}</h3>
              <p class="card-desc">{$t("settings.connectedAccountsDesc")}</p>
            </div>
            
            <div class="account-grid">
              {#each accountList as a (a.id)}
                <div class="account-card-item">
                  <div class="account-avatar">
                    {getInitials(a.name)}
                  </div>
                  <div class="account-details">
                    <div class="account-primary-info">
                      <span class="account-title-name">{a.name}</span>
                      <span class="status-indicator-badge" class:connected={a.connected}>
                        <span class="indicator-dot"></span>
                        {a.connected ? $t("settings.connected") : $t("settings.disconnected")}
                      </span>
                    </div>
                    <p class="account-sub-info">{a.username}</p>
                    <p class="account-tech-info">
                      <span>IMAP: {a.imap_host}:{a.imap_port}</span>
                      <span class="bullet-separator">•</span>
                      <span>SMTP: {a.smtp_host}:{a.smtp_port}</span>
                    </p>
                    <div class="account-sync-row">
                      <label class="sync-mode-label" for={`sync-mode-${a.id}`}>{$t("settings.syncMode")}</label>
                      <select
                        id={`sync-mode-${a.id}`}
                        class="input sync-mode-select"
                        value={a.sync_mode ?? 'mirror'}
                        onchange={(e) => handleSyncModeChange(a.id, (e.currentTarget as HTMLSelectElement).value)}
                      >
                        <option value="mirror">{$t("settings.syncModeMirror")}</option>
                        <option value="archive">{$t("settings.syncModeArchive")}</option>
                      </select>
                      <span class="sync-mode-hint">
                        {a.sync_mode === 'archive'
                          ? $t("settings.syncModeArchiveHint")
                          : $t("settings.syncModeMirrorHint")}
                      </span>
                    </div>
                  </div>
                  <div class="account-actions">
                    <button type="button" class="btn btn-sekundaer" onclick={() => connectAndEditAccount(a)}>
                      {$t("settings.edit")}
                    </button>
                    <button type="button" class="btn btn-sekundaer" onclick={() => handleDeleteAccount(a.id)}>
                      {$t("settings.remove")}
                    </button>
                  </div>
                  <!-- Calendar, tasks and contacts of this account (26.10.18),
                       below the buttons over the card's full width. -->
                  <details class="account-dav" open={a.id === neuesKonto}>
                    <summary>{$t("kontoDav.titel")}</summary>
                    <KontoDav accountId={a.id} />
                  </details>
                </div>
              {/each}
            </div>
          </section>
        {/if}

        <!-- Formular zum Hinzufügen / Bearbeiten -->
        <section class="karte seitenkarte settings-card" id="account-form">
          <div class="card-header">
            <h3>{isEditing ? $t("settings.editAccountTitle") : $t("settings.newAccountTitle")}</h3>
            <p class="card-desc">{$t("settings.accountFormDesc")}</p>
          </div>

          <div class="olares-import-row">
            <button type="button" class="btn btn-sekundaer" onclick={importFromOlares}>
              {$t("settings.olaresImport")}
            </button>
            <span class="olares-import-hint">{$t("settings.olaresImportHint")}</span>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="feld">
                <label for="acct-name">{$t("settings.accountName")}</label>
                <input id="acct-name" bind:value={acctName} placeholder={$t("settings.accountNamePlaceholder")} />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.imapSection")}</div>
            <div class="form-grid-3">
              <div class="feld">
                <label for="imap-host">{$t("settings.serverAddress")}</label>
                <input id="imap-host" bind:value={imapHost} placeholder="imap.provider.com" />
              </div>
              <div class="feld">
                <label for="imap-port">{$t("settings.port")}</label>
                <input id="imap-port" type="number" bind:value={imapPort} />
              </div>
              <div class="toggle-cell">
                <div class="schalter-zeile">
                  <button type="button" id="schalter-imap-ssl" class="schalter" class:an={imapSsl} role="switch" aria-checked={imapSsl} aria-labelledby="schalter-imap-ssl-titel" onclick={() => (imapSsl = !imapSsl)}><span class="schalter-knauf"></span></button>
                  <label class="schalter-text" id="schalter-imap-ssl-titel" for="schalter-imap-ssl">SSL</label>
                </div>
              </div>
              <div class="toggle-cell">
                <div class="schalter-zeile" title={$t("settings.insecureTitle")}>
                  <button type="button" id="schalter-imap-insecure" class="schalter" class:an={imapInsecure} role="switch" aria-checked={imapInsecure} aria-labelledby="schalter-imap-insecure-titel" onclick={() => (imapInsecure = !imapInsecure)}><span class="schalter-knauf"></span></button>
                  <label class="schalter-text" id="schalter-imap-insecure-titel" for="schalter-imap-insecure">{$t("settings.insecureAllow")}</label>
                </div>
              </div>
            </div>

            <div class="form-section-title">{$t("settings.smtpSection")}</div>
            <div class="form-grid-3">
              <div class="feld">
                <label for="smtp-host">{$t("settings.serverAddress")}</label>
                <input id="smtp-host" bind:value={smtpHost} placeholder="smtp.provider.com" />
              </div>
              <div class="feld">
                <label for="smtp-port">{$t("settings.port")}</label>
                <input id="smtp-port" type="number" bind:value={smtpPort} />
              </div>
              <div class="toggle-cell">
                <div class="schalter-zeile">
                  <button type="button" id="schalter-smtp-tls" class="schalter" class:an={smtpTls} role="switch" aria-checked={smtpTls} aria-labelledby="schalter-smtp-tls-titel" onclick={() => (smtpTls = !smtpTls)}><span class="schalter-knauf"></span></button>
                  <label class="schalter-text" id="schalter-smtp-tls-titel" for="schalter-smtp-tls">TLS</label>
                </div>
              </div>
            </div>

            <div class="form-section-title">{$t("settings.imapCredentials")}</div>
            <div class="form-grid-2">
              <div class="feld">
                <label for="acct-user">{$t("settings.username")}</label>
                <input id="acct-user" bind:value={acctUser} placeholder="name@provider.com" />
              </div>
              <div class="feld">
                <label for="acct-pass">{$t("settings.password")}</label>
                <input id="acct-pass" type="password" bind:value={acctPass} placeholder="••••••••••••••••" />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.smtpCredentialsOptional")}</div>
            <div class="form-grid-2">
              <div class="feld">
                <label for="smtp-user">{$t("settings.smtpUsername")}</label>
                <input id="smtp-user" bind:value={smtpUser} placeholder={$t("settings.optionalImapUser")} />
              </div>
              <div class="feld">
                <label for="smtp-pass">{$t("settings.smtpPassword")}</label>
                <input id="smtp-pass" type="password" bind:value={smtpPass} placeholder={$t("settings.optionalImapPassword")} />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.sender")}</div>
            <div class="form-grid-2 mt-2">
              <div class="feld">
                <label for="sender-name">{$t("settings.senderName")}</label>
                <input id="sender-name" bind:value={senderName} placeholder={$t("mail.pnameExample")} />
              </div>
              <div class="feld">
                <label for="sender-mail">{$t("settings.senderMail")}</label>
                <input id="sender-mail" type="text" inputmode="email" bind:value={senderMail} placeholder="name@provider.com" />
              </div>
            </div>

            {#if acctError}
              <div class="hinweis" data-art="fehler" role="alert">
                <Symbol name="achtung" size={16} />
                <span>{acctError}</span>
              </div>
            {/if}
            
            {#if acctSuccess}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{acctSuccess}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              {#if isEditing}
                <button type="button" class="btn btn-sekundaer" onclick={handleCancelEdit}>
                  {$t("common.cancel")}
                </button>
              {/if}
              <button type="button" class="btn btn-primaer" onclick={handleConnectAccount} disabled={acctConnecting}>
                {acctConnecting ? $t("settings.testing") : (isEditing ? $t("settings.saveChanges") : $t("settings.connectAccount"))}
              </button>
            </div>
          </div>
        </section>
      {/if}

      <!-- ================= TAB: KI & TEXT ================= -->
      {#if activeTab === 'ai'}

        <!-- Card: Textgenerierungs-Optionen -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.assistantBehavior")}</h3>
            <p class="card-desc">{$t("settings.assistantBehaviorDesc")}</p>
          </div>

          <div class="card-body">
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-diff-editor"
                class="schalter"
                class:an={$showDiffEnabled}
                role="switch"
                aria-checked={$showDiffEnabled}
                aria-labelledby="schalter-diff-editor-titel"
                aria-describedby="schalter-diff-editor-hinweis"
                onclick={() => ($showDiffEnabled = !$showDiffEnabled)}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-diff-editor">
                <span id="schalter-diff-editor-titel">{$t("settings.diffEditor")}</span>
                <span class="schalter-hinweis" id="schalter-diff-editor-hinweis">{$t("settings.diffEditorDesc")}</span>
              </label>
            </div>
          </div>
        </section>

        <!-- Card: Anbindung (Olares Router default / manual) -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.sourceTitle")}</h3>
            <p class="card-desc">{$t("settings.sourceDesc")}</p>
          </div>

          <div class="card-body">
            <div class="lang-toggle" role="group" aria-label={$t("settings.sourceTitle")}>
              <button type="button" class:active={aiSource === "router"} onclick={() => setAiSource("router")}>
                {$t("settings.sourceRouter")}
              </button>
              <button type="button" class:active={aiSource === "manual"} onclick={() => setAiSource("manual")}>
                {$t("settings.sourceManual")}
              </button>
            </div>

            {#if aiSource === "router"}
              <div class="router-status" class:ok={aiRouterAvailable}>
                <span class="status-dot"></span>
                <span class="status-text">
                  {aiRouterAvailable ? $t("settings.routerDetected") : $t("settings.routerDown")}
                  &middot; {$t("settings.routerModel")}: {aiChatModel}
                </span>
              </div>
              <p class="hint-text">{$t("settings.routerHint")}</p>
              <div class="form-grid-1">
                <div class="feld">
                  <label for="ai-router-url">{$t("settings.routerBase")}</label>
                  <input id="ai-router-url" type="text" value={aiRouterUrl} readonly />
                </div>
              </div>
              {#if !aiRouterAvailable}
                <div class="hinweis" data-art="achtung">
                  <Symbol name="achtung" size={16} />
                  <span>{$t("settings.routerUnavailable")}</span>
                </div>
              {/if}
            {:else}
              <div class="form-grid-1">
                <div class="feld">
                  <label for="ai-url">{$t("settings.apiUrl")}</label>
                  <input id="ai-url" type="url" bind:value={aiUrl} placeholder="https://llm.aimighty.de/v1" />
                </div>
              </div>

              <div class="form-grid-2">
                <div class="feld">
                  <label for="ai-key">{$t("settings.apiKey")}</label>
                  <input id="ai-key" type="password" bind:value={aiKey} placeholder="ollama" />
                </div>
                <div class="feld">
                  <label for="ai-model">{$t("settings.modelId")}</label>
                  <input id="ai-model" type="text" bind:value={aiModel} placeholder="llama3.2" />
                </div>
              </div>
            {/if}

            {#if aiError}
              <div class="hinweis" data-art="fehler" role="alert">
                <Symbol name="achtung" size={16} />
                <span>{aiError}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn btn-primaer" onclick={handleSaveAI}>
                {#if aiSaved}<Symbol name="erfolg" size={16} />{/if} {aiSaved ? $t("settings.saved") : $t("settings.saveConnection")}
              </button>
            </div>
          </div>
        </section>

        <!-- Card: KI-System-Status -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiStatus")}</h3>
            <p class="card-desc">{$t("settings.aiStatusDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-actions-row">
              <button type="button" class="btn btn-sekundaer" onclick={handleResetCircuitBreaker}>
                {#if cbResetDone}<Symbol name="erfolg" size={16} />{/if} {cbResetDone ? $t("settings.aiResetDone") : $t("settings.aiReset")}
              </button>
            </div>
          </div>
        </section>
      {/if}

       <!-- ================= TAB: CARDDAV ================= -->
      {#if activeTab === 'carddav'}

        <!-- Card: CardDAV Settings -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.carddav")}</h3>
            <p class="card-desc">{$t("settings.carddavDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="feld">
                <label for="carddav-url">{$t("settings.serverUrl")}</label>
                <input id="carddav-url" type="url" bind:value={carddavUrl} placeholder="https://nextcloud.example.com/remote.php/dav/addressbooks/users/username/contacts/" />
              </div>
            </div>

            <div class="form-grid-2">
              <div class="feld">
                <label for="carddav-user">{$t("settings.usernameShort")}</label>
                <input id="carddav-user" type="text" bind:value={carddavUser} placeholder={$t("settings.usernameShort")} />
              </div>
              <div class="feld">
                <label for="carddav-pass">{$t("settings.passwordToken")}</label>
                <input id="carddav-pass" type="password" bind:value={carddavPass} placeholder={$t("settings.passwordToken")} />
              </div>
            </div>

            <div class="form-grid-1">
              <div class="feld">
                <label for="carddav-interval">{$t("settings.syncInterval")}</label>
                <div class="input-with-badge">
                  <input id="carddav-interval" type="number" bind:value={carddavInterval} min="1" max="1440" />
                  <span class="input-badge">{$t("settings.minutes")}</span>
                </div>
              </div>
            </div>

            {#if carddavError}
              <div class="hinweis" data-art="fehler" role="alert">
                <Symbol name="achtung" size={16} />
                <span>{carddavError}</span>
              </div>
            {/if}
            
            {#if carddavSaved}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{$t("settings.carddavSaved")}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn btn-sekundaer" onclick={handleSyncCardDav} disabled={carddavSyncing}>
                {carddavSyncing ? $t("settings.syncing") : $t("settings.syncNow")}
              </button>
              <button type="button" class="btn btn-primaer" onclick={handleSaveCardDav}>
                {$t("common.save")}
              </button>
            </div>

            {#if carddavSyncResult !== null}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="neu-laden" size={16} />
                <span>{$t("settings.syncSuccess", { count: carddavSyncResult })}</span>
              </div>
            {/if}
          </div>
        </section>
      {/if}

      <!-- ================= TAB: CALDAV ================= -->
      {#if activeTab === 'caldav'}

        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.caldav")}</h3>
            <p class="card-desc">{$t("settings.caldavDesc")}</p>
          </div>

          <div class="card-body">
            {#if caldavAccounts.length === 0 && caldavEditingId === null}
              <p class="caldav-empty">{$t("settings.caldavNoAccounts")}</p>
            {:else}
              <div class="caldav-list">
                {#each caldavAccounts as a (a.id)}
                  <div class="caldav-row" class:caldav-row--disabled={!a.enabled}>
                    <div class="caldav-row-main">
                      <span class="caldav-row-name">{a.name || a.username}</span>
                      <span class="caldav-row-meta">{a.url} · {$t("settings.syncIntervalShort", { count: a.sync_interval_minutes })}</span>
                    </div>
                    <div class="caldav-row-actions">
                      <button type="button" class="schalter" class:an={a.enabled} role="switch" aria-checked={a.enabled} onclick={() => handleToggleCalDav(a)} aria-label={$t("settings.caldavEnable")} title={$t("settings.caldavEnable")}><span class="schalter-knauf"></span></button>
                      <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => caldavStartEdit(a)}>{$t("settings.caldavEdit")}</button>
                      <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => { pendingDeleteCalDavId = a.id; showDeleteCalDavConfirm = true; }}>{$t("settings.remove")}</button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}

            {#if caldavEditingId === null}
              <div class="form-actions-row">
                <button type="button" class="btn btn-primaer" onclick={caldavStartAdd}>{$t("settings.caldavAdd")}</button>
                <button type="button" class="btn btn-sekundaer" onclick={handleSyncCalDav} disabled={caldavSyncing}>
                  {caldavSyncing ? $t("settings.syncing") : $t("settings.syncNow")}
                </button>
              </div>
            {:else}
              <div class="form-grid-2">
                <div class="feld">
                  <label for="caldav-name">{$t("settings.caldavAccountName")}</label>
                  <input id="caldav-name" type="text" bind:value={caldavName} placeholder={$t("settings.caldavAccountNamePlaceholder")} />
                </div>
                <div class="feld">
                  <label for="caldav-interval">{$t("settings.syncInterval")}</label>
                  <div class="input-with-badge">
                    <input id="caldav-interval" type="number" bind:value={caldavInterval} min="1" max="1440" />
                    <span class="input-badge">{$t("settings.minutes")}</span>
                  </div>
                </div>
              </div>

              <div class="form-grid-1">
                <div class="feld">
                  <label for="caldav-url">{$t("settings.serverUrl")}</label>
                  <input id="caldav-url" type="url" bind:value={caldavUrl} placeholder="https://nextcloud.example.com/remote.php/dav/calendars/username/" />
                </div>
              </div>

              <div class="form-grid-2">
                <div class="feld">
                  <label for="caldav-user">{$t("settings.usernameShort")}</label>
                  <input id="caldav-user" type="text" bind:value={caldavUser} placeholder={$t("settings.usernameShort")} />
                </div>
                <div class="feld">
                  <label for="caldav-pass">{$t("settings.passwordToken")}</label>
                  <input id="caldav-pass" type="password" bind:value={caldavPass} placeholder={caldavEditingId !== "new" ? $t("settings.caldavPasswordKeep") : $t("settings.passwordToken")} />
                </div>
              </div>

              {#if caldavError}
                <div class="hinweis" data-art="fehler" role="alert">
                  <Symbol name="achtung" size={16} />
                  <span>{caldavError}</span>
                </div>
              {/if}

              {#if caldavSaved}
                <div class="hinweis" data-art="erfolg" role="status">
                  <Symbol name="erfolg" size={16} />
                  <span>{$t("settings.caldavSaved")}</span>
                </div>
              {/if}

              <div class="form-actions-row">
                <button type="button" class="btn btn-sekundaer" onclick={caldavCancelEdit}>{$t("common.cancel")}</button>
                <button type="button" class="btn btn-primaer" onclick={handleSaveCalDav}>{$t("common.save")}</button>
              </div>
            {/if}

            {#if caldavSyncResult !== null}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="neu-laden" size={16} />
                <span>{$t("settings.syncSuccessCal", { count: caldavSyncResult })}</span>
              </div>
            {/if}
          </div>
        </section>
      {/if}

      <!-- ================= TAB: VOICE ================= -->
      {#if activeTab === 'voice'}
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.voice2mail")}</h3>
            <p class="card-desc">{$t("settings.voice2mailDesc")}</p>
          </div>

          <div class="card-body">
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-voice-enable"
                class="schalter"
                class:an={voiceEnabled}
                role="switch"
                aria-checked={voiceEnabled}
                aria-labelledby="schalter-voice-enable-titel"
                aria-describedby="schalter-voice-enable-hinweis"
                onclick={() => (voiceEnabled = !voiceEnabled)}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-voice-enable">
                <span id="schalter-voice-enable-titel">{$t("settings.voiceEnable")}</span>
                <span class="schalter-hinweis" id="schalter-voice-enable-hinweis">{$t("settings.voiceEnableDesc")}</span>
              </label>
            </div>
          </div>
        </section>

        <!-- Card: Sprach-Anbindung (Olares Router default / manual) -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.sourceTitle")}</h3>
            <p class="card-desc">{$t("settings.sourceDesc")}</p>
          </div>

          <div class="card-body">
            <div class="lang-toggle" role="group" aria-label={$t("settings.sourceTitle")}>
              <button type="button" class:active={voiceSource === "router"} onclick={() => (voiceSource = "router")}>
                {$t("settings.sourceRouter")}
              </button>
              <button type="button" class:active={voiceSource === "manual"} onclick={() => (voiceSource = "manual")}>
                {$t("settings.sourceManual")}
              </button>
            </div>

            {#if voiceSource === "router"}
              <div class="router-status" class:ok={aiRouterAvailable}>
                <span class="status-dot"></span>
                <span class="status-text">
                  {aiRouterAvailable ? $t("settings.routerDetected") : $t("settings.routerDown")}
                </span>
              </div>
              <p class="hint-text">{$t("settings.voiceRouterStatus", { stt: "default-stt", tts: "default-tts" })}</p>
            {/if}
          </div>
        </section>

        <!-- Card: STT Endpoint -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.sttEndpoint")}</h3>
            <p class="card-desc">{$t("settings.sttEndpointDesc")}</p>
          </div>

          <div class="card-body">
            {#if voiceSource === "manual"}
              <div class="form-grid-1">
                <div class="feld">
                  <label for="voice-stt-url">{$t("settings.apiUrl")}</label>
                  <input id="voice-stt-url" type="url" bind:value={voiceSttUrl} placeholder="https://speaches.aimighty.de/v1" disabled={!voiceEnabled} />
                </div>
              </div>

              <div class="form-grid-2">
                <div class="feld">
                  <label for="voice-stt-key">{$t("settings.apiKey")}</label>
                  <input id="voice-stt-key" type="password" bind:value={voiceSttKey} placeholder={$t("settings.optional")} disabled={!voiceEnabled} />
                </div>
                <div class="feld">
                  <label for="voice-stt-model">{$t("settings.modelId")}</label>
                  <input id="voice-stt-model" type="text" bind:value={voiceSttModel} placeholder="Systran/faster-whisper-small" disabled={!voiceEnabled} />
                </div>
              </div>
            {:else}
              <p class="hint-text">{$t("settings.routerHint")}</p>
            {/if}
          </div>
        </section>

        <!-- Card: TTS (Phase D) -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.ttsEndpoint")}</h3>
            <p class="card-desc">{$t("settings.ttsEndpointDesc")}</p>
          </div>

          <div class="card-body">
            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-tts-enable"
                class="schalter"
                class:an={voiceTtsEnabled}
                role="switch"
                aria-checked={voiceTtsEnabled}
                aria-labelledby="schalter-tts-enable-titel"
                aria-describedby="schalter-tts-enable-hinweis"
                onclick={() => (voiceTtsEnabled = !voiceTtsEnabled)}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-tts-enable">
                <span id="schalter-tts-enable-titel">{$t("settings.ttsEnable")}</span>
                <span class="schalter-hinweis" id="schalter-tts-enable-hinweis">{$t("settings.ttsEnableDesc")}</span>
              </label>
            </div>

            {#if voiceSource === "manual"}
              <div class="form-grid-1">
                <div class="feld">
                  <label for="voice-tts-url">{$t("settings.apiUrl")}</label>
                  <input id="voice-tts-url" type="url" bind:value={voiceTtsUrl} placeholder="https://speaches.aimighty.de/v1" disabled={!voiceTtsEnabled} />
                </div>
              </div>

              <div class="form-grid-2">
                <div class="feld">
                  <label for="voice-tts-key">{$t("settings.apiKey")}</label>
                  <input id="voice-tts-key" type="password" bind:value={voiceTtsKey} placeholder={$t("settings.optional")} disabled={!voiceTtsEnabled} />
                </div>
                <div class="feld">
                  <label for="voice-tts-model">{$t("settings.modelId")}</label>
                  <input id="voice-tts-model" type="text" bind:value={voiceTtsModel} placeholder="tts-1" disabled={!voiceTtsEnabled} />
                </div>
              </div>
            {:else}
              <p class="hint-text">{$t("settings.voiceRouterStatus", { stt: "default-stt", tts: "default-tts" })}</p>
            {/if}

            <div class="schalter-zeile">
              <button
                type="button"
                id="schalter-tts-auto"
                class="schalter"
                class:an={voiceTtsAuto}
                role="switch"
                aria-checked={voiceTtsAuto}
                aria-labelledby="schalter-tts-auto-titel"
                aria-describedby="schalter-tts-auto-hinweis"
                onclick={() => (voiceTtsAuto = !voiceTtsAuto)}
              ><span class="schalter-knauf"></span></button>
              <label class="schalter-text" for="schalter-tts-auto">
                <span id="schalter-tts-auto-titel">{$t("settings.ttsAuto")}</span>
                <span class="schalter-hinweis" id="schalter-tts-auto-hinweis">{$t("settings.ttsAutoDesc")}</span>
              </label>
            </div>
          </div>
        </section>

        {#if voiceError}
          <div class="hinweis" data-art="fehler" role="alert">
            <Symbol name="achtung" size={16} />
            <span>{voiceError}</span>
          </div>
        {/if}

        <div class="form-actions-row">
          <button type="button" class="btn btn-primaer" onclick={handleSaveVoice}>
            {#if voiceSaved}<Symbol name="erfolg" size={16} />{/if} {voiceSaved ? $t("settings.saved") : $t("settings.saveConnection")}
          </button>
        </div>
      {/if}

      {#if activeTab === 'archive'}

        <!-- Card: Delete Queue Review -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.deleteQueue", { count: deleteQueue.length })}</h3>
            <p class="card-desc">{$t("settings.deleteQueueDesc")}</p>
          </div>
          {#if deleteQueue.length === 0}
            <p class="hint-text">{$t("settings.noPending")}</p>
          {:else}
            <div class="delete-queue-list">
              {#each deleteQueue as row}
                <div class="delete-queue-row">
                  <div class="delete-queue-info">
                    <span class="delete-queue-uid">{$t("settings.accountUid", { account: row.account_id, uid: row.uid })}</span>
                    <span class="delete-queue-folder">{row.folder}</span>
                    <span class="delete-queue-state" class:failed={row.state === 'failed'}>{row.state}</span>
                    {#if row.last_error}
                      <span class="delete-queue-error">{row.last_error}</span>
                    {/if}
                  </div>
                  <div class="delete-queue-actions">
                    <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => retryDeleteQueue(row.id)}>{$t("settings.retry")}</button>
                    <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => removeDeleteQueue(row.id)}>{$t("settings.discard")}</button>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>

        <!-- Card: Export -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.exportTitle")}</h3>
            <p class="card-desc">{$t("settings.exportDesc")}</p>
          </div>
          <div class="export-row">
            {#each accountList as a (a.id)}
              <div class="export-account">
                <span class="export-account-name">{a.name}</span>
                <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => downloadExport(a.id, "mbox")}>MBox</button>
                <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => downloadExport(a.id, "zip")}>EML-ZIP</button>
              </div>
            {/each}
            {#if accountList.length === 0}
              <p class="hint-text">{$t("settings.noAccountsExport")}</p>
            {/if}
          </div>
        </section>

        <!-- Card: Backup -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.backupTitle")}</h3>
            <p class="card-desc">{$t("settings.backupDesc")}</p>
          </div>
          <div class="export-row">
            <button type="button" class="btn btn-primaer" onclick={handleBackup} disabled={backupBusy}>
              {backupBusy ? $t("settings.createBackupBusy") : $t("settings.createBackup")}
            </button>
            {#if backupResult}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{$t("settings.backupCreated", { path: backupResult.path, size: formatBytes(backupResult.size) })}</span>
              </div>
            {/if}
          </div>

          <div class="card-header backup-list-header">
            <h4>{$t("settings.existingBackups")}</h4>
          </div>
          <div class="backup-list">
            {#each backups as b (b.name)}
              <div class="backup-row">
                <span class="backup-name">{b.name}</span>
                <span class="backup-size">{formatBytes(b.size)}</span>
                <button type="button" class="btn btn-sekundaer btn-klein" onclick={() => askRestoreBackup(b.name)}>{$t("settings.restore")}</button>
              </div>
            {:else}
              <p class="hint-text">{$t("settings.noBackups")}</p>
            {/each}
          </div>
          {#if restoreResult}
            <!-- Neutral notice: the line reports either outcome. -->
            <div class="hinweis" role="status">
              <Symbol name="hinweis" size={16} />
              <span>{restoreResult}</span>
            </div>
          {/if}
        </section>
      {/if}

      {#if activeTab === 'cache'}

        <!-- Card: Cache Statistics -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.cacheStats")}</h3>
            <p class="card-desc">{$t("settings.cacheStatsDesc")}</p>
          </div>

          <div class="card-body">
            <div class="cache-stats">
              <div class="stat-item">
                <span class="stat-label">{$t("settings.attachmentsTotal")}</span>
                <span class="stat-value">{cacheStats?.total_attachments ?? 0}</span>
              </div>
              <div class="stat-item">
                <span class="stat-label">{$t("settings.cachedWithContent")}</span>
                <span class="stat-value">{cacheStats?.cached_count ?? 0}</span>
              </div>
              <div class="stat-item">
                <span class="stat-label">{$t("settings.cacheSize")}</span>
                <span class="stat-value">{(cacheStats?.cached_size_mb ?? 0).toFixed(1)} MB</span>
              </div>
            </div>
          </div>
        </section>

        <!-- Card: Cache Cleanup -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.cacheCleanup")}</h3>
            <p class="card-desc">{$t("settings.cacheCleanupDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-2">
              <div class="feld">
                <label for="cache-max-mb">{$t("settings.cacheMaxMb")}</label>
                <input id="cache-max-mb" type="number" bind:value={cacheMaxMb} min="10" max="500" />
              </div>
            </div>

            {#if cacheCleanupResult !== null}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{$t("settings.cacheCleaned", { count: cacheCleanupResult })}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn btn-primaer" onclick={handleCleanupCache} disabled={cacheCleaning}>
                {cacheCleaning ? $t("settings.cleaning") : $t("settings.cacheCleanup")}
              </button>
              <button type="button" class="btn btn-sekundaer" onclick={() => (pendingClear = "cache")} disabled={cacheCleaning}>
                {cacheCleaning ? $t("settings.clearing") : $t("settings.clearAll")}
              </button>
            </div>
          </div>
        </section>

        <!-- Card: KI-Zusammenfassungen -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiSummaries")}</h3>
            <p class="card-desc">{$t("settings.aiSummariesDesc")}</p>
          </div>

          <div class="card-body">
            {#if aiSummariesResult !== null}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{$t("settings.aiSummariesCleared", { count: aiSummariesResult })}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn btn-sekundaer" onclick={() => (pendingClear = "summaries")} disabled={aiSummariesClearing}>
                {aiSummariesClearing ? $t("settings.clearing") : $t("settings.aiSummariesClearAll")}
              </button>
            </div>
          </div>
        </section>

        <!-- Card: KI-Aktionen -->
        <section class="karte seitenkarte settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiActions")}</h3>
            <p class="card-desc">{$t("settings.aiActionsDesc")}</p>
          </div>

          <div class="card-body">
            {#if aiActionsResult !== null}
              <div class="hinweis" data-art="erfolg" role="status">
                <Symbol name="erfolg" size={16} />
                <span>{$t("settings.aiActionsCleared", { count: aiActionsResult })}</span>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn btn-sekundaer" onclick={() => (pendingClear = "actions")} disabled={aiActionsClearing}>
                {aiActionsClearing ? $t("settings.clearing") : $t("settings.aiActionsClearAll")}
              </button>
            </div>
          </div>
        </section>
      {/if}

    </div>
  </main>
</Huelle>

<ConfirmationDialog
  open={showDeleteAccountConfirm}
  title={$t("settings.deleteAccountTitle")}
  message={$t("settings.deleteAccountMessage")}
  confirmLabel={$t("settings.remove")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={confirmDeleteAccount}
  oncancel={cancelDeleteAccount}
/>

<ConfirmationDialog
  open={showDeleteCalDavConfirm}
  title={$t("settings.caldavDeleteTitle")}
  message={$t("settings.caldavDeleteMessage")}
  confirmLabel={$t("settings.remove")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={doDeleteCalDav}
  oncancel={cancelDeleteCalDav}
/>

<ConfirmationDialog
  open={pendingClear !== null}
  title={$t("settings.clearConfirmTitle")}
  message={pendingClear === "cache" ? $t("settings.clearConfirmCache")
    : pendingClear === "summaries" ? $t("settings.clearConfirmSummaries")
    : $t("settings.clearConfirmActions")}
  confirmLabel={pendingClear === "cache" ? $t("settings.clearAll")
    : pendingClear === "summaries" ? $t("settings.aiSummariesClearAll")
    : $t("settings.aiActionsClearAll")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={confirmClear}
  oncancel={() => (pendingClear = null)}
/>

<ConfirmationDialog
  open={showRestoreConfirm}
  title={$t("settings.restoreTitle")}
  message={pendingRestoreName ? translate("settings.restoreConfirm", { name: pendingRestoreName }) : ""}
  confirmLabel={$t("settings.restore")}
  cancelLabel={$t("common.cancel")}
  danger={true}
  onconfirm={confirmRestoreBackup}
  oncancel={() => { showRestoreConfirm = false; pendingRestoreName = null; }}
/>

  <AssistantFab module="settings" />

<style>
  /* ── Settings page inside the shell [RL-EINSTELLUNGEN] ────────────────────
     The shell draws header, column and sections (EinstellungenSpalte); the
     page is the scrolling main pane with the HB-SEITENKOPF on top and the
     cards in the reading column (CI G1: forms and settings 720 px), aligned
     with the title. */
  .settings-content-wrapper {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    background: var(--am-seite);
  }

  .einst-kopf {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-1);
    min-width: 0;
  }

  .tab-desc {
    font-size: 0.875rem;
    color: var(--am-text-gedaempft);
    margin: 0;
  }

  .settings-content {
    width: 100%;
    max-width: calc(var(--am-lesespalte) + 2 * var(--am-raum-8));
    padding: var(--am-raum-6) var(--am-raum-8) var(--am-raum-16);
  }

  @media (max-width: 40rem) {
    .settings-content { padding: var(--am-raum-4) var(--am-raum-4) var(--am-raum-16); }
  }

  /* AM-KARTE draws the card; only the spacing between cards is ours. */
  .settings-card {
    margin-bottom: 20px;
  }

  .card-header {
    margin-bottom: 20px;
  }

  .card-header h3 {
    font-size: 1rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--am-text-primaer);
    margin: 0 0 4px 0;
  }

  .card-desc {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
    margin: 0;
    line-height: 1.5;
  }

  .card-body {
    display: flex;
    flex-direction: column;
  }

  /* ── Appearance cards (stay Relay-own until HB-DARSTELLUNG, Etappe 6) [RL-EINSTELLUNGEN] ─── */
  .theme-selection-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 16px;
  }

  .theme-card-option {
    background: transparent;
    border: 1.5px solid var(--am-rand);
    border-radius: 10px;
    padding: 12px;
    cursor: pointer;
    text-align: left;
    transition: all var(--am-dauer-mittel) var(--am-kurve);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .theme-card-option:hover {
    border-color: var(--am-text-gedaempft);
  }

  .theme-card-option.active {
    border-color: var(--am-handlung-ruhend);
    background: var(--am-flaeche-2);
    box-shadow: 0 0 0 1px var(--am-handlung-ruhend);
  }

  .theme-card-option.dark-option {
    background: var(--am-blau-800);
    border-color: var(--am-blau-600);
  }

  .theme-card-option.dark-option .theme-label {
    color: var(--am-blau-200);
  }

  .theme-card-option.dark-option:hover {
    border-color: var(--am-blau-500);
  }

  .theme-card-option.dark-option.active {
    background: var(--am-blau-700);
    border-color: var(--am-handlung-ruhend);
    box-shadow: 0 0 0 1px var(--am-handlung-ruhend);
  }

  .theme-preview {
    height: 90px;
    border-radius: 6px;
    padding: 8px;
    display: flex;
    align-items: stretch;
    overflow: hidden;
    border: 1px solid var(--am-rand);
  }

  .theme-preview.light {
    background: var(--am-text-auf-farbe);
  }

  .theme-preview.dark {
    background: var(--am-blau-800);
    border-color: var(--am-blau-600);
  }

  .theme-preview.dark .theme-window-mock {
    border-color: var(--am-blau-600);
  }

  .theme-window-mock {
    flex: 1;
    display: flex;
    border-radius: 4px;
    overflow: hidden;
    border: 1px solid var(--am-rand);
    box-shadow: none;
  }

  .mock-sidebar {
    width: 25%;
    background: var(--am-flaeche-1);
    border-right: 1px solid var(--am-rand);
  }

  .light .mock-sidebar { background: var(--am-blau-50); border-right: 1px solid var(--am-blau-200); }
  .dark .mock-sidebar { background: var(--am-blau-900); border-right: 1px solid var(--am-blau-600); }

  .mock-content {
    flex: 1;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--am-seite);
  }

  .light .mock-content { background: var(--am-text-auf-farbe); }
  .dark .mock-content { background: var(--am-blau-800); }

  .mock-line {
    height: 4px;
    border-radius: 2px;
    background: var(--am-rand);
  }

  .light .mock-line { background: var(--am-blau-200); }
  .dark .mock-line { background: var(--am-blau-600); }

  .mock-line.short {
    width: 60%;
  }

  .theme-option-info {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .theme-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }

  .light-dot { background: var(--am-blau-800); box-shadow: 0 0 0 1px var(--am-rand-betont-farbe); }
  .dark-dot { background: var(--am-gold-500); }
  .system-dot { background: linear-gradient(90deg, var(--am-blau-800) 50%, var(--am-gold-500) 50%); }
  /* System: left half light, right half dark — the mock follows the light card. */
  .theme-preview.system { background: linear-gradient(90deg, var(--am-text-auf-farbe) 50%, var(--am-blau-800) 50%); }

  .theme-label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--am-text-primaer);
  }

  /* ── Segmented choice: language and source (future HB-SEGMENT) [RL-EINSTELLUNGEN] ─── */
  .lang-toggle {
    display: inline-flex;
    gap: 4px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    padding: 2px;
    background: var(--am-flaeche-1);
    width: fit-content;
  }
  .lang-toggle button {
    background: transparent;
    border: none;
    color: var(--am-text-gedaempft);
    font-size: 0.8125rem;
    font-weight: 600;
    padding: 6px 14px;
    border-radius: 4px;
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .lang-toggle button.active {
    background: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
  }
  .lang-toggle button:focus-visible {
    outline: 2px solid var(--am-fokus-ring);
    outline-offset: 1px;
  }

  /* ── Router status line [RL-EINSTELLUNGEN] ─────────────────────────────── */
  .router-status {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
  }
  .router-status .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--am-text-gedaempft);
    flex: none;
  }
  .router-status.ok .status-dot {
    background: var(--am-erfolg);
  }

  /* ── Field layout (AM-FELD draws the fields) [RL-EINSTELLUNGEN] ────────── */
  .form-grid-1 {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
    margin-bottom: 16px;
  }

  .form-grid-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    margin-bottom: 16px;
  }

  .form-grid-3 {
    display: grid;
    grid-template-columns: 3fr 1fr auto;
    gap: 16px;
    align-items: flex-end;
    margin-bottom: 16px;
  }

  /* Grid gap spaces the fields; AM-FELD's own bottom margin would double it. */
  .form-grid-1 > .feld,
  .form-grid-2 > .feld,
  .form-grid-3 > .feld,
  .form-row > .feld {
    margin-bottom: 0;
  }

  /* Form row and helper layouts */
  .form-row {
    display: flex;
    gap: 16px;
  }

  .align-items-center {
    align-items: center;
  }

  .flex-2 { flex: 2; }
  .flex-3 { flex: 3; }
  /* SSL/TLS switches sit in the field grid, centred against the inputs. */
  .toggle-cell { justify-self: center; align-self: center; padding-bottom: var(--am-raum-3); }
  .toggle-cell .schalter-zeile { margin: 0; }

  .divider {
    height: 1px;
    background: var(--am-rand);
    margin: 16px 0;
  }

  .form-section-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin: 24px 0 12px 0;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--am-rand);
  }

  .input-with-badge {
    position: relative;
    display: flex;
    align-items: center;
  }

  .input-with-badge input {
    padding-right: 64px;
  }

  .input-badge {
    position: absolute;
    right: 12px;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
    text-transform: uppercase;
    background: var(--am-flaeche-1);
    padding: 4px 8px;
    border-radius: 4px;
    pointer-events: none;
    border: 1px solid var(--am-rand);
  }

  .hint-text {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
    line-height: 1.5;
    margin: 0;
  }

  /* ── Notices and action rows (HB-ZUSTAND, AM-KNOPF draw them) [RL-EINSTELLUNGEN] ─── */
  .hinweis {
    margin: var(--am-raum-4) 0;
  }

  .form-actions-row {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
    margin-top: 20px;
  }


  /* ── Cache statistics [RL-EINSTELLUNGEN] ───────────────────────────────── */
  .cache-stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 16px;
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .stat-label {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
  }

  .stat-value {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--am-text-primaer);
  }

  /* ── Mail accounts [RL-EINSTELLUNGEN] ──────────────────────────────────── */
  .account-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .account-card-item {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    padding: 16px;
    /* The rows sit on a .karte (flaeche-1), so they take the page colour. */
    background: var(--am-seite);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    transition: border-color var(--am-dauer-schnell) var(--am-kurve);
  }

  .account-card-item:hover {
    border-color: var(--am-text-gedaempft);
  }

  .account-avatar {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 1rem;
    box-shadow: none;
    flex-shrink: 0;
  }

  .account-details {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .account-primary-info {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .account-title-name {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--am-text-primaer);
  }

  .status-indicator-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 20px;
    font-size: 0.6875rem;
    font-weight: 600;
    background: color-mix(in srgb, var(--am-fehler) 8%, var(--am-seite));
    color: var(--am-fehler);
    border: 1px solid color-mix(in srgb, var(--am-fehler) 20%, transparent);
  }

  .status-indicator-badge.connected {
    background: color-mix(in srgb, var(--am-erfolg) 8%, var(--am-seite));
    color: var(--am-erfolg);
    border: 1px solid color-mix(in srgb, var(--am-erfolg) 20%, transparent);
  }

  .indicator-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--am-fehler);
  }

  .connected .indicator-dot {
    background: var(--am-erfolg);
  }

  .account-sub-info {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
    margin: 0;
  }

  .account-tech-info {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
    margin: 2px 0 0 0;
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .account-sync-row {
    margin-top: 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .sync-mode-label {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
  }

  /* Inline in the row: AM-FELD's .input draws it, only the width is ours. */
  .sync-mode-select {
    width: auto;
  }

  .sync-mode-hint {
    font-size: 0.7rem;
    color: var(--am-text-gedaempft);
  }

  .bullet-separator {
    color: var(--am-rand);
  }

  .account-actions {
    display: flex;
    gap: 8px;
  }
  /* Phone: the buttons go below the details instead of to the edge. */
  @media (max-width: 40rem) {
    .account-card-item { flex-wrap: wrap; align-items: flex-start; }
    .account-details { flex: 1 1 0; min-width: 0; }
    .account-actions { flex: 1 0 100%; justify-content: flex-end; }
  }

  .olares-import-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding: 0 24px 16px;
  }

  .olares-import-hint {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
  }

  /* ── Archive: delete queue, export, backup [RL-EINSTELLUNGEN] ──────────── */
  .backup-list-header {
    margin-top: 16px;
  }

  .backup-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .backup-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border: 1px solid var(--am-rand);
    border-radius: 8px;
    font-size: 0.8rem;
  }

  .backup-name {
    font-weight: 600;
    flex: 1;
  }

  .backup-size {
    color: var(--am-text-gedaempft);
  }

  .export-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .export-account {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 8px 4px;
  }

  /* Takes the line on a phone, the buttons wrap below (tour with data). */
  .export-account-name {
    flex: 1 1 180px;
    font-size: 0.85rem;
    font-weight: 600;
  }

  .delete-queue-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .delete-queue-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--am-rand);
    border-radius: 10px;
    background: var(--am-seite);
  }

  .delete-queue-info {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 0.8rem;
  }

  .delete-queue-uid {
    font-weight: 600;
  }

  .delete-queue-folder {
    color: var(--am-text-gedaempft);
  }

  .delete-queue-state {
    font-size: 0.7rem;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--am-flaeche-2);
  }

  .delete-queue-state.failed {
    background: color-mix(in srgb, var(--am-fehler) 12%, transparent);
    color: var(--am-fehler);
  }

  .delete-queue-error {
    font-size: 0.72rem;
    color: var(--am-fehler);
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .delete-queue-actions {
    display: flex;
    gap: 6px;
  }

  /* ── CalDAV accounts [RL-EINSTELLUNGEN] ────────────────────────────────── */
  .caldav-empty {
    color: var(--am-text-gedaempft);
    font-size: 0.875rem;
    margin: 0 0 var(--am-raum-4);
  }
  .caldav-list {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-2);
    margin-bottom: var(--am-raum-8);
  }
  .caldav-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--am-raum-4);
    padding: var(--am-raum-4) var(--am-raum-8);
    border: 1px solid var(--am-rand);
    border-radius: 10px;
    background: var(--am-seite);
  }
  .caldav-row--disabled {
    opacity: 0.55;
  }
  .caldav-row-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .caldav-row-name {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--am-text-primaer);
  }
  .caldav-row-meta {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .caldav-row-actions {
    display: flex;
    align-items: center;
    gap: var(--am-raum-2);
    flex-shrink: 0;
  }
  /* ── Calendar, tasks and contacts per account [RL-KONTODAV] ──────── */
  /* Its own row under name and buttons, lined up with the name. */
  .account-card-item { flex-wrap: wrap; }
  .account-dav { flex: 0 0 calc(100% - 60px); min-width: 0; margin-left: 60px; }
  @media (max-width: 40rem) {
    .account-dav { flex-basis: 100%; margin-left: 0; }
  }

  /* ── Phone [RL-EINSTELLUNGEN] ─────────────────────────────────────────── */
  /* The cards run edge to edge (AM-KARTE .seitenkarte, Kai 10.10.2026);
     inside them nothing is boxed a second time, two fields side by side
     only where both fit, buttons keep their words on one line. */
  @media (max-width: 40rem) {
    .settings-card { margin-bottom: var(--am-raum-3); }
    .form-grid-2 { grid-template-columns: minmax(0, 1fr); }
    /* Server alone, then port and the switches in one row. */
    .form-grid-3 { grid-template-columns: minmax(0, 6rem) auto minmax(0, 1fr); }
    .form-grid-3 > .feld:first-child { grid-column: 1 / -1; }
    .form-grid-3 > .toggle-cell { justify-self: start; }
    /* "Unsicher erlauben" gets a row of its own. */
    .form-grid-3 > :nth-child(4) { grid-column: 1 / -1; }
    .toggle-cell .schalter-text { white-space: nowrap; }
    /* Accounts as rows with a line between, not cards in the card. */
    .account-card-item {
      border: 0;
      border-top: 1px solid var(--am-rand);
      border-radius: 0;
      padding: var(--am-raum-4) 0;
      background: transparent;
    }
    .account-card-item:first-child { border-top: 0; padding-top: 0; }
    .caldav-row { flex-wrap: wrap; padding: var(--am-raum-3) var(--am-raum-4); }
    .caldav-row-main { flex: 1 1 100%; }
    .caldav-row-actions { flex: 1 1 100%; justify-content: flex-end; }
    .form-actions-row { flex-wrap: wrap; }
    .form-actions-row > .btn { flex: 1 1 auto; }
    .settings-card .btn { white-space: nowrap; }
    .export-account-name { flex-basis: 100%; }
  }
  .account-dav > summary { cursor: pointer; font-weight: 600; font-size: var(--fs-sm); color: var(--am-text-primaer); }
  .account-dav[open] > summary { margin-bottom: var(--am-raum-3); }
</style>
