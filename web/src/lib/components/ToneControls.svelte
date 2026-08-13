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

  let dragging: keyof ToneValues | null = null;
  let trackEls: Record<string, HTMLDivElement> = {};

   let seriositaetPct = $derived(((values.seriositaet - 1) / 6) * 100);
  let textumfangPct = $derived(((values.textumfang - 1) / 6) * 100);

  function labelFor(key: keyof ToneValues, val: number): string {
    if (key === "seriositaet") {
      if (val <= 2) return "Locker";
      if (val <= 4) return "Ausgewogen";
      return "Formell";
    }
    if (val <= 2) return "Knapp";
    if (val <= 4) return "Normal";
    return "Ausführlich";
  }

  function stepLabel(key: keyof ToneValues, val: number): string {
    if (key === "seriositaet") {
      if (val === 1) return "Sehr locker";
      if (val === 7) return "Sehr formell";
    } else {
      if (val === 1) return "Sehr knapp";
      if (val === 7) return "Sehr ausführlich";
    }
    return "";
  }

  function startDrag(key: keyof ToneValues, e: MouseEvent | TouchEvent) {
    dragging = key;
    updateFromEvent(key, e);
    window.addEventListener("mousemove", onDrag);
    window.addEventListener("mouseup", endDrag);
    window.addEventListener("touchmove", onDrag, { passive: false });
    window.addEventListener("touchend", endDrag);
  }

  function onDrag(e: MouseEvent | TouchEvent) {
    if (!dragging) return;
    e.preventDefault();
    updateFromEvent(dragging, e);
  }

  function endDrag() {
    dragging = null;
    window.removeEventListener("mousemove", onDrag);
    window.removeEventListener("mouseup", endDrag);
    window.removeEventListener("touchmove", onDrag);
    window.removeEventListener("touchend", endDrag);
  }

  function updateFromEvent(key: keyof ToneValues, e: MouseEvent | TouchEvent) {
    const track = trackEls[key];
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const clientX = "touches" in e ? e.touches[0].clientX : e.clientX;
    let frac = (clientX - rect.left) / rect.width;
    frac = Math.max(0, Math.min(1, frac));
    const val = Math.round(frac * 6) + 1;
    if (val !== values[key]) {
      values[key] = val;
      onchange?.({ ...values });
    }
  }

  $effect(() => {
    return () => {
      if (dragging) {
        endDrag();
      }
    };
  });
</script>

<div class="tone-controls">
  <div class="sliders-grid">
    <!-- Seriosität -->
    <span class="slider-name">Seriosität</span>
    <span class="range-label range-label--start">Locker</span>
    <div
      class="track"
      bind:this={trackEls["seriositaet"]}
      onmousedown={(e) => startDrag("seriositaet", e)}
      ontouchstart={(e) => startDrag("seriositaet", e)}
      role="slider"
      tabindex="0"
      aria-valuemin={1}
      aria-valuemax={7}
      aria-valuenow={values.seriositaet}
      aria-label="Seriosität"
      onkeydown={(e) => {
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          values.seriositaet = Math.min(7, values.seriositaet + 1);
          onchange?.({ ...values });
        } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          values.seriositaet = Math.max(1, values.seriositaet - 1);
          onchange?.({ ...values });
        }
      }}
    >
      <div class="track-fill" style="width: {seriositaetPct}%"></div>
      <div
        class="thumb"
        style="left: {seriositaetPct}%"
        class:dragging={dragging === 'seriositaet'}
      >
        <div class="thumb-ring"></div>
      </div>
    </div>
    <span class="range-label range-label--end">Formell</span>
    <span class="slider-label">{labelFor("seriositaet", values.seriositaet)}</span>

    <!-- Textumfang -->
    <span class="slider-name">Textumfang</span>
    <span class="range-label range-label--start">Knapp</span>
    <div
      class="track"
      bind:this={trackEls["textumfang"]}
      onmousedown={(e) => startDrag("textumfang", e)}
      ontouchstart={(e) => startDrag("textumfang", e)}
      role="slider"
      tabindex="0"
      aria-valuemin={1}
      aria-valuemax={7}
      aria-valuenow={values.textumfang}
      aria-label="Textumfang"
      onkeydown={(e) => {
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          values.textumfang = Math.min(7, values.textumfang + 1);
          onchange?.({ ...values });
        } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          values.textumfang = Math.max(1, values.textumfang - 1);
          onchange?.({ ...values });
        }
      }}
    >
      <div class="track-fill" style="width: {textumfangPct}%"></div>
      <div
        class="thumb"
        style="left: {textumfangPct}%"
        class:dragging={dragging === 'textumfang'}
      >
        <div class="thumb-ring"></div>
      </div>
    </div>
    <span class="range-label range-label--end">Ausführlich</span>
    <span class="slider-label">{labelFor("textumfang", values.textumfang)}</span>
  </div>
</div>

<style>
  .tone-controls {
    display: flex;
    flex-direction: column;
    padding: 8px 0;
    container-type: inline-size;
  }

  .sliders-grid {
    display: grid;
    grid-template-columns: 95px 46px 1fr 56px 95px;
    grid-template-rows: auto auto;
    gap: 6px 12px;
    align-items: center;
  }

  .slider-name {
    display: inline-block;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-accent);
    white-space: nowrap;
    text-align: center;
    padding: 2px 10px;
    border-radius: 100px;
    background: var(--color-sidebar);
    border: 1px solid var(--color-border);
    min-width: 95px;
  }

  .slider-name:nth-child(1) { grid-row: 1; grid-column: 1; }
  .slider-name:nth-child(7) { grid-row: 2; grid-column: 1; }

  .range-label {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-text-secondary, #8e8e93);
    letter-spacing: 0.01em;
  }

  .range-label--start {
    text-align: right;
  }

  .range-label--end {
    text-align: left;
  }

  .range-label:nth-child(2) { grid-row: 1; grid-column: 2; }
  .range-label:nth-child(5) { grid-row: 1; grid-column: 4; }
  .range-label:nth-child(8) { grid-row: 2; grid-column: 2; }
  .range-label:nth-child(11) { grid-row: 2; grid-column: 4; }

  .slider-label {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-accent, #007aff);
    background: var(--color-list);
    border: 1px solid var(--color-border);
    padding: 2px 10px;
    border-radius: 100px;
    min-width: 95px;
    text-align: center;
    white-space: nowrap;
  }

  .slider-label:nth-child(6) { grid-row: 1; grid-column: 5; }
  .slider-label:nth-child(12) { grid-row: 2; grid-column: 5; }

  .track {
    position: relative;
    height: 24px;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
  }

  .track:nth-child(3) { grid-row: 1; grid-column: 3; }
  .track:nth-child(9) { grid-row: 2; grid-column: 3; }

  .track::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    border-radius: 2px;
    background: var(--color-border, #e5e5ea);
  }

  .track-fill {
    position: absolute;
    left: 0;
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(90deg, var(--color-accent), color-mix(in srgb, var(--color-accent) 80%, #000000));
    transition: width 0.05s linear;
    pointer-events: none;
    z-index: 1;
  }

  .thumb {
    position: absolute;
    width: 18px;
    height: 18px;
    margin-left: -9px;
    z-index: 2;
    transition: transform 0.15s cubic-bezier(0.34, 1.56, 0.64, 1);
    pointer-events: none;
  }

  .thumb-ring {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow:
      0 1px 3px rgba(0, 0, 0, 0.15),
      0 0 0 1px rgba(0, 0, 0, 0.04);
    transition:
      transform 0.15s cubic-bezier(0.34, 1.56, 0.64, 1),
      box-shadow 0.15s ease;
  }

  .track:hover .thumb-ring,
  .thumb.dragging .thumb-ring {
    transform: scale(1.25);
    box-shadow:
      0 3px 8px rgba(0, 0, 0, 0.2),
      0 0 0 2px color-mix(in srgb, var(--color-accent, #007aff) 30%, transparent);
  }

  .track:active .thumb-ring {
    transform: scale(1.35);
  }

  .track:focus-visible {
    outline: none;
  }

  .track:focus-visible .thumb-ring {
    box-shadow:
      0 1px 3px rgba(0, 0, 0, 0.15),
      0 0 0 2px var(--color-accent, #007aff);
  }

  @container (max-width: 480px) {
    /* Compact stacked layout: pill (name + current value) above the slider,
       endpoint labels below. Sliders get bigger touch targets on phones. */
    .sliders-grid {
      grid-template-columns: 1fr auto;
      grid-template-rows: auto auto auto auto auto auto;
      grid-template-areas:
        "label badge"
        "slider slider"
        "start end"
        "label2 badge2"
        "slider2 slider2"
        "start2 end2";
      gap: 4px 8px;
    }

    .slider-name:nth-child(1) { grid-area: label; justify-self: start; min-width: 0; }
    .range-label:nth-child(2) { grid-area: start; text-align: left; }
    .track:nth-child(3) { grid-area: slider; }
    .range-label:nth-child(5) { grid-area: end; text-align: right; }
    .slider-label:nth-child(6) { grid-area: badge; min-width: 0; }

    .slider-name:nth-child(7) { grid-area: label2; justify-self: start; min-width: 0; margin-top: 6px; }
    .range-label:nth-child(8) { grid-area: start2; text-align: left; }
    .track:nth-child(9) { grid-area: slider2; }
    .range-label:nth-child(11) { grid-area: end2; text-align: right; }
    .slider-label:nth-child(12) { grid-area: badge2; min-width: 0; margin-top: 6px; }

    .slider-name,
    .slider-label {
      font-size: 0.6875rem;
      padding: 2px 8px;
    }

    /* Larger hit area + thumb for fingers. */
    .track {
      height: 32px;
    }
    .thumb {
      width: 26px;
      height: 26px;
      margin-left: -13px;
    }
    .thumb-ring {
      width: 26px;
      height: 26px;
    }
  }
</style>
