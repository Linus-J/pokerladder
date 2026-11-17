<script lang="ts">
  import { LoaderCircle, MailCheck, RotateCw, ScrollText, Swords, Trophy, UserPlus } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import LeaderboardTable from '../components/LeaderboardTable.svelte'
  import { api } from '../lib/api'
  import { game, session } from '../lib/game.svelte'
  import type { MatchSummary, Standings } from '../lib/protocol'
  import { link, router } from '../lib/router.svelte'
  import { QUALIFYING_MATCHES, seasonName, seasonOf } from '../lib/season'

  let board = $state<Standings | null>(null)
  let boardFailed = $state(false)
  let recent = $state<MatchSummary[] | null>(null)
  let recentFailed = $state(false)
  let resent = $state<'idle' | 'sending' | 'sent' | 'error'>('idle')
  let now = $state(Date.now())

  const me = $derived(session.me)

  onMount(() => {
    api.leaderboard().then(
      (b) => (board = b),
      () => (boardFailed = true),
    )
    const id = setInterval(() => (now = Date.now()), 1000)
    return () => clearInterval(id)
  })

  // Recent matches, for whoever is logged in, refreshed when a match ends. Deriveds
  // only change when their value does, so table updates mid-hand don't refetch.
  const myId = $derived(session.me?.id ?? null)
  const winner = $derived(game.table?.winner ?? null)
  let fetchId = 0
  $effect(() => {
    void winner
    if (myId === null) return
    const id = ++fetchId
    api.matches().then(
      (m) => {
        if (id !== fetchId) return
        recent = m.slice(0, 5)
        recentFailed = false
      },
      () => {
        if (id === fetchId) recentFailed = true
      },
    )
  })

  const waited = $derived(Math.max(0, Math.floor((now - game.queuedAt) / 1000)))
  const inMatch = $derived(game.table !== null && game.table.winner === null)
  const mustVerify = $derived(!!me && me.verification_required && !me.email_verified)

  async function resend() {
    resent = 'sending'
    try {
      await api.resendVerification()
      resent = 'sent'
    } catch {
      resent = 'error'
    }
  }

  function when(iso: string) {
    return new Date(iso).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
  }
</script>

<div class="index">
  <div class="main">
    {#if !me}
      <section class="block intro">
        <div class="block-body">
          <h1>Heads-up hold'em, ranked.</h1>
          <p class="lead">
            Play one-on-one No-Limit Hold'em against someone near your rating. Every match moves your
            rating up or down, and the best players top the ladder. It's free to play, and it stays free.
          </p>
          <div class="actions">
            <a class="button primary big" href="/register" onclick={link}><Icon of={UserPlus} />Create a free account</a>
            <span class="muted">Already have one? <a href="/login" onclick={link}>Log in</a></span>
          </div>
        </div>
      </section>
    {:else}
      <section class="block">
        <header><Icon of={Swords} /><h2>Play</h2></header>
        <div class="block-body">
          <h1>Welcome back, {me.name}</h1>
          <p class="muted">
            Your {seasonName(seasonOf(new Date(now)))} rating is <b class="num">{Math.round(me.rating)}</b>.
            {#if me.season_matches >= me.qualifying_matches}
              You're ranked, with {me.season_matches} rated matches this season.
            {:else if me.season_matches === 0}
              Play {me.qualifying_matches} rated matches this month to be ranked.
            {:else}
              {me.season_matches} of the {me.qualifying_matches} rated matches you need to be ranked this month.
            {/if}
          </p>

          {#if mustVerify}
            <div class="note">
              <Icon of={MailCheck} />
              <div>
                Confirm your email address before your first match. The link is in your inbox (check spam
                if it isn't there).
                {#if resent === 'sent'}
                  <b>We've sent a new link.</b>
                {:else}
                  <button type="button" class="link" disabled={resent === 'sending'} onclick={resend}>
                    {resent === 'sending' ? 'Sending...' : 'Send the link again'}
                  </button>
                  {#if resent === 'error'}<span class="bad">That didn't send. Try again in a minute.</span>{/if}
                {/if}
              </div>
            </div>
          {/if}

          <div class="queue" data-testid="queue">
            {#if inMatch}
              <button type="button" class="primary big" onclick={() => router.go('/play')}>
                Back to your match
              </button>
            {:else if game.queued}
              <span class="searching">
                <Icon of={LoaderCircle} spin />
                Looking for an opponent near your rating... <span class="muted num">{waited}s</span>
              </span>
              <button type="button" onclick={() => game.leaveQueue()}>Stop looking</button>
            {:else}
              <button
                type="button"
                class="primary big"
                disabled={game.status !== 'online' || mustVerify || game.restarting}
                onclick={() => game.joinQueue()}
              >
                Find a match
              </button>
              {#if game.restarting}
                <span class="muted"><Icon of={RotateCw} /> New matches are paused while the server restarts.</span>
              {:else if game.status !== 'online'}
                <span class="muted">Connecting to the game server...</span>
              {:else if !mustVerify}
                <span class="muted small">You'll be paired with the closest-rated player who is waiting.</span>
              {/if}
            {/if}
          </div>
        </div>
      </section>

      <section class="block">
        <header>
          <Icon of={ScrollText} /><h2>Your recent matches</h2>
          {#if recent && recent.length > 0}<a class="aside" href="/history" onclick={link}>All your matches</a>{/if}
        </header>
        {#if recentFailed}
          <p class="block-body">Your matches didn't load. Reload the page to try again.</p>
        {:else if !recent}
          <div class="block-body"><span class="skeleton" style:width="60%"></span></div>
        {:else if recent.length === 0}
          <p class="block-body muted">
            Nothing here yet. After your first match you'll see the result, the rating change and every hand
            you played.
          </p>
        {:else}
          <table class="list">
            <tbody>
              {#each recent as m (m.id)}
                {@const d = m.rating_after !== null && m.rating_before !== null ? m.rating_after - m.rating_before : null}
                <tr>
                  <td class="date muted">{when(m.started_at)}</td>
                  <td><a href="/history/{m.id}" onclick={link}>vs {m.opponent}</a></td>
                  <td>
                    {#if m.status === 'in_progress'}<span class="muted">In progress</span>
                    {:else if m.status === 'abandoned'}<span class="muted">Abandoned</span>
                    {:else if m.won}<span class="good">Won</span>
                    {:else}<span class="bad">Lost</span>{/if}
                  </td>
                  <td class="r num">
                    {#if d !== null}<span class={d >= 0 ? 'good' : 'bad'}>{d >= 0 ? '+' : ''}{d.toFixed(1)}</span>{:else if m.status === 'finished' && !m.rated}<span class="muted">Unrated</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </section>
    {/if}

    <section class="block">
      <header><h2>How a match works</h2><a class="aside" href="/about" onclick={link}>Full rules</a></header>
      <dl class="block-body facts">
        <dt>Format</dt>
        <dd>One opponent, No-Limit Hold'em, played until one of you has all the chips.</dd>
        <dt>Chips</dt>
        <dd>1,500 each. Blinds start at 10/20 and rise every 10 hands.</dd>
        <dt>Clock</dt>
        <dd>15 seconds a decision, plus a 60 second time bank that tops up as you play.</dd>
        <dt>Opponents</dt>
        <dd>Chosen by the server from players near your rating. You can't pick.</dd>
        <dt>Rating</dt>
        <dd>Everyone starts each month at 1500. Win and it goes up, lose and it goes down; upsets move it more.</dd>
        <dt>Season</dt>
        <dd>One calendar month. Play {QUALIFYING_MATCHES} rated matches to be ranked.</dd>
        <dt>Cost</dt>
        <dd>Nothing, ever. Sponsor-funded prizes for the top of the season are on the way.</dd>
      </dl>
    </section>
  </div>

  <aside class="side">
    <section class="block">
      <header>
        <Icon of={Trophy} /><h2>Leaderboard</h2>
        <a class="aside" href="/leaderboard" onclick={link}>Full standings</a>
      </header>
      <LeaderboardTable standings={board} failed={boardFailed} limit={10} compact me={me?.name} />
    </section>
  </aside>
</div>

<style>
  .index {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20rem;
    gap: var(--s5);
    align-items: start;
  }
  .main,
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--s5);
  }
  .intro .block-body {
    padding: var(--s6);
  }
  .lead {
    font-size: var(--t-md);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s4);
    margin-top: var(--s5);
  }
  .queue {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s3) var(--s4);
    align-items: center;
    margin-top: var(--s5);
    min-height: 2.75rem;
  }
  .searching {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
  }
  .note button {
    margin-left: var(--s1);
  }
  .date {
    width: 5.5rem;
    white-space: nowrap;
  }
  .facts {
    display: grid;
    grid-template-columns: 7rem minmax(0, 1fr);
    gap: var(--s2) var(--s4);
    margin: 0;
  }
  .facts dt {
    font-weight: 700;
  }
  .facts dd {
    margin: 0;
  }
  @media (max-width: 860px) {
    .index {
      grid-template-columns: 1fr;
    }
    .intro .block-body {
      padding: var(--s5) var(--s4);
    }
  }
  @media (max-width: 480px) {
    .facts {
      grid-template-columns: 1fr;
      gap: 0;
    }
    .facts dd {
      margin-bottom: var(--s2);
    }
  }
</style>
