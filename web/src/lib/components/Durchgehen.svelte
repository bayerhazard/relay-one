<script lang="ts">
  // "Durchgehen" — one mail after the other (Kai, 7.10.2026; part of
  // "Aufräumen"). Keep, archive, trash or spam with a button or a key; the
  // page executes every move with "Rückgängig". The queue is fixed when the
  // view opens, so a mail that arrives meanwhile does not jump in.
  import { untrack } from "svelte";
  import Symbol from "$lib/components/Symbol.svelte";
  import { lang, t } from "$lib/i18n";
  import { fetchMessageBody, markAsRead } from "$lib/services/tauri";
  import type { Message } from "$lib/stores/mailbox";

  type Art = "archiv" | "papierkorb" | "spam";

  interface Props {
    accountId: number;
    folder: string;
    messages: Message[];
    istSpamOrdner: boolean;
    onaktion: (uids: number[], art: Art) => void;
    onschliessen: () => void;
  }

  let { accountId, folder, messages, istSpamOrdner, onaktion, onschliessen }: Props = $props();

  // A snapshot of the folder at opening time (newest first).
  const warteschlange = $state<Message[]>(untrack(() => [...messages]));
  let index = $state(0);
  let text = $state<string | null>(null);
  let gesamt = warteschlange.length;
  let aktuell = $derived(warteschlange[index] ?? null);

  $effect(() => {
    const m = aktuell;
    text = null;
    if (!m) return;
    const uid = m.uid;
    fetchMessageBody(accountId, uid, folder)
      .then((b) => { if (aktuell?.uid === uid) text = (b.body_text ?? "").trim(); })
      .catch(() => { if (aktuell?.uid === uid) text = m.body_preview ?? ""; });
  });

  function behalten() {
    const m = aktuell;
    if (!m) return;
    if (!m.is_read) void markAsRead(accountId, m.uid, folder).catch(() => {});
    index += 1;
  }

  function zurueck() {
    if (index > 0) index -= 1;
  }

  function handeln(art: Art) {
    const m = aktuell;
    if (!m) return;
    onaktion([m.uid], art);
    warteschlange.splice(index, 1);
  }

  function taste(e: KeyboardEvent) {
    const ziel = e.target as HTMLElement | null;
    if (ziel?.closest("input, textarea, [contenteditable=true]")) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    switch (e.key) {
      case "ArrowRight": case "j": e.preventDefault(); behalten(); break;
      case "ArrowLeft": case "k": e.preventDefault(); zurueck(); break;
      case "e": case "E": e.preventDefault(); handeln("archiv"); break;
      case "Delete": case "Backspace": e.preventDefault(); handeln("papierkorb"); break;
      case "!": e.preventDefault(); handeln("spam"); break;
      case "Escape": e.preventDefault(); onschliessen(); break;
    }
  }

  function datum(d?: string): string {
    if (!d) return "";
    const x = new Date(d);
    return isNaN(x.getTime()) ? "" : x.toLocaleString($lang === "en" ? "en-GB" : "de-DE", { dateStyle: "medium", timeStyle: "short" });
  }
</script>

<svelte:window onkeydown={taste} />

<div class="durchgehen">
  <div class="seitenkopf">
    <div class="seitenkopf-zeile">
      <h1>{$t("mail.durchgehenTitel")}</h1>
      {#if aktuell}
        <span class="seitenkopf-zahl">{$t("mail.durchgehenStand", { nr: gesamt - warteschlange.length + index + 1, count: gesamt })}</span>
      {/if}
    </div>
    <div class="btn-reihe">
      <button type="button" class="btn btn-still" onclick={onschliessen}>{$t("mail.beenden")}</button>
    </div>
  </div>

  <div class="durchgehen-inhalt">
    {#if !aktuell}
      <div class="durchgehen-fertig">
        <Symbol name="erfolg" size={40} />
        <p>{$t("mail.durchgehenFertig")}</p>
        <button type="button" class="btn btn-primaer" onclick={onschliessen}>{$t("mail.beenden")}</button>
      </div>
    {:else}
      <article class="durchgehen-mail karte" aria-live="polite">
        <header class="durchgehen-kopf">
          <span class="durchgehen-von">{aktuell.from ?? ""}</span>
          <span class="durchgehen-datum">{datum(aktuell.date)}</span>
          <h2 class="durchgehen-betreff">{aktuell.subject || "—"}</h2>
        </header>
        <div class="durchgehen-text">{text ?? aktuell.body_preview ?? ""}</div>
      </article>

      <div class="durchgehen-knoepfe">
        <button type="button" class="btn btn-still" onclick={zurueck} disabled={index === 0}>{$t("mail.zurueck")}</button>
        <button type="button" class="btn btn-sekundaer" onclick={() => handeln("papierkorb")}>{$t("mail.deleteMail")}</button>
        {#if !istSpamOrdner}
          <button type="button" class="btn btn-sekundaer" onclick={() => handeln("spam")}>{$t("mail.spam")}</button>
        {/if}
        <button type="button" class="btn btn-sekundaer" onclick={() => handeln("archiv")}>{$t("mail.archive")}</button>
        <button type="button" class="btn btn-primaer" onclick={behalten}>{$t("mail.behalten")}</button>
      </div>
      <p class="durchgehen-tasten">{$t("mail.durchgehenTasten")}</p>
    {/if}
  </div>
</div>

<style>
  /* ── Going through, one mail at a time [RL-DURCHGEHEN] ───────────────────
     The mail as a card (plain text only — no remote content here), the
     actions below it, "Behalten" the one primary. */
  .durchgehen {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }
  .durchgehen-inhalt {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-4);
    padding: var(--am-raum-4) var(--am-raum-6) var(--am-raum-8);
    max-width: 48rem;
    width: 100%;
  }
  .durchgehen-mail {
    display: flex;
    flex-direction: column;
    gap: var(--am-raum-3);
    padding: var(--am-raum-4);
  }
  .durchgehen-kopf {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .durchgehen-von {
    font-weight: 600;
    color: var(--am-text-primaer);
    overflow-wrap: anywhere;
  }
  .durchgehen-datum {
    font-size: var(--fs-sm);
    color: var(--am-text-sekundaer);
  }
  .durchgehen-betreff {
    margin: var(--am-raum-2) 0 0;
    font-size: var(--fs-lg);
    color: var(--am-text-primaer);
    overflow-wrap: anywhere;
  }
  .durchgehen-text {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 50vh;
    overflow-y: auto;
    color: var(--am-text-primaer);
  }
  .durchgehen-knoepfe {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--am-raum-2);
  }
  .durchgehen-tasten {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--am-text-sekundaer);
    text-align: right;
  }
  .durchgehen-fertig {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--am-raum-4);
    padding: var(--am-raum-12) 0;
    text-align: center;
    color: var(--am-text-sekundaer);
  }
  @media (max-width: 40rem) {
    .durchgehen-inhalt {
      padding: var(--am-raum-3) var(--am-raum-4) var(--am-raum-8);
    }
    .durchgehen-tasten {
      display: none;
    }
  }
</style>
