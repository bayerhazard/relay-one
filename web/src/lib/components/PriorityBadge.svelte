<script lang="ts">
  interface Props {
    intensity: number; // 0.0 - 1.0
    fraudScore?: number; // 0.0 - 1.0
  }

  let { intensity, fraudScore = 0 }: Props = $props();
</script>

{#if intensity > 0.7 && fraudScore < 0.3}
  <span
    class="priority-dot"
    title="Hohe Dringlichkeit"
  >
  </span>
{/if}

<style>
  .priority-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--color-danger);
    margin-right: 5px;
    flex-shrink: 0;
    transform: translateY(-1px);
  }
</style>
