<script lang="ts">
  // HB-ASSISTENT from the AImighty CI (bauteile/HB-ASSISTENT.md), after
  // Rocket's components/schild.tsx: the AImighty shield — gold, with the two
  // eyes. Scales by `size` (width; the height follows the 168 × 210 viewBox).
  //
  // With `zwinkert` the eyes blink every few seconds while nothing is going
  // on — short, irregular, and not at all under prefers-reduced-motion (the
  // CI CSS, .schild-zwinkert). A sign of life, not a bounce.
  //
  // The one brand drawing allowed as inline svg (symbole.test.ts); its fixed
  // colours are the brand mark, as in Rocket, not theme colours.
  import { onMount } from "svelte";

  interface Props {
    size?: number;
    class?: string;
    zwinkert?: boolean;
  }

  let { size = 20, class: klasse = "", zwinkert = false }: Props = $props();

  // A delay per shield: two shields on one page would otherwise blink in
  // lockstep, and that looks like a machine. Rolled only after mounting —
  // a random value while rendering would differ between server and browser.
  let versatz = $state("0s");
  onMount(() => {
    versatz = `${(Math.random() * 4).toFixed(2)}s`;
  });

  const klassen = $derived(`${klasse}${zwinkert ? " schild-zwinkert" : ""}`.trim() || undefined);
</script>

<svg
  width={size}
  height={(size * 210) / 168}
  viewBox="0 0 168 210"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  class={klassen}
  style:--schild-versatz={zwinkert ? versatz : undefined}
  aria-hidden="true"
  focusable="false"
>
  <path d="M84 210C59.675 203.875 39.5938 189.919 23.7563 168.131C7.91875 146.344 0 122.15 0 95.55V31.5L84 0L168 31.5V95.55C168 122.15 160.081 146.344 144.244 168.131C128.406 189.919 108.325 203.875 84 210Z" fill="#CAA960" />
  <path d="M83.9999 184.471C65.6369 179.835 50.4775 169.272 38.5218 152.782C26.566 136.292 20.5881 117.98 20.5881 97.8477V49.3706L83.9999 25.5294L147.412 49.3706V97.8477C147.412 117.98 141.434 136.292 129.478 152.782C117.522 169.272 102.363 179.835 83.9999 184.471Z" fill="white" />
  <rect class="schild-auge" x="50.2354" y="56" width="19.7647" height="49.4118" rx="9.88235" fill="#CAA960" />
  <rect class="schild-auge" x="98" y="56" width="19.7647" height="49.4118" rx="9.88235" fill="#CAA960" />
</svg>
