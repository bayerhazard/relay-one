<script lang="ts" module>
  import type { SymbolName } from "$lib/symbole";

  export interface Abschnitt {
    id: string;
    text: string;
    zeichen: SymbolName;
    zahl?: number;
  }
</script>

<script lang="ts">
  // The settings sections in the shell's column (CI HB-UNTERNAV, RL-G2):
  // rows with a 20 px sign, the chosen one with the gold edge and
  // aria-current. The page closes the shell's sheet after a choice
  // (bind:spalteOffen on <Huelle>).
  import Symbol from "$lib/components/Symbol.svelte";
  import { t } from "$lib/i18n";

  interface Props {
    abschnitte: Abschnitt[];
    aktiv: string;
    onwahl: (id: string) => void;
  }

  let { abschnitte, aktiv, onwahl }: Props = $props();

</script>

<nav class="einst-spalte" aria-labelledby="einst-spalte-titel">
  <p class="einst-spalte-titel" id="einst-spalte-titel">{$t("settings.title")}</p>
  {#each abschnitte as a (a.id)}
    <button
      type="button"
      class="einst-eintrag"
      class:aktiv={aktiv === a.id}
      aria-current={aktiv === a.id ? "page" : undefined}
      onclick={() => onwahl(a.id)}
    >
      <Symbol name={a.zeichen} size={20} />
      <span class="einst-eintrag-text">{a.text}</span>
      {#if a.zahl}
        <span class="einst-eintrag-zahl">{a.zahl}</span>
      {/if}
    </button>
  {/each}
</nav>

<style>
  /* ── Settings sections in the column [RL-EINSTELLUNGEN] ──────────────────
     HB-UNTERNAV rows in the shell's column: 40 px, 8 px radius, hover
     surface 2; chosen = gold edge, bold, gold sign, no surface (CI G1). */
  .einst-spalte {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-1);
    padding: 0 var(--am-raum-4) var(--am-raum-4);
  }
  .einst-spalte-titel {
    margin: 0;
    padding: var(--am-raum-3) var(--am-raum-3) var(--am-raum-1);
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--am-text-gedaempft);
  }
  .einst-eintrag {
    display: flex;
    align-items: center;
    gap: var(--am-raum-3);
    width: 100%;
    min-height: var(--am-ziel-zeiger);
    padding: var(--am-raum-2) var(--am-raum-3);
    border: none;
    border-radius: var(--am-radius-mittel);
    background: transparent;
    color: var(--am-text-sekundaer);
    font-size: 0.875rem;
    text-align: left;
    cursor: pointer;
  }
  .einst-eintrag:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .einst-eintrag:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }
  .einst-eintrag :global(svg) { flex: none; }
  .einst-eintrag.aktiv {
    background: transparent;
    color: var(--am-text-primaer);
    font-weight: 600;
    box-shadow: inset 2px 0 0 var(--am-gold-auszeichnung);
    border-radius: 0 var(--am-radius-mittel) var(--am-radius-mittel) 0;
  }
  .einst-eintrag.aktiv:hover { background: var(--am-flaeche-2); }
  .einst-eintrag.aktiv :global(svg) { color: var(--am-gold-beschriftung); }
  .einst-eintrag-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .einst-eintrag-zahl {
    flex: none;
    font-family: var(--am-schrift-mono);
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
  }

  /* On the phone the sheet has touch-size rows. */
  @media (max-width: 1023px) {
    .einst-spalte { padding: var(--am-raum-2) var(--am-raum-3) var(--am-raum-4); }
    .einst-eintrag { min-height: var(--am-ziel-beruehrung); }
  }
</style>
