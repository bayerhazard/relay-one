<script lang="ts">
  // The profile top right — CI HB-KONTO (ABGLEICH G8, RL-G1; Kai 6.10.2026),
  // after Insilo's profil.tsx. Far right in the header, on desktop and phone
  // at the same place. The menu follows the CI's fixed order: head with the
  // name › Einstellungen (one person per box, so no organisation group) ›
  // Darstellung › Sprache. The only way to the settings. No sign-out: Olares
  // signs in and out, a button here would be a dummy. No photo either — the
  // circle carries the initials from the Olares identity.
  // Escape, a click outside and a page change close the menu; focus returns
  // to the button.
  import { onMount, tick } from "svelte";
  import { page } from "$app/state";
  import Symbol from "$lib/components/Symbol.svelte";
  import { t, lang } from "$lib/i18n";
  import { appearance, applyAppearance, type Appearance } from "$lib/stores/appearance";
  import { getOlaresMailStatus } from "$lib/services/tauri";
  import type { SymbolName } from "$lib/symbole";
  import { initialen } from "$lib/initialen";

  let name = $state("");
  let unter = $state("");
  let offen = $state(false);
  let knopf = $state<HTMLButtonElement | null>(null);
  let liste = $state<HTMLDivElement | null>(null);

  onMount(async () => {
    try {
      const s = await getOlaresMailStatus();
      const voll = [s.identity.first_name, s.identity.last_name].filter(Boolean).join(" ");
      name = voll || s.identity.username || s.identity.email || "";
      unter = s.identity.email && s.identity.email !== name ? s.identity.email : "";
    } catch {
      // Without an identity the circle shows "…" — never a made-up name.
    }
  });

  const anzeige = $derived(name || "…");

  // A page change closes the menu.
  $effect(() => {
    void page.url.pathname;
    offen = false;
  });

  async function umschalten() {
    offen = !offen;
    if (offen) {
      await tick();
      liste?.querySelector<HTMLElement>("a, button")?.focus();
    }
  }

  function schliessen(fokus = true) {
    offen = false;
    if (fokus) knopf?.focus();
  }

  function taste(e: KeyboardEvent) {
    if (offen && e.key === "Escape") schliessen(true);
  }

  function klick(e: MouseEvent) {
    if (!offen) return;
    const ziel = e.target as Node;
    if (liste?.contains(ziel) || knopf?.contains(ziel)) return;
    schliessen(false);
  }

  const WAHLEN: { wert: Appearance; text: string; zeichen: SymbolName }[] = [
    { wert: "system", text: "settings.systemMode", zeichen: "geraet-bildschirm" },
    { wert: "light", text: "settings.lightMode", zeichen: "hell" },
    { wert: "dark", text: "settings.darkMode", zeichen: "modus" },
  ];

  function waehle(wert: Appearance) {
    appearance.set(wert);
    applyAppearance();
  }
</script>

<svelte:document onkeydown={taste} onmousedown={klick} />

<div class="person">
  <button
    bind:this={knopf}
    type="button"
    class="person-knopf"
    aria-expanded={offen}
    aria-haspopup="dialog"
    aria-controls="konto-menue"
    aria-label={$t("konto.knopf", { name: anzeige })}
    title={$t("konto.knopf", { name: anzeige })}
    onclick={umschalten}
  >
    <span class="person-kreis" aria-hidden="true">{initialen(name)}</span>
  </button>

  {#if offen}
    <div class="person-liste" id="konto-menue" role="dialog" aria-label={$t("konto.menue")} bind:this={liste}>
      <div class="person-kopf">
        <span class="person-kreis" aria-hidden="true">{initialen(name)}</span>
        <span class="person-text">
          <span class="person-name">{anzeige}</span>
          {#if unter}<span class="person-unter">{unter}</span>{/if}
        </span>
      </div>

      <a href="/settings" class="person-eintrag">
        <Symbol name="einstellungen" />
        <span class="person-eintrag-text">{$t("konto.einstellungen")}</span>
      </a>

      <div class="konto-teil" role="group" aria-label={$t("konto.darstellung")}>
        <p class="konto-abschnitt">{$t("konto.darstellung")}</p>
        {#each WAHLEN as w (w.wert)}
          <button type="button" class="person-eintrag" aria-pressed={$appearance === w.wert} onclick={() => waehle(w.wert)}>
            <Symbol name={w.zeichen} />
            <span class="person-eintrag-text">{$t(w.text)}</span>
            {#if $appearance === w.wert}<Symbol name="erfolg" />{/if}
          </button>
        {/each}
      </div>

      <div class="konto-teil">
        <p class="konto-abschnitt">{$t("konto.sprache")}</p>
        <a href="/settings" class="person-eintrag">
          <Symbol name="netzrecherche" />
          <span class="person-eintrag-text">{$lang === "de" ? "Deutsch" : "English"}</span>
        </a>
      </div>
    </div>
  {/if}
</div>
