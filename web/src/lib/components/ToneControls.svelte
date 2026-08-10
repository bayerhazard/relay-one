<script lang="ts">
  export type ToneValues = { seriositaet: number; textumfang: number };

  interface Props {
    values: ToneValues;
    onchange?: (v: ToneValues) => void;
  }

  let {
    values = $bindable({ seriositaet: 4, textumfang: 4 }),
    onchange
  }: Props = $props();

  // Two clean choices per axis (LLM values stay on the 1-7 scale).
  const TONE_LOOSE = 2;
  const TONE_FORMAL = 6;
  const LEN_SHORT = 2;
  const LEN_LONG = 6;

  function setSeriositaet(formal: boolean) {
    values.seriositaet = formal ? TONE_FORMAL : TONE_LOOSE;
    onchange?.({ ...values });
  }

  function setTextumfang(long: boolean) {
    values.textumfang = long ? LEN_LONG : LEN_SHORT;
    onchange?.({ ...values });
  }
</script>

<div class="tone-controls">
  <div class="control-row">
    <span class="control-name">Tonfall</span>
    <div class="segmented" role="group" aria-label="Tonfall">
      <button
        type="button"
        class:active={values.seriositaet < 4}
        onclick={() => setSeriositaet(false)}
      >Locker</button>
      <button
        type="button"
        class:active={values.seriositaet >= 4}
        onclick={() => setSeriositaet(true)}
      >Formell</button>
    </div>
  </div>

  <div class="control-row">
    <span class="control-name">Textumfang</span>
    <div class="segmented" role="group" aria-label="Textumfang">
      <button
        type="button"
        class:active={values.textumfang < 4}
        onclick={() => setTextumfang(false)}
      >Knapp</button>
      <button
        type="button"
        class:active={values.textumfang >= 4}
        onclick={() => setTextumfang(true)}
      >Ausführlich</button>
    </div>
  </div>
</div>

<style>
  .tone-controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 0;
  }

  .control-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .control-name {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--color-text-secondary);
    white-space: nowrap;
  }

  .segmented {
    display: inline-flex;
    padding: 3px;
    gap: 3px;
    background: var(--color-sidebar);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    flex: 1;
    max-width: 280px;
  }

  .segmented button {
    flex: 1;
    border: none;
    background: none;
    padding: 7px 12px;
    border-radius: 7px;
    font-size: 0.8125rem;
    font-weight: 500;
    font-family: inherit;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, box-shadow 0.15s ease;
    white-space: nowrap;
  }

  .segmented button:hover {
    color: var(--color-text);
  }

  .segmented button.active {
    background: var(--color-list);
    color: var(--color-accent);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
    font-weight: 600;
  }

  @media (max-width: 480px) {
    .control-row {
      flex-direction: column;
      align-items: stretch;
      gap: 6px;
    }

    .segmented {
      max-width: none;
    }

    .segmented button {
      padding: 10px 12px;
      font-size: 0.875rem;
    }
  }
</style>
