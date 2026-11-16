<script lang="ts">
  import { game } from '../lib/game.svelte'
  import { raisePresets, type TableState } from '../lib/table'

  let { table }: { table: TableState } = $props()

  const legal = $derived(table.turn && table.turn.seat === table.mySeat ? table.turn.legal : null)
  const hand = $derived(table.hand)
  const facing = $derived(hand ? Math.max(hand.streetBets[0], hand.streetBets[1]) : 0)
  const presets = $derived(raisePresets(table))
  const lo = $derived(legal?.raise_min_to ?? 0)
  const hi = $derived(legal?.raise_max_to ?? 0)
  const canRaise = $derived(legal?.raise_min_to != null)

  let amount = $state(0)
  // Reset the slider to the minimum when a different decision arrives (by value: a
  // repeated turn message mustn't wipe what the player has dialled in). Within one
  // street a player only decides again after a raise, which changes `legal`.
  const decision = $derived(legal ? `${hand?.number}|${hand?.street}|${JSON.stringify(legal)}` : null)
  $effect(() => {
    if (decision !== null) amount = lo
  })
  const busy = $derived(game.pending)

  const target = $derived(Math.min(hi, Math.max(lo, Math.round(amount || 0))))
  const raiseLabel = $derived(
    target === hi ? 'All-in' : facing === 0 ? `Bet ${target.toLocaleString()}` : `Raise to ${target.toLocaleString()}`,
  )

  function fold() {
    game.act({ kind: 'fold' })
  }
  function passive() {
    if (!legal) return
    game.act(legal.can_check ? { kind: 'check' } : { kind: 'call' })
  }
  function raise() {
    if (!canRaise) return
    game.act(target === hi ? { kind: 'all_in' } : { kind: 'raise_to', amount: target })
  }

  function onkey(e: KeyboardEvent) {
    // A held key repeats: only the first press counts.
    if (!legal || busy || e.repeat) return
    if (e.target instanceof HTMLInputElement || e.metaKey || e.ctrlKey || e.altKey) return
    const k = e.key.toLowerCase()
    if (k === 'f' && legal.can_fold) fold()
    else if (k === 'c' && (legal.can_check || legal.call !== null)) passive()
    else if (k === 'r' && canRaise) raise()
    else return
    e.preventDefault()
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="bar" class:waiting={!legal} data-testid="action-bar">
  {#if legal}
    {#if canRaise}
      <div class="sizing">
        <div class="presets">
          {#each presets as p (p.label)}
            <button type="button" class:on={target === p.to} onclick={() => (amount = p.to)}>{p.label}</button>
          {/each}
        </div>
        <input
          type="range"
          min={lo}
          max={hi}
          step={hand?.bigBlind ? Math.max(1, Math.floor(hand.bigBlind / 2)) : 1}
          bind:value={amount}
          aria-label="Bet size"
        />
        <input class="amount num" type="number" min={lo} max={hi} bind:value={amount} aria-label="Bet amount" />
      </div>
    {/if}
    <div class="buttons">
      {#if legal.can_fold}
        <button type="button" class="fold" disabled={busy} onclick={fold}>Fold <kbd>F</kbd></button>
      {/if}
      <button type="button" class="call" disabled={busy} onclick={passive}>
        {legal.can_check ? 'Check' : `Call ${(legal.call ?? 0).toLocaleString()}`} <kbd>C</kbd>
      </button>
      {#if canRaise}
        <button type="button" class="primary raise" disabled={busy} onclick={raise}>{raiseLabel} <kbd>R</kbd></button>
      {/if}
    </div>
  {:else if table.winner === null}
    <p class="muted">
      {#if table.turn}Waiting for {table.names[table.turn.seat]}…{:else}Next hand shortly…{/if}
    </p>
  {/if}
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem 1.25rem;
    align-items: center;
    justify-content: flex-end;
    min-height: 3.5rem;
  }
  .waiting {
    justify-content: center;
  }
  .waiting p {
    margin: 0;
  }
  .sizing {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    flex: 1 1 22rem;
    flex-wrap: wrap;
  }
  .presets {
    display: flex;
    gap: 0.3rem;
  }
  .presets button {
    padding: 0.3em 0.65em;
    font-size: 0.8rem;
  }
  .presets button.on {
    border-color: var(--accent);
    color: var(--accent);
  }
  input[type='range'] {
    flex: 1 1 8rem;
    accent-color: var(--accent);
  }
  .amount {
    width: 6.5rem;
    text-align: right;
  }
  .buttons {
    display: flex;
    gap: 0.5rem;
  }
  .buttons button {
    min-width: 7.5rem;
    padding: 0.75em 1.1em;
  }
  .fold {
    color: #f3b0b0;
  }
  kbd {
    font-family: var(--mono);
    font-size: 0.65rem;
    opacity: 0.55;
    margin-left: 0.3em;
    border: 1px solid currentColor;
    border-radius: 3px;
    padding: 0 0.25em;
  }
  @media (max-width: 640px) {
    .buttons {
      width: 100%;
    }
    .buttons button {
      flex: 1;
      min-width: 0;
    }
    kbd {
      display: none;
    }
  }
</style>
