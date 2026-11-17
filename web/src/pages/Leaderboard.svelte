<script lang="ts">
  import { Trophy } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import LeaderboardTable from '../components/LeaderboardTable.svelte'
  import { api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import type { Standings } from '../lib/protocol'
  import { link } from '../lib/router.svelte'
  import { PAIR_DAILY_LIMIT, seasonName, timeLeft } from '../lib/season'

  let standings = $state<Standings | null>(null)
  let failed = $state(false)
  let seasons = $state<string[]>([])
  /** The season picked; '' for the current one. */
  let picked = $state('')
  let loadId = 0

  function load(season: string) {
    const id = ++loadId
    standings = null
    failed = false
    api.leaderboard(season || undefined).then(
      (s) => {
        if (id === loadId) standings = s
      },
      () => {
        if (id === loadId) failed = true
      },
    )
  }

  onMount(() => {
    load('')
    // Only for the picker: without it the page still works.
    api.seasons().then(
      (s) => (seasons = s),
      () => {},
    )
  })

  const need = $derived(standings?.qualifying_matches ?? 20)
</script>

<div class="page">
  <section class="block">
    <header>
      <Icon of={Trophy} /><h1>Leaderboard</h1>
      {#if seasons.length > 1}
        <label class="aside pick">
          <span class="muted">Season</span>
          <select
            bind:value={picked}
            onchange={() => load(picked)}
            data-testid="season-picker"
          >
            {#each seasons as s, i (s)}
              <option value={i === 0 ? '' : s}>{seasonName(s)}</option>
            {/each}
          </select>
        </label>
      {/if}
    </header>
    {#if standings}
      <p class="season small">
        <b>{seasonName(standings.season)}</b>
        {#if standings.current}
          <span class="muted">season, {timeLeft(standings.season, new Date())} (midnight UTC).</span>
        {:else}
          <span class="muted">season, final standings.</span>
        {/if}
      </p>
    {/if}
    <LeaderboardTable {standings} {failed} me={session.me?.name} />
  </section>

  <aside class="block">
    <header><h2>How the season works</h2></header>
    <div class="block-body small">
      <p>
        Each calendar month is a season. Everyone starts it at 1500 and the ratings from last month
        don't carry over.
      </p>
      <p>
        Play {need} rated matches to be ranked. Players are ranked by their <b>score</b>: their rating
        minus twice how unsure the system still is about it. Playing more makes it surer, and the
        uncertainty creeps back up if you stop playing.
      </p>
      <p>
        Only the first {PAIR_DAILY_LIMIT} matches against the same opponent in any 24 hours are rated.
        You're told at the table when a match isn't.
      </p>
      <p><a href="/about#ratings" onclick={link}>More about ratings and seasons</a></p>
    </div>
  </aside>
</div>

<style>
  .page {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 17rem;
    gap: var(--s5);
    align-items: start;
  }
  @media (max-width: 760px) {
    .page {
      grid-template-columns: 1fr;
    }
  }
  .pick {
    display: flex;
    align-items: center;
    gap: var(--s2);
    margin: 0;
    font-weight: 400;
  }
  .season {
    margin: 0;
    padding: var(--s3) var(--s4) 0;
  }
</style>
