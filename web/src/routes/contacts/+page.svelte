<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listContacts, contactZahlen, createContact, updateContact, deleteContact, syncCardDav,
    listAccounts, searchMessages,
    type ContactInfo, type ContactInput, type KontaktQuelle, type KontaktZahlen,
  } from "$lib/services/tauri";
  import type { Message } from "$lib/stores/mailbox";
  import Huelle from "$lib/components/Huelle.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { assistantAction } from "$lib/stores/assistantAction";
  import { t, translate } from "$lib/i18n";
  import { dataVersion } from "$lib/stores/invalidation";

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

  // Three columns (Kai, 9.10.2026): the sources left, the list in the
  // middle, the chosen contact right. The address book comes first; the
  // senders Relay collects from mail stand apart.
  let quelle = $state<KontaktQuelle>("adressbuch");
  let zahlen = $state<KontaktZahlen | null>(null);
  let spalteOffen = $state(false);
  let gewaehltUid = $state<string | null>(null);
  let gewaehlt = $derived(contacts.find((c) => c.vcard_uid === gewaehltUid) ?? null);

  function waehleQuelle(q: KontaktQuelle) {
    spalteOffen = false;
    if (q === quelle) return;
    quelle = q;
    gewaehltUid = null;
    void loadContacts();
  }

  async function loadContacts() {
    loading = true;
    error = null;
    try {
      const [liste, z] = await Promise.all([listContacts(search, quelle), contactZahlen().catch(() => zahlen)]);
      contacts = liste;
      zahlen = z;
      if (gewaehltUid && !liste.some((c) => c.vcard_uid === gewaehltUid)) gewaehltUid = null;
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

  // The search sits in the header now (CI HB-SUCHE, RL-G1); a change
  // reloads the list, as the field in the column did.
  let searchSeen = search;
  $effect(() => {
    if (search === searchSeen) return;
    searchSeen = search;
    void loadContacts();
  });

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

  // The last mails from the chosen contact, from every account (newest
  // first). Only read while a contact is shown.
  let letzteMails = $state<Message[]>([]);
  let mailsLaden = $state(false);
  let kontenIds: number[] | null = null;
  let mailsSeq = 0;
  $effect(() => {
    const email = gewaehlt?.email ?? null;
    const seq = ++mailsSeq;
    letzteMails = [];
    if (!email) return;
    mailsLaden = true;
    void (async () => {
      try {
        if (!kontenIds) kontenIds = (await listAccounts()).map((a) => a.id);
        const je = await Promise.all(
          kontenIds.map((id) => searchMessages(id, `von:${email}`, 5).catch(() => [] as Message[])),
        );
        if (seq !== mailsSeq) return;
        letzteMails = je.flat().sort((a, b) => (b.date ?? "").localeCompare(a.date ?? "")).slice(0, 5);
      } finally {
        if (seq === mailsSeq) mailsLaden = false;
      }
    })();
  });

  function datumKurz(d?: string): string {
    if (!d) return "";
    const t = new Date(d);
    return Number.isNaN(t.getTime()) ? "" : t.toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit", year: "numeric" });
  }

  // "Alle Mails" hands the search to the mail area, as the assistant does.
  async function mailsVon(email: string | null) {
    if (!email) return;
    assistantAction.set({ type: "search", query: `von:${email}` });
    await goto("/");
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

<svelte:head><title>{tabTitel($t("contacts.title"))}</title></svelte:head>

<Huelle bereich="contacts" bind:spalteOffen bind:suche={search} suchePlatzhalter={$t("contacts.searchPlaceholder")}>
  {#snippet spalte()}
    <!-- The inside of the area (RL-G2): where the contacts come from. -->
    <nav class="ct-quellen" aria-label={$t("contacts.quellen")}>
      <button type="button" class="ct-quelle" class:active={quelle === "adressbuch"} aria-current={quelle === "adressbuch" ? "page" : undefined} onclick={() => waehleQuelle("adressbuch")}>
        <Symbol name="team" size={20} />
        <span class="ct-quelle-name">{$t("contacts.adressbuch")}</span>
        {#if zahlen}<span class="ct-zahl">{zahlen.adressbuch}</span>{/if}
      </button>
      <button type="button" class="ct-quelle" class:active={quelle === "mail"} aria-current={quelle === "mail" ? "page" : undefined} onclick={() => waehleQuelle("mail")}>
        <Symbol name="post" size={20} />
        <span class="ct-quelle-name">{$t("contacts.ausMails")}</span>
        {#if zahlen}<span class="ct-zahl">{zahlen.mail}</span>{/if}
      </button>
      <button type="button" class="ct-quelle" class:active={quelle === "alle"} aria-current={quelle === "alle" ? "page" : undefined} onclick={() => waehleQuelle("alle")}>
        <Symbol name="liste" size={20} />
        <span class="ct-quelle-name">{$t("contacts.alle")}</span>
        {#if zahlen}<span class="ct-zahl">{zahlen.alle}</span>{/if}
      </button>
    </nav>
    <div class="ct-spalte-fuss">
      <button type="button" class="btn btn-primaer ct-neu" onclick={() => { spalteOffen = false; openCreate(); }}>
        <Symbol name="plus" size={16} />
        {$t("contacts.new")}
      </button>
    </div>
  {/snippet}

  <main class="ct-main" class:ct-mit-auswahl={gewaehlt !== null}>
    <section class="ct-liste-spalte" aria-label={$t("contacts.title")}>
      <!-- HB-SEITENKOPF: the source as title with its count, refresh right. -->
      <div class="seitenkopf ct-kopf">
        <div class="seitenkopf-zeile">
          <h1>{quelle === "adressbuch" ? $t("contacts.adressbuch") : quelle === "mail" ? $t("contacts.ausMails") : $t("contacts.alle")}</h1>
          <span class="seitenkopf-zahl">{contacts.length}</span>
        </div>
        <div class="btn-reihe">
          <button type="button" class="btn btn-still btn-symbol" onclick={handleRefresh} disabled={syncing}
            title={syncing ? $t("common.syncing") : $t("common.refresh")} aria-label={syncing ? $t("common.syncing") : $t("common.refresh")}>
            <Symbol name="neu-laden" size={20} />
          </button>
        </div>
      </div>
      <div class="ct-inhalt">
      {#if loading && contacts.length === 0}
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
            <li>
              <button
                type="button"
                class="ct-item"
                class:selected={gewaehltUid === c.vcard_uid}
                aria-current={gewaehltUid === c.vcard_uid ? "true" : undefined}
                onclick={() => (gewaehltUid = c.vcard_uid)}
                oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, contact: c }; }}
              >
                <span class="ct-avatar" aria-hidden="true">{initials(c)}</span>
                <span class="ct-item-body">
                  <span class="ct-item-name">{c.display_name || c.email || $t("contacts.unnamed")}</span>
                  <span class="ct-item-sub">{c.organization || c.email || ""}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      </div>
    </section>

    <section class="ct-detail-spalte" aria-label={$t("contacts.detail")}>
      {#if gewaehlt}
        {@const c = gewaehlt}
        <div class="ct-detail">
          <button type="button" class="btn btn-still btn-klein ct-zurueck" onclick={() => (gewaehltUid = null)}>
            <Symbol name="chevron-links" size={16} />
            {$t("contacts.zurueck")}
          </button>
          <div class="ct-detail-kopf">
            <span class="ct-avatar ct-avatar-gross" aria-hidden="true">{initials(c)}</span>
            <div class="ct-detail-titel">
              <h2>{c.display_name || c.email || $t("contacts.unnamed")}</h2>
              {#if c.organization}<p class="ct-detail-firma">{c.organization}</p>{/if}
              {#if c.source === "mail"}<p class="ct-herkunft">{$t("contacts.gesammeltHinweis")}</p>{/if}
            </div>
          </div>
          <div class="btn-reihe ct-detail-aktionen">
            {#if c.email}
              <button type="button" class="btn btn-primaer" onclick={() => composeTo(c.email)}>
                <Symbol name="post" size={16} />
                {$t("contacts.mailSchreiben")}
              </button>
            {/if}
            <button type="button" class="btn btn-sekundaer" onclick={() => openEdit(c)}>
              <Symbol name="bearbeiten" size={16} />
              {c.source === "mail" ? $t("contacts.uebernehmen") : $t("contacts.editBtn")}
            </button>
          </div>
          <dl class="ct-angaben">
            {#if c.email}
              <dt>{$t("contacts.email")}</dt>
              <dd><button type="button" class="ct-link" onclick={() => composeTo(c.email)} title={$t("contacts.newMailTo", { email: c.email })}>{c.email}</button></dd>
            {/if}
            {#if c.phone}
              <dt>{$t("contacts.phone")}</dt>
              <dd><a class="ct-link" href={telHref(c.phone)} title={$t("contacts.call")}>{c.phone}</a></dd>
            {/if}
            {#if c.organization}
              <dt>{$t("contacts.organization")}</dt>
              <dd>{c.organization}</dd>
            {/if}
          </dl>
          {#if c.email}
            <div class="ct-mails">
              <h3>{$t("contacts.letzteMails")}</h3>
              {#if mailsLaden}
                <p class="ct-leise">{$t("contacts.mailsLaden")}</p>
              {:else if letzteMails.length === 0}
                <p class="ct-leise">{$t("contacts.keineMails")}</p>
              {:else}
                <ul class="ct-mail-liste">
                  {#each letzteMails as m (`${m.uid}-${m.date}`)}
                    <li>
                      <span class="ct-mail-betreff">{m.subject || $t("contacts.ohneBetreff")}</span>
                      <span class="ct-mail-datum">{datumKurz(m.date)}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
              <button type="button" class="btn btn-still btn-klein" onclick={() => mailsVon(c.email)}>{$t("contacts.alleMails")}</button>
            </div>
          {/if}
        </div>
      {:else}
        <div class="ct-leer">
          <EmptyState icon="nutzer" title={$t("contacts.waehlen")} />
        </div>
      {/if}
    </section>
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
</Huelle>

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
  /* ── Contacts in three columns [RL-KONTAKTE] (Kai, 9.10.2026) ───────── */
  /* The sources in the shell's column; the list and the chosen contact
     side by side, as the mail list and its reading pane. */
  .ct-main {
    display: grid;
    grid-template-columns: minmax(280px, 380px) 1fr;
    min-height: 0;
    height: 100%;
  }
  .ct-liste-spalte {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--am-rand);
  }
  .ct-kopf { padding-inline: var(--am-raum-4); flex-shrink: 0; }
  .ct-inhalt { flex: 1; min-height: 0; overflow-y: auto; }
  .ct-detail-spalte { min-height: 0; overflow-y: auto; }

  /* ── The sources [RL-KONTAKTE] ────────────────────────────────────────── */
  .ct-quellen { display: flex; flex-direction: column; gap: 2px; }
  .ct-quelle {
    display: flex;
    align-items: center;
    gap: var(--am-raum-3);
    width: 100%;
    min-height: var(--am-ziel-zeiger);
    padding: var(--am-raum-2) var(--am-raum-3);
    border: none;
    background: none;
    color: var(--am-text-sekundaer);
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    font-size: 0.875rem;
    font-family: inherit;
    text-align: left;
  }
  .ct-quelle > :global(svg) { flex: none; }
  .ct-quelle:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .ct-quelle:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }
  .ct-quelle.active {
    background: transparent;
    color: var(--am-text-primaer);
    font-weight: 600;
    box-shadow: inset 2px 0 0 var(--am-gold-auszeichnung);
    border-radius: 0 var(--am-radius-mittel) var(--am-radius-mittel) 0;
  }
  .ct-quelle.active:hover { background: var(--am-flaeche-2); }
  .ct-quelle.active > :global(svg) { color: var(--am-gold-beschriftung); }
  .ct-quelle-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-zahl {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--am-text-gedaempft);
    background: var(--am-flaeche-2);
    border-radius: 999px;
    padding: 0 7px;
    min-width: 20px;
    text-align: center;
  }
  .ct-spalte-fuss { margin-top: var(--am-raum-4); }
  .ct-neu { width: 100%; justify-content: center; }

  /* ── Loading and error state [RL-KONTAKTE] ───────────────────────────── */
  .ct-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
    padding: var(--am-raum-6);
    color: var(--am-text-gedaempft);
    font-size: var(--fs-base);
  }

  /* ── List rows [RL-KONTAKTE] ─────────────────────────────────────────── */
  /* As the mail list: lines between the rows, the chosen one light blue. */
  .ct-list { list-style: none; margin: 0; padding: 0 0 84px; }
  .ct-item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 16px;
    border: none;
    border-bottom: 1px solid var(--am-rand);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .ct-item:hover { background: var(--am-flaeche-1); }
  .ct-item.selected, .ct-item.selected:hover { background: var(--rl-zeile-auswahl); }
  .ct-item:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: -2px; }
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

  /* ── The chosen contact [RL-KONTAKTE] ────────────────────────────────── */
  .ct-detail { padding: var(--am-raum-6) var(--am-raum-8); max-width: 720px; }
  .ct-zurueck { display: none; margin-bottom: var(--am-raum-4); }
  .ct-detail-kopf { display: flex; align-items: center; gap: var(--am-raum-4); }
  .ct-avatar-gross { width: 64px; height: 64px; min-width: 64px; font-size: 1.25rem; }
  .ct-detail-titel { min-width: 0; }
  .ct-detail-titel h2 { margin: 0; font-size: 1.375rem; overflow-wrap: anywhere; }
  .ct-detail-firma { margin: 2px 0 0; color: var(--am-text-sekundaer); }
  .ct-herkunft { margin: var(--am-raum-1) 0 0; font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .ct-detail-aktionen { margin: var(--am-raum-5) 0; flex-wrap: wrap; }
  .ct-angaben {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--am-raum-2) var(--am-raum-5);
    margin: 0 0 var(--am-raum-6);
    font-size: var(--fs-base);
  }
  .ct-angaben dt { color: var(--am-text-gedaempft); }
  .ct-angaben dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  /* Mail address and phone number read as links. */
  .ct-link {
    color: var(--am-handlung-ruhend);
    text-decoration: none;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .ct-link:hover { text-decoration: underline; }
  .ct-mails h3 { margin: 0 0 var(--am-raum-2); font-size: var(--fs-base); }
  .ct-mail-liste { list-style: none; margin: 0 0 var(--am-raum-3); padding: 0; }
  .ct-mail-liste li {
    display: flex;
    justify-content: space-between;
    gap: var(--am-raum-4);
    padding: var(--am-raum-2) 0;
    border-bottom: 1px solid var(--am-rand);
    font-size: var(--fs-base);
  }
  .ct-mail-betreff { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-mail-datum { flex: none; color: var(--am-text-gedaempft); font-size: var(--fs-xs); }
  .ct-leise { color: var(--am-text-gedaempft); font-size: var(--fs-base); margin: 0 0 var(--am-raum-3); }
  .ct-leer { height: 100%; display: flex; align-items: center; justify-content: center; }

  /* ── Contact editor dialog [RL-KONTAKTE] ─────────────────────────────── */
  /* Destructive last in the footer, away from "Speichern" (CI R1/G2). */
  .ct-delete { margin-left: auto; }

  /* ── Narrow [RL-KONTAKTE] ────────────────────────────────────────────── */
  /* The list alone; a chosen contact takes its place, "Zurück" leads back. */
  @media (max-width: 52rem) {
    .ct-main { grid-template-columns: 1fr; }
    .ct-liste-spalte { border-right: none; }
    .ct-detail-spalte { display: none; }
    .ct-mit-auswahl .ct-liste-spalte { display: none; }
    .ct-mit-auswahl .ct-detail-spalte { display: block; }
    .ct-zurueck { display: inline-flex; }
    .ct-detail { padding: var(--am-raum-4); }
  }
</style>
