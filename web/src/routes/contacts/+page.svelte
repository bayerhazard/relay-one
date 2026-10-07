<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listContacts, createContact, updateContact, deleteContact, syncCardDav,
    type ContactInfo, type ContactInput,
  } from "$lib/services/tauri";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import SidebarFooter from "$lib/components/SidebarFooter.svelte";
  import SidebarSearch from "$lib/components/SidebarSearch.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { assistantAction } from "$lib/stores/assistantAction";
  import { useSidebarResize } from "$lib/composables/useSidebarResize";
  import { t, translate } from "$lib/i18n";
  import { fabHidden } from "$lib/stores/fabHidden";
  import { dataVersion } from "$lib/stores/invalidation";

  const { width: sidebarWidth, startResize, destroy: destroyResize } = useSidebarResize();
  $effect(() => () => destroyResize());

  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isNarrow = $derived(viewportWidth <= 768);
  let sidebarOpen = $state(false);
  $effect(() => { fabHidden.set(isNarrow && sidebarOpen); });
  $effect(() => {
    const onResize = () => (viewportWidth = window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  let contacts = $state<ContactInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let search = $state("");
  let busy = $state(false);

  // Editor state
  let editorOpen = $state(false);
  // Focus into the editor's first field when it opens (CI HB-DIALOG).
  let givenNameInput = $state<HTMLInputElement | null>(null);
  $effect(() => {
    if (editorOpen && givenNameInput) givenNameInput.focus();
  });
  let editingUid = $state<string | null>(null);
  let form = $state<ContactInput>({
    given_name: "", family_name: "", display_name: "",
    email: "", phone: "", organization: "",
  });

  async function loadContacts() {
    loading = true;
    error = null;
    try {
      contacts = await listContacts(search);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // Refresh (T7, Review 2026-09-13): CardDAV-Sync triggern und Liste neu
  // holen — die anderen Module haben dieselbe Sync-Affordance.
  let syncing = $state(false);
  async function handleRefresh() {
    if (syncing) return;
    syncing = true;
    try {
      await syncCardDav();
      await loadContacts();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      syncing = false;
    }
  }

  onMount(() => { loadContacts(); });

  // Reload after an assistant plan execution (Concept §10.5). Skips the first
  // run so the onMount load is not duplicated.
  let assistantReloaded = false;
  $effect(() => {
    const v = $dataVersion;
    if (!assistantReloaded) { assistantReloaded = true; return; }
    void loadContacts();
  });

  function openCreate() {
    editingUid = null;
    form = { given_name: "", family_name: "", display_name: "", email: "", phone: "", organization: "" };
    editorOpen = true;
  }

  function openEdit(c: ContactInfo) {
    editingUid = c.vcard_uid;
    form = {
      given_name: c.given_name ?? "",
      family_name: c.family_name ?? "",
      display_name: c.display_name ?? "",
      email: c.email ?? "",
      phone: c.phone ?? "",
      organization: c.organization ?? "",
    };
    editorOpen = true;
  }

  async function saveContact() {
    busy = true;
    error = null;
    try {
      if (editingUid) {
        await updateContact(editingUid, form);
      } else {
        await createContact(form);
      }
      editorOpen = false;
      await loadContacts();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  // In-app Bestätigung (S4, Review 2026-09-13): natives confirm() durch
  // die gemeinsame ConfirmationDialog-Komponente ersetzt.
  let deleteTarget = $state<ContactInfo | null>(null);

  // Rechtsklick (T3, Review 2026-09-13): Kontextmenü auf der Kontaktliste.
  let ctxMenu = $state<{ x: number; y: number; contact: ContactInfo } | null>(null);
  let ctxItems = $derived.by(() => {
    const c = ctxMenu;
    if (!c) return [];
    return [
      { label: translate("contacts.editBtn"), action: () => openEdit(c.contact) },
      { label: translate("contacts.deleteBtn"), danger: true, action: () => askDelete(c.contact) },
    ];
  });

  function askDelete(c: ContactInfo): void {
    deleteTarget = c;
  }

  async function removeContact(c: ContactInfo) {
    const name = c.display_name || c.email || translate("contacts.unnamed");
    busy = true;
    error = null;
    try {
      await deleteContact(c.vcard_uid);
      await loadContacts();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      deleteTarget = null;
    }
  }

  function initials(c: ContactInfo): string {
    const g = (c.given_name ?? "").trim();
    const f = (c.family_name ?? "").trim();
    const d = (c.display_name ?? "").trim();
    const first = g ? g[0] : (d ? d[0] : "");
    const last = f ? f[0] : "";
    return (first + last).toUpperCase() || "?";
  }

  // tel:-Link: nur Ziffern + eventuelles führendes Plus behalten (keine
  // Leerzeichen, Bindestriche, Klammern), damit der Anruf zuverlässig startet.
  function telHref(phone: string): string {
    return `tel:${phone.replace(/[^\d+]/g, "")}`;
  }

  // Klick auf eine Kontakt-Mailadresse öffnet das in-app-Compose im Mail-Modul
  // (statt dem externen Mailprogramm via mailto:). Gleicher Hand-off wie der
  // Assistent: Aktion setzen, dann ins Mail-Modul navigieren.
  async function composeTo(email: string | null) {
    if (!email) return;
    assistantAction.set({ type: "open_compose", to: email, subject: "", body: "" });
    await goto("/");
  }
</script>

<!-- T8 (Review 2026-09-13): Escape schließt Dialoge global — der alte
     Backdrop-Handler reagierte nur, wenn der Backdrop selbst den Fokus hielt. -->
<svelte:window
  onkeydown={(e) => {
    if (e.key !== "Escape" || busy) return;
    if (editorOpen) { editorOpen = false; return; }
    if (deleteTarget) deleteTarget = null;
  }}
/>

<div class="ct-app" class:narrow={isNarrow} class:sidebar-open={isNarrow && sidebarOpen}>
  {#if isNarrow && sidebarOpen}
    <div class="ct-scrim" role="presentation" onclick={() => (sidebarOpen = false)}></div>
  {/if}
  <aside class="ct-sidebar" style={isNarrow ? "" : `width: ${$sidebarWidth}px; min-width: ${$sidebarWidth}px;`}>
    <div class="ct-sidebar-header">
      {#if isNarrow}
        <button type="button" class="btn btn-still btn-symbol ct-sidebar-close" onclick={() => (sidebarOpen = false)} aria-label={$t("contacts.close")} title={$t("contacts.close")}><Symbol name="seitenleiste-zu" size={20} /></button>
      {/if}
      <ModuleLogo to="/" label={$t("contacts.title")} noHover />
    </div>

    <div class="ct-tools">
      <button type="button" class="btn btn-primaer" onclick={openCreate}>
        <Symbol name="plus" size={16} />
        {$t("contacts.new")}
      </button>
      <button type="button" class="btn btn-still" onclick={handleRefresh} disabled={syncing}>
        {syncing ? $t("common.syncing") : $t("common.refresh")}
      </button>
    </div>

    <div class="ct-count">{$t("contacts.count", { n: contacts.length })}</div>

    <SidebarFooter active="contacts">
      <SidebarSearch
        bind:value={search}
        placeholder={$t("contacts.searchPlaceholder")}
        ariaLabel={$t("contacts.searchLabel")}
        clearLabel={$t("contacts.clearSearch")}
        onInput={loadContacts}
      />
    </SidebarFooter>
  </aside>
  {#if !isNarrow}
    <div class="resize-handle" role="separator" aria-orientation="vertical" onmousedown={startResize}></div>
  {/if}

  <main class="ct-main">
    {#if isNarrow}
      <div class="ct-mobile-header">
        <button type="button" class="btn btn-still btn-symbol ct-menu-toggle" onclick={() => (sidebarOpen = true)} aria-label={$t("contacts.menu")} title={$t("contacts.menu")}><Symbol name="seitenleiste-auf" size={20} /></button>
        <h1>{$t("contacts.title")}</h1>
      </div>
    {/if}
    {#if loading}
      <div class="ct-state">{$t("contacts.loading")}</div>
    {:else if error}
      <div class="ct-state">
        <div class="hinweis" data-art="fehler" role="alert">
          <Symbol name="achtung" size={16} />
          <span>{error}</span>
        </div>
        <button type="button" class="btn btn-sekundaer" onclick={loadContacts}>{$t("contacts.reload")}</button>
      </div>
    {:else if contacts.length === 0}
      {#if search}
        <EmptyState icon="suche" title={$t("contacts.notFound")} />
      {:else}
        <EmptyState icon="nutzer" title={$t("contacts.empty")} actionLabel={$t("contacts.create")} onaction={openCreate} />
      {/if}
    {:else}
      <ul class="ct-list">
        {#each contacts as c (c.vcard_uid)}
          <li
            class="ct-item"
            oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, contact: c }; }}
          >
            <div class="ct-avatar">{initials(c)}</div>
            <div class="ct-item-body">
              <span class="ct-item-name">{c.display_name || c.email || $t("contacts.unnamed")}</span>
              <span class="ct-item-sub">
                {#if c.email}<button type="button" class="ct-link" onclick={() => composeTo(c.email)} title={$t("contacts.newMailTo", { email: c.email })}>{c.email}</button>{/if}
                {#if c.email && c.phone}<span class="ct-sep">·</span>{/if}
                {#if c.phone}<a class="ct-link" href={telHref(c.phone)} title={$t("contacts.call")}>{c.phone}</a>{/if}
                {#if c.organization}<span class="ct-sep">·</span><span>{c.organization}</span>{/if}
              </span>
            </div>
            <div class="ct-item-actions">
              <button type="button" class="btn btn-still btn-symbol btn-klein" onclick={() => openEdit(c)} aria-label={$t("contacts.editBtn")} title={$t("contacts.editBtn")}>
                <Symbol name="bearbeiten" size={16} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </main>

  {#if editorOpen}
    <!-- HB-DIALOG, narrow: six fields in `.feld`; footer "Speichern", then
         "Abbrechen", and "Kontakt löschen" pushed away to the far end. -->
    <div
      class="dialog-schicht"
      role="dialog"
      aria-modal="true"
      aria-labelledby="ct-editor-title"
      tabindex="-1"
      onclick={(e) => { if ((e.target as HTMLElement).classList.contains("dialog-schicht") && !busy) editorOpen = false; }}
      onkeydown={(e) => { if (e.key === "Escape") !busy && (editorOpen = false); }}
    >
      <div class="karte dialog-karte" data-breite="schmal">
        <div class="dialog-kopf">
          <h2 id="ct-editor-title">{editingUid ? $t("contacts.edit") : $t("contacts.new")}</h2>
          <button type="button" class="dialog-zu" aria-label={$t("contacts.closeDialog")} title={$t("contacts.closeDialog")} onclick={() => (editorOpen = false)} disabled={busy}>
            <Symbol name="schliessen" size={20} />
          </button>
        </div>
        <div class="dialog-koerper">
          <div class="feld">
            <label for="ct-given-name">{$t("contacts.firstName")}</label>
            <input id="ct-given-name" type="text" bind:this={givenNameInput} bind:value={form.given_name} placeholder={$t("contacts.phFirst")} />
          </div>
          <div class="feld">
            <label for="ct-family-name">{$t("contacts.lastName")}</label>
            <input id="ct-family-name" type="text" bind:value={form.family_name} placeholder={$t("contacts.phLast")} />
          </div>
          <div class="feld">
            <label for="ct-display-name">{$t("contacts.displayName")}</label>
            <input id="ct-display-name" type="text" bind:value={form.display_name} placeholder={$t("contacts.phDisplay")} />
          </div>
          <div class="feld">
            <label for="ct-email">{$t("contacts.email")}</label>
            <input id="ct-email" type="email" bind:value={form.email} placeholder="max@example.com" />
          </div>
          <div class="feld">
            <label for="ct-phone">{$t("contacts.phone")}</label>
            <input id="ct-phone" type="tel" bind:value={form.phone} placeholder="+49 123 4567890" />
          </div>
          <div class="feld">
            <label for="ct-organization">{$t("contacts.organization")}</label>
            <input id="ct-organization" type="text" bind:value={form.organization} placeholder={$t("contacts.phOrg")} />
          </div>
        </div>
        <div class="dialog-fuss">
          <button type="button" class="btn btn-primaer" onclick={saveContact} disabled={busy}>
            {busy ? $t("contacts.saving") : $t("common.save")}
          </button>
          <button type="button" class="btn btn-sekundaer" onclick={() => editorOpen = false} disabled={busy}>{$t("common.cancel")}</button>
          {#if editingUid}
            {@const target = contacts.find((x) => x.vcard_uid === editingUid)}
            {#if target}
              <button type="button" class="btn btn-gefahr ct-delete" onclick={() => { editorOpen = false; askDelete(target); }} disabled={busy}>{$t("contacts.deleteBtn")}</button>
            {/if}
          {/if}
        </div>
      </div>
    </div>
  {/if}
</div>

  <ContextMenu
    menu={ctxMenu}
    items={ctxItems}
    onclose={() => (ctxMenu = null)}
  />

  <AssistantFab module="contacts" />

  <ConfirmationDialog
    open={deleteTarget !== null}
    title={$t("contacts.delete")}
    message={deleteTarget
      ? translate("contacts.deleteConfirm", {
          name: deleteTarget.display_name || deleteTarget.email || translate("contacts.unnamed"),
        })
      : ""}
    confirmLabel={$t("contacts.delete")}
    cancelLabel={$t("common.cancel")}
    danger={true}
    onconfirm={() => { if (deleteTarget) void removeContact(deleteTarget); }}
    oncancel={() => (deleteTarget = null)}
  />

<style>
  /* ── Contacts shell: sidebar and main pane [RL-KONTAKTE] ─────────────── */
  .ct-app {
    display: flex;
    height: 100vh;
    background: var(--am-seite);
    color: var(--am-text-primaer);
  }
  .ct-sidebar {
    flex-shrink: 0;
    background: var(--am-flaeche-1);
    border-right: 1px solid var(--am-rand);
    display: flex;
    flex-direction: column;
  }
  .ct-sidebar-header {
    height: var(--am-leistenhoehe);
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--am-rand);
    flex-shrink: 0;
    margin-bottom: 16px;
  }

  .ct-tools { padding: 12px 12px 4px; display: flex; flex-direction: column; gap: 8px; }
  .ct-count {
    padding: 10px 16px;
    font-size: var(--fs-xs);
    color: var(--am-text-gedaempft);
    border-top: 1px solid var(--am-rand);
    margin-top: 8px;
  }
  .ct-main { flex: 1; overflow-y: auto; padding: 20px 24px; }

  /* ── Loading and error state [RL-KONTAKTE] ───────────────────────────── */
  .ct-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
    color: var(--am-text-gedaempft);
    font-size: var(--fs-base);
  }

  /* ── Contact list rows [RL-KONTAKTE] ─────────────────────────────────── */
  /* List rows stay Relay's own; only the edit icon is a `.btn`. */
  .ct-list { list-style: none; margin: 0; padding: 0 0 84px; display: flex; flex-direction: column; gap: 6px; }
  .ct-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: var(--am-flaeche-1);
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
  }
  .ct-item:hover { border-color: var(--am-handlung-ruhend); }
  .ct-avatar {
    width: 40px;
    height: 40px;
    min-width: 40px;
    border-radius: 50%;
    background: var(--am-flaeche-2);
    color: var(--am-text-primaer);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: var(--fs-base);
  }
  .ct-item-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .ct-item-name { font-weight: 600; font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-item-sub { font-size: var(--fs-xs); color: var(--am-text-gedaempft); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Mail address and phone number read as links inside the row. */
  .ct-link {
    color: var(--am-handlung-ruhend);
    text-decoration: none;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    cursor: pointer;
  }
  .ct-link:hover { text-decoration: underline; }
  .ct-sep { margin: 0 4px; opacity: 0.5; }

  .ct-item-actions { display: flex; gap: 4px; }

  /* ── Contact editor dialog [RL-KONTAKTE] ─────────────────────────────── */
  /* Destructive last in the footer, away from "Speichern" (CI R1/G2). */
  .ct-delete { margin-left: auto; }

  /* ── Narrow (mobile ≤768px): sidebar as slide-in overlay [RL-KONTAKTE] ── */
  .ct-app.narrow .ct-sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 85%;
    max-width: 320px;
    z-index: 60;
    transform: translateX(-100%);
    transition: transform var(--am-dauer-mittel) var(--am-kurve);
    box-shadow: var(--am-schatten-1);
  }
  .ct-app.narrow.sidebar-open .ct-sidebar { transform: translateX(0); }
  .ct-app.narrow .ct-scrim {
    position: fixed;
    inset: 0;
    background: var(--am-deckschicht);
    z-index: 55;
  }
  .ct-app.narrow .ct-sidebar-close,
  .ct-app.narrow .ct-menu-toggle { display: inline-flex; }
  .ct-app:not(.narrow) .ct-sidebar-close,
  .ct-app:not(.narrow) .ct-menu-toggle { display: none; }
  .ct-app.narrow .resize-handle { display: none; }
  .ct-app.narrow .ct-main { padding: 12px; }
  .ct-mobile-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }
  .ct-mobile-header h1 {
    margin: 0;
    font-size: var(--fs-md);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
