<script lang="ts">
  import { onMount } from 'svelte'
  import ActionBar from '../components/ActionBar.svelte'
  import Card from '../components/Card.svelte'
  import Seat from '../components/Seat.svelte'
  import { game } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'
  import { pot, remainingMs } from '../lib/table'

  const t = $derived(game.table)

  // A clock for the countdowns.
  let now = $state(performance.now())
  onMount(() => {
    const id = setInterval(() => (now = performance.now()), 200)
    return () => clearInterval(id)
  })

  let autoMuck = $state(true)
  let logEl: HTMLElement | undefined = $state()
  $effect(() => {
    void t?.log.length
    if (logEl) logEl.scrollTop = logEl.scrollHeight
  })

  function seatProps(seat: number) {
    const tt = t!
    const h = tt.hand
    const me = seat === tt.mySeat
    const acting = tt.turn?.seat === seat
    return {
      name: tt.names[seat],
      rating: tt.ratings[seat],
      stack: h ? h.stacks[seat] : tt.stacks[seat],
      cards: h ? h.cards[seat] : null,
      faceDown: !!h && !me && !h.mucked[seat] && !h.complete,
      shown: h?.shown[seat] ?? null,
      mucked: h?.mucked[seat] ?? false,
      dealer: h?.button === seat,
      allIn: h?.allIn[seat] ?? false,
      winnings: h?.winnings[seat] ?? 0,
      acting,
      clockMs: acting ? remainingMs(tt, now) : 0,
      clockMax: tt.perActionMs + tt.timeBanksMs[seat],
      me,
    }
  }

  function playAgain() {
    game.dismiss()
    game.joinQueue()
    router.go('/')
  }
  function lobby() {
    game.dismiss()
    router.go('/')
  }
</script>

{#if !t}
  <div class="empty panel">
    <p>You're not in a match.</p>
    <a href="/" onclick={link}>Back to the lobby</a>
  </div>
{:else}
  {@const opp = 1 - t.mySeat}
  {@const h = t.hand}
  <div class="layout">
    <section class="main">
      <header class="info">
        {#if h}
          <span>Hand <b class="num">#{h.number}</b></span>
          <span>Level <b class="num">{h.level}</b></span>
          <span>Blinds <b class="num">{h.smallBlind}/{h.bigBlind}</b></span>
        {:else}
          <span>Match #{t.matchId} — starting…</span>
        {/if}
        <label class="muck">
          <input
            type="checkbox"
            bind:checked={autoMuck}
            onchange={() => game.setAutoMuck(autoMuck)}
          />
          Muck losing hands
        </label>
      </header>

      <div class="felt" data-testid="table">
        <div class="slot top">
          <Seat {...seatProps(opp)} />
          {#if h && h.streetBets[opp] > 0}<div class="bet num">{h.streetBets[opp].toLocaleString()}</div>{/if}
        </div>

        <div class="centre">
          <div class="board" data-testid="board">
            {#each [0, 1, 2, 3, 4] as i (i)}
              {#if h?.board[i]}
                <Card card={h.board[i]} size="md" />
              {:else}
                <div class="ghost"></div>
              {/if}
            {/each}
          </div>
          <div class="pot num" data-testid="pot">
            {#if h && pot(h) > 0}Pot {pot(h).toLocaleString()}{:else}&nbsp;{/if}
          </div>
        </div>

        <div class="slot bottom">
          {#if h && h.streetBets[t.mySeat] > 0}<div class="bet num">{h.streetBets[t.mySeat].toLocaleString()}</div>{/if}
          <Seat {...seatProps(t.mySeat)} />
        </div>
      </div>

      <div class="actions panel">
        <ActionBar table={t} />
      </div>
    </section>

    <aside class="log panel" bind:this={logEl} aria-label="Hand log" aria-live="polite">
      {#each t.log as l, i (i)}
        <div class:sep={l.startsWith('—')}>{l}</div>
      {/each}
    </aside>
  </div>

  {#if t.winner !== null}
    {@const won = t.winner === t.mySeat}
    {@const r = t.result?.[t.mySeat]}
    <div class="overlay" role="dialog" aria-modal="true" aria-labelledby="result-title">
      <div class="result panel" data-testid="result">
        <h1 id="result-title" class={won ? 'good' : 'error'}>{won ? 'You won!' : 'You lost'}</h1>
        <p class="muted">
          {t.names[t.winner]} took all the chips after {t.handsPlayed} {t.handsPlayed === 1 ? 'hand' : 'hands'}.
        </p>
        {#if r}
          {@const delta = r.after - r.before}
          <p class="rating num">
            Rating {Math.round(r.before)} → <b>{Math.round(r.after)}</b>
            <span class={delta >= 0 ? 'good' : 'error'}>({delta >= 0 ? '+' : ''}{delta.toFixed(1)})</span>
          </p>
        {:else}
          <p class="muted">Recording the result…</p>
        {/if}
        <div class="row">
          <button type="button" onclick={lobby}>Lobby</button>
          <button type="button" class="primary" onclick={playAgain}>Play again</button>
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .empty {
    max-width: 24rem;
    margin: 4rem auto;
    text-align: center;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 17rem;
    gap: 1rem;
    align-items: start;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .info {
    display: flex;
    gap: 1.25rem;
    align-items: center;
    color: var(--muted);
    font-size: 0.9rem;
  }
  .info b {
    color: var(--text);
  }
  .muck {
    margin: 0 0 0 auto;
    display: flex;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
  }
  .felt {
    position: relative;
    border-radius: 50% / 38%;
    background: radial-gradient(ellipse at 50% 45%, #1b7a50 0%, var(--felt) 55%, var(--felt-edge) 100%);
    border: 14px solid var(--rail);
    box-shadow:
      inset 0 0 40px rgba(0, 0, 0, 0.45),
      0 10px 30px rgba(0, 0, 0, 0.5);
    min-height: 30rem;
    display: grid;
    grid-template-rows: auto 1fr auto;
    justify-items: center;
    padding: 1rem 1rem;
  }
  .slot {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }
  .centre {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.6rem;
  }
  .board {
    display: flex;
    gap: 0.4rem;
  }
  .ghost {
    width: 3.4rem;
    height: 4.76rem;
    border-radius: 0.37rem;
    border: 1px dashed rgba(255, 255, 255, 0.14);
  }
  .pot {
    font-weight: 700;
    background: rgba(0, 0, 0, 0.3);
    padding: 0.15rem 0.9rem;
    border-radius: 999px;
  }
  .bet {
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid rgba(227, 179, 65, 0.6);
    color: var(--accent);
    padding: 0.05rem 0.7rem;
    border-radius: 999px;
    font-weight: 700;
    font-size: 0.9rem;
  }
  .actions {
    padding: 0.75rem 1rem;
  }
  .log {
    height: 41rem;
    overflow-y: auto;
    font-size: 0.82rem;
    padding: 0.75rem 1rem;
    color: #c9d4cd;
  }
  .log .sep {
    color: var(--muted);
    margin-top: 0.6rem;
    font-weight: 600;
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 10;
    padding: 1rem;
  }
  .result {
    text-align: center;
    min-width: 20rem;
  }
  .result .rating {
    font-size: 1.1rem;
  }
  .row {
    display: flex;
    gap: 0.75rem;
    justify-content: center;
    margin-top: 1.25rem;
  }
  @media (max-width: 900px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .log {
      height: 12rem;
    }
  }
  @media (max-width: 640px) {
    .felt {
      border-width: 8px;
      border-radius: 28px;
      min-height: 26rem;
      padding: 0.75rem 0.25rem;
    }
    .info {
      gap: 0.75rem;
      flex-wrap: wrap;
    }
  }
</style>
