<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  // Confirmation card for an ActionPlan (Concept §10.3) in the shape of
  // HB-ASSISTENT's card (Rocket: assistent.tsx → Karte): the ready request,
  // a person presses "Ausführen". Presentational: the parent (Drawer)
  // performs the confirm/discard/undo calls and passes the resulting plan
  // back. Relay's own on top of Rocket's card: several steps per plan, the
  // second stage for steps that go outside (external tier), undo after
  // execution, and the "Öffnen" link into the created item.
  import { t, translate } from "$lib/i18n";
  import type { AgentPlan } from "$lib/services/tauri";

  interface Props {
    plan: AgentPlan;
    busy?: boolean;
    onConfirm?: (plan: AgentPlan, confirmExternal: boolean) => void;
    onDiscard?: (plan: AgentPlan) => void;
    onUndo?: (plan: AgentPlan) => void;
    onOpen?: (path: string) => void;
  }

  let { plan, busy = false, onConfirm, onDiscard, onUndo, onOpen }: Props = $props();

  let externalArmed = $state(false);

  const isExternal = $derived(plan.steps.some((s) => s.tier === "external"));
  const stepLabel = $derived(
    plan.steps.length === 1 ? translate("assistant.plan.step") : translate("assistant.plan.steps", { n: plan.steps.length }),
  );

  // After execution, the first result may carry the created id — substitute it
  // into the `danach` navigation template.
  const openPath = $derived.by(() => {
    const first = plan.steps[0];
    if (!first || !first.danach) return "";
    let id = "";
    if (plan.result_json) {
      try {
        const arr = JSON.parse(plan.result_json);
        if (Array.isArray(arr) && arr.length > 0) {
          const r = arr[0];
          id = String(r?.id ?? r?.uid ?? r?.event_id ?? "");
        }
      } catch {
        id = "";
      }
    }
    return first.danach.replace("{id}", id);
  });

  const statusKey = $derived(`assistant.plan.${plan.status}`);
  const verworfen = $derived(plan.status === "cancelled" || plan.status === "expired");
</script>

<div
  class="assistent-karte"
  class:verworfen
  class:ausgefuehrt={plan.status === "executed"}
  class:fehlgeschlagen={plan.status === "failed"}
>
  <div class="assistent-karte-kopf">
    <strong>{stepLabel}</strong>
    <span class="plan-status" data-status={plan.status}>
      {#if plan.status === "executed"}<Symbol name="erfolg" size={16} />{/if}
      {$t(statusKey)}
    </span>
  </div>

  {#each plan.steps as step, i (i)}
    <div class="plan-schritt">
      <p class="plan-schritt-titel">{step.title}</p>
      {#if step.rows.length > 0}
        <dl class="assistent-karte-zeilen">
          {#each step.rows as [k, v], j (j)}
            <div>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          {/each}
        </dl>
      {/if}
    </div>
  {/each}

  {#if isExternal && plan.status === "pending"}
    <div class="hinweis plan-hinweis" data-art="achtung"><Symbol name="achtung" size={16} /><span>{$t("assistant.plan.externalWarning")}</span></div>
  {/if}

  {#if plan.status === "pending"}
    <div class="btn-reihe plan-aktionen">
      {#if isExternal && externalArmed}
        <button type="button" class="btn btn-primaer btn-klein" disabled={busy} onclick={() => onConfirm?.(plan, true)}>
          {$t("assistant.plan.confirmExternal")}
        </button>
      {:else if isExternal}
        <!-- First stage only arms; the request goes out on the second. -->
        <button type="button" class="btn btn-primaer btn-klein" disabled={busy} onclick={() => (externalArmed = true)}>
          {$t("assistant.plan.execute")}
        </button>
      {:else}
        <button type="button" class="btn btn-primaer btn-klein" disabled={busy} onclick={() => onConfirm?.(plan, false)}>
          {$t("assistant.plan.execute")}
        </button>
      {/if}
      <button type="button" class="btn btn-still btn-klein" disabled={busy} onclick={() => onDiscard?.(plan)}>
        {$t("assistant.plan.discard")}
      </button>
    </div>
  {:else if plan.status === "executed"}
    <div class="btn-reihe plan-aktionen">
      <button type="button" class="btn btn-sekundaer btn-klein" disabled={busy} onclick={() => onUndo?.(plan)}>
        {$t("assistant.plan.undo")}
      </button>
    </div>
    {#if openPath && onOpen}
      <p class="assistent-hinweis plan-oeffnen">
        <a
          href={openPath}
          onclick={(e) => {
            e.preventDefault();
            onOpen(openPath);
          }}>{$t("assistant.plan.open")}</a
        >
      </p>
    {/if}
  {/if}
</div>

<style>
  /* ── Plan card [RL-PLAN] ──────────────────────────────────────────────────
     Card, head and rows are HB-ASSISTENT (.assistent-karte …). Relay's own:
     the status word (Rocket uses its lead stage chip, which Relay has not),
     the red accent of a failed plan, several steps per card, and the
     spacing of the action row (inline in Rocket). */
  .assistent-karte.fehlgeschlagen {
    border-left-color: var(--am-fehler);
  }
  .plan-status {
    display: inline-flex;
    align-items: center;
    gap: var(--am-raum-1);
    flex: none;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
  }
  .plan-status[data-status="executed"] {
    color: var(--am-erfolg);
  }
  .plan-status[data-status="failed"] {
    color: var(--am-fehler);
  }

  /* ── Steps [RL-PLAN] ──────────────────────────────────────────────────── */
  .plan-schritt + .plan-schritt {
    margin-top: var(--am-raum-2);
    padding-top: var(--am-raum-2);
    border-top: 1px solid var(--am-trennlinie);
  }
  .plan-schritt-titel {
    margin: 0 0 var(--am-raum-1);
    font-weight: 600;
    color: var(--am-text-primaer);
  }

  /* ── Actions [RL-PLAN] ────────────────────────────────────────────────────
     Buttons and row from AM-KNOPF; the external warning is HB-ZUSTAND. */
  .plan-hinweis,
  .plan-aktionen,
  .plan-oeffnen {
    margin-top: var(--am-raum-2);
  }
  .plan-aktionen {
    gap: var(--am-raum-2);
  }
</style>
