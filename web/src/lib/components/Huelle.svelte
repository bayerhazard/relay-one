<script lang="ts" module>
  export type Bereich = "mail" | "contacts" | "calendar" | "meetings" | "tasks";

  /** What components inside the shell may ask of it (getContext("huelle")).
   * The page itself renders the shell and so cannot see this context — it
   * binds `spalteOffen` instead. */
  export interface HuelleKontext {
    /** Closes the column sheet on the phone, e.g. after picking a folder. */
    schliesseSpalte: () => void;
  }
</script>

<script lang="ts">
  // The shell — AM-HUELLE with HB-MARKE, HB-SUCHE and HB-KONTO from the CI,
  // the CSS word for word in global.css. The same frame as Rocket and Insilo
  // (CI ABGLEICH Paket 5, RL-G1, RL-G2; Kai 6.10.2026):
  //
  // - Header corner 240 × 56 with the AImighty word mark and "Relay" (R4, G1);
  //   it leads to the mail, not to the settings.
  // - In the header the search of the area the page is in (HB-SUCHE), far
  //   right the profile (HB-KONTO) — the only way to the settings.
  // - The five areas as signs in the header, right of the search (Kai
  //   7.10.2026, replaces the rows in the column of RL-G2): the chosen one
  //   with a gold sign and a gold edge below, no surface (G1); name and
  //   tooltip on each (G4).
  // - Column 240 px, fixed, all of it for the inside of the area (folders,
  //   views, settings sections) as the page hands it in. Nothing at the
  //   bottom (G8). A page without an inside has no column on the desktop.
  // - On the phone the five areas are the bottom bar (G5); the inside of the
  //   area opens as a sheet from the header.
  import { setContext, type Snippet } from "svelte";
  import { page } from "$app/state";
  import Marke from "$lib/components/Marke.svelte";
  import Profil from "$lib/components/Profil.svelte";
  import Symbol from "$lib/components/Symbol.svelte";
  import { t } from "$lib/i18n";
  import type { SymbolName } from "$lib/symbole";

  interface Props {
    /** The area the page belongs to; none for the settings. */
    bereich?: Bereich | null;
    /** The area's search, shown in the header when a placeholder is given. */
    suche?: string;
    suchePlatzhalter?: string;
    /** The inside of the area: the column on the desktop, a sheet on the phone. */
    spalte?: Snippet;
    /** Whether the column sheet is open on the phone; bind it to close the
     * sheet after a choice in the column (`spalteOffen = false`). */
    spalteOffen?: boolean;
    children: Snippet;
  }

  let { bereich = null, suche = $bindable(""), suchePlatzhalter = "", spalte, spalteOffen = $bindable(false), children }: Props = $props();

  // Translated here, literally, so the i18n guard sees the keys.
  const BEREICHE: { id: Bereich; href: string; text: string; zeichen: SymbolName }[] = $derived([
    { id: "mail", href: "/", text: $t("mail.title"), zeichen: "post" },
    { id: "contacts", href: "/contacts", text: $t("contacts.title"), zeichen: "team" },
    { id: "calendar", href: "/calendar", text: $t("calendar.title"), zeichen: "kalender" },
    { id: "meetings", href: "/meetings", text: $t("meetings.title"), zeichen: "besprechung" },
    { id: "tasks", href: "/tasks", text: $t("tasks.title"), zeichen: "aufgabe" },
  ]);

  setContext<HuelleKontext>("huelle", { schliesseSpalte: () => (spalteOffen = false) });

  // A page change closes the sheet.
  $effect(() => {
    void page.url.pathname;
    spalteOffen = false;
  });

  function taste(e: KeyboardEvent) {
    if (e.key === "Escape" && spalteOffen) spalteOffen = false;
  }
</script>

<svelte:window onkeydown={taste} />

<div class="huelle relay-huelle" class:spalte-offen={spalteOffen} class:ohne-spalte={!spalte}>
  <header class="kopfleiste">
    {#if spalte}
      <button
        type="button"
        class="btn btn-still btn-symbol relay-spalte-knopf"
        aria-expanded={spalteOffen}
        aria-controls="relay-spalte"
        aria-label={spalteOffen ? $t("huelle.spalteZu") : $t("huelle.spalteAuf")}
        title={spalteOffen ? $t("huelle.spalteZu") : $t("huelle.spalteAuf")}
        onclick={() => (spalteOffen = !spalteOffen)}
      >
        <Symbol name={spalteOffen ? "seitenleiste-zu" : "seitenleiste-auf"} size={20} />
      </button>
    {/if}
    <div class="kopfleiste-marke">
      <a href="/" class="marke" aria-label={$t("huelle.start")}>
        <Marke />
        <span class="marke-produkt" aria-hidden="true">Relay</span>
      </a>
    </div>
    {#if suchePlatzhalter}
      <div class="kopfleiste-suche" role="search">
        <div class="kopfsuche">
          <span class="kopfsuche-zeichen"><Symbol name="suche" /></span>
          <input
            type="search"
            class="kopfsuche-feld"
            bind:value={suche}
            placeholder={suchePlatzhalter}
            aria-label={suchePlatzhalter}
            onkeydown={(e) => { if (e.key === "Escape" && suche) { e.stopPropagation(); suche = ""; } }}
          />
        </div>
      </div>
    {/if}
    <!-- The five areas, signs only, right of the search (desktop). -->
    <nav class="relay-bereiche" aria-label={$t("huelle.bereiche")}>
      {#each BEREICHE as b (b.id)}
        <a href={b.href} class="relay-bereich" class:aktiv={bereich === b.id} aria-current={bereich === b.id ? "page" : undefined} aria-label={b.text} title={b.text}>
          <Symbol name={b.zeichen} size={20} />
        </a>
      {/each}
    </nav>
    <Profil />
  </header>

  <nav class="huelle-nav" aria-label={$t("huelle.navigation")}>

    <!-- The narrow bar at the bottom: the five areas, all of them (G5). -->
    <div class="huelle-nav-mobil">
      {#each BEREICHE as b (b.id)}
        <a href={b.href} class="huelle-nav-item" class:aktiv={bereich === b.id} aria-current={bereich === b.id ? "page" : undefined} title={b.text}>
          <Symbol name={b.zeichen} size={20} />
          <span>{b.text}</span>
        </a>
      {/each}
    </div>

    {#if spalte}
      <!-- The inside of the area. Rendered once: the column on the desktop,
           a sheet over the content on the phone. -->
      <div class="relay-spalte" id="relay-spalte">
        {@render spalte()}
      </div>
    {/if}
  </nav>

  {#if spalte && spalteOffen}
    <div class="relay-spalte-deckschicht" role="presentation" onclick={() => (spalteOffen = false)}></div>
  {/if}

  <div class="huelle-inhalt">
    {@render children()}
  </div>
</div>

<style>
  /* ── Shell in a fixed window [RL-HUELLE] ─────────────────────────────────
     Relay scrolls in its columns, not as a page (the mail has list and
     reading pane side by side), so the shell is exactly the window high and
     the content area hands its height to the page. Layout only; AM-HUELLE
     draws everything else. */
  .relay-huelle {
    height: 100dvh;
    min-height: 0;
  }
  .relay-huelle :global(.huelle-inhalt) {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .relay-huelle :global(.huelle-inhalt > *) {
    flex: 1;
    min-height: 0;
  }

  /* ── The five areas in the header [RL-BEREICHSWECHSEL] ──────────────────
     Signs only, 40 px targets, right of the search (Kai 7.10.2026). Chosen:
     gold sign and a 2 px gold edge below, no surface — G1's gold edge,
     turned to the bar it sits in. Hover as every quiet button. Hidden on
     the phone, where the bottom bar carries the areas. */
  .relay-bereiche {
    display: flex;
    align-items: center;
    gap: var(--am-raum-1);
    flex: none;
  }
  .relay-bereich {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--am-ziel-zeiger);
    height: var(--am-ziel-zeiger);
    border-radius: var(--am-radius-mittel);
    color: var(--am-text-sekundaer);
    transition: background var(--am-dauer-schnell) var(--am-kurve), color var(--am-dauer-schnell) var(--am-kurve);
  }
  .relay-bereich:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .relay-bereich.aktiv {
    color: var(--am-gold-beschriftung);
    box-shadow: inset 0 -2px 0 var(--am-gold-auszeichnung);
    border-radius: var(--am-radius-mittel) var(--am-radius-mittel) 0 0;
  }
  .relay-bereich:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }

  /* ── The inside of the area [RL-SPALTE] ──────────────────────────────────
     Desktop: the whole column, scrolling with it. Phone: a sheet from the
     left under the header, floating over the content (betonter Rand, the
     one shadow, R6), opened from the header. Without an inside the column
     goes on the desktop; the bottom bar on the phone stays. */
  .relay-spalte {
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin: 0 calc(-1 * var(--am-raum-4));
  }
  @media (min-width: 1024px) {
    .ohne-spalte { grid-template-columns: minmax(0, 1fr); }
    .ohne-spalte :global(.huelle-nav) { display: none; }
    .ohne-spalte :global(.huelle-inhalt) { grid-column: 1; }
  }
  .relay-spalte-knopf { display: none; }

  @media (max-width: 1023px) {
    .relay-bereiche { display: none; }
    .relay-spalte-knopf { display: inline-flex; }
    .relay-spalte {
      display: none;
      position: fixed;
      top: var(--am-leistenhoehe);
      bottom: 0;
      left: 0;
      width: min(85vw, 20rem);
      margin: 0;
      padding: 0;
      overflow-y: auto;
      background: var(--am-flaeche-1);
      border-right: 1px solid var(--am-rand-betont-farbe);
      box-shadow: var(--am-schatten-1);
      z-index: var(--am-ebene-menue);
    }
    .spalte-offen .relay-spalte { display: flex; }
    .relay-spalte-deckschicht {
      position: fixed;
      inset: var(--am-leistenhoehe) 0 0 0;
      background: var(--am-deckschicht);
      z-index: calc(var(--am-ebene-menue) - 1);
    }
  }
</style>
