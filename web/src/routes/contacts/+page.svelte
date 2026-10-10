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

  // As the iPhone's contacts (Kai, 10.10.2026, with his screenshots):
  // "Listen" lead — "Alle Kontakte" is the address book, the senders Relay
  // collects from mail stand apart below as "Weitere Kontakte". On the
  // desktop the lists are the left column; on the phone a page of their
  // own, reached with "‹ Listen" above the list. The last choice is kept.
  const QUELLE_MERKEN = "relay_kontakte_quelle";
  function gemerkteQuelle(): KontaktQuelle {
    try {
      const q = localStorage.getItem(QUELLE_MERKEN);
      if (q === "mail" || q?.startsWith("liste:")) return q as KontaktQuelle;
    } catch { /* no storage */ }
    return "alle";
  }
  let quelle = $state<KontaktQuelle>(gemerkteQuelle());
  let listenOffen = $state(false);
  // The title of the chosen list; a list gone from the address book falls
  // back to all contacts.
  let quellenTitel = $derived.by(() => {
    if (quelle === "mail") return translate("contacts.ausMails");
    if (quelle.startsWith("liste:")) {
      return zahlen?.listen?.find((l) => `liste:${l.uid}` === quelle)?.name || translate("contacts.liste");
    }
    return translate("contacts.alle");
  });
  $effect(() => {
    if (zahlen && quelle.startsWith("liste:") && !zahlen.listen?.some((l) => `liste:${l.uid}` === quelle)) {
      waehleQuelle("alle");
    }
  });
  let zahlen = $state<KontaktZahlen | null>(null);
  let spalteOffen = $state(false);
  let gewaehltUid = $state<string | null>(null);
  let gewaehlt = $derived(contacts.find((c) => c.vcard_uid === gewaehltUid) ?? null);

  function waehleQuelle(q: KontaktQuelle) {
    spalteOffen = false;
    listenOffen = false;
    if (q === quelle) return;
    quelle = q;
    try { localStorage.setItem(QUELLE_MERKEN, q); } catch { /* no storage */ }
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

  // A company has no person's name: a building instead of a letter, as on
  // the iPhone (Walsroder Zeitung).
  function istFirma(c: ContactInfo): boolean {
    return !(c.given_name ?? "").trim() && !(c.family_name ?? "").trim() && !!(c.organization ?? "").trim();
  }

  // The family name in bold, as the iPhone marks what the list is sorted
  // by; a name without one is bold as a whole.
  function namensTeile(c: ContactInfo): [string, string, string] {
    const voll = c.display_name || c.email || translate("contacts.unnamed");
    const f = (c.family_name ?? "").trim();
    const i = f ? voll.indexOf(f) : -1;
    return i < 0 ? ["", voll, ""] : [voll.slice(0, i), f, voll.slice(i + f.length)];
  }

  function initials(c: ContactInfo): string {
    const g = (c.given_name ?? "").trim();
    const f = (c.family_name ?? "").trim();
    const d = (c.display_name ?? "").trim();
    const first = g ? g[0] : (d ? d[0] : "");
    const last = f ? f[0] : "";
    return (first + last).toUpperCase() || "?";
  }

  // A, B, C … as in Apple's and Google's contacts; umlauts under their
  // letter, anything else under "#" at the end.
  function name(c: ContactInfo): string {
    return (c.display_name || c.email || "").trim();
  }
  function buchstabe(c: ContactInfo): string {
    const b = name(c).normalize("NFD").replace(/[\u0300-\u036f]/g, "").charAt(0).toUpperCase();
    return b >= "A" && b <= "Z" ? b : "#";
  }
  let gruppen = $derived.by(() => {
    const sortiert = [...contacts].sort((a, b) => {
      const ba = buchstabe(a), bb = buchstabe(b);
      if (ba !== bb) return ba === "#" ? 1 : bb === "#" ? -1 : ba.localeCompare(bb);
      return name(a).localeCompare(name(b), "de", { sensitivity: "base" });
    });
    const aus: { b: string; kontakte: ContactInfo[] }[] = [];
    for (const c of sortiert) {
      const b = buchstabe(c);
      if (aus.at(-1)?.b !== b) aus.push({ b, kontakte: [] });
      aus.at(-1)!.kontakte.push(c);
    }
    return aus;
  });
  const indexId = (b: string) => `ct-b-${b === "#" ? "anderes" : b}`;
  // The whole alphabet at the edge, as on the iPhone; a letter without
  // contacts leads to the next one that has some.
  const ALPHABET = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ", "#"];
  function springe(b: string) {
    const da = gruppen.map((g) => g.b);
    const ziel = da.find((x) => ALPHABET.indexOf(x) >= ALPHABET.indexOf(b)) ?? da.at(-1);
    if (ziel) document.getElementById(indexId(ziel))?.scrollIntoView({ block: "start" });
  }
  // The index follows the finger, as on the iPhone.
  let ziehen = false;
  function indexBei(e: PointerEvent) {
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const b = el?.dataset?.b;
    if (b) springe(b);
  }

  let kopiert = $state(false);
  async function kopieren(text: string | null) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      kopiert = true;
      setTimeout(() => (kopiert = false), 1500);
    } catch { /* no clipboard: the address stays selectable */ }
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

{#snippet listen()}
  <!-- The lists (iPhone "Listen"): the address book first, the senders
       collected from mail apart below with a line saying what they are. -->
  <nav class="ct-listen" aria-label={$t("contacts.listen")}>
    <div class="ct-listen-gruppe">
      <button type="button" class="ct-quelle" class:active={quelle === "alle"} aria-current={quelle === "alle" ? "page" : undefined} onclick={() => waehleQuelle("alle")}>
        <Symbol name="team" size={20} />
        <span class="ct-quelle-name">{$t("contacts.alle")}</span>
        {#if zahlen}<span class="ct-zahl">{zahlen.alle}</span>{/if}
        <span class="ct-quelle-pfeil"><Symbol name="chevron-rechts" size={16} /></span>
      </button>
      <!-- The address book's own lists (iCloud: Arbeit, Fußball …). -->
      {#each zahlen?.listen ?? [] as l (l.uid)}
        {@const q = `liste:${l.uid}` as KontaktQuelle}
        <button type="button" class="ct-quelle ct-quelle-liste" class:active={quelle === q} aria-current={quelle === q ? "page" : undefined} onclick={() => waehleQuelle(q)}>
          <Symbol name="liste" size={20} />
          <span class="ct-quelle-name">{l.name || $t("contacts.liste")}</span>
          <span class="ct-zahl">{l.zahl}</span>
          <span class="ct-quelle-pfeil"><Symbol name="chevron-rechts" size={16} /></span>
        </button>
      {/each}
    </div>
    <div class="ct-listen-gruppe">
      <button type="button" class="ct-quelle" class:active={quelle === "mail"} aria-current={quelle === "mail" ? "page" : undefined} onclick={() => waehleQuelle("mail")}>
        <Symbol name="post" size={20} />
        <span class="ct-quelle-name">{$t("contacts.ausMails")}</span>
        {#if zahlen}<span class="ct-zahl">{zahlen.mail}</span>{/if}
        <span class="ct-quelle-pfeil"><Symbol name="chevron-rechts" size={16} /></span>
      </button>
    </div>
    <p class="ct-listen-hinweis">{$t("contacts.weitereHinweis")}</p>
  </nav>
{/snippet}

<Huelle bereich="contacts" bind:spalteOffen spalteMobil={false} bind:suche={search} suchePlatzhalter={$t("contacts.searchPlaceholder")}>
  {#snippet spalte()}
    {@render listen()}
  {/snippet}

  <main class="ct-main" class:ct-mit-auswahl={gewaehlt !== null} class:ct-mit-listen={listenOffen}>
    {#if listenOffen}
      <!-- Phone: the lists as a page of their own (iPhone "Listen"). -->
      <section class="ct-listen-seite" aria-labelledby="ct-listen-titel">
        <div class="seitenkopf ct-kopf">
          <div class="seitenkopf-zeile"><h1 id="ct-listen-titel">{$t("contacts.listen")}</h1></div>
        </div>
        {@render listen()}
      </section>
    {/if}
    <section class="ct-liste-spalte" aria-label={$t("contacts.title")}>
      <!-- HB-SEITENKOPF: the list as title with its count; refresh and
           "Neuer Kontakt" right. On the phone "‹ Listen" above it. -->
      <div class="seitenkopf ct-kopf">
        <div class="ct-kopf-oben">
          <button type="button" class="btn btn-still btn-klein ct-zu-listen" onclick={() => (listenOffen = true)}>
            <Symbol name="chevron-links" size={16} />
            {$t("contacts.listen")}
          </button>
        </div>
        <div class="seitenkopf-zeile">
          <h1>{quellenTitel}</h1>
          <span class="seitenkopf-zahl">{contacts.length}</span>
        </div>
        <div class="btn-reihe">
          <button type="button" class="btn btn-still btn-symbol" onclick={handleRefresh} disabled={syncing}
            title={syncing ? $t("common.syncing") : $t("common.refresh")} aria-label={syncing ? $t("common.syncing") : $t("common.refresh")}>
            <Symbol name="neu-laden" size={20} />
          </button>
          <!-- "Neuer Kontakt" in the list's head on every size, as "Neue
               E-Mail" in the mail (Kai, 10.10.2026): a person with a plus. -->
          <button type="button" class="btn btn-symbol ct-neu-kopf" onclick={openCreate}
            title={$t("contacts.new")} aria-label={$t("contacts.new")}>
            <span class="person-plus" aria-hidden="true">
              <Symbol name="nutzer" size={20} />
              <span class="person-plus-zeichen"><Symbol name="plus" size={16} /></span>
            </span>
          </button>
        </div>
      </div>
      {#if quelle === "mail"}
        <p class="ct-weitere-hinweis">{$t("contacts.weitereHinweis")}</p>
      {/if}
      <div class="ct-koerper">
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
        {:else if quelle === "alle"}
          <!-- An empty address book points to the collected ones (the page
               used to stay empty, Kai 10.10.2026). -->
          <EmptyState icon="nutzer" title={$t("contacts.adressbuchLeer")} actionLabel={$t("contacts.create")} onaction={openCreate} />
          {#if zahlen && zahlen.mail > 0}
            <div class="ct-state"><button type="button" class="btn btn-sekundaer" onclick={() => waehleQuelle("mail")}>{$t("contacts.weitereAnsehen", { n: zahlen.mail })}</button></div>
          {/if}
        {:else}
          <EmptyState icon="nutzer" title={$t("contacts.empty")} />
        {/if}
      {:else}
        <ul class="ct-list">
          {#each gruppen as g (g.b)}
            <li class="ct-buchstabe" id={indexId(g.b)}>{g.b}</li>
            {#each g.kontakte as c (c.vcard_uid)}
              {@const [vor, fett, nach] = namensTeile(c)}
              <li>
                <button
                  type="button"
                  class="ct-item"
                  class:selected={gewaehltUid === c.vcard_uid}
                  aria-current={gewaehltUid === c.vcard_uid ? "true" : undefined}
                  onclick={() => (gewaehltUid = c.vcard_uid)}
                  oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, contact: c }; }}
                >
                  <span class="ct-avatar" class:ct-avatar-firma={istFirma(c)} aria-hidden="true">
                    {#if istFirma(c)}<Symbol name="firma" size={20} />{:else}{initials(c)}{/if}
                  </span>
                  <span class="ct-item-name">{vor}<strong>{fett}</strong>{nach}</span>
                </button>
              </li>
            {/each}
          {/each}
        </ul>
      {/if}
      </div>
      {#if gruppen.length > 1}
        <!-- Jump to a letter: tap or slide the finger along (pointer only;
             the list and the search serve the keyboard). -->
        <div
          class="ct-index"
          aria-hidden="true"
          title={$t("contacts.index")}
          onpointerdown={(e) => { ziehen = true; (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); indexBei(e); }}
          onpointermove={(e) => { if (ziehen) indexBei(e); }}
          onpointerup={() => (ziehen = false)}
          onpointercancel={() => (ziehen = false)}
        >
          {#each ALPHABET as b (b)}<span data-b={b}>{b}</span>{/each}
        </div>
      {/if}
      </div>
    </section>

    <section class="ct-detail-spalte" aria-label={$t("contacts.detail")}>
      {#if gewaehlt}
        {@const c = gewaehlt}
        <div class="ct-detail">
          <!-- As on the iPhone: back left, edit right. -->
          <div class="ct-detail-leiste">
            <button type="button" class="btn btn-still btn-klein ct-zurueck" onclick={() => (gewaehltUid = null)}>
              <Symbol name="chevron-links" size={16} />
              {$t("contacts.zurueck")}
            </button>
            <button type="button" class="btn btn-sekundaer btn-klein ct-bearbeiten" onclick={() => openEdit(c)}
              title={c.source === "mail" ? $t("contacts.uebernehmen") : $t("contacts.edit")}>
              {c.source === "mail" ? $t("contacts.aktionUebernehmen") : $t("contacts.editBtn")}
            </button>
          </div>
          <div class="ct-detail-kopf">
            <span class="ct-avatar ct-avatar-gross" class:ct-avatar-firma={istFirma(c)} aria-hidden="true">
              {#if istFirma(c)}<Symbol name="firma" size={40} />{:else}{initials(c)}{/if}
            </span>
            <h2>{c.display_name || c.email || $t("contacts.unnamed")}</h2>
            {#if c.organization && !istFirma(c)}<p class="ct-detail-firma">{c.organization}</p>{/if}
            {#if c.source === "mail"}<p class="ct-herkunft">{$t("contacts.gesammeltHinweis")}</p>{/if}
          </div>
          <!-- Four round buttons that always stand in the same place; what
               a contact lacks stays grey (iPhone: mail without an address). -->
          <div class="ct-aktionen">
            <button type="button" class="ct-aktion" onclick={() => composeTo(c.email)} disabled={!c.email}
              title={c.email ? $t("contacts.newMailTo", { email: c.email }) : $t("contacts.ohneMail")}>
              <span class="ct-aktion-kreis"><Symbol name="post" size={20} /></span>
              <span>{$t("contacts.aktionMail")}</span>
            </button>
            {#if c.phone}
              <a class="ct-aktion" href={telHref(c.phone)} title={$t("contacts.call")}>
                <span class="ct-aktion-kreis"><Symbol name="geraet-telefon" size={20} /></span>
                <span>{$t("contacts.aktionAnrufen")}</span>
              </a>
            {:else}
              <button type="button" class="ct-aktion" disabled title={$t("contacts.ohneTelefon")}>
                <span class="ct-aktion-kreis"><Symbol name="geraet-telefon" size={20} /></span>
                <span>{$t("contacts.aktionAnrufen")}</span>
              </button>
            {/if}
            <button type="button" class="ct-aktion" onclick={() => kopieren(c.email)} disabled={!c.email}
              title={c.email ? $t("contacts.aktionKopierenTitel") : $t("contacts.ohneMail")}>
              <span class="ct-aktion-kreis"><Symbol name={kopiert ? "erfolg" : "kopieren"} size={20} /></span>
              <span>{kopiert ? $t("contacts.aktionKopiert") : $t("contacts.aktionKopieren")}</span>
            </button>
            <button type="button" class="ct-aktion" onclick={() => mailsVon(c.email)} disabled={!c.email}
              title={c.email ? $t("contacts.alleMails") : $t("contacts.ohneMail")}>
              <span class="ct-aktion-kreis"><Symbol name="suche" size={20} /></span>
              <span>{$t("contacts.aktionVerlauf")}</span>
            </button>
          </div>
          {#if c.email || c.phone || (c.organization && !istFirma(c))}
            <dl class="karte ct-angaben">
              {#if c.email}
                <div class="ct-angabe">
                  <dt>{$t("contacts.email")}</dt>
                  <dd><button type="button" class="ct-link" onclick={() => composeTo(c.email)} title={$t("contacts.newMailTo", { email: c.email })}>{c.email}</button></dd>
                </div>
              {/if}
              {#if c.phone}
                <div class="ct-angabe">
                  <dt>{$t("contacts.phone")}</dt>
                  <dd><a class="ct-link" href={telHref(c.phone)} title={$t("contacts.call")}>{c.phone}</a></dd>
                </div>
              {/if}
              {#if c.organization && !istFirma(c)}
                <div class="ct-angabe">
                  <dt>{$t("contacts.organization")}</dt>
                  <dd>{c.organization}</dd>
                </div>
              {/if}
            </dl>
          {/if}
          {#if c.email}
            <div class="karte ct-mails">
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
    grid-template-columns: minmax(280px, 380px) minmax(0, 1fr);
    min-height: 0;
    height: 100%;
  }
  .ct-liste-spalte {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    border-right: 1px solid var(--am-rand);
  }
  .ct-kopf { padding-inline: var(--am-raum-4); flex-shrink: 0; }
  /* The list and its index share one box below the head, so the index
     never reaches under the title. */
  .ct-koerper { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .ct-inhalt { flex: 1; min-height: 0; overflow-y: auto; }
  .ct-detail-spalte { min-width: 0; min-height: 0; overflow-y: auto; }

  /* ── The lists [RL-KONTAKTE] ──────────────────────────────────────────── */
  /* iPhone "Listen" (Kai, 10.10.2026): each list in its own group, the
     count right; "Weitere Kontakte" set apart below with its line. */
  .ct-listen { display: flex; flex-direction: column; gap: var(--am-raum-3); }
  .ct-listen-gruppe { display: flex; flex-direction: column; gap: 2px; }
  .ct-listen-hinweis { margin: calc(-1 * var(--am-raum-2)) var(--am-raum-3) 0; font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
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
  .ct-quelle-pfeil { display: none; color: var(--am-text-gedaempft); }
  /* A list stands under "Alle Kontakte", one step in. */
  .ct-quelle-liste { padding-inline-start: var(--am-raum-6); }
  .ct-listen-seite .ct-quelle + .ct-quelle { border-top: 1px solid var(--am-rand); }
  .ct-zahl { font-size: var(--fs-xs); font-weight: 400; color: var(--am-text-gedaempft); }
  /* The lists as a page of their own on the phone: rows as large as a
     finger, in a card, as the iPhone's groups. */
  .ct-listen-seite { display: none; min-width: 0; overflow-y: auto; }
  .ct-listen-seite .ct-listen { padding: var(--am-raum-4) var(--am-raum-4) var(--am-raum-6); }
  .ct-listen-seite .ct-listen-gruppe {
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-gross);
    background: var(--am-flaeche-1);
    overflow: hidden;
  }
  .ct-listen-seite .ct-quelle {
    min-height: var(--am-ziel-beruehrung);
    padding: var(--am-raum-3) var(--am-raum-4);
    font-size: var(--fs-base);
    color: var(--am-text-primaer);
    border-radius: 0;
  }
  .ct-listen-seite .ct-quelle.active { box-shadow: none; border-radius: 0; font-weight: 600; }
  .ct-listen-seite .ct-zahl { font-size: var(--fs-base); }
  .ct-listen-seite .ct-quelle-pfeil { display: inline-flex; }
  .ct-listen-seite .ct-listen-hinweis { margin-inline: var(--am-raum-4); }

  /* "Neuer Kontakt": a little clearer than the refresh beside it, the plus
     on the person's corner — the same as the letter in the mail. */
  .ct-neu-kopf {
    margin-inline-start: var(--am-raum-1);
    background: var(--am-flaeche-2);
    color: var(--am-text-primaer);
  }
  .ct-neu-kopf:hover { background: var(--am-flaeche-3); }
  .person-plus { position: relative; display: inline-flex; }
  .person-plus-zeichen {
    position: absolute;
    right: -8px;
    bottom: -7px;
    display: inline-flex;
    border-radius: 50%;
    background: var(--am-flaeche-2);
    padding: 1px;
  }
  .ct-neu-kopf:hover .person-plus-zeichen { background: var(--am-flaeche-3); }

  /* "‹ Listen" above the title where the column is not shown. */
  .ct-kopf { flex-wrap: wrap; }
  .ct-kopf-oben { display: none; flex-basis: 100%; }
  @media (max-width: 1023px) {
    .ct-kopf-oben { display: block; }
    .ct-mit-listen .ct-listen-seite { display: block; grid-column: 1 / -1; }
    .ct-mit-listen .ct-liste-spalte, .ct-mit-listen .ct-detail-spalte { display: none !important; }
  }
  .ct-weitere-hinweis {
    margin: 0 var(--am-raum-4) var(--am-raum-3);
    font-size: var(--fs-xs);
    color: var(--am-text-gedaempft);
    flex-shrink: 0;
  }

  /* ── Letters and the index [RL-KONTAKTE] ─────────────────────────────── */
  /* Plain grey letters above a line, no band (iPhone). */
  .ct-buchstabe {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: var(--am-raum-3) var(--am-raum-4) var(--am-raum-1);
    background: var(--am-seite);
    border-bottom: 1px solid var(--am-rand);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--am-text-gedaempft);
  }
  .ct-index {
    position: absolute;
    right: 2px;
    top: var(--am-raum-2);
    bottom: var(--am-raum-2);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 0 2px;
    touch-action: none;
    user-select: none;
    z-index: 2;
    cursor: pointer;
  }
  .ct-index span {
    font-size: 0.6875rem;
    font-weight: 600;
    line-height: 1.3;
    min-height: 0;
    flex: 0 1 auto;
    overflow: hidden;
    color: var(--am-handlung-ruhend);
    padding: 0 4px;
  }
  /* Room for the index next to the rows. */
  .ct-list .ct-item, .ct-list .ct-buchstabe { padding-right: calc(var(--am-raum-4) + 14px); }

  /* ── Loading and error state [RL-KONTAKTE] ───────────────────────────── */
  .ct-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: var(--am-raum-6);
    color: var(--am-text-gedaempft);
    font-size: var(--fs-base);
  }

  /* ── List rows [RL-KONTAKTE] ─────────────────────────────────────────── */
  /* One line per contact, only the name, the family name bold (iPhone);
     the line under a row starts at the name. */
  .ct-list { list-style: none; margin: 0; padding: 0 0 84px; }
  .ct-list li { position: relative; }
  .ct-item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: var(--am-ziel-beruehrung);
    padding: 6px 16px;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .ct-list li:not(.ct-buchstabe) + li:not(.ct-buchstabe) .ct-item { box-shadow: inset 0 1px 0 var(--am-rand); }
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
    font-size: 0.875rem;
  }
  /* A company: a building in a rounded square, as on the iPhone. */
  .ct-avatar-firma { border-radius: var(--am-radius-mittel); color: var(--am-text-sekundaer); }
  .ct-item-name { flex: 1; min-width: 0; font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-item-name strong { font-weight: 600; }

  /* ── The chosen contact [RL-KONTAKTE] ────────────────────────────────── */
  /* As on the iPhone: back left and edit right, the monogram and the name
     in the middle, four round buttons, then the details in cards. Flat:
     no poster, no gradient, no glass (CI). */
  .ct-detail { padding: var(--am-raum-4) var(--am-raum-8) var(--am-raum-8); max-width: 640px; margin-inline: auto; }
  .ct-detail-leiste { display: flex; align-items: center; justify-content: flex-end; gap: var(--am-raum-2); min-height: 40px; }
  .ct-zurueck { display: none; margin-inline-end: auto; }
  .ct-detail-kopf { display: flex; flex-direction: column; align-items: center; text-align: center; gap: var(--am-raum-2); margin-top: var(--am-raum-2); }
  .ct-avatar-gross { width: 96px; height: 96px; min-width: 96px; font-size: 2.25rem; font-weight: 600; }
  .ct-avatar-gross.ct-avatar-firma { border-radius: var(--am-radius-gross); }
  .ct-detail-kopf h2 { margin: var(--am-raum-2) 0 0; font-size: 1.75rem; line-height: 1.2; overflow-wrap: anywhere; }
  .ct-detail-firma { margin: 0; color: var(--am-text-sekundaer); }
  .ct-herkunft { margin: 0; font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .ct-aktionen {
    display: flex;
    justify-content: center;
    gap: var(--am-raum-4);
    margin: var(--am-raum-6) 0 var(--am-raum-6);
  }
  .ct-aktion {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--am-raum-1);
    width: 4.5rem;
    padding: 0;
    border: none;
    background: none;
    color: var(--am-handlung-ruhend);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
  }
  .ct-aktion-kreis {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    border: 1px solid var(--am-rand);
    background: var(--am-flaeche-1);
  }
  .ct-aktion:hover .ct-aktion-kreis { background: var(--am-flaeche-2); }
  .ct-aktion:focus-visible { outline: none; }
  .ct-aktion:focus-visible .ct-aktion-kreis { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }
  .ct-aktion:disabled { color: var(--am-text-gedaempft); cursor: default; }
  .ct-aktion:disabled .ct-aktion-kreis { background: transparent; }
  .ct-aktion:disabled:hover .ct-aktion-kreis { background: transparent; }
  .ct-angaben { display: grid; margin: 0 0 var(--am-raum-4); padding: 0 var(--am-raum-4); }
  .ct-angabe { padding: var(--am-raum-3) 0; }
  .ct-angabe + .ct-angabe { border-top: 1px solid var(--am-rand); }
  .ct-angaben dt { font-size: var(--fs-xs); color: var(--am-text-gedaempft); }
  .ct-angaben dd { margin: 2px 0 0; min-width: 0; overflow-wrap: anywhere; font-size: var(--fs-base); }
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
  .ct-mails { padding: var(--am-raum-4); }
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
    .ct-main { grid-template-columns: minmax(0, 1fr); }
    .ct-liste-spalte { border-right: none; }
    .ct-detail-spalte { display: none; }
    .ct-mit-auswahl .ct-liste-spalte { display: none; }
    .ct-mit-auswahl .ct-detail-spalte { display: block; }
    .ct-zurueck { display: inline-flex; }
    .ct-detail { padding: var(--am-raum-2) var(--am-raum-4) var(--am-raum-8); }
  }
</style>
