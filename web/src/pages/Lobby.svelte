<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import { game, session } from '../lib/game.svelte'
  import type { LeaderboardRow } from '../lib/protocol'
  import { link, router } from '../lib/router.svelte'

  let board = $state<LeaderboardRow[] | null>(null)
  let boardError = $state(false)
  let resent = $state<'idle' | 'sending' | 'sent' | 'error'>('idle')
  let now = $state(Date.now())

  onMount(() => {
    api.leaderboard().then(
      (b) => (board = b),
      () => (boardError = true),
    )
    const id = setInterval(() => (now = Date.now()), 1000)
    return () => clearInterval(id)
  })

  const waited = $derived(Math.max(0, Math.floor((now - game.queuedAt) / 1000)))
  const me = $derived(session.me)
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
</script>

<div class="grid">
  <section class="hero panel">
    {#if !me}
      <h1>Heads-up hold'em, ranked.</h1>
      <p class="lead">
        Play one-on-one No-Limit Hold'em sit-and-gos against opponents near your rating. Climb the
        monthly ladder — <b>free to enter</b>, with sponsor-funded prizes.
      </p>
      <div class="row">
        <a class="btn primary" href="/register" onclick={link}>Create account</a>
        <a class="btn" href="/login" onclick={link}>Log in</a>
      </div>
    {:else}
      <h1>Welcome back, {me.name}</h1>
      <p class="muted">
        Rating <b class="num rating">{Math.round(me.rating)}</b> · 1,500 chips each, blinds rise every
        10 hands, 15 s per decision plus a time bank.
      </p>

      {#if mustVerify}
        <div class="notice">
          Confirm your email address to start playing — check your inbox for the link.
          {#if resent === 'sent'}
            <span class="good">Sent a new link.</span>
          {:else}
            <button type="button" class="link" disabled={resent === 'sending'} onclick={resend}>
              Send it again
            </button>
            {#if resent === 'error'}<span class="error">Couldn't send; try again shortly.</span>{/if}
          {/if}
        </div>
      {/if}

      <div class="queue" data-testid="queue">
        {#if inMatch}
          <button type="button" class="primary big" onclick={() => router.go('/play')}>
            Return to your match
          </button>
        {:else if game.queued}
          <div class="searching">
            <span class="spinner" aria-hidden="true"></span>
            <span>Finding an opponent… <span class="muted num">{waited}s</span></span>
          </div>
          <button type="button" onclick={() => game.leaveQueue()}>Cancel</button>
        {:else}
          <button
            type="button"
            class="primary big"
            disabled={game.status !== 'online' || mustVerify || game.restarting}
            onclick={() => game.joinQueue()}
          >
            Find a match
          </button>
          {#if game.status !== 'online'}<span class="muted">Connecting…</span>{/if}
        {/if}
      </div>
    {/if}
  </section>

  <section class="panel">
    <h2>Leaderboard</h2>
    {#if boardError}
      <p class="muted">Couldn't load the leaderboard.</p>
    {:else if !board}
      <p class="muted">Loading…</p>
    {:else if board.length === 0}
      <p class="muted">No rated matches yet — be the first.</p>
    {:else}
      <table class="list" data-testid="leaderboard">
        <thead><tr><th>#</th><th>Player</th><th class="r">Rating</th><th class="r">Matches</th></tr></thead>
        <tbody>
          {#each board as row, i (row.name)}
            <tr class:mine={row.name === me?.name}>
              <td class="num muted">{i + 1}</td>
              <td>{row.name}</td>
              <td class="r num">{Math.round(row.rating)}</td>
              <td class="r num muted">{row.rated_matches}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
    gap: 1rem;
    align-items: start;
  }
  .hero {
    padding: 2rem;
    background:
      radial-gradient(ellipse at 100% 0%, rgba(19, 87, 58, 0.55), transparent 60%),
      var(--panel);
  }
  .lead {
    font-size: 1.1rem;
    color: #cfd9d2;
    max-width: 34rem;
  }
  .row {
    display: flex;
    gap: 0.75rem;
    margin-top: 1.5rem;
  }
  .btn {
    display: inline-block;
    border: 1px solid var(--line);
    background: var(--panel-2);
    border-radius: 8px;
    padding: 0.6em 1.3em;
    font-weight: 600;
    color: var(--text);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .btn:hover {
    text-decoration: none;
    filter: brightness(1.08);
  }
  .rating {
    color: var(--accent);
  }
  .notice {
    margin: 1rem 0;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    border: 1px solid rgba(227, 179, 65, 0.4);
    background: rgba(227, 179, 65, 0.08);
  }
  .notice button {
    margin-left: 0.4rem;
  }
  .queue {
    display: flex;
    gap: 1rem;
    align-items: center;
    margin-top: 1.5rem;
    min-height: 3.2rem;
  }
  .big {
    font-size: 1.1rem;
    padding: 0.75em 1.8em;
  }
  .searching {
    display: flex;
    gap: 0.6rem;
    align-items: center;
  }
  .spinner {
    width: 1.1rem;
    height: 1.1rem;
    border-radius: 50%;
    border: 2px solid var(--line);
    border-top-color: var(--accent);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  tr.mine td {
    color: var(--accent);
  }
  @media (max-width: 800px) {
    .grid {
      grid-template-columns: 1fr;
    }
    .hero {
      padding: 1.25rem;
    }
  }
</style>
