<script lang="ts">
  interface Props {
    summary: string;
  }

  let { summary }: Props = $props();
</script>

<p class="summary-line">{summary}</p>

<style>
  .summary-line {
    font-size: 0.625rem;
    color: var(--color-text-secondary);
    margin-top: 2px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.3;
  }
</style>
