<script lang="ts">
  // Calendar, tasks and contacts of one mail account [RL-KONTODAV]
  // (Kai, 9.10.2026, 26.10.18): chosen right when the account is added,
  // switchable later in the settings. For known providers the address comes
  // from Relay and the login is the account's own; the password never
  // reaches the page. Gmail needs Google's sign-in — not yet.
  import { onMount } from "svelte";
  import Symbol from "$lib/components/Symbol.svelte";
  import { getKontoDav, setKontoDav, type KontoDavStand } from "$lib/services/tauri";
  import { t, translate, localizeError } from "$lib/i18n";

  interface Props {
    accountId: number;
    /** "einrichtung": in the setup, its own buttons are "Weiter"/"Überspringen". */
    art?: "einrichtung" | "einstellungen";
    onfertig?: () => void;
  }
  let { accountId, art = "einstellungen", onfertig }: Props = $props();

  let stand = $state<KontoDavStand | null>(null);
  let kalender = $state(false);
  let aufgaben = $state(false);
  let kontakte = $state(false);
  let caldavUrl = $state("");
  let carddavUrl = $state("");
  let eigeneAdresse = $state(false);
  let laeuft = $state(false);
  let fehler = $state<string | null>(null);
  let meldung = $state<string | null>(null);

  let google = $derived(!!stand?.anbieter?.nur_google_anmeldung);
  // Without a known provider the addresses must be given.
  let adressenZeigen = $derived(!!stand && (!stand.anbieter || google || eigeneAdresse));

  onMount(async () => {
    try {
      const s = await getKontoDav(accountId);
      stand = s;
      const neu = !s.kalender && !s.aufgaben && !s.kontakte;
      // In the setup a known provider starts with everything on.
      const vorschlag = art === "einrichtung" && neu && !!s.anbieter && !s.anbieter.nur_google_anmeldung;
      kalender = s.kalender || vorschlag;
      aufgaben = s.aufgaben || vorschlag;
      kontakte = s.kontakte || vorschlag;
      caldavUrl = s.caldav_url;
      carddavUrl = s.carddav_url;
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    }
  });

  async function speichern() {
    laeuft = true;
    fehler = null;
    meldung = null;
    try {
      const r = await setKontoDav(accountId, {
        kalender,
        aufgaben,
        kontakte,
        caldav_url: adressenZeigen ? caldavUrl : undefined,
        carddav_url: adressenZeigen ? carddavUrl : undefined,
      });
      meldung = !kalender && !aufgaben && !kontakte
        ? translate("kontoDav.aus")
        : translate("kontoDav.verbunden", { kalender: r.kalender_gefunden, adressbuecher: r.adressbuecher_gefunden });
      onfertig?.();
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      laeuft = false;
    }
  }
</script>

<div class="konto-dav">
  {#if !stand && !fehler}
    <p class="konto-dav-leise">{$t("kontoDav.laden")}</p>
  {:else if stand}
    <p class="konto-dav-leise">
      {#if google}
        {$t("kontoDav.google")}
      {:else if stand.anbieter}
        {$t("kontoDav.anbieter", { name: stand.anbieter.name })}
      {:else}
        {$t("kontoDav.unbekannt")}
      {/if}
    </p>

    {#each [
      { id: "kalender", wort: $t("kontoDav.kalender"), an: kalender, setze: (v: boolean) => (kalender = v) },
      { id: "aufgaben", wort: $t("kontoDav.aufgaben"), an: aufgaben, setze: (v: boolean) => (aufgaben = v) },
      { id: "kontakte", wort: $t("kontoDav.kontakte"), an: kontakte, setze: (v: boolean) => (kontakte = v) },
    ] as s (s.id)}
      <div class="schalter-zeile">
        <button type="button" id={`dav-${s.id}-${accountId}`} class="schalter" class:an={s.an} role="switch"
          aria-checked={s.an} aria-labelledby={`dav-${s.id}-${accountId}-text`} onclick={() => s.setze(!s.an)}>
          <span class="schalter-knauf" aria-hidden="true"></span>
        </button>
        <label for={`dav-${s.id}-${accountId}`} id={`dav-${s.id}-${accountId}-text`} class="schalter-text">{s.wort}</label>
      </div>
    {/each}

    {#if stand.anbieter && !google}
      <button type="button" class="btn btn-still btn-klein konto-dav-eigen" onclick={() => (eigeneAdresse = !eigeneAdresse)}>
        {eigeneAdresse ? $t("kontoDav.anbieterAdresse") : $t("kontoDav.eigeneAdresse")}
      </button>
    {/if}
    {#if adressenZeigen && (kalender || aufgaben || kontakte)}
      {#if kalender || aufgaben}
        <div class="feld">
          <label for={`dav-caldav-${accountId}`}>{$t("kontoDav.caldavUrl")}</label>
          <input id={`dav-caldav-${accountId}`} type="text" inputmode="url" bind:value={caldavUrl} placeholder="https://cloud.example.de/remote.php/dav/" />
        </div>
      {/if}
      {#if kontakte}
        <div class="feld">
          <label for={`dav-carddav-${accountId}`}>{$t("kontoDav.carddavUrl")}</label>
          <input id={`dav-carddav-${accountId}`} type="text" inputmode="url" bind:value={carddavUrl} placeholder="https://cloud.example.de/remote.php/dav/" />
        </div>
      {/if}
    {/if}
  {/if}

  {#if fehler}
    <div class="hinweis" data-art="fehler" role="alert"><Symbol name="achtung" size={16} /><span>{fehler}</span></div>
  {/if}
  {#if meldung}
    <div class="hinweis" data-art="erfolg" role="status"><Symbol name="erfolg" size={16} /><span>{meldung}</span></div>
  {/if}

  {#if stand}
    <div class="btn-reihe">
      {#if art === "einrichtung"}
        <button type="button" class="btn btn-sekundaer" onclick={() => onfertig?.()} disabled={laeuft}>{$t("kontoDav.ueberspringen")}</button>
      {/if}
      <button type="button" class="btn btn-primaer" onclick={speichern} disabled={laeuft}>
        {laeuft ? $t("kontoDav.verbinde") : art === "einrichtung" ? $t("common.next") : $t("common.save")}
      </button>
    </div>
  {/if}
</div>

<style>
  /* ── Calendar, tasks, contacts of an account [RL-KONTODAV] ──────────── */
  .konto-dav { display: flex; flex-direction: column; gap: var(--am-raum-3); }
  .konto-dav-leise { margin: 0; color: var(--am-text-sekundaer); font-size: var(--fs-sm); }
  .konto-dav-eigen { align-self: flex-start; }
  /* The column's gap spaces the switches; their own margin would double it. */
  .konto-dav .schalter-zeile { margin: 0; align-items: center; }
  .konto-dav .schalter-text { margin-bottom: 0; }
</style>
