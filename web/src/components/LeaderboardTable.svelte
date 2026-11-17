<script lang="ts">
  import type { Standings } from '../lib/protocol'

  let {
    standings,
    failed = false,
    limit,
    compact = false,
    me,
  }: {
    /** null while loading. */
    standings: Standings | null
    failed?: boolean
    /** Rows to show of each list (ranked, still qualifying). */
    limit?: number
    /** Fewer columns, for a sidebar. */
    compact?: boolean
    me?: string
  } = $props()

  const cut = <T,>(rows: T[]) => (limit === undefined ? rows : rows.slice(0, limit))
  const ranked = $derived(standings ? cut(standings.rows.filter((r) => r.qualified)) : [])
  // The sidebar shows only the closest few to qualifying.
  const qualifying = $derived(
    standings ? cut(standings.rows.filter((r) => !r.qualified)).slice(0, compact ? 5 : undefined) : [],
  )
  const need = $derived(standings?.qualifying_matches ?? 20)
</script>

{#if failed}
  <p class="state">The standings didn't load. Reload the page to try again.</p>
{:else if !standings}
  <table class="list" aria-busy="true" aria-label="Standings loading">
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
{:else if standings.rows.length === 0}
  <div class="state">
    <p><b>Nobody has played a rated match {standings.current ? 'this season yet' : 'that season'}.</b></p>
    {#if standings.current}
      <p class="muted">
        Every rated match puts both players on this page. After {need} of them you're ranked.
      </p>
    {/if}
  </div>
{:else}
  <table class="list" data-testid="leaderboard">
    <thead>
      <tr>
        <th class="rank">#</th><th>Player</th>
        {#if !compact}<th class="r">Rating</th>{/if}
        <th class="r" title="Rating minus twice its uncertainty">Score</th>
        {#if !compact}<th class="r wide">Won</th>{/if}
      </tr>
    </thead>
    <tbody>
      {#each ranked as row, i (row.name)}
        <tr class:mine={row.name === me}>
          <td class="rank num muted">{i + 1}</td>
          <td>{row.name}{#if row.name === me}<span class="you"> (you)</span>{/if}</td>
          {#if !compact}
            <td class="r num nowrap">{Math.round(row.rating)} <span class="muted small">±{Math.round(row.rd)}</span></td>
          {/if}
          <td class="r num"><b>{Math.round(row.score)}</b></td>
          {#if !compact}<td class="r num muted wide">{row.wins}/{row.matches}</td>{/if}
        </tr>
      {:else}
        <tr>
          <td colspan={compact ? 3 : 5} class="muted">
            Nobody has qualified yet. Players are ranked once they've played {need} rated matches this
            season.
          </td>
        </tr>
      {/each}
    </tbody>
    {#if qualifying.length > 0}
      <tbody data-testid="qualifying">
        <tr class="group">
          <th colspan={compact ? 2 : 3}>Still qualifying</th>
          <th class="r" colspan={compact ? 1 : 2}>Matches</th>
        </tr>
        {#each qualifying as row (row.name)}
          <tr class:mine={row.name === me}>
            <td class="rank"></td>
            <td>{row.name}{#if row.name === me}<span class="you"> (you)</span>{/if}</td>
            {#if !compact}<td class="r num muted">{Math.round(row.rating)}</td>{/if}
            <td class="r num muted" colspan={compact ? 1 : 2}>{row.matches} of {need}</td>
          </tr>
        {/each}
      </tbody>
    {/if}
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
  .group th {
    border-top: 1px solid var(--line);
  }
  .nowrap {
    white-space: nowrap;
  }
  @media (max-width: 480px) {
    .wide {
      display: none;
    }
  }
</style>
