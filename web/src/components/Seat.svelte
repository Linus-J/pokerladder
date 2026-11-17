<script lang="ts">
  import Card from './Card.svelte'

  let {
    name,
    rating,
    stack,
    cards,
    faceDown,
    shown,
    mucked,
    dealer,
    allIn,
    winnings,
    acting,
    clockMs,
    clockMax,
    me = false,
  }: {
    name: string
    rating: number
    stack: number
    cards: string[] | null
    faceDown: boolean
    shown: string | null
    mucked: boolean
    dealer: boolean
    allIn: boolean
    winnings: number
    acting: boolean
    clockMs: number
    clockMax: number
    me?: boolean
  } = $props()

  const secs = $derived(Math.ceil(clockMs / 1000))
  const frac = $derived(clockMax > 0 ? Math.min(1, clockMs / clockMax) : 0)
</script>

<div class="seat" class:me class:acting data-testid={me ? 'seat-me' : 'seat-opponent'}>
  <div class="cards">
    {#if cards}
      {#each cards as c (c)}<Card card={c} size="lg" dim={mucked} />{/each}
    {:else if faceDown}
      <Card card={null} size="lg" /><Card card={null} size="lg" />
    {/if}
  </div>
  <div class="plate">
    {#if dealer}<span class="dealer" title="Dealer button (small blind)">D</span>{/if}
    <div class="who">
      <span class="name">{name}</span>
      <span class="rating num">{Math.round(rating)}</span>
    </div>
    <div class="stack num" data-testid={me ? 'my-stack' : 'opp-stack'}>
      {allIn && stack === 0 ? 'ALL-IN' : stack.toLocaleString()}
    </div>
    {#if acting}
      <div class="clock" class:low={secs <= 10} aria-label="{secs} seconds left">
        <div class="bar" style:width="{frac * 100}%"></div>
        <span class="num">{secs}s</span>
      </div>
    {/if}
  </div>
  {#if shown}<div class="shown">{shown}</div>{/if}
  {#if winnings > 0}<div class="won num">+{winnings.toLocaleString()}</div>{/if}
</div>

<style>
  .seat {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s2);
    position: relative;
  }
  .me {
    flex-direction: column-reverse;
  }
  .cards {
    display: flex;
    gap: var(--s1);
    min-height: 6.2rem;
    align-items: flex-end;
  }
  .me .cards {
    align-items: flex-start;
  }
  .plate {
    position: relative;
    min-width: 11rem;
    background: var(--plate);
    color: var(--plate-ink);
    border: 2px solid var(--line-strong);
    border-radius: var(--radius);
    padding: var(--s1) var(--s4) var(--s2);
    text-align: center;
  }
  .acting .plate {
    border-color: var(--acting);
    border-width: 3px;
    padding: calc(var(--s1) - 1px) calc(var(--s4) - 1px) calc(var(--s2) - 1px);
  }
  .who {
    display: flex;
    justify-content: center;
    gap: var(--s2);
    align-items: baseline;
  }
  .name {
    font-weight: 700;
  }
  .rating {
    font-size: var(--t-xs);
    color: var(--muted);
  }
  .stack {
    font-family: var(--serif);
    font-size: var(--t-lg);
    font-weight: 700;
  }
  .dealer {
    position: absolute;
    top: -0.7rem;
    right: -0.7rem;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 50%;
    background: var(--felt-ink);
    color: var(--spade);
    border: 2px solid var(--chip);
    font-size: var(--t-xs);
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .clock {
    position: relative;
    margin-top: var(--s1);
    height: 1.1rem;
    border-radius: 2px;
    background: var(--sunk);
    overflow: hidden;
    font-size: var(--t-xs);
    display: grid;
    place-items: center;
  }
  .clock .bar {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--good);
    opacity: 0.45;
    transition: width 0.2s linear;
  }
  .clock.low .bar {
    background: var(--bad);
  }
  .clock span {
    position: relative;
    font-weight: 700;
  }
  .shown {
    font-size: var(--t-sm);
    background: rgba(0, 0, 0, 0.3);
    padding: 0 var(--s3);
    border-radius: var(--radius);
  }
  .won {
    font-weight: 700;
    color: var(--chip-ink);
  }
</style>
