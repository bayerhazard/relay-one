<script lang="ts">
  // HB-SYMBOL from the AImighty CI (bauteile/HB-SYMBOL.md), after Rocket's
  // components/symbol.tsx. Every icon in Relay comes from the CI set
  // (lib/symbole.ts, generated). Two CI rules (kern/icons.md, ABGLEICH R2):
  // - sizes only 16 · 20 · 24, plus 40 in empty states — the type allows
  //   nothing else;
  // - the stroke stays 1.5 px at every size, i.e. 1.5 × 24 / size in the
  //   24-unit grid; an icon that thickens with its size sits too bold next
  //   to text.
  import { SYMBOLE, type SymbolName, type Symbolgroesse } from "$lib/symbole";

  interface Props {
    name: SymbolName;
    size?: Symbolgroesse;
    /** Accessible name; without it the icon is decorative (aria-hidden). */
    label?: string;
    class?: string;
    /** Filled state, e.g. a set star; the stroke stays. */
    filled?: boolean;
  }

  let { name, size = 16, label = "", class: klass = "", filled = false }: Props = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width={(1.5 * 24) / size}
  stroke-linecap="round"
  stroke-linejoin="round"
  class={klass || undefined}
  data-symbol={name}
  role={label ? "img" : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : "true"}
>{@html SYMBOLE[name]}</svg>
