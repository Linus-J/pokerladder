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
  <section class="block not-seated">
    <header><h1 class="h">You're not at a table</h1></header>
    <div class="block-body">
      <p>Matches start from the lobby. Once you're paired with an opponent, this page shows the table.</p>
      <p><a href="/" onclick={link}>Go to the lobby</a></p>
    </div>
  </section>
{:else}
  {@const opp = 1 - t.mySeat}
  {@const h = t.hand}
  <div class="layout">
    <section class="main">
      <div class="info">
        <span class="vs"><b>{t.names[t.mySeat]}</b> vs <b>{t.names[opp]}</b></span>
        {#if h}
          <span>Hand <b class="num">{h.number}</b></span>
          <span>Level <b class="num">{h.level}</b></span>
          <span>Blinds <b class="num">{h.smallBlind}/{h.bigBlind}</b></span>
        {:else}
          <span class="muted">Shuffling up for the first hand...</span>
        {/if}
        <label class="muck">
          <input
            type="checkbox"
            checked={t.autoMuck}
            onchange={(e) => {
              const box = e.currentTarget
              if (!game.setAutoMuck(box.checked)) box.checked = !box.checked
            }}
          />
          Don't show my losing hands
        </label>
      </div>

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
          <div class="pot num" class:empty={!h || pot(h) === 0} data-testid="pot">
            {#if h && pot(h) > 0}Pot {pot(h).toLocaleString()}{:else}&nbsp;{/if}
          </div>
        </div>

        <div class="slot bottom">
          {#if h && h.streetBets[t.mySeat] > 0}<div class="bet num">{h.streetBets[t.mySeat].toLocaleString()}</div>{/if}
          <Seat {...seatProps(t.mySeat)} />
        </div>
      </div>

      <div class="block actions">
        <ActionBar table={t} />
      </div>
    </section>

    <aside class="block log-block" aria-labelledby="log-title">
      <header><h2 id="log-title">Hand log</h2></header>
      <div class="log" bind:this={logEl} aria-live="polite">
        {#each t.log as l, i (i)}
          <div class:sep={l.startsWith('Hand #')}>{l}</div>
        {:else}
          <p class="none">Every bet, card and result will be written here as the hands are played.</p>
        {/each}
      </div>
    </aside>
  </div>

  {#if t.winner !== null}
    {@const won = t.winner === t.mySeat}
    {@const r = t.result?.[t.mySeat]}
    <div class="overlay" role="dialog" aria-modal="true" aria-labelledby="result-title">
      <div class="block result" data-testid="result">
        <header><h1 id="result-title" class="h">{won ? 'You won the match' : 'You lost the match'}</h1></header>
        <div class="block-body">
          <p>
            {t.names[t.winner]} took all the chips after {t.handsPlayed}
            {t.handsPlayed === 1 ? 'hand' : 'hands'}.
          </p>
          {#if r}
            {@const delta = r.after - r.before}
            <p class="rating num">
              Rating {Math.round(r.before)} → <b>{Math.round(r.after)}</b>
              <span class={delta >= 0 ? 'good' : 'bad'}>({delta >= 0 ? '+' : ''}{delta.toFixed(1)})</span>
            </p>
          {:else}
            <p class="muted">Saving the result and updating ratings...</p>
          {/if}
          <div class="row">
            <button type="button" class="primary" disabled={game.restarting} onclick={playAgain}>Find another match</button>
            <button type="button" onclick={lobby}>Back to the lobby</button>
          </div>
          {#if game.restarting}
            <p class="muted small restart">New matches are paused while the server restarts.</p>
          {/if}
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .not-seated {
    max-width: 34rem;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 18rem;
    gap: var(--s4);
    align-items: start;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }
  .info {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2) var(--s5);
    align-items: center;
    padding: var(--s2) var(--s4);
    background: var(--sunk);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    font-size: var(--t-sm);
  }
  .muck {
    margin: 0 0 0 auto;
    display: flex;
    gap: var(--s2);
    align-items: center;
    font-weight: 400;
  }
  .felt {
    position: relative;
    border-radius: 50% / 38%;
    background: var(--felt);
    border: 14px solid var(--rail);
    box-shadow: inset 0 0 0 2px var(--felt-line);
    min-height: 30rem;
    display: grid;
    grid-template-rows: auto 1fr auto;
    justify-items: center;
    padding: var(--s4);
    color: var(--felt-ink);
  }
  .slot {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s2);
  }
  .centre {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s3);
  }
  .board {
    display: flex;
    gap: var(--s2);
  }
  .ghost {
    width: 3.4rem;
    height: 4.76rem;
    border-radius: 0.3rem;
    border: 1px dashed var(--felt-line);
  }
  .pot {
    font-weight: 700;
    background: rgba(0, 0, 0, 0.25);
    padding: 0 var(--s4);
    border-radius: var(--radius);
  }
  .pot.empty {
    background: none;
  }
  .log .none {
    font-family: var(--font);
    color: var(--muted);
  }
  .bet {
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--chip);
    color: var(--chip-ink);
    padding: 0 var(--s3);
    border-radius: var(--radius);
    font-weight: 700;
    font-size: var(--t-sm);
  }
  .actions {
    padding: var(--s3) var(--s4);
  }
  .log {
    height: 38rem;
    overflow-y: auto;
    font-family: var(--mono);
    font-size: var(--t-xs);
    line-height: 1.55;
    padding: var(--s3) var(--s4);
  }
  .log .sep {
    font-weight: 700;
    margin-top: var(--s3);
    padding-top: var(--s2);
    border-top: 1px solid var(--line);
  }
  .log .sep:first-child {
    margin-top: 0;
    border-top: none;
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(20, 14, 12, 0.6);
    display: grid;
    place-items: center;
    z-index: 10;
    padding: var(--s4);
  }
  .result {
    width: min(26rem, 100%);
  }
  .result .rating {
    font-size: var(--t-md);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s3);
    margin-top: var(--s4);
  }
  .restart {
    margin-top: var(--s3);
  }
  @media (max-width: 960px) {
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
      padding: var(--s3) var(--s1);
    }
  }
</style>
