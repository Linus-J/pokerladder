<script lang="ts">
  import { face } from '../lib/cards'

  /** A card like "As", or null for a face-down card. */
  let { card, size = 'md', dim = false }: { card: string | null; size?: 'sm' | 'md' | 'lg'; dim?: boolean } =
    $props()
  const f = $derived(card ? face(card) : null)
</script>

{#if f}
  <div class="card {size} suit-{f.suit}" class:dim role="img" aria-label={f.label}>
    <span class="rank">{f.rank}</span>
    <span class="pip">{f.symbol}</span>
  </div>
{:else}
  <div class="card back {size}" class:dim role="img" aria-label="face-down card"></div>
{/if}

<style>
  .card {
    --w: 3.4rem;
    width: var(--w);
    height: calc(var(--w) * 1.4);
    border-radius: calc(var(--w) * 0.09);
    background: var(--card);
    border: 1px solid rgba(0, 0, 0, 0.25);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: calc(var(--w) * 0.08) calc(var(--w) * 0.12);
    font-family: var(--serif);
    font-weight: 700;
    user-select: none;
    flex: none;
  }
  .sm {
    --w: 2.3rem;
  }
  .lg {
    --w: 4.4rem;
  }
  .rank {
    font-size: calc(var(--w) * 0.42);
    line-height: 1;
  }
  .pip {
    font-size: calc(var(--w) * 0.5);
    line-height: 1;
    align-self: flex-end;
  }
  .suit-s {
    color: var(--spade);
  }
  .suit-h {
    color: var(--heart);
  }
  .suit-d {
    color: var(--diamond);
  }
  .suit-c {
    color: var(--club);
  }
  /* A plain two-tone lattice, like a printed card back. */
  .back {
    background-color: var(--card-back);
    background-image:
      repeating-linear-gradient(45deg, var(--card-back-line) 0 2px, transparent 2px 9px),
      repeating-linear-gradient(-45deg, var(--card-back-line) 0 2px, transparent 2px 9px);
    border: calc(var(--w) * 0.07) solid var(--card);
  }
  .dim {
    opacity: 0.45;
    filter: saturate(0.5);
  }
</style>
