<script lang="ts" module>
  export type Bereich = "mail" | "contacts" | "calendar" | "meetings" | "tasks";

  /** What a page inside the shell may ask of it (getContext("huelle")). */
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
  // - Column 240 px, fixed: on top the five areas as navigation rows with
  //   sign and word, the chosen one with the gold edge (G1); below them the
  //   inside of the area (folders, views, settings sections) as the page
  //   hands it in. Nothing at the bottom (G8).
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
    /** The inside of the area, under the five areas in the column. */
    spalte?: Snippet;
    children: Snippet;
  }

  let { bereich = null, suche = $bindable(""), suchePlatzhalter = "", spalte, children }: Props = $props();

  const BEREICHE: { id: Bereich; href: string; text: string; zeichen: SymbolName }[] = [
    { id: "mail", href: "/", text: "mail.title", zeichen: "post" },
    { id: "contacts", href: "/contacts", text: "contacts.title", zeichen: "team" },
    { id: "calendar", href: "/calendar", text: "calendar.title", zeichen: "kalender" },
    { id: "meetings", href: "/meetings", text: "meetings.title", zeichen: "besprechung" },
    { id: "tasks", href: "/tasks", text: "tasks.title", zeichen: "aufgabe" },
  ];

  let spalteOffen = $state(false);
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

<div class="huelle relay-huelle" class:spalte-offen={spalteOffen}>
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
    <Profil />
  </header>

  <nav class="huelle-nav" aria-label={$t("huelle.bereiche")}>
    <div class="huelle-nav-gruppe">
      {#each BEREICHE as b (b.id)}
        <a href={b.href} class="huelle-nav-item" class:aktiv={bereich === b.id} aria-current={bereich === b.id ? "page" : undefined} title={$t(b.text)}>
          <Symbol name={b.zeichen} size={20} />
          <span>{$t(b.text)}</span>
        </a>
      {/each}
    </div>

    <!-- The narrow bar at the bottom: the five areas, all of them (G5). -->
    <div class="huelle-nav-mobil">
      {#each BEREICHE as b (b.id)}
        <a href={b.href} class="huelle-nav-item" class:aktiv={bereich === b.id} aria-current={bereich === b.id ? "page" : undefined} title={$t(b.text)}>
          <Symbol name={b.zeichen} size={20} />
          <span>{$t(b.text)}</span>
        </a>
      {/each}
    </div>

    {#if spalte}
      <!-- The inside of the area (RL-G2). Rendered once: in the column on
           the desktop, as a sheet over the content on the phone. -->
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

  /* ── The inside of the area [RL-SPALTE] ──────────────────────────────────
     Desktop: below the five areas, separated by a line, scrolling with the
     column. Phone: a sheet from the left under the header, floating over the
     content (betonter Rand, the one shadow, R6), opened from the header. */
  .relay-spalte {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-top: 1px solid var(--am-trennlinie);
    margin: var(--am-raum-2) calc(-1 * var(--am-raum-4)) 0;
    padding-top: var(--am-raum-2);
  }
  .relay-spalte-knopf { display: none; }

  @media (max-width: 1023px) {
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
      border-top: none;
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
