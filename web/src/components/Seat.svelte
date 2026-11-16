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
    gap: 0.45rem;
    position: relative;
  }
  .me {
    flex-direction: column-reverse;
  }
  .cards {
    display: flex;
    gap: 0.35rem;
    min-height: 6.2rem;
    align-items: flex-end;
  }
  .me .cards {
    align-items: flex-start;
  }
  .plate {
    position: relative;
    min-width: 11rem;
    background: rgba(10, 14, 12, 0.88);
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 0.45rem 0.9rem 0.5rem;
    text-align: center;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4);
    transition:
      border-color 0.15s,
      box-shadow 0.15s;
  }
  .acting .plate {
    border-color: var(--accent);
    box-shadow:
      0 0 0 1px var(--accent),
      0 4px 18px rgba(227, 179, 65, 0.25);
  }
  .who {
    display: flex;
    justify-content: center;
    gap: 0.5rem;
    align-items: baseline;
  }
  .name {
    font-weight: 700;
  }
  .rating {
    font-size: 0.75rem;
    color: var(--muted);
  }
  .stack {
    font-size: 1.15rem;
    font-weight: 700;
    color: var(--accent);
  }
  .dealer {
    position: absolute;
    top: -0.6rem;
    right: -0.6rem;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 50%;
    background: #f5f1e6;
    color: #222;
    font-size: 0.75rem;
    font-weight: 800;
    display: grid;
    place-items: center;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.5);
  }
  .clock {
    position: relative;
    margin-top: 0.35rem;
    height: 1.1rem;
    border-radius: 6px;
    background: #222c27;
    overflow: hidden;
    font-size: 0.7rem;
    display: grid;
    place-items: center;
  }
  .clock .bar {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--good);
    opacity: 0.55;
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
    font-size: 0.8rem;
    color: var(--text);
    background: rgba(0, 0, 0, 0.35);
    padding: 0.1rem 0.6rem;
    border-radius: 999px;
  }
  .won {
    font-weight: 800;
    color: var(--good);
  }
</style>
