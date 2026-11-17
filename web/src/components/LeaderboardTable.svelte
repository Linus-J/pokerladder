<script lang="ts">
  import type { LeaderboardRow } from '../lib/protocol'

  let {
    rows,
    failed = false,
    limit,
    me,
  }: {
    /** null while loading. */
    rows: LeaderboardRow[] | null
    failed?: boolean
    limit?: number
    me?: string
  } = $props()

  const shown = $derived(rows && limit !== undefined ? rows.slice(0, limit) : rows)
</script>

{#if failed}
  <p class="state">The leaderboard didn't load. Reload the page to try again.</p>
{:else if !shown}
  <table class="list" aria-busy="true" aria-label="Leaderboard loading">
    <tbody>
      {#each [70, 55, 62, 48, 58] as w, i (i)}
        <tr>
          <td class="rank"><span class="skeleton" style:width="1.2em"></span></td>
          <td><span class="skeleton" style:width="{w}%"></span></td>
          <td class="r"><span class="skeleton" style:width="2.5em"></span></td>
        </tr>
      {/each}
    </tbody>
  </table>
{:else if shown.length === 0}
  <div class="state">
    <p><b>Nobody is ranked yet.</b></p>
    <p class="muted">
      Players appear here after their first rated match. Win or lose, finishing a match puts you on
      the board.
    </p>
  </div>
{:else}
  <table class="list" data-testid="leaderboard">
    <thead>
      <tr><th class="rank">#</th><th>Player</th><th class="r">Rating</th><th class="r">Matches</th></tr>
    </thead>
    <tbody>
      {#each shown as row, i (row.name)}
        <tr class:mine={row.name === me}>
          <td class="rank num muted">{i + 1}</td>
          <td>{row.name}{#if row.name === me}<span class="you"> (you)</span>{/if}</td>
          <td class="r num">{Math.round(row.rating)}</td>
          <td class="r num muted">{row.rated_matches}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .state {
    padding: var(--s4);
    margin: 0;
  }
  .state p:last-child {
    margin-bottom: 0;
  }
  .rank {
    width: 2.5rem;
  }
  .you {
    font-weight: 400;
    color: var(--muted);
  }
</style>
