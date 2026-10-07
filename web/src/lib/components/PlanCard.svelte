<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  // Confirmation card for an ActionPlan (Concept §10.3). Presentational: the
  // parent (Drawer) performs the confirm/discard/undo calls and passes the
  // resulting plan back. External-tier plans need a second, explicit stage.
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
  const stepCount = plan.steps.length;
  const stepLabel = $derived(stepCount === 1 ? translate("assistant.plan.step") : translate("assistant.plan.steps", { n: stepCount }));

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

  const statusKey = $derived(
    `assistant.plan.${plan.status}`,
  );
</script>

<article class="plan-card" class:external={isExternal} class:executed={plan.status === "executed"}
  class:cancelled={plan.status === "cancelled"} class:failed={plan.status === "failed"}>
  <header class="plan-head">
    <span class="plan-tier" aria-hidden="true">
      {#if isExternal}
        <Symbol name="achtung" size={16} />
      {:else}
        <Symbol name="erfolg" size={16} />
      {/if}
    </span>
    <span class="plan-stepcount">{stepLabel}</span>
    <span class="plan-status status-{plan.status}">{$t(statusKey)}</span>
  </header>

  <div class="plan-steps">
    {#each plan.steps as step}
      <div class="plan-step">
        <p class="plan-step-title">{step.title}</p>
        {#if step.rows.length > 0}
          <dl class="plan-rows">
            {#each step.rows as [k, v]}
              <div class="plan-row">
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            {/each}
          </dl>
        {/if}
      </div>
    {/each}
  </div>

  {#if isExternal && plan.status === "pending"}
    <p class="plan-external-warn">{$t("assistant.plan.externalWarning")}</p>
  {/if}

  <footer class="plan-actions">
    {#if plan.status === "pending"}
      {#if isExternal}
        {#if !externalArmed}
          <button type="button" class="btn btn-secondary" disabled={busy}
            onclick={() => (externalArmed = true)}>
            {$t("assistant.plan.execute")}
          </button>
        {:else}
          <button type="button" class="btn btn-danger" disabled={busy}
            onclick={() => onConfirm?.(plan, true)}>
            {$t("assistant.plan.confirmExternal")}
          </button>
        {/if}
      {:else}
        <button type="button" class="btn btn-primary" disabled={busy}
          onclick={() => onConfirm?.(plan, false)}>
          {$t("assistant.plan.execute")}
        </button>
      {/if}
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => onDiscard?.(plan)}>
        {$t("assistant.plan.discard")}
      </button>
    {:else if plan.status === "executed"}
      {#if openPath && onOpen}
        <button type="button" class="btn btn-secondary" onclick={() => onOpen(openPath)}>
          {$t("assistant.plan.open")}
        </button>
      {/if}
      <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => onUndo?.(plan)}>
        {$t("assistant.plan.undo")}
      </button>
    {/if}
  </footer>
</article>

<style>
  .plan-card {
    border: 1px solid var(--am-rand);
    border-left: 3px solid var(--am-handlung-ruhend);
    border-radius: var(--am-radius-mittel, 8px);
    background: var(--am-flaeche-1);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .plan-card.external {
    border-left-color: var(--am-achtung);
  }
  .plan-card.executed {
    border-left-color: var(--am-erfolg);
    opacity: 0.9;
  }
  .plan-card.failed {
    border-left-color: var(--am-fehler);
  }
  .plan-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
  }
  .plan-tier {
    display: inline-flex;
    color: var(--am-handlung-ruhend);
  }
  .plan-card.external .plan-tier {
    color: var(--am-achtung);
  }
  .plan-stepcount {
    font-weight: 600;
  }
  .plan-status {
    margin-left: auto;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--am-flaeche-2);
  }
  .plan-status.status-executed {
    color: var(--am-erfolg);
    background: color-mix(in srgb, var(--am-erfolg) 12%, transparent);
  }
  .plan-status.status-cancelled,
  .plan-status.status-expired {
    color: var(--am-text-gedaempft);
  }
  .plan-status.status-failed {
    color: var(--am-fehler);
    background: color-mix(in srgb, var(--am-fehler) 12%, transparent);
  }
  .plan-steps {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .plan-step + .plan-step {
    border-top: 1px solid var(--am-rand);
    padding-top: 8px;
  }
  .plan-step-title {
    margin: 0 0 4px;
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--am-text-primaer);
  }
  .plan-rows {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .plan-row {
    display: flex;
    gap: 8px;
    font-size: 0.8125rem;
  }
  .plan-row dt {
    flex: 0 0 96px;
    color: var(--am-text-gedaempft);
  }
  .plan-row dd {
    margin: 0;
    color: var(--am-text-primaer);
    word-break: break-word;
  }
  .plan-external-warn {
    margin: 0;
    font-size: 0.75rem;
    color: var(--am-achtung);
  }
  .plan-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .btn {
    font-family: inherit;
    font-size: 0.8125rem;
    font-weight: 600;
    padding: 7px 14px;
    border-radius: var(--am-radius-mittel);
    border: 1px solid var(--am-rand);
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-primary {
    background: var(--am-handlung-ruhend);
    border-color: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
  }
  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn-secondary {
    background: var(--am-seite);
    color: var(--am-text-primaer);
  }
  .btn-secondary:hover:not(:disabled) {
    border-color: var(--am-handlung-ruhend);
    color: var(--am-handlung-ruhend);
  }
  .btn-ghost {
    background: transparent;
    color: var(--am-text-gedaempft);
  }
  .btn-ghost:hover:not(:disabled) {
    color: var(--am-text-primaer);
  }
  .btn-danger {
    background: var(--am-fehler);
    border-color: var(--am-fehler);
    color: var(--am-handlung-text);
  }
  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.08);
  }
</style>
