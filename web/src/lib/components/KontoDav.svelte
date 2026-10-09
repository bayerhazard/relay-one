<script lang="ts">
  // Calendar, tasks and contacts of one mail account [RL-KONTODAV]
  // (Kai, 9.10.2026, 26.10.18): chosen right when the account is added,
  // switchable later in the settings. For known providers the address comes
  // from Relay and the login is the account's own; the password never
  // reaches the page. Gmail signs in with Google (26.10.19): the box gets a
  // Google project once, each Gmail account signs in in a window of its own.
  import { onMount } from "svelte";
  import Symbol from "$lib/components/Symbol.svelte";
  import {
    getKontoDav,
    setKontoDav,
    getGoogleApp,
    setGoogleApp,
    startGoogle,
    trenneGoogle,
    type KontoDavStand,
  } from "$lib/services/tauri";
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
  // Google project of the box (only shown for Gmail before it has one).
  let clientId = $state("");
  let clientSecret = $state("");
  let rueckweg = $state("/api/v1/google/rueckweg");
  let kopiert = $state(false);
  let wartetAufGoogle = $state(false);

  let google = $derived(stand?.google ?? null);
  let googleAngemeldet = $derived(!!google?.angemeldet);
  // Gmail shows the switches only once it is signed in.
  let schalterZeigen = $derived(!!stand && (!google || googleAngemeldet));
  // Without a known provider the addresses must be given.
  let adressenZeigen = $derived(!!stand && !google && (!stand.anbieter || eigeneAdresse));
  let rueckwegUrl = $derived(`${typeof location === "undefined" ? "" : location.origin}${rueckweg}`);

  async function laden(vorschlagen: boolean) {
    const s = await getKontoDav(accountId);
    stand = s;
    const neu = !s.kalender && !s.aufgaben && !s.kontakte;
    // In the setup a known provider starts with everything on, and so does
    // an account that has just signed in with Google.
    const bereit = !!s.anbieter && (!s.google || !!s.google.angemeldet);
    const vorschlag = vorschlagen && neu && bereit;
    kalender = s.kalender || vorschlag;
    aufgaben = s.aufgaben || vorschlag;
    kontakte = s.kontakte || vorschlag;
    caldavUrl = s.caldav_url;
    carddavUrl = s.carddav_url;
  }

  onMount(() => {
    laden(art === "einrichtung").catch((e: unknown) => {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    });
    getGoogleApp()
      .then((a) => {
        rueckweg = a.rueckweg;
        clientId = a.client_id ?? "";
      })
      .catch(() => {});
    // The sign-in window reports back and closes itself.
    const antwort = (e: MessageEvent) => {
      if (e.origin !== location.origin || e.data?.typ !== "relay-google") return;
      if (e.data.ok && e.data.konto !== accountId) return;
      wartetAufGoogle = false;
      if (e.data.ok) {
        fehler = null;
        laden(true).catch(() => {});
      } else {
        fehler = String(e.data.meldung ?? "");
      }
    };
    window.addEventListener("message", antwort);
    return () => window.removeEventListener("message", antwort);
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
        : google
          ? translate("kontoDav.verbundenGoogle", {
              kalender: r.kalender_gefunden,
              adressbuecher: r.adressbuecher_gefunden,
              listen: r.aufgabenlisten_gefunden,
            })
          : translate("kontoDav.verbunden", { kalender: r.kalender_gefunden, adressbuecher: r.adressbuecher_gefunden });
      onfertig?.();
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      laeuft = false;
    }
  }

  async function projektSpeichern() {
    laeuft = true;
    fehler = null;
    try {
      await setGoogleApp(clientId, clientSecret);
      clientSecret = "";
      await laden(false);
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      laeuft = false;
    }
  }

  async function mitGoogle() {
    fehler = null;
    try {
      const { url } = await startGoogle(accountId, location.origin);
      const fenster = window.open(url, "relay-google", "popup,width=520,height=680");
      if (!fenster) {
        // Pop-ups blocked: the whole page goes to Google and comes back to
        // the settings.
        location.href = url;
        return;
      }
      wartetAufGoogle = true;
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    }
  }

  async function googleTrennen() {
    laeuft = true;
    fehler = null;
    meldung = null;
    try {
      await trenneGoogle(accountId);
      await laden(false);
      meldung = translate("kontoDav.googleGetrennt");
    } catch (e: unknown) {
      fehler = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      laeuft = false;
    }
  }

  async function kopieren() {
    try {
      await navigator.clipboard.writeText(rueckwegUrl);
      kopiert = true;
      setTimeout(() => (kopiert = false), 2000);
    } catch {
      // Without clipboard access the address stays selectable.
    }
  }
</script>

<div class="konto-dav">
  {#if !stand && !fehler}
    <p class="konto-dav-leise">{$t("kontoDav.laden")}</p>
  {:else if stand}
    <p class="konto-dav-leise">
      {#if google && !google.eingerichtet}
        {$t("kontoDav.googleOhneProjekt")}
      {:else if google && google.angemeldet}
        {$t("kontoDav.googleAngemeldet", { email: google.angemeldet })}
      {:else if google}
        {$t("kontoDav.googleBereit")}
      {:else if stand.anbieter}
        {$t("kontoDav.anbieter", { name: stand.anbieter.name })}
      {:else}
        {$t("kontoDav.unbekannt")}
      {/if}
    </p>

    {#if google && !google.eingerichtet}
      <!-- Once per box: the Google project's client. -->
      <ol class="konto-dav-schritte">
        <li>{$t("kontoDav.googleSchritt1")}</li>
        <li>{$t("kontoDav.googleSchritt2")}</li>
        <li>{$t("kontoDav.googleSchritt3")}</li>
        <li>
          {$t("kontoDav.googleSchritt4")}
          <span class="konto-dav-rueckweg">
            <code>{rueckwegUrl}</code>
            <button type="button" class="btn btn-still btn-klein" onclick={kopieren}>
              <Symbol name={kopiert ? "erfolg" : "kopieren"} size={16} />
              {kopiert ? $t("kontoDav.kopiert") : $t("kontoDav.kopieren")}
            </button>
          </span>
        </li>
        <li>{$t("kontoDav.googleSchritt5")}</li>
      </ol>
      <div class="feld">
        <label for={`google-client-${accountId}`}>{$t("kontoDav.googleClientId")}</label>
        <input id={`google-client-${accountId}`} type="text" autocomplete="off" bind:value={clientId}
          placeholder="123456789-abc.apps.googleusercontent.com" />
      </div>
      <div class="feld">
        <label for={`google-secret-${accountId}`}>{$t("kontoDav.googleSecret")}</label>
        <input id={`google-secret-${accountId}`} type="password" autocomplete="off" bind:value={clientSecret} />
      </div>
    {:else if google && !google.angemeldet}
      <button type="button" class="btn btn-primaer konto-dav-eigen" onclick={mitGoogle} disabled={wartetAufGoogle}>
        <Symbol name="extern" size={16} />
        {wartetAufGoogle ? $t("kontoDav.googleWartet") : $t("kontoDav.googleAnmelden")}
      </button>
    {/if}

    {#if schalterZeigen}
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
    {/if}

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
      {#if google && !google.eingerichtet}
        <button type="button" class="btn btn-primaer" onclick={projektSpeichern}
          disabled={laeuft || !clientId.trim() || !clientSecret.trim()}>{$t("kontoDav.googleProjektSpeichern")}</button>
      {:else if schalterZeigen}
        <button type="button" class="btn btn-primaer" onclick={speichern} disabled={laeuft}>
          {laeuft ? $t("kontoDav.verbinde") : art === "einrichtung" ? $t("common.next") : $t("common.save")}
        </button>
      {/if}
      {#if googleAngemeldet}
        <button type="button" class="btn btn-still" onclick={googleTrennen} disabled={laeuft}>{$t("kontoDav.googleTrennen")}</button>
      {/if}
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
  .konto-dav .feld { margin-bottom: 0; }
  .konto-dav-schritte {
    margin: 0;
    padding-left: var(--am-raum-6);
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-2);
    font-size: var(--fs-sm);
  }
  .konto-dav-rueckweg {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--am-raum-2);
    margin-top: var(--am-raum-2);
  }
  .konto-dav-rueckweg code {
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: all;
    font-family: var(--am-schrift-mono, monospace);
    font-size: var(--fs-sm);
  }
</style>
