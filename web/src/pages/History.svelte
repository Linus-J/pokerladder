<script lang="ts">
  import { api } from '../lib/api'
  import type { HandHistory, MatchSummary } from '../lib/protocol'
  import { link, router } from '../lib/router.svelte'

  let matches = $state<MatchSummary[] | null>(null)
  let hands = $state<HandHistory[] | null>(null)
  let error = $state<string | null>(null)

  const selected = $derived.by(() => {
    const m = router.path.match(/^\/history\/(\d+)$/)
    return m ? Number(m[1]) : null
  })

  $effect(() => {
    api.matches().then(
      (m) => (matches = m),
      () => (error = "Couldn't load your matches."),
    )
  })

  $effect(() => {
    hands = null
    if (selected === null) return
    const id = selected
    api.hands(id).then(
      (h) => {
        if (selected === id) hands = h
      },
      () => (error = "Couldn't load that match."),
    )
  })

  function when(iso: string) {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }

  function outcome(m: MatchSummary) {
    if (m.status === 'in_progress') return { text: 'In progress', cls: 'muted' }
    if (m.status === 'abandoned') return { text: 'Abandoned', cls: 'muted' }
    return m.won ? { text: 'Won', cls: 'good' } : { text: 'Lost', cls: 'error' }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text)
  }
</script>

<div class="wrap">
  <section class="panel matches">
    <h2>Your matches</h2>
    {#if error}<p class="error">{error}</p>{/if}
    {#if !matches}
      <p class="muted">Loading…</p>
    {:else if matches.length === 0}
      <p class="muted">No matches yet.</p>
    {:else}
      <table class="list">
        <thead><tr><th>Date</th><th>Opponent</th><th>Result</th><th class="r">Rating</th></tr></thead>
        <tbody>
          {#each matches as m (m.id)}
            {@const o = outcome(m)}
            {@const d = m.rating_after !== null && m.rating_before !== null ? m.rating_after - m.rating_before : null}
            <tr class:sel={m.id === selected}>
              <td><a href="/history/{m.id}" onclick={link}>{when(m.started_at)}</a></td>
              <td>{m.opponent}</td>
              <td class={o.cls}>{o.text}</td>
              <td class="r num">
                {#if d !== null}<span class={d >= 0 ? 'good' : 'error'}>{d >= 0 ? '+' : ''}{d.toFixed(1)}</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  {#if selected !== null}
    <section class="panel hands">
      <h2>Hand histories</h2>
      {#if !hands}
        <p class="muted">Loading…</p>
      {:else if hands.length === 0}
        <p class="muted">No completed hands.</p>
      {:else}
        <button type="button" onclick={() => copy(hands!.map((h) => h.text).join('\n\n'))}>
          Copy all ({hands.length})
        </button>
        {#each hands as h (h.number)}
          <details>
            <summary>Hand {h.number} <span class="muted">· {new Date(h.started_at).toLocaleTimeString()}</span></summary>
            <pre>{h.text}</pre>
          </details>
        {/each}
      {/if}
    </section>
  {/if}
</div>

<style>
  .wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 1rem;
    align-items: start;
  }
  .wrap:has(.hands) .matches {
    align-self: start;
  }
  tr.sel td {
    background: rgba(227, 179, 65, 0.07);
  }
  details {
    border-bottom: 1px solid var(--line);
    padding: 0.4rem 0;
  }
  details:first-of-type {
    margin-top: 0.75rem;
  }
  summary {
    cursor: pointer;
    font-weight: 600;
  }
  pre {
    font-family: var(--mono);
    font-size: 0.78rem;
    white-space: pre-wrap;
    background: var(--bg);
    border-radius: 6px;
    padding: 0.75rem;
    overflow-x: auto;
  }
  @media (max-width: 900px) {
    .wrap {
      grid-template-columns: 1fr;
    }
  }
</style>
