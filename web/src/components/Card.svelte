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
    border-radius: calc(var(--w) * 0.11);
    background: var(--card);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.6) inset,
      0 3px 8px rgba(0, 0, 0, 0.45);
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: calc(var(--w) * 0.08) calc(var(--w) * 0.12);
    font-weight: 800;
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
    letter-spacing: -0.04em;
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
  .back {
    background:
      repeating-linear-gradient(45deg, #8a1c22 0 6px, #a3262d 6px 12px);
    border: calc(var(--w) * 0.06) solid #f3eee2;
  }
  .dim {
    opacity: 0.45;
    filter: saturate(0.5);
  }
</style>
