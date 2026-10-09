<script lang="ts">
  // "Aufräumen" — who writes to you most (Kai, 7.10.2026; behind the switch
  // in Einstellungen › Allgemein). One row per sender with count and unread;
  // per row: archive all, trash all, spam all, and "Abo beenden" where the
  // newest mail offers it. The page executes every move (with "Rückgängig");
  // this view only removes the row at once and reloads when told to.
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import Symbol from "$lib/components/Symbol.svelte";
  import { lang, t } from "$lib/i18n";
  import type { SymbolName } from "$lib/symbole";
  import {
    getSenders, getUnsubscribeOffer, unsubscribe,
    type Absender, type AbmeldeArt,
  } from "$lib/services/tauri";

  type Art = "archiv" | "papierkorb" | "spam";

  interface Props {
    accountId: number;
    folder: string;
    folderLabel: string;
    istSpamOrdner: boolean;
    /** Bumped by the page after "Rückgängig" — reload the senders. */
    neuLaden: number;
    onaktion: (uids: number[], art: Art) => void;
    onmeldung: (text: string) => void;
    ondurchgehen: () => void;
    onschliessen: () => void;
  }

  let { accountId, folder, folderLabel, istSpamOrdner, neuLaden, onaktion, onmeldung, ondurchgehen, onschliessen }: Props = $props();

  let absender = $state<Absender[]>([]);
  let laedt = $state(true);
  let fehler = $state<string | null>(null);
  let filter = $state("");
  // "Abo beenden" per sender (Kai, 9.10.2026): the sync marks every mail
  // that offers it, so the button stands at each such sender, not only at
  // the 25 most frequent; where the sync has not asked yet, the server is
  // asked for the newest mail of the senders in view.
  let angebote = $state<Record<string, { uid: number; art: AbmeldeArt; ziel: string }>>({});
  let frage = $state<{ adresse: string; uid: number; art: AbmeldeArt; ziel: string } | null>(null);
  let laeuft = $state(false);
  // "Nur Abos": only the senders that offer "Abo beenden".
  let nurAbos = $state(false);
  // Unsubscribe and clean up in one step: afterwards all of the sender's
  // mails go to the trash (with "Rückgängig" as always).
  let danachLoeschen = $state(true);
  let gefragt = new Set<string>();

  /** A sender whose mail offers "Abo beenden" (known from the sync, or
   *  found by asking the server). */
  const hatAbo = (a: Absender) => a.abo === true || !!angebote[a.adresse];

  async function laden() {
    laedt = true;
    fehler = null;
    try {
      absender = await getSenders(accountId, folder);
      angebote = {};
      gefragt = new Set();
    } catch (e: unknown) {
      fehler = e instanceof Error ? e.message : String(e);
    } finally {
      laedt = false;
    }
  }

  // One after the other, so the server is never asked many times at once.
  // Only senders the sync has not judged yet (abo null), each once.
  async function angeboteHolen(liste: Absender[]) {
    if (istSpamOrdner) return; // no unsubscribe from the spam folder
    const stand = `${accountId}/${folder}`;
    for (const a of liste) {
      if (`${accountId}/${folder}` !== stand) return;
      if (a.abo !== null && a.abo !== undefined) continue;
      if (gefragt.has(a.adresse)) continue;
      gefragt.add(a.adresse);
      const uid = a.uids[0];
      if (uid == null) continue;
      try {
        const o = await getUnsubscribeOffer(accountId, uid, folder);
        if (o.art) angebote = { ...angebote, [a.adresse]: { uid, art: o.art, ziel: o.ziel ?? a.adresse } };
      } catch { /* no button */ }
    }
  }

  // The senders in view (the first 25, or what the search shows).
  $effect(() => {
    const blick = sichtbar.slice(0, 25);
    void angeboteHolen(blick);
  });

  // The button: what the sender offers, asked of the server for the mail
  // that has the header, then the question.
  async function abmeldenFragen(a: Absender) {
    if (istSpamOrdner || laeuft) return;
    const bekannt = angebote[a.adresse];
    if (bekannt) {
      frage = { adresse: a.adresse, ...bekannt };
      return;
    }
    const uid = a.abo_uid ?? a.uids[0];
    if (uid == null) return;
    try {
      const o = await getUnsubscribeOffer(accountId, uid, folder);
      if (!o.art) {
        onmeldung($t("mail.unsubscribeNichtMehr"));
        return;
      }
      const angebot = { uid, art: o.art, ziel: o.ziel ?? a.adresse };
      angebote = { ...angebote, [a.adresse]: angebot };
      frage = { adresse: a.adresse, ...angebot };
    } catch (e: unknown) {
      onmeldung(e instanceof Error ? e.message : String(e));
    }
  }

  $effect(() => {
    void accountId;
    void folder;
    void neuLaden;
    void laden();
  });

  let sichtbar = $derived(
    absender.filter((a) =>
      (!nurAbos || hatAbo(a))
      && (!filter.trim() || (a.name + " " + a.adresse).toLowerCase().includes(filter.trim().toLowerCase()))),
  );

  function handeln(a: Absender, art: Art) {
    absender = absender.filter((x) => x.adresse !== a.adresse);
    onaktion([...a.uids], art);
  }

  async function abmelden() {
    const f = frage;
    frage = null;
    if (!f || laeuft) return;
    laeuft = true;
    try {
      const r = await unsubscribe(accountId, f.uid, folder);
      if (r.art === "link" && r.url) {
        window.open(r.url, "_blank", "noopener,noreferrer");
      } else {
        const ziel = r.ziel ?? f.ziel;
        onmeldung(r.art === "mail" ? $t("mail.unsubscribeMailDone", { ziel }) : $t("mail.unsubscribeDone", { ziel }));
        const { [f.adresse]: _, ...rest } = angebote;
        angebote = rest;
      }
      // Clean up in the same step: the sender's mails into the trash.
      const a = absender.find((x) => x.adresse === f.adresse);
      if (a && danachLoeschen) handeln(a, "papierkorb");
    } catch (e: unknown) {
      onmeldung(e instanceof Error ? e.message : String(e));
    } finally {
      laeuft = false;
    }
  }

  function datum(d: string | null): string {
    if (!d) return "";
    const x = new Date(d);
    return isNaN(x.getTime()) ? "" : x.toLocaleDateString($lang === "en" ? "en-GB" : "de-DE", { day: "numeric", month: "short", year: "numeric" });
  }
</script>

{#snippet zeichen(name: SymbolName, wort: string, aktion: () => void)}
  <button type="button" class="btn btn-still btn-symbol" title={wort} aria-label={wort} onclick={aktion}>
    <Symbol {name} size={20} />
  </button>
{/snippet}

<div class="aufraeumen">
  <div class="seitenkopf">
    <div class="seitenkopf-zeile">
      <h1>{$t("mail.aufraeumen")}</h1>
      <span class="seitenkopf-zahl">{folderLabel}</span>
    </div>
    <div class="btn-reihe">
      <button type="button" class="btn btn-sekundaer" onclick={ondurchgehen}>
        {$t("mail.durchgehen")}
      </button>
      <button type="button" class="btn btn-still" onclick={onschliessen}>{$t("mail.beenden")}</button>
    </div>
  </div>

  <div class="aufraeumen-inhalt">
    <p class="aufraeumen-untertitel">{$t("mail.aufraeumenUntertitel")}</p>
    <div class="aufraeumen-werkzeuge">
      <div class="feld aufraeumen-filter">
        <input
          type="search"
          bind:value={filter}
          placeholder={$t("mail.absenderSuchen")}
          aria-label={$t("mail.absenderSuchen")}
        />
      </div>
      {#if !istSpamOrdner}
        <label class="aufraeumen-schalter">
          <input type="checkbox" bind:checked={nurAbos} />
          <span>{$t("mail.nurAbos")}</span>
        </label>
      {/if}
    </div>

    {#if laedt}
      <p class="aufraeumen-hinweis">{$t("mail.aufraeumenLaedt")}</p>
    {:else if fehler}
      <div class="hinweis" data-art="fehler"><Symbol name="achtung" size={16} /><span>{fehler}</span></div>
    {:else if sichtbar.length === 0}
      <p class="aufraeumen-hinweis">{nurAbos ? $t("mail.keineAbos") : $t("mail.aufraeumenLeer")}</p>
    {:else}
      <ul class="aufraeumen-liste">
        {#each sichtbar as a (a.adresse)}
          <li class="aufraeumen-zeile">
            <div class="aufraeumen-wer">
              <span class="aufraeumen-name">{a.name}</span>
              <span class="aufraeumen-adresse">{a.adresse}</span>
              <span class="aufraeumen-zahlen">
                {[
                  a.anzahl === 1 ? $t("mail.aufraeumenEineMail") : $t("mail.aufraeumenMails", { count: a.anzahl }),
                  a.ungelesen > 0 ? $t("mail.aufraeumenUngelesen", { count: a.ungelesen }) : "",
                  datum(a.neueste),
                ].filter(Boolean).join(" · ")}
              </span>
            </div>
            <div class="aufraeumen-knoepfe">
              {#if !istSpamOrdner && hatAbo(a)}
                <button type="button" class="btn btn-sekundaer btn-klein" disabled={laeuft}
                  onclick={() => abmeldenFragen(a)}>
                  {$t("mail.unsubscribe")}
                </button>
              {/if}
              <!-- Signs with the word as tooltip and name (CI G4): every
                   action comes back with "Rückgängig". -->
              {@render zeichen("archiv", a.anzahl === 1 ? $t("mail.archive") : $t("mail.alleArchivieren", { count: a.anzahl }), () => handeln(a, "archiv"))}
              {#if !istSpamOrdner}
                {@render zeichen("spam", a.anzahl === 1 ? $t("mail.alsSpam") : $t("mail.alleSpam", { count: a.anzahl }), () => handeln(a, "spam"))}
              {/if}
              {@render zeichen("loeschen", a.anzahl === 1 ? $t("mail.inPapierkorb") : $t("mail.alleLoeschen", { count: a.anzahl }), () => handeln(a, "papierkorb"))}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

{#if frage}
  <ConfirmationDialog
    open={true}
    title={$t("mail.unsubscribeTitle")}
    message={frage.art === "link"
      ? $t("mail.unsubscribeLinkMsg", { ziel: frage.ziel })
      : $t("mail.unsubscribeMsg", { ziel: frage.ziel })}
    confirmLabel={frage.art === "link" ? $t("mail.unsubscribeLinkConfirm") : $t("mail.unsubscribe")}
    cancelLabel={$t("common.cancel")}
    enterConfirms={false}
    onconfirm={abmelden}
    oncancel={() => (frage = null)}
  >
    {@const wer = absender.find((x) => x.adresse === frage?.adresse)}
    {#if wer}
      <label class="aufraeumen-danach">
        <input type="checkbox" bind:checked={danachLoeschen} />
        <span>{wer.anzahl === 1
          ? $t("mail.danachEineLoeschen", { name: wer.name })
          : $t("mail.danachAlleLoeschen", { count: wer.anzahl, name: wer.name })}</span>
      </label>
    {/if}
  </ConfirmationDialog>
{/if}

<style>
  /* ── Clean up by sender [RL-AUFRAEUMEN] ──────────────────────────────────
     A page in the content area: the page head, then rows on surface 1 with
     the sender left and its actions right, wrapping on the phone. */
  .aufraeumen {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }
  .aufraeumen-inhalt {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-3);
    padding: var(--am-raum-4) var(--am-raum-6) var(--am-raum-8);
    max-width: 60rem;
  }
  .aufraeumen-untertitel,
  .aufraeumen-hinweis {
    margin: 0;
    color: var(--am-text-sekundaer);
  }
  .aufraeumen-werkzeuge {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--am-raum-2) var(--am-raum-4);
  }
  .aufraeumen-filter {
    flex: 1 1 14rem;
    max-width: 24rem;
    margin-bottom: 0;
  }
  .aufraeumen-schalter,
  .aufraeumen-danach {
    display: inline-flex;
    align-items: center;
    gap: var(--am-raum-2);
    color: var(--am-text-primaer);
    cursor: pointer;
  }
  .aufraeumen-danach {
    margin: 0 0 var(--am-raum-4);
  }
  .aufraeumen-liste {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    background: var(--am-flaeche-1);
  }
  .aufraeumen-zeile {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--am-raum-2) var(--am-raum-4);
    padding: var(--am-raum-3) var(--am-raum-4);
  }
  .aufraeumen-zeile + .aufraeumen-zeile {
    border-top: 1px solid var(--am-rand);
  }
  .aufraeumen-wer {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 14rem;
  }
  .aufraeumen-name {
    font-weight: 600;
    color: var(--am-text-primaer);
    overflow-wrap: anywhere;
  }
  .aufraeumen-adresse,
  .aufraeumen-zahlen {
    font-size: var(--fs-sm);
    color: var(--am-text-sekundaer);
    overflow-wrap: anywhere;
  }
  .aufraeumen-knoepfe {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
  }
  .aufraeumen-knoepfe .btn-klein {
    margin-right: var(--am-raum-2);
  }
  @media (max-width: 40rem) {
    .aufraeumen-inhalt {
      padding: var(--am-raum-3) var(--am-raum-4) var(--am-raum-8);
    }
  }
</style>
