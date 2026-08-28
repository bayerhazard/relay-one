<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
import {
    getSettings, saveSettings,
    connectAccount, listAccounts, deleteAccount, updateAccountSettings,
    getMoveToTrash, setMoveToTrash,
    getCardDavSettings, setCardDavSettings, syncCardDav, getOwnPhoto, saveOwnPhoto,
    getCalDavSettings, setCalDavSettings, syncCalDav,
    getVoiceSettings, saveVoiceSettings,
    resetCircuitBreaker,
    getAttachmentCacheStats, cleanupAttachmentCache, clearAttachmentCache, clearAiSummaries,
    setupPush, teardownPush, pushEnabled,
    getDeleteQueue, retryDeleteQueueRow, removeDeleteQueueRow, downloadExport, createBackup, listBackups, restoreBackupSnapshot,
  } from "$lib/services/tauri";
  import type { AccountInfo } from "$lib/stores/accounts";
  import { accounts } from "$lib/stores/accounts";
  import { settings, showDiffEnabled } from "$lib/stores/settings";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import ModuleIcons from "$lib/components/ModuleIcons.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import { t, lang, setLang, translate, localizeError } from "$lib/i18n";

  // ─── Active Tab State ────────────────────────
  let activeTab = $state("general"); // 'general' | 'accounts' | 'ai' | 'carddav' | 'voice'

  // ─── Mobile drill-down (iOS settings style) ──
  // On phones (≤600px) the menu list fills the screen; tapping an item
  // pushes the content view with a back button returning to the menu.
  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isNarrow = $derived(viewportWidth <= 600);
  let mobileContentOpen = $state(false);

  $effect(() => {
    if (typeof window === "undefined") return;
    let raf = 0;
    const onResize = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => { viewportWidth = window.innerWidth; });
    };
    window.addEventListener("resize", onResize);
    return () => { cancelAnimationFrame(raf); window.removeEventListener("resize", onResize); };
  });

  function selectTab(tab: string) {
    activeTab = tab;
    if (isNarrow) mobileContentOpen = true;
  }

  // ─── Theme ───────────────────────────────────
  let theme = $state("blue");
  try { theme = localStorage.getItem("relay_theme") || "blue"; } catch {}

  $effect(() => {
    if (typeof document !== 'undefined') {
      if (theme === "dark") {
        document.documentElement.classList.add("theme-dark");
      } else {
        document.documentElement.classList.remove("theme-dark");
      }
      localStorage.setItem("relay_theme", theme);
      // macOS: the web-app titlebar uses theme-color — match it to the theme.
      let meta = document.querySelector('meta[name="theme-color"]') as HTMLMetaElement | null;
      if (!meta) {
        meta = document.createElement("meta");
        meta.name = "theme-color";
        document.head.appendChild(meta);
      }
      meta.content = theme === "dark" ? "#0a2238" : "#f4f7fa";
    }
  });

  function handleThemeChange(newTheme: string) {
    theme = newTheme;
  }

  // ─── LLM ─────────────────────────────────────
  let aiUrl = $state("https://llm.aimighty.de/v1");
  let aiKey = $state("ollama");
  let aiModel = $state("chat");
  let aiSaved = $state(false);
  let aiError = $state<string | null>(null);
  let cbResetDone = $state(false);
  let moveToTrash = $state(true);
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

  // ─── CalDAV State ───────────────────────────
  let caldavUrl = $state("https://");
  let caldavUser = $state("");
  let caldavPass = $state("");
  let caldavInterval = $state(30);
  let caldavSaved = $state(false);
  let caldavError = $state<string | null>(null);
  let caldavSyncing = $state(false);
  let caldavSyncResult = $state<number | null>(null);
  let ownPhoto = $state<{ data: string; type: string } | null>(null);

  // ─── Voice ───────────────────────────────────
  let voiceEnabled = $state(false);
  let voiceSttUrl = $state("");
  let voiceSttKey = $state("");
  let voiceSttModel = $state("Systran/faster-whisper-small");
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

  async function restoreBackup(name: string) {
    if (!window.confirm(translate("settings.restoreConfirm", { name }))) return;
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
    } catch (e) { console.warn("Settings load failed, using defaults", e); }
    try {
      moveToTrash = await getMoveToTrash();
    } catch (e) { console.warn("move_to_trash load failed, using default", e); }
    await loadAccountList();

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

    // Load CalDAV settings
    try {
      const cs = await getCalDavSettings();
      if (cs) {
        caldavUrl = cs.url;
        caldavUser = cs.username;
        caldavPass = cs.password;
        caldavInterval = cs.sync_interval_minutes;
      }
    } catch (e) { console.warn("CalDAV settings load failed", e); }

    // Load own photo
    try {
      ownPhoto = await getOwnPhoto();
    } catch (e) { console.warn("Photo load failed", e); }

    // Load Voice settings
    try {
      const vs = await getVoiceSettings();
      if (vs) {
        voiceEnabled = vs.enabled;
        voiceSttUrl = vs.sttUrl;
        voiceSttKey = vs.sttKey;
        voiceSttModel = vs.sttModel;
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

  async function handleSaveAI() {
    aiError = null;
    try {
      await settings.save(aiUrl, aiKey, aiModel);
      aiSaved = true;
      setTimeout(() => (aiSaved = false), 2000);
    } catch (e: unknown) {
      aiError = e instanceof Error ? e.message : String(e);
    }
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
    // Validate before saving — garbage in, "saved" out is misleading.
    if (voiceEnabled) {
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
    try {
      await saveVoiceSettings(voiceEnabled, voiceSttUrl.trim(), voiceSttKey.trim(), voiceSttModel.trim());
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

  async function handleSaveCalDav() {
    caldavError = null;
    caldavSaved = false;
    try {
      await setCalDavSettings({
        url: caldavUrl,
        username: caldavUser,
        password: caldavPass,
        sync_interval_minutes: caldavInterval,
      });
      caldavSaved = true;
      setTimeout(() => (caldavSaved = false), 2000);
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

  // ─── Photo Upload ──────────────────────────
  async function handlePhotoUpload() {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/jpeg,image/png,image/webp,image/svg+xml';
    input.onchange = async (e: Event) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = async () => {
        const result = reader.result as string;
        const base64 = result.split(',')[1];
        try {
          await saveOwnPhoto(base64, file.type);
          ownPhoto = { data: base64, type: file.type };
        } catch (e) {
          console.error("Photo upload failed", e);
        }
      };
      reader.readAsDataURL(file);
    };
    input.click();
  }

  async function handleClearPhoto() {
    try {
      await saveOwnPhoto("", "");
      ownPhoto = null;
    } catch (e) {
      console.error("Photo clear failed", e);
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
        // Edit mode: update the EXISTING account (imap_insecure etc.) instead
        // of creating a duplicate.
        await updateAccountSettings(editingAccountId, undefined, undefined, imapInsecure);
        acctSuccess = translate("settings.accountUpdated", {
          name: acctName,
          cert: imapInsecure ? translate("settings.certInsecure") : translate("settings.certVerified"),
        });
      } else {
        await connectAccount(
          acctName, imapHost, imapPort, imapSsl,
          smtpHost, smtpPort, smtpTls,
          acctUser, acctPass, smtpUser, smtpPass, senderName, senderMail,
          imapInsecure,
        );
        acctSuccess = translate("settings.accountConnected", { name: acctName });
      }
      
      // Clear fields
      acctName = ""; imapHost = ""; smtpHost = ""; acctUser = "";
      acctPass = ""; smtpUser = ""; smtpPass = ""; senderName = ""; senderMail = "";
      imapPort = 993; imapSsl = true; imapInsecure = false;
      smtpPort = 587; smtpTls = true;
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

  function handleCancelEdit() {
    acctName = ""; imapHost = ""; smtpHost = ""; acctUser = "";
    acctPass = ""; smtpUser = ""; smtpPass = ""; senderName = ""; senderMail = "";
    imapPort = 993; imapSsl = true; imapInsecure = false;
    smtpPort = 587; smtpTls = true;
    isEditing = false;
    editingAccountId = null;
    acctError = null;
    acctSuccess = null;
  }

  // In-app confirmation (window.confirm is unreliable in the Tauri WKWebView).
  let showDeleteAccountConfirm = $state(false);
  let pendingDeleteAccountId = $state<number | null>(null);

  function handleDeleteAccount(id: number) {
    pendingDeleteAccountId = id;
    showDeleteAccountConfirm = true;
  }

  async function handleSyncModeChange(accountId: number, mode: string) {
    try {
      await updateAccountSettings(accountId, mode);
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

<div class="settings-page" class:narrow={isNarrow} class:mobile-content={isNarrow && mobileContentOpen}>
  <!-- 1. LEFT SIDEBAR (HubSpot-Style Navigation) -->
  <aside class="settings-sidebar">
    <div class="sidebar-header">
      <ModuleLogo to="/" label={$t("settings.title")} />
    </div>

    <nav class="sidebar-menu">
      <button type="button" class="menu-item" class:active={activeTab === 'general'} onclick={() => selectTab('general')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        </div>
        <span>{$t("settings.general")}</span>
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'accounts'} onclick={() => selectTab('accounts')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0l-7.5-4.615a2.25 2.25 0 01-1.07-1.916V6.75" />
          </svg>
        </div>
        <span>{$t("settings.accounts")}</span>
        {#if accountList.length > 0}
          <span class="badge-pill">{accountList.length}</span>
        {/if}
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'carddav'} onclick={() => selectTab('carddav')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 002.625.372 9.337 9.337 0 004.121-.952 4.125 4.125 0 00-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.29-.792-3.07M15 19.128v.106A12.318 12.318 0 018.5 21c-2.191 0-4.22-.558-6-1.54v-.036a4.125 4.125 0 017.533-2.493c.505.78.967 1.659.967 2.638M8.25 3.75a4.125 4.125 0 100 8.25 4.125 4.125 0 000-8.25zM12.971 6.304a4.125 4.125 0 010 6.392m8.404-1.446a2.25 2.25 0 010 3.5" />
          </svg>
        </div>
        <span>{$t("settings.contacts")}</span>
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'caldav'} onclick={() => selectTab('caldav')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6.75 3v2.25M17.25 3v2.25M3 18.75V7.5a2.25 2.25 0 012.25-2.25h13.5A2.25 2.25 0 0121 7.5v11.25m-18 0A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75m-18 0v-7.5A2.25 2.25 0 015.25 9h13.5A2.25 2.25 0 0121 11.25v7.5" />
          </svg>
        </div>
        <span>{$t("settings.calendar")}</span>
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'ai'} onclick={() => selectTab('ai')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904L9 21l-.813-5.096L3 15l5.187-.813L9 9l.813 5.187L15 15l-5.187.814zM18 10.5L17.25 15l-.75-4.5L12 10l4.5-.75.75-4.5.75 4.5L22 10l-4.5.75z" />
          </svg>
        </div>
        <span>{$t("settings.ai")}</span>
      </button>

    <button type="button" class="menu-item" class:active={activeTab === 'voice'} onclick={() => selectTab('voice')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5a6 6 0 01-6-6v-1.5m6 7.5a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
          </svg>
        </div>
        <span>{$t("settings.voice")}</span>
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'cache'} onclick={() => selectTab('cache')}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125" />
          </svg>
        </div>
        <span>{$t("settings.cache")}</span>
      </button>

      <button type="button" class="menu-item" class:active={activeTab === 'archive'} onclick={() => { selectTab('archive'); loadDeleteQueue(); loadBackups(); }}>
        <div class="menu-icon-wrapper">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z" />
          </svg>
        </div>
        <span>{$t("settings.archive")}</span>
      </button>
    </nav>

    <div class="settings-module-row">
      <ModuleIcons active="settings" />
    </div>
  </aside>

  <!-- 2. RIGHT MAIN CONTENT AREA -->
  <main class="settings-content-wrapper">
    {#if isNarrow}
      <div class="mobile-content-header">
        <button type="button" class="back-btn" onclick={() => mobileContentOpen = false} title={$t("common.back")}>
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2.5" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
          </svg>
          <span>{$t("settings.title")}</span>
        </button>
      </div>
    {/if}
    <div class="settings-content">

      <!-- ================= TAB: ALLGEMEIN ================= -->
      {#if activeTab === 'general'}
        <header class="tab-header">
          <h1>{$t("settings.generalTitle")}</h1>
          <p class="tab-desc">{$t("settings.generalDesc")}</p>
        </header>

        <!-- Card: Sprache -->
        <section class="settings-card">
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
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.appearance")}</h3>
            <p class="card-desc">{$t("settings.appearanceDesc")}</p>
          </div>
          
          <div class="theme-selection-grid">
            <button
              type="button"
              class="theme-card-option"
              class:active={theme === 'blue'}
              onclick={() => handleThemeChange('blue')}
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
              class:active={theme === 'dark'}
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
          </div>
        </section>

        <!-- Card: Postfach Synchronisation -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.mailboxBehavior")}</h3>
            <p class="card-desc">{$t("settings.mailboxBehaviorDesc")}</p>
          </div>

          <div class="card-body">
            <!-- Sync limit -->
            <div class="form-row align-items-center">
              <div class="form-group flex-2">
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
                    class="form-control"
                  />
                  <span class="input-badge">{$t("settings.mails")}</span>
                </div>
              </div>
              <div class="flex-3 py-2">
                <p class="hint-text mt-4">{$t("settings.fetchLimitHint")}</p>
              </div>
            </div>

            <div class="divider"></div>

            <!-- Custom Switch for Move to Trash -->
            <div class="switch-row">
              <label class="switch-container">
                <input type="checkbox" checked={moveToTrash} onchange={handleMoveToTrashToggle} />
                <span class="switch-slider"></span>
                <span class="switch-label-group">
                  <span class="switch-title">{$t("settings.moveToTrash")}</span>
                  <span class="switch-desc">{$t("settings.moveToTrashDesc")}</span>
                </span>
              </label>
            </div>

            <div class="divider"></div>

            <!-- Custom Switch for Auto Download Images -->
            <div class="switch-row">
              <label class="switch-container">
                <input type="checkbox" checked={autoDownloadImages} onchange={handleAutoDownloadImagesToggle} />
                <span class="switch-slider"></span>
                <span class="switch-label-group">
                  <span class="switch-title">{$t("settings.autoDownloadImages")}</span>
                  <span class="switch-desc">{$t("settings.autoDownloadImagesDesc")}</span>
                </span>
              </label>
            </div>

            <div class="divider"></div>

            <!-- Custom Switch for Push Notifications -->
            <div class="switch-row">
              <label class="switch-container">
                <input type="checkbox" checked={notificationsEnabled} onchange={handleNotificationsToggle} disabled={notificationsBusy} />
                <span class="switch-slider"></span>
                <span class="switch-label-group">
                  <span class="switch-title">{$t("settings.push")}</span>
                  <span class="switch-desc">{$t("settings.pushDesc")}</span>
                </span>
              </label>
              {#if notificationsError}
                <p class="text-xs text-red-500 mt-2">{notificationsError}</p>
              {/if}
            </div>
          </div>
        </section>
      {/if}

      <!-- ================= TAB: E-MAIL-KONTEN ================= -->
      {#if activeTab === 'accounts'}
        <header class="tab-header">
          <h1>{$t("settings.accounts")}</h1>
          <p class="tab-desc">{$t("settings.accountsDesc")}</p>
        </header>

        <!-- Liste verbundener Konten -->
        {#if accountList.length > 0}
          <section class="settings-card">
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
                        class="sync-mode-select"
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
                    <button type="button" class="btn-action-ghost" onclick={() => connectAndEditAccount(a)}>
                      {$t("settings.edit")}
                    </button>
                    <button type="button" class="btn-action-danger-ghost" onclick={() => handleDeleteAccount(a.id)}>
                      {$t("settings.remove")}
                    </button>
                  </div>
                </div>
              {/each}
            </div>
          </section>
        {/if}

        <!-- Formular zum Hinzufügen / Bearbeiten -->
        <section class="settings-card" id="account-form">
          <div class="card-header">
            <h3>{isEditing ? $t("settings.editAccountTitle") : $t("settings.newAccountTitle")}</h3>
            <p class="card-desc">{$t("settings.accountFormDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="form-group">
                <label for="acct-name">{$t("settings.accountName")}</label>
                <input id="acct-name" bind:value={acctName} placeholder={$t("settings.accountNamePlaceholder")} class="form-control" />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.imapSection")}</div>
            <div class="form-grid-3">
              <div class="form-group">
                <label for="imap-host">{$t("settings.serverAddress")}</label>
                <input id="imap-host" bind:value={imapHost} placeholder="imap.provider.com" class="form-control" />
              </div>
              <div class="form-group">
                <label for="imap-port">{$t("settings.port")}</label>
                <input id="imap-port" type="number" bind:value={imapPort} class="form-control" />
              </div>
              <div class="form-group justify-self-center">
                <label class="toggle-label">
                  <input type="checkbox" class="toggle" bind:checked={imapSsl} />
                  <span class="toggle-track" aria-hidden="true"></span>
                  <span class="toggle-text">SSL</span>
                </label>
              </div>
              <div class="form-group justify-self-center">
                <label class="toggle-label" title={$t("settings.insecureTitle")}>
                  <input type="checkbox" class="toggle" bind:checked={imapInsecure} />
                  <span class="toggle-track" aria-hidden="true"></span>
                  <span class="toggle-text">{$t("settings.insecureAllow")}</span>
                </label>
              </div>
            </div>

            <div class="form-section-title">{$t("settings.smtpSection")}</div>
            <div class="form-grid-3">
              <div class="form-group">
                <label for="smtp-host">{$t("settings.serverAddress")}</label>
                <input id="smtp-host" bind:value={smtpHost} placeholder="smtp.provider.com" class="form-control" />
              </div>
              <div class="form-group">
                <label for="smtp-port">{$t("settings.port")}</label>
                <input id="smtp-port" type="number" bind:value={smtpPort} class="form-control" />
              </div>
              <div class="form-group justify-self-center">
                <label class="toggle-label">
                  <input type="checkbox" class="toggle" bind:checked={smtpTls} />
                  <span class="toggle-track" aria-hidden="true"></span>
                  <span class="toggle-text">TLS</span>
                </label>
              </div>
            </div>

            <div class="form-section-title">{$t("settings.imapCredentials")}</div>
            <div class="form-grid-2">
              <div class="form-group">
                <label for="acct-user">{$t("settings.username")}</label>
                <input id="acct-user" bind:value={acctUser} placeholder="name@provider.com" class="form-control" />
              </div>
              <div class="form-group">
                <label for="acct-pass">{$t("settings.password")}</label>
                <input id="acct-pass" type="password" bind:value={acctPass} placeholder="••••••••••••••••" class="form-control" />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.smtpCredentialsOptional")}</div>
            <div class="form-grid-2">
              <div class="form-group">
                <label for="smtp-user">{$t("settings.smtpUsername")}</label>
                <input id="smtp-user" bind:value={smtpUser} placeholder={$t("settings.optionalImapUser")} class="form-control" />
              </div>
              <div class="form-group">
                <label for="smtp-pass">{$t("settings.smtpPassword")}</label>
                <input id="smtp-pass" type="password" bind:value={smtpPass} placeholder={$t("settings.optionalImapPassword")} class="form-control" />
              </div>
            </div>

            <div class="form-section-title">{$t("settings.sender")}</div>
            <div class="form-grid-2 mt-2">
              <div class="form-group">
                <label for="sender-name">{$t("settings.senderName")}</label>
                <input id="sender-name" bind:value={senderName} placeholder="Max Mustermann" class="form-control" />
              </div>
              <div class="form-group">
                <label for="sender-mail">{$t("settings.senderMail")}</label>
                <input id="sender-mail" type="text" inputmode="email" bind:value={senderMail} placeholder="name@provider.com" class="form-control" />
              </div>
            </div>

            {#if acctError}
              <div class="alert-box error">
                <div class="alert-icon">⚠️</div>
                <div class="alert-text">{acctError}</div>
              </div>
            {/if}
            
            {#if acctSuccess}
              <div class="alert-box success">
                <div class="alert-icon">✓</div>
                <div class="alert-text">{acctSuccess}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              {#if isEditing}
                <button type="button" class="btn-cancel" onclick={handleCancelEdit}>
                  {$t("common.cancel")}
                </button>
              {/if}
              <button type="button" class="btn-submit" onclick={handleConnectAccount} disabled={acctConnecting}>
                {acctConnecting ? $t("settings.testing") : (isEditing ? $t("settings.saveChanges") : $t("settings.connectAccount"))}
              </button>
            </div>
          </div>
        </section>
      {/if}

      <!-- ================= TAB: KI & TEXT ================= -->
      {#if activeTab === 'ai'}
        <header class="tab-header">
          <h1>{$t("settings.aiTitle")}</h1>
          <p class="tab-desc">{$t("settings.aiDesc")}</p>
        </header>

        <!-- Card: Textgenerierungs-Optionen -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.assistantBehavior")}</h3>
            <p class="card-desc">{$t("settings.assistantBehaviorDesc")}</p>
          </div>

          <div class="card-body">
            <div class="switch-row">
              <label class="switch-container">
                <input type="checkbox" bind:checked={$showDiffEnabled} />
                <span class="switch-slider"></span>
                <span class="switch-label-group">
                  <span class="switch-title">{$t("settings.diffEditor")}</span>
                  <span class="switch-desc">{$t("settings.diffEditorDesc")}</span>
                </span>
              </label>
            </div>
          </div>
        </section>

        <!-- Card: API Endpunkt -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiEndpoint")}</h3>
            <p class="card-desc">{$t("settings.aiEndpointDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="form-group">
                <label for="ai-url">{$t("settings.apiUrl")}</label>
                <input id="ai-url" type="url" bind:value={aiUrl} placeholder="https://llm.aimighty.de/v1" class="form-control" />
              </div>
            </div>

            <div class="form-grid-2">
              <div class="form-group">
                <label for="ai-key">{$t("settings.apiKey")}</label>
                <input id="ai-key" type="password" bind:value={aiKey} placeholder="ollama" class="form-control" />
              </div>
              <div class="form-group">
                <label for="ai-model">{$t("settings.modelId")}</label>
                <input id="ai-model" type="text" bind:value={aiModel} placeholder="llama3.2" class="form-control" />
              </div>
            </div>

            {#if aiError}
              <div class="alert-box error">
                <div class="alert-icon">⚠️</div>
                <div class="alert-text">{aiError}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn-submit" onclick={handleSaveAI}>
                {aiSaved ? $t("settings.saved") : $t("settings.saveConnection")}
              </button>
            </div>
          </div>
        </section>

        <!-- Card: KI-System-Status -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiStatus")}</h3>
            <p class="card-desc">{$t("settings.aiStatusDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-actions-row">
              <button type="button" class="btn-submit" onclick={handleResetCircuitBreaker}>
                {cbResetDone ? $t("settings.aiResetDone") : $t("settings.aiReset")}
              </button>
            </div>
          </div>
        </section>
      {/if}

       <!-- ================= TAB: CARDDAV ================= -->
      {#if activeTab === 'carddav'}
        <header class="tab-header">
          <h1>{$t("settings.contactsTitle")}</h1>
          <p class="tab-desc">{$t("settings.contactsDesc")}</p>
        </header>

        <!-- Card: CardDAV Settings -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.carddav")}</h3>
            <p class="card-desc">{$t("settings.carddavDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="form-group">
                <label for="carddav-url">{$t("settings.serverUrl")}</label>
                <input id="carddav-url" type="url" bind:value={carddavUrl} placeholder="https://nextcloud.example.com/remote.php/dav/addressbooks/users/username/contacts/" class="form-control" />
              </div>
            </div>

            <div class="form-grid-2">
              <div class="form-group">
                <label for="carddav-user">{$t("settings.usernameShort")}</label>
                <input id="carddav-user" type="text" bind:value={carddavUser} placeholder={$t("settings.usernameShort")} class="form-control" />
              </div>
              <div class="form-group">
                <label for="carddav-pass">{$t("settings.passwordToken")}</label>
                <input id="carddav-pass" type="password" bind:value={carddavPass} placeholder={$t("settings.passwordToken")} class="form-control" />
              </div>
            </div>

            <div class="form-grid-1">
              <div class="form-group">
                <label for="carddav-interval">{$t("settings.syncInterval")}</label>
                <div class="input-with-badge">
                  <input id="carddav-interval" type="number" bind:value={carddavInterval} min="1" max="1440" class="form-control" />
                  <span class="input-badge">{$t("settings.minutes")}</span>
                </div>
              </div>
            </div>

            {#if carddavError}
              <div class="alert-box error">
                <div class="alert-icon">⚠️</div>
                <div class="alert-text">{carddavError}</div>
              </div>
            {/if}
            
            {#if carddavSaved}
              <div class="alert-box success">
                <div class="alert-icon">✓</div>
                <div class="alert-text">{$t("settings.carddavSaved")}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn-cancel" onclick={handleSyncCardDav} disabled={carddavSyncing}>
                {carddavSyncing ? $t("settings.syncing") : $t("settings.syncNow")}
              </button>
              <button type="button" class="btn-submit" onclick={handleSaveCardDav}>
                {$t("common.save")}
              </button>
            </div>

            {#if carddavSyncResult !== null}
              <div class="sync-success-pill">
                <span class="sync-icon">🔄</span>
                <span>{$t("settings.syncSuccess", { count: carddavSyncResult })}</span>
              </div>
            {/if}
          </div>
        </section>

        <!-- Card: Profile Photo -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.profilePhoto")}</h3>
            <p class="card-desc">{$t("settings.profilePhotoDesc")}</p>
          </div>

          <div class="card-body">
            <div class="photo-upload-row">
              <div class="photo-preview" class:has-photo={ownPhoto}>
                {#if ownPhoto}
                  <img src="data:{ownPhoto.type};base64,{ownPhoto.data}" alt={$t("settings.profilePhoto")} />
                {:else}
                  <div class="photo-placeholder">+</div>
                {/if}
              </div>
              <div class="photo-actions">
                <button type="button" class="btn-cancel" onclick={handlePhotoUpload}>
                  {$t("settings.uploadImage")}
                </button>
                {#if ownPhoto}
                  <button type="button" class="btn-cancel" onclick={handleClearPhoto}>
                    {$t("settings.remove")}
                  </button>
                {/if}
              </div>
            </div>
          </div>
        </section>
      {/if}

      <!-- ================= TAB: CALDAV ================= -->
      {#if activeTab === 'caldav'}
        <header class="tab-header">
          <h1>{$t("settings.calendarTitle")}</h1>
          <p class="tab-desc">{$t("settings.calendarDesc")}</p>
        </header>

        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.caldav")}</h3>
            <p class="card-desc">{$t("settings.caldavDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="form-group">
                <label for="caldav-url">{$t("settings.serverUrl")}</label>
                <input id="caldav-url" type="url" bind:value={caldavUrl} placeholder="https://nextcloud.example.com/remote.php/dav/calendars/username/" class="form-control" />
              </div>
            </div>

            <div class="form-grid-2">
              <div class="form-group">
                <label for="caldav-user">{$t("settings.usernameShort")}</label>
                <input id="caldav-user" type="text" bind:value={caldavUser} placeholder={$t("settings.usernameShort")} class="form-control" />
              </div>
              <div class="form-group">
                <label for="caldav-pass">{$t("settings.passwordToken")}</label>
                <input id="caldav-pass" type="password" bind:value={caldavPass} placeholder={$t("settings.passwordToken")} class="form-control" />
              </div>
            </div>

            <div class="form-grid-1">
              <div class="form-group">
                <label for="caldav-interval">{$t("settings.syncInterval")}</label>
                <div class="input-with-badge">
                  <input id="caldav-interval" type="number" bind:value={caldavInterval} min="1" max="1440" class="form-control" />
                  <span class="input-badge">{$t("settings.minutes")}</span>
                </div>
              </div>
            </div>

            {#if caldavError}
              <div class="alert-box error">
                <div class="alert-icon">⚠️</div>
                <div class="alert-text">{caldavError}</div>
              </div>
            {/if}

            {#if caldavSaved}
              <div class="alert-box success">
                <div class="alert-icon">✓</div>
                <div class="alert-text">{$t("settings.caldavSaved")}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn-cancel" onclick={handleSyncCalDav} disabled={caldavSyncing}>
                {caldavSyncing ? $t("settings.syncing") : $t("settings.syncNow")}
              </button>
              <button type="button" class="btn-submit" onclick={handleSaveCalDav}>
                {$t("common.save")}
              </button>
            </div>

            {#if caldavSyncResult !== null}
              <div class="sync-success-pill">
                <span class="sync-icon">🔄</span>
                <span>{$t("settings.syncSuccessCal", { count: caldavSyncResult })}</span>
              </div>
            {/if}
          </div>
        </section>
      {/if}

      <!-- ================= TAB: VOICE ================= -->
      {#if activeTab === 'voice'}
        <header class="tab-header">
          <h1>{$t("settings.voiceTitle")}</h1>
          <p class="tab-desc">{$t("settings.voiceDesc")}</p>
        </header>
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.voice2mail")}</h3>
            <p class="card-desc">{$t("settings.voice2mailDesc")}</p>
          </div>

          <div class="card-body">
            <div class="switch-row">
              <label class="switch-container">
                <input type="checkbox" bind:checked={voiceEnabled} />
                <span class="switch-slider"></span>
                <span class="switch-label-group">
                  <span class="switch-title">{$t("settings.voiceEnable")}</span>
                  <span class="switch-desc">{$t("settings.voiceEnableDesc")}</span>
                </span>
              </label>
            </div>
          </div>
        </section>

        <!-- Card: STT Endpoint -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.sttEndpoint")}</h3>
            <p class="card-desc">{$t("settings.sttEndpointDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-1">
              <div class="form-group">
                <label for="voice-stt-url">{$t("settings.apiUrl")}</label>
                <input id="voice-stt-url" type="url" bind:value={voiceSttUrl} placeholder="https://speaches.aimighty.de/v1" class="form-control" disabled={!voiceEnabled} />
              </div>
            </div>

            <div class="form-grid-2">
              <div class="form-group">
                <label for="voice-stt-key">{$t("settings.apiKey")}</label>
                <input id="voice-stt-key" type="password" bind:value={voiceSttKey} placeholder={$t("settings.optional")} class="form-control" disabled={!voiceEnabled} />
              </div>
              <div class="form-group">
                <label for="voice-stt-model">{$t("settings.modelId")}</label>
                <input id="voice-stt-model" type="text" bind:value={voiceSttModel} placeholder="Systran/faster-whisper-small" class="form-control" disabled={!voiceEnabled} />
              </div>
            </div>

            {#if voiceError}
              <div class="alert-box error">
                <div class="alert-icon">⚠️</div>
                <div class="alert-text">{voiceError}</div>
              </div>
            {/if}

            <div class="form-actions-row">
                <button type="button" class="btn-submit" onclick={handleSaveVoice} disabled={!voiceEnabled}>
                  {voiceSaved ? $t("settings.saved") : $t("settings.saveConnection")}
                </button>
              </div>
          </div>
        </section>
      {/if}

      {#if activeTab === 'archive'}
        <header class="tab-header">
          <h1>{$t("settings.archiveTitle")}</h1>
          <p class="tab-desc">{$t("settings.archiveDesc")}</p>
        </header>

        <!-- Card: Delete Queue Review -->
        <section class="settings-card">
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
                    <button type="button" class="btn-action-ghost" onclick={() => retryDeleteQueue(row.id)}>{$t("settings.retry")}</button>
                    <button type="button" class="btn-action-danger-ghost" onclick={() => removeDeleteQueue(row.id)}>{$t("settings.discard")}</button>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>

        <!-- Card: Export -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.exportTitle")}</h3>
            <p class="card-desc">{$t("settings.exportDesc")}</p>
          </div>
          <div class="export-row">
            {#each accountList as a (a.id)}
              <div class="export-account">
                <span class="export-account-name">{a.name}</span>
                <button type="button" class="btn-action-ghost" onclick={() => downloadExport(a.id, "mbox")}>MBox</button>
                <button type="button" class="btn-action-ghost" onclick={() => downloadExport(a.id, "zip")}>EML-ZIP</button>
              </div>
            {/each}
            {#if accountList.length === 0}
              <p class="hint-text">{$t("settings.noAccountsExport")}</p>
            {/if}
          </div>
        </section>

        <!-- Card: Backup -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.backupTitle")}</h3>
            <p class="card-desc">{$t("settings.backupDesc")}</p>
          </div>
          <div class="export-row">
            <button type="button" class="btn-action" onclick={handleBackup} disabled={backupBusy}>
              {backupBusy ? $t("settings.createBackupBusy") : $t("settings.createBackup")}
            </button>
            {#if backupResult}
              <p class="hint-text mt-2">{$t("settings.backupCreated", { path: backupResult.path, size: formatBytes(backupResult.size) })}</p>
            {/if}
          </div>

          <div class="card-header" style="margin-top:16px">
            <h4>{$t("settings.existingBackups")}</h4>
          </div>
          <div class="backup-list">
            {#each backups as b (b.name)}
              <div class="backup-row">
                <span class="backup-name">{b.name}</span>
                <span class="backup-size">{formatBytes(b.size)}</span>
                <button type="button" class="btn-action-danger-ghost" onclick={() => restoreBackup(b.name)}>{$t("settings.restore")}</button>
              </div>
            {:else}
              <p class="hint-text">{$t("settings.noBackups")}</p>
            {/each}
          </div>
          {#if restoreResult}
            <p class="hint-text mt-2">{restoreResult}</p>
          {/if}
        </section>
      {/if}

      {#if activeTab === 'cache'}
        <header class="tab-header">
          <h1>{$t("settings.cacheTitle")}</h1>
          <p class="tab-desc">{$t("settings.cacheDesc")}</p>
        </header>

        <!-- Card: Cache Statistics -->
        <section class="settings-card">
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
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.cacheCleanup")}</h3>
            <p class="card-desc">{$t("settings.cacheCleanupDesc")}</p>
          </div>

          <div class="card-body">
            <div class="form-grid-2">
              <div class="form-group">
                <label for="cache-max-mb">{$t("settings.cacheMaxMb")}</label>
                <input id="cache-max-mb" type="number" bind:value={cacheMaxMb} min="10" max="500" class="form-control" />
              </div>
            </div>

            {#if cacheCleanupResult !== null}
              <div class="alert-box success">
                <div class="alert-icon">✓</div>
                <div class="alert-text">{$t("settings.cacheCleaned", { count: cacheCleanupResult })}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn-submit" onclick={handleCleanupCache} disabled={cacheCleaning}>
                {cacheCleaning ? $t("settings.cleaning") : $t("settings.cacheCleanup")}
              </button>
              <button type="button" class="btn-danger" onclick={handleClearCache} disabled={cacheCleaning}>
                {cacheCleaning ? $t("settings.clearing") : $t("settings.clearAll")}
              </button>
            </div>
          </div>
        </section>

        <!-- Card: KI-Zusammenfassungen -->
        <section class="settings-card">
          <div class="card-header">
            <h3>{$t("settings.aiSummaries")}</h3>
            <p class="card-desc">{$t("settings.aiSummariesDesc")}</p>
          </div>

          <div class="card-body">
            {#if aiSummariesResult !== null}
              <div class="alert-box success">
                <div class="alert-icon">✓</div>
                <div class="alert-text">{$t("settings.aiSummariesCleared", { count: aiSummariesResult })}</div>
              </div>
            {/if}

            <div class="form-actions-row">
              <button type="button" class="btn-danger" onclick={handleClearAiSummaries} disabled={aiSummariesClearing}>
                {aiSummariesClearing ? $t("settings.clearing") : $t("settings.aiSummariesClearAll")}
              </button>
            </div>
          </div>
        </section>
      {/if}

    </div>
  </main>
</div>

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

  <AssistantFab module="settings" />

<style>
  /* ─── BASE LAYOUT (HubSpot Split-Screen) ─── */
  .settings-page {
    display: flex;
    height: 100vh;
    background: var(--color-preview);
    color: var(--color-text);
    overflow: hidden;
  }

  /* ─── SIDEBAR ─── */
  .settings-sidebar {
    width: 280px;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    padding: 28px 20px;
    display: flex;
    flex-direction: column;
    gap: 24px;
    flex-shrink: 0;
  }

  .sidebar-header {
    height: 72px;
    padding: 0 15px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .back-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    background: transparent;
    border: none;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    padding: 6px 12px 6px 4px;
    border-radius: 6px;
    transition: all 0.15s ease;
    width: fit-content;
  }

  .back-btn:hover {
    color: var(--color-text);
    background: var(--color-active-wash);
  }

  .back-btn svg {
    width: 14px;
    height: 14px;
  }

  .sidebar-header h2 {
    font-size: 1.25rem;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--color-text);
    margin: 0;
    padding-left: 4px;
  }

  .sidebar-menu {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .settings-module-row {
    margin-top: auto;
    display: flex;
    justify-content: center;
    padding-top: 12px;
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    background: transparent;
    border: none;
    border-radius: 8px;
    color: var(--color-text-secondary);
    font-size: 0.875rem;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
    transition: all 0.15s ease;
    width: 100%;
    font-family: inherit;
  }

  .menu-item:hover {
    color: var(--color-text);
    background: var(--color-active-wash);
  }

  .menu-item.active {
    color: var(--color-accent);
    background: var(--color-active-wash);
    font-weight: 600;
  }

  .menu-icon-wrapper {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
  }

  .menu-icon-wrapper svg {
    width: 18px;
    height: 18px;
  }

  .badge-pill {
    margin-left: auto;
    background: var(--color-accent);
    color: #fff;
    font-size: 0.75rem;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 20px;
  }

  /* ─── CONTENT WRAPPER ─── */
  .settings-content-wrapper {
    flex: 1;
    overflow-y: auto;
    height: 100%;
    background: var(--color-preview);
  }

  .settings-content {
    max-width: 800px;
    margin: 0 auto;
    padding: 48px 40px 80px 40px;
  }

  /* ─── MOBILE (≤600px): iOS-style drill-down ─────────────────
     Menu list fills the screen; selecting an item pushes the content
     view (with a back button). Sidebar and content never show at once. */
  .mobile-content-header {
    display: none;
  }

  @media (max-width: 600px) {
    .settings-sidebar {
      width: 100%;
      border-right: none;
      padding: 16px;
      padding-top: max(16px, env(safe-area-inset-top, 0px));
      padding-bottom: max(16px, env(safe-area-inset-bottom, 0px));
      overflow-y: auto;
    }
    .settings-page.mobile-content .settings-sidebar {
      display: none;
    }
    .settings-content-wrapper {
      display: none;
    }
    .settings-page.mobile-content .settings-content-wrapper {
      display: block;
    }
    .mobile-content-header {
      display: block;
      padding: 8px 12px;
      padding-top: max(8px, env(safe-area-inset-top, 0px));
      border-bottom: 1px solid var(--color-border);
      background: var(--color-list);
      position: sticky;
      top: 0;
      z-index: 10;
    }
    .mobile-content-header .back-btn {
      min-height: 44px;
      font-size: 0.9375rem;
    }
    .settings-content {
      max-width: none;
      padding: 20px 16px 64px;
    }
    .tab-header {
      margin-bottom: 20px;
    }
    .tab-header h1 {
      font-size: 1.375rem;
    }
    .settings-card {
      padding: 16px;
      border-radius: 10px;
    }
    .menu-item {
      min-height: 48px;
      padding: 12px;
      font-size: 1rem;
    }
    .sidebar-header h2 {
      font-size: 1.5rem;
    }
  }

  .tab-header {
    margin-bottom: 28px;
  }

  .tab-header h1 {
    font-size: 1.75rem;
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--color-text);
    margin: 0 0 6px 0;
  }

  .tab-desc {
    font-size: 0.875rem;
    color: var(--color-text-secondary);
    margin: 0;
  }

  /* ─── CARDS ─── */
  .settings-card {
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 20px;
    box-shadow: none;
  }

  .card-header {
    margin-bottom: 20px;
  }

  .card-header h3 {
    font-size: 1rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--color-text);
    margin: 0 0 4px 0;
  }

  .card-desc {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    margin: 0;
    line-height: 1.5;
  }

  .card-body {
    display: flex;
    flex-direction: column;
  }

  /* ─── THEME GRID SELECTION ─── */
  .theme-selection-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  .theme-card-option {
    background: transparent;
    border: 1.5px solid var(--color-border);
    border-radius: 10px;
    padding: 12px;
    cursor: pointer;
    text-align: left;
    transition: all 0.2s ease;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .theme-card-option:hover {
    border-color: var(--color-text-secondary);
  }

  .theme-card-option.active {
    border-color: var(--color-accent);
    background: var(--color-active-wash);
    box-shadow: 0 0 0 1px var(--color-accent);
  }

  .theme-card-option.dark-option {
    background: var(--b-150);
    border-color: var(--b-300);
  }

  .theme-card-option.dark-option .theme-label {
    color: var(--b-800);
  }

  .theme-card-option.dark-option:hover {
    border-color: var(--b-400);
  }

  .theme-card-option.dark-option.active {
    background: var(--b-200);
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }

  .theme-preview {
    height: 90px;
    border-radius: 6px;
    padding: 8px;
    display: flex;
    align-items: stretch;
    overflow: hidden;
    border: 1px solid var(--color-border);
  }

  .theme-preview.light {
    background: #FFFFFF;
  }

  .theme-preview.dark {
    background: var(--b-150);
    border-color: var(--b-300);
  }

  .theme-preview.dark .theme-window-mock {
    border-color: var(--b-300);
  }

  .theme-window-mock {
    flex: 1;
    display: flex;
    border-radius: 4px;
    overflow: hidden;
    border: 1px solid var(--color-border);
    box-shadow: none;
  }

  .mock-sidebar {
    width: 25%;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
  }

  .light .mock-sidebar { background: var(--b-900); border-right: 1px solid var(--b-800); }
  .dark .mock-sidebar { background: var(--b-100); border-right: 1px solid var(--b-300); }

  .mock-content {
    flex: 1;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--color-list);
  }

  .light .mock-content { background: #FFFFFF; }
  .dark .mock-content { background: var(--b-150); }

  .mock-line {
    height: 4px;
    border-radius: 2px;
    background: var(--color-border);
  }

  .light .mock-line { background: var(--b-800); }
  .dark .mock-line { background: var(--b-300); }

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

  .light-dot { background: var(--color-accent); }
  .dark-dot { background: #caa960; }

  .theme-label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-text);
  }

  /* ─── LANGUAGE TOGGLE ─── */
  .lang-toggle {
    display: inline-flex;
    gap: 4px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 2px;
    background: var(--color-sidebar);
    width: fit-content;
  }
  .lang-toggle button {
    background: transparent;
    border: none;
    color: var(--color-text-secondary);
    font-size: 0.8125rem;
    font-weight: 600;
    padding: 6px 14px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .lang-toggle button.active {
    background: var(--color-accent);
    color: #ffffff;
  }
  .lang-toggle button:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }

  /* ─── SWITCH CONTROL (iOS / HubSpot Style) ─── */
  .switch-row {
    padding: 6px 0;
  }

  .switch-container {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    cursor: pointer;
    user-select: none;
    width: 100%;
  }

  .switch-container input {
    display: none;
  }

  .switch-slider {
    position: relative;
    display: inline-block;
    width: 44px;
    height: 24px;
    background-color: var(--color-border);
    border-radius: 24px;
    transition: background-color 0.2s ease;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .switch-slider::before {
    position: absolute;
    content: "";
    height: 18px;
    width: 18px;
    left: 3px;
    bottom: 3px;
    background-color: #FFFFFF;
    border-radius: 50%;
    transition: transform 0.2s ease;
    box-shadow: none;
  }

  input:checked + .switch-slider {
    background-color: var(--color-accent);
  }

  input:checked + .switch-slider::before {
    transform: translateX(20px);
  }

  .switch-label-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .switch-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-text);
  }

  .switch-desc {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    line-height: 1.5;
  }

  /* ─── FORM ELEMENTS ─── */
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

  .form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-group label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-text-secondary);
  }

  .form-control {
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    font-size: 0.875rem;
    background: var(--color-list);
    color: var(--color-text);
    box-sizing: border-box;
    transition: all 0.15s ease;
  }

  .form-control::placeholder {
    color: var(--color-text-secondary);
    opacity: 0.5;
  }

  .form-control:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 12%, transparent);
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
  .justify-self-center { justify-self: center; align-self: center; padding-bottom: 12px; }

  .divider {
    height: 1px;
    background: var(--color-border);
    margin: 16px 0;
  }

  .form-section-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin: 24px 0 12px 0;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--color-border);
  }

  .input-with-badge {
    position: relative;
    display: flex;
    align-items: center;
  }

  .input-with-badge .form-control {
    padding-right: 64px;
  }

  .input-badge {
    position: absolute;
    right: 12px;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    text-transform: uppercase;
    background: var(--color-sidebar);
    padding: 4px 8px;
    border-radius: 4px;
    pointer-events: none;
    border: 1px solid var(--color-border);
  }

  .hint-text {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    line-height: 1.5;
    margin: 0;
  }

  /* Toggle Switch (SSL/TLS) */
  .toggle-label {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    user-select: none;
    padding: 4px 8px;
  }
  .toggle-label .toggle {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .toggle-label .toggle-track {
    position: relative;
    width: 34px;
    height: 20px;
    border-radius: 999px;
    background: var(--color-border);
    transition: background 0.2s ease;
    flex-shrink: 0;
    display: inline-block;
  }
  .toggle-label .toggle-track::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: none;
    transition: transform 0.2s ease;
  }
  .toggle-label .toggle:checked + .toggle-track {
    background: var(--color-accent);
  }
  .toggle-label .toggle:checked + .toggle-track::after {
    transform: translateX(14px);
  }
  .toggle-label .toggle:focus-visible + .toggle-track {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 20%, transparent);
  }
  .toggle-text {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-text);
    line-height: 1;
  }

  /* ─── ALERTS / ALERTMESSAGES ─── */
  .alert-box {
    display: flex;
    gap: 12px;
    padding: 12px 16px;
    border-radius: 8px;
    margin: 16px 0;
    font-size: 0.8125rem;
    line-height: 1.4;
  }

  .alert-box.error {
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-danger) 30%, transparent);
    color: var(--color-danger);
  }

  .alert-box.success {
    background: color-mix(in srgb, var(--color-success) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-success) 30%, transparent);
    color: var(--color-success);
  }

  .alert-icon {
    font-size: 1rem;
    font-weight: 700;
  }

  .alert-text {
    font-weight: 500;
  }

  /* ─── Cache Stats ─── */
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
    color: var(--color-text-secondary);
  }

  .stat-value {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--color-text);
  }

  /* ─── BUTTONS ─── */
  .form-actions-row {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
    margin-top: 20px;
  }

  .btn-submit {
    padding: 10px 24px;
    background: var(--color-accent);
    color: #FFFFFF;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-submit:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-submit:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-cancel {
    padding: 10px 20px;
    background: transparent;
    border: 1.5px solid var(--color-border);
    color: var(--color-text-secondary);
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-cancel:hover {
    background: var(--color-sidebar);
    color: var(--color-text);
    border-color: var(--color-text-secondary);
  }

  /* Primary action button (Backup erstellen) — same visual language as
     .btn-submit so the Archiv tab matches the other tabs. */
  .btn-action {
    padding: 10px 24px;
    background: var(--color-accent);
    color: #FFFFFF;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-action:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Destructive action button (Cache komplett leeren / KI-Zusammenfassungen
     löschen) — theme-aware danger variant of .btn-submit. */
  .btn-danger {
    padding: 10px 24px;
    background: var(--color-danger);
    color: #FFFFFF;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-danger) 85%, #000000);
  }

  .btn-danger:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ─── ACCOUNT CARDS ─── */
  .account-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .account-card-item {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 16px;
    background: var(--color-sidebar);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    transition: border-color 0.15s ease;
  }

  .account-card-item:hover {
    border-color: var(--color-text-secondary);
  }

  .account-avatar {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: var(--color-accent);
    color: #FFFFFF;
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
    color: var(--color-text);
  }

  .status-indicator-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 20px;
    font-size: 0.6875rem;
    font-weight: 600;
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
    color: var(--color-danger);
    border: 1px solid color-mix(in srgb, var(--color-danger) 20%, transparent);
  }

  .status-indicator-badge.connected {
    background: color-mix(in srgb, var(--color-success) 8%, transparent);
    color: var(--color-success);
    border: 1px solid color-mix(in srgb, var(--color-success) 20%, transparent);
  }

  .indicator-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-danger);
  }

  .connected .indicator-dot {
    background: var(--color-success);
  }

  .account-sub-info {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    margin: 0;
  }

  .account-tech-info {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    margin: 2px 0 0 0;
    display: flex;
    gap: 6px;
    align-items: center;
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
    border: 1px solid var(--color-border);
    border-radius: 8px;
    font-size: 0.8rem;
  }

  .backup-name {
    font-weight: 600;
    flex: 1;
  }

  .backup-size {
    color: var(--color-text-secondary);
  }

  .export-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .export-account {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 4px;
  }

  .export-account-name {
    font-size: 0.85rem;
    font-weight: 600;
    min-width: 180px;
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
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: var(--color-list);
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
    color: var(--color-text-secondary);
  }

  .delete-queue-state {
    font-size: 0.7rem;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--color-active-wash);
  }

  .delete-queue-state.failed {
    background: color-mix(in srgb, var(--color-danger) 12%, transparent);
    color: var(--color-danger);
  }

  .delete-queue-error {
    font-size: 0.72rem;
    color: var(--color-danger);
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .delete-queue-actions {
    display: flex;
    gap: 6px;
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
    color: var(--color-text-secondary);
  }

  .sync-mode-select {
    font-size: 0.78rem;
    padding: 4px 8px;
    border-radius: 8px;
    border: 1px solid var(--color-border, rgba(127,127,127,0.35));
    background: var(--color-surface, #fff);
    color: var(--color-text);
    cursor: pointer;
  }

  .sync-mode-hint {
    font-size: 0.7rem;
    color: var(--color-text-tertiary, var(--color-text-secondary));
  }

  .bullet-separator {
    color: var(--color-border);
  }

  .account-actions {
    display: flex;
    gap: 8px;
  }

  .btn-action-ghost {
    background: transparent;
    border: 1.5px solid var(--color-border);
    color: var(--color-text);
    padding: 10px 20px;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-action-ghost:hover {
    background: var(--color-list);
    border-color: var(--color-accent);
  }

  .btn-action-danger-ghost {
    background: transparent;
    border: 1.5px solid var(--color-border);
    color: var(--color-danger);
    padding: 10px 20px;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-action-danger-ghost:hover {
    background: color-mix(in srgb, var(--color-danger) 6%, transparent);
    border-color: var(--color-danger);
  }

  /* ─── CARDDAV SPECIFIC ─── */
  .sync-success-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: var(--color-active-wash);
    color: var(--color-accent);
    border: 1px solid var(--color-border);
    border-radius: 20px;
    font-size: 0.8125rem;
    margin-top: 16px;
  }

  .sync-icon {
    font-size: 1rem;
  }

  /* ─── PHOTO UPLOAD ─── */
  .photo-upload-row {
    display: flex;
    align-items: center;
    gap: 20px;
  }

  .photo-preview {
    width: 72px;
    height: 72px;
    border-radius: 50%;
    overflow: hidden;
    background: var(--color-active-wash);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .photo-preview img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .photo-placeholder {
    font-size: 1.5rem;
    font-weight: 300;
    color: var(--color-text-secondary);
  }

  .photo-actions {
    display: flex;
    flex-direction: row;
    gap: 8px;
  }
</style>