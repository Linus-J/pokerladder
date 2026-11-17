<script lang="ts">
  import { Trophy } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import LeaderboardTable from '../components/LeaderboardTable.svelte'
  import { api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import type { LeaderboardRow } from '../lib/protocol'
  import { link } from '../lib/router.svelte'

  let rows = $state<LeaderboardRow[] | null>(null)
  let failed = $state(false)
  onMount(() => {
    api.leaderboard().then(
      (r) => (rows = r),
      () => (failed = true),
    )
  })
</script>

<div class="page">
  <section class="block">
    <header><Icon of={Trophy} /><h1>Leaderboard</h1><span class="aside muted">Top 100</span></header>
    <LeaderboardTable {rows} {failed} me={session.me?.name} />
  </section>

  <aside class="block">
    <header><h2>How ratings work</h2></header>
    <div class="block-body small">
      <p>Everyone starts at 1500. Beating an equal player is worth 16 points; upsets are worth more.</p>
      <p>Only finished matches count. You appear here after your first one.</p>
      <p><a href="/about#ratings" onclick={link}>More about ratings</a></p>
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
</style>
